//! What the revwalk uses of the modules around it: their types under the C
//! names it was written against, and their Rust definitions of the C ABI,
//! with the pointer types the walk holds.

#![allow(non_camel_case_types)]

use crate::util::cabi::Out;
use core::ffi::c_int;

use crate::oid::Oid;

pub use crate::commit::GitCommit as git_commit;
pub use crate::object::GitObject as git_object;
pub use crate::odb::{GIT_OBJECT_ANY, GIT_OBJECT_COMMIT, GitOdb as git_odb};
pub use crate::refdb::git_reference_iterator;
pub use crate::refs::GitReference as git_reference;
pub use crate::repository::GitRepository as git_repository;
pub use crate::tree::{GitTree as git_tree, GitTreeEntry as git_tree_entry};

pub use crate::object::ffi::{git_object_free, git_object_id, git_object_lookup, git_object_peel};

/// `git_commit_lookup`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_lookup(
    out: *mut *mut git_commit,
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
pub unsafe fn git_commit_free(commit: *mut git_commit) {
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_commit_free(commit.cast()) }
}

/// `git_tree_free`.
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_tree_free(tree: *mut git_tree) {
    // SAFETY: the caller's contract.
    unsafe { crate::object_api::ffi::git_tree_free(tree.cast()) }
}
