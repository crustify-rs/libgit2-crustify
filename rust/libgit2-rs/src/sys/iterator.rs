//! `src/libgit2/iterator.c`, C that stays C: the filesystem iterator that
//! `refdb_fs` walks the loose references with.
//!
//! `git_iterator_advance` is a `GIT_INLINE` that calls `iter->cb->advance`,
//! so the start of `struct git_iterator` and `git_iterator_callbacks` are
//! mirrored here (layout-tested); `git_iterator_options` is mirrored whole.
//! Of `git_index_entry` (public, owned by WP l-index) only `path` is read,
//! at its offset (layout-tested).

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use crate::util::errors::{ErrorCode, Result};

/// `GIT_ITERATOR_IGNORE_CASE`.
pub const GIT_ITERATOR_IGNORE_CASE: c_uint = 1 << 0;
/// `GIT_ITERATOR_PRECOMPOSE_UNICODE`.
pub const GIT_ITERATOR_PRECOMPOSE_UNICODE: c_uint = 1 << 4;
/// `GIT_ITERATOR_DESCEND_SYMLINKS`.
pub const GIT_ITERATOR_DESCEND_SYMLINKS: c_uint = 1 << 7;

/// `git_strarray` (public), as embedded in the options.
#[repr(C)]
pub struct GitStrarrayRaw {
    /// The strings.
    pub strings: *mut *mut c_char,
    /// Their count.
    pub count: usize,
}

/// `git_iterator_options`, mirrored.
#[repr(C)]
pub struct GitIteratorOptions {
    /// `start`.
    pub start: *const c_char,
    /// `end`.
    pub end: *const c_char,
    /// `pathlist`.
    pub pathlist: GitStrarrayRaw,
    /// `flags` (`git_iterator_flag_t`).
    pub flags: c_uint,
    /// `oid_type`.
    pub oid_type: c_int,
}

/// An opaque `git_index_entry`.
#[repr(C)]
pub struct GitIndexEntryRaw {
    _opaque: [u8; 0],
}

/// `offsetof(git_index_entry, path)`. PENDING(l-index): read through the
/// integrated `IndexEntry`.
pub const INDEX_ENTRY_PATH_OFFSET: usize = 80;

/// The `advance` callback of `git_iterator_callbacks`.
pub type AdvanceFn =
    unsafe extern "C" fn(*mut *const GitIndexEntryRaw, *mut GitIteratorRaw) -> c_int;

/// `git_iterator_callbacks`, mirrored.
#[repr(C)]
pub struct GitIteratorCallbacks {
    /// `current`.
    pub current: Option<AdvanceFn>,
    /// `advance`.
    pub advance: Option<AdvanceFn>,
    /// `advance_into`.
    pub advance_into: Option<AdvanceFn>,
    /// `advance_over`.
    pub advance_over: *const c_void,
    /// `reset`.
    pub reset: *const c_void,
    /// `free`.
    pub free: *const c_void,
}

/// The start of `struct git_iterator`: `type` and `cb`.
#[repr(C)]
pub struct GitIteratorRaw {
    /// `type`.
    pub kind: c_int,
    /// `cb`.
    pub cb: *const GitIteratorCallbacks,
}

unsafe extern "C" {
    fn git_iterator_for_filesystem(
        out: *mut *mut GitIteratorRaw,
        root: *const c_char,
        options: *mut GitIteratorOptions,
    ) -> c_int;
    fn git_iterator_free(iter: *mut GitIteratorRaw);
}

/// A filesystem iterator (`git_iterator_for_filesystem`), freed on drop.
pub struct FsIterator(NonNull<GitIteratorRaw>);

impl FsIterator {
    /// `git_iterator_for_filesystem(root)` with `flags` and `oid_type`.
    pub fn new(root: &CStr, flags: c_uint, oid_type: c_int) -> Result<FsIterator> {
        let mut opts = GitIteratorOptions {
            start: core::ptr::null(),
            end: core::ptr::null(),
            pathlist: GitStrarrayRaw {
                strings: core::ptr::null_mut(),
                count: 0,
            },
            flags,
            oid_type,
        };
        let mut out = core::ptr::null_mut();
        // SAFETY: `root` is a NUL-terminated string and `opts` valid
        // options (no path list, no range), both alive for the call; `out`
        // is writable. The iterator copies what it keeps.
        let r = unsafe { git_iterator_for_filesystem(&raw mut out, root.as_ptr(), &raw mut opts) };
        if r < 0 {
            // SAFETY: on failure the C frees what it made; `out` is NULL or
            // an iterator we own.
            unsafe { git_iterator_free(out) };
            ErrorCode::check(r)?;
        }
        NonNull::new(out).map(FsIterator).ok_or(ErrorCode::GENERIC)
    }

    /// `git_iterator_advance`: the next entry's path (relative to the
    /// root), or `None` at the end (or on an error, which the C loop in
    /// `refdb_fs.c` treats the same).
    pub fn advance(&mut self) -> Option<&CStr> {
        let mut entry: *const GitIndexEntryRaw = core::ptr::null();
        let it = self.0.as_ptr();
        // SAFETY: `it` is a live iterator we own; its `cb` table is the
        // filesystem iterator's, whose `advance` is set.
        let cb = unsafe { (*it).cb };
        // SAFETY: as above.
        let advance = unsafe { (*cb).advance }?;
        // SAFETY: as above; `entry` is writable.
        if unsafe { advance(&raw mut entry, it) } != 0 || entry.is_null() {
            return None;
        }
        // SAFETY: on success `entry` is the iterator's current entry, whose
        // `path` (at the layout-tested offset) is a NUL-terminated string
        // owned by the iterator until the next advance; the borrow of
        // `self` (mutable) ends before that.
        let path = unsafe {
            entry
                .wrapping_byte_add(INDEX_ENTRY_PATH_OFFSET)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        // SAFETY: as above.
        Some(unsafe { CStr::from_ptr(path) })
    }
}

impl Drop for FsIterator {
    fn drop(&mut self) {
        // SAFETY: an iterator we own, freed once.
        unsafe { git_iterator_free(self.0.as_ptr()) };
    }
}

crate::c_layout_test! {
    headers: ["iterator.h"],
    types: [
        GitIteratorOptions => "git_iterator_options" { start, end, pathlist, flags, oid_type },
        GitStrarrayRaw => "git_strarray" { strings, count },
        GitIteratorCallbacks => "git_iterator_callbacks" {
            current, advance, advance_into, advance_over, reset, free
        },
    ],
    asserts: [
        "offsetof(struct git_iterator, type) == 0",
        "offsetof(struct git_iterator, cb) == 8",
        "offsetof(git_index_entry, path) == 80",
        "GIT_ITERATOR_IGNORE_CASE == 1",
        "GIT_ITERATOR_PRECOMPOSE_UNICODE == 16",
        "GIT_ITERATOR_DESCEND_SYMLINKS == 128",
    ],
}
