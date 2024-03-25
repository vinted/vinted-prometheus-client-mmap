use nix::errno::Errno;
use nix::libc::c_long;
use std::borrow::Cow;
use std::fmt::Display;
use std::io;
use std::mem::size_of;

use crate::error::MmapError;
use crate::exemplars::{Exemplar, EXEMPLAR_ENTRY_MAX_SIZE_BYTES};
use crate::Result;

/// Wrapper around `checked_add()` that converts failures
/// to `MmapError::Overflow`.
pub trait CheckedOps: Sized {
    fn add_chk(self, rhs: Self) -> Result<Self>;
    fn mul_chk(self, rhs: Self) -> Result<Self>;
}

impl CheckedOps for usize {
    fn add_chk(self, rhs: Self) -> Result<Self> {
        self.checked_add(rhs)
            .ok_or_else(|| MmapError::overflowed(self, rhs, "adding"))
    }

    fn mul_chk(self, rhs: Self) -> Result<Self> {
        self.checked_mul(rhs)
            .ok_or_else(|| MmapError::overflowed(self, rhs, "multiplying"))
    }
}

impl CheckedOps for c_long {
    fn add_chk(self, rhs: Self) -> Result<Self> {
        self.checked_add(rhs)
            .ok_or_else(|| MmapError::overflowed(self, rhs, "adding"))
    }

    fn mul_chk(self, rhs: Self) -> Result<Self> {
        self.checked_mul(rhs)
            .ok_or_else(|| MmapError::overflowed(self, rhs, "multiplying"))
    }
}

/// A wrapper around `TryFrom`, returning `MmapError::FailedCast` on error.
pub fn cast_chk<T, U>(val: T, name: &str) -> Result<U>
where
    T: Copy + Display,
    U: std::convert::TryFrom<T>,
{
    U::try_from(val).map_err(|_| MmapError::failed_cast::<T, U>(val, name))
}

/// Retrieve errno(3).
pub fn errno() -> i32 {
    // UNWRAP: This will always return `Some` when called from `last_os_error()`.
    io::Error::last_os_error().raw_os_error().unwrap()
}

/// Get the error string associated with errno(3).
/// Equivalent to strerror(3).
pub fn strerror(errno: i32) -> &'static str {
    Errno::from_i32(errno).desc()
}

/// Read a `u32` value from a byte slice starting from `offset`.
#[inline]
pub fn read_u32(buf: &[u8], offset: usize) -> Result<u32> {
    if let Some(slice) = buf.get(offset..offset + size_of::<u32>()) {
        // UNWRAP: We can safely unwrap the conversion from slice to array as we
        // the source and targets are constructed here with the same length.
        let out: &[u8; size_of::<u32>()] = slice.try_into().unwrap();

        return Ok(u32::from_ne_bytes(*out));
    }
    Err(MmapError::out_of_bounds(offset, buf.len()))
}

/// Read an `f64` value from a byte slice starting from `offset`.
#[inline]
pub fn read_f64(buf: &[u8], offset: usize) -> Result<f64> {
    if let Some(slice) = buf.get(offset..offset + size_of::<f64>()) {
        // UNWRAP: We can safely unwrap the conversion from slice to array as we
        // can be sure the target array has same length as the source slice.
        let out: &[u8; size_of::<f64>()] = slice.try_into().unwrap();

        return Ok(f64::from_ne_bytes(*out));
    }
    Err(MmapError::out_of_bounds(
        offset + size_of::<f64>(),
        buf.len(),
    ))
}

pub fn read_exemplar(buf: &[u8], offset: usize) -> Result<Exemplar> {
    if let Some(slice) = buf.get(offset..offset + EXEMPLAR_ENTRY_MAX_SIZE_BYTES) {
        // UNWRAP: We can safely unwrap the conversion from slice to array as we
       // can be sure the target array has same length as the source slice.
       let out: &[u8; EXEMPLAR_ENTRY_MAX_SIZE_BYTES] = slice.try_into().expect("failed to convert slice to array");

       let res: Vec<u8> = out.iter().cloned().filter(|&x| x != 0).collect();

        let v: Exemplar = serde_json::from_slice(&res).expect("failed to convert string to Exemplar");
        
        return Ok(v)
    }
    Err(MmapError::out_of_bounds(
        offset + EXEMPLAR_ENTRY_MAX_SIZE_BYTES,
        buf.len(),
    ))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read_u32() {
        let buf = 1u32.to_ne_bytes();

        assert!(matches!(read_u32(&buf, 0), Ok(1)), "index ok");
        assert!(read_u32(&buf, 10).is_err(), "index out of range");
        assert!(
            read_u32(&buf, 1).is_err(),
            "index in range but end out of range"
        );
    }

    #[test]
    fn test_read_f64() {
        let buf = 1.00f64.to_ne_bytes();

        let ok = read_f64(&buf, 0);
        assert!(ok.is_ok());
        assert_eq!(ok.unwrap(), 1.00);

        assert!(read_f64(&buf, 10).is_err(), "index out of range");
        assert!(
            read_f64(&buf, 1).is_err(),
            "index in range but end out of range"
        );
    }
}
