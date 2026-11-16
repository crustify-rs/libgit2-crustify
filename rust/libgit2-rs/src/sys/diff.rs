//! `src/libgit2/diff*.c` (stays C): what `git_commit_create_from_stage` /
//! `_from_tree` use to refuse an empty commit.

use core::ffi::{c_int, c_void};
use core::marker::{PhantomData, PhantomPinned};

use crate::sys::pending::l_commit::{git_index, git_repository};
use crate::tree::GitTree;

/// `git_diff`, opaque.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct git_diff {
    _private: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

unsafe extern "C" {
    fn git_diff_tree_to_tree(
        diff: *mut *mut git_diff,
        repo: *mut git_repository,
        old_tree: *mut GitTree,
        new_tree: *mut GitTree,
        opts: *const c_void,
    ) -> c_int;
    fn git_diff_tree_to_index(
        diff: *mut *mut git_diff,
        repo: *mut git_repository,
        old_tree: *mut GitTree,
        index: *mut git_index,
        opts: *const c_void,
    ) -> c_int;
    fn git_diff_num_deltas(diff: *const git_diff) -> usize;
    fn git_diff_free(diff: *mut git_diff);
}

/// The number of deltas between `old_tree` and `new_tree` (when given) or
/// `index`, with the default options; the diff is freed.
///
/// # Safety
///
/// `repo` is a live repository; `old_tree` is NULL or a live tree of it;
/// `new_tree` is NULL or a live tree, and `index` a live index when
/// `new_tree` is NULL; none is freed by another thread during the call.
pub unsafe fn count_deltas(
    repo: *mut git_repository,
    old_tree: *mut GitTree,
    new_tree: *mut GitTree,
    index: *mut git_index,
) -> Result<usize, c_int> {
    let mut diff = core::ptr::null_mut();
    let r = if new_tree.is_null() {
        // SAFETY: the caller's contract; `diff` is writable.
        unsafe { git_diff_tree_to_index(&mut diff, repo, old_tree, index, core::ptr::null()) }
    } else {
        // SAFETY: as above.
        unsafe { git_diff_tree_to_tree(&mut diff, repo, old_tree, new_tree, core::ptr::null()) }
    };
    if r < 0 {
        // SAFETY: NULL or the diff the call made.
        unsafe { git_diff_free(diff) };
        return Err(r);
    }
    // SAFETY: the diff just made.
    let n = unsafe { git_diff_num_deltas(diff) };
    // SAFETY: freed once.
    unsafe { git_diff_free(diff) };
    Ok(n)
}
