//! Port of `src/libgit2/libgit2.c`: library initialization and shutdown,
//! the version, the features and their backends, the build information.
//!
//! `git_libgit2_init` runs the global init functions of every subsystem
//! in the C's order, Rust and C alike, through the runtime
//! ([`crate::util::runtime`]); the last `git_libgit2_shutdown` runs their
//! shutdown callbacks. The features are those of the build's
//! `git2_features.h` (read by `build.rs` into `cfg(git_feature = …)`).
//!
//! The safe API is [`Libgit2`], a guard that keeps the library initialized
//! while it lives, and [`version`], [`features`], [`feature_backend`],
//! [`buildinfo`].

use core::ffi::{CStr, c_char, c_int};

use crate::util::errors::{Error, ErrorCode};

/// `git_feature_t`.
pub mod feature {
    use core::ffi::c_int;
    /// `GIT_FEATURE_THREADS`.
    pub const THREADS: c_int = 1 << 0;
    /// `GIT_FEATURE_HTTPS`.
    pub const HTTPS: c_int = 1 << 1;
    /// `GIT_FEATURE_SSH`.
    pub const SSH: c_int = 1 << 2;
    /// `GIT_FEATURE_NSEC`.
    pub const NSEC: c_int = 1 << 3;
    /// `GIT_FEATURE_HTTP_PARSER`.
    pub const HTTP_PARSER: c_int = 1 << 4;
    /// `GIT_FEATURE_REGEX`.
    pub const REGEX: c_int = 1 << 5;
    /// `GIT_FEATURE_I18N`.
    pub const I18N: c_int = 1 << 6;
    /// `GIT_FEATURE_AUTH_NTLM`.
    pub const AUTH_NTLM: c_int = 1 << 7;
    /// `GIT_FEATURE_AUTH_NEGOTIATE`.
    pub const AUTH_NEGOTIATE: c_int = 1 << 8;
    /// `GIT_FEATURE_COMPRESSION`.
    pub const COMPRESSION: c_int = 1 << 9;
    /// `GIT_FEATURE_SHA1`.
    pub const SHA1: c_int = 1 << 10;
    /// `GIT_FEATURE_SHA256`.
    pub const SHA256: c_int = 1 << 11;
    /// `GIT_FEATURE_HTTP`.
    pub const HTTP: c_int = 1 << 12;
}

/// `GIT_BUILDINFO_CPU`.
pub const GIT_BUILDINFO_CPU: c_int = 1;
/// `GIT_BUILDINFO_COMMIT`.
pub const GIT_BUILDINFO_COMMIT: c_int = 2;

const fn parse_int(s: &str) -> c_int {
    let b = s.as_bytes();
    let mut i = 0;
    let mut v: c_int = 0;
    while i < b.len() {
        v = v * 10 + (b[i] - b'0') as c_int;
        i += 1;
    }
    v
}

/// `LIBGIT2_VERSION_MAJOR`.
pub const VERSION_MAJOR: c_int = parse_int(env!("LIBGIT2_VERSION_MAJOR"));
/// `LIBGIT2_VERSION_MINOR`.
pub const VERSION_MINOR: c_int = parse_int(env!("LIBGIT2_VERSION_MINOR"));
/// `LIBGIT2_VERSION_REVISION`.
pub const VERSION_REVISION: c_int = parse_int(env!("LIBGIT2_VERSION_REVISION"));
/// `LIBGIT2_VERSION`.
pub const VERSION: &str = env!("LIBGIT2_VERSION");

/// `LIBGIT2_VERSION_PRERELEASE`: `None` for `NULL`.
pub fn prerelease() -> Option<&'static CStr> {
    const P: &str = env!("LIBGIT2_VERSION_PRERELEASE");
    const C: &[u8] = concat!(env!("LIBGIT2_VERSION_PRERELEASE"), "\0").as_bytes();
    if P == "NULL" {
        None
    } else {
        CStr::from_bytes_with_nul(C).ok()
    }
}

/// `git_libgit2_features`.
pub const fn features() -> c_int {
    let mut f = 0;
    if cfg!(git_feature = "THREADS") {
        f |= feature::THREADS;
    }
    if cfg!(git_feature = "HTTP") {
        f |= feature::HTTP;
    }
    if cfg!(git_feature = "HTTPS") {
        f |= feature::HTTPS;
    }
    if cfg!(git_feature = "SSH") {
        f |= feature::SSH;
    }
    if cfg!(git_feature = "NSEC") {
        f |= feature::NSEC;
    }
    f |= feature::HTTP_PARSER | feature::REGEX;
    if cfg!(git_feature = "I18N_ICONV") {
        f |= feature::I18N;
    }
    if cfg!(git_feature = "AUTH_NTLM") {
        f |= feature::AUTH_NTLM;
    }
    if cfg!(git_feature = "AUTH_NEGOTIATE") {
        f |= feature::AUTH_NEGOTIATE;
    }
    f | feature::COMPRESSION | feature::SHA1 | feature::SHA256
}

macro_rules! first_of {
    ($($cfg:literal => $v:literal),* $(,)?) => {{
        let mut r: Option<&'static CStr> = None;
        $( if r.is_none() && cfg!(git_feature = $cfg) { r = Some($v); } )*
        r
    }};
}

