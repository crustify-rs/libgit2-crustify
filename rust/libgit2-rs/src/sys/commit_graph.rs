//! `src/libgit2/commit_graph.c` (stays C): the commit-graph lookups that
//! `git_commit_list_parse` uses for its fast path.

use core::ffi::c_int;

use crate::oid::Oid;
use crate::sys::pending::l_commit::git_commit_graph_file;

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
    // SAFETY: `e` is writable, `id` readable; `file` is live (contract).
    let r = unsafe { git_commit_graph_entry_find(&mut e, file, id, len) };
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
    // SAFETY: `e` is writable, `entry` readable; `file` is live (contract).
    let r = unsafe { git_commit_graph_entry_parent(&mut e, file, entry, n) };
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
