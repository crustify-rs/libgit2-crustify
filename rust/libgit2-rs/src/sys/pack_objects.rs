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
/// `pb` is a live `git_packbuilder *` the caller may use.
pub unsafe fn insert(pb: *mut c_void, id: &Oid) -> Result<()> {
    // SAFETY: the caller's contract; `id` is borrowed for the call.
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
        // SAFETY: the caller's contract; `out` is writable.
        ErrorCode::check(unsafe { git_packbuilder_new(&raw mut out, repo) })?;
        NonNull::new(out).map(Packbuilder).ok_or(ErrorCode::GENERIC)
    }

    /// `git_packbuilder_set_threads`.
    pub fn set_threads(&mut self, n: c_uint) {
        // SAFETY: a live builder.
        unsafe { git_packbuilder_set_threads(self.0.as_ptr(), n) };
    }

    /// `git_packbuilder_insert_commit`.
    pub fn insert_commit(&mut self, id: &Oid) -> Result<()> {
        // SAFETY: a live builder; `id` borrowed for the call.
        ErrorCode::check(unsafe { git_packbuilder_insert_commit(self.0.as_ptr(), id) }).map(drop)
    }

    /// `git_packbuilder__write_buf`.
    pub fn write_buf(&mut self, buf: &mut GitStr) -> Result<()> {
        // SAFETY: a live builder; `buf` a valid string borrowed for the call.
        ErrorCode::check(unsafe { git_packbuilder__write_buf(buf, self.0.as_ptr()) }).map(drop)
    }
}

impl Drop for Packbuilder {
    fn drop(&mut self) {
        // SAFETY: owned, freed once.
        unsafe { git_packbuilder_free(self.0.as_ptr()) }
    }
}
