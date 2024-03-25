use libc::off_t;
use memmap2::{MmapMut, MmapOptions};
use nix::libc::c_long;
use std::fs::File;
use std::mem::size_of;
use std::ops::Range;
use std::os::unix::prelude::{AsRawFd, RawFd};
use std::path::PathBuf;

use crate::error::{MmapError, RubyError};
use crate::raw_entry::RawEntry;
use crate::util::CheckedOps;
use crate::util::{self, errno, read_f64, read_u32};
use crate::Result;
use crate::HEADER_SIZE;

/// A mmapped file and its metadata. Ruby never directly interfaces
/// with this struct.
#[derive(Debug)]
pub(super) struct InnerMmap {
    /// The handle of the file being mmapped. When resizing the
    /// file we must drop the `InnerMmap` while keeping this open,
    /// truncate/extend the file, and establish a new `InnerMmap` to
    /// re-map it.
    file: File,
    /// The path of the file.
    path: PathBuf,
    /// The mmap itself. When initializing a new entry the length of
    /// the mmap is used for bounds checking.
    map: MmapMut,
    /// The length of data written to the file, used to validate
    /// whether a `load/save_value` call is in bounds and the length
    /// we truncate the file to when unmapping.
    ///
    /// Equivalent to `i_mm->t->real` in the C implementation.
    len: usize,
}

impl InnerMmap {
    /// Constructs a new `InnerMmap`, mmapping `path`.
    /// Use when mmapping a file for the first time. When re-mapping a file
    /// after expanding it the `reestablish` function should be used.
    pub fn new(path: PathBuf, file: File) -> Result<Self> {
        let stat = file.metadata().map_err(|e| {
            MmapError::legacy(
                format!("Can't stat {}: {e}", path.display()),
                RubyError::Arg,
            )
        })?;

        let file_size = util::cast_chk::<_, usize>(stat.len(), "file length")?;

        // We need to ensure the underlying file descriptor is at least a page size.
        // Otherwise, we could get a SIGBUS error if mmap() attempts to read or write
        // past the file.
        let reserve_size = Self::next_page_boundary(file_size)?;

        // Cast: no-op.
        Self::reserve_mmap_file_bytes(file.as_raw_fd(), reserve_size as off_t).map_err(|e| {
            MmapError::legacy(
                format!(
                    "Can't reserve {reserve_size} bytes for memory-mapped file in {}: {e}",
                    path.display()
                ),
                RubyError::Io,
            )
        })?;

        // Ensure we always have space for the header.
        let map_len = file_size.max(HEADER_SIZE);

        // SAFETY: There is the possibility of UB if the file is modified outside of
        // this program.
        let map = unsafe { MmapOptions::new().len(map_len).map_mut(&file) }.map_err(|e| {
            MmapError::legacy(format!("mmap failed ({}): {e}", errno()), RubyError::Arg)
        })?;

        let len = file_size;

        Ok(Self {
            file,
            path,
            map,
            len,
        })
    }

    /// Re-mmap a file that was previously mapped.
    pub fn reestablish(path: PathBuf, file: File, map_len: usize) -> Result<Self> {
        // SAFETY: There is the possibility of UB if the file is modified outside of
        // this program.
        let map = unsafe { MmapOptions::new().len(map_len).map_mut(&file) }.map_err(|e| {
            MmapError::legacy(format!("mmap failed ({}): {e}", errno()), RubyError::Arg)
        })?;

        // TODO should we keep this as the old len? We'd want to be able to truncate
        // to the old length at this point if closing the file. Matching C implementation
        // for now.
        let len = map_len;

        Ok(Self {
            file,
            path,
            map,
            len,
        })
    }

