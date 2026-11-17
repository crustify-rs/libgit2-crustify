//! Port of `src/libgit2/oidarray.c`: `git_oidarray`, the array of oids the
//! public API returns, and the helpers over `git_array_oid_t`.
//!
//! [`GitOidArray`] is the public `git_oidarray` (kind 3, mirrored: `{ ids,
//! count }`, whose `ids` C frees with `git_oidarray_dispose`). [`OidArray`]
//! is its owner for Rust: it derefs to `&[Oid]` and frees on drop.
//! `git_array_oid_t` is [`GitArray<Oid>`].
//!
//! ```
//! use libgit2_rs::__test::util::array::GitArray;
//! use libgit2_rs::__test::oidarray::OidArray;
//! use libgit2_rs::{Oid, OidType};
//! # let _g = libgit2_rs::__test::util::test_support::init();
//! let a = Oid::from_prefix(b"aa", OidType::Sha1).unwrap();
//! let mut arr: GitArray<Oid> = GitArray::new();
//! libgit2_rs::__test::oidarray::add(&mut arr, &a)?;
//! libgit2_rs::__test::oidarray::add(&mut arr, &a)?; // already there
//! let ids = OidArray::from_array(arr);
//! assert_eq!(&ids[..], &[a]);
//! # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
//! ```

use core::ptr;

use crate::oid::Oid;
use crate::util::alloc;
use crate::util::array::GitArray;
use crate::util::errors::Result;

/// `git_oidarray`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitOidArray {
    /// The ids (from `git__allocator`).
    pub ids: *mut Oid,
    /// Their number.
    pub count: usize,
}

/// `git_oidarray`.
#[allow(non_camel_case_types)]
pub type git_oidarray = GitOidArray;

/// An owned array of oids (`git_oidarray`), freed on drop
/// (`git_oidarray_dispose`).
pub struct OidArray(GitOidArray);

// SAFETY: the one field that is not `Send` is `ids`, a `git__allocator`
// buffer of plain oids that the array owns alone (its constructors); it is
// freed with `git__free`, from any thread.
unsafe impl Send for OidArray {}
// SAFETY: through `&OidArray` the buffer is only read (`as_slice`); every
// write takes `&mut`.
unsafe impl Sync for OidArray {}

impl OidArray {
    /// An empty array.
    pub const fn new() -> OidArray {
        OidArray(GitOidArray {
            ids: ptr::null_mut(),
            count: 0,
        })
    }

    /// `git_oidarray__from_array`: take over a `git_array_oid_t`'s buffer.
    pub fn from_array(a: GitArray<Oid>) -> OidArray {
        let (ids, count, _) = a.into_raw_parts();
        OidArray(GitOidArray { ids, count })
    }

    /// `git_oidarray__to_array`: give the buffer to a `git_array_oid_t`.
    pub fn into_array(self) -> GitArray<Oid> {
        let raw = self.into_raw();
        // SAFETY: the array's own `git__allocator` buffer of `count`
        // initialized oids (NULL with 0), whose ownership `into_raw` gave
        // up: passed on once.
        unsafe { GitArray::from_raw_parts(raw.ids, raw.count, raw.count) }
    }

    /// Give the array to C, which frees it with `git_oidarray_dispose`.
    pub(crate) fn into_raw(self) -> GitOidArray {
        let me = core::mem::ManuallyDrop::new(self);
        GitOidArray {
            ids: me.0.ids,
            count: me.0.count,
        }
    }

    /// Adopt C's array.
    ///
    /// # Safety
    ///
    /// `raw.ids` is NULL or a `git__allocator` buffer of `raw.count`
    /// initialized oids, owned by the result.
    #[allow(dead_code)] // for the ports that adopt a C-made `git_oidarray`
    pub(crate) unsafe fn from_raw(raw: GitOidArray) -> OidArray {
        OidArray(raw)
    }

    /// The ids.
    pub fn as_slice(&self) -> &[Oid] {
        if self.0.ids.is_null() || self.0.count == 0 {
            &[]
        } else {
            // SAFETY: the invariant: a non-null `ids` is a buffer of `count`
            // initialized, aligned oids (one allocation, so at most
            // `isize::MAX` bytes) that the array owns; `&self` keeps it from
            // being written or freed meanwhile.
            unsafe { core::slice::from_raw_parts(self.0.ids, self.0.count) }
        }
    }

