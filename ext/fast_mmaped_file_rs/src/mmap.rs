use magnus::exception::*;
use magnus::prelude::*;
use magnus::rb_sys::{AsRawValue, FromRawValue};
use magnus::typed_data::Obj;
use magnus::value::Fixnum;
use magnus::{eval, scan_args, Error, Integer, RArray, RClass, RHash, RString, Value};
use nix::libc::{c_char, c_long, c_ulong};
use rb_sys::rb_str_new_static;
use std::fs::File;
use std::io::{prelude::*, SeekFrom};
use std::mem;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::RwLock;

use crate::err;
use crate::error::MmapError;
use crate::file_entry::FileEntry;
use crate::map::EntryMap;
use crate::raw_entry::RawEntry;
use crate::util::{self, CheckedOps};
use crate::Result;
use crate::HEADER_SIZE;
use inner::InnerMmap;

mod inner;

/// The Ruby `STR_NOEMBED` flag, aka `FL_USER1`.
const STR_NOEMBED: c_ulong = 1 << (13);
/// The Ruby `STR_SHARED` flag, aka `FL_USER2`.
const STR_SHARED: c_ulong = 1 << (14);

/// A Rust struct wrapped in a Ruby object, providing access to a memory-mapped
/// file used to store, update, and read out Prometheus metrics.
///
/// - File format:
///     - Header:
///         - 4 bytes: u32 - total size of metrics in file.
///         - 4 bytes: NUL byte padding.
///     - Repeating metrics entries:
///         - 4 bytes: u32 - entry JSON string size.
///         - `N` bytes: UTF-8 encoded JSON string used as entry key.
///         - (8 - (4 + `N`) % 8) bytes: 1 to 8 padding space (0x20) bytes to
///           reach 8-byte alignment.
///         - 8 bytes: f64 - entry value.
///
/// All numbers are saved in native-endian format.
///
/// Generated via [luismartingarcia/protocol](https://github.com/luismartingarcia/protocol):
///
///
/// ```
/// protocol "Used:4,Pad:4,K1 Size:4,K1 Name:4,K1 Value:8,K2 Size:4,K2 Name:4,K2 Value:8"
///
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |  Used |  Pad  |K1 Size|K1 Name|   K1 Value    |K2 Size|K2 Name|
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |  K2 Value   |
/// +-+-+-+-+-+-+-+
/// ```
//
// The API imposed by `magnus` requires all methods to use shared borrows.
// This means we can't store any mutable state in the top-level struct,
// and must store the interior data behind a `RwLock`, which adds run-time
// checks that mutable operations have no concurrent read or writes.
//
// We are further limited by the need to support subclassing in Ruby, which
// requires us to define an allocation function for the class, the
// `magnus::class::define_alloc_func()` function. This needs a support the
// `Default` trait, so a `File` cannot directly help by the object being
// constructed. Having the `RwLock` hold an `Option` of the interior object
// resolves this.
#[derive(Debug, Default)]
#[magnus::wrap(class = "FastMmapedFileRs", free_immediately, size)]
pub struct MmapedFile(RwLock<Option<InnerMmap>>);

impl MmapedFile {
    /// call-seq:
    ///   new(file)
    ///
    /// create a new Mmap object
    ///
    /// * <em>file</em>
    ///
    ///
    ///     Creates a mapping that's shared with all other processes
    ///     mapping the same area of the file.
    pub fn new(klass: RClass, args: &[Value]) -> magnus::error::Result<Obj<Self>> {
        let args = scan_args::scan_args::<(RString,), (), (), (), (), ()>(args)?;
        let path = args.required.0;

        let lock = MmapedFile(RwLock::new(None));
        let obj = Obj::wrap_as(lock, klass);

        let _: Value = obj.funcall("initialize", (path,))?;

        Ok(obj)
    }