    /// Add a new metrics entry to the end of the mmap. This will fail if the mmap is at
    /// capacity. Callers must expand the file first.
    ///
    /// SAFETY: Must not call any Ruby code for the lifetime of `key`, otherwise we risk
    /// Ruby mutating the underlying `RString`.
    pub unsafe fn initialize_entry(&mut self, key: &[u8], value: f64) -> Result<usize> {
        // CAST: no-op on 32-bit, widening on 64-bit.
        let current_used = self.load_used()? as usize;
        let entry_length = RawEntry::calc_total_len(key.len())?;

        let new_used = current_used.add_chk(entry_length)?;

        // Increasing capacity requires expanding the file and re-mmapping it, we can't
        // perform this from `InnerMmap`.
        if self.capacity() < new_used {
            return Err(MmapError::Other(format!(
                "mmap capacity {} less than {}",
                self.capacity(),
                new_used
            )));
        }

        let bytes = self.map.as_mut();
        let value_offset = RawEntry::save(&mut bytes[current_used..new_used], key, value)?;

        // Won't overflow as value_offset is less than new_used.
        let position = current_used + value_offset;
        let new_used32 = util::cast_chk::<_, u32>(new_used, "used")?;

        self.save_used(new_used32)?;
        Ok(position)
    }

    /// Save a metrics value to an existing entry in the mmap.
    pub fn save_value(&mut self, offset: usize, value: f64) -> Result<()> {
        if self.len.add_chk(size_of::<f64>())? <= offset {
            return Err(MmapError::out_of_bounds(
                offset + size_of::<f64>(),
                self.len,
            ));
        }

        if offset < HEADER_SIZE {
            return Err(MmapError::Other(format!(
                "writing to offset {offset} would overwrite file header"
            )));
        }

        let value_bytes = value.to_ne_bytes();
        let value_range = self.item_range(offset, value_bytes.len())?;

        let bytes = self.map.as_mut();
        bytes[value_range].copy_from_slice(&value_bytes);

        Ok(())
    }

    /// Load a metrics value from an entry in the mmap.
    pub fn load_value(&self, offset: usize) -> Result<f64> {
        if self.len.add_chk(size_of::<f64>())? <= offset {
            return Err(MmapError::out_of_bounds(
                offset + size_of::<f64>(),
                self.len,
            ));
        }
        read_f64(self.map.as_ref(), offset)
    }

