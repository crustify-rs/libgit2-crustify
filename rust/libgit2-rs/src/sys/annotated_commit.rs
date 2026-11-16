//! `src/libgit2/annotated_commit.c` (C that stays C): what
//! `git_repository_set_head_detached_from_annotated` reads.

use core::ffi::{CStr, c_char, c_void};
use core::ptr::NonNull;

use crate::oid::Oid;

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
    // SAFETY: the caller's contract; the id lives in the commit.
    unsafe { &*git_annotated_commit_id(c.as_ptr()) }
}

/// The commit's `description` field.
///
/// # Safety
///
/// As [`id`].
pub unsafe fn description<'a>(c: NonNull<c_void>) -> Option<&'a CStr> {
    // SAFETY: the caller's contract; the field is a `const char *` at the
    // checked offset, NULL or NUL-terminated and owned by the commit.
    unsafe {
        let p = c
            .as_ptr()
            .cast::<u8>()
            .add(ANNOTATED_COMMIT_DESCRIPTION_OFFSET)
            .cast::<*const c_char>()
            .read();
        if p.is_null() { None } else { Some(CStr::from_ptr(p)) }
    }
}
