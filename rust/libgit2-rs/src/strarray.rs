//! Port of `src/libgit2/strarray.c`: `git_strarray`, the public array of
//! strings (`include/git2/strarray.h`).
//!
//! [`StrArray`] is an owned `git_strarray` (kind 3, a public type): the
//! layout of `{ char **strings; size_t count; }`, as ffibox's `CVec` of
//! owned strings in libgit2's allocator, which `git_strarray_dispose` frees
//! as `CVec`'s drop does (each string, then the array). An array C passes
//! in is borrowed: [`StrArrayRef`].

use core::ffi::{CStr, c_char, c_int};
use core::marker::PhantomData;

use ffibox::CVec;

use crate::util::alloc::{self, GIT_ALLOC, GitAlloc, GitCStr};
use crate::util::errors::{self, ErrorCode, Result};

/// `git_strarray`'s fields.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitStrArray {
    /// The strings.
    pub strings: *mut *mut c_char,
    /// How many.
    pub count: usize,
}

/// An owned `git_strarray`: its strings and array are libgit2's allocations,
/// freed on drop.
#[repr(transparent)]
#[derive(Default)]
pub struct StrArray(CVec<Option<GitCStr>, GitAlloc>);

impl StrArray {
    /// An empty array (`{ NULL, 0 }`).
    pub fn new() -> StrArray {
        StrArray(CVec::empty())
    }

    /// An array of copies of `strings`.
    pub fn from_cstrs(strings: &[&CStr]) -> Result<StrArray> {
        let mut v: Vec<Option<GitCStr>, GitAlloc> = Vec::new_in(GIT_ALLOC);
        if v.try_reserve_exact(strings.len()).is_err() {
            return Err(errors::oom());
        }
        for s in strings {
            // SAFETY: `strdup` returns NULL or a fresh NUL-terminated
            // allocation of `git__allocator`, now owned here.
            match unsafe { GitCStr::from_raw(alloc::strdup(s)) } {
                Some(c) => v.push(Some(c)),
                None => return Err(ErrorCode::GENERIC),
            }
        }
        Ok(StrArray::from_vec(v))
    }

    fn from_vec(v: Vec<Option<GitCStr>, GitAlloc>) -> StrArray {
        let mut v = core::mem::ManuallyDrop::new(v);
        let (ptr, len) = (v.as_mut_ptr(), v.len());
        if v.capacity() == 0 {
            return StrArray::new();
        }
        // SAFETY: the vector's buffer, from `GitAlloc`, holding `len`
        // initialized owners; its capacity may exceed `len`, which the
        // family's free (ignoring the size) accepts. The vector is not
        // dropped, so ownership passes once.
        StrArray(unsafe { CVec::from_raw_parts_or_empty(ptr, len) }.unwrap_or_default())
    }

    /// Adopt a `git_strarray` that libgit2 filled.
    ///
    /// # Safety
    ///
    /// `raw` is an array libgit2 allocated (or `{ NULL, 0 }`), whose
    /// strings are distinct allocations (or NULL), and nothing else frees
    /// it.
    pub unsafe fn from_raw(raw: GitStrArray) -> StrArray {
        if raw.strings.is_null() {
            return StrArray::new();
        }
        // SAFETY: `strings` is a libgit2 allocation of `count` pointers,
        // each NULL or a distinct libgit2 string, that nothing else frees
        // (this function's contract); `Option<GitCStr>` is a nullable
        // `char *`, so the buffer is a `CVec` of them.
        StrArray(
            unsafe { CVec::from_raw_parts_or_empty(raw.strings.cast(), raw.count) }
                .unwrap_or_default(),
        )
    }

    /// Give the array up to C (which frees it with `git_strarray_dispose`).
    pub fn into_raw(self) -> GitStrArray {
        let (p, n) = self.0.into_raw_parts();
        GitStrArray {
            strings: p.cast(),
            count: n,
        }
    }

    /// `count`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no strings.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The string at `i` (`None` past the end, or for a NULL entry).
    pub fn get(&self, i: usize) -> Option<&CStr> {
        self.0.as_slice().get(i)?.as_deref()
    }

    /// The (non-NULL) strings.
    pub fn iter(&self) -> impl Iterator<Item = &CStr> {
        self.0.as_slice().iter().filter_map(|s| s.as_deref())
    }

