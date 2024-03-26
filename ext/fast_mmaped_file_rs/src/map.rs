use hashbrown::hash_map::RawEntryMut;
use hashbrown::HashMap;
use magnus::class::file;
use magnus::{eval, exception::*, Error, RArray, Value};
use std::hash::{BuildHasher, Hash, Hasher};
use std::mem::size_of;

use crate::error::MmapError;
use crate::file_entry::{BorrowedData, EntryData, EntryMetadata, FileEntry};
use crate::file_info::FileInfo;
use crate::raw_entry::RawEntry;
use crate::util::read_u32;
use crate::Result;
use crate::{err, HEADER_SIZE};

/// A HashMap of JSON strings and their associated metadata.
/// Used to print metrics in text format.
///
/// The map key is the entry's JSON string and an optional pid string. The latter
/// allows us to have multiple entries on the map for multiple pids using the
/// same string.
#[derive(Default, Debug)]
pub struct EntryMap(HashMap<EntryData, EntryMetadata>);

impl EntryMap {
    /// Construct a new EntryMap.
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Given a list of files, read each one into memory and parse the metrics it contains.
    pub fn aggregate_files(&mut self, list_of_files: RArray) -> magnus::error::Result<()> {
        // Pre-allocate the `HashMap` and validate we don't OOM. The C implementation
        // ignores allocation failures here. We perform this check to avoid potential
        // panics. We assume ~1,000 entries per file, so 72 KiB allocated per file.
        self.0
            .try_reserve(list_of_files.len() * 1024)
            .map_err(|_| {
                err!(
                    no_mem_error(),
                    "Couldn't allocate for {} memory",
                    size_of::<FileEntry>() * list_of_files.len() * 1024
                )
            })?;

        // We expect file sizes between 4KiB and 4MiB. Pre-allocate 16KiB to reduce reallocations
        // a bit.
        let mut buf = Vec::new();
        buf.try_reserve(16_384)
            .map_err(|_| err!(no_mem_error(), "Couldn't allocate for {} memory", 16_384))?;

        for item in list_of_files.each() {
            let params = RArray::from_value(item?).expect("file list was not a Ruby Array");
            if params.len() != 4 {
                return Err(err!(
                    arg_error(),
                    "wrong number of arguments {} instead of 4",
                    params.len()
                ));
            }

            let params = params.to_value_array::<4>()?;

            let mut file_info = FileInfo::open_from_params(&params)?;
            file_info.read_from_file(&mut buf)?;
            self.process_buffer(file_info, &buf)?;
        }
        Ok(())
    }

    /// Consume the `EntryMap` and convert the key/value into`FileEntry`
    /// objects, sorting them by their JSON strings.
    pub fn into_sorted(self) -> Result<Vec<FileEntry>> {
        let mut sorted = Vec::new();

        // To match the behavior of the C version, pre-allocate the entries
        // and check for allocation failure. Generally idiomatic Rust would
        // `collect` the iterator into a new `Vec` in place, but this panics
        // if it can't allocate and we want to continue execution in that
        // scenario.
        if sorted.try_reserve_exact(self.0.len()).is_err() {
            return Err(MmapError::OutOfMemory(
                self.0.len() * size_of::<FileEntry>(),
            ));
        }

        sorted.extend(
            self.0
                .into_iter()
                .map(|(data, meta)| FileEntry { data, meta }),
        );

        sorted.sort_unstable_by(|x, y| x.data.cmp(&y.data));

        Ok(sorted)
    }

    /// Check if the `EntryMap` already contains the JSON string.
    /// If yes, update the associated value, if not insert the
    /// entry into the map.
    pub fn merge_or_store(&mut self, data: BorrowedData, meta: EntryMetadata) -> Result<()> {
        // Manually hash the `BorrowedData` and perform an equality check on the
        // key. This allows us to perform the comparison without allocating a
        // new `EntryData` that may not be needed.
        let mut state = self.0.hasher().build_hasher();
        data.hash(&mut state);
        let hash = state.finish();

        match self.0.raw_entry_mut().from_hash(hash, |k| k == &data) {
            RawEntryMut::Vacant(entry) => {
                // Allocate a new `EntryData` as the JSON/pid combination is
                // not present in the map.
                let owned = EntryData::try_from(data)?;
                entry.insert(owned, meta);
            }
            RawEntryMut::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.merge(&meta);
            }
        }

