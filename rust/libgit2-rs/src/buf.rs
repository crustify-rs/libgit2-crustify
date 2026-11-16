//! Port of `src/libgit2/buf.c`: `git_buf`, the public output buffer
//! (`include/git2/buffer.h`), and its conversions to and from `git_str`.
//!
//! [`GitBuf`] is the mirrored `git_buf` (kind 3, a public type):
//! `{ char *ptr; size_t reserved; size_t size; }`. libgit2 fills one by
//! moving a `git_str`'s buffer into it, so `reserved` is that buffer's
//! `asize`; the user frees it with `git_buf_dispose`.
//!
//! The safe Rust API returns `Vec<u8>` where C returns a `git_buf`;
//! [`GitBuf::into_vec`] and [`GitBuf::from_str`] convert.

use core::ffi::c_int;

use crate::util::alloc;
use crate::util::errors::{ErrorCode, Result};
use crate::util::str::{GitStr, git_str__initstr, git_str__oom};

/// `git_buf`, mirrored.
///
/// Invariant (for one made in Rust; C's are the caller's contract): `ptr`
/// is `git_str__initstr` or NULL with `reserved == size == 0`, or a
/// `git__allocator` allocation of `reserved > size` bytes holding `size`
/// bytes and a NUL, owned by the buffer. Dropping it frees the allocation
/// (as `git_buf_dispose`).
#[repr(C)]
pub struct GitBuf {
    ptr: *mut core::ffi::c_char,
    reserved: usize,
    size: usize,
}

// SAFETY: the allocation belongs to the buffer alone and `git__allocator`
// frees from any thread; `&GitBuf` only reads.
unsafe impl Send for GitBuf {}
// SAFETY: as above.
unsafe impl Sync for GitBuf {}

impl Default for GitBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GitBuf {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl core::fmt::Debug for GitBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitBuf")
            .field("contents", &crate::util::errors::Bytes(self.as_bytes()))
            .field("reserved", &self.reserved)
            .finish()
    }
}

fn is_sentinel(p: *mut core::ffi::c_char) -> bool {
    p == git_str__initstr.as_ptr() || p == git_str__oom.as_ptr()
}

impl GitBuf {
    /// An empty buffer (`ptr` = `git_str__initstr`).
    pub const fn new() -> GitBuf {
        GitBuf { ptr: git_str__initstr.as_ptr(), reserved: 0, size: 0 }
    }

    /// The contents.
    pub fn as_bytes(&self) -> &[u8] {
        if self.ptr.is_null() || self.size == 0 || is_sentinel(self.ptr) {
            return &[];
        }
        // SAFETY: by the invariant, `ptr` holds `size` initialized bytes,
        // which only the buffer's owner writes.
        unsafe { core::slice::from_raw_parts(self.ptr.cast::<u8>(), self.size) }
    }

    /// A copy of the contents.
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    /// The contents, as a `Vec` (a copy; the buffer is freed).
    pub fn into_vec(self) -> Vec<u8> {
        self.to_vec()
    }

    /// `git_buf_fromstr` for Rust: move `s`'s buffer into a `git_buf`. A
    /// borrowed string's contents are copied first; an out-of-memory one
    /// fails.
    pub fn from_str(mut s: GitStr) -> Result<GitBuf> {
        if s.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        if !s.is_allocated() && !s.is_empty() {
            s = GitStr::from_bytes(s.as_bytes())?;
        }
        let mut out = GitBuf::new();
        fromstr(&mut out, &mut s);
        Ok(out)
    }

    /// `git_buf_dispose`.
    pub fn dispose(&mut self) {
        if !is_sentinel(self.ptr) {
            // SAFETY: by the invariant (or C's contract), a non-sentinel
            // `ptr` is NULL or an allocation of `git__allocator` owned by
            // the buffer, freed once here.
            unsafe { alloc::free(self.ptr.cast()) };
        }
        self.ptr = git_str__initstr.as_ptr();
        self.reserved = 0;
        self.size = 0;
    }

