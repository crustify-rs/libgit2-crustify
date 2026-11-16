//! Port of `src/libgit2/settings.c`: the storage of the library's
//! options, and `git_libgit2_opts`.
//!
//! `git_libgit2_opts` is variadic: its C shim (`src/util/rust_shims.c`)
//! reads each option's arguments from the `va_list`, as the C did, into an
//! [`OptArgs`], and calls [`ffi::git_libgit2__opts`], which does the rest.
//! The options are mostly globals of other modules (`git_mwindow__*`,
//! `git_cache__*`, …): the ported modules' Rust statics, or C globals
//! declared in `sys` (PENDING for the L-wave modules not yet merged); the
//! user agents and the TLS ciphers are this module's (`git__user_agent`,
//! `git__user_agent_product`, `git__ssl_ciphers`, read by C).
//!
//! The safe API is the [`opts`] module.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::buf::GitBuf;
use crate::git_error_set;
use crate::util::alloc;
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

/// The string global (or `default`), as C reads it.
///
/// # Safety
///
/// The result is used only until the next options call that replaces the
/// global or the last `git_libgit2_shutdown` (C's rule).
unsafe fn read(g: &AtomicPtr<c_char>, default: &'static CStr) -> &'static CStr {
    let p = g.load(Ordering::Acquire);
    if p.is_null() {
        default
    } else {
        // SAFETY: the caller's contract.
        unsafe { CStr::from_ptr(p) }
    }
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
/// `git_buf_fromstr`.
///
/// # Safety
///
/// `out` is a valid `git_buf` (or NULL, rejected).
unsafe fn buf_out(out: *mut GitBuf, f: impl FnOnce(&mut GitStr) -> Result<()>) -> Result<()> {
    let mut s = GitStr::new();
    // SAFETY: the caller's contract; `s` is local.
    ErrorCode::check(unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, out) })?;
    f(&mut s)?;
    // SAFETY: as above.
    ErrorCode::check(unsafe { crate::buf::ffi::git_buf_fromstr(out, &raw mut s) })?;
    Ok(())
}

/// The globals of other modules the options read and write. Each is a C
/// global (`size_t`, `int`, `bool`, …) that the Rust reaches as the atomic
/// of the same layout.
mod globals {
    pub(super) use crate::sys::indexer::*;
    pub(super) use crate::sys::pending::l_repo::git_object__strict_input_validation;
    pub(super) use crate::sys::streams::{
        git_socket_stream__connect_timeout, git_socket_stream__timeout,
    };
    pub(super) use crate::sys::transports::{
        git_http__expect_continue, git_smart__ofs_delta_enabled,
    };
}

macro_rules! g {
    ($name:ident) => {
        // SAFETY: a C global defined (with this layout) by its module;
        // reaching it as an atomic is sound however C accesses it.
        unsafe { &globals::$name }
    };
}

/// Write `v` to a C out-pointer.
///
/// # Safety
///
/// `p` is NULL or writable for a `T`.
unsafe fn out<T>(p: *mut c_void, v: T) -> Result<()> {
    if p.is_null() {
        return Err(errors::assert_arg_failed("out"));
    }
    // SAFETY: the caller's contract.
    unsafe { p.cast::<T>().write(v) };
    Ok(())
}

/// An options call's arguments, with the contract of [`opts`].
struct Args<'a>(&'a OptArgs);

impl Args<'_> {
    fn put1<T>(&self, v: T) -> Result<()> {
        // SAFETY: `opts`' contract: `p1` is the option's out-pointer.
        unsafe { out(self.0.p1, v) }
    }
    fn put2<T>(&self, v: T) -> Result<()> {
        // SAFETY: as `put1`, for `p2`.
        unsafe { out(self.0.p2, v) }
    }
    fn buf1(&self, f: impl FnOnce(&mut GitStr) -> Result<()>) -> Result<()> {
        // SAFETY: `opts`' contract: `p1` is the option's `git_buf *`.
        unsafe { buf_out(self.0.p1.cast(), f) }
    }
    fn cstr1(&self) -> Option<&CStr> {
        // SAFETY: `opts`' contract: `p1` is NULL or the option's string.
        unsafe { crate::util::export::cstr(self.0.p1.cast()) }
    }
    fn cstr2(&self) -> Option<&CStr> {
        // SAFETY: as `cstr1`, for `p2`.
        unsafe { crate::util::export::cstr(self.0.p2.cast()) }
    }
}