/// `git_libgit2_feature_backend`: the backend of a feature, `None` when it
/// is not built in (or for an unknown feature). As C, a feature that is
/// built in with an unknown backend sets an assertion error.
pub fn feature_backend(f: c_int) -> Option<&'static CStr> {
    let unknown = |enabled: bool, what: &'static str| {
        if enabled {
            let _ = crate::util::errors::assert_failed(what);
        }
        None
    };
    match f {
        feature::THREADS => first_of!("THREADS_PTHREADS" => c"pthread", "THREADS_WIN32" => c"win32")
            .or_else(|| unknown(cfg!(git_feature = "THREADS"), "!\"Unknown threads backend\"")),
        feature::HTTP => None,
        feature::HTTPS => first_of!(
            "HTTPS_OPENSSL" => c"openssl",
            "HTTPS_OPENSSL_DYNAMIC" => c"openssl-dynamic",
            "HTTPS_MBEDTLS" => c"mbedtls",
            "HTTPS_SECURETRANSPORT" => c"securetransport",
            "HTTPS_SCHANNEL" => c"schannel",
            "HTTPS_WINHTTP" => c"winhttp",
        )
        .or_else(|| unknown(cfg!(git_feature = "HTTPS"), "!\"Unknown HTTPS backend\"")),
        feature::SSH => first_of!("SSH_EXEC" => c"exec", "SSH_LIBSSH2" => c"libssh2")
            .or_else(|| unknown(cfg!(git_feature = "SSH"), "!\"Unknown SSH backend\"")),
        feature::NSEC => first_of!(
            "NSEC_MTIMESPEC" => c"mtimespec",
            "NSEC_MTIM" => c"mtim",
            "NSEC_MTIME_NSEC" => c"mtime_nsec",
            "NSEC_WIN32" => c"win32",
        )
        .or_else(|| unknown(cfg!(git_feature = "NSEC"), "!\"Unknown high-resolution time backend\"")),
        feature::HTTP_PARSER => first_of!(
            "HTTPPARSER_HTTPPARSER" => c"httpparser",
            "HTTPPARSER_LLHTTP" => c"llhttp",
            "HTTPPARSER_BUILTIN" => c"builtin",
        )
        .or_else(|| unknown(true, "!\"Unknown HTTP parser backend\"")),
        feature::REGEX => first_of!(
            "REGEX_REGCOMP_L" => c"regcomp_l",
            "REGEX_REGCOMP" => c"regcomp",
            "REGEX_PCRE" => c"pcre",
            "REGEX_PCRE2" => c"pcre2",
            "REGEX_BUILTIN" => c"builtin",
        )
        .or_else(|| unknown(true, "!\"Unknown regular expression backend\"")),
        feature::I18N => first_of!("I18N_ICONV" => c"iconv")
            .or_else(|| unknown(cfg!(git_feature = "I18N"), "!\"Unknown internationalization backend\"")),
        feature::AUTH_NTLM => first_of!("AUTH_NTLM_BUILTIN" => c"builtin", "AUTH_NTLM_SSPI" => c"sspi")
            .or_else(|| unknown(cfg!(git_feature = "AUTH_NTLM"), "!\"Unknown NTLM backend\"")),
        feature::AUTH_NEGOTIATE => first_of!(
            "AUTH_NEGOTIATE_GSSFRAMEWORK" => c"gssframework",
            "AUTH_NEGOTIATE_GSSAPI" => c"gssapi",
            "AUTH_NEGOTIATE_SSPI" => c"sspi",
        )
        .or_else(|| unknown(cfg!(git_feature = "AUTH_NEGOTIATE"), "!\"Unknown Negotiate backend\"")),
        feature::COMPRESSION => first_of!("COMPRESSION_ZLIB" => c"zlib", "COMPRESSION_BUILTIN" => c"builtin")
            .or_else(|| unknown(true, "!\"Unknown compression backend\"")),
        feature::SHA1 => first_of!(
            "SHA1_BUILTIN" => c"builtin",
            "SHA1_OPENSSL" => c"openssl",
            "SHA1_OPENSSL_FIPS" => c"openssl-fips",
            "SHA1_OPENSSL_DYNAMIC" => c"openssl-dynamic",
            "SHA1_MBEDTLS" => c"mbedtls",
            "SHA1_COMMON_CRYPTO" => c"commoncrypto",
            "SHA1_WIN32" => c"win32",
        )
        .or_else(|| unknown(true, "!\"Unknown SHA1 backend\"")),
        feature::SHA256 => first_of!(
            "SHA256_BUILTIN" => c"builtin",
            "SHA256_OPENSSL" => c"openssl",
            "SHA256_OPENSSL_FIPS" => c"openssl-fips",
            "SHA256_OPENSSL_DYNAMIC" => c"openssl-dynamic",
            "SHA256_MBEDTLS" => c"mbedtls",
            "SHA256_COMMON_CRYPTO" => c"commoncrypto",
            "SHA256_WIN32" => c"win32",
        )
        .or_else(|| unknown(true, "!\"Unknown SHA256 backend\"")),
        _ => None,
    }
}

