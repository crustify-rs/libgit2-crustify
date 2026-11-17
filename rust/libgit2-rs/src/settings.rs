//! Port of `src/libgit2/settings.c`: the storage of the library's
//! options, and `git_libgit2_opts`.
//!
//! `git_libgit2_opts` is variadic: its C shim (`src/util/rust_shims.c`)
//! reads each option's arguments from the `va_list`, as the C did, into an
//! [`OptArgs`], and calls [`ffi::git_libgit2__opts`], which does the rest.
//! The options are mostly globals of other modules (`git_mwindow__*`,
//! `git_cache__*`, …): the ported modules' Rust statics, or the C globals,
//! declared in `sys`, of the modules that stay C (sysdir, streams,
//! transports, indexer); the
//! user agents and the TLS ciphers are this module's (`git__user_agent`,
//! `git__user_agent_product`, `git__ssl_ciphers`, read by C).
//!
//! The safe API is `crate::api::opts` (`libgit2_rs::opts`).

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::buf::GitBuf;
use crate::git_error_set;
use crate::util::alloc;
use crate::util::cabi::CStrArg;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;

/// `git__user_agent` (`char *`, NULL for the default).
#[unsafe(no_mangle)]
pub static git__user_agent: AtomicPtr<c_char> = AtomicPtr::new(ptr::null_mut());
/// `git__user_agent_product`.
#[unsafe(no_mangle)]
pub static git__user_agent_product: AtomicPtr<c_char> = AtomicPtr::new(ptr::null_mut());
/// `git__ssl_ciphers` (read by `streams/openssl.c`).
#[unsafe(no_mangle)]
pub static git__ssl_ciphers: AtomicPtr<c_char> = AtomicPtr::new(ptr::null_mut());

const DEFAULT_USER_AGENT: &CStr = {
    match CStr::from_bytes_with_nul(concat!("libgit2 ", env!("LIBGIT2_VERSION"), "\0").as_bytes()) {
        Ok(c) => c,
        Err(_) => c"libgit2",
    }
};

/// Serializes the Rust's writers and readers of the string globals: a
/// safe reader copies under it, so no setter can free what it reads. (C's
/// readers, `git_settings__user_agent` and `streams/openssl.c`, keep C's
/// rule: no options call while they use the string.)
static STRINGS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn strings_lock() -> std::sync::MutexGuard<'static, ()> {
    STRINGS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Replace a string global, freeing the old value.
fn replace(g: &AtomicPtr<c_char>, v: Option<&CStr>) -> Result<()> {
    let _l = strings_lock();
    // C frees the old value first, then copies (so the global is NULL if
    // the copy fails).
    let new = match v {
        Some(s) => {
            let p = alloc::strdup(s);
            if p.is_null() {
                let old = g.swap(ptr::null_mut(), Ordering::AcqRel);
                // SAFETY: the global's own string.
                unsafe { alloc::free(old.cast()) };
                return Err(ErrorCode::GENERIC);
            }
            p
        }
        None => ptr::null_mut(),
    };
    let old = g.swap(new, Ordering::AcqRel);
    // SAFETY: the global's previous string (from `git__allocator`), which
    // C does not keep past an options call.
    unsafe { alloc::free(old.cast()) };
    Ok(())
}