/// `git_libgit2_opts(key, …)` with the arguments read.
///
/// # Safety
///
/// `a` holds the arguments C's `git_libgit2_opts` documents for `key`
/// (pointers valid for what the option does with them).
pub unsafe fn opts(key: c_int, args: &OptArgs) -> Result<()> {
    use opt::*;
    let r = Ordering::Relaxed;
    let a = Args(args);
    {
        match key {
            SET_MWINDOW_SIZE => crate::mwindow::git_mwindow__window_size.store(a.0.sz, r),
            GET_MWINDOW_SIZE => a.put1(crate::mwindow::git_mwindow__window_size.load(r))?,
            SET_MWINDOW_MAPPED_LIMIT => crate::mwindow::git_mwindow__mapped_limit.store(a.0.sz, r),
            GET_MWINDOW_MAPPED_LIMIT => {
                a.put1(crate::mwindow::git_mwindow__mapped_limit.load(r))?
            }
            SET_MWINDOW_FILE_LIMIT => crate::mwindow::git_mwindow__file_limit.store(a.0.sz, r),
            GET_MWINDOW_FILE_LIMIT => a.put1(crate::mwindow::git_mwindow__file_limit.load(r))?,
            GET_SEARCH_PATH => a.buf1(|s| {
                let level = config_level_to_sysdir(a.0.i)?;
                crate::sys::sysdir::get_into(s, level)
            })?,
            SET_SEARCH_PATH => {
                let level = config_level_to_sysdir(a.0.i)?;
                crate::sys::sysdir::set(level, a.cstr1())?;
            }
            SET_CACHE_OBJECT_LIMIT => {
                crate::cache::set_max_object_size(a.0.i, a.0.sz)?;
            }
            SET_CACHE_MAX_SIZE => crate::cache::git_cache__max_storage.store(a.0.ssz, r),
            ENABLE_CACHING => crate::cache::git_cache__enabled.store(a.0.i != 0, r),
            GET_CACHED_MEMORY => {
                a.put1(crate::cache::git_cache__current_storage.load(Ordering::SeqCst))?;
                a.put2(crate::cache::git_cache__max_storage.load(r))?;
            }
            GET_TEMPLATE_PATH => a.buf1(|s| {
                crate::sys::sysdir::get_into(s, crate::sys::sysdir::GIT_SYSDIR_TEMPLATE)
            })?,
            SET_TEMPLATE_PATH => {
                crate::sys::sysdir::set(crate::sys::sysdir::GIT_SYSDIR_TEMPLATE, a.cstr1())?
            }
            SET_SSL_CERT_LOCATIONS => {
                crate::sys::streams::openssl_set_cert_location(a.cstr1(), a.cstr2())?
            }
            ADD_SSL_X509_CERT => {
                // SAFETY: the caller passes an `X509 *` (the option's contract).
                unsafe { crate::sys::streams::openssl_add_x509_cert(a.0.p1) }?
            }
            SET_USER_AGENT => replace(&git__user_agent, a.cstr1())?,
            GET_USER_AGENT => a.buf1(|s| s.puts(&user_agent()))?,
            SET_USER_AGENT_PRODUCT => replace(&git__user_agent_product, a.cstr1())?,
            GET_USER_AGENT_PRODUCT => a.buf1(|s| s.puts(&user_agent_product()))?,
            ENABLE_STRICT_OBJECT_CREATION => {
                g!(git_object__strict_input_validation).store(a.0.i != 0, r)
            }
            ENABLE_STRICT_SYMBOLIC_REF_CREATION => {
                crate::refs::git_reference__enable_symbolic_ref_target_validation
                    .store(a.0.i != 0, r)
            }
            SET_SSL_CIPHERS => {
                // C copies with `git__strdup` and reports its failure as
                // out-of-memory; a NULL list (which crashes the C) clears
                // the setting.
                if replace(&git__ssl_ciphers, a.cstr1()).is_err() {
                    return Err(errors::oom());
                }
            }
            ENABLE_OFS_DELTA => g!(git_smart__ofs_delta_enabled).store(a.0.i != 0, r),
            ENABLE_FSYNC_GITDIR => {
                crate::repository::git_repository__fsync_gitdir.store(a.0.i != 0, r)
            }
            GET_WINDOWS_SHAREMODE | SET_WINDOWS_SHAREMODE => {}
            ENABLE_STRICT_HASH_VERIFICATION => {
                crate::odb::git_odb__strict_hash_verification.store(a.0.i != 0, r)
            }
            SET_ALLOCATOR => {
                // SAFETY: NULL or a valid `git_allocator` (the option's contract).
                ErrorCode::check(unsafe {
                    crate::util::alloc::ffi::git_allocator_setup(a.0.p1.cast())
                })?;
            }
            ENABLE_UNSAVED_INDEX_SAFETY => {
                crate::index::git_index__enforce_unsaved_safety.store(a.0.i != 0, r)
            }
            SET_PACK_MAX_OBJECTS => g!(git_indexer__max_objects).store(a.0.sz, r),
            GET_PACK_MAX_OBJECTS => a.put1(g!(git_indexer__max_objects).load(r))?,
            SET_PACK_MAX_OBJECT_SIZE => g!(git_indexer__max_object_size).store(a.0.sz, r),
            GET_PACK_MAX_OBJECT_SIZE => a.put1(g!(git_indexer__max_object_size).load(r))?,
            DISABLE_PACK_KEEP_FILE_CHECKS => {
                crate::pack::git_disable_pack_keep_file_checks.store(a.0.i != 0, r)
            }
            ENABLE_HTTP_EXPECT_CONTINUE => g!(git_http__expect_continue).store(a.0.i != 0, r),
            SET_ODB_PACKED_PRIORITY => crate::odb::git_odb__packed_priority.store(a.0.i, r),
            SET_ODB_LOOSE_PRIORITY => crate::odb::git_odb__loose_priority.store(a.0.i, r),
            SET_EXTENSIONS => {
                // SAFETY: `len` strings at `extensions` (the option's contract).
                ErrorCode::check(unsafe {
                    crate::repository::ffi::git_repository__set_extensions(a.0.p1.cast(), a.0.sz)
                })?;
            }
            GET_EXTENSIONS => {
                let arr = a.0.p1.cast::<crate::strarray::GitStrArray>();
                if arr.is_null() {
                    return Err(errors::assert_arg_failed("out"));
                }
                // SAFETY: a writable `git_strarray` (the option's contract).
                // SAFETY: a field projection of a valid struct.
                let strings = unsafe { &raw mut (*arr).strings };
                // SAFETY: as above.
                let count = unsafe { &raw mut (*arr).count };
                // SAFETY: two writable fields.
                ErrorCode::check(unsafe {
                    crate::repository::ffi::git_repository__extensions(strings, count)
                })?;
            }
            GET_OWNER_VALIDATION => a.put1(c_int::from(
                crate::repository::git_repository__validate_ownership.load(r),
            ))?,
            SET_OWNER_VALIDATION => {
                crate::repository::git_repository__validate_ownership.store(a.0.i != 0, r)
            }
            GET_HOMEDIR => {
                a.buf1(|s| crate::sys::sysdir::get_into(s, crate::sys::sysdir::GIT_SYSDIR_HOME))?
            }
            SET_HOMEDIR => crate::sys::sysdir::set(crate::sys::sysdir::GIT_SYSDIR_HOME, a.cstr1())?,
            GET_SERVER_CONNECT_TIMEOUT => a.put1(g!(git_socket_stream__connect_timeout).load(r))?,
            SET_SERVER_CONNECT_TIMEOUT => {
                if a.0.i < 0 {
                    return Err(git_error_set!(
                        ErrorClass::INVALID,
                        "invalid connect timeout"
                    ));
                }
                g!(git_socket_stream__connect_timeout).store(a.0.i, r);
            }
            GET_SERVER_TIMEOUT => a.put1(g!(git_socket_stream__timeout).load(r))?,
            SET_SERVER_TIMEOUT => {
                if a.0.i < 0 {
                    return Err(git_error_set!(ErrorClass::INVALID, "invalid timeout"));
                }
                g!(git_socket_stream__timeout).store(a.0.i, r);
            }
            _ => return Err(git_error_set!(ErrorClass::INVALID, "invalid option key")),
        }
    }
    Ok(())
}