    /// Initialize a new `FastMmapedFileRs` object. This must be defined in
    /// order for inheritance to work.
    pub fn initialize(rb_self: Obj<Self>, fname: String) -> magnus::error::Result<()> {
        let file = File::options()
            .read(true)
            .write(true)
            .open(&fname)
            .map_err(|_| err!(arg_error(), "Can't open {}", fname))?;

        let inner = InnerMmap::new(fname.into(), file)?;
        rb_self.insert_inner(inner)?;

        let weak_klass = RClass::from_value(eval("ObjectSpace::WeakMap")?)
            .ok_or_else(|| err!(no_method_error(), "unable to create WeakMap"))?;
        let weak_obj_tracker = weak_klass.new_instance(())?;

        // We will need to iterate over strings backed by the mmapped file, but
        // don't want to prevent the GC from reaping them when the Ruby code
        // has finished with them. `ObjectSpace::WeakMap` allows us to track
        // them without extending their lifetime.
        //
        // https://ruby-doc.org/core-3.0.0/ObjectSpace/WeakMap.html
        rb_self.ivar_set("@weak_obj_tracker", weak_obj_tracker)?;

        Ok(())
    }

    /// Read the list of files provided from Ruby and convert them to a Prometheus
    /// metrics String.
    pub fn to_metrics(file_list: RArray) -> magnus::error::Result<String> {
        let mut map = EntryMap::new();
        map.aggregate_files(file_list)?;

        let sorted = map.into_sorted()?;

        FileEntry::entries_to_string(sorted).map_err(|e| e.into())
    }

    /// Document-method: []
    /// Document-method: slice
    ///
    /// call-seq: [](args)
    ///
    /// Element reference - with the following syntax:
    ///
    ///   self[nth]
    ///
    /// retrieve the <em>nth</em> character
    ///
    ///   self[start..last]
    ///
    /// return a substring from <em>start</em> to <em>last</em>
    ///
    ///   self[start, length]
    ///
    /// return a substring of <em>lenght</em> characters from <em>start</em>
    pub fn slice(rb_self: Obj<Self>, args: &[Value]) -> magnus::error::Result<RString> {
        // The C implementation would trigger a GC cycle via `rb_gc_force_recycle`
        // if the `MM_PROTECT` flag is set, but in practice this is never used.
        // We omit this logic, particularly because `rb_gc_force_recycle` is a
        // no-op as of Ruby 3.1.
        let rs_self = &*rb_self;

        let str = rs_self.str(rb_self)?;
        rs_self._slice(rb_self, str, args)
    }

    fn _slice(
        &self,
        rb_self: Obj<Self>,
        str: RString,
        args: &[Value],
    ) -> magnus::error::Result<RString> {
        let substr: RString = str.funcall("[]", args)?;

        // Track shared child strings which use the same backing storage.
        if Self::rb_string_is_shared(substr) {
            (*rb_self).track_rstring(rb_self, substr)?;
        }

        // The C implementation does this, perhaps to validate that the len we
        // provided is actually being used.
        (*rb_self).inner_mut(|inner| {
            inner.set_len(str.len());
            Ok(())
        })?;

        Ok(substr)
    }

    /// Document-method: msync
    /// Document-method: sync
    /// Document-method: flush
    ///
    /// call-seq: msync
    ///
    /// flush the file
    pub fn sync(&self, args: &[Value]) -> magnus::error::Result<()> {
        use nix::sys::mman::MsFlags;

        let mut ms_async = false;
        let args = scan_args::scan_args::<(), (Option<i32>,), (), (), (), ()>(args)?;

        if let Some(flag) = args.optional.0 {
            let flag = MsFlags::from_bits(flag).unwrap_or(MsFlags::empty());
            ms_async = flag.contains(MsFlags::MS_ASYNC);
        }

        // The `memmap2` crate does not support the `MS_INVALIDATE` flag. We ignore that
        // flag if passed in, checking only for `MS_ASYNC`. In practice no arguments are ever
        // passed to this function, but we do this to maintain compatibility with the
        // C implementation.
        self.inner_mut(|inner| inner.flush(ms_async))
            .map_err(|e| e.into())
    }

