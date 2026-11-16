//! `src/libgit2/streams/*` (C that stays C): the global init functions of
//! `git_libgit2_init`, and what the options read and write.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::sync::atomic::AtomicI32;

use crate::repository::handles::{check, opt_ptr};
use crate::util::errors::Result;

unsafe extern "C" {
    /// `git_stream_registry_global_init`.
    pub fn git_stream_registry_global_init() -> c_int;
    /// `git_socket_stream_global_init`.
    pub fn git_socket_stream_global_init() -> c_int;
    /// `git_openssl_stream_global_init`.
    pub fn git_openssl_stream_global_init() -> c_int;
    /// `git_mbedtls_stream_global_init`.
    pub fn git_mbedtls_stream_global_init() -> c_int;
    fn git_openssl__set_cert_location(file: *const c_char, path: *const c_char) -> c_int;
    fn git_openssl__add_x509_cert(cert: *mut c_void) -> c_int;

    // `safe`: each is an `int` that `streams/socket.c` defines, always
    // initialized; `AtomicI32` has `int`'s size and alignment, every bit
    // pattern is valid, and its accesses are atomic, so a C write while the
    // Rust reads is not a Rust data race (C's plain accesses race only with
    // each other, as in the C).
    /// `git_socket_stream__connect_timeout`.
    pub safe static git_socket_stream__connect_timeout: AtomicI32;
    /// `git_socket_stream__timeout`.
    pub safe static git_socket_stream__timeout: AtomicI32;
}

/// `git_openssl__set_cert_location`.
pub fn openssl_set_cert_location(file: Option<&CStr>, path: Option<&CStr>) -> Result<()> {
    // SAFETY: each is NULL (which `SSL_CTX_load_verify_locations`
    // accepts) or a NUL-terminated string borrowed for the call; the
    // certificates are loaded during the call, and no pointer is kept.
    check(unsafe { git_openssl__set_cert_location(opt_ptr(file), opt_ptr(path)) }).map(drop)
}

/// `git_openssl__add_x509_cert`.
///
/// # Safety
///
/// `cert` is NULL (nothing is added) or a valid `X509 *` for the call
/// (`X509_STORE_add_cert` takes its own reference).
pub unsafe fn openssl_add_x509_cert(cert: *mut c_void) -> Result<()> {
    // SAFETY: `cert` is NULL or a valid `X509 *` (this function's
    // contract), the only pointer the C reads.
    check(unsafe { git_openssl__add_x509_cert(cert) }).map(drop)
}