/// The safe API of the options this module keeps (the others belong to
/// their modules' APIs).
pub mod opts {
    use super::*;
    use crate::util::errors::Error;

    /// `GIT_OPT_GET_USER_AGENT`.
    pub fn user_agent() -> Vec<u8> {
        super::user_agent()
    }

    /// `GIT_OPT_SET_USER_AGENT` (`None`: the default).
    pub fn set_user_agent(agent: Option<&CStr>) -> core::result::Result<(), Error> {
        Ok(replace(&git__user_agent, agent)?)
    }

    /// `GIT_OPT_GET_USER_AGENT_PRODUCT`.
    pub fn user_agent_product() -> Vec<u8> {
        super::user_agent_product()
    }

    /// `GIT_OPT_SET_USER_AGENT_PRODUCT` (`None`: the default).
    pub fn set_user_agent_product(product: Option<&CStr>) -> core::result::Result<(), Error> {
        Ok(replace(&git__user_agent_product, product)?)
    }

    /// `GIT_OPT_GET_OWNER_VALIDATION`.
    pub fn owner_validation() -> bool {
        crate::repository::git_repository__validate_ownership.load(Ordering::Relaxed)
    }

    /// `GIT_OPT_SET_OWNER_VALIDATION`.
    pub fn set_owner_validation(on: bool) {
        crate::repository::git_repository__validate_ownership.store(on, Ordering::Relaxed)
    }

