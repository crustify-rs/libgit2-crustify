//! libgit2's global options (`git_libgit2_opts`), as typed getters and
//! setters, one pair per `GIT_OPT_*` (A-1).
//!
//! An option set here takes effect for the operations that start after the
//! call; the numeric and boolean ones are atomics, the paths and strings are
//! copied under a lock their Rust readers take. The C ABI keeps C's rule
//! (no `git_libgit2_opts` call while another thread uses the library).
//!
//! Not here, with the reason:
//! - `GIT_OPT_SET_ALLOCATOR`: replacing the allocator under live
//!   allocations is not something a safe function can allow;
//! - `GIT_OPT_SET_SSL_CERT_LOCATIONS`, `GIT_OPT_ADD_SSL_X509_CERT`: they
//!   configure the TLS streams of the transports, which the safe API does
//!   not offer (and the latter takes an OpenSSL `X509 *`);
//! - `GIT_OPT_GET/SET_WINDOWS_SHAREMODE`: Windows only.
//!
//! ```no_run
//! use libgit2_rs::{ConfigLevel, opts};
//! opts::set_mwindow_size(32 << 20);
//! opts::set_search_path(ConfigLevel::Global, Some("/home/me/.config/git"))?;
//! assert!(opts::search_path(ConfigLevel::Global)?.ends_with("git"));
//! opts::enable_caching(false);
//! # Ok::<(), libgit2_rs::Error>(())
//! ```

use core::ffi::{c_int, c_void};
use core::ptr;
use core::sync::atomic::Ordering::Relaxed;
use core::time::Duration;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::api::error::{Error, Result};
use crate::config::ConfigLevel;
use crate::odb::ObjectType;
use crate::settings::{OptArgs, opt};
use crate::sys::sysdir;
use crate::util::errors::{ErrorClass, ErrorCode};
use crate::util::str::GitStr;

/// Run the C ABI's option `key` with `args`.
///
/// # Safety
///
/// `args` holds what [`settings::opts`](crate::settings::opts) requires
/// for `key`: each pointer NULL or valid for the option's use of it.
unsafe fn call(key: c_int, args: OptArgs) -> Result<()> {
    crate::libgit2::ensure_init()?;
    // SAFETY: this function's contract is `opts`'.
    Ok(unsafe { crate::settings::opts(key, &args) }?)
}

/// [`call`] with integer arguments only (`p1` and `p2` NULL).
///
/// # Panics
///
/// For `GIT_OPT_SET_ALLOCATOR`, whose NULL resets the allocator (unsound
/// under live allocations of another one).
fn call_ints(key: c_int, args: OptArgs) -> Result<()> {
    assert_ne!(key, opt::SET_ALLOCATOR, "not an integer option");
    let args = OptArgs {
        p1: ptr::null_mut(),
        p2: ptr::null_mut(),
        ..args
    };
    // SAFETY: both pointers are NULL, which every option but
    // `SET_ALLOCATOR` (refused above) rejects (`out`, `buf`, `strarray`
    // arguments: an invalid-argument error) or reads as "none / the
    // default" (the strings, the X509 certificate, and the extensions,
    // which are none for NULL whatever `sz` is).
    unsafe { call(key, args) }
}

fn args() -> OptArgs {
    OptArgs {
        i: 0,
        sz: 0,
        ssz: 0,
        p1: ptr::null_mut(),
        p2: ptr::null_mut(),
    }
}

/// A getter of a `T` out-parameter.
///
/// # Safety
///
/// `T` is the type `key` writes through its pointer (`size_t` is `usize`,
/// `int` is `c_int`, `ssize_t` is `isize`).
unsafe fn get<T: Copy>(key: c_int, init: T) -> T {
    let mut v = init;
    let a = OptArgs {
        p1: (&raw mut v).cast::<c_void>(),
        ..args()
    };
    // Integer getters cannot fail once the library is initialized.
    // SAFETY: `p1` points at a local `T`, aligned and writable for the
    // call and not otherwise referenced, which is the type `key` writes
    // (this function's contract).
    let _ = unsafe { call(key, a) };
    v
}

fn set_bool(key: c_int, on: bool) {
    let _ = call_ints(
        key,
        OptArgs {
            i: c_int::from(on),
            ..args()
        },
    );
}

fn set_size(key: c_int, v: usize) {
    let _ = call_ints(key, OptArgs { sz: v, ..args() });
}

fn nul_error(what: &str) -> Error {
    Error::with(
        ErrorCode::GENERIC,
        ErrorClass::INVALID,
        format!("{what} contains a NUL byte"),
    )
}

fn c_bytes(b: &[u8], what: &str) -> Result<CString> {
    CString::new(b).map_err(|_| nul_error(what))
}

fn os_path(b: &[u8]) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(b))
}

