//! `src/libgit2/commit_graph.c` (C that stays C): the odb keeps a
//! `git_commit_graph *` it creates, refreshes, hands out and frees; and the
//! commit-graph lookups that `git_commit_list_parse` uses for its fast
//! path.

use core::ffi::{c_char, c_int, c_void};
use core::ptr::NonNull;

use crate::oid::{Oid, git_oid_t};
use crate::util::errors::{ErrorCode, Result};

unsafe extern "C" {
    fn git_commit_graph_new(
        cgraph_out: *mut *mut c_void,
        objects_dir: *const c_char,
        open_file: bool,
        oid_type: git_oid_t,
    ) -> c_int;
    fn git_commit_graph_free(cgraph: *mut c_void);
    fn git_commit_graph_get_file(file_out: *mut *mut c_void, cgraph: *mut c_void) -> c_int;
    fn git_commit_graph_refresh(cgraph: *mut c_void);
}

/// An owned `git_commit_graph *` (freed on drop).
#[derive(Debug)]
pub struct CommitGraph(NonNull<c_void>);

// SAFETY: the one field owns a `git_commit_graph` (its file name, its oid
// type, a `checked` flag and a lazily opened `git_commit_graph_file`: heap
// data and a mapping, none tied to a thread). Its C functions are reached
// only through `&mut self` (or `Drop`), so moving it to another thread and
// using it there is what C does under the odb's mutex. (Not `Sync`.)
unsafe impl Send for CommitGraph {}

impl CommitGraph {
    /// `git_commit_graph_new(&out, objects_dir, false, oid_type)`.
    pub fn new(objects_dir: &core::ffi::CStr, oid_type: git_oid_t) -> Result<CommitGraph> {
        let mut out = core::ptr::null_mut();
        // SAFETY: `out` is a writable slot, `objects_dir` a C string alive
        // for the call (C copies it).
        let r =
            unsafe { git_commit_graph_new(&raw mut out, objects_dir.as_ptr(), false, oid_type) };
        ErrorCode::check(r)?;
        NonNull::new(out).map(CommitGraph).ok_or(ErrorCode::GENERIC)
    }

    /// Adopt a commit-graph C made (`git_odb_set_commit_graph`).
    ///
    /// # Safety
    ///
    /// `p` is a live `git_commit_graph *` whose ownership passes here.
    pub unsafe fn from_raw(p: NonNull<c_void>) -> CommitGraph {
        CommitGraph(p)
    }

    /// `git_commit_graph_get_file`: the file, still owned by the graph.
    pub fn get_file(&mut self) -> Result<*mut c_void> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live graph, used exclusively (`&mut self`).
        let r = unsafe { git_commit_graph_get_file(&raw mut out, self.0.as_ptr()) };
        ErrorCode::check_zero(r)?;
        Ok(out)
    }

    /// `git_commit_graph_refresh`.
    pub fn refresh(&mut self) {
        // SAFETY: as above.
        unsafe { git_commit_graph_refresh(self.0.as_ptr()) }
    }
}

impl Drop for CommitGraph {
    fn drop(&mut self) {
        // SAFETY: owned, freed once.
        unsafe { git_commit_graph_free(self.0.as_ptr()) }
    }
}

/// `git_commit_graph_file` (`commit_graph.h`), opaque here.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct git_commit_graph_file {
    _private: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

/// `git_commit_graph_entry` (`commit_graph.h`), filled by the C.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitCommitGraphEntry {
    pub generation: usize,
    /// `git_time_t`.
    pub commit_time: i64,
    pub parent_count: usize,
    pub parent_indices: [usize; 2],
    pub extra_parents_index: usize,
    pub tree_oid: Oid,
    pub sha1: Oid,
}

impl GitCommitGraphEntry {
    /// An entry for the C to fill.
    pub const fn zeroed() -> GitCommitGraphEntry {
        GitCommitGraphEntry {
            generation: 0,
            commit_time: 0,
            parent_count: 0,
            parent_indices: [0; 2],
            extra_parents_index: 0,
            tree_oid: Oid::from_raw_parts(0, [0; crate::oid::GIT_OID_MAX_SIZE]),
            sha1: Oid::from_raw_parts(0, [0; crate::oid::GIT_OID_MAX_SIZE]),
        }
    }
}

unsafe extern "C" {
    fn git_commit_graph_entry_find(
        e: *mut GitCommitGraphEntry,
        file: *const git_commit_graph_file,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int;
    fn git_commit_graph_entry_parent(
        parent: *mut GitCommitGraphEntry,
        file: *const git_commit_graph_file,
        entry: *const GitCommitGraphEntry,
        n: usize,
    ) -> c_int;
}

/// `git_commit_graph_entry_find`: the entry of the commit whose id starts
/// with the first `len` hex digits of `id`.
///
/// # Safety
///
/// `file` is a live commit-graph file (from `git_odb__get_commit_graph_file`)
/// that no other thread frees during the call.
pub unsafe fn entry_find(
    file: *const git_commit_graph_file,
    id: &Oid,
    len: usize,
) -> Result<GitCommitGraphEntry, c_int> {
    let mut e = GitCommitGraphEntry::zeroed();
    // SAFETY: `e` is a writable local, `id` a valid id borrowed for the
    // call; `file` is live and not freed during the call (this fn's
    // contract); C only reads `file`.
    let r = unsafe { git_commit_graph_entry_find(&raw mut e, file, id, len) };
    if r == 0 { Ok(e) } else { Err(r) }
}

/// `git_commit_graph_entry_parent`: the `n`th parent of `entry`.
///
/// # Safety
///
/// As [`entry_find`]; `entry` came from that file.
pub unsafe fn entry_parent(
    file: *const git_commit_graph_file,
    entry: &GitCommitGraphEntry,
    n: usize,
) -> Result<GitCommitGraphEntry, c_int> {
    let mut e = GitCommitGraphEntry::zeroed();
    // SAFETY: `e` is a writable local, `entry` a borrowed entry of `file`;
    // `file` is live and not freed during the call (this fn's contract); C
    // only reads `file`.
    let r = unsafe { git_commit_graph_entry_parent(&raw mut e, file, entry, n) };
    if r == 0 { Ok(e) } else { Err(r) }
}

crate::c_layout_test! {
    headers: ["commit_graph.h"],
    types: [
        GitCommitGraphEntry => "git_commit_graph_entry" {
            generation, commit_time, parent_count, parent_indices,
            extra_parents_index, tree_oid, sha1
        },
    ],
}
