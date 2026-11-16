//! `src/libgit2/midx.c` (the multi-pack-index, C that stays C), as the pack
//! backend (`odb_pack.rs`) uses it: open, look up, iterate and write
//! `multi-pack-index` files.
//!
//! [`Midx`] owns a `git_midx_file` and frees it on drop. The file is
//! read-only once open (its tables are a private read-only mapping), so
//! lookups through a shared reference may run on several threads.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use crate::oid::{Oid, git_oid_t};
use crate::util::errors::{ErrorCode, Result};
use crate::util::posix::GitMap;
use crate::util::vector::GitVector;

/// The prefix of `git_midx_file` (`midx.h`) that the Rust reads: the
/// table of pack names.
#[repr(C)]
pub struct GitMidxFilePrefix {
    /// `index_map`.
    pub index_map: GitMap,
    /// `packfile_names`: the `.idx` names, in the order of the pack ids.
    pub packfile_names: GitVector,
}

/// `git_midx_entry`, mirrored.
#[repr(C)]
pub struct GitMidxEntry {
    /// `pack_index`.
    pub pack_index: usize,
    /// `offset`.
    pub offset: i64,
    /// `sha1`.
    pub sha1: Oid,
}

impl Default for GitMidxEntry {
    fn default() -> Self {
        GitMidxEntry {
            pack_index: 0,
            offset: 0,
            sha1: Oid::NONE,
        }
    }
}

/// `git_midx_writer_options`, mirrored (public: `git2/sys/midx.h`).
#[repr(C)]
pub struct GitMidxWriterOptions {
    /// `version`.
    pub version: c_uint,
    /// `oid_type`.
    pub oid_type: git_oid_t,
}

/// `git_odb_foreach_cb`.
pub type OdbForeachCb = unsafe extern "C" fn(id: *const Oid, payload: *mut c_void) -> c_int;