        Ok(())
    }

    /// Parse metrics data from a `.db` file and store in the `EntryMap`.
    fn process_buffer(&mut self, file_info: FileInfo, source: &[u8]) -> Result<()> {
        if source.len() < HEADER_SIZE {
            // Nothing to read, OK.
            return Ok(());
        }

        // CAST: no-op on 32-bit, widening on 64-bit.
        let used = read_u32(source, 0)? as usize;

        if used > source.len() {
            return Err(MmapError::PromParsing(format!(
                "source file {} corrupted, used {used} > file size {}",
                file_info.path.display(),
                source.len()
            )));
        }

        let mut pos = HEADER_SIZE;

        while pos + size_of::<u32>() < used {
            let raw_entry: RawEntry;

            if file_info.type_.to_string() == "exemplar" {
                raw_entry = RawEntry::from_slice_exemplar(&source[pos..used])?;

                if pos + raw_entry.total_len_exemplar() > used {
                    return Err(MmapError::PromParsing(format!(
                        "source file {} corrupted, used {used} < stored data length {}",
                        file_info.path.display(),
                        pos + raw_entry.total_len()
                    )));
                }

                pos += raw_entry.total_len_exemplar();

            } else {
                raw_entry = RawEntry::from_slice(&source[pos..used])?;

                if pos + raw_entry.total_len() > used {
                    return Err(MmapError::PromParsing(format!(
                        "source file {} corrupted, used {used} < stored data length {}",
                        file_info.path.display(),
                        pos + raw_entry.total_len()
                    )));
                }

                pos += raw_entry.total_len();
            }
            
            let meta = EntryMetadata::new(&raw_entry, &file_info)?;
            let data = BorrowedData::new(&raw_entry, &file_info, meta.is_pid_significant())?;

            self.merge_or_store(data, meta)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use magnus::Symbol;
    use std::mem;

    use super::*;
    use crate::file_entry::FileEntry;
    use crate::testhelper::{self, TestFile};

    impl EntryData {
        /// A helper function for tests to convert owned data to references.
        fn as_borrowed(&self) -> BorrowedData {
            BorrowedData {
                json: &self.json,
                pid: self.pid.as_deref(),
            }
        }
    }

    #[test]
    fn test_into_sorted() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let entries = vec![
            FileEntry {
                data: EntryData {
                    json: "zzzzzz".to_string(),
                    pid: Some("worker-0_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("max"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
            FileEntry {
                data: EntryData {
                    json: "zzz".to_string(),
                    pid: Some("worker-0_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("max"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
            FileEntry {
                data: EntryData {
                    json: "zzzaaa".to_string(),
                    pid: Some("worker-0_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("max"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
            FileEntry {
                data: EntryData {
                    json: "aaa".to_string(),
                    pid: Some("worker-0_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("max"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
            FileEntry {
                data: EntryData {
                    json: "ooo".to_string(),
                    pid: Some("worker-1_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("all"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
            FileEntry {
                data: EntryData {
                    json: "ooo".to_string(),
                    pid: Some("worker-0_0".to_string()),
                },
                meta: EntryMetadata {
                    multiprocess_mode: Symbol::new("all"),
                    type_: Symbol::new("gauge"),
                    value: Some(1.0),
                    ex: None,
                },
            },
        ];

        let mut map = EntryMap::new();

        for entry in entries {
            map.0.insert(entry.data, entry.meta);
        }

        let result = map.into_sorted();
        assert!(result.is_ok());
        let sorted = result.unwrap();
        assert_eq!(sorted.len(), 6);
        assert_eq!(sorted[0].data.json, "aaa");
        assert_eq!(sorted[1].data.json, "ooo");
        assert_eq!(sorted[1].data.pid.as_deref(), Some("worker-0_0"));
        assert_eq!(sorted[2].data.json, "ooo");
        assert_eq!(sorted[2].data.pid.as_deref(), Some("worker-1_0"));
        assert_eq!(sorted[3].data.json, "zzz");
        assert_eq!(sorted[4].data.json, "zzzaaa");
        assert_eq!(sorted[5].data.json, "zzzzzz");
    }

    #[test]
    fn test_merge_or_store() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let key = "foobar";

        let starting_entry = FileEntry {
            data: EntryData {
                json: key.to_string(),
                pid: Some("worker-0_0".to_string()),
            },
            meta: EntryMetadata {
                multiprocess_mode: Symbol::new("all"),
                type_: Symbol::new("gauge"),
                value: Some(1.0),
                ex: None,
            },
        };

        let matching_entry = FileEntry {
            data: EntryData {
                json: key.to_string(),
                pid: Some("worker-0_0".to_string()),
            },
            meta: EntryMetadata {
                multiprocess_mode: Symbol::new("all"),
                type_: Symbol::new("gauge"),
                value: Some(5.0),
                ex: None,
            },
        };

        let same_key_different_worker = FileEntry {
            data: EntryData {
                json: key.to_string(),
                pid: Some("worker-1_0".to_string()),
            },
            meta: EntryMetadata {
                multiprocess_mode: Symbol::new("all"),
                type_: Symbol::new("gauge"),
                value: Some(100.0),
                ex: None,
            },
        };

        let unmatched_entry = FileEntry {
            data: EntryData {
                json: "another key".to_string(),
                pid: Some("worker-0_0".to_string()),
            },
            meta: EntryMetadata {
                multiprocess_mode: Symbol::new("all"),
                type_: Symbol::new("gauge"),
                value: Some(100.0),
                ex: None,
            },
        };

        let mut map = EntryMap::new();

        map.0
            .insert(starting_entry.data.clone(), starting_entry.meta.clone());

        let matching_borrowed = matching_entry.data.as_borrowed();
        map.merge_or_store(matching_borrowed, matching_entry.meta)
            .unwrap();

        assert_eq!(
            5.0,
            map.0.get(&starting_entry.data).unwrap().value.unwrap(),
            "value updated"
        );
        assert_eq!(1, map.0.len(), "no entry added");

        let same_key_different_worker_borrowed = same_key_different_worker.data.as_borrowed();
        map.merge_or_store(
            same_key_different_worker_borrowed,
            same_key_different_worker.meta,
        )
        .unwrap();

        assert_eq!(
            5.0,
            map.0.get(&starting_entry.data).unwrap().value.unwrap(),
            "value unchanged"
        );

        assert_eq!(2, map.0.len(), "additional entry added");

        let unmatched_entry_borrowed = unmatched_entry.data.as_borrowed();
        map.merge_or_store(unmatched_entry_borrowed, unmatched_entry.meta)
            .unwrap();

        assert_eq!(
            5.0,
            map.0.get(&starting_entry.data).unwrap().value.unwrap(),
            "value unchanged"
        );
        assert_eq!(3, map.0.len(), "entry added");
    }

    #[test]
    fn test_process_buffer() {
        struct TestCase {
            name: &'static str,
            json: &'static [&'static str],
            values: &'static [f64],
            used: Option<u32>,
            expected_ct: usize,
            expected_err: Option<MmapError>,
        }

        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let tc = vec![
            TestCase {
                name: "single entry",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0],
                used: None,
                expected_ct: 1,
                expected_err: None,
            },
            TestCase {
                name: "multiple entries",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                    r#"["second_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0, 2.0],
                used: None,
                expected_ct: 2,
                expected_err: None,
            },
            TestCase {
                name: "empty",
                json: &[],
                values: &[],
                used: None,
                expected_ct: 0,
                expected_err: None,
            },
            TestCase {
                name: "used too long",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0],
                used: Some(9999),
                expected_ct: 0,
                expected_err: Some(MmapError::PromParsing(String::new())),
            },
            TestCase {
                name: "used too short",
                json: &[
                    r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#,
                ],
                values: &[1.0],
                used: Some(15),
                expected_ct: 0,
                expected_err: Some(MmapError::out_of_bounds(88, 7)),
            },
        ];

        for case in tc {
            let name = case.name;

            let input_bytes = testhelper::entries_to_db(case.json, case.values, case.used);

            let TestFile {
                file,
                path,
                dir: _dir,
            } = TestFile::new(&input_bytes);

            let info = FileInfo {
                file,
                path,
                len: case.json.len(),
                multiprocess_mode: Symbol::new("max"),
                type_: Symbol::new("gauge"),
                pid: "worker-1".to_string(),
            };

            let mut map = EntryMap::new();
            let result = map.process_buffer(info, &input_bytes);

            assert_eq!(case.expected_ct, map.0.len(), "test case: {name} - count");

            if let Some(expected_err) = case.expected_err {
                // Validate we have the right enum type for the error. Error
                // messages contain the temp dir path and can't be predicted
                // exactly.
                assert_eq!(
                    mem::discriminant(&expected_err),
                    mem::discriminant(&result.unwrap_err()),
                    "test case: {name} - failure"
                );
            } else {
                assert_eq!(Ok(()), result, "test case: {name} - success");

                assert_eq!(
                    case.json.len(),
                    map.0.len(),
                    "test case: {name} - all entries captured"
                );
            }
        }
    }
}