    /// The ids, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [Oid] {
        if self.0.ids.is_null() || self.0.count == 0 {
            &mut []
        } else {
            // SAFETY: as in `as_slice`; `&mut self` makes the buffer ours
            // alone for the borrow.
            unsafe { core::slice::from_raw_parts_mut(self.0.ids, self.0.count) }
        }
    }

    /// `git_oidarray__reverse`.
    pub fn reverse(&mut self) {
        self.as_mut_slice().reverse();
    }
}

impl Default for OidArray {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OidArray {
    fn drop(&mut self) {
        // SAFETY: the array's own `git__allocator` buffer (or NULL), freed
        // once; nothing borrows it any more (`drop` has `&mut`).
        unsafe { alloc::free(self.0.ids.cast()) };
    }
}

impl core::ops::Deref for OidArray {
    type Target = [Oid];
    fn deref(&self) -> &[Oid] {
        self.as_slice()
    }
}

impl core::fmt::Debug for OidArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

/// `git_oidarray__add`: append `id` unless it is there.
pub fn add(arr: &mut GitArray<Oid>, id: &Oid) -> Result<()> {
    if arr.iter().any(|x| x == id) {
        return Ok(());
    }
    arr.push(*id).map(|_| ())
}

/// `git_oidarray__remove`: remove the first `id`; whether it was there.
pub fn remove(arr: &mut GitArray<Oid>, id: &Oid) -> bool {
    match arr.iter().position(|x| x == id) {
        Some(i) => {
            arr.remove(i);
            true
        }
        None => false,
    }
}

/// The exports of `oidarray.c`.
pub mod ffi {
    use crate::git_arg;
    use crate::git_assert_arg;
    use crate::util::cabi::Out;
    use core::ffi::c_int;

    use super::*;
    use crate::util::export::{barrier, status};

    /// `git_oidarray_dispose`: free `ids` (the fields are left as they
    /// are, as C does).
    ///
    /// # Safety
    ///
    /// `arr` is NULL (an invalid argument; C crashes) or a valid
    /// `git_oidarray`, unwritten during the call, whose `ids` is NULL or
    /// from `git__allocator`, not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray_dispose(arr: Option<&GitOidArray>) {
        let arr = git_arg!(arr, "arr", ());
        // SAFETY: `ids` is NULL or from `git__allocator`, not used again
        // (this export's contract); nothing references the buffer.
        unsafe { alloc::free(arr.ids.cast()) };
    }

    /// `git_oidarray__from_array`.
    ///
    /// # Safety
    ///
    /// `out` is writable, not overlapping `array`; `array` is NULL (an
    /// invalid argument; C crashes) or a valid `git_array_oid_t`, unwritten
    /// during the call, whose buffer `out` now shares.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__from_array(
        mut out: Out<'_, GitOidArray>,
        array: Option<&GitArray<Oid>>,
    ) {
        let a = git_arg!(array, "array", ());
        let raw = GitOidArray {
            ids: a.as_ptr().cast_mut(),
            count: a.len(),
        };
        out.write(raw);
    }

    /// `git_oidarray__to_array`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `array` is a valid `git_oidarray`, whose buffer
    /// `out` now shares.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__to_array(
        mut out: Out<'_, GitArray<Oid>>,
        array: Option<&GitOidArray>,
    ) {
        let a = git_arg!(array, "array", ());
        // SAFETY: `a.ids` is NULL or a `git__allocator` buffer of `count`
        // initialized oids (this export's contract); the new array shares
        // it, as in C (C keeps track of which one frees it).
        let arr = unsafe { GitArray::from_raw_parts(a.ids, a.count, a.count) };
        out.write(arr);
    }

    /// `git_oidarray__reverse`.
    ///
    /// # Safety
    ///
    /// `arr` is a valid `git_oidarray`, not used concurrently.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__reverse(arr: Option<&GitOidArray>) {
        let a = git_arg!(arr, "arr", ());
        if a.count > 0 {
            // SAFETY: `a.ids` holds `count` initialized oids, one
            // allocation (at most `isize::MAX` bytes), that nothing else
            // accesses during the call (this export's contract); the
            // `&GitOidArray` covers only the struct, not the buffer.
            unsafe { core::slice::from_raw_parts_mut(a.ids, a.count) }.reverse();
        }
    }

    /// `git_oidarray__add`.
    ///
    /// # Safety
    ///
    /// `arr` is NULL or a valid `git_array_oid_t` that nothing else uses
    /// during the call; `id` a valid oid, unwritten during the call (it may
    /// be an element of the array: then nothing is pushed, so the buffer
    /// is only read).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__add(
        arr: Option<&mut GitArray<Oid>>,
        id: Option<&Oid>,
    ) -> c_int {
        barrier(|| {
            let id = git_arg!(id, "id", -1);
            status(add(git_arg!(arr, "arr", -1), id))
        })
    }

    /// `git_oidarray__remove`.
    ///
    /// # Safety
    ///
    /// As [`git_oidarray__add`]. `id` stays a raw pointer, copied first:
    /// it may be an element of the array, which the removal overwrites
    /// (shifting the elements after it), and a write under a reference
    /// argument (protected for the call) would be undefined behaviour.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__remove(
        arr: Option<&mut GitArray<Oid>>,
        id: *const Oid,
    ) -> bool {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", false);
            // SAFETY: `id` is non-null, so a valid, aligned oid (this
            // export's contract), copied before the removal shifts the
            // elements it may point at.
            let id = unsafe { id.read() };
            remove(git_arg!(arr, "arr", false), &id)
        })
    }
}

