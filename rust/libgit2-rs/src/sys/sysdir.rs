//! `src/libgit2/sysdir.c` (C that stays C): the search paths.

use core::ffi::{CStr, c_char, c_int};
use core::ptr;

use crate::repository::handles::{check, opt_ptr};
use crate::util::errors::Result;
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_sysdir_find_template_dir(path: *mut GitStr) -> c_int;
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

/// `git_sysdir_find_template_dir`.
pub fn find_template_dir(out: &mut GitStr) -> Result<()> {
    // SAFETY: an exclusive `git_str`.
    check(unsafe { git_sysdir_find_template_dir(out) }).map(drop)
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
    check(unsafe { git_sysdir_set(which, opt_ptr(paths)) }).map(drop)
}
