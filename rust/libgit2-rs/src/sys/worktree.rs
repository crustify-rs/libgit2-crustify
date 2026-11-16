//! `src/libgit2/worktree.c` (C that stays C): the calls the repository
//! makes, and the field of `git_worktree` it reads.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::repository::GitRepository;
use crate::repository::handles::check;
use crate::strarray::{GitStrArray, StrArray};
use crate::util::alloc::GitCStr;
use crate::util::errors::{ErrorCode, Result};

unsafe extern "C" {
    fn git_worktree__read_link(base: *const c_char, file: *const c_char) -> *mut c_char;
    fn git_worktree_lookup(out: *mut *mut c_void, repo: *mut c_void, name: *const c_char)
    -> c_int;
    fn git_worktree_list(out: *mut GitStrArray, repo: *mut c_void) -> c_int;
    fn git_worktree_free(wt: *mut c_void);
}

/// `offsetof(git_worktree, gitlink_path)` (checked by the repository's
/// layout test).
pub const WORKTREE_GITLINK_PATH_OFFSET: usize = 16;

/// `git_worktree__read_link(base, file)`: the trimmed contents of
/// `base/file`, or `None` (with an error set) when it cannot be read.
pub fn read_link(base: &CStr, file: &CStr) -> Option<GitCStr> {
    // SAFETY: two NUL-terminated strings; the result is NULL or a string
    // of `git__allocator` that the caller now owns.
    unsafe { GitCStr::from_raw(git_worktree__read_link(base.as_ptr(), file.as_ptr())) }
}

/// An owned `git_worktree`.
pub struct Worktree(NonNull<c_void>);

impl Drop for Worktree {
    fn drop(&mut self) {
        // SAFETY: the owned worktree, freed once.
        unsafe { git_worktree_free(self.0.as_ptr()) }
    }
}

impl Worktree {
    /// `git_worktree_lookup`.
    pub fn lookup(repo: &GitRepository, name: &CStr) -> Result<Worktree> {
        let mut out = ptr::null_mut();
        // SAFETY: a live repository, a NUL-terminated name, a local out.
        check(unsafe { git_worktree_lookup(&mut out, repo.as_ptr(), name.as_ptr()) })?;
        NonNull::new(out).map(Worktree).ok_or(ErrorCode::GENERIC)
    }

    /// Adopt a worktree C passes in, without owning it.
    ///
    /// # Safety
    ///
    /// `p` is a live `git_worktree` for `'a`.
    pub unsafe fn gitlink_path_of<'a>(p: NonNull<c_void>) -> Option<&'a CStr> {
        // SAFETY: the caller's contract; `gitlink_path` is a `char *` at
        // `WORKTREE_GITLINK_PATH_OFFSET`, NULL or NUL-terminated, owned by
        // the worktree.
        unsafe {
            let s = p
                .as_ptr()
                .cast::<u8>()
                .add(WORKTREE_GITLINK_PATH_OFFSET)
                .cast::<*const c_char>()
                .read();
            if s.is_null() { None } else { Some(CStr::from_ptr(s)) }
        }
    }

    /// The worktree's `gitlink_path`.
    pub fn gitlink_path(&self) -> Option<&CStr> {
        // SAFETY: the owned, live worktree.
        unsafe { Worktree::gitlink_path_of(self.0) }
    }
}

/// `git_worktree_list`.
pub fn list(repo: &GitRepository) -> Result<StrArray> {
    let mut out = GitStrArray {
        strings: ptr::null_mut(),
        count: 0,
    };
    // SAFETY: a live repository and a local array, which C fills with
    // libgit2 allocations on success.
    let r = check(unsafe { git_worktree_list(&mut out, repo.as_ptr()) });
    // SAFETY: what C filled (or `{NULL, 0}`), owned by nobody else.
    let arr = unsafe { StrArray::from_raw(out) };
    r.map(|_| arr)
}