crate::c_layout_test! {
    headers: ["oidarray.h"],
    types: [
        GitOidArray => "git_oidarray" { ids, count },
        GitArray<Oid> => "git_array_oid_t" {},
    ],
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
mod tests {
    use super::*;
    use crate::oid::OidType;

    #[test]
    fn add_remove_reverse() {
        let _g = crate::util::test_support::init();
        let ids: Vec<Oid> = (0..5u8)
            .map(|i| Oid::parse_prefix(&[b'a' + i], OidType::Sha1).unwrap())
            .collect();
        let mut arr = GitArray::new();
        for id in ids.iter().chain(ids.iter()) {
            add(&mut arr, id).unwrap();
        }
        assert_eq!(arr.len(), 5);
        assert!(remove(&mut arr, &ids[1]));
        assert!(!remove(&mut arr, &ids[1]));
        assert_eq!(arr.as_slice(), &[ids[0], ids[2], ids[3], ids[4]]);
        let mut o = OidArray::from_array(arr);
        o.reverse();
        assert_eq!(&o[..], &[ids[4], ids[3], ids[2], ids[0]]);
        let back = o.into_array();
        assert_eq!(back.len(), 4);
        let raw = OidArray::from_array(back).into_raw();
        // SAFETY: as C would call it.
        unsafe {
            ffi::git_oidarray__reverse(Some(&raw));
            assert_eq!(*raw.ids, ids[0]);
            ffi::git_oidarray_dispose(Some(&raw));
        }
        assert!(OidArray::new().is_empty());
    }

    /// The C passes an id that is an element of the array itself:
    /// `remove` shifts the buffer over it (with `id` as a reference
    /// argument, Miri reports the write under a protected borrow).
    #[test]
    fn add_and_remove_an_element_of_the_array() {
        let _g = crate::util::test_support::init();
        let ids: Vec<Oid> = (0..3u8)
            .map(|i| Oid::parse_prefix(&[b'a' + i], OidType::Sha1).unwrap())
            .collect();
        let mut arr = GitArray::new();
        for id in &ids {
            add(&mut arr, id).unwrap();
        }
        // SAFETY: a valid array; the ids point into its own buffer.
        unsafe {
            let first = &*arr.as_slice().as_ptr();
            // Already there: nothing is pushed.
            assert_eq!(ffi::git_oidarray__add(Some(&mut arr), Some(first)), 0);
            assert_eq!(arr.len(), 3);
            let p = arr.as_slice().as_ptr();
            assert!(ffi::git_oidarray__remove(Some(&mut arr), p));
        }
        assert_eq!(arr.as_slice(), &ids[1..]);
    }
}
