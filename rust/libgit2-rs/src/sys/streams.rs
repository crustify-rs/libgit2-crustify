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

    /// `git_socket_stream__connect_timeout`.
    pub static git_socket_stream__connect_timeout: AtomicI32;
    /// `git_socket_stream__timeout`.
    pub static git_socket_stream__timeout: AtomicI32;
}

/// `git_openssl__set_cert_location`.
pub fn openssl_set_cert_location(file: Option<&CStr>, path: Option<&CStr>) -> Result<()> {
    // SAFETY: NULL or NUL-terminated strings.
    check(unsafe { git_openssl__set_cert_location(opt_ptr(file), opt_ptr(path)) }).map(drop)
}

/// `git_openssl__add_x509_cert`.
///
/// # Safety
///
/// `cert` is a valid `X509 *` (OpenSSL's contract).
pub unsafe fn openssl_add_x509_cert(cert: *mut c_void) -> Result<()> {
    // SAFETY: the caller's contract.
    check(unsafe { git_openssl__add_x509_cert(cert) }).map(drop)
}
