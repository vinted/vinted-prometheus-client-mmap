use core::panic;
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
use std::io::Cursor;
use varint_rs::VarintWriter;

pub mod io {
    pub mod prometheus {
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/io.prometheus.client.rs"));
        }
    }
}

/// A metrics entry extracted from a `*.db` file.
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub data: EntryData,
    pub meta: EntryMetadata,
}

/// String slices pointing to the fields of a borrowed `Entry`'s JSON data.
#[derive(Deserialize, Debug, Clone)]
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

use crate::io::prometheus::client::MetricType::{Counter, Gauge, Histogram, Summary};
use itertools::Itertools;
use prost::Message;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hash;
use std::hash::Hasher;

use std::io::Write as OtherWrite;
impl FileEntry {
    pub fn trim_quotes(s: &str) -> String {
        let mut chars = s.chars();

        if s.starts_with('"') {
            chars.next();
        }
        if s.ends_with('"') {
            chars.next_back();
        }

        chars.as_str().to_string()
    }

    pub fn entries_to_protobuf(entries: Vec<FileEntry>) -> Result<String> {
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut mtrcs: HashMap<u64, io::prometheus::client::Metric> = HashMap::new();
        let mut metric_types = HashMap::new();
        let mut metric_names = HashMap::new();

        entries
            .iter()
            // TODO: Don't just unwrap. Handle the error gracefully.
            .map(|v| {
                (
                    v,
                    serde_json::from_str::<MetricText>(&v.data.json)
                        .expect("cannot parse json entry"),
                    v.meta.type_.name().expect("getting name").into_owned(),
                )
            })
            .filter(|v| v.1.labels.len() == v.1.values.len())
            .group_by(|v| v.1.family_name)
            .into_iter()
            .for_each(|(_, group)| {
                // NOTE(GiedriusS): different dynamic labels fall under the same
                // metric group.

                'outer: for gr in group {
                    let metric_type = gr.2;

                    let lbls =
                        gr.1.labels
                            .iter()
                            .map(|l| Self::trim_quotes(l))
                            .zip(gr.1.values.iter().map(|v| Self::trim_quotes(v.get())));

                    let mut m = io::prometheus::client::Metric {
                        label: lbls
                            .clone()
                            .map(|l| io::prometheus::client::LabelPair {
                                name: Some(l.0),
                                value: Some(l.1.to_string()),
                            })
                            .collect::<Vec<io::prometheus::client::LabelPair>>(),
                        gauge: None,
                        counter: None,
                        summary: None,
                        untyped: None,
                        histogram: None,
                        timestamp_ms: None,
                    };

                    match metric_type.as_str() {
                        "counter" => {
                            let mut hasher = DefaultHasher::new();

                            // Iterate over the tuples and hash their elements
                            for (a, b) in lbls {
                                a.hash(&mut hasher);
                                b.hash(&mut hasher);
                            }
                            "counter".hash(&mut hasher);

                            // Get the final u64 hash value
                            let hash_value = hasher.finish();
                            m.counter = Some(io::prometheus::client::Counter {
                                value: Some(gr.0.meta.value),
                                created_timestamp: None,
                                exemplar: None,
                            });

                            mtrcs.insert(hash_value, m);
                            metric_types.insert(hash_value, "counter");
                            metric_names.insert(hash_value, gr.1.metric_name);
                        }
                        "gauge" => {
                            let mut hasher = DefaultHasher::new();

                            // Iterate over the tuples and hash their elements
                            for (a, b) in lbls {
                                a.hash(&mut hasher);
                                b.hash(&mut hasher);
                            }
                            "gauge".hash(&mut hasher);

                            let hash_value = hasher.finish();

                            m.gauge = Some(io::prometheus::client::Gauge {
                                value: Some(gr.0.meta.value),
                            });
                            mtrcs.insert(hash_value, m);
                            metric_types.insert(hash_value, "gauge");
                            metric_names.insert(hash_value, gr.1.metric_name);
                        }
                        "histogram" => {
                            let mut hasher = DefaultHasher::new();

                            let mut le: Option<f64> = None;

                            // Iterate over the tuples and hash their elements
                            for (a, b) in lbls {
                                if a != "le" {
                                    a.hash(&mut hasher);
                                    b.hash(&mut hasher);
                                }

                                // Safe to ignore +Inf bound.
                                if a == "le" {
                                    if b == "+Inf" {
                                        continue 'outer;
                                    }
                                    let leparsed = b.parse::<f64>();
                                    match leparsed {
                                        Ok(p) => le = Some(p),
                                        Err(e) => panic!("failed to parse {} due to {}", b, e),
                                    }
                                }
                            }
                            "histogram".hash(&mut hasher);

                            let hash_value = hasher.finish();

                            match mtrcs.get_mut(&hash_value) {
                                Some(v) => {
                                    let hs =
                                        v.histogram.as_mut().expect("getting mutable histogram");

                                    for bucket in &mut hs.bucket {
                                        if bucket.upper_bound != le {
                                            continue;
                                        }

                                        let mut curf: f64 =
                                            bucket.cumulative_count_float.unwrap_or_default();
                                        curf += gr.0.meta.value;

                                        bucket.cumulative_count_float = Some(curf);
                                    }
                                }
                                None => {
                                    let mut final_metric_name = gr.1.metric_name;

                                    if let Some(stripped) =
                                        final_metric_name.strip_suffix("_bucket")
                                    {
                                        final_metric_name = stripped;
                                    }
                                    if let Some(stripped) = final_metric_name.strip_suffix("_sum") {
                                        final_metric_name = stripped;
                                    }
                                    if let Some(stripped) = final_metric_name.strip_suffix("_count")
                                    {
                                        final_metric_name = stripped;
                                    }

                                    let buckets = vec![io::prometheus::client::Bucket {
                                        cumulative_count: None,
                                        cumulative_count_float: Some(gr.0.meta.value),
                                        upper_bound: Some(
                                            le.expect(
                                                &format!("got no LE for {}", gr.1.metric_name)
                                                    .to_string(),
                                            ),
                                        ),
                                        exemplar: None,
                                    }];
                                    m.label = m
                                        .label
                                        .into_iter()
                                        .filter(|l| l.name != Some("le".to_string()))
                                        .collect_vec();
                                    // Create a new metric.
                                    m.histogram = Some(io::prometheus::client::Histogram {
                                        // All native histogram fields.
                                        sample_count: None,
                                        sample_count_float: None,
                                        sample_sum: None,
                                        created_timestamp: None,
                                        schema: None,
                                        zero_count: None,
                                        zero_count_float: None,
                                        zero_threshold: None,
                                        negative_count: vec![],
                                        negative_delta: vec![],
                                        negative_span: vec![],
                                        positive_count: vec![],
                                        positive_delta: vec![],
                                        positive_span: vec![],
                                        // All classic histogram fields.
                                        bucket: buckets,
                                    });
                                    mtrcs.insert(hash_value, m);
                                    metric_types.insert(hash_value, "histogram");
                                    metric_names.insert(hash_value, final_metric_name);
                                }
                            }
                        }
                        "summary" => {
                            let mut hasher = DefaultHasher::new();

                            let mut quantile: Option<f64> = None;

                            // Iterate over the tuples and hash their elements
                            for (a, b) in lbls {
                                if a != "quantile" {
                                    a.hash(&mut hasher);
                                    b.hash(&mut hasher);
                                }
                                if a == "quantile" {
                                    let quantileparsed = b.parse::<f64>();
                                    match quantileparsed {
                                        Ok(p) => quantile = Some(p),
                                        Err(e) => {
                                            panic!("failed to parse quantile {} due to {}", b, e)
                                        }
                                    }
                                }
                            }
                            "summary".hash(&mut hasher);
                            let hash_value = hasher.finish();

                            match mtrcs.get_mut(&hash_value) {
                                Some(v) => {
                                    // Go through and edit buckets.
                                    let smry = v.summary.as_mut().expect(
                                        &format!(
                                            "getting mutable summary for {}",
                                            gr.1.metric_name
                                        )
                                        .to_string(),
                                    );

                                    if gr.1.metric_name.ends_with("_count") {
                                        let samplecount = smry.sample_count.unwrap_or_default();
                                        smry.sample_count =
                                            Some((gr.0.meta.value as u64) + samplecount);
                                    } else if gr.1.metric_name.ends_with("_sum") {
                                        let samplesum: f64 = smry.sample_sum.unwrap_or_default();
                                        smry.sample_sum = Some(gr.0.meta.value + samplesum);
                                    } else {
                                        let mut found_quantile = false;
                                        for qntl in &mut smry.quantile {
                                            if qntl.quantile != quantile {
                                                continue;
                                            }

                                            let mut curq: f64 = qntl.quantile.unwrap_or_default();
                                            curq += gr.0.meta.value;

                                            qntl.quantile = Some(curq);
                                            found_quantile = true;
                                        }

                                        if !found_quantile {
                                            smry.quantile.push(io::prometheus::client::Quantile {
                                                quantile: quantile,
                                                value: Some(gr.0.meta.value),
                                            });
                                        }
                                    }
                                }
                                None => {
                                    m.label = m
                                        .label
                                        .into_iter()
                                        .filter(|l| l.name != Some("quantile".to_string()))
                                        .collect_vec();

                                    let mut final_metric_name = gr.1.metric_name;
                                    // If quantile then add to quantiles.
                                    // if ends with _count then add it to count.
                                    // If ends with _sum then add it to sum.
                                    if gr.1.metric_name.ends_with("_count") {
                                        final_metric_name =
                                            gr.1.metric_name.strip_suffix("_count").unwrap();
                                        m.summary = Some(io::prometheus::client::Summary {
                                            quantile: vec![],
                                            sample_count: Some(gr.0.meta.value as u64),
                                            sample_sum: None,
                                            created_timestamp: None,
                                        });
                                    } else if gr.1.metric_name.ends_with("_sum") {
                                        final_metric_name =
                                            gr.1.metric_name.strip_suffix("_sum").unwrap();
                                        m.summary = Some(io::prometheus::client::Summary {
                                            quantile: vec![],
                                            sample_sum: Some(gr.0.meta.value),
                                            sample_count: None,
                                            created_timestamp: None,
                                        });
                                    } else {
                                        let quantiles = vec![io::prometheus::client::Quantile {
                                            quantile: quantile,
                                            value: Some(gr.0.meta.value),
                                        }];
                                        m.summary = Some(io::prometheus::client::Summary {
                                            quantile: quantiles,
                                            sample_count: None,
                                            sample_sum: None,
                                            created_timestamp: None,
                                        });
                                    }

                                    mtrcs.insert(hash_value, m);
                                    metric_types.insert(hash_value, "summary");
                                    metric_names.insert(hash_value, final_metric_name);
                                }
                            }
                        }
                        mtype => {
                            panic!("unhandled metric type {}", mtype)
                        }
                    }
                }
            });

