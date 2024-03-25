use magnus::Symbol;
use serde::Deserialize;
use serde_json::value::RawValue;
use smallvec::SmallVec;
use std::fmt::Write;
use std::str;

use crate::error::{MmapError, RubyError};
use crate::file_info::FileInfo;
use crate::raw_entry::RawEntry;
use crate::Result;
use crate::{SYM_GAUGE, SYM_LIVESUM, SYM_MAX, SYM_MIN};

/// A metrics entry extracted from a `*.db` file.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub data: EntryData,
    pub meta: EntryMetadata,
}

/// String slices pointing to the fields of a borrowed `Entry`'s JSON data.
#[derive(Deserialize, Debug)]
pub struct MetricText<'a> {
    pub family_name: &'a str,
    pub metric_name: &'a str,
    pub labels: SmallVec<[&'a str; 4]>,
    #[serde(borrow)]
    pub values: SmallVec<[&'a RawValue; 4]>,
}

/// The primary data payload for a `FileEntry`, the JSON string and the
/// associated pid, if significant. Used as the key for `EntryMap`.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct EntryData {
    pub json: String,
    pub pid: Option<String>,
}

impl<'a> PartialEq<BorrowedData<'a>> for EntryData {
    fn eq(&self, other: &BorrowedData) -> bool {
        self.pid.as_deref() == other.pid && self.json == other.json
    }
}

impl<'a> TryFrom<BorrowedData<'a>> for EntryData {
    type Error = MmapError;

    fn try_from(borrowed: BorrowedData) -> Result<Self> {
        let mut json = String::new();
        if json.try_reserve_exact(borrowed.json.len()).is_err() {
            return Err(MmapError::OutOfMemory(borrowed.json.len()));
        }
        json.push_str(borrowed.json);

        Ok(Self {
            json,
            // Don't bother checking for allocation failure, typically ~10 bytes
            pid: borrowed.pid.map(|p| p.to_string()),
        })
    }
}

/// A borrowed copy of the JSON string and pid for a `FileEntry`. We use this
/// to check if a given string/pid combination is present in the `EntryMap`,
/// copying them to owned values only when needed.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct BorrowedData<'a> {
    pub json: &'a str,
    pub pid: Option<&'a str>,
}

impl<'a> BorrowedData<'a> {
    pub fn new(
        raw_entry: &'a RawEntry,
        file_info: &'a FileInfo,
        pid_significant: bool,
    ) -> Result<Self> {
        let json = str::from_utf8(raw_entry.json())
            .map_err(|e| MmapError::Encoding(format!("invalid UTF-8 in entry JSON: {e}")))?;

        let pid = if pid_significant {
            Some(file_info.pid.as_str())
        } else {
            None
        };

        Ok(Self { json, pid })
    }
}

/// The metadata associated with a `FileEntry`. The value in `EntryMap`.
#[derive(Clone, Debug)]
pub struct EntryMetadata {
    pub multiprocess_mode: Symbol,
    pub type_: Symbol,
    pub value: f64,
}

impl EntryMetadata {
    /// Construct a new `FileEntry`, copying the JSON string from the `RawEntry`
    /// into an internal buffer.
    pub fn new(mmap_entry: &RawEntry, file: &FileInfo) -> Result<Self> {
        let value = mmap_entry.value();

        Ok(EntryMetadata {
            multiprocess_mode: file.multiprocess_mode,
            type_: file.type_,
            value,
        })
    }

    /// Combine values with another `EntryMetadata`.
    pub fn merge(&mut self, other: &Self) {
        if self.type_ == SYM_GAUGE {
            match self.multiprocess_mode {
                s if s == SYM_MIN => self.value = self.value.min(other.value),
                s if s == SYM_MAX => self.value = self.value.max(other.value),
                s if s == SYM_LIVESUM => self.value += other.value,
                _ => self.value = other.value,
            }
        } else {
            self.value += other.value;
        }
    }

    /// Validate if pid is significant for metric.
    pub fn is_pid_significant(&self) -> bool {
        let mp = self.multiprocess_mode;

        self.type_ == SYM_GAUGE && !(mp == SYM_MIN || mp == SYM_MAX || mp == SYM_LIVESUM)
    }
}

