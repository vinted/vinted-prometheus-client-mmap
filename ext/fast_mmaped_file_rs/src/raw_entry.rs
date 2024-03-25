use std::mem::size_of;

use crate::error::MmapError;
use crate::util;
use crate::util::CheckedOps;
use crate::Result;

/// The logic to save a `MetricsEntry`, or parse one from a byte slice.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct RawEntry<'a> {
    bytes: &'a [u8],
    encoded_len: usize,
}

impl<'a> RawEntry<'a> {
    /// Save an entry to the mmap, returning the value offset in the newly created entry.
    pub fn save(bytes: &'a mut [u8], key: &[u8], value: f64) -> Result<usize> {
        let total_len = Self::calc_total_len(key.len())?;

        if total_len > bytes.len() {
            return Err(MmapError::Other(format!(
                "entry length {total_len} larger than slice length {}",
                bytes.len()
            )));
        }

        // CAST: `calc_len` runs `check_encoded_len`, we know the key len
        // is less than i32::MAX. No risk of overflows or failed casts.
        let key_len: u32 = key.len() as u32;

        // Write the key length to the mmap.
        bytes[..size_of::<u32>()].copy_from_slice(&key_len.to_ne_bytes());

        // Advance slice past the size.
        let bytes = &mut bytes[size_of::<u32>()..];

        bytes[..key.len()].copy_from_slice(key);

        // Advance to end of key.
        let bytes = &mut bytes[key.len()..];

        let pad_len = Self::padding_len(key.len());
        bytes[..pad_len].fill(b' ');
        let bytes = &mut bytes[pad_len..];

        bytes[..size_of::<f64>()].copy_from_slice(&value.to_ne_bytes());

        Self::calc_value_offset(key.len())
    }

    /// Parse a byte slice starting into an `MmapEntry`.
    pub fn from_slice(bytes: &'a [u8]) -> Result<Self> {
        // CAST: no-op on 32-bit, widening on 64-bit.
        let encoded_len = util::read_u32(bytes, 0)? as usize;

        let total_len = Self::calc_total_len(encoded_len)?;

        // Confirm the value is in bounds of the slice provided.
        if total_len > bytes.len() {
            return Err(MmapError::out_of_bounds(total_len, bytes.len()));
        }

        // Advance slice past length int and cut at end of entry.
        let bytes = &bytes[size_of::<u32>()..total_len];

        Ok(Self { bytes, encoded_len })
    }

    /// Read the `f64` value of an entry from memory.
    #[inline]
    pub fn value(&self) -> f64 {
        // We've stripped off the leading u32, don't include that here.
        let offset = self.encoded_len + Self::padding_len(self.encoded_len);

        // UNWRAP: We confirm in the constructor that the value offset
        // is in-range for the slice.
        util::read_f64(self.bytes, offset).unwrap()
    }

    /// The length of the entry key without padding.
    #[inline]
    pub fn encoded_len(&self) -> usize {
        self.encoded_len
    }

    /// Returns a slice with the JSON string in the entry, excluding padding.
    #[inline]
    pub fn json(&self) -> &[u8] {
        &self.bytes[..self.encoded_len]
    }

    /// Calculate the total length of an `MmapEntry`, including the string length,
    /// string, padding, and value.
    #[inline]
    pub fn total_len(&self) -> usize {
        // UNWRAP:: We confirmed in the constructor that this doesn't overflow.
        Self::calc_total_len(self.encoded_len).unwrap()
    }

    /// Calculate the total length of an `MmapEntry`, including the string length,
    /// string, padding, and value. Validates encoding_len is within expected bounds.
    #[inline]
    pub fn calc_total_len(encoded_len: usize) -> Result<usize> {
        Self::calc_value_offset(encoded_len)?.add_chk(size_of::<f64>())
    }

    /// Calculate the value offset of an `MmapEntry`, including the string length,
    /// string, padding. Validates encoding_len is within expected bounds.
    #[inline]
    pub fn calc_value_offset(encoded_len: usize) -> Result<usize> {
        Self::check_encoded_len(encoded_len)?;

        Ok(size_of::<u32>() + encoded_len + Self::padding_len(encoded_len))
    }

    /// Calculate the number of padding bytes to add to the value key to reach
    /// 8-byte alignment. Does not validate key length.
    #[inline]
    pub fn padding_len(encoded_len: usize) -> usize {
        8 - (size_of::<u32>() + encoded_len) % 8
    }

