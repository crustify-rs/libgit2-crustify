//! `src/libgit2/pack-objects.c` (C that stays C): `git_mempack_dump` and
//! `git_mempack_write_thin_pack` drive a `git_packbuilder`.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use crate::oid::Oid;
use crate::util::errors::{ErrorCode, Result};
use crate::util::str::GitStr;

unsafe extern "C" {
    fn git_packbuilder_new(out: *mut *mut c_void, repo: *mut c_void) -> c_int;
    fn git_packbuilder_set_threads(pb: *mut c_void, n: c_uint) -> c_uint;
    fn git_packbuilder_insert(pb: *mut c_void, id: *const Oid, name: *const c_char) -> c_int;
    fn git_packbuilder_insert_commit(pb: *mut c_void, id: *const Oid) -> c_int;
    fn git_packbuilder__write_buf(buf: *mut GitStr, pb: *mut c_void) -> c_int;
    fn git_packbuilder_free(pb: *mut c_void);
}

/// Insert `id` into the C-owned packbuilder `pb` (`git_packbuilder_insert(pb, id, NULL)`).
///
/// # Safety
///
/// `pb` is a live `git_packbuilder *` that no other thread uses during the
/// call.
pub unsafe fn insert(pb: *mut c_void, id: &Oid) -> Result<()> {
    // SAFETY: `pb` is a live builder used by this thread alone (this fn's
    // contract); `id` is a readable id borrowed for the call; NULL name.
    ErrorCode::check_zero(unsafe { git_packbuilder_insert(pb, id, core::ptr::null()) })
}

/// An owned `git_packbuilder`.
#[derive(Debug)]
pub struct Packbuilder(NonNull<c_void>);

impl Packbuilder {
    /// `git_packbuilder_new(&pb, repo)`.
    ///
    /// # Safety
    ///
    /// `repo` is a live `git_repository *` that outlives the builder.
    pub unsafe fn new(repo: *mut c_void) -> Result<Packbuilder> {
        let mut out = core::ptr::null_mut();
        // SAFETY: `repo` is a live repository (this fn's contract); `out`
        // is a writable local.
        ErrorCode::check(unsafe { git_packbuilder_new(&raw mut out, repo) })?;
        NonNull::new(out).map(Packbuilder).ok_or(ErrorCode::GENERIC)
    }

    /// `git_packbuilder_set_threads`.
    pub fn set_threads(&mut self, n: c_uint) {
        // SAFETY: the live builder this value owns (made by `new`, whose
        // repository outlives it), used through `&mut self` alone.
        unsafe { git_packbuilder_set_threads(self.0.as_ptr(), n) };
    }

    /// `git_packbuilder_insert_commit`.
    pub fn insert_commit(&mut self, id: &Oid) -> Result<()> {
        // SAFETY: as in `set_threads`; `id` a readable id borrowed for the
        // call.
        ErrorCode::check(unsafe { git_packbuilder_insert_commit(self.0.as_ptr(), id) }).map(drop)
    }

    /// `git_packbuilder__write_buf`.
    pub fn write_buf(&mut self, buf: &mut GitStr) -> Result<()> {
        // SAFETY: as in `set_threads`; `buf` a valid string, borrowed
        // exclusively for the call.
        ErrorCode::check(unsafe { git_packbuilder__write_buf(buf, self.0.as_ptr()) }).map(drop)
    }
}

impl Drop for Packbuilder {
    fn drop(&mut self) {
        // SAFETY: the builder this value owns, freed once (no use after).
        unsafe { git_packbuilder_free(self.0.as_ptr()) }
    }
}