    /// `GIT_OPT_ENABLE_FSYNC_GITDIR`.
    pub fn set_fsync_gitdir(on: bool) {
        crate::repository::git_repository__fsync_gitdir.store(on, Ordering::Relaxed)
    }

    /// `GIT_OPT_GET_EXTENSIONS`.
    pub fn extensions() -> Vec<Vec<u8>> {
        crate::repository::api::extensions()
    }

    /// `GIT_OPT_SET_EXTENSIONS`.
    pub fn set_extensions(exts: &[&[u8]]) {
        crate::repository::api::set_extensions(exts)
    }
}

/// The exports of `settings.c`.
pub mod ffi {
    use super::*;
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
        // SAFETY: the caller's contract is `read`'s.
        unsafe { read(&git__user_agent, DEFAULT_USER_AGENT) }.as_ptr()
    }

    /// `git_settings__user_agent_product`.
    ///
    /// # Safety
    ///
    /// As [`git_settings__user_agent`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_settings__user_agent_product() -> *const c_char {
        // SAFETY: the caller's contract is `read`'s.
        unsafe { read(&git__user_agent_product, c"git/2.0") }.as_ptr()
    }

    /// `git_libgit2_opts`' body: the shim in `rust_shims.c` read the
    /// arguments of `key` into `args`.
    ///
    /// # Safety
    ///
    /// `args` is valid, and holds what C's `git_libgit2_opts` documents
    /// for `key`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_libgit2__opts(key: c_int, args: *const OptArgs) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let a = unsafe { &*args };
            // SAFETY: the caller's contract.
            status(unsafe { opts(key, a) })
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
        opts::set_user_agent(Some(c"agent/1")).unwrap();
        assert_eq!(opts::user_agent(), b"agent/1");
        opts::set_user_agent(None).unwrap();
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
                opts::set_user_agent(Some(v)).unwrap();
            }
        });
        for _ in 0..20 {
            let v = opts::user_agent();
            assert!(!v.is_empty());
        }
        t.join().unwrap();
        let held = opts::user_agent();
        opts::set_user_agent(None).unwrap();
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
        assert_eq!(v, c_int::from(opts::owner_validation()));
        let mut b = args();
        b.i = 1;
        // SAFETY: an int.
        unsafe { opts(opt::ENABLE_FSYNC_GITDIR, &b) }.unwrap();
        assert!(crate::repository::git_repository__fsync_gitdir.load(Ordering::Relaxed));
        opts::set_fsync_gitdir(false);
        assert!(!crate::repository::git_repository__fsync_gitdir.load(Ordering::Relaxed));
        // A NULL out-pointer is rejected (C would crash).
        // SAFETY: NULL is checked.
        assert!(unsafe { opts(opt::GET_OWNER_VALIDATION, &args()) }.is_err());
    }
}