    /// Document-method: munmap
    /// Document-method: unmap
    ///
    /// call-seq: munmap
    ///
    /// terminate the association
    pub fn munmap(rb_self: Obj<Self>) -> magnus::error::Result<()> {
        let rs_self = &*rb_self;

        rs_self.inner_mut(|inner| {
            // We are about to release the backing mmap for Ruby's String
            // objects. If Ruby attempts to read from them the program will
            // segfault. We update the length of all Strings to zero so Ruby
            // does not attempt to access the now invalid address between now
            // and when GC eventually reaps the objects.
            //
            // See the following for more detail:
            // https://gitlab.com/gitlab-org/ruby/gems/prometheus-client-mmap/-/issues/39
            // https://gitlab.com/gitlab-org/ruby/gems/prometheus-client-mmap/-/issues/41
            // https://gitlab.com/gitlab-org/ruby/gems/prometheus-client-mmap/-/merge_requests/80
            inner.set_len(0);
            Ok(())
        })?;

        // Update each String object to be zero-length.
        let cap = util::cast_chk::<_, c_long>(rs_self.capacity(), "capacity")?;
        rs_self.update_weak_map(rb_self, rs_self.as_mut_ptr(), cap)?;

        // Remove the `InnerMmap` from the `RwLock`. This will drop
        // end of this function, unmapping and closing the file.
        let _ = rs_self.take_inner()?;
        Ok(())
    }

    /// Fetch the `used` header from the `.db` file, the length
    /// in bytes of the data written to the file.
    pub fn load_used(&self) -> magnus::error::Result<Integer> {
        let used = self.inner(|inner| inner.load_used())?;

        Ok(Integer::from_u64(used as u64))
    }

    /// Update the `used` header for the `.db` file, the length
    /// in bytes of the data written to the file.
    pub fn save_used(rb_self: Obj<Self>, used: Fixnum) -> magnus::error::Result<Fixnum> {
        let rs_self = &*rb_self;
        let used_uint = used.to_u32()?;

        // If the underlying mmap is smaller than the header, then resize to fit.
        // The file has already been expanded to page size when first opened, so
        // even if the map is less than HEADER_SIZE, we're not at risk of a
        // SIGBUS.
        if rs_self.capacity() < HEADER_SIZE {
            rs_self.expand_to_fit(rb_self, HEADER_SIZE)?;
        }

        rs_self.inner_mut(|inner| inner.save_used(used_uint))?;

        Ok(used)
    }

    /// Fetch the value associated with a key from the mmap.
    /// If no entry is present, initialize with the default
    /// value provided.
    pub fn fetch_entry(
        rb_self: Obj<Self>,
        positions: RHash,
        key: RString,
        default_value: f64,
    ) -> magnus::error::Result<f64> {
        let rs_self = &*rb_self;
        let position: Option<Fixnum> = positions.lookup(key)?;

        if let Some(pos) = position {
            let pos = pos.to_usize()?;
            return rs_self
                .inner(|inner| inner.load_value(pos))
                .map_err(|e| e.into());
        }

        rs_self.check_expand(rb_self, key.len())?;

        let value_offset: usize = rs_self.inner_mut(|inner| {
            // SAFETY: We must not call any Ruby code for the lifetime of this borrow.
            unsafe { inner.initialize_entry(key.as_slice(), default_value) }
        })?;

        // CAST: no-op on 64-bit, widening on 32-bit.
        positions.aset(key, Integer::from_u64(value_offset as u64))?;

        rs_self.load_value(value_offset)
    }

    /// Update the value of an existing entry, if present. Otherwise create a new entry
    /// for the key.
    pub fn upsert_entry(
        rb_self: Obj<Self>,
        positions: RHash,
        key: RString,
        value: f64,
    ) -> magnus::error::Result<f64> {
        let rs_self = &*rb_self;
        let position: Option<Fixnum> = positions.lookup(key)?;

        if let Some(pos) = position {
            let pos = pos.to_usize()?;
            return rs_self
                .inner_mut(|inner| {
                    inner.save_value(pos, value)?;

                    // TODO just return `value` here instead of loading it?
                    // This is how the C implementation did it, but I don't
                    // see what the extra load gains us.
                    inner.load_value(pos)
                })
                .map_err(|e| e.into());
        }

        rs_self.check_expand(rb_self, key.len())?;

        let value_offset: usize = rs_self.inner_mut(|inner| {
            // SAFETY: We must not call any Ruby code for the lifetime of this borrow.
            unsafe { inner.initialize_entry(key.as_slice(), value) }
        })?;

        // CAST: no-op on 64-bit, widening on 32-bit.
        positions.aset(key, Integer::from_u64(value_offset as u64))?;

        rs_self.load_value(value_offset)
    }