    /// The strings, as byte vectors.
    pub fn to_vec(&self) -> Vec<Vec<u8>> {
        self.iter().map(|s| s.to_bytes().to_vec()).collect()
    }

    /// `git_strarray_copy`: copies of `src`'s non-NULL strings.
    pub fn copy_from(src: StrArrayRef<'_>) -> Result<StrArray> {
        let n = src.raw.count;
        if n == 0 {
            return Ok(StrArray::new());
        }
        let mut v: Vec<Option<GitCStr>, GitAlloc> = Vec::new_in(GIT_ALLOC);
        // C allocates `count` slots with `git__calloc`.
        if v.try_reserve_exact(n).is_err() {
            return Err(errors::oom());
        }
        for s in src.iter_opt().flatten() {
            // SAFETY: as in `from_cstrs`.
            match unsafe { GitCStr::from_raw(alloc::strdup(s)) } {
                Some(c) => v.push(Some(c)),
                None => return Err(ErrorCode::GENERIC),
            }
        }
        Ok(StrArray::from_vec(v))
    }
}

impl From<&StrArray> for Vec<Vec<u8>> {
    fn from(a: &StrArray) -> Vec<Vec<u8>> {
        a.to_vec()
    }
}

impl core::fmt::Debug for StrArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.0.as_slice().iter().map(|s| s.as_deref()))
            .finish()
    }
}

/// A `git_strarray` that C passes in, borrowed for `'a`.
#[derive(Clone, Copy)]
pub struct StrArrayRef<'a> {
    raw: GitStrArray,
    _p: PhantomData<&'a CStr>,
}

impl<'a> StrArrayRef<'a> {
    /// Borrow `*raw`.
    ///
    /// # Safety
    ///
    /// `raw` is a valid `git_strarray`: `strings` holds `count` pointers,
    /// each NULL or NUL-terminated, all readable and unwritten for `'a`.
    pub unsafe fn from_ptr(raw: *const GitStrArray) -> StrArrayRef<'a> {
        // SAFETY: `raw` points to a valid, aligned `git_strarray` (this
        // function's contract); it is copied by value.
        let raw = unsafe { raw.read() };
        StrArrayRef {
            raw,
            _p: PhantomData,
        }
    }

    /// A borrow of an owned array.
    pub fn from_owned(a: &'a StrArray) -> StrArrayRef<'a> {
        let (strings, count) = (a.0.as_ptr().cast(), a.0.len());
        StrArrayRef {
            raw: GitStrArray { strings, count },
            _p: PhantomData,
        }
    }

    /// `count`.
    pub fn len(&self) -> usize {
        self.raw.count
    }

    /// Whether `count` is 0.
    pub fn is_empty(&self) -> bool {
        self.raw.count == 0
    }

    /// The entries, `None` for NULL ones.
    pub fn iter_opt(&self) -> impl Iterator<Item = Option<&'a CStr>> + use<'a> {
        let raw = self.raw;
        (0..if raw.strings.is_null() { 0 } else { raw.count }).map(move |i| {
            // SAFETY: by `from_ptr`'s contract (or the owned array's
            // invariant), `strings` holds `count` pointers, each NULL or a
            // NUL-terminated string readable for `'a`.
            let p = unsafe { raw.strings.wrapping_add(i).read() };
            // SAFETY: as above.
            unsafe { crate::util::export::cstr(p) }
        })
    }

    /// The non-NULL strings.
    pub fn iter(&self) -> impl Iterator<Item = &'a CStr> + use<'a> {
        self.iter_opt().flatten()
    }
}

/// The exports of `strarray.c`.
pub mod ffi {
    use super::*;
    use crate::git_assert_arg;
    use crate::util::cabi::Out;
    use crate::util::export::barrier;