    /// The length of data written to the file.
    /// With a new file this is only set when Ruby calls `slice` on
    /// `FastMmapedFileRs`, so even if data has been written to the
    /// mmap attempts to read will fail until a String is created.
    /// When an existing file is read we set this value immediately.
    ///
    /// Equivalent to `i_mm->t->real` in the C implementation.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// The total length in bytes of the mmapped file.
    ///
    /// Equivalent to `i_mm->t->len` in the C implementation.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.map.len()
    }

    /// Update the length of the mmap considered to be written.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
    }

    /// Returns a raw pointer to the mmap.
    pub fn as_ptr(&self) -> *const u8 {
        self.map.as_ptr()
    }

    /// Returns a mutable raw pointer to the mmap.
    /// For use in updating RString internals which requires a mutable pointer.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.map.as_ptr().cast_mut()
    }

    /// Perform an msync(2) on the mmap, flushing all changes written
    /// to disk. The sync may optionally be performed asynchronously.
    pub fn flush(&mut self, f_async: bool) -> Result<()> {
        if f_async {
            self.map
                .flush_async()
                .map_err(|_| MmapError::legacy(format!("msync({})", errno()), RubyError::Arg))
        } else {
            self.map
                .flush()
                .map_err(|_| MmapError::legacy(format!("msync({})", errno()), RubyError::Arg))
        }
    }

    /// Load the `used` header containing the size of the metrics data written.
    pub fn load_used(&self) -> Result<u32> {
        match read_u32(self.map.as_ref(), 0) {
            // CAST: we know HEADER_SIZE fits in a u32.
            Ok(0) => Ok(HEADER_SIZE as u32),
            u => u,
        }
    }

    /// Update the `used` header to the value provided.
    /// value provided.
    pub fn save_used(&mut self, used: u32) -> Result<()> {
        let bytes = self.map.as_mut();
        bytes[..size_of::<u32>()].copy_from_slice(&used.to_ne_bytes());

        Ok(())
    }

    /// Drop self, which performs an munmap(2) on the mmap,
    /// returning the open `File` and `PathBuf` so the
    /// caller can expand the file and re-mmap it.
    pub fn munmap(self) -> (File, PathBuf) {
        (self.file, self.path)
    }

    // From https://stackoverflow.com/a/22820221: The difference with
    // ftruncate(2) is that (on file systems supporting it, e.g. Ext4)
    // disk space is indeed reserved by posix_fallocate but ftruncate
    // extends the file by adding holes (and without reserving disk
    // space).
    #[cfg(target_os = "linux")]
    fn reserve_mmap_file_bytes(fd: RawFd, len: off_t) -> nix::Result<()> {
        nix::fcntl::posix_fallocate(fd, 0, len)
    }

    // We simplify the reference implementation since we generally
    // don't need to reserve more than a page size.
    #[cfg(not(target_os = "linux"))]
    fn reserve_mmap_file_bytes(fd: RawFd, len: off_t) -> nix::Result<()> {
        nix::unistd::ftruncate(fd, len)
    }

    fn item_range(&self, start: usize, len: usize) -> Result<Range<usize>> {
        let offset_end = start.add_chk(len)?;

        if offset_end >= self.capacity() {
            return Err(MmapError::out_of_bounds(offset_end, self.capacity()));
        }

        Ok(start..offset_end)
    }

    fn next_page_boundary(len: usize) -> Result<c_long> {
        use nix::unistd::{self, SysconfVar};

        let len = c_long::try_from(len)
            .map_err(|_| MmapError::failed_cast::<_, c_long>(len, "file len"))?;

        let mut page_size = match unistd::sysconf(SysconfVar::PAGE_SIZE) {
            Ok(Some(p)) if p > 0 => p,
            Ok(Some(p)) => {
                return Err(MmapError::legacy(
                    format!("Invalid page size {p}"),
                    RubyError::Io,
                ))
            }
            Ok(None) => {
                return Err(MmapError::legacy(
                    "No system page size found",
                    RubyError::Io,
                ))
            }
            Err(_) => {
                return Err(MmapError::legacy(
                    "Failed to get system page size: {e}",
                    RubyError::Io,
                ))
            }
        };

        while page_size < len {
            page_size = page_size.mul_chk(2)?;
        }

        Ok(page_size)
    }
}

#[cfg(test)]
mod test {
    use nix::unistd::{self, SysconfVar};

    use super::*;
    use crate::testhelper::{self, TestEntry, TestFile};
    use crate::HEADER_SIZE;

    #[test]
    fn test_new() {
        struct TestCase {
            name: &'static str,
            existing: bool,
            expected_len: usize,
        }

        let page_size = unistd::sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap();

        let json = r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#;
        let value = 1.0;
        let entry_len = TestEntry::new(json, value).as_bytes().len();

        let tc = vec![
            TestCase {
                name: "empty file",
                existing: false,
                expected_len: 0,
            },
            TestCase {
                name: "existing file",
                existing: true,
                expected_len: HEADER_SIZE + entry_len,
            },
        ];

        for case in tc {
            let name = case.name;

            let data = match case.existing {
                true => testhelper::entries_to_db(&[json], &[1.0], None),
                false => Vec::new(),
            };

            let TestFile {
                file: original_file,
                path,
                dir: _dir,
            } = TestFile::new(&data);

            let original_stat = original_file.metadata().unwrap();

            let inner = InnerMmap::new(path.clone(), original_file).unwrap();

            let updated_file = File::open(&path).unwrap();
            let updated_stat = updated_file.metadata().unwrap();

            assert!(
                updated_stat.len() > original_stat.len(),
                "test case: {name} - file has been extended"
            );

            assert_eq!(
                updated_stat.len(),
                page_size as u64,
                "test case: {name} - file extended to page size"
            );

            assert_eq!(
                inner.capacity() as u64,
                original_stat.len().max(HEADER_SIZE as u64),
                "test case: {name} - mmap capacity matches original file len, unless smaller than HEADER_SIZE"
            );

            assert_eq!(
                case.expected_len,
                inner.len(),
                "test case: {name} - len set"
            );
        }
    }

