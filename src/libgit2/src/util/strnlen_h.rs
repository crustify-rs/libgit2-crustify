//! `src/util/strnlen.h`: the bounded string length libgit2 names `p_strnlen`, ported to
//! Rust.
//!
//! The header has no translation unit and no state: its one item is native Rust, reached by
//! no C, so the module has no unsafe regions and no exports. The macro stays in the header
//! for the C that still expands it (`alloc.c`, `index.c`, `midx.c`, `refdb_reftable.c`),
//! where it names libc's `strnlen`, as porting's Macros section keeps a header's macro.
#![forbid(unsafe_code)]

use core::ffi::CStr;

/// The length of the string at the start of `bytes`: the index of their first NUL, or
/// `bytes.len()` when they hold none.
///
/// C's `p_strnlen(s, maxlen)` reads at most `maxlen` bytes from `s`; here the pointer and its
/// bound are one slice, so `p_strnlen(s, n)` is `strnlen(&s[..n])`. A C caller may pass a
/// bound past the end of its buffer when a NUL comes first; a Rust caller slices the bound to
/// what it holds, which gives the same length.
///
/// On this build the macro is libc's `strnlen` (`#define p_strnlen strnlen`). The header
/// defines it instead as a `GIT_INLINE` function where libc may lack `strnlen` (`NO_STRNLEN`:
/// MinGW, Solaris, Apple, `MidnightBSD` and MSVC before 2008), which scans for the NUL with
/// `memchr(s, 0, maxlen)` and returns its offset, or `maxlen`. Both give the length this
/// function gives, for every input.
///
/// It computes on its argument alone, so it is a `const fn`. The NUL is found by
/// [`CStr::from_bytes_until_nul`], whose scan is the standard library's `memchr`, word by word
/// at run time, as glibc's `strnlen` and the fallback's `memchr` are.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::strnlen_h::strnlen;
///
/// assert_eq!(strnlen(b"abc\0def"), 3);
/// assert_eq!(strnlen(b"abc"), 3);
/// assert_eq!(strnlen(&b"abcdef"[..2]), 2);
/// assert_eq!(strnlen(b""), 0);
/// ```
///
/// Replaces `mac::p_strnlen`
#[doc(alias = "p_strnlen")]
#[must_use]
#[inline]
pub const fn strnlen(bytes: &[u8]) -> usize {
    match CStr::from_bytes_until_nul(bytes) {
        Ok(string) => string.count_bytes(),
        Err(_) => bytes.len(),
    }
}
