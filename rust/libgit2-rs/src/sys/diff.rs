//! `src/libgit2/diff*.c` (stays C): the tree-to-tree diff that a revwalk
//! limited by a wildcard pathspec runs.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::marker::{PhantomData, PhantomPinned};

use super::pending::l_revwalk::{git_repository, git_tree};
use crate::oid::Oid;
use crate::strarray::GitStrArray;

/// `git_diff`, opaque.
#[repr(C)]
pub struct git_diff {
    _opaque: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// `git_diff_file` (public header, mirrored).
#[repr(C)]
#[derive(Debug)]
pub struct git_diff_file {
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

/// `git_diff_delta` (public header, mirrored).
#[repr(C)]
#[derive(Debug)]
pub struct git_diff_delta {
    /// `status` (`git_delta_t`).
    pub status: c_int,
    /// `flags`.
    pub flags: u32,
    /// `similarity`.
    pub similarity: u16,
    /// `nfiles`.
    pub nfiles: u16,
    /// `old_file`.
    pub old_file: git_diff_file,
    /// `new_file`.
    pub new_file: git_diff_file,
}

/// `git_diff_options` (public header, mirrored).
#[repr(C)]
#[derive(Debug)]
pub struct git_diff_options {
    /// `version`.
    pub version: c_uint,
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
    pub oid_type: c_int,
    /// `id_abbrev`.
    pub id_abbrev: u16,
    /// `max_size`.
    pub max_size: i64,
    /// `old_prefix`.
    pub old_prefix: *const c_char,
    /// `new_prefix`.
    pub new_prefix: *const c_char,
}

/// `GIT_DIFF_OPTIONS_VERSION`.
pub const GIT_DIFF_OPTIONS_VERSION: c_uint = 1;

impl git_diff_options {
    /// `GIT_DIFF_OPTIONS_INIT`.
    pub const INIT: git_diff_options = git_diff_options {
        version: GIT_DIFF_OPTIONS_VERSION,
        flags: 0,
        // GIT_SUBMODULE_IGNORE_UNSPECIFIED
        ignore_submodules: -1,
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
    };
}

unsafe extern "C" {
    /// `git_diff_tree_to_tree`.
    pub fn git_diff_tree_to_tree(
        diff: *mut *mut git_diff,
        repo: *mut git_repository,
        old_tree: *mut git_tree,
        new_tree: *mut git_tree,
        opts: *const git_diff_options,
    ) -> c_int;
    /// `git_diff_num_deltas`.
    pub fn git_diff_num_deltas(diff: *const git_diff) -> usize;
    /// `git_diff_get_delta`.
    pub fn git_diff_get_delta(diff: *const git_diff, idx: usize) -> *const git_diff_delta;
    /// `git_diff_free`.
    pub fn git_diff_free(diff: *mut git_diff);
}

crate::c_layout_test! {
    headers: ["git2/diff.h"],
    types: [
        git_diff_file => "git_diff_file" { id, path, size, flags, mode, id_abbrev },
        git_diff_delta => "git_diff_delta" { status, flags, similarity, nfiles, old_file, new_file },
        git_diff_options => "git_diff_options" {
            version, flags, ignore_submodules, pathspec, notify_cb, progress_cb, payload,
            context_lines, interhunk_lines, oid_type, id_abbrev, max_size, old_prefix, new_prefix
        },
    ],
    asserts: [
        "GIT_DIFF_OPTIONS_VERSION == 1",
        "GIT_SUBMODULE_IGNORE_UNSPECIFIED == -1",
    ],
}
