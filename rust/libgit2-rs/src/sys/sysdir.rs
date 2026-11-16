//! `src/libgit2/sysdir.c` (C that stays C): the search paths of the
//! global, XDG, system and template files, and `~` expansion.

use core::ffi::{CStr, c_char, c_int};
use core::ptr;

use crate::util::errors::{ErrorCode, Result};
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_sysdir_find_template_dir(path: *mut GitStr) -> c_int;
    fn git_sysdir_find_global_file(path: *mut GitStr, filename: *const c_char) -> c_int;
    fn git_sysdir_find_xdg_file(path: *mut GitStr, filename: *const c_char) -> c_int;
    fn git_sysdir_find_system_file(path: *mut GitStr, filename: *const c_char) -> c_int;
    fn git_sysdir_expand_homedir_file(path: *mut GitStr, filename: *const c_char) -> c_int;
    fn git_sysdir_get(out: *mut *const GitStr, which: c_int) -> c_int;
    fn git_sysdir_set(which: c_int, paths: *const c_char) -> c_int;
    /// `git_sysdir_global_init`.
    pub fn git_sysdir_global_init() -> c_int;
}

/// `GIT_SYSDIR_SYSTEM`.
pub const GIT_SYSDIR_SYSTEM: c_int = 0;
/// `GIT_SYSDIR_GLOBAL`.
pub const GIT_SYSDIR_GLOBAL: c_int = 1;
/// `GIT_SYSDIR_XDG`.
pub const GIT_SYSDIR_XDG: c_int = 2;
/// `GIT_SYSDIR_TEMPLATE`.
pub const GIT_SYSDIR_TEMPLATE: c_int = 3;
/// `GIT_SYSDIR_HOME`.
pub const GIT_SYSDIR_HOME: c_int = 4;

/// The C status of a sysdir call as a `Result`, keeping its code.
fn check(r: c_int) -> Result<()> {
    ErrorCode::check(r).map(drop)
}

/// A NULL-or-string argument.
fn opt_ptr(s: Option<&CStr>) -> *const c_char {
    s.map_or(ptr::null(), CStr::as_ptr)
}

/// `git_sysdir_find_template_dir`.
pub fn find_template_dir(out: &mut GitStr) -> Result<()> {
    // SAFETY: an exclusive `git_str`.
    check(unsafe { git_sysdir_find_template_dir(out) })
}

/// `git_sysdir_find_global_file`.
pub fn find_global_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: `path` is a valid, exclusive `git_str`; `filename` is
    // NUL-terminated; the C reads both during the call only.
    check(unsafe { git_sysdir_find_global_file(path, filename.as_ptr()) })
}

/// `git_sysdir_find_xdg_file`.
pub fn find_xdg_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: as in `find_global_file`.
    check(unsafe { git_sysdir_find_xdg_file(path, filename.as_ptr()) })
}

/// `git_sysdir_find_system_file`.
pub fn find_system_file(path: &mut GitStr, filename: &CStr) -> Result<()> {
    // SAFETY: as in `find_global_file`.
    check(unsafe { git_sysdir_find_system_file(path, filename.as_ptr()) })
}

/// `git_sysdir_expand_homedir_file` (`filename` NULL for the home
/// directory itself).
pub fn expand_homedir_file(path: &mut GitStr, filename: Option<&CStr>) -> Result<()> {
    // SAFETY: as in `find_global_file`; NULL is allowed for `filename`.
    check(unsafe { git_sysdir_expand_homedir_file(path, opt_ptr(filename)) })
}

/// `git_sysdir_get`: a copy of the search path `which` (`None` if it is
/// unset).
pub fn get(which: c_int) -> Result<Option<Vec<u8>>> {
    let mut out: *const GitStr = ptr::null();
    // SAFETY: `out` is writable; the C stores a pointer to its own
    // (thread-local or global) path.
    check(unsafe { git_sysdir_get(&raw mut out, which) })?;
    if out.is_null() {
        return Ok(None);
    }
    // SAFETY: a valid `git_str` the C keeps; it is copied at once, before
    // anything could change the search path.
    Ok(Some(unsafe { &*out }.as_bytes().to_vec()))
}

/// `git_sysdir_get(&s, which)`, appended to `out` (the C callers'
/// `git_str_put(out, tmp->ptr, tmp->size)`).
pub fn get_into(out: &mut GitStr, which: c_int) -> Result<()> {
    let mut s: *const GitStr = ptr::null();
    // SAFETY: a local out-parameter; C points it at its own path buffer.
    check(unsafe { git_sysdir_get(&raw mut s, which) })?;
    // SAFETY: on success `s` is the sysdir's `git_str`, which nothing
    // changes while we copy it (sysdir is set only by the options, which
    // the C does not run concurrently with a get either).
    let bytes = unsafe { (*s).as_bytes() };
    out.put(bytes)
}

/// `git_sysdir_set`.
pub fn set(which: c_int, paths: Option<&CStr>) -> Result<()> {
    // SAFETY: NULL or a NUL-terminated path list.
    check(unsafe { git_sysdir_set(which, opt_ptr(paths)) })
}
