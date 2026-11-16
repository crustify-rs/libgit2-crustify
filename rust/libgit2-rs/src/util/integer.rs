//! Rust equivalents of `src/util/integer.h`: range checks, and the
//! overflow-checked size arithmetic of `GIT_ERROR_CHECK_ALLOC_ADD` and
//! `GIT_ERROR_CHECK_ALLOC_MULTIPLY`, which fail with the out-of-memory
//! error.

use super::errors::{self, Result};

/// `git__is_sizet`.
#[inline]
pub const fn is_sizet(p: i64) -> bool {
    p >= 0 && (p as u64) <= usize::MAX as u64
}

/// `git__is_ssizet`.
#[inline]
pub const fn is_ssizet(p: usize) -> bool {
    p <= isize::MAX as usize
}

/// `git__is_uint16`.
#[inline]
pub const fn is_uint16(p: usize) -> bool {
    p <= u16::MAX as usize
}

/// `git__is_uint32`.
#[inline]
pub const fn is_uint32(p: usize) -> bool {
    p <= u32::MAX as usize
}

/// `git__is_ulong` (`unsigned long` is 64 bits on the port's platforms).
#[inline]
pub const fn is_ulong(p: i64) -> bool {
    p >= 0
}

/// `git__is_int`.
#[inline]
pub const fn is_int(p: i64) -> bool {
    p >= i32::MIN as i64 && p <= i32::MAX as i64
}

/// `GIT_ERROR_CHECK_ALLOC_ADD`: `one + two`, or the out-of-memory error on
/// overflow.
#[inline]
pub fn alloc_add(one: usize, two: usize) -> Result<usize> {
    match one.checked_add(two) {
        Some(v) => Ok(v),
        None => Err(errors::oom()),
    }
}

/// `GIT_ERROR_CHECK_ALLOC_ADD3`.
#[inline]
pub fn alloc_add3(one: usize, two: usize, three: usize) -> Result<usize> {
    alloc_add(alloc_add(one, two)?, three)
}

/// `GIT_ERROR_CHECK_ALLOC_ADD4`.
#[inline]
pub fn alloc_add4(one: usize, two: usize, three: usize, four: usize) -> Result<usize> {
    alloc_add(alloc_add3(one, two, three)?, four)
}

/// `GIT_ERROR_CHECK_ALLOC_MULTIPLY`: `nelem * elsize`, or the out-of-memory
/// error on overflow.
#[inline]
pub fn alloc_multiply(nelem: usize, elsize: usize) -> Result<usize> {
    match nelem.checked_mul(elsize) {
        Some(v) => Ok(v),
        None => Err(errors::oom()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert!(is_sizet(0) && !is_sizet(-1));
        assert!(is_int(i32::MIN as i64) && !is_int(i32::MAX as i64 + 1));
        assert!(is_uint16(65535) && !is_uint16(65536));
        assert!(is_ssizet(isize::MAX as usize) && !is_ssizet(usize::MAX));
    }

    #[test]
    fn overflow_sets_oom() {
        let _g = crate::util::test_support::init();
        assert_eq!(alloc_add(1, 2), Ok(3));
        assert!(alloc_add(usize::MAX, 1).is_err());
        assert_eq!(errors::last_class(), Some(errors::ErrorClass::NOMEMORY));
        assert!(alloc_multiply(usize::MAX, 2).is_err());
    }
}