    /// Creates a Ruby String containing the section of the mmapped file that
    /// has been written to.
    fn str(&self, rb_self: Obj<Self>) -> magnus::error::Result<RString> {
        let val_id = (*rb_self).inner(|inner| {
            let ptr = inner.as_ptr();
            let len = inner.len();

            // SAFETY: This is safe so long as the data provided to Ruby meets its
            // requirements. When unmapping the file this will no longer be the
            // case, see the comment on `munmap` for how we handle this.
            Ok(unsafe { rb_str_new_static(ptr as _, len as _) })
        })?;

        // SAFETY: We know that rb_str_new_static returns a VALUE.
        let val = unsafe { Value::from_raw(val_id) };

        // UNWRAP: We created this value as a string above.
        let str = RString::from_value(val).unwrap();

        // Freeze the root string so it can't be mutated out from under any
        // substrings created. This object is never exposed to callers.
        str.freeze();

        // Track the RString in our `WeakMap` so we can update its address if
        // we re-mmap the backing file.
        (*rb_self).track_rstring(rb_self, str)?;

        Ok(str)
    }

    /// If we reallocate, any live Ruby strings provided by the `str()` method
    /// will be invalidated. We need to iterate over them using and update their
    /// heap pointers to the newly allocated memory region.
    fn update_weak_map(
        &self,
        rb_self: Obj<Self>,
        old_ptr: *const c_char,
        old_cap: c_long,
    ) -> magnus::error::Result<()> {
        let tracker: Value = rb_self.ivar_get("@weak_obj_tracker")?;

        let new_len = self.inner(|inner| util::cast_chk::<_, c_long>(inner.len(), "mmap len"))?;

        // Iterate over the values of the `WeakMap`.
        for val in tracker.enumeratorize("each_value", ()) {
            let rb_string = val?;
            let str = RString::from_value(rb_string)
                .ok_or_else(|| err!(arg_error(), "weakmap value was not a string"))?;

            // SAFETY: We're messing with Ruby's internals here, YOLO.
            unsafe {
                // Convert the magnus wrapper type to a raw string exposed by `rb_sys`,
                // which provides access to its internals.
                let mut raw_str = Self::rb_string_internal(str);

                // Shared string have their own `ptr` and `len` values, but `aux`
                // is the id of the parent string so the GC can track this
                // dependency. The `ptr` will always be an offset from the base
                // address of the mmap, and `len` will be the length of the mmap
                // less the offset from the base.
                if Self::rb_string_is_shared(str) && new_len > 0 {
                    // Calculate how far into the original mmap the shared string
                    // started and update to the equivalent address in the new
                    // one.
                    let substr_ptr = raw_str.as_ref().as_.heap.ptr;
                    let offset = substr_ptr.offset_from(old_ptr);

                    raw_str.as_mut().as_.heap.ptr = self.as_mut_ptr().offset(offset);

                    let current_len = str.len() as c_long;
                    let new_shared_len = old_cap + current_len;

                    self.update_rstring_len(raw_str, new_shared_len);
                    continue;
                }

                // Update the string to point to the new mmapped file.
                // We're matching the behavior of Ruby's `str_new_static` function.
                // See https://github.com/ruby/ruby/blob/e51014f9c05aa65cbf203442d37fef7c12390015/string.c#L1030-L1053
                //
                // We deliberately do _NOT_ increment the `capa` field of the
                // string to match the new `len`. We were initially doing this,
                // but consistently triggered GCs in the middle of updating the
                // string pointers, causing a segfault.
                //
                // See https://gitlab.com/gitlab-org/ruby/gems/prometheus-client-mmap/-/issues/45
                raw_str.as_mut().as_.heap.ptr = self.as_mut_ptr();
                self.update_rstring_len(raw_str, new_len);
            }
        }

        Ok(())
    }