/// `git_libgit2_buildinfo`.
pub fn buildinfo(key: c_int) -> Option<&'static CStr> {
    match key {
        GIT_BUILDINFO_CPU => build_cpu(),
        GIT_BUILDINFO_COMMIT => build_commit(),
        _ => None,
    }
}

#[cfg(git_build_cpu)]
fn build_cpu() -> Option<&'static CStr> {
    CStr::from_bytes_with_nul(concat!(env!("LIBGIT2_BUILD_CPU"), "\0").as_bytes()).ok()
}

#[cfg(not(git_build_cpu))]
fn build_cpu() -> Option<&'static CStr> {
    None
}

#[cfg(git_build_commit)]
fn build_commit() -> Option<&'static CStr> {
    CStr::from_bytes_with_nul(concat!(env!("LIBGIT2_BUILD_COMMIT"), "\0").as_bytes()).ok()
}

#[cfg(not(git_build_commit))]
fn build_commit() -> Option<&'static CStr> {
    None
}

/// The global init functions, in C's order (`git_threads_global_init` is
/// an inline no-op with pthreads, and `git_win32_leakcheck_global_init`
/// is Windows-only).
fn init_fns() -> [crate::util::runtime::InitFn; 17] {
    use crate::sys;
    [
        crate::util::alloc::ffi::git_allocator_global_init,
        crate::util::errors::ffi::git_error_global_init,
        crate::oid::ffi::git_oid_global_init,
        crate::util::rand::ffi::git_rand_global_init,
        crate::util::hash::ffi::git_hash_global_init,
        sys::sysdir::git_sysdir_global_init,
        sys::filter::git_filter_global_init,
        sys::merge_driver::git_merge_driver_global_init,
        sys::transports::git_transport_ssh_libssh2_global_init,
        sys::streams::git_stream_registry_global_init,
        sys::streams::git_socket_stream_global_init,
        sys::streams::git_openssl_stream_global_init,
        sys::streams::git_mbedtls_stream_global_init,
        // PENDING(l-pack): `crate::mwindow::global_init`.
        sys::pending::l_repo::git_mwindow_global_init,
        crate::util::pool::ffi::git_pool_global_init,
        crate::settings::ffi::git_settings_global_init,
        sys::refdb_reftable::git_reftable_global_init,
    ]
}

/// `git_libgit2_init`: the new init count, or a negative error.
pub fn init() -> c_int {
    let fns = init_fns();
    // SAFETY: each is a subsystem's global init function, sound to call at
    // initialization with the runtime's lock held (as in C).
    unsafe { crate::util::runtime::init(&fns) }
}

/// `git_libgit2_shutdown`: the remaining init count.
pub fn shutdown() -> c_int {
    crate::util::runtime::shutdown()
}

/// A guard that keeps libgit2 initialized while it lives
/// (`git_libgit2_init` / `git_libgit2_shutdown`).
#[derive(Debug)]
pub struct Libgit2 {
    _p: (),
}

impl Libgit2 {
    /// Initialize the library (or count one more user).
    pub fn init() -> Result<Libgit2, Error> {
        let r = init();
        if r < 0 {
            return Err(Error::from(ErrorCode::check(r).err().unwrap_or(ErrorCode::GENERIC)));
        }
        Ok(Libgit2 { _p: () })
    }
}

impl Drop for Libgit2 {
    fn drop(&mut self) {
        shutdown();
    }
}

/// `git_libgit2_version`: (major, minor, revision).
pub const fn version() -> (c_int, c_int, c_int) {
    (VERSION_MAJOR, VERSION_MINOR, VERSION_REVISION)
}

/// The exports of `libgit2.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// `git_libgit2_init`.
    ///
    /// # Safety
    ///
    /// None (it may be called at any time, from any thread).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_init() -> c_int {
        barrier(init)
    }

    /// `git_libgit2_shutdown`.
    ///
    /// # Safety
    ///
    /// Nothing of the library is used after the last shutdown.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_shutdown() -> c_int {
        barrier(shutdown)
    }

    /// `git_libgit2_version`.
    ///
    /// # Safety
    ///
    /// The three pointers are writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_version(major: *mut c_int, minor: *mut c_int, rev: *mut c_int) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe {
                major.write(VERSION_MAJOR);
                minor.write(VERSION_MINOR);
                rev.write(VERSION_REVISION);
            }
            0
        })
    }

    /// `git_libgit2_prerelease`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_prerelease() -> *const c_char {
        barrier(|| prerelease().map_or(core::ptr::null(), CStr::as_ptr))
    }

    /// `git_libgit2_features`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_features() -> c_int {
        features()
    }

    /// `git_libgit2_feature_backend`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_feature_backend(feature: c_int) -> *const c_char {
        barrier(|| feature_backend(feature).map_or(core::ptr::null(), CStr::as_ptr))
    }

    /// `git_libgit2_buildinfo`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2_buildinfo(key: c_int) -> *const c_char {
        barrier(|| buildinfo(key).map_or(core::ptr::null(), CStr::as_ptr))
    }
}
