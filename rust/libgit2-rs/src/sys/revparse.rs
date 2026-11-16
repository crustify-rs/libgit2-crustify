//! `src/libgit2/revparse.c` (C that stays C).

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;

use crate::repository::GitRepository;
use crate::repository::handles::{Object, Reference};

unsafe extern "C" {
    fn git_revparse_ext(
        object_out: *mut *mut c_void,
        reference_out: *mut *mut c_void,
        repo: *mut c_void,
        spec: *const c_char,
    ) -> c_int;
}

/// `git_revparse_ext`: the C status, the object, and the reference (each
/// owned, or `None`).
pub fn ext<'r>(repo: &'r GitRepository, spec: &CStr) -> (c_int, Option<Object<'r>>, Option<Reference<'r>>) {
    let (mut o, mut r) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: a live repository, a NUL-terminated spec, local outs.
    let e = unsafe { git_revparse_ext(&raw mut o, &raw mut r, repo.as_ptr(), spec.as_ptr()) };
    // SAFETY: what C wrote is NULL or owned by us.
    let o = unsafe { Object::from_raw(o) };
    // SAFETY: as above.
    let r = unsafe { Reference::from_raw(r) };
    (e, o, r)
}