    /// `git_strarray_copy`.
    ///
    /// # Safety
    ///
    /// `tgt` is NULL or writable (its old contents are overwritten, not
    /// freed); `src` is NULL or a valid `git_strarray`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strarray_copy(
        mut tgt: Out<'_, GitStrArray>,
        src: *const GitStrArray,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tgt.is_null(), "tgt", -1);
            git_assert_arg!(!src.is_null(), "src", -1);
            if core::ptr::eq(tgt.as_ptr().cast_const(), src) {
                // C zeroes `tgt` first, which empties `src`: the copy is
                // empty, and the strings are left to whoever else holds
                // them (rust/DEFECTS.md).
                tgt.write(GitStrArray {
                    strings: core::ptr::null_mut(),
                    count: 0,
                });
                return 0;
            }
            // SAFETY: `src` is non-null, so a valid array whose strings are
            // NULL or NUL-terminated and unwritten during the call (this
            // export's contract); it is not `tgt` (checked above) and is
            // read before `tgt` is written.
            let r = StrArray::copy_from(unsafe { StrArrayRef::from_ptr(src) });
            let (out, ret) = match r {
                Ok(a) => (a.into_raw(), 0),
                Err(e) => (
                    GitStrArray {
                        strings: core::ptr::null_mut(),
                        count: 0,
                    },
                    e.raw(),
                ),
            };
            tgt.write(out);
            ret
        })
    }

    /// `git_strarray_dispose` (NULL allowed): free the strings and the
    /// array, and zero it.
    ///
    /// # Safety
    ///
    /// `array` is NULL or a `git_strarray` that libgit2 filled (or zeroed),
    /// which nothing else accesses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_strarray_dispose(array: Option<&mut GitStrArray>) {
        barrier(|| {
            let Some(array) = array else {
                return;
            };
            let raw = core::mem::replace(
                array,
                GitStrArray {
                    strings: core::ptr::null_mut(),
                    count: 0,
                },
            );
            // SAFETY: `raw` is what libgit2 filled (or zeroed) (this
            // export's contract): adopted once, and `array` no longer holds
            // it.
            drop(unsafe { StrArray::from_raw(raw) });
        })
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)] // each block's SAFETY covers its sequence
mod tests {
    use super::*;
    use crate::util::cabi::Out;

    #[test]
    fn copy_skips_null_and_disposes() {
        let _g = crate::util::test_support::init();
        let a = StrArray::from_cstrs(&[c"one", c"two"]).unwrap();
        assert_eq!(a.to_vec(), [b"one".to_vec(), b"two".to_vec()]);
        let mut ptrs = [
            c"x".as_ptr().cast_mut(),
            core::ptr::null_mut(),
            c"y".as_ptr().cast_mut(),
        ];
        let src = GitStrArray {
            strings: ptrs.as_mut_ptr(),
            count: 3,
        };
        let mut tgt = GitStrArray {
            strings: core::ptr::null_mut(),
            count: 99,
        };
        // SAFETY: valid arrays, as C passes them.
        unsafe {
            assert_eq!(ffi::git_strarray_copy(Out::from(&mut tgt), &src), 0);
            assert_eq!(tgt.count, 2);
            let r = StrArrayRef::from_ptr(&tgt);
            assert_eq!(r.iter().collect::<Vec<_>>(), [c"x", c"y"]);
            ffi::git_strarray_dispose(Some(&mut tgt));
        }
        assert!(tgt.strings.is_null() && tgt.count == 0);
        let empty = GitStrArray {
            strings: core::ptr::null_mut(),
            count: 0,
        };
        // SAFETY: as above.
        unsafe {
            assert_eq!(ffi::git_strarray_copy(Out::from(&mut tgt), &empty), 0);
            assert!(tgt.strings.is_null());
        }
    }

    #[test]
    fn copy_onto_itself_empties_as_c() {
        let _g = crate::util::test_support::init();
        let mut ptrs = [c"x".as_ptr().cast_mut(), c"y".as_ptr().cast_mut()];
        let mut arr = GitStrArray {
            strings: ptrs.as_mut_ptr(),
            count: 2,
        };
        let p = &raw mut arr;
        // SAFETY: one valid array passed as both, as C may; `Out` and the
        // source are the same raw pointer, no reference covers it.
        let r = unsafe { ffi::git_strarray_copy(Out::from_ptr(p), p) };
        assert_eq!(r, 0);
        assert!(arr.strings.is_null() && arr.count == 0);
        // The strings are untouched (still the caller's).
        assert_eq!(ptrs[0], c"x".as_ptr().cast_mut());
    }
}

crate::c_layout_test! {
    headers: ["git2/strarray.h"],
    types: [
        GitStrArray => "git_strarray" { strings, count },
        StrArray => "git_strarray" {},
    ],
}
