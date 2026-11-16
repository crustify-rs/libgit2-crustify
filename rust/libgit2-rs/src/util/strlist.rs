//! Port of `src/util/strlist.c`: arrays of C strings (`char **`), as the C
//! that stays uses them. Rust code uses `Vec`s; this module has the C's
//! helpers and their exports.

use core::ffi::CStr;

use super::util::strncmp;

/// `git_strlist_contains_prefix`: whether `strncmp(s, str, n) == 0` for
/// one of `strings` (each read up to its NUL, as `strncmp` does).
pub fn contains_prefix(strings: &[&CStr], s: &[u8], n: usize) -> bool {
    strings.iter().any(|x| strncmp(x.to_bytes(), s, n) == 0)
}

/// `git_strlist_contains_key`: whether one of `strings` starts with the
/// part of `key` before the first `delimiter`; false if `key` has none.
pub fn contains_key(strings: &[&CStr], key: &[u8], delimiter: u8) -> bool {
    let key = match memchr::memchr(0, key) {
        Some(n) => &key[..n],
        None => key,
    };
    match key.iter().position(|&c| c == delimiter) {
        // C's loop stops at the delimiter or the NUL; a NUL delimiter is
        // the end of the string, which means "none".
        Some(n) if delimiter != 0 => contains_prefix(strings, key, n),
        _ => false,
    }
}

/// The exports of `strlist.c`.
pub mod ffi {
    use super::*;
    use crate::util::alloc;
    use crate::util::errors::{self, ErrorCode};
    use crate::util::export::{barrier, cstr_bytes};
    use crate::util::integer::alloc_add;
    use core::ffi::{CStr, c_char, c_int};

    /// Copy `len` strings into a new `git__calloc`'d array of `size`
    /// slots; on failure, free what was copied (C leaks it) and set the
    /// out-of-memory error.
    ///
    /// # Safety
    ///
    /// `input` is readable for `len` NUL-terminated strings; `size >= len`.
    unsafe fn copy(
        out: *mut *mut *mut c_char,
        input: *const *const c_char,
        len: usize,
        size: usize,
    ) -> c_int {
        let dup = alloc::calloc(size, size_of::<*mut c_char>()).cast::<*mut c_char>();
        if dup.is_null() {
            return ErrorCode::GENERIC.raw();
        }
        for i in 0..len {
            // SAFETY: `i < len`: within the caller's array.
            let p = unsafe { input.wrapping_add(i).read() };
            // SAFETY: a NUL-terminated string.
            let s = unsafe { CStr::from_ptr(p) };
            let d = alloc::strdup(s);
            if d.is_null() {
                // SAFETY: the first `i` slots hold our copies; the array
                // is ours.
                unsafe { git_strlist_free(dup, i) };
                return errors::oom().raw();
            }
            // SAFETY: `i < len <= size`, a slot of the array.
            unsafe { dup.wrapping_add(i).write(d) };
        }
        // SAFETY: writable.
        unsafe { out.write(dup) };
        0
    }

