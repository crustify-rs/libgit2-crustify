//! Port of `src/util/strlist.c`: arrays of C strings (`char **`), as the C
//! that stays uses them. Rust code uses `Vec`s; this module has the C's
//! helpers and their exports.

use core::ffi::CStr;

use super::util::strncmp;

/// `git_strlist_contains_prefix`: whether `strncmp(s, str, n) == 0` for
/// one of `strings` (each read up to its NUL, as `strncmp` does).
pub fn contains_prefix(strings: &[&CStr], s: &[u8], n: usize) -> bool {
    contains_prefix_iter(strings.iter().copied(), s, n)
}

fn contains_prefix_iter<'a>(
    mut strings: impl Iterator<Item = &'a CStr>,
    s: &[u8],
    n: usize,
) -> bool {
    strings.any(|x| strncmp(x.to_bytes(), s, n) == 0)
}

/// `git_strlist_contains_key`: whether one of `strings` starts with the
/// part of `key` before the first `delimiter`; false if `key` has none.
pub fn contains_key(strings: &[&CStr], key: &[u8], delimiter: u8) -> bool {
    contains_key_iter(strings.iter().copied(), key, delimiter)
}

fn contains_key_iter<'a>(
    strings: impl Iterator<Item = &'a CStr>,
    key: &[u8],
    delimiter: u8,
) -> bool {
    let key = match memchr::memchr(0, key) {
        Some(n) => &key[..n],
        None => key,
    };
    match key.iter().position(|&c| c == delimiter) {
        // C's loop stops at the delimiter or the NUL; a NUL delimiter is
        // the end of the string, which means "none".
        Some(n) if delimiter != 0 => contains_prefix_iter(strings, key, n),
        _ => false,
    }
}

/// The exports of `strlist.c`.
pub mod ffi {
    use crate::util::alloc;
    use crate::util::cabi::{self, CStrArg, Out};
    use crate::util::errors::{self, ErrorCode};
    use crate::util::export::barrier;
    use crate::util::integer::alloc_add;
    use core::ffi::{c_char, c_int};