    /// Check that the mmap is large enough to contain the value to be added,
    /// and expand it to fit if necessary.
    fn check_expand(&self, rb_self: Obj<Self>, key_len: usize) -> magnus::error::Result<()> {
        // CAST: no-op on 32-bit, widening on 64-bit.
        let used = self.inner(|inner| inner.load_used())? as usize;
        let entry_len = RawEntry::calc_total_len(key_len)?;

        // We need the mmapped region to contain at least one byte beyond the
        // written data to create a NUL- terminated C string. Validate that
        // new length does not exactly match or exceed the length of the mmap.
        while self.capacity() <= used.add_chk(entry_len)? {
            self.expand_to_fit(rb_self, self.capacity().mul_chk(2)?)?;
        }

        Ok(())
    }

    /// Expand the underlying file until it is long enough to fit `target_cap`.
    /// This will remove the existing mmap, expand the file, then update any
    /// strings held by the `WeakMap` to point to the newly mmapped address.
    fn expand_to_fit(&self, rb_self: Obj<Self>, target_cap: usize) -> magnus::error::Result<()> {
        if target_cap < self.capacity() {
            return Err(err!(arg_error(), "Can't reduce the size of mmap"));
        }

        let mut new_cap = self.capacity();
        while new_cap < target_cap {
            new_cap = new_cap.mul_chk(2)?;
        }

        if new_cap != self.capacity() {
            let old_ptr = self.as_mut_ptr();
            let old_cap = util::cast_chk::<_, c_long>(self.capacity(), "capacity")?;

            // Drop the old mmap.
            let (mut file, path) = self.take_inner()?.munmap();

            self.expand_file(&mut file, &path, target_cap)?;

            // Re-mmap the expanded file.
            let new_inner = InnerMmap::reestablish(path, file, target_cap)?;

            self.insert_inner(new_inner)?;

            return self.update_weak_map(rb_self, old_ptr, old_cap);
        }

        Ok(())
    }

    /// Use lseek(2) to seek past the end of the file and write a NUL byte. This
    /// creates a file hole that expands the size of the file without consuming
    /// disk space until it is actually written to.
    fn expand_file(&self, file: &mut File, path: &Path, len: usize) -> Result<()> {
        if len == 0 {
            return Err(MmapError::overflowed(0, -1, "adding"));
        }

        // CAST: no-op on 64-bit, widening on 32-bit.
        let len = len as u64;

        match file.seek(SeekFrom::Start(len - 1)) {
            Ok(_) => {}
            Err(_) => {
                return Err(MmapError::with_errno(format!("Can't lseek {}", len - 1)));
            }
        }

        match file.write(&[0x0]) {
            Ok(1) => {}
            _ => {
                return Err(MmapError::with_errno(format!(
                    "Can't extend {}",
                    path.display()
                )));
            }
        }

        Ok(())
    }

    fn track_rstring(&self, rb_self: Obj<Self>, str: RString) -> magnus::error::Result<()> {
        let tracker: Value = rb_self.ivar_get("@weak_obj_tracker")?;

        // Use the string's Id as the key in the `WeakMap`.
        let key = str.as_raw();
        let _: Value = tracker.funcall("[]=", (key, str))?;
        Ok(())
    }

    /// The total capacity of the underlying mmap.
    #[inline]
    fn capacity(&self) -> usize {
        // UNWRAP: This is actually infallible, but we need to
        // wrap it in a `Result` for use with `inner()`.
        self.inner(|inner| Ok(inner.capacity())).unwrap()
    }

    fn load_value(&self, position: usize) -> magnus::error::Result<f64> {
        self.inner(|inner| inner.load_value(position))
            .map_err(|e| e.into())
    }

