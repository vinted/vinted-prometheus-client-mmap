use magnus::exception::*;
use magnus::{Error, RString, Symbol, Value};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Seek};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use crate::err;
use crate::error::{MmapError, RubyError};
use crate::util;
use crate::Result;

/// The details of a `*.db` file.
#[derive(Debug)]
pub struct FileInfo {
    pub file: File,
    pub path: PathBuf,
    pub len: usize,
    pub multiprocess_mode: Symbol,
    pub type_: Symbol,
    pub pid: String,
}

impl FileInfo {
    /// Receive the details of a file from Ruby and store as a `FileInfo`.
    pub fn open_from_params(params: &[Value; 4]) -> magnus::error::Result<Self> {
        if params.len() != 4 {
            return Err(err!(
                arg_error(),
                "wrong number of arguments {} instead of 4",
                params.len()
            ));
        }

        let filepath = RString::from_value(params[0])
            .ok_or_else(|| err!(arg_error(), "can't convert filepath to String"))?;

        // SAFETY: We immediately copy the string buffer from Ruby, preventing
        // it from being mutated out from under us.
        let path_bytes: Vec<_> = unsafe { filepath.as_slice().to_owned() };
        let path = PathBuf::from(OsString::from_vec(path_bytes));

        let mut file = File::open(&path).map_err(|_| {
            err!(
                arg_error(),
                "Can't open {}, errno: {}",
                path.display(),
                util::errno()
            )
        })?;

        let stat = file
            .metadata()
            .map_err(|_| err!(io_error(), "Can't stat file, errno: {}", util::errno()))?;

        let length = util::cast_chk::<_, usize>(stat.len(), "file size")?;

        let multiprocess_mode = Symbol::from_value(params[1])
            .ok_or_else(|| err!(arg_error(), "expected multiprocess_mode to be a symbol"))?;

        let type_ = Symbol::from_value(params[2])
            .ok_or_else(|| err!(arg_error(), "expected file type to be a symbol"))?;

        let pid = RString::from_value(params[3])
            .ok_or_else(|| err!(arg_error(), "expected pid to be a String"))?;

        file.rewind()
            .map_err(|_| err!(io_error(), "Can't fseek 0, errno: {}", util::errno()))?;

        Ok(Self {
            file,
            path,
            len: length,
            multiprocess_mode,
            type_,
            pid: pid.to_string()?,
        })
    }

    /// Read the contents of the associated file into the buffer provided by
    /// the caller.
    pub fn read_from_file(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        buf.clear();
        buf.try_reserve(self.len).map_err(|_| {
            MmapError::legacy(
                format!("Can't malloc {}, errno: {}", self.len, util::errno()),
                RubyError::Io,
            )
        })?;

        match self.file.read_to_end(buf) {
            Ok(n) if n == self.len => Ok(()),
            // A worker may expand the file between our `stat` and `read`, no harm done.
            Ok(n) if n > self.len => {
                self.len = n;
                Ok(())
            }
            Ok(_) => Err(MmapError::io(
                "read",
                &self.path,
                io::Error::from(io::ErrorKind::UnexpectedEof),
            )),
            Err(e) => Err(MmapError::io("read", &self.path, e)),
        }
    }
}

#[cfg(test)]
mod test {
    use magnus::{eval, RArray, Symbol};
    use rand::{thread_rng, Rng};
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::Write;

    use super::*;
    use crate::testhelper::TestFile;

    #[test]
    fn test_open_from_params() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let file_data = b"foobar";
        let TestFile {
            file: _file,
            path,
            dir: _dir,
        } = TestFile::new(file_data);

        let pid = "worker-1_0";
        let args = RArray::from_value(
            eval(&format!("['{}', :max, :gauge, '{pid}']", path.display())).unwrap(),
        )
        .unwrap();
        let arg0 = args.shift().unwrap();
        let arg1 = args.shift().unwrap();
        let arg2 = args.shift().unwrap();
        let arg3 = args.shift().unwrap();

        let out = FileInfo::open_from_params(&[arg0, arg1, arg2, arg3]);
        assert!(out.is_ok());

        let out = out.unwrap();

        assert_eq!(out.path, path);
        assert_eq!(out.len, file_data.len());
        assert_eq!(out.multiprocess_mode, Symbol::new("max"));
        assert_eq!(out.type_, Symbol::new("gauge"));
        assert_eq!(out.pid, pid);
    }

    #[test]
    fn test_read_from_file() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        const BUF_LEN: usize = 1 << 20; // 1MiB

        // Create a buffer with random data.
        let mut buf = vec![0u8; BUF_LEN];
        thread_rng().fill(buf.as_mut_slice());

        let TestFile {
            file,
            path,
            dir: _dir,
        } = TestFile::new(&buf);

        let mut info = FileInfo {
            file,
            path: path.clone(),
            len: buf.len(),
            multiprocess_mode: Symbol::new("puma"),
            type_: Symbol::new("max"),
            pid: "worker-0_0".to_string(),
        };

        let mut out_buf = Vec::new();
        info.read_from_file(&mut out_buf).unwrap();

        assert_eq!(buf.len(), out_buf.len(), "buffer lens");

        let mut in_hasher = Sha256::new();
        in_hasher.update(&buf);
        let in_hash = in_hasher.finalize();

        let mut out_hasher = Sha256::new();
        out_hasher.update(&out_buf);
        let out_hash = out_hasher.finalize();

        assert_eq!(in_hash, out_hash, "content hashes");
    }

    #[test]
    fn test_read_from_file_resized() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        const BUF_LEN: usize = 1 << 14; // 16KiB

        // Create a buffer with random data.
        let mut buf = vec![0u8; BUF_LEN];
        thread_rng().fill(buf.as_mut_slice());

        let TestFile {
            file,
            path,
            dir: _dir,
        } = TestFile::new(&buf);

        let mut info = FileInfo {
            file,
            path: path.clone(),
            len: buf.len(),
            multiprocess_mode: Symbol::new("puma"),
            type_: Symbol::new("max"),
            pid: "worker-0_0".to_string(),
        };

        let mut resized_file = fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(path)
            .unwrap();

        // Write data to file after it has been `stat`ed in the
        // constructor.
        resized_file.write_all(&[1; 1024]).unwrap();

        let mut out_buf = Vec::new();
        info.read_from_file(&mut out_buf).unwrap();

        assert_eq!(BUF_LEN + 1024, info.len, "resized file updated len");
    }
}
