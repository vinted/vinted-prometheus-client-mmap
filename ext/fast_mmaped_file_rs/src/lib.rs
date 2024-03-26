use magnus::exception::*;
use magnus::prelude::*;
use magnus::value::{Fixnum, Lazy, LazyId};
use magnus::{class, define_class, exception, function, method, Ruby};
use std::mem::size_of;

use crate::mmap::MmapedFile;

pub mod error;
pub mod file_entry;
pub mod file_info;
mod macros;
pub mod map;
pub mod mmap;
pub mod raw_entry;
pub mod util;
pub mod exemplars;

pub mod io {
    pub mod prometheus {
        pub mod client {
            include!(concat!(env!("OUT_DIR"), "/io.prometheus.client.rs"));
        }
    }
}

#[cfg(test)]
mod testhelper;

type Result<T> = std::result::Result<T, crate::error::MmapError>;

const MAP_SHARED: i64 = libc::MAP_SHARED as i64;
const HEADER_SIZE: usize = 2 * size_of::<u32>();

static SYM_GAUGE: LazyId = LazyId::new("gauge");
static SYM_MIN: LazyId = LazyId::new("min");
static SYM_MAX: LazyId = LazyId::new("max");
static SYM_LIVESUM: LazyId = LazyId::new("livesum");
static SYM_PID: LazyId = LazyId::new("pid");
static SYM_SAMPLES: LazyId = LazyId::new("samples");

static PROM_EPARSING_ERROR: Lazy<ExceptionClass> = Lazy::new(|_| {
    let prom_err = define_class(
        "PrometheusParsingError",
        exception::runtime_error().as_r_class(),
    )
    .expect("failed to create class `PrometheusParsingError`");
    ExceptionClass::from_value(prom_err.as_value())
        .expect("failed to create exception class from `PrometheusParsingError`")
});

#[magnus::init]
fn init(ruby: &Ruby) -> magnus::error::Result<()> {
    // Initialize the static symbols
    LazyId::force(&SYM_GAUGE, ruby);
    LazyId::force(&SYM_MIN, ruby);
    LazyId::force(&SYM_MAX, ruby);
    LazyId::force(&SYM_LIVESUM, ruby);
    LazyId::force(&SYM_PID, ruby);
    LazyId::force(&SYM_SAMPLES, ruby);

    // Initialize `PrometheusParsingError` class.
    Lazy::force(&PROM_EPARSING_ERROR, ruby);

    let klass = define_class("FastMmapedFileRs", class::object())?;
    klass.undef_default_alloc_func();

    // UNWRAP: We know `MAP_SHARED` fits in a `Fixnum`.
    klass.const_set("MAP_SHARED", Fixnum::from_i64(MAP_SHARED).unwrap())?;

    klass.define_singleton_method("to_metrics", function!(MmapedFile::to_metrics, 1))?;
    klass.define_singleton_method("to_protobuf", function!(MmapedFile::to_protobuf, 1))?;

    // Required for subclassing to work
    klass.define_alloc_func::<MmapedFile>();
    klass.define_singleton_method("new", method!(MmapedFile::new, -1))?;
    klass.define_method("initialize", method!(MmapedFile::initialize, 1))?;
    klass.define_method("slice", method!(MmapedFile::slice, -1))?;
    klass.define_method("sync", method!(MmapedFile::sync, -1))?;
    klass.define_method("munmap", method!(MmapedFile::munmap, 0))?;

    klass.define_method("used", method!(MmapedFile::load_used, 0))?;
    klass.define_method("used=", method!(MmapedFile::save_used, 1))?;
    klass.define_method("fetch_entry", method!(MmapedFile::fetch_entry, 3))?;
    klass.define_method("upsert_entry", method!(MmapedFile::upsert_entry, 3))?;
    klass.define_method("upsert_exemplar", method!(MmapedFile::upsert_exemplar, 5))?;

    Ok(())
}