    fn as_mut_ptr(&self) -> *mut c_char {
        // UNWRAP: This is actually infallible, but we need to
        // wrap it in a `Result` for use with `inner()`.
        self.inner(|inner| Ok(inner.as_mut_ptr() as *mut c_char))
            .unwrap()
    }

    /// Takes a closure with immutable access to InnerMmap. Will fail if the inner
    /// object has a mutable borrow or has been dropped.
    fn inner<F, T>(&self, func: F) -> Result<T>
    where
        F: FnOnce(&InnerMmap) -> Result<T>,
    {
        let inner_opt = self.0.try_read().map_err(|_| MmapError::ConcurrentAccess)?;

        let inner = inner_opt.as_ref().ok_or(MmapError::UnmappedFile)?;

        func(inner)
    }

    /// Takes a closure with mutable access to InnerMmap. Will fail if the inner
    /// object has an existing mutable borrow, or has been dropped.
    fn inner_mut<F, T>(&self, func: F) -> Result<T>
    where
        F: FnOnce(&mut InnerMmap) -> Result<T>,
    {
        let mut inner_opt = self
            .0
            .try_write()
            .map_err(|_| MmapError::ConcurrentAccess)?;

        let inner = inner_opt.as_mut().ok_or(MmapError::UnmappedFile)?;

        func(inner)
    }

    /// Take ownership of the `InnerMmap` from the `RwLock`.
    /// Will fail if a mutable borrow is already held or the inner
    /// object has been dropped.
    fn take_inner(&self) -> Result<InnerMmap> {
        let mut inner_opt = self
            .0
            .try_write()
            .map_err(|_| MmapError::ConcurrentAccess)?;
        match (*inner_opt).take() {
            Some(i) => Ok(i),
            None => Err(MmapError::UnmappedFile),
        }
    }

    /// Move `new_inner` into the `RwLock`.
    /// Will return an error if a mutable borrow is already held.
    fn insert_inner(&self, new_inner: InnerMmap) -> Result<()> {
        let mut inner_opt = self
            .0
            .try_write()
            .map_err(|_| MmapError::ConcurrentAccess)?;
        (*inner_opt).replace(new_inner);

        Ok(())
    }

    /// Check if an RString is shared. Shared string use the same underlying
    /// storage as their parent, taking an offset from the start. By default
    /// they must run to the end of the parent string.
    fn rb_string_is_shared(rb_str: RString) -> bool {
        // SAFETY: We only hold a reference to the raw object for the duration
        // of this function, and no Ruby code is called.
        let flags = unsafe {
            let raw_str = Self::rb_string_internal(rb_str);
            raw_str.as_ref().basic.flags
        };
        let shared_flags = STR_SHARED | STR_NOEMBED;

        flags & shared_flags == shared_flags
    }

    /// Convert `magnus::RString` into the raw binding used by `rb_sys::RString`.
    /// We need this to manually change the pointer and length values for strings
    /// when moving the mmap to a new file.
    ///
    /// SAFETY: Calling Ruby code while the returned object is held may result
    /// in it being mutated or dropped.
    unsafe fn rb_string_internal(rb_str: RString) -> NonNull<rb_sys::RString> {
        mem::transmute::<RString, NonNull<rb_sys::RString>>(rb_str)
    }

    #[cfg(ruby_lte_3_2)]
    unsafe fn update_rstring_len(&self, mut raw_str: NonNull<rb_sys::RString>, new_len: c_long) {
        raw_str.as_mut().as_.heap.len = new_len;
    }

    #[cfg(ruby_gte_3_3)]
    unsafe fn update_rstring_len(&self, mut raw_str: NonNull<rb_sys::RString>, new_len: c_long) {
        raw_str.as_mut().len = new_len;
    }
}

#[cfg(test)]
mod test {
    use magnus::error::Error;
    use magnus::eval;
    use magnus::Range;
    use nix::unistd::{sysconf, SysconfVar};
    use std::mem::size_of;

    use super::*;
    use crate::raw_entry::RawEntry;
    use crate::testhelper::TestFile;