/// A copy of a string global (or `default`), taken under the lock the
/// setters take.
fn copy(g: &AtomicPtr<c_char>, default: &CStr) -> Vec<u8> {
    let _l = strings_lock();
    let p = g.load(Ordering::Acquire);
    if p.is_null() {
        return default.to_bytes().to_vec();
    }
    // SAFETY: the global's string, which no setter frees while the lock is
    // held.
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

/// The string global (or `default`), as C reads it: the pointer only (C
/// may use it until the next options call that replaces the global, or
/// the last `git_libgit2_shutdown`).
fn read(g: &AtomicPtr<c_char>, default: &'static CStr) -> *const c_char {
    let p = g.load(Ordering::Acquire);
    if p.is_null() { default.as_ptr() } else { p }
}

/// `git_settings__user_agent`: the user agent (a copy).
pub fn user_agent() -> Vec<u8> {
    copy(&git__user_agent, DEFAULT_USER_AGENT)
}

/// `git_settings__user_agent_product`: the product (a copy).
pub fn user_agent_product() -> Vec<u8> {
    copy(&git__user_agent_product, c"git/2.0")
}

extern "C" fn settings_global_shutdown() {
    let _l = strings_lock();
    for g in [
        &git__user_agent,
        &git__user_agent_product,
        &git__ssl_ciphers,
    ] {
        let old = g.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: the global's own string, no longer used at shutdown.
        unsafe { alloc::free(old.cast()) };
    }
    crate::repository::ffi_free_extensions();
}

/// `config_level_to_sysdir`.
fn config_level_to_sysdir(level: c_int) -> Result<c_int> {
    use crate::sys::sysdir::*;
    match level {
        crate::repository::GIT_CONFIG_LEVEL_SYSTEM => Ok(GIT_SYSDIR_SYSTEM),
        crate::repository::GIT_CONFIG_LEVEL_XDG => Ok(GIT_SYSDIR_XDG),
        crate::repository::GIT_CONFIG_LEVEL_GLOBAL => Ok(GIT_SYSDIR_GLOBAL),
        _ => Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid config path selector {}",
            level
        )),
    }
}

/// The arguments of one `git_libgit2_opts` call, as the shim read them.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OptArgs {
    /// An `int` argument.
    pub i: c_int,
    /// A `size_t` argument.
    pub sz: usize,
    /// An `ssize_t` argument.
    pub ssz: isize,
    /// The first pointer argument.
    pub p1: *mut c_void,
    /// The second pointer argument.
    pub p2: *mut c_void,
}

/// `git_libgit2_opt_t`.
#[allow(missing_docs)]
pub mod opt {
    use core::ffi::c_int;
    pub const GET_MWINDOW_SIZE: c_int = 0;
    pub const SET_MWINDOW_SIZE: c_int = 1;
    pub const GET_MWINDOW_MAPPED_LIMIT: c_int = 2;
    pub const SET_MWINDOW_MAPPED_LIMIT: c_int = 3;
    pub const GET_SEARCH_PATH: c_int = 4;
    pub const SET_SEARCH_PATH: c_int = 5;
    pub const SET_CACHE_OBJECT_LIMIT: c_int = 6;
    pub const SET_CACHE_MAX_SIZE: c_int = 7;
    pub const ENABLE_CACHING: c_int = 8;
    pub const GET_CACHED_MEMORY: c_int = 9;
    pub const GET_TEMPLATE_PATH: c_int = 10;
    pub const SET_TEMPLATE_PATH: c_int = 11;
    pub const SET_SSL_CERT_LOCATIONS: c_int = 12;
    pub const SET_USER_AGENT: c_int = 13;
    pub const ENABLE_STRICT_OBJECT_CREATION: c_int = 14;
    pub const ENABLE_STRICT_SYMBOLIC_REF_CREATION: c_int = 15;
    pub const SET_SSL_CIPHERS: c_int = 16;
    pub const GET_USER_AGENT: c_int = 17;
    pub const ENABLE_OFS_DELTA: c_int = 18;
    pub const ENABLE_FSYNC_GITDIR: c_int = 19;
    pub const GET_WINDOWS_SHAREMODE: c_int = 20;
    pub const SET_WINDOWS_SHAREMODE: c_int = 21;
    pub const ENABLE_STRICT_HASH_VERIFICATION: c_int = 22;
    pub const SET_ALLOCATOR: c_int = 23;
    pub const ENABLE_UNSAVED_INDEX_SAFETY: c_int = 24;
    pub const GET_PACK_MAX_OBJECTS: c_int = 25;
    pub const SET_PACK_MAX_OBJECTS: c_int = 26;
    pub const DISABLE_PACK_KEEP_FILE_CHECKS: c_int = 27;
    pub const ENABLE_HTTP_EXPECT_CONTINUE: c_int = 28;
    pub const GET_MWINDOW_FILE_LIMIT: c_int = 29;
    pub const SET_MWINDOW_FILE_LIMIT: c_int = 30;
    pub const SET_ODB_PACKED_PRIORITY: c_int = 31;
    pub const SET_ODB_LOOSE_PRIORITY: c_int = 32;
    pub const GET_EXTENSIONS: c_int = 33;
    pub const SET_EXTENSIONS: c_int = 34;
    pub const GET_OWNER_VALIDATION: c_int = 35;
    pub const SET_OWNER_VALIDATION: c_int = 36;
    pub const GET_HOMEDIR: c_int = 37;
    pub const SET_HOMEDIR: c_int = 38;
    pub const SET_SERVER_CONNECT_TIMEOUT: c_int = 39;
    pub const GET_SERVER_CONNECT_TIMEOUT: c_int = 40;
    pub const SET_SERVER_TIMEOUT: c_int = 41;
    pub const GET_SERVER_TIMEOUT: c_int = 42;
    pub const SET_USER_AGENT_PRODUCT: c_int = 43;
    pub const GET_USER_AGENT_PRODUCT: c_int = 44;
    pub const ADD_SSL_X509_CERT: c_int = 45;
    pub const GET_PACK_MAX_OBJECT_SIZE: c_int = 46;
    pub const SET_PACK_MAX_OBJECT_SIZE: c_int = 47;
}