    #[inline]
    fn check_encoded_len(encoded_len: usize) -> Result<()> {
        if encoded_len as u64 > i32::MAX as u64 {
            return Err(MmapError::KeyLength);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use bstr::ByteSlice;

    use super::*;
    use crate::testhelper::TestEntry;

    #[test]
    fn test_from_slice() {
        #[derive(PartialEq, Default, Debug)]
        struct TestCase {
            name: &'static str,
            input: TestEntry,
            expected_enc_len: Option<usize>,
            expected_err: Option<MmapError>,
        }

        let tc = vec![
            TestCase {
                name: "ok",
                input: TestEntry {
                    header: 61,
                    json: r#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                    padding_len: 7,
                    value: 1.0,
                },
                expected_enc_len: Some(61),
                ..Default::default()
            },
            TestCase {
                name: "zero length key",
                input: TestEntry {
                    header: 0,
                    json: "",
                    padding_len: 4,
                    value: 1.0,
                },
                expected_enc_len: Some(0),
                ..Default::default()
            },
            TestCase {
                name: "header value too large",
                input: TestEntry {
                    header: i32::MAX as u32 + 1,
                    json: "foo",
                    padding_len: 1,
                    value: 0.0,
                },
                expected_err: Some(MmapError::KeyLength),
                ..Default::default()
            },
            TestCase {
                name: "header value much longer than json len",
                input: TestEntry {
                    header: 256,
                    json: "foobar",
                    padding_len: 6,
                    value: 1.0,
                },
                expected_err: Some(MmapError::out_of_bounds(272, 24)),
                ..Default::default()
            },
            TestCase {
                // Situations where encoded_len is wrong but padding makes the
                // value offset the same are not caught.
                name: "header off by one",
                input: TestEntry {
                    header: 4,
                    json: "123",
                    padding_len: 1,
                    value: 1.0,
                },
                expected_err: Some(MmapError::out_of_bounds(24, 16)),
                ..Default::default()
            },
        ];

        for case in tc {
            let name = case.name;
            let input = case.input.as_bstring();

            let resp = RawEntry::from_slice(&input);

            if case.expected_err.is_none() {
                let expected_buf = case.input.as_bytes_no_header();
                let resp = resp.as_ref().unwrap();
                let bytes = resp.bytes;

                assert_eq!(expected_buf, bytes.as_bstr(), "test case: {name} - bytes",);

                assert_eq!(
                    resp.json(),
                    case.input.json.as_bytes(),
                    "test case: {name} - json matches"
                );

                assert_eq!(
                    resp.total_len(),
                    case.input.as_bstring().len(),
                    "test case: {name} - total_len matches"
                );

                assert_eq!(
                    resp.encoded_len(),
                    case.input.json.len(),
                    "test case: {name} - encoded_len matches"
                );

                assert!(
                    resp.json().iter().all(|&c| c != b' '),
                    "test case: {name} - no spaces in json"
                );

                let padding_len = RawEntry::padding_len(case.input.json.len());
                assert!(
                    bytes[resp.encoded_len..resp.encoded_len + padding_len]
                        .iter()
                        .all(|&c| c == b' '),
                    "test case: {name} - padding is spaces"
                );

                assert_eq!(
                    resp.value(),
                    case.input.value,
                    "test case: {name} - value is correct"
                );
            }

            if let Some(expected_enc_len) = case.expected_enc_len {
                assert_eq!(
                    expected_enc_len,
                    resp.as_ref().unwrap().encoded_len,
                    "test case: {name} - encoded len",
                );
            }

            if let Some(expected_err) = case.expected_err {
                assert_eq!(expected_err, resp.unwrap_err(), "test case: {name} - error",);
            }
        }
    }

    #[test]
    fn test_save() {
        struct TestCase {
            name: &'static str,
            key: &'static [u8],
            value: f64,
            buf_len: usize,
            expected_entry: Option<TestEntry>,
            expected_resp: Result<usize>,
        }

        // TODO No test case to validate keys with len > i32::MAX, adding a static that large crashes
        // the test binary.
        let tc = vec![
            TestCase {
                name: "ok",
                key: br#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                value: 256.0,
                buf_len: 256,
                expected_entry: Some(TestEntry {
                    header: 61,
                    json: r#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                    padding_len: 7,
                    value: 256.0,
                }),
                expected_resp: Ok(72),
            },
            TestCase {
                name: "zero length key",
                key: b"",
                value: 1.0,
                buf_len: 256,
                expected_entry: Some(TestEntry {
                    header: 0,
                    json: "",
                    padding_len: 4,
                    value: 1.0,
                }),
                expected_resp: Ok(8),
            },
            TestCase {
                name: "infinite value",
                key: br#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                value: f64::INFINITY,
                buf_len: 256,
                expected_entry: Some(TestEntry {
                    header: 61,
                    json: r#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                    padding_len: 7,
                    value: f64::INFINITY,
                }),
                expected_resp: Ok(72),
            },
            TestCase {
                name: "buf len matches entry len",
                key: br#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                value: 1.0,
                buf_len: 80,
                expected_entry: Some(TestEntry {
                    header: 61,
                    json: r#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                    padding_len: 7,
                    value: 1.0,
                }),
                expected_resp: Ok(72),
            },
            TestCase {
                name: "buf much too short",
                key: br#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                value: 1.0,
                buf_len: 5,
                expected_entry: None,
                expected_resp: Err(MmapError::Other(format!(
                    "entry length {} larger than slice length {}",
                    80, 5,
                ))),
            },
            TestCase {
                name: "buf short by one",
                key: br#"["metric","name",["label_a","label_b"],["value_a","value_b"]]"#,
                value: 1.0,
                buf_len: 79,
                expected_entry: None,
                expected_resp: Err(MmapError::Other(format!(
                    "entry length {} larger than slice length {}",
                    80, 79,
                ))),
            },
        ];

        for case in tc {
            let mut buf = vec![0; case.buf_len];
            let resp = RawEntry::save(&mut buf, case.key, case.value);

            assert_eq!(
                case.expected_resp, resp,
                "test case: {} - response",
                case.name,
            );

            if let Some(e) = case.expected_entry {
                let expected_buf = e.as_bstring();

                assert_eq!(
                    expected_buf,
                    buf[..expected_buf.len()].as_bstr(),
                    "test case: {} - buffer state",
                    case.name
                );

                let header_len = u32::from_ne_bytes(buf[..size_of::<u32>()].try_into().unwrap());
                assert_eq!(
                    case.key.len(),
                    header_len as usize,
                    "test case: {} - size header",
                    case.name,
                );
            }
        }
    }

    #[test]
    fn test_calc_value_offset() {
        struct TestCase {
            name: &'static str,
            encoded_len: usize,
            expected_value_offset: Option<usize>,
            expected_total_len: Option<usize>,
            expected_err: Option<MmapError>,
        }

        let tc = vec![
            TestCase {
                name: "ok",
                encoded_len: 8,
                expected_value_offset: Some(16),
                expected_total_len: Some(24),
                expected_err: None,
            },
            TestCase {
                name: "padding length one",
                encoded_len: 3,
                expected_value_offset: Some(8),
                expected_total_len: Some(16),
                expected_err: None,
            },
            TestCase {
                name: "padding length eight",
                encoded_len: 4,
                expected_value_offset: Some(16),
                expected_total_len: Some(24),
                expected_err: None,
            },
            TestCase {
                name: "encoded len gt i32::MAX",
                encoded_len: i32::MAX as usize + 1,
                expected_value_offset: None,
                expected_total_len: None,
                expected_err: Some(MmapError::KeyLength),
            },
        ];

        for case in tc {
            let name = case.name;
            if let Some(expected_value_offset) = case.expected_value_offset {
                assert_eq!(
                    expected_value_offset,
                    RawEntry::calc_value_offset(case.encoded_len).unwrap(),
                    "test case: {name} - value offset"
                );
            }

            if let Some(expected_total_len) = case.expected_total_len {
                assert_eq!(
                    expected_total_len,
                    RawEntry::calc_total_len(case.encoded_len).unwrap(),
                    "test case: {name} - total len"
                );
            }

            if let Some(expected_err) = case.expected_err {
                assert_eq!(
                    expected_err,
                    RawEntry::calc_value_offset(case.encoded_len).unwrap_err(),
                    "test case: {name} - err"
                );
            }
        }
    }

    #[test]
    fn test_padding_len() {
        for encoded_len in 0..64 {
            let padding = RawEntry::padding_len(encoded_len);

            // Validate we're actually aligning to 8 bytes.
            assert!((size_of::<u32>() + encoded_len + padding) % 8 == 0)
        }
    }
}
