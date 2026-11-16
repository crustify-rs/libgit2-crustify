//! `src/libgit2/filter.c`, which stays C: the filter lists that blob
//! creation, `git_blob_filter` and object hashing apply.
//!
//! [`FilterList`] owns a `git_filter_list *` (freed with
//! `git_filter_list_free`). The loading functions take the repository and
//! blob as C pointers, so they stay `unsafe`; applying a list is safe.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::buf::GitBuf;
use crate::oid::Oid;
use crate::sys::pending::l_obj::git_repository;
use crate::util::errors::{ErrorCode, Result};
use crate::util::str::GitStr;

/// `git_filter_list`, opaque.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct git_filter_list {
    _private: [u8; 0],
    _not_send_sync: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

/// `git_filter_options`, mirrored (public; with `GIT_DEPRECATE_HARD`,
/// `commit_id` is `void *reserved`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct git_filter_options {
    /// `GIT_FILTER_OPTIONS_VERSION`.
    pub version: core::ffi::c_uint,
    /// `git_filter_flag_t`.
    pub flags: u32,
    /// Reserved (`commit_id` without `GIT_DEPRECATE_HARD`).
    pub reserved: *mut c_void,
    /// The commit to load attributes from.
    pub attr_commit_id: Oid,
}

impl git_filter_options {
    /// `GIT_FILTER_OPTIONS_INIT`.
    pub const INIT: git_filter_options = git_filter_options {
        version: 1,
        flags: 0,
        reserved: ptr::null_mut(),
        attr_commit_id: Oid::NONE,
    };
}

/// `GIT_FILTER_TO_WORKTREE`.
pub const GIT_FILTER_TO_WORKTREE: c_int = 0;
/// `GIT_FILTER_TO_ODB`.
pub const GIT_FILTER_TO_ODB: c_int = 1;
/// `GIT_FILTER_DEFAULT`.
pub const GIT_FILTER_DEFAULT: u32 = 0;
/// `GIT_FILTER_NO_SYSTEM_ATTRIBUTES`.
pub const GIT_FILTER_NO_SYSTEM_ATTRIBUTES: u32 = 1 << 1;
/// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`.
pub const GIT_FILTER_ATTRIBUTES_FROM_HEAD: u32 = 1 << 2;
/// `GIT_FILTER_ATTRIBUTES_FROM_COMMIT`.
pub const GIT_FILTER_ATTRIBUTES_FROM_COMMIT: u32 = 1 << 3;
/// `GIT_FILTER_BYTES_TO_CHECK_NUL`.
pub const GIT_FILTER_BYTES_TO_CHECK_NUL: usize = 8000;

#[allow(clashing_extern_declarations)]
unsafe extern "C" {
    fn git_filter_list_length(fl: *const git_filter_list) -> usize;
    fn git_filter_list_free(fl: *mut c_void);
    fn git_filter_list_load(
        filters: *mut *mut c_void,
        repo: *mut c_void,
        blob: *mut c_void,
        path: *const c_char,
        mode: c_int,
        flags: u32,
    ) -> c_int;
    fn git_filter_list_load_ext(
        filters: *mut *mut git_filter_list,
        repo: *mut git_repository,
        blob: *mut c_void,
        path: *const c_char,
        mode: c_int,
        opts: *mut git_filter_options,
    ) -> c_int;
    fn git_filter_list__apply_to_buffer(
        out: *mut GitStr,
        filters: *mut git_filter_list,
        data: *const c_char,
        len: usize,
    ) -> c_int;
    fn git_filter_list__apply_to_file(
        out: *mut GitStr,
        filters: *mut git_filter_list,
        repo: *mut git_repository,
        path: *const c_char,
    ) -> c_int;
    fn git_filter_list_apply_to_blob(
        out: *mut GitBuf,
        filters: *mut git_filter_list,
        blob: *mut c_void,
    ) -> c_int;
}

/// `git_filter_list_length` of a list C gave (NULL is the empty list).
///
/// # Safety
///
/// `fl` is NULL or a live filter list that no other thread modifies.
pub unsafe fn list_length(fl: *const git_filter_list) -> usize {
    // SAFETY: the caller's contract; the function reads the count only.
    unsafe { git_filter_list_length(fl) }
}

