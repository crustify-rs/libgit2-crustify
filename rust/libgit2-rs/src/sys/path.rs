//! `src/libgit2/path.c` (stays C): the validation of tree entry names.

use core::ffi::{c_char, c_uint};

use crate::sys::pending::l_commit::git_repository;
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_path_str_is_valid(
        repo: *mut git_repository,
        path: *const GitStr,
        file_mode: u16,
        flags: c_uint,
    ) -> bool;
}

/// `GIT_FS_PATH_REJECT_TRAVERSAL`.
pub const REJECT_TRAVERSAL: c_uint = 1 << 1;
/// `GIT_FS_PATH_REJECT_SLASH`.
pub const REJECT_SLASH: c_uint = 1 << 2;
/// `GIT_PATH_REJECT_DOT_GIT` (`GIT_FS_PATH_REJECT_MAX << 1`).
pub const REJECT_DOT_GIT: c_uint = 1 << 10;

/// `git_path_is_valid(repo, path, file_mode, flags)` (the inline of
/// `path.h`, over a NUL-terminated `path`).
///
/// # Safety
///
/// `repo` is NULL or a live repository not freed during the call; `path`
/// is NUL-terminated.
pub unsafe fn is_valid(
    repo: *mut git_repository,
    path: *const c_char,
    file_mode: u16,
    flags: c_uint,
) -> bool {
    // GIT_STR_INIT_CONST(path, SIZE_MAX): a borrowed string the C only reads
    // up to its NUL.
    // SAFETY: `asize == 0`: a borrowed string, never written or freed (and
    // not dropped: `ManuallyDrop`).
    let s = core::mem::ManuallyDrop::new(unsafe {
        GitStr::from_raw_parts(path.cast_mut(), 0, usize::MAX)
    });
    // SAFETY: `s` is a borrowed `git_str` over `path`, which the C reads as a
    // NUL-terminated string; `repo` per the contract.
    unsafe { git_path_str_is_valid(repo, &raw const *s, file_mode, flags) }
}