    /// Create a wrapped MmapedFile object.
    fn create_obj() -> Obj<MmapedFile> {
        let TestFile {
            file: _file,
            path,
            dir: _dir,
        } = TestFile::new(&[0u8; 8]);

        let path_str = path.display().to_string();
        let rpath = RString::new(&path_str);

        eval!("FastMmapedFileRs.new(path)", path = rpath).unwrap()
    }

    /// Add three entries to the mmap. Expected length is 56, 3x 16-byte
    /// entries with 8-byte header.
    fn populate_entries(rb_self: &Obj<MmapedFile>) -> RHash {
        let positions = RHash::from_value(eval("{}").unwrap()).unwrap();

        MmapedFile::upsert_entry(*rb_self, positions, RString::new("a"), 0.0).unwrap();
        MmapedFile::upsert_entry(*rb_self, positions, RString::new("b"), 1.0).unwrap();
        MmapedFile::upsert_entry(*rb_self, positions, RString::new("c"), 2.0).unwrap();

        positions
    }

    #[test]
    fn test_new() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let TestFile {
            file,
            path,
            dir: _dir,
        } = TestFile::new(&[0u8; 8]);

        let path_str = path.display().to_string();
        let rpath = RString::new(&path_str);

        // Object created successfully
        let result: std::result::Result<Obj<MmapedFile>, Error> =
            eval!("FastMmapedFileRs.new(path)", path = rpath);
        assert!(result.is_ok());

        // Weak map added
        let obj = result.unwrap();
        let weak_tracker: Value = obj.ivar_get("@weak_obj_tracker").unwrap();
        assert_eq!("ObjectSpace::WeakMap", weak_tracker.class().inspect());

        // File expanded to page size
        let page_size = sysconf(SysconfVar::PAGE_SIZE).unwrap().unwrap() as u64;
        let stat = file.metadata().unwrap();
        assert_eq!(page_size, stat.len());

