//! `src/libgit2/diff*.c` (C that stays C): the index diffs its entries
//! against the working directory (`git_index_add_all`,
//! `git_index_update_all`, and the racy-entry check of `git_index_write`).
//! Safe wrappers: `crate::index::foreign`.

use core::ffi::{c_char, c_int, c_void};

use crate::oid::{Oid, git_oid_t};
use crate::strarray::GitStrArray;

/// `GIT_DIFF_INCLUDE_IGNORED`.
pub const GIT_DIFF_INCLUDE_IGNORED: u32 = 1 << 1;
/// `GIT_DIFF_RECURSE_IGNORED_DIRS`.
pub const GIT_DIFF_RECURSE_IGNORED_DIRS: u32 = 1 << 2;
/// `GIT_DIFF_INCLUDE_UNTRACKED`.
pub const GIT_DIFF_INCLUDE_UNTRACKED: u32 = 1 << 3;
/// `GIT_DIFF_RECURSE_UNTRACKED_DIRS`.
pub const GIT_DIFF_RECURSE_UNTRACKED_DIRS: u32 = 1 << 4;
/// `GIT_DIFF_INCLUDE_TYPECHANGE`.
pub const GIT_DIFF_INCLUDE_TYPECHANGE: u32 = 1 << 6;
/// `GIT_DIFF_IGNORE_SUBMODULES`.
pub const GIT_DIFF_IGNORE_SUBMODULES: u32 = 1 << 9;
/// `GIT_DIFF_DISABLE_PATHSPEC_MATCH`.
pub const GIT_DIFF_DISABLE_PATHSPEC_MATCH: u32 = 1 << 12;
/// `GIT_DIFF_FLAG_EXISTS`.
pub const GIT_DIFF_FLAG_EXISTS: u32 = 1 << 3;
/// `GIT_SUBMODULE_IGNORE_UNSPECIFIED`.
pub const GIT_SUBMODULE_IGNORE_UNSPECIFIED: c_int = -1;

/// `git_diff_options` (public header), mirrored.
#[repr(C)]
pub struct GitDiffOptions {
    /// `version`.
    pub version: u32,
    /// `flags`.
    pub flags: u32,
    /// `ignore_submodules`.
    pub ignore_submodules: c_int,
    /// `pathspec`.
    pub pathspec: GitStrArray,
    /// `notify_cb`.
    pub notify_cb: *mut c_void,
    /// `progress_cb`.
    pub progress_cb: *mut c_void,
    /// `payload`.
    pub payload: *mut c_void,
    /// `context_lines`.
    pub context_lines: u32,
    /// `interhunk_lines`.
    pub interhunk_lines: u32,
    /// `oid_type`.
    pub oid_type: git_oid_t,
    /// `id_abbrev`.
    pub id_abbrev: u16,
    /// `max_size`.
    pub max_size: i64,
    /// `old_prefix`.
    pub old_prefix: *const c_char,
    /// `new_prefix`.
    pub new_prefix: *const c_char,
}

impl Default for GitDiffOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl GitDiffOptions {
    /// `GIT_DIFF_OPTIONS_INIT`.
    pub const fn new() -> GitDiffOptions {
        GitDiffOptions {
            version: 1,
            flags: 0,
            ignore_submodules: GIT_SUBMODULE_IGNORE_UNSPECIFIED,
            pathspec: GitStrArray {
                strings: core::ptr::null_mut(),
                count: 0,
            },
            notify_cb: core::ptr::null_mut(),
            progress_cb: core::ptr::null_mut(),
            payload: core::ptr::null_mut(),
            context_lines: 3,
            interhunk_lines: 0,
            oid_type: 0,
            id_abbrev: 0,
            max_size: 0,
            old_prefix: core::ptr::null(),
            new_prefix: core::ptr::null(),
        }
    }
}

/// `git_diff_file` (public header), mirrored.
#[repr(C)]
pub struct GitDiffFile {
    /// `id`.
    pub id: Oid,
    /// `path`.
    pub path: *const c_char,
    /// `size`.
    pub size: u64,
    /// `flags`.
    pub flags: u32,
    /// `mode`.
    pub mode: u16,
    /// `id_abbrev`.
    pub id_abbrev: u16,
}

/// `git_diff_delta` (public header), mirrored.
#[repr(C)]
pub struct GitDiffDelta {
    /// `status`.
    pub status: c_int,
    /// `flags`.
    pub flags: u32,
    /// `similarity`.
    pub similarity: u16,
    /// `nfiles`.
    pub nfiles: u16,
    /// `old_file`.
    pub old_file: GitDiffFile,
    /// `new_file`.
    pub new_file: GitDiffFile,
}

/// `git_diff_file_cb`.
pub type GitDiffFileCb =
    unsafe extern "C" fn(delta: *const GitDiffDelta, progress: f32, payload: *mut c_void) -> c_int;

unsafe extern "C" {
    /// `git_diff_index_to_workdir`.
    pub fn git_diff_index_to_workdir(
        diff: *mut *mut c_void,
        repo: *mut c_void,
        index: *mut c_void,
        opts: *const GitDiffOptions,
    ) -> c_int;
    /// `git_diff_foreach`.
    pub fn git_diff_foreach(
        diff: *mut c_void,
        file_cb: Option<GitDiffFileCb>,
        binary_cb: *mut c_void,
        hunk_cb: *mut c_void,
        line_cb: *mut c_void,
        payload: *mut c_void,
    ) -> c_int;
    /// `git_diff_free`.
    pub fn git_diff_free(diff: *mut c_void);
    /// `git_diff_num_deltas`.
    pub fn git_diff_num_deltas(diff: *const c_void) -> usize;
    /// `git_diff_get_delta`.
    pub fn git_diff_get_delta(diff: *const c_void, idx: usize) -> *const GitDiffDelta;
}

crate::c_layout_test! {
    headers: ["git2/diff.h"],
    types: [
        GitDiffOptions => "git_diff_options" { version, flags, ignore_submodules, pathspec, notify_cb, progress_cb, payload, context_lines, interhunk_lines, oid_type, id_abbrev, max_size, old_prefix, new_prefix },
        GitDiffFile => "git_diff_file" { id, path, size, flags, mode, id_abbrev },
        GitDiffDelta => "git_diff_delta" { status, flags, similarity, nfiles, old_file, new_file },
    ],
}
