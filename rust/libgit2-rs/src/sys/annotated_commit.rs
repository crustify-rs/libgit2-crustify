//! `src/libgit2/annotated_commit.c` (C that stays C): what
//! `git_repository_set_head_detached_from_annotated` reads.

use core::ffi::{CStr, c_char, c_void};
use core::ptr::NonNull;

use crate::oid::Oid;
use crate::repository::handles::{opt_cstr, read_field};

unsafe extern "C" {
    fn git_annotated_commit_id(commit: *const c_void) -> *const Oid;
}

/// `offsetof(git_annotated_commit, description)` (checked by the
/// repository's layout test).
pub const ANNOTATED_COMMIT_DESCRIPTION_OFFSET: usize = 56;

/// `git_annotated_commit_id`.
///
/// # Safety
///
/// `c` is a live `git_annotated_commit` for `'a`.
pub unsafe fn id<'a>(c: NonNull<c_void>) -> &'a Oid {
    // SAFETY: the caller's contract.
    let id = unsafe { git_annotated_commit_id(c.as_ptr()) };
    // SAFETY: the id lives in the commit, for `'a`.
    unsafe { &*id }
}

/// The commit's `description` field.
///
/// # Safety
///
/// As [`id`].
pub unsafe fn description<'a>(c: NonNull<c_void>) -> Option<&'a CStr> {
    // SAFETY: the caller's contract; the field is a `const char *` at the
    // checked offset.
    let p: *const c_char = unsafe { read_field(c, ANNOTATED_COMMIT_DESCRIPTION_OFFSET) };
    // SAFETY: NULL or NUL-terminated, owned by the live commit.
    unsafe { opt_cstr(p) }
}
