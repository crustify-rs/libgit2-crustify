//! `src/libgit2/path.c` (C that stays C): repository-aware path
//! validation, which the index applies to every path it stores.

use core::ffi::{c_char, c_uint, c_void};

/// A `git_str` that C only reads (`GIT_STR_INIT_CONST`): `asize` 0.
#[repr(C)]
pub struct ConstStr {
    /// The bytes.
    pub ptr: *const c_char,
    /// 0: borrowed.
    pub asize: usize,
    /// The length (`SIZE_MAX` for a C string).
    pub size: usize,
}

/// `GIT_FS_PATH_REJECT_EMPTY_COMPONENT`.
pub const GIT_FS_PATH_REJECT_EMPTY_COMPONENT: c_uint = 1 << 0;
/// `GIT_FS_PATH_REJECT_TRAVERSAL`.
pub const GIT_FS_PATH_REJECT_TRAVERSAL: c_uint = 1 << 1;
/// `GIT_FS_PATH_REJECT_MAX`.
pub const GIT_FS_PATH_REJECT_MAX: c_uint = 1 << 9;
/// `GIT_PATH_REJECT_DOT_GIT`.
pub const GIT_PATH_REJECT_DOT_GIT: c_uint = GIT_FS_PATH_REJECT_MAX << 1;
/// `GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS` (Unix).
pub const GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS: c_uint =
    GIT_FS_PATH_REJECT_EMPTY_COMPONENT | GIT_FS_PATH_REJECT_TRAVERSAL;
/// `GIT_PATH_REJECT_WORKDIR_DEFAULTS`.
pub const GIT_PATH_REJECT_WORKDIR_DEFAULTS: c_uint =
    GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS | GIT_PATH_REJECT_DOT_GIT;
/// `GIT_PATH_REJECT_INDEX_DEFAULTS`.
pub const GIT_PATH_REJECT_INDEX_DEFAULTS: c_uint =
    GIT_FS_PATH_REJECT_TRAVERSAL | GIT_PATH_REJECT_DOT_GIT;

unsafe extern "C" {
    /// `git_path_str_is_valid`. Safe wrapper:
    /// `crate::index::foreign::path_is_valid`.
    pub fn git_path_str_is_valid(
        repo: *mut c_void,
        path: *const ConstStr,
        file_mode: u16,
        flags: c_uint,
    ) -> bool;
}

crate::c_layout_test! {
    headers: ["str.h"],
    types: [
        ConstStr => "git_str" { ptr, asize, size },
    ],
}

/// `git_path_validate_str_length` (`path.h`'s inline): fails with
/// "path too long" when `path` is too long for the platform.
pub fn validate_str_length(
    repo: Option<&crate::repository::GitRepository>,
    path: &crate::util::str::GitStr,
) -> crate::util::errors::Result<()> {
    use crate::util::errors::{Bytes, ErrorClass};
    let r = repo.map_or(
        core::ptr::null_mut(),
        crate::repository::GitRepository::as_ptr,
    );
    // A `GitStr` is a `git_str` too (both layout-tested): C only reads it.
    let s = core::ptr::from_ref(path).cast::<ConstStr>();
    let flags = crate::util::fs_path::GIT_FS_PATH_REJECT_LONG_PATHS;
    // SAFETY: `r` is NULL or the live repository `repo` borrows (`Sync`;
    // C reads its config and flags); `s` is a valid `git_str` (`GitStr` has
    // its layout, both layout-tested) that C only reads, borrowed for the
    // call.
    if unsafe { git_path_str_is_valid(r, s, 0, flags) } {
        return Ok(());
    }
    Err(crate::git_error_set!(
        ErrorClass::FILESYSTEM,
        "path too long: '{}'",
        Bytes(path.as_bytes())
    ))
}

/// `GIT_FS_PATH_REJECT_TRAVERSAL` (tree entry names).
pub const REJECT_TRAVERSAL: c_uint = GIT_FS_PATH_REJECT_TRAVERSAL;
/// `GIT_FS_PATH_REJECT_SLASH`.
pub const REJECT_SLASH: c_uint = 1 << 2;
/// `GIT_PATH_REJECT_DOT_GIT` (`GIT_FS_PATH_REJECT_MAX << 1`).
pub const REJECT_DOT_GIT: c_uint = GIT_PATH_REJECT_DOT_GIT;

/// `git_path_is_valid(repo, path, file_mode, flags)` (the inline of
/// `path.h`, over a NUL-terminated `path`).
///
/// # Safety
///
/// `repo` is NULL or a live repository not freed during the call; `path`
/// is a NUL-terminated string, unwritten during the call.
pub unsafe fn is_valid<R>(
    repo: *mut R,
    path: *const c_char,
    file_mode: u16,
    flags: c_uint,
) -> bool {
    // GIT_STR_INIT_CONST(path, SIZE_MAX): a borrowed string the C only reads
    // up to its NUL.
    let s = ConstStr {
        ptr: path,
        asize: 0,
        size: usize::MAX,
    };
    // SAFETY: `s` is a borrowed `git_str` over `path`, which the C reads as a
    // NUL-terminated string (this function's contract); `repo` is NULL or
    // a live repository not freed during the call (likewise).
    unsafe { git_path_str_is_valid(repo.cast(), &raw const s, file_mode, flags) }
}

/// `git_path_is_valid(repo, path, file_mode, flags)` over a byte string
/// (the refdb's reference names and paths).
///
/// # Safety
///
/// `repo` is NULL or a live repository that may be used on this thread.
pub unsafe fn is_valid_bytes<R>(repo: *mut R, path: &[u8], file_mode: u16, flags: c_uint) -> bool {
    let s = ConstStr {
        ptr: path.as_ptr().cast(),
        asize: 0,
        size: path.len(),
    };
    // SAFETY: `s` is a borrowed `git_str` over `path`, which the C only
    // reads, alive for the call; `repo` per this function's contract.
    unsafe { git_path_str_is_valid(repo.cast(), &raw const s, file_mode, flags) }
}
