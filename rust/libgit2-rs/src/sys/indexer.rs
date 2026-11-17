//! `src/libgit2/indexer.c` (C that stays C): the globals the Rust reads and
//! writes (`GIT_OPT_SET_MAX_OBJECT_SIZE` and friends in `settings.rs`), and
//! the indexer the pack backend's `writepack` uses.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use crate::oid::git_oid_t;
use crate::util::errors::{ErrorCode, Result};

use core::sync::atomic::AtomicUsize;
#[cfg(not(miri))]
use core::sync::atomic::Ordering;

/// `UINT31_MAX`, `git_indexer__max_object_size`'s initial value (what
/// Miri, without the C, reads).
#[cfg(miri)]
pub const DEFAULT_MAX_OBJECT_SIZE: usize = 0x7fff_ffff;

#[cfg_attr(miri, allow(dead_code))]
unsafe extern "C" {
    /// `size_t git_indexer__max_objects`. Declared as an `AtomicUsize`, which
    /// has `size_t`'s layout and lets C write it while Rust reads it.
    pub static git_indexer__max_objects: AtomicUsize;
    /// `size_t git_indexer__max_object_size`, likewise.
    pub static git_indexer__max_object_size: AtomicUsize;
}

/// The current `git_indexer__max_object_size`: the largest object the
/// indexer and delta application accept. Under Miri (no C), the C's
/// initial value.
#[inline]
pub fn max_object_size() -> usize {
    #[cfg(miri)]
    {
        DEFAULT_MAX_OBJECT_SIZE
    }
    #[cfg(not(miri))]
    {
        // SAFETY: the C global is a `size_t` defined (and so initialized)
        // in `indexer.c`, which lives for the program and has the size and
        // alignment of `AtomicUsize`; C writes it only from
        // `git_libgit2_opts` (a plain store), which a Rust atomic load may
        // race with exactly as C's own plain loads would.
        unsafe { git_indexer__max_object_size.load(Ordering::Relaxed) }
    }
}

/// `git_indexer_progress *`, which the Rust only passes through.
pub type GitIndexerProgress = c_void;

/// `git_indexer_progress_cb`.
pub type IndexerProgressCb = crate::odb::IndexerProgressCb;

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

// SAFETY: the one field owns a `git_indexer` (heap state, a pack file
// being written, its mapped windows, the odb pointer and progress callback
// of its options): none is tied to a thread, and `Indexer::new`'s contract
// lets the odb be used and the callback be called from whichever thread
// uses the indexer. Its C functions are reached only through `&mut self`
// (or `Drop`), so one thread at a time.
unsafe impl Send for Indexer {}
// SAFETY: `&Indexer` gives access to nothing (no `&self` methods).
unsafe impl Sync for Indexer {}

impl Indexer {
    /// `git_indexer_new` into the directory `path` (which C asserts is not
    /// NULL).
    ///
    /// # Safety
    ///
    /// `opts.odb` is NULL or an odb that outlives the indexer;
    /// `opts.progress_cb`, if set, may be called with `progress_cb_payload`
    /// during [`append_raw`](Self::append_raw) and [`commit`](Self::commit),
    /// on whichever thread makes those calls (the indexer is `Send`).
    pub unsafe fn new(path: Option<&CStr>, opts: &mut GitIndexerOptions) -> Result<Indexer> {
        let mut out = core::ptr::null_mut();
        let path = path.map_or(core::ptr::null(), CStr::as_ptr);
        // SAFETY: `out` is a writable local; `opts` a valid options struct
        // borrowed for the call, of which C copies what it keeps; `path` is
        // NULL (C asserts) or NUL-terminated, borrowed for the call. C keeps
        // `opts.odb` and the callback with its payload, which this fn's
        // contract makes valid for the indexer's life.
        let r = unsafe { git_indexer_new(&raw mut out, path, opts) };
        ErrorCode::check(r)?;
        NonNull::new(out).map(Indexer).ok_or(ErrorCode::GENERIC)
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
        // SAFETY: the live indexer this value owns (`&mut self`: used by
        // this call alone); `data` readable for `size` bytes and `stats`
        // NULL or writable (this fn's contract).
        unsafe { git_indexer_append(self.0.as_ptr(), data, size, stats) }
    }

    /// `git_indexer_commit`.
    ///
    /// # Safety
    ///
    /// `stats` is NULL or a writable `git_indexer_progress`.
    pub unsafe fn commit(&mut self, stats: *mut GitIndexerProgress) -> c_int {
        // SAFETY: the live indexer this value owns (`&mut self`); `stats`
        // NULL or writable (this fn's contract).
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