/// An owned `git_filter_list`, freed on drop.
#[derive(Debug)]
pub struct FilterList(NonNull<git_filter_list>);

impl Drop for FilterList {
    fn drop(&mut self) {
        // SAFETY: the list is owned by `self` and freed once.
        unsafe { git_filter_list_free(self.0.as_ptr().cast()) }
    }
}

impl FilterList {
    /// `git_filter_list_load`: the filters for `path` (`None` when none
    /// apply).
    ///
    /// # Safety
    ///
    /// `repo` is a live repository; `blob` is NULL or a live `git_blob` of
    /// it.
    pub unsafe fn load(
        repo: *mut git_repository,
        blob: *mut c_void,
        path: &CStr,
        mode: c_int,
        flags: u32,
    ) -> Result<Option<FilterList>> {
        let mut fl: *mut git_filter_list = ptr::null_mut();
        // SAFETY: the caller's contract on `repo` and `blob`; `path` is a
        // NUL-terminated string, `fl` writable.
        let r = unsafe {
            git_filter_list_load(
                (&raw mut fl).cast(),
                repo.cast(),
                blob,
                path.as_ptr(),
                mode,
                flags,
            )
        };
        ErrorCode::check(r)?;
        Ok(NonNull::new(fl).map(FilterList))
    }

    /// `git_filter_list_load_ext`.
    ///
    /// # Safety
    ///
    /// As [`load`](Self::load).
    pub unsafe fn load_ext(
        repo: *mut git_repository,
        blob: *mut c_void,
        path: &CStr,
        mode: c_int,
        opts: &mut git_filter_options,
    ) -> Result<Option<FilterList>> {
        let mut fl = ptr::null_mut();
        // SAFETY: as in `load`; `opts` is a valid options struct.
        let r =
            unsafe { git_filter_list_load_ext(&raw mut fl, repo, blob, path.as_ptr(), mode, opts) };
        ErrorCode::check(r)?;
        Ok(NonNull::new(fl).map(FilterList))
    }

    /// The raw list, for C calls that borrow it.
    pub fn as_ptr(&self) -> *mut git_filter_list {
        self.0.as_ptr()
    }

    /// `git_filter_list__apply_to_file`: filter the file at `path` into
    /// `out`.
    ///
    /// # Safety
    ///
    /// `repo` is the live repository the list was loaded for.
    pub unsafe fn apply_to_file(
        &self,
        out: &mut GitStr,
        repo: *mut git_repository,
        path: &CStr,
    ) -> Result<()> {
        // SAFETY: a live list, a valid `git_str`, the caller's `repo`, a
        // NUL-terminated path.
        let r =
            unsafe { git_filter_list__apply_to_file(out, self.0.as_ptr(), repo, path.as_ptr()) };
        ErrorCode::check(r).map(|_| ())
    }
}

/// `git_filter_list__apply_to_buffer` with a list C gave.
///
/// # Safety
///
/// `fl` is a live filter list that no other thread uses meanwhile.
pub unsafe fn apply_to_buffer(
    out: &mut GitStr,
    fl: *mut git_filter_list,
    data: &[u8],
) -> Result<()> {
    // SAFETY: the caller's contract on `fl`; `data` is readable for its
    // length, `out` a valid `git_str` that does not alias `data`
    // (`&mut` and `&` cannot overlap).
    let r = unsafe { git_filter_list__apply_to_buffer(out, fl, data.as_ptr().cast(), data.len()) };
    ErrorCode::check(r).map(|_| ())
}

/// `git_filter_list_apply_to_blob` with a list C gave, or NULL (no
/// filters: the contents are copied).
///
/// # Safety
///
/// `out` is a valid `git_buf`; `fl` NULL or a live list; `blob` a live
/// `git_blob`.
pub unsafe fn apply_to_blob(
    out: *mut GitBuf,
    fl: *mut git_filter_list,
    blob: *mut c_void,
) -> Result<()> {
    // SAFETY: the caller's contract.
    let r = unsafe { git_filter_list_apply_to_blob(out, fl, blob) };
    ErrorCode::check(r).map(|_| ())
}
