use bstr::{BString, B};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

use crate::raw_entry::RawEntry;
use crate::HEADER_SIZE;

#[derive(PartialEq, Default, Debug)]
pub struct TestEntry {
    pub header: u32,
    pub json: &'static str,
    pub padding_len: usize,
    pub value: f64,
}

impl TestEntry {
    pub fn new(json: &'static str, value: f64) -> Self {
        TestEntry {
            header: json.len() as u32,
            json,
            padding_len: RawEntry::padding_len(json.len()),
            value,
        }
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        [
            B(&self.header.to_ne_bytes()),
            self.json.as_bytes(),
            &vec![b' '; self.padding_len],
            B(&self.value.to_ne_bytes()),
        ]
        .concat()
    }
    pub fn as_bstring(&self) -> BString {
        [
            B(&self.header.to_ne_bytes()),
            self.json.as_bytes(),
            &vec![b' '; self.padding_len],
            B(&self.value.to_ne_bytes()),
        ]
        .concat()
        .into()
    }

    pub fn as_bytes_no_header(&self) -> BString {
        [
            self.json.as_bytes(),
            &vec![b' '; self.padding_len],
            B(&self.value.to_ne_bytes()),
        ]
        .concat()
        .into()
    }
}

/// Format the data for a `.db` file.
/// Optional header value can be used to set an invalid `used` size.
pub fn entries_to_db(entries: &[&'static str], values: &[f64], header: Option<u32>) -> Vec<u8> {
    let mut out = Vec::new();

    let entry_bytes: Vec<_> = entries
        .iter()
        .zip(values)
        .flat_map(|(e, val)| TestEntry::new(e, *val).as_bytes())
        .collect();

    let used = match header {
        Some(u) => u,
        None => (entry_bytes.len() + HEADER_SIZE) as u32,
    };

    out.extend(used.to_ne_bytes());
    out.extend([0x0u8; 4]); // Padding.
    out.extend(entry_bytes);

    out
}

/// A temporary file, path, and dir for use with testing.
#[derive(Debug)]
pub struct TestFile {
    pub file: File,
    pub path: PathBuf,
    pub dir: TempDir,
}

impl TestFile {
    pub fn new(file_data: &[u8]) -> TestFile {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();

        file.write_all(file_data).unwrap();
        file.sync_all().unwrap();
        file.rewind().unwrap();

        // We need to keep `dir` in scope so it doesn't drop before the files it
        // contains, which may prevent cleanup.
        TestFile { file, path, dir }
    }
}

mod test {
    use super::*;

    #[test]
    fn test_entry_new() {
        let json = "foobar";
        let value = 1.0f64;
        let expected = TestEntry {
            header: 6,
            json,
            padding_len: 6,
            value,
        };

        let actual = TestEntry::new(json, value);
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_entry_bytes() {
        let json = "foobar";
        let value = 1.0f64;
        let expected = [
            &6u32.to_ne_bytes(),
            B(json),
            &[b' '; 6],
            &value.to_ne_bytes(),
        ]
        .concat();

        let actual = TestEntry::new(json, value).as_bstring();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_entry_bytes_no_header() {
        let json = "foobar";
        let value = 1.0f64;
        let expected = [B(json), &[b' '; 6], &value.to_ne_bytes()].concat();

        let actual = TestEntry::new(json, value).as_bytes_no_header();
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_entries_to_db_header_correct() {
        let json = &["foobar", "qux"];
        let values = &[1.0, 2.0];

        let out = entries_to_db(json, values, None);

        assert_eq!(48u32.to_ne_bytes(), out[0..4], "used set correctly");
        assert_eq!([0u8; 4], out[4..8], "padding set");
        assert_eq!(
            TestEntry::new(json[0], values[0]).as_bytes(),
            out[8..32],
            "first entry matches"
        );
        assert_eq!(
            TestEntry::new(json[1], values[1]).as_bytes(),
            out[32..48],
            "second entry matches"
        );
    }

    #[test]
    fn test_entries_to_db_header_wrong() {
        let json = &["foobar", "qux"];
        let values = &[1.0, 2.0];

        const WRONG_USED: u32 = 1000;
        let out = entries_to_db(json, values, Some(WRONG_USED));

        assert_eq!(
            WRONG_USED.to_ne_bytes(),
            out[0..4],
            "used set to value requested"
        );
        assert_eq!([0u8; 4], out[4..8], "padding set");
        assert_eq!(
            TestEntry::new(json[0], values[0]).as_bytes(),
            out[8..32],
            "first entry matches"
        );
        assert_eq!(
            TestEntry::new(json[1], values[1]).as_bytes(),
            out[32..48],
            "second entry matches"
        );
    }

    #[test]
    fn test_file() {
        let mut test_file = TestFile::new(b"foobar");
        let stat = test_file.file.metadata().unwrap();

        assert_eq!(6, stat.len(), "file length");
        assert_eq!(
            0,
            test_file.file.stream_position().unwrap(),
            "at start of file"
        );
        let mut out_buf = vec![0u8; 256];
        let read_result = test_file.file.read(&mut out_buf);
        assert!(read_result.is_ok());
        assert_eq!(6, read_result.unwrap(), "file is readable");

        let write_result = test_file.file.write(b"qux");
        assert!(write_result.is_ok());
        assert_eq!(3, write_result.unwrap(), "file is writable");
    }
}
