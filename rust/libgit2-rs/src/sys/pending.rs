//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

use core::ffi::c_void;

// PENDING(pool): `src/util/pool.c` (in PORTING.md §1's list; not yet
// ported). `git_futils_mkdir_relative`'s path cache allocates from the
// caller's (C-owned) pool.
unsafe extern "C" {
    /// `git_pool_malloc`. PENDING(pool)
    pub fn git_pool_malloc(pool: *mut c_void, items: usize) -> *mut c_void;
}

// PENDING(l-odb): `src/libgit2/odb.c` (the odb's error helpers) and
// `src/libgit2/delta.c`, ported concurrently by WP l-odb. The pack reader
// (pack.rs, odb_pack.rs) calls them through their C symbols until both have
// landed.
#[allow(improper_ctypes)] // the stream is passed through to C, opaque
unsafe extern "C" {
    /// `git_odb__error_notfound`: sets `object not found - <message>` (with
    /// the first `oid_len` hex digits of `oid` when it is not NULL), class
    /// `GIT_ERROR_ODB`; returns `GIT_ENOTFOUND`. PENDING(l-odb)
    pub fn git_odb__error_notfound(
        message: *const core::ffi::c_char,
        oid: *const crate::oid::Oid,
        oid_len: usize,
    ) -> core::ffi::c_int;
    /// `git_odb__error_ambiguous`: sets `ambiguous OID prefix - <message>`,
    /// class `GIT_ERROR_ODB`; returns `GIT_EAMBIGUOUS`. PENDING(l-odb)
    pub fn git_odb__error_ambiguous(message: *const core::ffi::c_char) -> core::ffi::c_int;
    /// `git_delta_apply`: applies `delta` to `base` into a new
    /// `git__malloc`ed, NUL-terminated buffer (`*out`, `*out_len` bytes);
    /// `*out` is NULL on failure. PENDING(l-odb)
    pub fn git_delta_apply(
        out: *mut *mut c_void,
        out_len: *mut usize,
        base: *const u8,
        base_len: usize,
        delta: *const u8,
        delta_len: usize,
    ) -> core::ffi::c_int;
    /// `git_delta_read_header_fromstream`: the base and result sizes of the
    /// delta that `stream` inflates (it reads the stream through
    /// `git_packfile_stream_read`). PENDING(l-odb)
    pub fn git_delta_read_header_fromstream(
        base_out: *mut usize,
        result_out: *mut usize,
        stream: *mut crate::pack::GitPackfileStream,
    ) -> core::ffi::c_int;
}

/// `git_odb__error_notfound` with a Rust message (`message` is a C string
/// literal) and an optional id prefix.
pub fn odb_error_notfound(
    message: &core::ffi::CStr,
    oid: Option<&crate::oid::Oid>,
    oid_len: usize,
) -> crate::util::errors::ErrorCode {
    let oid = oid.map_or(core::ptr::null(), |o| o as *const _);
    // SAFETY: `message` is NUL-terminated; `oid` is NULL or a valid oid,
    // of which C reads `oid_len` hex digits (callers pass at most its hex
    // size). The C formats into the thread's error state and keeps nothing.
    let r = unsafe { git_odb__error_notfound(message.as_ptr(), oid, oid_len) };
    crate::util::errors::ErrorCode::from_raw(r).unwrap_or(crate::util::errors::ErrorCode::NOT_FOUND)
}

/// `git_odb__error_ambiguous` with a Rust message.
pub fn odb_error_ambiguous(message: &core::ffi::CStr) -> crate::util::errors::ErrorCode {
    // SAFETY: `message` is NUL-terminated; C only formats it.
    let r = unsafe { git_odb__error_ambiguous(message.as_ptr()) };
    crate::util::errors::ErrorCode::from_raw(r).unwrap_or(crate::util::errors::ErrorCode::AMBIGUOUS)
}
