//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

// PENDING(l-config, l-odb, l-pack, l-refs, l-index, l-obj, l-commit): what
// the repository (WP l-repo) calls of them.
pub mod l_repo;
use core::ffi::c_void;

// ---------------------------------------------------------------------------
// WP l-odb (odb, odb_loose, odb_mempack, cache)
// ---------------------------------------------------------------------------

use core::ffi::c_int;
use core::sync::atomic::AtomicBool;

// `git_odb_backend` holds a `git_odb *`, opaque to C (the Rust's).
#[allow(improper_ctypes)]
unsafe extern "C" {
    /// `git_object__free` (object.c). PENDING(l-obj)
    fn git_object__free(object: *mut c_void);

    /// `git_repository__configmap_lookup` (repository.c). PENDING(l-repo)
    fn git_repository__configmap_lookup(out: *mut c_int, repo: *mut c_void, item: c_int) -> c_int;

    /// `git_repository__fsync_gitdir` (repository.c; settings.c writes it).
    /// PENDING(l-repo)
    static git_repository__fsync_gitdir: AtomicBool;

}

/// Free a parsed object (`git_object__free`), PENDING(l-obj).
///
/// # Safety
///
/// `object` is a `git_object` whose last reference was just dropped.
pub unsafe fn object_free(object: core::ptr::NonNull<c_void>) {
    // SAFETY: the caller's contract.
    unsafe { git_object__free(object.as_ptr()) }
}

/// `GIT_CONFIGMAP_FSYNCOBJECTFILES` (repository.h).
pub const GIT_CONFIGMAP_FSYNCOBJECTFILES: c_int = 13;

/// `git_repository__configmap_lookup(&out, repo, item)`, PENDING(l-repo).
///
/// # Safety
///
/// `repo` is a live `git_repository *`.
pub unsafe fn repository_configmap_lookup(
    repo: *mut c_void,
    item: c_int,
) -> crate::util::errors::Result<c_int> {
    let mut out = 0;
    // SAFETY: the caller's contract; `out` is writable.
    let r = unsafe { git_repository__configmap_lookup(&raw mut out, repo, item) };
    crate::util::errors::ErrorCode::check_zero(r).map(|()| out)
}

/// `git_repository__fsync_gitdir` (`GIT_OPT_ENABLE_FSYNC_GITDIR`),
/// PENDING(l-repo).
pub fn repository_fsync_gitdir() -> bool {
    // SAFETY: a C `bool` global, read atomically (C writes it as a plain
    // store; an atomic of the same size and alignment).
    unsafe { &git_repository__fsync_gitdir }.load(core::sync::atomic::Ordering::Relaxed)
}
