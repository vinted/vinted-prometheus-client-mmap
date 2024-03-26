use magnus::{exception, Ruby};
use std::any;
use std::fmt::Display;
use std::io;
use std::path::Path;
use thiserror::Error;

use crate::util;
use crate::PROM_EPARSING_ERROR;

/// A lightweight representation of Ruby ExceptionClasses.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RubyError {
    Arg,
    Encoding,
    Frozen,
    Index,
    Io,
    NoMem,
    PromParsing,
    Runtime,
    Type,
}

impl From<RubyError> for magnus::ExceptionClass {
    fn from(err: RubyError) -> magnus::ExceptionClass {
        match err {
            RubyError::Arg => exception::arg_error(),
            RubyError::Encoding => exception::encoding_error(),
            RubyError::Frozen => exception::frozen_error(),
            RubyError::Index => exception::index_error(),
            RubyError::Io => exception::io_error(),
            RubyError::NoMem => exception::no_mem_error(),
            RubyError::Runtime => exception::runtime_error(),
            RubyError::PromParsing => {
                // UNWRAP: this will panic if called outside of a Ruby thread.
                let ruby = Ruby::get().unwrap();
                ruby.get_inner(&PROM_EPARSING_ERROR)
            }
            RubyError::Type => exception::type_error(),
        }
    }
}

/// Errors returned internally within the crate. Methods called directly by Ruby return
/// `magnus::error::Error` as do functions that interact heavily with Ruby. This can be
/// converted into a `magnus::error::Error` at the boundary between Rust and Ruby.
#[derive(PartialEq, Eq, Error, Debug)]
pub enum MmapError {
    /// A read or write was made while another thread had mutable access to the mmap.
    #[error("read/write operation attempted while mmap was being written to")]
    ConcurrentAccess,
    /// An error message used to exactly match the messages returned by the C
    /// implementation.
    #[error("{0}")]
    Legacy(String, RubyError),
    /// A String had invalid UTF-8 sequences.
    #[error("{0}")]
    Encoding(String),
    /// A failed attempt to cast an integer from one type to another.
    #[error("failed to cast {object_name} {value} from {from} to {to}")]
    FailedCast {
        from: &'static str,
        to: &'static str,
        value: String,
        object_name: String,
    },
    /// The mmap was frozen when a mutable operation was attempted.
    #[error("mmap")]
    Frozen,
    /// An io operation failed.
    #[error("failed to {operation} path '{path}': {err}")]
    Io {
        operation: String,
        path: String,
        err: String,
    },
    #[error("string length gt {}", i32::MAX)]
    KeyLength,
    /// Failed to allocate memory.
    #[error("Couldn't allocate for {0} memory")]
    OutOfMemory(usize),
    /// A memory operation fell outside of the containers bounds.
    #[error("offset {index} out of bounds of len {len}")]
    OutOfBounds { index: String, len: String },
    /// A numeric operation overflowed.
    #[error("overflow when {op} {value} and {added} of type {ty}")]
    Overflow {
        value: String,
        added: String,
        op: String,
        ty: &'static str,
    },
    /// A miscellaneous error.
    #[error("{0}")]
    Other(String),
    /// A failure when parsing a `.db` file containing Prometheus metrics.
    #[error("{0}")]
    PromParsing(String),
    /// No mmap open.
    #[error("unmapped file")]
    UnmappedFile,
    /// A custom error message with `strerror(3)` appended.
    #[error("{0}")]
    WithErrno(String),
}

impl MmapError {
    pub fn legacy<T: Into<String>>(msg: T, ruby_err: RubyError) -> Self {
        MmapError::Legacy(msg.into(), ruby_err)
    }

    pub fn failed_cast<T: Display, U>(value: T, object_name: &str) -> Self {
        MmapError::FailedCast {
            from: any::type_name::<T>(),
            to: any::type_name::<U>(),
            value: value.to_string(),
            object_name: object_name.to_string(),
        }
    }
    pub fn io(operation: &str, path: &Path, err: io::Error) -> Self {
        MmapError::Io {
            operation: operation.to_string(),
            path: path.display().to_string(),
            err: err.to_string(),
        }
    }

    pub fn overflowed<T: Display>(value: T, added: T, op: &str) -> Self {
        MmapError::Overflow {
            value: value.to_string(),
            added: added.to_string(),
            op: op.to_string(),
            ty: any::type_name::<T>(),
        }
    }

    pub fn out_of_bounds<T: Display>(index: T, len: T) -> Self {      
        MmapError::OutOfBounds {
            index: index.to_string(),
            len: len.to_string(),
        }
    }

    pub fn with_errno<T: Into<String>>(msg: T) -> Self {
        let strerror = util::strerror(util::errno());
        MmapError::WithErrno(format!("{}: ({strerror})", msg.into()))
    }

    pub fn ruby_err(&self) -> RubyError {
        match self {
            MmapError::ConcurrentAccess => RubyError::Arg,
            MmapError::Legacy(_, e) => *e,
            MmapError::Encoding(_) => RubyError::Encoding,
            MmapError::Io { .. } => RubyError::Io,
            MmapError::FailedCast { .. } => RubyError::Arg,
            MmapError::Frozen => RubyError::Frozen,
            MmapError::KeyLength => RubyError::Arg,
            MmapError::Overflow { .. } => RubyError::Arg,
            MmapError::OutOfBounds { .. } => RubyError::Index,
            MmapError::OutOfMemory { .. } => RubyError::NoMem,
            MmapError::Other(_) => RubyError::Arg,
            MmapError::PromParsing(_) => RubyError::PromParsing,
            MmapError::UnmappedFile => RubyError::Io,
            MmapError::WithErrno(_) => RubyError::Io,
        }
    }
}

impl From<MmapError> for magnus::error::Error {
    fn from(err: MmapError) -> magnus::error::Error {
        magnus::error::Error::new(err.ruby_err().into(), err.to_string())
    }
}