unsafe extern "C" {
    fn git_midx_open(
        out: *mut *mut GitMidxFilePrefix,
        path: *const c_char,
        oid_type: git_oid_t,
    ) -> c_int;
    fn git_midx_needs_refresh(idx: *const GitMidxFilePrefix, path: *const c_char) -> bool;
    fn git_midx_entry_find(
        e: *mut GitMidxEntry,
        idx: *mut GitMidxFilePrefix,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int;
    fn git_midx_foreach_entry(
        idx: *mut GitMidxFilePrefix,
        cb: OdbForeachCb,
        data: *mut c_void,
    ) -> c_int;
    fn git_midx_free(idx: *mut GitMidxFilePrefix);
    fn git_midx_writer_new(
        out: *mut *mut c_void,
        pack_dir: *const c_char,
        opts: *mut GitMidxWriterOptions,
    ) -> c_int;
    fn git_midx_writer_add(w: *mut c_void, idx_path: *const c_char) -> c_int;
    fn git_midx_writer_commit(w: *mut c_void) -> c_int;
    fn git_midx_writer_free(w: *mut c_void);
}

/// An open `multi-pack-index` (`git_midx_file *`), freed on drop.
pub struct Midx(NonNull<GitMidxFilePrefix>);

// SAFETY: a `git_midx_file` is immutable once `git_midx_open` returns (a
// read-only mapping and the tables parsed from it); `git_midx_entry_find`
// and `git_midx_foreach_entry` only read it, and `git_midx_free` may run on
// any thread once nothing else uses it (`Drop`, with `&mut self`).
unsafe impl Send for Midx {}
// SAFETY: as above: shared use only reads.
unsafe impl Sync for Midx {}

impl Midx {
    /// `git_midx_open`.
    pub fn open(path: &CStr, oid_type: git_oid_t) -> Result<Midx> {
        let mut out = core::ptr::null_mut();
        // SAFETY: `out` is writable, `path` NUL-terminated; C keeps neither.
        let r = unsafe { git_midx_open(&mut out, path.as_ptr(), oid_type) };
        if r < 0 {
            // C frees what it allocated on failure.
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        NonNull::new(out).map(Midx).ok_or(ErrorCode::GENERIC)
    }

    /// `git_midx_needs_refresh`.
    pub fn needs_refresh(&self, path: &CStr) -> bool {
        // SAFETY: a live midx (only read) and a NUL-terminated path.
        unsafe { git_midx_needs_refresh(self.0.as_ptr(), path.as_ptr()) }
    }

    /// `git_midx_entry_find`: the status, and the entry it filled.
    pub fn entry_find(&self, short_oid: &Oid, len: usize) -> (c_int, GitMidxEntry) {
        let mut e = GitMidxEntry::default();
        // SAFETY: a live midx, which the lookup only reads (the `*mut` is
        // C's signature); `e` is writable; `short_oid` is valid.
        let r = unsafe { git_midx_entry_find(&mut e, self.0.as_ptr(), short_oid, len) };
        (r, e)
    }

    /// The number of pack names.
    pub fn packfile_count(&self) -> usize {
        // SAFETY: a live midx; `packfile_names` is at the prefix's offset
        // (layout test) and not written after `git_midx_open`.
        unsafe { (*self.0.as_ptr()).packfile_names.len() }
    }

    /// The `i`th pack name (an `.idx` file name, relative to the pack
    /// directory).
    pub fn packfile_name(&self, i: usize) -> Option<&CStr> {
        // SAFETY: as in `packfile_count`; the names are NUL-terminated
        // strings in the mapping, which lives as long as `self`.
        unsafe {
            let names = &(*self.0.as_ptr()).packfile_names;
            names.get(i).map(|p| CStr::from_ptr(p.cast::<c_char>()))
        }
    }

    /// `git_midx_foreach_entry`.
    ///
    /// # Safety
    ///
    /// `cb` may be called with `data` for every entry: it keeps the contract
    /// of a `git_odb_foreach_cb` with that payload.
    pub unsafe fn foreach_entry(&self, cb: OdbForeachCb, data: *mut c_void) -> c_int {
        // SAFETY: a live midx (only read); the callback contract is the
        // caller's.
        unsafe { git_midx_foreach_entry(self.0.as_ptr(), cb, data) }
    }
}

impl Drop for Midx {
    fn drop(&mut self) {
        // SAFETY: the midx `git_midx_open` gave, freed once.
        unsafe { git_midx_free(self.0.as_ptr()) }
    }
}

/// A `git_midx_writer`, freed on drop.
pub struct MidxWriter(*mut c_void);

impl MidxWriter {
    /// `git_midx_writer_new`. `pack_dir` may be NULL (C then fails its
    /// argument check, as for the C backend without a pack directory).
    pub fn new(pack_dir: Option<&CStr>, oid_type: git_oid_t) -> Result<MidxWriter> {
        let mut opts = GitMidxWriterOptions {
            version: 1,
            oid_type,
        };
        let mut out = core::ptr::null_mut();
        let dir = pack_dir.map_or(core::ptr::null(), CStr::as_ptr);
        // SAFETY: `out` and `opts` are writable and not kept; `dir` is NULL
        // or NUL-terminated.
        let r = unsafe { git_midx_writer_new(&mut out, dir, &mut opts) };
        ErrorCode::check(r)?;
        Ok(MidxWriter(out))
    }

    /// `git_midx_writer_add`.
    pub fn add(&mut self, idx_path: &CStr) -> Result<()> {
        // SAFETY: a live writer; `idx_path` is NUL-terminated.
        ErrorCode::check(unsafe { git_midx_writer_add(self.0, idx_path.as_ptr()) }).map(|_| ())
    }

    /// `git_midx_writer_commit`.
    pub fn commit(&mut self) -> Result<()> {
        // SAFETY: a live writer.
        ErrorCode::check(unsafe { git_midx_writer_commit(self.0) }).map(|_| ())
    }
}

impl Drop for MidxWriter {
    fn drop(&mut self) {
        // SAFETY: the writer `git_midx_writer_new` gave (or NULL), freed once.
        unsafe { git_midx_writer_free(self.0) }
    }
}

crate::c_layout_test! {
    headers: ["midx.h"],
    types: [
        GitMidxEntry => "git_midx_entry" { pack_index, offset, sha1 },
        GitMidxWriterOptions => "git_midx_writer_options" { version, oid_type },
    ],
    asserts: [
        "offsetof(git_midx_file, index_map) == 0",
        "offsetof(git_midx_file, packfile_names) == 16",
        "sizeof(git_vector) == 40",
    ],
}

const _: () = {
    assert!(core::mem::offset_of!(GitMidxFilePrefix, packfile_names) == 16);
    assert!(core::mem::size_of::<GitVector>() == 40);
};