    /// The `len` strings at `input` (C's `const char **`), as string
    /// arguments.
    ///
    /// # Safety
    ///
    /// `input` is readable for `len` pointers, each NULL or a
    /// NUL-terminated string, none written for `'a`.
    unsafe fn strings<'a>(input: *const *const c_char, len: usize) -> &'a [CStrArg<'a>] {
        // SAFETY: `CStrArg` is `#[repr(transparent)]` over `*const c_char`,
        // so the array is one of `len` `CStrArg`s, readable and unwritten
        // for `'a`; each pointer is NULL or a string live and unwritten for
        // `'a`, `CStrArg`'s invariant (this fn's contract).
        unsafe { cabi::slice(input.cast::<CStrArg<'a>>(), len) }
    }

    /// Copy `input` into a new `git__calloc`'d array of `size` slots and
    /// store it in `out`; on failure, free what was copied (C leaks it) and
    /// set the out-of-memory error. A NULL string (which C passes to
    /// `strdup`) is copied as NULL.
    fn copy(mut out: Out<'_, *mut *mut c_char>, input: &[CStrArg<'_>], size: usize) -> c_int {
        debug_assert!(size >= input.len());
        let dup = alloc::calloc(size, size_of::<*mut c_char>()).cast::<*mut c_char>();
        if dup.is_null() {
            return ErrorCode::GENERIC.raw();
        }
        // SAFETY: `dup` is a fresh, zeroed (all NULL) and aligned array of
        // `size` pointers, ours alone until it is stored in `out`.
        let slots = unsafe { core::slice::from_raw_parts_mut(dup, size) };
        for (i, s) in input.iter().enumerate() {
            let d = match s.get() {
                Some(s) => alloc::strdup(s),
                None => core::ptr::null_mut(),
            };
            if d.is_null() && !s.is_null() {
                // SAFETY: the array and the first `i` strings (copies, or
                // NULL) are ours, from `git__allocator`; nothing uses them
                // after.
                unsafe { free_list(dup, i) };
                return errors::oom().raw();
            }
            slots[i] = d;
        }
        out.write(dup);
        0
    }

    /// `git_strlist_copy`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `input` is readable for `len` pointers to
    /// NUL-terminated strings, none written during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_copy(
        out: Out<'_, *mut *mut c_char>,
        input: *const *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: as this export's contract says.
            let input = unsafe { strings(input, len) };
            copy(out, input, len)
        })
    }

    /// `git_strlist_copy_with_null`: the copy is followed by a NULL.
    ///
    /// # Safety
    ///
    /// As [`git_strlist_copy`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_copy_with_null(
        out: Out<'_, *mut *mut c_char>,
        input: *const *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            let new_len = match alloc_add(len, 1) {
                Ok(n) => n,
                Err(e) => return e.raw(),
            };
            // SAFETY: as this export's contract says.
            let input = unsafe { strings(input, len) };
            copy(out, input, new_len)
        })
    }

    /// `strncmp(a, b, n) == 0`, reading both strings byte by byte in C's
    /// order: never past the first difference, a NUL, or `n` bytes.
    ///
    /// # Safety
    ///
    /// `a` and `b` are readable that far.
    unsafe fn strneq_raw(a: *const c_char, b: *const c_char, n: usize) -> bool {
        for i in 0..n {
            // SAFETY: byte `i` comes before the first difference or NUL and
            // within `n`: readable (this fn's contract).
            let x = unsafe { a.wrapping_add(i).read() };
            // SAFETY: as for `a`.
            let y = unsafe { b.wrapping_add(i).read() };
            if x != y {
                return false;
            }
            if x == 0 {
                return true;
            }
        }
        true
    }

    /// Whether `strncmp(s, str, n)` is 0 for one of `strings`.
    ///
    /// # Safety
    ///
    /// Each of `strings` is a NUL-terminated string; `str` is readable as
    /// `strncmp` reads it (up to the first difference with each string, a
    /// NUL, or `n` bytes).
    unsafe fn contains_prefix_raw(strings: &[*const c_char], str: *const c_char, n: usize) -> bool {
        // SAFETY: each `p` is NUL-terminated, and `str` readable as far as
        // `strneq_raw` reads it (this fn's contract).
        strings.iter().any(|&p| unsafe { strneq_raw(p, str, n) })
    }

    /// `git_strlist_contains_prefix`: whether `strncmp(strings[i], str, n)`
    /// is 0 for one of the strings.
    ///
    /// # Safety
    ///
    /// `strings` is readable for `len` pointers to NUL-terminated strings;
    /// `str` is readable as `strncmp` reads it (up to the first difference
    /// with each string, a NUL, or `n` bytes).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_contains_prefix(
        strings: *const *const c_char,
        len: usize,
        str: *const c_char,
        n: usize,
    ) -> bool {
        barrier(|| {
            // SAFETY: `strings` is readable for `len` pointers, unwritten
            // during the call (this export's contract).
            let strings = unsafe { cabi::slice(strings, len) };
            // SAFETY: the strings and `str` are as `contains_prefix_raw`
            // requires (this export's contract).
            unsafe { contains_prefix_raw(strings, str, n) }
        })
    }

    /// `git_strlist_contains_key`: whether one of the strings starts with
    /// the part of `key` before the first `delimiter` (false if none).
    ///
    /// # Safety
    ///
    /// `strings` as for [`git_strlist_contains_prefix`]; `key` is a
    /// NUL-terminated string (read up to the delimiter, then as `strncmp`
    /// reads it).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strlist_contains_key(
        strings: *const *const c_char,
        len: usize,
        key: *const c_char,
        delimiter: c_char,
    ) -> bool {
        barrier(|| {
            let mut n = 0;
            loop {
                // SAFETY: `n` is at most the index of the delimiter or the
                // NUL, which stop the loop: within the string (this
                // export's contract).
                let c = unsafe { key.wrapping_add(n).read() };
                if c == 0 {
                    return false;
                }
                if c == delimiter {
                    break;
                }
                n += 1;
            }
            // SAFETY: `strings` is readable for `len` pointers, unwritten
            // during the call (this export's contract).
            let strings = unsafe { cabi::slice(strings, len) };
            // SAFETY: the strings are NUL-terminated (this export's
            // contract), and `key` is readable for `n` bytes and up to its
            // NUL, all `strneq_raw` reads.
            unsafe { contains_prefix_raw(strings, key, n) }
        })
    }

    /// Free the first `len` strings of `strings`, then the array.
    ///
    /// # Safety
    ///
    /// `strings` is a `git__allocator` array of at least `len` pointers,
    /// each NULL or a `git__allocator` string, all ours to free and not
    /// used after.
    unsafe fn free_list(strings: *mut *mut c_char, len: usize) {
        // SAFETY: `strings` holds `len` readable pointers (this fn's
        // contract), not written while the slice lives.
        for &p in unsafe { cabi::slice(strings.cast_const(), len) } {
            // SAFETY: NULL or a `git__allocator` string that is ours (this
            // fn's contract), freed once.
            unsafe { alloc::free(p.cast()) };
        }
        // SAFETY: the array is a `git__allocator` allocation that is ours
        // (this fn's contract), freed once, after its last read.
        unsafe { alloc::free(strings.cast()) };
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
        // SAFETY: as this export's contract says.
        unsafe { free_list(strings, len) }
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
        let mut len = 0;
        // SAFETY: `len` is at most the index of the NULL terminator, which
        // stops the loop: within the array (this export's contract).
        while !unsafe { strings.wrapping_add(len).read() }.is_null() {
            len += 1;
        }
        // SAFETY: the array holds `len` strings before its NULL, all ours
        // (this export's contract).
        unsafe { free_list(strings, len) }
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
mod tests {
    use super::*;
    use crate::util::cabi::Out;
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
    fn exports() {
        let _g = crate::util::test_support::init();
        let a = [c"one".as_ptr(), c"two".as_ptr()];
        let mut out: *mut *mut c_char = core::ptr::null_mut();
        // SAFETY: as C would call them.
        unsafe {
            assert_eq!(
                ffi::git_strlist_copy_with_null(Out::from(&mut out), a.as_ptr(), 2),
                0
            );
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
            assert_eq!(ffi::git_strlist_copy(Out::from(&mut out), a.as_ptr(), 2), 0);
            ffi::git_strlist_free(out, 2);
        }
    }
}