    #[test]
    fn test_reestablish() {
        struct TestCase {
            name: &'static str,
            target_len: usize,
            expected_len: usize,
        }

        let json = r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#;

        let tc = vec![TestCase {
            name: "ok",
            target_len: 4096,
            expected_len: 4096,
        }];

        for case in tc {
            let name = case.name;

            let data = testhelper::entries_to_db(&[json], &[1.0], None);

            let TestFile {
                file: original_file,
                path,
                dir: _dir,
            } = TestFile::new(&data);

            let inner =
                InnerMmap::reestablish(path.clone(), original_file, case.target_len).unwrap();

            assert_eq!(
                case.target_len,
                inner.capacity(),
                "test case: {name} - mmap capacity set to target len",
            );

            assert_eq!(
                case.expected_len,
                inner.len(),
                "test case: {name} - len set"
            );
        }
    }

    #[test]
    fn test_initialize_entry() {
        struct TestCase {
            name: &'static str,
            empty: bool,
            used: Option<u32>,
            expected_used: Option<u32>,
            expected_value_offset: Option<usize>,
            expected_err: Option<MmapError>,
        }

        let json = r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#;
        let value = 1.0;
        let entry_len = TestEntry::new(json, value).as_bytes().len();

        let tc = vec![
            TestCase {
                name: "empty file, not expanded by outer mmap",
                empty: true,
                used: None,
                expected_used: None,
                expected_value_offset: None,
                expected_err: Some(MmapError::Other(format!(
                    "mmap capacity {HEADER_SIZE} less than {}",
                    entry_len + HEADER_SIZE,
                ))),
            },
            TestCase {
                name: "data in file",
                empty: false,
                used: None,
                expected_used: Some(HEADER_SIZE as u32 + (entry_len * 2) as u32),
                expected_value_offset: Some(176),
                expected_err: None,
            },
            TestCase {
                name: "data in file, invalid used larger than file",
                empty: false,
                used: Some(10_000),
                expected_used: None,
                expected_value_offset: None,
                expected_err: Some(MmapError::Other(format!(
                    "mmap capacity 4096 less than {}",
                    10_000 + entry_len
                ))),
            },
        ];

        for case in tc {
            let name = case.name;

            let data = match case.empty {
                true => Vec::new(),
                false => testhelper::entries_to_db(&[json], &[1.0], case.used),
            };

            let TestFile {
                file,
                path,
                dir: _dir,
            } = TestFile::new(&data);

            if !case.empty {
                // Ensure the file is large enough to have additional entries added.
                // Normally the outer mmap handles this.
                file.set_len(4096).unwrap();
            }
            let mut inner = InnerMmap::new(path, file).unwrap();

            let result = unsafe { inner.initialize_entry(json.as_bytes(), value) };

            if let Some(expected_used) = case.expected_used {
                assert_eq!(
                    expected_used,
                    inner.load_used().unwrap(),
                    "test case: {name} - used"
                );
            }

            if let Some(expected_value_offset) = case.expected_value_offset {
                assert_eq!(
                    expected_value_offset,
                    *result.as_ref().unwrap(),
                    "test case: {name} - value_offset"
                );
            }

            if let Some(expected_err) = case.expected_err {
                assert_eq!(
                    expected_err,
                    result.unwrap_err(),
                    "test case: {name} - error"
                );
            }
        }
    }

