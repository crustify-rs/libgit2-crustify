//! What the commit, commit-list and tree modules use of the modules around
//! them: their types under the C names this code was written against, and
//! their Rust definitions of the C ABI, with the pointer types these
//! modules hold.

#![allow(non_camel_case_types, non_snake_case)]

use crate::util::cabi::Out;
use core::ffi::{c_char, c_int, c_void};

use crate::commit::{GitCommit, GitSignature};
use crate::oid::{Oid, git_oid_t};
use crate::tree::GitTree;

pub use crate::grafts::{GitCommitGraft, Grafts as git_grafts};
pub use crate::index::GitIndex as git_index;
pub use crate::object::{GitObject as git_object, GitObject as GitObjectHead};
pub use crate::odb::{GitOdb as git_odb, GitOdbObject as git_odb_object};
pub use crate::refs::GitReference as git_reference;
pub use crate::repository::GitRepository as git_repository;
pub use crate::revwalk::GitRevwalk as git_revwalk;
pub use crate::sys::commit_graph::git_commit_graph_file;
pub use crate::tree_cache::GitTreeCache as git_tree_cache;

/// `git_commitarray` (`include/git2/commit.h`, public; its dispose is
/// `commit.c`'s).
#[repr(C)]
pub struct GitCommitArray {
    pub commits: *const *mut GitCommit,
    pub count: usize,
}

pub type GitCommitParentCallback =
    Option<unsafe extern "C" fn(idx: usize, payload: *mut c_void) -> *const Oid>;

pub use crate::grafts::ffi::git_grafts_get;
pub use crate::object::ffi::{git_object__is_valid, git_object_lookup};
pub use crate::signature::ffi::{git_signature_default, git_signature_free};

/// `git_commit_lookup`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_lookup(
    out: *mut *mut GitCommit,
    repo: *mut git_repository,
    id: Option<&Oid>,
) -> c_int {
    // SAFETY: `out` is NULL or writable (as the call below requires), and no reference to it is
    // live.
    let out_out = unsafe { Out::from_ptr(out.cast()) };
    // SAFETY: `repo` is NULL or a live repository (as the call below requires), only read through
    // `&` (its state is interior-mutable).
    let repo_ref = unsafe { repo.as_ref() };
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_commit_lookup(out_out, repo_ref, id) }
}

/// `git_commit_free`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_free(commit: *mut GitCommit) {
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_commit_free(commit.cast()) }
}

/// `git_commit_dup`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_dup(out: *mut *mut GitCommit, source: *mut GitCommit) -> c_int {
    // SAFETY: `out` is NULL or writable (as the call below requires), and no reference to it is
    // live.
    let out_out = unsafe { Out::from_ptr(out.cast()) };
    // SAFETY: forwarded (the caller's contract).
    unsafe { crate::object_api::ffi::git_commit_dup(out_out, source.cast()) }
}

/// `git_tree_lookup`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_tree_lookup(
    out: *mut *mut GitTree,
    repo: *mut git_repository,
    id: Option<&Oid>,
) -> c_int {
    // SAFETY: `out` is NULL or writable (as the call below requires), and no reference to it is
    // live.
    let out_out = unsafe { Out::from_ptr(out.cast()) };
    // SAFETY: `repo` is NULL or a live repository (as the call below requires), only read through
    // `&` (its state is interior-mutable).
    let repo_ref = unsafe { repo.as_ref() };
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_tree_lookup(out_out, repo_ref, id) }
}

/// `git_tree_free`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_tree_free(tree: *mut GitTree) {
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_tree_free(tree.cast()) }
}

/// `git_tree_dup`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_tree_dup(out: *mut *mut GitTree, source: *mut GitTree) -> c_int {
    // SAFETY: `out` is NULL or writable (as the call below requires), and no reference to it is
    // live.
    let out_out = unsafe { Out::from_ptr(out.cast()) };
    // SAFETY: forwarded (the caller's contract).
    unsafe { crate::object_api::ffi::git_tree_dup(out_out, source.cast()) }
}

// Unused parameter types kept for the signatures above.
const _: Option<(c_char, git_oid_t, GitSignature)> = None;