        mtrcs.iter().for_each(|mtrc| {
            let metric_name = metric_names.get(mtrc.0).expect("getting metric name");
            let metric_type = metric_types.get(mtrc.0).expect("getting metric type");

            let protobuf_mf = io::prometheus::client::MetricFamily {
                name: Some(metric_name.to_string()),
                help: Some("Multiprocess metric".to_string()),
                r#type: match metric_type.to_string().as_str() {
                    "counter" => Some(Counter.into()),
                    "gauge" => Some(Gauge.into()),
                    "histogram" => Some(Histogram.into()),
                    "summary" => Some(Summary.into()),
                    mtype => panic!("unhandled metric type {}", mtype),
                },
                metric: vec![mtrc.1.clone()],
            };

            let encoded_mf = protobuf_mf.encode_to_vec();

            buffer
                .write_u32_varint(
                    encoded_mf
                        .len()
                        .try_into()
                        .expect("failed to encode metricfamily"),
                )
                .unwrap();
            buffer
                .write_all(&encoded_mf)
                .expect("failed to write output");
        });

        // NOTE: Rust strings are bytes encoded in UTF-8. Ruby doesn't have such
        // invariant. So, let's convert those bytes to a string since everything ends
        // up as a string in Ruby.
        unsafe { Ok(str::from_utf8_unchecked(buffer.get_ref()).to_string()) }
    }

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
    fn test_trim_quotes() {
        assert_eq!("foo", FileEntry::trim_quotes("foo"));
        assert_eq!("foo", FileEntry::trim_quotes("\"foo\""));
    }

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
