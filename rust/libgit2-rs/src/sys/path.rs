//! `src/libgit2/path.c` (C that stays C): repository-aware path checks.

use core::ffi::{c_uint, c_void};

use crate::git_error_set;
use crate::repository::GitRepository;
use crate::util::errors::{Bytes, ErrorClass, Result};
use crate::util::fs_path::GIT_FS_PATH_REJECT_LONG_PATHS;
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_path_str_is_valid(
        repo: *mut c_void,
        path: *const GitStr,
        file_mode: u16,
        flags: c_uint,
    ) -> bool;
}

/// `git_path_validate_str_length` (`path.h`'s inline): fails with
/// "path too long" when `path` is too long for the platform.
pub fn validate_str_length(repo: Option<&GitRepository>, path: &GitStr) -> Result<()> {
    let r = repo.map_or(core::ptr::null_mut(), GitRepository::as_ptr);
    // SAFETY: NULL or a live repository, and a valid `git_str` that C
    // only reads.
    if unsafe { git_path_str_is_valid(r, path, 0, GIT_FS_PATH_REJECT_LONG_PATHS) } {
        return Ok(());
    }
    let b = path.as_bytes();
    Err(git_error_set!(
        ErrorClass::FILESYSTEM,
        "path too long: '{}'",
        Bytes(b)
    ))
}
