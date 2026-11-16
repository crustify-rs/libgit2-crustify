//! `src/libgit2/commit_graph.c` (C that stays C): the odb keeps a
//! `git_commit_graph *` it creates, refreshes, hands out and frees.

use core::ffi::{c_char, c_int, c_void};
use core::ptr::NonNull;

use crate::oid::git_oid_t;
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

// SAFETY: a commit-graph is used from any thread under its owner's lock
// (the odb's mutex), as C does.
unsafe impl Send for CommitGraph {}

impl CommitGraph {
    /// `git_commit_graph_new(&out, objects_dir, false, oid_type)`.
    pub fn new(objects_dir: &core::ffi::CStr, oid_type: git_oid_t) -> Result<CommitGraph> {
        let mut out = core::ptr::null_mut();
        // SAFETY: `out` is a writable slot, `objects_dir` a C string alive
        // for the call (C copies it).
        let r = unsafe { git_commit_graph_new(&mut out, objects_dir.as_ptr(), false, oid_type) };
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
        let r = unsafe { git_commit_graph_get_file(&mut out, self.0.as_ptr()) };
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
