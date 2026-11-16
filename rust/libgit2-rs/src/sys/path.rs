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