    /// `git_strlist_copy`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `input` is readable for `len` NUL-terminated
    /// strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_copy(
        out: *mut *mut *mut c_char,
        input: *const *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { copy(out, input, len, len) })
    }

    /// `git_strlist_copy_with_null`: the copy is followed by a NULL.
    ///
    /// # Safety
    ///
    /// As [`git_strlist_copy`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_copy_with_null(
        out: *mut *mut *mut c_char,
        input: *const *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            let new_len = match alloc_add(len, 1) {
                Ok(n) => n,
                Err(e) => return e.raw(),
            };
            // SAFETY: the caller's contract.
            unsafe { copy(out, input, len, new_len) }
        })
    }

    /// The `len` strings at `strings`.
    ///
    /// # Safety
    ///
    /// `strings` is readable for `len` NUL-terminated strings.
    unsafe fn list<'a>(strings: *const *const c_char, len: usize) -> Vec<&'a CStr> {
        (0..len)
            .map(|i| {
                // SAFETY: `i < len`: within the caller's array.
                let p = unsafe { strings.wrapping_add(i).read() };
                // SAFETY: a NUL-terminated string.
                unsafe { CStr::from_ptr(p) }
            })
            .collect()
    }

    /// `git_strlist_contains_prefix`.
    ///
    /// # Safety
    ///
    /// `strings` is readable for `len` NUL-terminated strings; `str` is
    /// readable up to `n` bytes or a NUL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_contains_prefix(
        strings: *const *const c_char,
        len: usize,
        str: *const c_char,
        n: usize,
    ) -> bool {
        barrier(|| {
            // SAFETY: `strnlen` reads at most `n` bytes and stops at a NUL.
            let slen = unsafe { libc::strnlen(str, n) };
            // SAFETY: those bytes are readable.
            let s = unsafe { crate::util::export::bytes(str, slen) };
            // SAFETY: the caller's contract.
            let strings = unsafe { list(strings, len) };
            contains_prefix(&strings, s, n)
        })
    }

    /// `git_strlist_contains_key`.
    ///
    /// # Safety
    ///
    /// `strings` as for [`git_strlist_contains_prefix`]; `key` is a
    /// NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_contains_key(
        strings: *const *const c_char,
        len: usize,
        key: *const c_char,
        delimiter: c_char,
    ) -> bool {
        barrier(|| {
            // SAFETY: the caller's contract.
            let strings = unsafe { list(strings, len) };
            // SAFETY: a NUL-terminated string.
            let key = unsafe { cstr_bytes(key) };
            contains_key(&strings, key, delimiter as u8)
        })
    }

    /// `git_strlist_free`.
    ///
    /// # Safety
    ///
    /// `strings` is NULL or a `git__allocator` array of `len` strings from
    /// `git__allocator` (or NULL), all freed here.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_free(strings: *mut *mut c_char, len: usize) {
        if strings.is_null() {
            return;
        }
        for i in 0..len {
            // SAFETY: `i < len`: within the caller's array.
            let p = unsafe { strings.wrapping_add(i).read() };
            // SAFETY: the caller's contract: ours to free.
            unsafe { alloc::free(p.cast()) };
        }
        // SAFETY: as above.
        unsafe { alloc::free(strings.cast()) };
    }

    /// `git_strlist_free_with_null`.
    ///
    /// # Safety
    ///
    /// `strings` is NULL or a NULL-terminated `git__allocator` array of
    /// `git__allocator` strings, all freed here.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_free_with_null(strings: *mut *mut c_char) {
        if strings.is_null() {
            return;
        }
        let mut s = strings;
        loop {
            // SAFETY: within the NULL-terminated array.
            let p = unsafe { s.read() };
            if p.is_null() {
                break;
            }
            // SAFETY: the caller's contract.
            unsafe { alloc::free(p.cast()) };
            s = s.wrapping_add(1);
        }
        // SAFETY: as above.
        unsafe { alloc::free(strings.cast()) };
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
mod tests {
    use super::*;
    use core::ffi::c_char;

    #[test]
    fn prefixes_and_keys() {
        let l: [&CStr; 2] = [c"core.bare", c"user.name"];
        assert!(contains_prefix(&l, b"core", 4));
        assert!(!contains_prefix(&l, b"cores", 5));
        assert!(contains_prefix(&l, b"core.bare.x", 9));
        assert!(contains_key(&l, b"user=x", b'='));
        assert!(contains_key(&l, b"core.bare=1", b'='));
        assert!(!contains_key(&l, b"user", b'='));
        assert!(!contains_key(&l, b"x=1", b'='));
    }

    #[test]
    #[cfg_attr(miri, ignore = "strnlen is a foreign function")]
    fn exports() {
        let _g = crate::util::test_support::init();
        let a = [c"one".as_ptr(), c"two".as_ptr()];
        let mut out: *mut *mut c_char = core::ptr::null_mut();
        // SAFETY: as C would call them.
        unsafe {
            assert_eq!(ffi::git_strlist_copy_with_null(&mut out, a.as_ptr(), 2), 0);
            assert_eq!(CStr::from_ptr(*out.add(1)), c"two");
            assert!(out.add(2).read().is_null());
            assert!(ffi::git_strlist_contains_prefix(
                out.cast(),
                2,
                c"tw".as_ptr(),
                2
            ));
            assert!(ffi::git_strlist_contains_key(
                out.cast(),
                2,
                c"one:x".as_ptr(),
                b':' as c_char
            ));
            ffi::git_strlist_free_with_null(out);
            assert_eq!(ffi::git_strlist_copy(&mut out, a.as_ptr(), 2), 0);
            ffi::git_strlist_free(out, 2);
        }
    }
}