/// The sysdir of a configuration level (`config_level_to_sysdir`).
fn level_sysdir(level: ConfigLevel) -> Result<c_int> {
    match level {
        ConfigLevel::System => Ok(sysdir::GIT_SYSDIR_SYSTEM),
        ConfigLevel::Xdg => Ok(sysdir::GIT_SYSDIR_XDG),
        ConfigLevel::Global => Ok(sysdir::GIT_SYSDIR_GLOBAL),
        _ => Err(Error::from(crate::git_error_set!(
            ErrorClass::INVALID,
            "invalid config path selector {}",
            level.raw()
        ))),
    }
}

fn sysdir_get(which: c_int) -> Result<PathBuf> {
    crate::libgit2::ensure_init()?;
    let mut s = GitStr::new();
    sysdir::get_into(&mut s, which)?;
    Ok(os_path(s.as_bytes()))
}

fn sysdir_set(which: c_int, paths: Option<&Path>) -> Result<()> {
    crate::libgit2::ensure_init()?;
    let c = paths
        .map(|p| c_bytes(p.as_os_str().as_bytes(), "path"))
        .transpose()?;
    Ok(sysdir::set(which, c.as_deref())?)
}

// --- memory windows ---------------------------------------------------------

/// `GIT_OPT_GET_MWINDOW_SIZE`: the size of a pack's mapped window.
pub fn mwindow_size() -> usize {
    crate::mwindow::git_mwindow__window_size.load(Relaxed)
}

/// `GIT_OPT_SET_MWINDOW_SIZE`.
pub fn set_mwindow_size(size: usize) {
    crate::mwindow::git_mwindow__window_size.store(size, Relaxed)
}

/// `GIT_OPT_GET_MWINDOW_MAPPED_LIMIT`: the memory mapped for packs before
/// windows are unmapped.
pub fn mwindow_mapped_limit() -> usize {
    crate::mwindow::git_mwindow__mapped_limit.load(Relaxed)
}

/// `GIT_OPT_SET_MWINDOW_MAPPED_LIMIT`.
pub fn set_mwindow_mapped_limit(limit: usize) {
    crate::mwindow::git_mwindow__mapped_limit.store(limit, Relaxed)
}

/// `GIT_OPT_GET_MWINDOW_FILE_LIMIT`: the pack files kept open (0: no
/// limit).
pub fn mwindow_file_limit() -> usize {
    crate::mwindow::git_mwindow__file_limit.load(Relaxed)
}

/// `GIT_OPT_SET_MWINDOW_FILE_LIMIT`.
pub fn set_mwindow_file_limit(limit: usize) {
    crate::mwindow::git_mwindow__file_limit.store(limit, Relaxed)
}

// --- search paths -------------------------------------------------------------

/// `GIT_OPT_GET_SEARCH_PATH`: where the configuration files of `level`
/// (`System`, `Xdg` or `Global`) are looked for (a `:`-separated list).
pub fn search_path(level: ConfigLevel) -> Result<PathBuf> {
    sysdir_get(level_sysdir(level)?)
}

/// `GIT_OPT_SET_SEARCH_PATH`: set the search path of `level` (a
/// `:`-separated list, in which `$PATH` stands for the current value);
/// `None` restores the default.
pub fn set_search_path(level: ConfigLevel, paths: Option<impl AsRef<Path>>) -> Result<()> {
    let which = level_sysdir(level)?;
    sysdir_set(which, paths.as_ref().map(AsRef::as_ref))
}

/// `GIT_OPT_GET_TEMPLATE_PATH`: where repository templates are looked for.
pub fn template_path() -> Result<PathBuf> {
    sysdir_get(sysdir::GIT_SYSDIR_TEMPLATE)
}

/// `GIT_OPT_SET_TEMPLATE_PATH` (`None`: the default).
pub fn set_template_path(path: Option<impl AsRef<Path>>) -> Result<()> {
    sysdir_set(
        sysdir::GIT_SYSDIR_TEMPLATE,
        path.as_ref().map(AsRef::as_ref),
    )
}

/// `GIT_OPT_GET_HOMEDIR`: the home directory `~` stands for.
pub fn homedir() -> Result<PathBuf> {
    sysdir_get(sysdir::GIT_SYSDIR_HOME)
}

/// `GIT_OPT_SET_HOMEDIR` (`None`: from the environment).
pub fn set_homedir(path: Option<impl AsRef<Path>>) -> Result<()> {
    sysdir_set(sysdir::GIT_SYSDIR_HOME, path.as_ref().map(AsRef::as_ref))
}

// --- the object cache -----------------------------------------------------------

/// `GIT_OPT_SET_CACHE_OBJECT_LIMIT`: the largest object of type `kind`
/// that is cached (0: none of that type).
pub fn set_cache_object_limit(kind: ObjectType, size: usize) -> Result<()> {
    Ok(crate::cache::set_max_object_size(kind.raw(), size)?)
}