impl FileEntry {
    /// Convert the sorted entries into a String in Prometheus metrics format.
    pub fn entries_to_string(entries: Vec<FileEntry>) -> Result<String> {
        // We guesstimate that lines are ~100 bytes long, preallocate the string to
        // roughly that size.
        let mut out = String::new();
        out.try_reserve(entries.len() * 128)
            .map_err(|_| MmapError::OutOfMemory(entries.len() * 128))?;

        let mut prev_name: Option<String> = None;

        let entry_count = entries.len();
        let mut processed_count = 0;

        for entry in entries {
            let metrics_data = match serde_json::from_str::<MetricText>(&entry.data.json) {
                Ok(m) => {
                    if m.labels.len() != m.values.len() {
                        continue;
                    }
                    m
                }
                // We don't exit the function here so the total number of invalid
                // entries can be calculated below.
                Err(_) => continue,
            };

            match prev_name.as_ref() {
                Some(p) if p == metrics_data.family_name => {}
                _ => {
                    entry.append_header(metrics_data.family_name, &mut out);
                    prev_name = Some(metrics_data.family_name.to_owned());
                }
            }

            entry.append_entry(metrics_data, &mut out)?;

            writeln!(&mut out, " {}", entry.meta.value)
                .map_err(|e| MmapError::Other(format!("Failed to append to output: {e}")))?;

            processed_count += 1;
        }

        if processed_count != entry_count {
            return Err(MmapError::legacy(
                format!("Processed entries {processed_count} != map entries {entry_count}"),
                RubyError::Runtime,
            ));
        }

        Ok(out)
    }

    fn append_header(&self, family_name: &str, out: &mut String) {
        out.push_str("# HELP ");
        out.push_str(family_name);
        out.push_str(" Multiprocess metric\n");

        out.push_str("# TYPE ");
        out.push_str(family_name);
        out.push(' ');

        out.push_str(&self.meta.type_.name().expect("name was invalid UTF-8"));
        out.push('\n');
    }

