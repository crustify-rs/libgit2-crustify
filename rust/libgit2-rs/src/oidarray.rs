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

// SAFETY: it owns its `git__allocator` buffer of plain values.
unsafe impl Send for OidArray {}
// SAFETY: shared access only reads.
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
        // initialized oids (NULL with 0).
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
            // SAFETY: the invariant: `count` initialized oids, owned.
            unsafe { core::slice::from_raw_parts(self.0.ids, self.0.count) }
        }
    }

    /// The ids, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [Oid] {
        if self.0.ids.is_null() || self.0.count == 0 {
            &mut []
        } else {
            // SAFETY: as above, exclusively.
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
        // SAFETY: the array's own buffer (or NULL), freed once.
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
    use crate::util::cabi::Out;
    use core::ffi::c_int;

    use super::*;
    use crate::util::export::{barrier, status};

    /// `git_oidarray_dispose`: free `ids` (the fields are left as they
    /// are, as C does).
    ///
    /// # Safety
    ///
    /// `arr` is a valid `git_oidarray` whose `ids` is NULL or from
    /// `git__allocator`, not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray_dispose(arr: *mut GitOidArray) {
        // SAFETY: a valid `git_oidarray`.
        let ids = unsafe { (*arr).ids };
        // SAFETY: NULL or from `git__allocator`, not used again.
        unsafe { alloc::free(ids.cast()) };
    }

    /// `git_oidarray__from_array`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `array` is a valid `git_array_oid_t`, whose
    /// buffer `out` now shares.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__from_array(
        mut out: Out<'_, GitOidArray>,
        array: *const GitArray<Oid>,
    ) {
        // SAFETY: a valid array, only read.
        let a = unsafe { &*array };
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
        let array = git_arg!(array, "array", ());
        let a = array;
        // SAFETY: the new array shares the buffer, as in C (C keeps track
        // of which one frees it).
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
        let arr = git_arg!(arr, "arr", ());
        let a = arr;
        if a.count > 0 {
            // SAFETY: as above, exclusively ours for the call.
            unsafe { core::slice::from_raw_parts_mut(a.ids, a.count) }.reverse();
        }
    }

    /// `git_oidarray__add`.
    ///
    /// # Safety
    ///
    /// `arr` is a valid `git_array_oid_t`, not used concurrently; `id` a
    /// valid oid (it may be in the array).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__add(arr: *mut GitArray<Oid>, id: Option<&Oid>) -> c_int {
        barrier(|| {
            let id = git_arg!(id, "id", -1);
            let id = *id;
            // SAFETY: a valid array, exclusively ours.
            status(add(unsafe { &mut *arr }, &id))
        })
    }

    /// `git_oidarray__remove`.
    ///
    /// # Safety
    ///
    /// As [`git_oidarray__add`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_oidarray__remove(
        arr: *mut GitArray<Oid>,
        id: Option<&Oid>,
    ) -> bool {
        barrier(|| {
            let id = git_arg!(id, "id", false);
            let id = *id;
            // SAFETY: a valid array, exclusively ours.
            remove(unsafe { &mut *arr }, &id)
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
            let mut raw = raw;
            ffi::git_oidarray__reverse(Some(&raw));
            assert_eq!(*raw.ids, ids[0]);
            ffi::git_oidarray_dispose(&mut raw);
        }
        assert!(OidArray::new().is_empty());
    }
}