/// `GIT_OPT_SET_CACHE_MAX_SIZE`: the memory all caches may use together.
pub fn set_cache_max_size(size: usize) {
    let size = isize::try_from(size).unwrap_or(isize::MAX);
    crate::cache::git_cache__max_storage.store(size, Relaxed)
}

/// `GIT_OPT_ENABLE_CACHING`.
pub fn enable_caching(on: bool) {
    crate::cache::git_cache__enabled.store(on, Relaxed)
}

/// `GIT_OPT_GET_CACHED_MEMORY`: (the memory the caches use now, the most
/// they may use).
pub fn cached_memory() -> (usize, usize) {
    let cur = crate::cache::git_cache__current_storage.load(core::sync::atomic::Ordering::SeqCst);
    let max = crate::cache::git_cache__max_storage.load(Relaxed);
    (
        usize::try_from(cur).unwrap_or(0),
        usize::try_from(max).unwrap_or(0),
    )
}

// --- strictness -----------------------------------------------------------------

/// `GIT_OPT_ENABLE_STRICT_OBJECT_CREATION`: check that the objects a new
/// object refers to exist (and are of the right type).
pub fn enable_strict_object_creation(on: bool) {
    crate::object::git_object__strict_input_validation.store(on, Relaxed)
}

/// `GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION`: check the targets of new
/// symbolic references.
pub fn enable_strict_symbolic_ref_creation(on: bool) {
    crate::refs::git_reference__enable_symbolic_ref_target_validation.store(on, Relaxed)
}

/// `GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION`: check objects read against
/// their ids.
pub fn enable_strict_hash_verification(on: bool) {
    crate::odb::git_odb__strict_hash_verification.store(on, Relaxed)
}

/// `GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY`: refuse to overwrite an index
/// with unsaved changes when reading it again.
pub fn enable_unsaved_index_safety(on: bool) {
    crate::index::git_index__enforce_unsaved_safety.store(on, Relaxed)
}

/// `GIT_OPT_ENABLE_FSYNC_GITDIR`: `fsync` what is written to the git
/// directory.
pub fn enable_fsync_gitdir(on: bool) {
    crate::repository::git_repository__fsync_gitdir.store(on, Relaxed)
}

/// `GIT_OPT_GET_OWNER_VALIDATION`: whether a repository owned by another
/// user is refused.
pub fn owner_validation() -> bool {
    crate::repository::git_repository__validate_ownership.load(Relaxed)
}

/// `GIT_OPT_SET_OWNER_VALIDATION`.
pub fn set_owner_validation(on: bool) {
    crate::repository::git_repository__validate_ownership.store(on, Relaxed)
}

// --- packs and the odb ------------------------------------------------------------

/// `GIT_OPT_GET_PACK_MAX_OBJECTS`: the most objects a fetched pack may
/// have (0: no limit).
pub fn pack_max_objects() -> usize {
    // SAFETY: the option writes a `size_t`.
    unsafe { get(opt::GET_PACK_MAX_OBJECTS, 0usize) }
}

/// `GIT_OPT_SET_PACK_MAX_OBJECTS`.
pub fn set_pack_max_objects(n: usize) {
    set_size(opt::SET_PACK_MAX_OBJECTS, n)
}

/// `GIT_OPT_GET_PACK_MAX_OBJECT_SIZE`: the largest object a fetched pack
/// may have.
pub fn pack_max_object_size() -> usize {
    // SAFETY: the option writes a `size_t`.
    unsafe { get(opt::GET_PACK_MAX_OBJECT_SIZE, 0usize) }
}

/// `GIT_OPT_SET_PACK_MAX_OBJECT_SIZE`.
pub fn set_pack_max_object_size(n: usize) {
    set_size(opt::SET_PACK_MAX_OBJECT_SIZE, n)
}

/// `GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS`: do not look for `.keep` files
/// next to packs.
pub fn disable_pack_keep_file_checks(on: bool) {
    crate::pack::git_disable_pack_keep_file_checks.store(on, Relaxed)
}

/// `GIT_OPT_SET_ODB_PACKED_PRIORITY`: the priority of the pack backend of
/// object directories opened later.
pub fn set_odb_packed_priority(priority: i32) {
    crate::odb::git_odb__packed_priority.store(priority, Relaxed)
}

/// `GIT_OPT_SET_ODB_LOOSE_PRIORITY`: the priority of the loose backend.
pub fn set_odb_loose_priority(priority: i32) {
    crate::odb::git_odb__loose_priority.store(priority, Relaxed)
}

/// `GIT_OPT_GET_EXTENSIONS`: the repository format extensions this library
/// accepts (the built-in ones and those added).
pub fn extensions() -> Result<Vec<Vec<u8>>> {
    crate::libgit2::ensure_init()?;
    Ok(crate::repository::api::extensions())
}