        // Used set to header size
        assert_eq!(
            HEADER_SIZE as u64,
            obj.load_used().unwrap().to_u64().unwrap()
        );
    }

    #[test]
    fn test_slice() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let obj = create_obj();
        let _ = populate_entries(&obj);

        // Validate header updated with new length
        let header_range = Range::new(0, HEADER_SIZE, true).unwrap().as_value();
        let header_slice = MmapedFile::slice(obj, &[header_range]).unwrap();
        assert_eq!([56, 0, 0, 0, 0, 0, 0, 0], unsafe {
            header_slice.as_slice()
        });

        let value_range = Range::new(HEADER_SIZE, 24, true).unwrap().as_value();
        let value_slice = MmapedFile::slice(obj, &[value_range]).unwrap();

        // Validate string length
        assert_eq!(1u32.to_ne_bytes(), unsafe { &value_slice.as_slice()[0..4] });

        // Validate string and padding
        assert_eq!("a   ", unsafe {
            String::from_utf8_lossy(&value_slice.as_slice()[4..8])
        });

        // Validate value
        assert_eq!(0.0f64.to_ne_bytes(), unsafe {
            &value_slice.as_slice()[8..16]
        });
    }

    #[test]
    fn test_slice_resize() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        fn assert_internals(
            obj: Obj<MmapedFile>,
            parent_id: c_ulong,
            child_id: c_ulong,
            unshared_id: c_ulong,
        ) {
            let rs_self = &*obj;
            let tracker: Value = obj.ivar_get("@weak_obj_tracker").unwrap();

            let mmap_ptr = rs_self.as_mut_ptr();
            let mmap_len = rs_self.capacity();

            let mut parent_checked = false;
            let mut child_checked = false;

            for val in tracker.enumeratorize("each_value", ()) {
                let rb_string = val.unwrap();
                let str = RString::from_value(rb_string).unwrap();

                unsafe {
                    let raw_str = MmapedFile::rb_string_internal(str);
                    if str.as_raw() == child_id {
                        assert_eq!(parent_id, raw_str.as_ref().as_.heap.aux.shared);

                        let child_offset = mmap_len as isize - str.len() as isize;
                        assert_eq!(mmap_ptr.offset(child_offset), raw_str.as_ref().as_.heap.ptr);

                        child_checked = true;
                    } else if str.as_raw() == parent_id {
                        assert_eq!(parent_id, str.as_raw());

                        assert_eq!(mmap_ptr, raw_str.as_ref().as_.heap.ptr);
                        assert_eq!(mmap_len as c_long, str.len() as c_long);
                        assert!(raw_str.as_ref().basic.flags & (STR_SHARED | STR_NOEMBED) > 0);
                        assert!(str.is_frozen());

                        parent_checked = true;
                    } else if str.as_raw() == unshared_id {
                        panic!("tracking unshared string");
                    } else {
                        panic!("unknown string");
                    }
                }
            }
            assert!(parent_checked && child_checked);
        }

        let obj = create_obj();
        let _ = populate_entries(&obj);

        let rs_self = &*obj;

        // Create a string containing the full mmap.
        let parent_str = rs_self.str(obj).unwrap();
        let parent_id = parent_str.as_raw();

        // Ruby's shared strings are only created when they go to the end of
        // original string.
        let len = rs_self.inner(|inner| Ok(inner.len())).unwrap();
        let shareable_range = Range::new(1, len - 1, false).unwrap().as_value();

        // This string should re-use the parent's buffer with an offset and have
        // the parent's id in `as.heap.aux.shared`
        let child_str = rs_self._slice(obj, parent_str, &[shareable_range]).unwrap();
        let child_id = child_str.as_raw();

        // A range that does not reach the end of the parent will not be shared.
        assert!(len > 4);
        let unshareable_range = Range::new(0, 4, false).unwrap().as_value();

        // This string should NOT be tracked, it should own its own buffer.
        let unshared_str = rs_self
            ._slice(obj, parent_str, &[unshareable_range])
            .unwrap();
        let unshared_id = unshared_str.as_raw();
        assert!(!MmapedFile::rb_string_is_shared(unshared_str));

        assert_internals(obj, parent_id, child_id, unshared_id);

        let orig_ptr = rs_self.as_mut_ptr();
        // Expand a bunch to ensure we remap
        for _ in 0..16 {
            rs_self.expand_to_fit(obj, rs_self.capacity() * 2).unwrap();
        }
        let new_ptr = rs_self.as_mut_ptr();
        assert!(orig_ptr != new_ptr);

        // If we haven't updated the pointer to the newly remapped file this will segfault.
        let _: Value = eval!("puts parent", parent = parent_str).unwrap();
        let _: Value = eval!("puts child", child = child_str).unwrap();
        let _: Value = eval!("puts unshared", unshared = unshared_str).unwrap();

        // Confirm that tracked strings are still valid.
        assert_internals(obj, parent_id, child_id, unshared_id);
    }

    #[test]
    fn test_dont_fill_mmap() {
        let _cleanup = unsafe { magnus::embed::init() };
        let ruby = magnus::Ruby::get().unwrap();
        crate::init(&ruby).unwrap();

        let obj = create_obj();
        let positions = populate_entries(&obj);

        let rs_self = &*obj;

        rs_self.expand_to_fit(obj, 1024).unwrap();

        let current_used = rs_self.inner(|inner| inner.load_used()).unwrap() as usize;
        let current_cap = rs_self.inner(|inner| Ok(inner.len())).unwrap();

        // Create a new entry that exactly fills the capacity of the mmap.
        let val_len =
            current_cap - current_used - HEADER_SIZE - size_of::<f64>() - size_of::<u32>();
        assert_eq!(
            current_cap,
            RawEntry::calc_total_len(val_len).unwrap() + current_used
        );

        let str = String::from_utf8(vec![b'A'; val_len]).unwrap();
        MmapedFile::upsert_entry(obj, positions, RString::new(&str), 1.0).unwrap();

        // Validate that we have expanded the mmap, ensuring a trailing NUL.
        assert!(rs_self.capacity() > current_cap);
    }
}
