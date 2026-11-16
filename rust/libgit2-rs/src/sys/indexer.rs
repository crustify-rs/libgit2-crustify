//! `src/libgit2/indexer.c` (C that stays C), as the pack backend's
//! `writepack` uses it.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use crate::oid::git_oid_t;
use crate::util::errors::{ErrorCode, Result};

/// `git_indexer_progress` (public: `git2/indexer.h`), which the Rust only
/// passes through.
#[repr(C)]
pub struct GitIndexerProgress {
    _opaque: [c_uint; 0],
}

/// `git_indexer_progress_cb`.
pub type IndexerProgressCb =
    Option<unsafe extern "C" fn(stats: *const GitIndexerProgress, payload: *mut c_void) -> c_int>;

/// `git_indexer_options`, mirrored (public: `git2/indexer.h`).
#[repr(C)]
pub struct GitIndexerOptions {
    /// `version`.
    pub version: c_uint,
    /// `mode`.
    pub mode: c_uint,
    /// `oid_type`.
    pub oid_type: git_oid_t,
    /// `odb` (a `git_odb *`).
    pub odb: *mut c_void,
    /// `progress_cb`.
    pub progress_cb: IndexerProgressCb,
    /// `progress_cb_payload`.
    pub progress_cb_payload: *mut c_void,
    /// `verify`.
    pub verify: u8,
}

impl Default for GitIndexerOptions {
    /// `GIT_INDEXER_OPTIONS_INIT`.
    fn default() -> Self {
        GitIndexerOptions {
            version: 1,
            mode: 0,
            oid_type: 0,
            odb: core::ptr::null_mut(),
            progress_cb: None,
            progress_cb_payload: core::ptr::null_mut(),
            verify: 0,
        }
    }
}

unsafe extern "C" {
    fn git_indexer_new(
        out: *mut *mut c_void,
        path: *const c_char,
        opts: *mut GitIndexerOptions,
    ) -> c_int;
    fn git_indexer_append(
        idx: *mut c_void,
        data: *const c_void,
        size: usize,
        stats: *mut GitIndexerProgress,
    ) -> c_int;
    fn git_indexer_commit(idx: *mut c_void, stats: *mut GitIndexerProgress) -> c_int;
    fn git_indexer_free(idx: *mut c_void);
}

/// A `git_indexer`, freed on drop.
pub struct Indexer(NonNull<c_void>);

// SAFETY: an indexer is not tied to a thread; it is used by one thread at a
// time (its methods take `&mut self`), and freed once.
unsafe impl Send for Indexer {}
// SAFETY: `&Indexer` gives access to nothing.
unsafe impl Sync for Indexer {}

impl Indexer {
    /// `git_indexer_new` into the directory `path` (which C asserts is not
    /// NULL).
    ///
    /// # Safety
    ///
    /// `opts.odb` is NULL or an odb that outlives the indexer;
    /// `opts.progress_cb`, if set, may be called with `progress_cb_payload`
    /// during [`append`](Self::append) and [`commit`](Self::commit).
    pub unsafe fn new(path: Option<&CStr>, opts: &mut GitIndexerOptions) -> Result<Indexer> {
        let mut out = core::ptr::null_mut();
        let path = path.map_or(core::ptr::null(), CStr::as_ptr);
        // SAFETY: `out` and `opts` are writable; C copies what it keeps of
        // `opts`; the odb and callback contract is the caller's.
        let r = unsafe { git_indexer_new(&raw mut out, path, opts) };
        ErrorCode::check(r)?;
        NonNull::new(out).map(Indexer).ok_or(ErrorCode::GENERIC)
    }

    /// `git_indexer_append`.
    ///
    /// # Safety
    ///
    /// `stats` is NULL or a writable `git_indexer_progress`.
    pub unsafe fn append(&mut self, data: &[u8], stats: *mut GitIndexerProgress) -> c_int {
        // SAFETY: a live indexer used by this thread alone; `data` is
        // readable for its length; `stats` per the caller.
        unsafe { git_indexer_append(self.0.as_ptr(), data.as_ptr().cast(), data.len(), stats) }
    }

    /// `git_indexer_append` over raw input, as C passed it.
    ///
    /// # Safety
    ///
    /// `data` is readable for `size` bytes (C reads nothing when `size` is
    /// 0); `stats` is NULL or a writable `git_indexer_progress`.
    pub unsafe fn append_raw(
        &mut self,
        data: *const c_void,
        size: usize,
        stats: *mut GitIndexerProgress,
    ) -> c_int {
        // SAFETY: the caller's contract; a live indexer.
        unsafe { git_indexer_append(self.0.as_ptr(), data, size, stats) }
    }

    /// `git_indexer_commit`.
    ///
    /// # Safety
    ///
    /// `stats` is NULL or a writable `git_indexer_progress`.
    pub unsafe fn commit(&mut self, stats: *mut GitIndexerProgress) -> c_int {
        // SAFETY: a live indexer; `stats` per the caller.
        unsafe { git_indexer_commit(self.0.as_ptr(), stats) }
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        // SAFETY: the indexer `git_indexer_new` gave, freed once.
        unsafe { git_indexer_free(self.0.as_ptr()) }
    }
}

crate::c_layout_test! {
    headers: ["git2/indexer.h"],
    types: [
        GitIndexerOptions => "git_indexer_options" {
            version, mode, oid_type, odb, progress_cb, progress_cb_payload, verify
        },
    ],
}