/// `GIT_OPT_SET_EXTENSIONS`: accept these extensions too (`"!name"`
/// refuses a built-in one).
pub fn set_extensions<I>(extensions: I) -> Result<()>
where
    I: IntoIterator,
    I::Item: AsRef<[u8]>,
{
    let v: Vec<Vec<u8>> = extensions
        .into_iter()
        .map(|e| e.as_ref().to_vec())
        .collect();
    if v.iter().any(|e| e.contains(&0)) {
        return Err(nul_error("extension"));
    }
    crate::libgit2::ensure_init()?;
    let refs: Vec<&[u8]> = v.iter().map(Vec::as_slice).collect();
    crate::repository::api::set_extensions(&refs);
    Ok(())
}

// --- transports ---------------------------------------------------------------------

/// `GIT_OPT_GET_USER_AGENT`: what HTTP requests send as the user agent.
pub fn user_agent() -> Vec<u8> {
    crate::settings::user_agent()
}

/// `GIT_OPT_SET_USER_AGENT` (`None`: the default).
pub fn set_user_agent(agent: Option<impl AsRef<[u8]>>) -> Result<()> {
    let c = agent
        .as_ref()
        .map(|a| c_bytes(a.as_ref(), "user agent"))
        .transpose()?;
    crate::libgit2::ensure_init()?;
    Ok(crate::settings::set_user_agent(c.as_deref())?)
}

/// `GIT_OPT_GET_USER_AGENT_PRODUCT`: the product of the user agent.
pub fn user_agent_product() -> Vec<u8> {
    crate::settings::user_agent_product()
}

/// `GIT_OPT_SET_USER_AGENT_PRODUCT` (`None`: the default).
pub fn set_user_agent_product(product: Option<impl AsRef<[u8]>>) -> Result<()> {
    let c = product
        .as_ref()
        .map(|a| c_bytes(a.as_ref(), "user agent product"))
        .transpose()?;
    crate::libgit2::ensure_init()?;
    Ok(crate::settings::set_user_agent_product(c.as_deref())?)
}

/// `GIT_OPT_SET_SSL_CIPHERS`: the TLS cipher list (`None`: the default).
pub fn set_ssl_ciphers(ciphers: Option<impl AsRef<[u8]>>) -> Result<()> {
    let c = ciphers
        .as_ref()
        .map(|a| c_bytes(a.as_ref(), "cipher list"))
        .transpose()?;
    let mut a = args();
    if let Some(c) = &c {
        a.p1 = c.as_ptr().cast_mut().cast();
    }
    // SAFETY: `p1` is NULL or the NUL-terminated list `c`, alive and
    // unwritten for the call, which copies it.
    unsafe { call(opt::SET_SSL_CIPHERS, a) }
}

/// `GIT_OPT_ENABLE_OFS_DELTA`: ask servers for offset deltas.
pub fn enable_ofs_delta(on: bool) {
    set_bool(opt::ENABLE_OFS_DELTA, on)
}

/// `GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE`.
pub fn enable_http_expect_continue(on: bool) {
    set_bool(opt::ENABLE_HTTP_EXPECT_CONTINUE, on)
}

fn timeout_ms(d: Duration, what: &str) -> Result<c_int> {
    c_int::try_from(d.as_millis()).map_err(|_| {
        Error::with(
            ErrorCode::GENERIC,
            ErrorClass::INVALID,
            format!("invalid {what}"),
        )
    })
}

/// `GIT_OPT_GET_SERVER_CONNECT_TIMEOUT` (`Duration::ZERO`: the system's).
pub fn server_connect_timeout() -> Duration {
    // SAFETY: the option writes an `int`.
    let ms = unsafe { get(opt::GET_SERVER_CONNECT_TIMEOUT, 0 as c_int) };
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

/// `GIT_OPT_SET_SERVER_CONNECT_TIMEOUT` (in milliseconds, as C).
pub fn set_server_connect_timeout(timeout: Duration) -> Result<()> {
    let i = timeout_ms(timeout, "connect timeout")?;
    call_ints(opt::SET_SERVER_CONNECT_TIMEOUT, OptArgs { i, ..args() })
}

/// `GIT_OPT_GET_SERVER_TIMEOUT` (`Duration::ZERO`: none).
pub fn server_timeout() -> Duration {
    // SAFETY: the option writes an `int`.
    let ms = unsafe { get(opt::GET_SERVER_TIMEOUT, 0 as c_int) };
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

/// `GIT_OPT_SET_SERVER_TIMEOUT` (in milliseconds, as C).
pub fn set_server_timeout(timeout: Duration) -> Result<()> {
    let i = timeout_ms(timeout, "timeout")?;
    call_ints(opt::SET_SERVER_TIMEOUT, OptArgs { i, ..args() })
}