    #[test]
    fn test_save_value() {
        let json = r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#;
        let value = 1.0;
        let upper_bound = TestEntry::new(json, value).as_bytes().len() + HEADER_SIZE;
        let value_offset = upper_bound - size_of::<f64>();

        struct TestCase {
            name: &'static str,
            empty: bool,
            len: Option<usize>,
            offset: usize,
            expected_err: Option<MmapError>,
        }

        let tc = vec![
            TestCase {
                name: "existing file, in bounds",
                empty: false,
                len: None,
                offset: upper_bound - size_of::<f64>() - 1,
                expected_err: None,
            },
            TestCase {
                name: "existing file, out of bounds",
                empty: false,
                len: Some(100),
                offset: upper_bound * 2,
                expected_err: Some(MmapError::out_of_bounds(
                    upper_bound * 2 + size_of::<f64>(),
                    100,
                )),
            },
            TestCase {
                name: "existing file, off by one",
                empty: false,
                len: None,
                offset: value_offset + 1,
                expected_err: Some(MmapError::out_of_bounds(
                    value_offset + 1 + size_of::<f64>(),
                    upper_bound,
                )),
            },
            TestCase {
                name: "empty file cannot be saved to",
                empty: true,
                len: None,
                offset: 8,
                expected_err: Some(MmapError::out_of_bounds(8 + size_of::<f64>(), 0)),
            },
            TestCase {
                name: "overwrite header",
                empty: false,
                len: None,
                offset: 7,
                expected_err: Some(MmapError::Other(
                    "writing to offset 7 would overwrite file header".to_string(),
                )),
            },
        ];

        for case in tc {
            let name = case.name;

            let mut data = match case.empty {
                true => Vec::new(),
                false => testhelper::entries_to_db(&[json], &[1.0], None),
            };

            if let Some(len) = case.len {
                // Pad input to desired length.
                data.append(&mut vec![0xff; len - upper_bound]);
            }

            let TestFile {
                file,
                path,
                dir: _dir,
            } = TestFile::new(&data);

            let mut inner = InnerMmap::new(path, file).unwrap();

            let result = inner.save_value(case.offset, value);

            if let Some(expected_err) = case.expected_err {
                assert_eq!(
                    expected_err,
                    result.unwrap_err(),
                    "test case: {name} - expected err"
                );
            } else {
                assert!(result.is_ok(), "test case: {name} - success");

                assert_eq!(
                    value,
                    util::read_f64(&inner.map, case.offset).unwrap(),
                    "test case: {name} - value saved"
                );
            }
        }
    }

    #[test]
    fn test_load_value() {
        let json = r#"["first_family","first_name",["label_a","label_b"],["value_a","value_b"]]"#;
        let value = 1.0;
        let total_len = TestEntry::new(json, value).as_bytes().len() + HEADER_SIZE;
        let value_offset = total_len - size_of::<f64>();

        struct TestCase {
            name: &'static str,
            offset: usize,
            expected_err: Option<MmapError>,
        }

        let tc = vec![
            TestCase {
                name: "in bounds",
                offset: value_offset,
                expected_err: None,
            },
            TestCase {
                name: "out of bounds",
                offset: value_offset * 2,
                expected_err: Some(MmapError::out_of_bounds(
                    value_offset * 2 + size_of::<f64>(),
                    total_len,
                )),
            },
            TestCase {
                name: "off by one",
                offset: value_offset + 1,
                expected_err: Some(MmapError::out_of_bounds(
                    value_offset + 1 + size_of::<f64>(),
                    total_len,
                )),
            },
        ];

        for case in tc {
            let name = case.name;

            let data = testhelper::entries_to_db(&[json], &[1.0], None);

            let TestFile {
                file,
                path,
                dir: _dir,
            } = TestFile::new(&data);

            let inner = InnerMmap::new(path, file).unwrap();

            let result = inner.load_value(case.offset);

            if let Some(expected_err) = case.expected_err {
                assert_eq!(
                    expected_err,
                    result.unwrap_err(),
                    "test case: {name} - expected err"
                );
            } else {
                assert!(result.is_ok(), "test case: {name} - success");

                assert_eq!(value, result.unwrap(), "test case: {name} - value loaded");
            }
        }
    }
}