    /// `git_buf_sanitize`: empty the buffer, keeping its allocation.
    pub fn sanitize(&mut self) {
        if self.reserved > 0 && !self.ptr.is_null() && !is_sentinel(self.ptr) {
            // SAFETY: a buffer with `reserved > 0` owns an allocation of
            // `reserved` bytes.
            unsafe { self.ptr.write(0) };
        } else {
            self.ptr = git_str__initstr.as_ptr();
        }
        self.size = 0;
    }
}

/// Move `s`'s buffer into `out` (whose old buffer is not freed, as in C),
/// leaving `s` empty.
fn fromstr(out: &mut GitBuf, s: &mut GitStr) {
    let (ptr, asize, size) = s.take_raw();
    out.ptr = ptr;
    out.reserved = asize;
    out.size = size;
}

/// The exports of `buf.c`.
pub mod ffi {
    use super::*;
    use crate::git_assert_arg;
    use crate::util::export::barrier;

    /// `git_buf_sanitize`.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_buf` that libgit2 filled (or
    /// `GIT_BUF_INIT`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_buf_sanitize(buf: *mut GitBuf) -> c_int {
        barrier(|| {
            git_assert_arg!(!buf.is_null(), "buf", -1);
            // SAFETY: a valid `git_buf`, not otherwise accessed.
            unsafe { &mut *buf }.sanitize();
            0
        })
    }

    /// `git_buf_tostr`: move `buf`'s allocation (emptied) into `out`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable (its old contents are overwritten, not
    /// freed); `buf` is NULL or a valid `git_buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_buf_tostr(out: *mut GitStr, buf: *mut GitBuf) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!buf.is_null(), "buf", -1);
            // SAFETY: a valid `git_buf`, not otherwise accessed.
            let b = unsafe { &mut *buf };
            b.sanitize();
            // SAFETY: `b`'s allocation (with `reserved` bytes, now holding
            // an empty string) becomes the string's; `out` is writable.
            unsafe { out.write(GitStr::from_raw_parts(b.ptr, b.reserved, b.size)) };
            b.ptr = git_str__initstr.as_ptr();
            b.reserved = 0;
            b.size = 0;
            0
        })
    }

    /// `git_buf_fromstr`: move `str`'s buffer into `out`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable (its old contents are overwritten, not
    /// freed); `str` is NULL or a valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_buf_fromstr(out: *mut GitBuf, str: *mut GitStr) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!str.is_null(), "str", -1);
            // SAFETY: a valid `git_str`, not otherwise accessed.
            let (ptr, reserved, size) = unsafe { &mut *str }.take_raw();
            // SAFETY: `out` is writable.
            unsafe { out.write(GitBuf { ptr, reserved, size }) };
            0
        })
    }

    /// `git_buf_dispose` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_buf` that libgit2 filled (or
    /// `GIT_BUF_INIT`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_buf_dispose(buf: *mut GitBuf) {
        barrier(|| {
            if !buf.is_null() {
                // SAFETY: a valid `git_buf`, not otherwise accessed.
                unsafe { &mut *buf }.dispose()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_round_trip() {
        let _g = crate::util::test_support::init();
        let s = GitStr::from_bytes(b"hello").unwrap();
        let cap = s.capacity();
        let b = GitBuf::from_str(s).unwrap();
        assert_eq!(b.as_bytes(), b"hello");
        assert_eq!(b.reserved, cap);
        let mut b2 = GitBuf::new();
        // SAFETY: the export on valid buffers, as C calls it.
        unsafe {
            let mut out = core::mem::MaybeUninit::<GitStr>::uninit();
            assert_eq!(ffi::git_buf_tostr(out.as_mut_ptr(), &mut b2), 0);
            let out = out.assume_init();
            assert!(out.is_empty());
        }
        assert_eq!(b.into_vec(), b"hello");
        // A borrowed string is copied.
        let data = *b"xyz";
        let mut s = GitStr::new();
        // SAFETY: `data` outlives `s`'s use of it (it is copied at once).
        unsafe { s.attach_notowned(data.as_ptr().cast(), 3) };
        let b = GitBuf::from_str(s).unwrap();
        assert_eq!(b.as_bytes(), b"xyz");
        assert_ne!(b.ptr.cast_const(), data.as_ptr().cast());
    }
}