/// Write a `git_buf` as C's options do: `git_buf_tostr`, fill, then
/// `git_buf_fromstr` (C's `GIT_BUF_WRAP_PRIVATE`).
///
/// # Safety
///
/// `out` is NULL (rejected) or a valid `git_buf` that nothing else
/// accesses during the call.
unsafe fn buf_out(out: *mut GitBuf, f: impl FnOnce(&mut GitStr) -> Result<()>) -> Result<()> {
    // SAFETY: this function's contract is `wrap_private`'s.
    ErrorCode::check(unsafe { crate::buf::wrap_private(out, f) }).map(drop)
}

/// The globals of other modules the options read and write. Each is a C
/// global (`size_t`, `int`, `bool`, …) that the Rust reaches as the atomic
/// of the same layout.
mod globals {
    pub(super) use crate::sys::indexer::*;
    pub(super) use crate::sys::streams::{
        git_socket_stream__connect_timeout, git_socket_stream__timeout,
    };
    pub(super) use crate::sys::transports::{
        git_http__expect_continue, git_smart__ofs_delta_enabled,
    };
}

/// `git_indexer__max_objects` (declared in `sys::indexer`).
fn indexer_max_objects() -> &'static core::sync::atomic::AtomicUsize {
    // SAFETY: a `size_t` that `indexer.c` defines, always initialized,
    // reached as the `AtomicUsize` of the same layout: every bit pattern is
    // valid, and atomic accesses do not race with C's as Rust data races.
    unsafe { &globals::git_indexer__max_objects }
}

/// `git_indexer__max_object_size` (declared in `sys::indexer`).
fn indexer_max_object_size() -> &'static core::sync::atomic::AtomicUsize {
    // SAFETY: as in `indexer_max_objects`.
    unsafe { &globals::git_indexer__max_object_size }
}

/// Write `v` to a C out-pointer (C's `*(va_arg(ap, T *)) = v`), rejecting
/// NULL (where C crashes).
///
/// # Safety
///
/// `p` is NULL, or aligned and valid for writes of a `T`, with no
/// reference to the pointee live.
unsafe fn out<T>(p: *mut c_void, v: T) -> Result<()> {
    if p.is_null() {
        return Err(errors::assert_arg_failed("out"));
    }
    // SAFETY: non-null here, so aligned and writable for a `T` with no
    // reference to it live (this function's contract).
    unsafe { p.cast::<T>().write(v) };
    Ok(())
}

/// The string argument `p` (NULL is `None`).
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that stays readable, and
/// unwritten, for `'a`.
unsafe fn cstr<'a>(p: *mut c_void) -> Option<&'a CStr> {
    // SAFETY: this function's contract is `CStrArg::from_ptr`'s.
    unsafe { CStrArg::from_ptr(p.cast()) }.get()
}