    fn append_entry(&self, json_data: MetricText, out: &mut String) -> Result<()> {
        out.push_str(json_data.metric_name);

        if json_data.labels.is_empty() {
            if let Some(pid) = self.data.pid.as_ref() {
                out.push_str("{pid=\"");
                out.push_str(pid);
                out.push_str("\"}");
            }

            return Ok(());
        }

        out.push('{');

        let it = json_data.labels.iter().zip(json_data.values.iter());

        for (i, (&key, val)) in it.enumerate() {
            out.push_str(key);
            out.push('=');

            match val.get() {
                "null" => out.push_str("\"\""),
                s if s.starts_with('"') => out.push_str(s),
                s => {
                    // Quote numeric values.
                    out.push('"');
                    out.push_str(s);
                    out.push('"');
                }
            }

            if i < json_data.labels.len() - 1 {
                out.push(',');
            }
        }

        if let Some(pid) = self.data.pid.as_ref() {
            out.push_str(",pid=\"");
            out.push_str(pid);
            out.push('"');
        }

        out.push('}');

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use bstr::BString;
    use indoc::indoc;

    use super::*;
    use crate::file_info::FileInfo;
    use crate::raw_entry::RawEntry;
    use crate::testhelper::{TestEntry, TestFile};

    #[test]
    fn test_entries_to_string() {
        struct TestCase {
            name: &'static str,
            multiprocess_mode: &'static str,
            json: &'static [&'static str],
            values: &'static [f64],
            pids: &'static [&'static str],
            expected_out: Option<&'static str>,
            expected_err: Option<MmapError>,
        }

        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let tc = vec![
            TestCase {
                name: "one metric, pid significant",
                multiprocess_mode: "all",
                json: &[r#"["family","name",["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.0],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="value_b",pid="worker-1"} 1
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "one metric, no pid",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.0],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="value_b"} 1
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "many labels",
                multiprocess_mode: "min",
                json: &[
                    r#"["family","name",["label_a","label_b","label_c","label_d","label_e"],["value_a","value_b","value_c","value_d","value_e"]]"#,
                ],
                values: &[1.0],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="value_b",label_c="value_c",label_d="value_d",label_e="value_e"} 1
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "floating point shown",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="value_b"} 1.5
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "numeric value",
                multiprocess_mode: "min",
                json: &[
                    r#"["family","name",["label_a","label_b","label_c"],["value_a",403,-0.2E5]]"#,
                ],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="403",label_c="-0.2E5"} 1.5
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "null value",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a",null]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b=""} 1.5
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "comma in value",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a","value,_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{label_a="value_a",label_b="value,_b"} 1.5
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "no labels, pid significant",
                multiprocess_mode: "all",
                json: &[r#"["family","name",[],[]]"#],
                values: &[1.0],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name{pid="worker-1"} 1
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "no labels, no pid",
                multiprocess_mode: "min",
                json: &[r#"["family","name",[],[]]"#],
                values: &[1.0],
                pids: &["worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    name 1
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "two metrics, same family, pid significant",
                multiprocess_mode: "all",
                json: &[
                    r#"["family","first",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["family","second",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0],
                pids: &["worker-1", "worker-1"],
                expected_out: Some(indoc! {r##"# HELP family Multiprocess metric
                    # TYPE family gauge
                    first{label_a="value_a",label_b="value_b",pid="worker-1"} 1
                    second{label_a="value_a",label_b="value_b",pid="worker-1"} 2
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "two metrics, different family, pid significant",
                multiprocess_mode: "min",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["second_family","second_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0],
                pids: &["worker-1", "worker-1"],
                expected_out: Some(indoc! {r##"# HELP first_family Multiprocess metric
                    # TYPE first_family gauge
                    first_name{label_a="value_a",label_b="value_b"} 1
                    # HELP second_family Multiprocess metric
                    # TYPE second_family gauge
                    second_name{label_a="value_a",label_b="value_b"} 2
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "three metrics, two different families, pid significant",
                multiprocess_mode: "all",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["first_family","second_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["second_family","second_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0, 3.0],
                pids: &["worker-1", "worker-1", "worker-1"],
                expected_out: Some(indoc! {r##"# HELP first_family Multiprocess metric
                    # TYPE first_family gauge
                    first_name{label_a="value_a",label_b="value_b",pid="worker-1"} 1
                    second_name{label_a="value_a",label_b="value_b",pid="worker-1"} 2
                    # HELP second_family Multiprocess metric
                    # TYPE second_family gauge
                    second_name{label_a="value_a",label_b="value_b",pid="worker-1"} 3
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "same metrics, pid significant, separate workers",
                multiprocess_mode: "all",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0],
                pids: &["worker-1", "worker-2"],
                expected_out: Some(indoc! {r##"# HELP first_family Multiprocess metric
                    # TYPE first_family gauge
                    first_name{label_a="value_a",label_b="value_b",pid="worker-1"} 1
                    first_name{label_a="value_a",label_b="value_b",pid="worker-2"} 2
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "same metrics, pid not significant, separate workers",
                multiprocess_mode: "max",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0],
                pids: &["worker-1", "worker-2"],
                expected_out: Some(indoc! {r##"# HELP first_family Multiprocess metric
                    # TYPE first_family gauge
                    first_name{label_a="value_a",label_b="value_b"} 1
                    first_name{label_a="value_a",label_b="value_b"} 2
                    "##}),
                expected_err: None,
            },
            TestCase {
                name: "entry fails to parse",
                multiprocess_mode: "min",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"[not valid"#,
                ],
                values: &[1.0, 2.0],
                pids: &["worker-1", "worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 1 != map entries 2".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many values",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a"],["value_a","value,_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "no values",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "no labels or values",
                multiprocess_mode: "min",
                json: &[r#"["family","name","foo"]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many leading brackets",
                multiprocess_mode: "min",
                json: &[r#"[["family","name",["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many trailing brackets",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a","value_b"]]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many leading label brackets",
                multiprocess_mode: "min",
                json: &[r#"["family","name",[["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many leading label brackets",
                multiprocess_mode: "min",
                json: &[r#"["family","name",[["label_a","label_b"],["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "too many leading value brackets",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],[["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "misplaced bracket",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],]["value_a","value_b"]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "comma in numeric",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a",403,0]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
            TestCase {
                name: "non-e letter in numeric",
                multiprocess_mode: "min",
                json: &[r#"["family","name",["label_a","label_b"],["value_a",-2.0c5]]"#],
                values: &[1.5],
                pids: &["worker-1"],
                expected_out: None,
                expected_err: Some(MmapError::legacy(
                    "Processed entries 0 != map entries 1".to_owned(),
                    RubyError::Runtime,
                )),
            },
        ];

        for case in tc {
            let name = case.name;

            let input_bytes: Vec<BString> = case
                .json
                .iter()
                .zip(case.values)
                .map(|(&s, &value)| TestEntry::new(s, value).as_bstring())
                .collect();

            let mut file_infos = Vec::new();
            for pid in case.pids {
                let TestFile {
                    file,
                    path,
                    dir: _dir,
                } = TestFile::new(b"foobar");

                let info = FileInfo {
                    file,
                    path,
                    len: case.json.len(),
                    multiprocess_mode: Symbol::new(case.multiprocess_mode),
                    type_: Symbol::new("gauge"),
                    pid: pid.to_string(),
                };
                file_infos.push(info);
            }

            let file_entries: Vec<FileEntry> = input_bytes
                .iter()
                .map(|s| RawEntry::from_slice(s).unwrap())
                .zip(file_infos)
                .map(|(entry, info)| {
                    let meta = EntryMetadata::new(&entry, &info).unwrap();
                    let borrowed =
                        BorrowedData::new(&entry, &info, meta.is_pid_significant()).unwrap();
                    let data = EntryData::try_from(borrowed).unwrap();
                    FileEntry { data, meta }
                })
                .collect();

            let output = FileEntry::entries_to_string(file_entries);

            if let Some(expected_out) = case.expected_out {
                assert_eq!(
                    expected_out,
                    output.as_ref().unwrap(),
                    "test case: {name} - output"
                );
            }

            if let Some(expected_err) = case.expected_err {
                assert_eq!(
                    expected_err,
                    output.unwrap_err(),
                    "test case: {name} - error"
                );
            }
        }
    }

    #[test]
    fn test_merge() {
        struct TestCase {
            name: &'static str,
            metric_type: &'static str,
            multiprocess_mode: &'static str,
            values: &'static [f64],
            expected_value: f64,
        }

        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let tc = vec![
            TestCase {
                name: "gauge max",
                metric_type: "gauge",
                multiprocess_mode: "max",
                values: &[1.0, 5.0],
                expected_value: 5.0,
            },
            TestCase {
                name: "gauge min",
                metric_type: "gauge",
                multiprocess_mode: "min",
                values: &[1.0, 5.0],
                expected_value: 1.0,
            },
            TestCase {
                name: "gauge livesum",
                metric_type: "gauge",
                multiprocess_mode: "livesum",
                values: &[1.0, 5.0],
                expected_value: 6.0,
            },
            TestCase {
                name: "gauge all",
                metric_type: "gauge",
                multiprocess_mode: "all",
                values: &[1.0, 5.0],
                expected_value: 5.0,
            },
            TestCase {
                name: "not a gauge",
                metric_type: "histogram",
                multiprocess_mode: "max",
                values: &[1.0, 5.0],
                expected_value: 6.0,
            },
        ];

        for case in tc {
            let name = case.name;
            let json = r#"["family","metric",["label_a","label_b"],["value_a","value_b"]]"#;

            let TestFile {
                file,
                path,
                dir: _dir,
            } = TestFile::new(b"foobar");

            let info = FileInfo {
                file,
                path,
                len: json.len(),
                multiprocess_mode: Symbol::new(case.multiprocess_mode),
                type_: Symbol::new(case.metric_type),
                pid: "worker-1".to_string(),
            };

            let input_bytes: Vec<BString> = case
                .values
                .iter()
                .map(|&value| TestEntry::new(json, value).as_bstring())
                .collect();

            let entries: Vec<FileEntry> = input_bytes
                .iter()
                .map(|s| RawEntry::from_slice(s).unwrap())
                .map(|entry| {
                    let meta = EntryMetadata::new(&entry, &info).unwrap();
                    let borrowed =
                        BorrowedData::new(&entry, &info, meta.is_pid_significant()).unwrap();
                    let data = EntryData::try_from(borrowed).unwrap();
                    FileEntry { data, meta }
                })
                .collect();

            let mut entry_a = entries[0].clone();
            let entry_b = entries[1].clone();
            entry_a.meta.merge(&entry_b.meta);

            assert_eq!(
                case.expected_value, entry_a.meta.value,
                "test case: {name} - value"
            );
        }
    }
}
