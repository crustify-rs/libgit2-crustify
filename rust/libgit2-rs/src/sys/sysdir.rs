//! `src/libgit2/sysdir.c` (C that stays C): the search paths of the
//! global, XDG and system configuration files, and `~` expansion.

use core::ffi::{CStr, c_char, c_int};

use crate::util::errors::{ErrorCode, Result};
use crate::util::str::GitStr;

/// `GIT_SYSDIR_GLOBAL`.
pub const GIT_SYSDIR_GLOBAL: c_int = 1;

mod raw {
    use super::*;
    unsafe extern "C" {
        pub fn git_sysdir_find_global_file(path: *mut GitStr, filename: *const c_char) -> c_int;
        pub fn git_sysdir_find_xdg_file(path: *mut GitStr, filename: *const c_char) -> c_int;
        pub fn git_sysdir_find_system_file(path: *mut GitStr, filename: *const c_char) -> c_int;
        pub fn git_sysdir_expand_homedir_file(path: *mut GitStr, filename: *const c_char)
        -> c_int;
        pub fn git_sysdir_get(out: *mut *const GitStr, which: c_int) -> c_int;
    }
}

/// The C status of a sysdir call as a `Result`, keeping its code.
fn check(r: c_int) -> Result<()> {
    if r < 0 { Err(ErrorCode::check(r).unwrap_err()) } else { Ok(()) }
}

/// `git_sysdir_find_global_file`.
pub fn find_global_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: `path` is a valid, exclusive `git_str`; `filename` is
    // NUL-terminated; the C reads both during the call only.
    check(unsafe { raw::git_sysdir_find_global_file(path, filename.as_ptr()) })
}

/// `git_sysdir_find_xdg_file`.
pub fn find_xdg_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: as in `find_global_file`.
    check(unsafe { raw::git_sysdir_find_xdg_file(path, filename.as_ptr()) })
}

/// `git_sysdir_find_system_file`.
pub fn find_system_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: as in `find_global_file`.
    check(unsafe { raw::git_sysdir_find_system_file(path, filename.as_ptr()) })
}

/// `git_sysdir_expand_homedir_file` (`filename` NULL for the home
/// directory itself).
pub fn expand_homedir_file(path: &mut GitStr, filename: Option<&CStr>) -> Result<()> {
    let f = filename.map_or(core::ptr::null(), CStr::as_ptr);
    // SAFETY: as in `find_global_file`; NULL is allowed for `filename`.
    check(unsafe { raw::git_sysdir_expand_homedir_file(path, f) })
}

/// `git_sysdir_get`: a copy of the search path `which` (`None` if it is
/// unset).
pub fn get(which: c_int) -> Result<Option<Vec<u8>>> {
    let mut out: *const GitStr = core::ptr::null();
    // SAFETY: `out` is writable; the C stores a pointer to its own
    // (thread-local or global) path.
    check(unsafe { raw::git_sysdir_get(&mut out, which) })?;
    if out.is_null() {
        return Ok(None);
    }
    // SAFETY: a valid `git_str` the C keeps; it is copied at once, before
    // anything could change the search path.
    Ok(Some(unsafe { &*out }.as_bytes().to_vec()))
}