/// `git_libgit2_opts(key, …)` with the arguments read.
///
/// # Safety
///
/// `args` holds the arguments C's `git_libgit2_opts` documents for `key`,
/// read by the shim into the fields it reads for `key`: each pointer is
/// NULL or valid for what the option does with it for the call (an out
/// pointer aligned and writable for the option's type — `size_t`,
/// `ssize_t` or `int` — with no reference to it live; a `git_buf *` a
/// valid buffer; a `const char *` a NUL-terminated string; an `X509 *`, a
/// `git_allocator *`, a `git_strarray *` or `len` strings as the option
/// says).
pub unsafe fn opts(key: c_int, args: &OptArgs) -> Result<()> {
    use crate::sys::sysdir::{self, GIT_SYSDIR_HOME, GIT_SYSDIR_TEMPLATE};
    use opt::*;
    let r = Ordering::Relaxed;
    let a = args;
    // Every `unsafe` block below reads or writes a pointer argument of
    // `key`, which this function's contract makes NULL or valid for that
    // use: the SAFETY comments name the C type the option documents.
    match key {
        SET_MWINDOW_SIZE => crate::mwindow::git_mwindow__window_size.store(a.sz, r),
        GET_MWINDOW_SIZE => {
            let v = crate::mwindow::git_mwindow__window_size.load(r);
            // SAFETY: `p1` is NULL or a `size_t *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_MWINDOW_MAPPED_LIMIT => crate::mwindow::git_mwindow__mapped_limit.store(a.sz, r),
        GET_MWINDOW_MAPPED_LIMIT => {
            let v = crate::mwindow::git_mwindow__mapped_limit.load(r);
            // SAFETY: `p1` is NULL or a `size_t *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_MWINDOW_FILE_LIMIT => crate::mwindow::git_mwindow__file_limit.store(a.sz, r),
        GET_MWINDOW_FILE_LIMIT => {
            let v = crate::mwindow::git_mwindow__file_limit.load(r);
            // SAFETY: `p1` is NULL or a `size_t *` for this key.
            unsafe { out(a.p1, v) }?
        }
        GET_SEARCH_PATH => {
            let f = |s: &mut GitStr| {
                let level = config_level_to_sysdir(a.i)?;
                sysdir::get_into(s, level)
            };
            // SAFETY: `p1` is NULL or a valid `git_buf *` for this key.
            unsafe { buf_out(a.p1.cast(), f) }?
        }
        SET_SEARCH_PATH => {
            let level = config_level_to_sysdir(a.i)?;
            // SAFETY: for a valid level (checked just above, as the shim
            // did before reading it) `p1` is NULL or a string.
            sysdir::set(level, unsafe { cstr(a.p1) })?;
        }
        SET_CACHE_OBJECT_LIMIT => {
            crate::cache::set_max_object_size(a.i, a.sz)?;
        }
        SET_CACHE_MAX_SIZE => crate::cache::git_cache__max_storage.store(a.ssz, r),
        ENABLE_CACHING => crate::cache::git_cache__enabled.store(a.i != 0, r),
        GET_CACHED_MEMORY => {
            let v = crate::cache::git_cache__current_storage.load(Ordering::SeqCst);
            // SAFETY: `p1` and `p2` are NULL or `ssize_t *` for this key.
            unsafe { out(a.p1, v) }?;
            let v = crate::cache::git_cache__max_storage.load(r);
            // SAFETY: as above.
            unsafe { out(a.p2, v) }?;
        }
        GET_TEMPLATE_PATH => {
            let f = |s: &mut GitStr| sysdir::get_into(s, GIT_SYSDIR_TEMPLATE);
            // SAFETY: `p1` is NULL or a valid `git_buf *` for this key.
            unsafe { buf_out(a.p1.cast(), f) }?
        }
        SET_TEMPLATE_PATH => {
            // SAFETY: `p1` is NULL or a string for this key.
            sysdir::set(GIT_SYSDIR_TEMPLATE, unsafe { cstr(a.p1) })?
        }
        SET_SSL_CERT_LOCATIONS => {
            // SAFETY: `p1` is NULL or a string for this key.
            let file = unsafe { cstr(a.p1) };
            // SAFETY: `p2` is NULL or a string for this key.
            let path = unsafe { cstr(a.p2) };
            crate::sys::streams::openssl_set_cert_location(file, path)?
        }
        ADD_SSL_X509_CERT => {
            // SAFETY: `p1` is NULL or an `X509 *` for this key.
            unsafe { crate::sys::streams::openssl_add_x509_cert(a.p1) }?
        }
        SET_USER_AGENT => {
            // SAFETY: `p1` is NULL or a string for this key.
            replace(&git__user_agent, unsafe { cstr(a.p1) })?
        }
        GET_USER_AGENT => {
            let f = |s: &mut GitStr| s.puts(&user_agent());
            // SAFETY: `p1` is NULL or a valid `git_buf *` for this key.
            unsafe { buf_out(a.p1.cast(), f) }?
        }
        SET_USER_AGENT_PRODUCT => {
            // SAFETY: `p1` is NULL or a string for this key.
            replace(&git__user_agent_product, unsafe { cstr(a.p1) })?
        }
        GET_USER_AGENT_PRODUCT => {
            let f = |s: &mut GitStr| s.puts(&user_agent_product());
            // SAFETY: `p1` is NULL or a valid `git_buf *` for this key.
            unsafe { buf_out(a.p1.cast(), f) }?
        }
        ENABLE_STRICT_OBJECT_CREATION => {
            crate::object::git_object__strict_input_validation.store(a.i != 0, r)
        }
        ENABLE_STRICT_SYMBOLIC_REF_CREATION => {
            crate::refs::git_reference__enable_symbolic_ref_target_validation.store(a.i != 0, r)
        }
        SET_SSL_CIPHERS => {
            // C copies with `git__strdup` and reports its failure as
            // out-of-memory; a NULL list (which crashes the C) clears
            // the setting.
            // SAFETY: `p1` is NULL or a string for this key.
            if replace(&git__ssl_ciphers, unsafe { cstr(a.p1) }).is_err() {
                return Err(errors::oom());
            }
        }
        ENABLE_OFS_DELTA => globals::git_smart__ofs_delta_enabled.store(a.i != 0, r),
        ENABLE_FSYNC_GITDIR => crate::repository::git_repository__fsync_gitdir.store(a.i != 0, r),
        GET_WINDOWS_SHAREMODE | SET_WINDOWS_SHAREMODE => {}
        ENABLE_STRICT_HASH_VERIFICATION => {
            crate::odb::git_odb__strict_hash_verification.store(a.i != 0, r)
        }
        SET_ALLOCATOR => {
            // SAFETY: `p1` is NULL or a valid `git_allocator *` for this
            // key, which is what the export requires.
            ErrorCode::check(unsafe { crate::util::alloc::ffi::git_allocator_setup(a.p1.cast()) })?;
        }
        ENABLE_UNSAVED_INDEX_SAFETY => {
            crate::index::git_index__enforce_unsaved_safety.store(a.i != 0, r)
        }
        SET_PACK_MAX_OBJECTS => indexer_max_objects().store(a.sz, r),
        GET_PACK_MAX_OBJECTS => {
            let v = indexer_max_objects().load(r);
            // SAFETY: `p1` is NULL or a `size_t *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_PACK_MAX_OBJECT_SIZE => indexer_max_object_size().store(a.sz, r),
        GET_PACK_MAX_OBJECT_SIZE => {
            let v = indexer_max_object_size().load(r);
            // SAFETY: `p1` is NULL or a `size_t *` for this key.
            unsafe { out(a.p1, v) }?
        }
        DISABLE_PACK_KEEP_FILE_CHECKS => {
            crate::pack::git_disable_pack_keep_file_checks.store(a.i != 0, r)
        }
        ENABLE_HTTP_EXPECT_CONTINUE => globals::git_http__expect_continue.store(a.i != 0, r),
        SET_ODB_PACKED_PRIORITY => crate::odb::git_odb__packed_priority.store(a.i, r),
        SET_ODB_LOOSE_PRIORITY => crate::odb::git_odb__loose_priority.store(a.i, r),
        SET_EXTENSIONS => {
            // SAFETY: for this key `p1` is a `const char **` of `sz`
            // strings (or NULL with 0), what the export requires.
            ErrorCode::check(unsafe {
                crate::repository::ffi::git_repository__set_extensions(a.p1.cast(), a.sz)
            })?;
        }
        GET_EXTENSIONS => {
            let arr = a.p1.cast::<crate::strarray::GitStrArray>();
            if arr.is_null() {
                return Err(errors::assert_arg_failed("out"));
            }
            // SAFETY: `arr` is non-null, so a writable `git_strarray *` for
            // this key: the projection stays in its allocation (and forms
            // no reference).
            let strings = unsafe { &raw mut (*arr).strings };
            // SAFETY: as above.
            let count = unsafe { &raw mut (*arr).count };
            // SAFETY: two writable, aligned, distinct fields of `*arr`,
            // which nothing else references during the call.
            ErrorCode::check(unsafe {
                crate::repository::ffi::git_repository__extensions(strings, count)
            })?;
        }
        GET_OWNER_VALIDATION => {
            let v = c_int::from(crate::repository::git_repository__validate_ownership.load(r));
            // SAFETY: `p1` is NULL or an `int *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_OWNER_VALIDATION => {
            crate::repository::git_repository__validate_ownership.store(a.i != 0, r)
        }
        GET_HOMEDIR => {
            let f = |s: &mut GitStr| sysdir::get_into(s, GIT_SYSDIR_HOME);
            // SAFETY: `p1` is NULL or a valid `git_buf *` for this key.
            unsafe { buf_out(a.p1.cast(), f) }?
        }
        SET_HOMEDIR => {
            // SAFETY: `p1` is NULL or a string for this key.
            sysdir::set(GIT_SYSDIR_HOME, unsafe { cstr(a.p1) })?
        }
        GET_SERVER_CONNECT_TIMEOUT => {
            let v = globals::git_socket_stream__connect_timeout.load(r);
            // SAFETY: `p1` is NULL or an `int *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_SERVER_CONNECT_TIMEOUT => {
            if a.i < 0 {
                return Err(git_error_set!(
                    ErrorClass::INVALID,
                    "invalid connect timeout"
                ));
            }
            globals::git_socket_stream__connect_timeout.store(a.i, r);
        }
        GET_SERVER_TIMEOUT => {
            let v = globals::git_socket_stream__timeout.load(r);
            // SAFETY: `p1` is NULL or an `int *` for this key.
            unsafe { out(a.p1, v) }?
        }
        SET_SERVER_TIMEOUT => {
            if a.i < 0 {
                return Err(git_error_set!(ErrorClass::INVALID, "invalid timeout"));
            }
            globals::git_socket_stream__timeout.store(a.i, r);
        }
        _ => return Err(git_error_set!(ErrorClass::INVALID, "invalid option key")),
    }
    Ok(())
}

/// `GIT_OPT_SET_USER_AGENT` (`None`: the default), for the safe API.
pub(crate) fn set_user_agent(agent: Option<&CStr>) -> Result<()> {
    replace(&git__user_agent, agent)
}

/// `GIT_OPT_SET_USER_AGENT_PRODUCT` (`None`: the default).
pub(crate) fn set_user_agent_product(product: Option<&CStr>) -> Result<()> {
    replace(&git__user_agent_product, product)
}

/// The exports of `settings.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::export::{barrier, status};

    /// `git_settings_global_init`.
    ///
    /// # Safety
    ///
    /// Called from `git_libgit2_init`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_settings_global_init() -> c_int {
        barrier(|| {
            status(crate::util::runtime::shutdown_register(
                settings_global_shutdown,
            ))
        })
    }

    /// `git_settings__user_agent`.
    ///
    /// # Safety
    ///
    /// The result is valid until the next `GIT_OPT_SET_USER_AGENT`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_settings__user_agent() -> *const c_char {
        read(&git__user_agent, DEFAULT_USER_AGENT)
    }

    /// `git_settings__user_agent_product`.
    ///
    /// # Safety
    ///
    /// As [`git_settings__user_agent`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_settings__user_agent_product() -> *const c_char {
        read(&git__user_agent_product, c"git/2.0")
    }

    /// `git_libgit2_opts`' body: the shim in `rust_shims.c` read the
    /// arguments of `key` into `args`.
    ///
    /// # Safety
    ///
    /// `args` is valid, and holds what C's `git_libgit2_opts` documents
    /// for `key`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2__opts(key: c_int, args: Option<&OptArgs>) -> c_int {
        barrier(|| {
            let args = git_arg!(args, "args", -1);
            // SAFETY: `args` holds what C documents for `key` (this
            // export's contract, which the shim's `va_arg` reads keep),
            // which is `opts`' contract.
            status(unsafe { opts(key, args) })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support;

    fn args() -> OptArgs {
        OptArgs {
            i: 0,
            sz: 0,
            ssz: 0,
            p1: ptr::null_mut(),
            p2: ptr::null_mut(),
        }
    }

    #[test]
    fn user_agents() {
        let _g = test_support::init();
        let _l = AGENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(user_agent(), b"libgit2 1.9.0");
        assert_eq!(user_agent_product(), b"git/2.0");
        set_user_agent(Some(c"agent/1")).unwrap();
        assert_eq!(user_agent(), b"agent/1");
        set_user_agent(None).unwrap();
        assert_eq!(user_agent(), b"libgit2 1.9.0");
        let mut a = args();
        a.p1 = c"prod/2".as_ptr().cast_mut().cast();
        // SAFETY: a string argument.
        unsafe { opts(opt::SET_USER_AGENT_PRODUCT, &a) }.unwrap();
        assert_eq!(user_agent_product(), b"prod/2");
        a.p1 = ptr::null_mut();
        // SAFETY: NULL: back to the default.
        unsafe { opts(opt::SET_USER_AGENT_PRODUCT, &a) }.unwrap();
        assert_eq!(user_agent_product(), b"git/2.0");
    }

    /// Keeps the two tests that set the user agent from seeing each
    /// other's values (it orders expectations; memory safety does not
    /// depend on it: `user_agent_reader_races_setter` races its own
    /// threads without it).
    static AGENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Review l-repo-r1 LR-1: safe readers against a safe setter on another
    /// thread (a use after free and a data race under Miri before the fix).
    #[test]
    fn user_agent_reader_races_setter() {
        let _g = test_support::init();
        let _l = AGENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let t = std::thread::spawn(|| {
            for i in 0..20 {
                let v: &CStr = if i % 2 == 0 {
                    c"aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                } else {
                    c"b"
                };
                set_user_agent(Some(v)).unwrap();
            }
        });
        for _ in 0..20 {
            let v = user_agent();
            assert!(!v.is_empty());
        }
        t.join().unwrap();
        let held = user_agent();
        set_user_agent(None).unwrap();
        // A copy stays valid after the setter frees the global's string.
        assert!(held == b"b" || held.starts_with(b"aaaa"));
    }

    #[test]
    fn errors_as_c() {
        let _g = test_support::init();
        let a = args();
        // SAFETY: no argument is used.
        assert_eq!(unsafe { opts(9999, &a) }, Err(ErrorCode::GENERIC));
        assert_eq!(errors::last_message().unwrap(), b"invalid option key");
        assert_eq!(errors::last_class(), Some(ErrorClass::INVALID));
        assert!(config_level_to_sysdir(1).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid config path selector 1"
        );
        assert_eq!(
            config_level_to_sysdir(4),
            Ok(crate::sys::sysdir::GIT_SYSDIR_GLOBAL)
        );
        let mut a = args();
        a.i = -1;
        // SAFETY: an int argument.
        assert!(unsafe { opts(opt::SET_SERVER_TIMEOUT, &a) }.is_err());
        assert_eq!(errors::last_message().unwrap(), b"invalid timeout");
        // SAFETY: as above.
        assert!(unsafe { opts(opt::SET_SERVER_CONNECT_TIMEOUT, &a) }.is_err());
        assert_eq!(errors::last_message().unwrap(), b"invalid connect timeout");
        // Windows-only options do nothing.
        // SAFETY: no argument is used.
        unsafe { opts(opt::GET_WINDOWS_SHAREMODE, &args()) }.unwrap();
    }

    #[test]
    fn owner_validation_and_fsync() {
        let _g = test_support::init();
        let mut v: c_int = -1;
        let mut a = args();
        a.p1 = (&raw mut v).cast();
        // SAFETY: an `int *`.
        unsafe { opts(opt::GET_OWNER_VALIDATION, &a) }.unwrap();
        assert_eq!(v, c_int::from(crate::api::opts::owner_validation()));
        let mut b = args();
        b.i = 1;
        // SAFETY: an int.
        unsafe { opts(opt::ENABLE_FSYNC_GITDIR, &b) }.unwrap();
        assert!(crate::repository::git_repository__fsync_gitdir.load(Ordering::Relaxed));
        crate::api::opts::enable_fsync_gitdir(false);
        assert!(!crate::repository::git_repository__fsync_gitdir.load(Ordering::Relaxed));
        // A NULL out-pointer is rejected (C would crash).
        // SAFETY: NULL is checked.
        assert!(unsafe { opts(opt::GET_OWNER_VALIDATION, &args()) }.is_err());
    }
}
