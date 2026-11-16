//! `src/libgit2/path.c`, C that stays C: repository-aware path validation.

use core::ffi::c_uint;

use crate::sys::pending::git_repository;
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_path_str_is_valid(
        repo: *mut git_repository,
        path: *const GitStr,
        file_mode: u16,
        flags: c_uint,
    ) -> bool;
}

/// `git_path_is_valid(repo, path, file_mode, flags)` (the `GIT_INLINE` of
/// `path.h`).
///
/// # Safety
///
/// `repo` is NULL or a live repository that may be used on this thread.
pub unsafe fn is_valid(repo: *mut git_repository, path: &[u8], file_mode: u16, flags: c_uint) -> bool {
    let s = GitStr::borrowed(path);
    // SAFETY: `s` is a valid borrowed `git_str` over `path`, which the C
    // only reads, alive for the call; `repo` per this function's contract.
    unsafe { git_path_str_is_valid(repo, &raw const *s, file_mode, flags) }
}
