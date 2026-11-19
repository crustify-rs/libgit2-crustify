//! Port of `src/libgit2/repository.c`: the repository object, opening and
//! discovering repositories, initializing them, their HEAD, state files and
//! lazily created odb, config, refdb and index.
//!
//! # The type
//!
//! [`GitRepository`] is `struct git_repository` (`repository.h`), kind 2
//! (shared prefix, PORTING.md §4.1): nearly all of the C that stays reads
//! its fields (`repo->gitdir`, `workdir`, `commondir`, `namespace`,
//! `is_bare`, `oid_type`, `objects`, `attrcache`, `diff_drivers`,
//! `submodule_cache`, `configmap_cache`, …), and some writes them, but only
//! the Rust allocates and frees it. Every field the header declares is kept,
//! in order and at the C's offsets (the layout test checks them), so the
//! header is unchanged; nothing is Rust-only.
//!
//! A repository is shared between threads (`tests/libgit2/threads`), and C
//! updates several fields with atomic operations (`_odb`, `_refdb`,
//! `_config`, `_index`, `attrcache`, `diff_drivers`, `configmap_cache`,
//! `ident_*`). In the Rust every field is interior-mutable: the pointers
//! the repository owns are `ffibox::CAtomicOwner`s (the odb, config, refdb
//! and index as `CShared` references, the strings as `GitCStr`, the grafts
//! as `GraftsBox`), the pointers C owns `AtomicPtr`, the integers atomics,
//! the bitfield a `CAtomicBitfield`, the embedded `git_cache` an
//! `UnsafeCell`. So a `&GitRepository` is sound while C (on this or another
//! thread) writes any field, and the type is `Sync` with real atomics.
//!
//! The bitfield `unsigned use_env:1, is_bare:1, is_worktree:1` is one
//! `unsigned` at offset 232: `use_env` is bit 0, `is_bare` bit 1,
//! `is_worktree` bit 2 (x86-64 SysV: the first declared is the least
//! significant bit).
//!
//! The string fields (`gitdir`, `workdir`, …) are NULL or NUL-terminated
//! strings of `git__allocator` (`CAtomicOwner<GitCStr>`), owned by the
//! repository. Only
//! [`GitRepository::replace_string`] (`set_workdir`, `set_bare`,
//! `set_namespace`) and `git_repository_free` free them, so a string
//! borrowed from a `&GitRepository` stays valid until one of those runs;
//! their callers guarantee no such borrow is live (C's rule: these setters
//! are not called while the repository is used elsewhere).

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicPtr, AtomicU32, Ordering};

use crate::cache::GitCache;
use crate::config::GitConfig;
use crate::git_error_set;
use crate::grafts::Grafts;
use crate::index::{GIT_INDEX_CAPABILITY_FROM_OWNER, GitIndex, IndexPtr};
use crate::odb::{GitOdb, Odb};
use crate::oid::{GIT_OID_SHA1, OidType, git_oid_t};
use crate::refdb::GitRefdb;
use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::thread::{Atomic32, StartsWithRefcount, shared_nn};
use ffibox::{CAtomicBitfield, CAtomicOwner, CBits, CShared};

pub mod api;
pub mod ffi;
pub mod handles;
pub(crate) mod head;
mod init;
pub(crate) mod live;
mod open;
#[cfg(test)]
mod tests;

pub use api::{
    RefStorage, Repository, RepositoryInitFlags, RepositoryInitMode, RepositoryInitOptions,
    RepositoryItem, RepositoryOpenFlags, RepositoryState,
};
pub use handles::{Refdb, Reference};
pub(crate) use head::OwnedRepo;

use handles::check;

/// `DOT_GIT`.
pub const DOT_GIT: &[u8] = b".git";
/// `GIT_DIR`.
pub const GIT_DIR: &[u8] = b".git/";
/// `GIT_DIR_MODE`.
pub const GIT_DIR_MODE: u32 = 0o755;
/// `GIT_BARE_DIR_MODE`.
pub const GIT_BARE_DIR_MODE: u32 = 0o777;

/// `GIT_REPOSITORY_OPEN_NO_SEARCH`.
pub const GIT_REPOSITORY_OPEN_NO_SEARCH: u32 = 1 << 0;
/// `GIT_REPOSITORY_OPEN_CROSS_FS`.
pub const GIT_REPOSITORY_OPEN_CROSS_FS: u32 = 1 << 1;
/// `GIT_REPOSITORY_OPEN_BARE`.
pub const GIT_REPOSITORY_OPEN_BARE: u32 = 1 << 2;
/// `GIT_REPOSITORY_OPEN_NO_DOTGIT`.
pub const GIT_REPOSITORY_OPEN_NO_DOTGIT: u32 = 1 << 3;
/// `GIT_REPOSITORY_OPEN_FROM_ENV`.
pub const GIT_REPOSITORY_OPEN_FROM_ENV: u32 = 1 << 4;

/// `GIT_REPOSITORY_INIT_BARE`.
pub const GIT_REPOSITORY_INIT_BARE: u32 = 1 << 0;
/// `GIT_REPOSITORY_INIT_NO_REINIT`.
pub const GIT_REPOSITORY_INIT_NO_REINIT: u32 = 1 << 1;
/// `GIT_REPOSITORY_INIT_MKDIR`.
pub const GIT_REPOSITORY_INIT_MKDIR: u32 = 1 << 3;
/// `GIT_REPOSITORY_INIT_MKPATH`.
pub const GIT_REPOSITORY_INIT_MKPATH: u32 = 1 << 4;
/// `GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE`.
pub const GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE: u32 = 1 << 5;
/// `GIT_REPOSITORY_INIT_RELATIVE_GITLINK`.
pub const GIT_REPOSITORY_INIT_RELATIVE_GITLINK: u32 = 1 << 6;
/// `GIT_REPOSITORY_INIT__HAS_DOTGIT` (internal).
pub const GIT_REPOSITORY_INIT__HAS_DOTGIT: u32 = 1 << 16;
/// `GIT_REPOSITORY_INIT__NATURAL_WD` (internal).
pub const GIT_REPOSITORY_INIT__NATURAL_WD: u32 = 1 << 17;
/// `GIT_REPOSITORY_INIT__IS_REINIT` (internal).
pub const GIT_REPOSITORY_INIT__IS_REINIT: u32 = 1 << 18;

/// `GIT_REPOSITORY_INIT_SHARED_UMASK`.
pub const GIT_REPOSITORY_INIT_SHARED_UMASK: u32 = 0;
/// `GIT_REPOSITORY_INIT_SHARED_GROUP`.
pub const GIT_REPOSITORY_INIT_SHARED_GROUP: u32 = 0o002775;
/// `GIT_REPOSITORY_INIT_SHARED_ALL`.
pub const GIT_REPOSITORY_INIT_SHARED_ALL: u32 = 0o002777;

pub use crate::refdb::GIT_REFDB_FILES;
/// `GIT_REFDB_REFTABLE`.
pub const GIT_REFDB_REFTABLE: c_int = 2;

/// `GIT_CONFIGMAP_CACHE_MAX`: the entries of `configmap_cache`.
pub const GIT_CONFIGMAP_CACHE_MAX: usize = 15;
/// `GIT_CONFIGMAP_IGNORECASE`.
pub const GIT_CONFIGMAP_IGNORECASE: c_int = 3;
/// `GIT_CONFIGMAP_ABBREV`.
pub const GIT_CONFIGMAP_ABBREV: c_int = 7;
/// `GIT_CONFIGMAP_FSYNCOBJECTFILES`.
pub const GIT_CONFIGMAP_FSYNCOBJECTFILES: c_int = 13;
/// `GIT_ABBREV_MINIMUM`.
pub const GIT_ABBREV_MINIMUM: c_int = 4;
/// `GIT_ABBREV_FALSE` (`GIT_OID_MAX_HEXSIZE`).
pub const GIT_ABBREV_FALSE: c_int = 64;

pub use crate::config::{
    GIT_CONFIG_LEVEL_GLOBAL, GIT_CONFIG_LEVEL_LOCAL, GIT_CONFIG_LEVEL_SYSTEM,
    GIT_CONFIG_LEVEL_WORKTREE, GIT_CONFIG_LEVEL_XDG,
};

/// `unsigned use_env:1`, the first bit of the bitfield.
const USE_ENV: CBits<u32> = CBits::first(1);
/// `unsigned is_bare:1`.
const IS_BARE: CBits<u32> = USE_ENV.next(1);
/// `unsigned is_worktree:1`.
const IS_WORKTREE: CBits<u32> = IS_BARE.next(1);

/// The repository's grafts (`git_grafts *`), owned: freed with
/// `git_grafts_free`.
#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct GraftsBox(NonNull<Grafts>);

impl GraftsBox {
    /// Adopt grafts of `git_grafts_new` / `git_grafts_open`.
    ///
    /// # Safety
    ///
    /// `p` is NULL or grafts of those functions that the caller gives up.
    pub(crate) unsafe fn from_raw(p: *mut Grafts) -> Option<GraftsBox> {
        NonNull::new(p).map(GraftsBox)
    }
}

impl Drop for GraftsBox {
    fn drop(&mut self) {
        // SAFETY: the grafts this owner holds, freed once.
        unsafe { crate::grafts::ffi::git_grafts_free(self.0.as_ptr()) }
    }
}

// SAFETY: `GraftsBox` is one non-null pointer, which `into_raw_ptr` gives
// and `from_raw_ptr` takes back as the same owner.
unsafe impl ffibox::CPtrOwner for GraftsBox {
    type Target = Grafts;
    fn into_raw_ptr(self) -> NonNull<Grafts> {
        let p = self.0;
        core::mem::forget(self);
        p
    }
    unsafe fn from_raw_ptr(ptr: NonNull<Grafts>) -> Self {
        GraftsBox(ptr)
    }
}

/// `git_array_t(git_str)`, the repository's `reserved_names` (only the
/// Windows build fills it; here it is allocated and freed, as the C).
#[repr(C)]
struct ReservedNames {
    ptr: *mut GitStr,
    size: usize,
    asize: usize,
}

/// `struct git_repository`, with every field interior-mutable (see the
/// [module documentation](self)).
#[repr(C)]
pub struct GitRepository {
    odb: CAtomicOwner<CShared<GitOdb>>,
    refdb: CAtomicOwner<CShared<GitRefdb>>,
    config: CAtomicOwner<CShared<GitConfig>>,
    index: CAtomicOwner<CShared<GitIndex>>,
    /// `git_cache objects`: `Sync` (its map is behind its rwlock).
    objects: GitCache,
    attrcache: AtomicPtr<c_void>,
    diff_drivers: AtomicPtr<c_void>,
    gitlink: CAtomicOwner<GitCStr>,
    gitdir: CAtomicOwner<GitCStr>,
    commondir: CAtomicOwner<GitCStr>,
    workdir: CAtomicOwner<GitCStr>,
    namespace: CAtomicOwner<GitCStr>,
    ident_name: CAtomicOwner<GitCStr>,
    ident_email: CAtomicOwner<GitCStr>,
    reserved_names: UnsafeCell<ReservedNames>,
    bits: CAtomicBitfield<u32>,
    oid_type: AtomicU32,
    refdb_type: AtomicI32,
    lru_counter: AtomicU32,
    grafts: CAtomicOwner<GraftsBox>,
    shallow_grafts: CAtomicOwner<GraftsBox>,
    attr_session_key: Atomic32,
    configmap_cache: [AtomicIsize; GIT_CONFIGMAP_CACHE_MAX],
    submodule_cache: AtomicPtr<c_void>,
}

// SAFETY: the fields, and why each may move to and be shared between
// threads:
// - `odb`, `refdb`, `config`, `index`, `gitlink` … `ident_email`, `grafts`,
//   `shallow_grafts`: `CAtomicOwner`s (an atomic pointer to an owned
//   `CShared` / `GitCStr` / `GraftsBox`), published with compare-and-swap
//   and replaced only by the `unsafe` setters, whose contracts keep other
//   users away; the owned objects are refcounted with atomics or plain
//   strings, released from any thread; the grafts the two graft owners
//   hold are written (refreshed in place) only by the `unsafe`
//   `open::load_grafts` / `head::shallow_roots*`, whose contracts keep
//   every other user of the repository away, and are otherwise only read;
// - `objects`: `GitCache`, `Sync` (its map is behind its rwlock);
// - `attrcache`, `diff_drivers`, `submodule_cache`: `AtomicPtr`s to C
//   objects, which the Rust never dereferences (it only creates, swaps out
//   and frees them: `submodule_cache_clear` is `unsafe`, with a contract
//   that keeps other users away). The C that reaches their contents does
//   so under C's rules: the attribute cache has its own lock; the diff
//   driver registry (`git_diff_driver_load` inserts into its map
//   unlocked) and the submodule cache (a plain map) have none, so C uses
//   them, as in C, only from one thread at a time on a repository (libgit2
//   documents no thread safety for them: docs/threading.md). The safe API
//   reaches the submodule cache only through `&mut Repository`, and loads
//   no diff content;
// - `bits`, `oid_type`, `refdb_type`, `lru_counter`, `attr_session_key`,
//   `configmap_cache`: atomics;
// - `reserved_names`: an `UnsafeCell`, written only by `alloc` (before the
//   repository is shared) and `free` (after its last user), never while
//   shared.
// Nothing is tied to the thread that made the repository.
unsafe impl Send for GitRepository {}
// SAFETY: as for `Send`: every access through `&GitRepository` is atomic,
// behind a lock, or (`reserved_names`) does not happen while shared.
unsafe impl Sync for GitRepository {}

/// Which string field ([`GitRepository::replace_string`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrField {
    Gitlink,
    Gitdir,
    Commondir,
    Workdir,
    Namespace,
}

impl GitRepository {
    /// `repository_alloc`: a zeroed repository in `git__allocator`'s
    /// memory, its cache initialized, its configmap cache cleared and room
    /// for four reserved names. NULL (with the out-of-memory error) when an
    /// allocation fails.
    pub(crate) fn alloc() -> *mut GitRepository {
        let repo = alloc::calloc(1, size_of::<GitRepository>()).cast::<GitRepository>();
        if repo.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: a fresh, zeroed allocation of the struct's size and
        // alignment (8, `git__allocator`'s), no one else sees; the cache
        // is written in place before any reference is made
        // (`git_cache_init`, which cannot fail here).
        let objects = unsafe { &raw mut (*repo).objects };
        // SAFETY: as above: in bounds, aligned, not yet referenced.
        unsafe { objects.write(GitCache::new()) };
        // SAFETY: as above; all-zero bytes are a valid `GitRepository` but
        // for the cache (NULL pointers, zero integers), set above.
        let r = unsafe { &*repo };
        let names = alloc::mallocarray(4, size_of::<GitStr>()).cast::<GitStr>();
        if names.is_null() {
            // SAFETY: the allocation above, referenced by nothing else; the
            // fresh cache owns nothing (C's `git_cache_dispose`).
            unsafe { alloc::free(repo.cast()) };
            return ptr::null_mut();
        }
        // SAFETY: the repository is ours alone (not yet returned), so no
        // other reference to the cell's contents exists; the old value is
        // all zero (nothing to drop).
        unsafe {
            *r.reserved_names.get() = ReservedNames {
                ptr: names,
                size: 0,
                asize: 4,
            }
        };
        r.configmap_cache_clear();
        live::register(repo);
        repo
    }

    /// `git_repository_free`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a repository from [`GitRepository::alloc`] that
    /// nothing uses any more.
    pub(crate) unsafe fn free(repo: *mut GitRepository) {
        if repo.is_null() {
            return;
        }
        // First, so that a holder without a reference (a file config
        // backend) stops reaching it, and in-flight uses end.
        live::unregister(repo);
        {
            // SAFETY: a live repository from `alloc` (the contract), aligned;
            // the reference ends with this block.
            let r = unsafe { &*repo };
            // SAFETY: nothing uses the repository or its weak pointers any
            // more (the contract), as `cleanup` requires.
            let _ = unsafe { r.cleanup() };
        }
        // SAFETY: nothing else uses the repository (the contract) and no
        // reference to it is live (the one above has ended): the `&mut`
        // to its cache is the only one.
        unsafe { (*repo).objects.dispose() };
        {
            // SAFETY: as above, for this block.
            let r = unsafe { &*repo };
            // SAFETY: the registry (NULL or C's, set with compare-and-swap),
            // taken out of the repository, which nothing uses any more.
            unsafe {
                crate::sys::diff_driver::registry_free(
                    r.diff_drivers.swap(ptr::null_mut(), Ordering::AcqRel),
                )
            };
            // SAFETY: the repository's own array, which nothing else uses
            // now (the contract): the only reference to the cell's contents.
            let names = unsafe { &mut *r.reserved_names.get() };
            for i in 0..names.size {
                let name = names.ptr.wrapping_add(i);
                // SAFETY: entries `0..size` (`size <= asize`) of the
                // array, aligned, are initialized strings that nothing
                // else references.
                let name = unsafe { &mut *name };
                name.dispose();
            }
            // SAFETY: the array's buffer, from `git__allocator`, freed
            // once.
            unsafe { alloc::free(names.ptr.cast()) };
            for f in [
                &r.gitlink,
                &r.gitdir,
                &r.commondir,
                &r.workdir,
                &r.namespace,
                &r.ident_name,
                &r.ident_email,
            ] {
                // SAFETY: no borrow of the repository's strings is live
                // (the contract: nothing uses the repository), so the old
                // string may be freed.
                drop(unsafe { f.replace(None) });
            }
        }
        // SAFETY: the whole allocation (`alloc`'s), no longer referenced;
        // then freed (`git__memzero` + `git__free`).
        unsafe { ptr::write_bytes(repo.cast::<u8>(), 0, size_of::<GitRepository>()) };
        // SAFETY: as above: from `git__allocator`, freed once.
        unsafe { alloc::free(repo.cast()) };
    }

    /// `git_repository__cleanup`: drop the caches, grafts and the
    /// odb, config, refdb and index.
    ///
    /// # Safety
    ///
    /// No weak pointer (`config_weak`, `odb_weak`, …), grafts or cache entry
    /// of this repository is used after the call, and nothing uses the
    /// repository concurrently (C's rule).
    pub(crate) unsafe fn cleanup(&self) -> Result<()> {
        // SAFETY: nothing uses the repository concurrently (the contract),
        // so nothing uses its submodule cache.
        let _ = unsafe { self.submodule_cache_clear() };
        self.objects.clear();
        let _ = crate::sys::attrcache::flush(self);
        // SAFETY: the grafts are not used after the call (the contract),
        // so the old ones may be freed.
        drop(unsafe { self.grafts.replace(None) });
        // SAFETY: as above.
        drop(unsafe { self.shallow_grafts.replace(None) });
        // SAFETY: no weak pointer of the old config (or of the others
        // below) is used after the call and none is borrowed (the
        // contract), as the setters require.
        unsafe { self.set_config(None) };
        // SAFETY: as above.
        unsafe { self.set_index(None) };
        // SAFETY: as above.
        unsafe { self.set_odb(None) };
        // SAFETY: as above.
        unsafe { self.set_refdb(None) };
        Ok(())
    }

    /// The `git_repository *` for C.
    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }

    /// `repo->objects`, the cache of parsed and raw objects.
    #[inline]
    pub fn objects(&self) -> &GitCache {
        &self.objects
    }

    /// `&repo->submodule_cache`, for C to set.
    #[inline]
    pub(crate) fn submodule_cache_slot(&self) -> *mut *mut c_void {
        self.submodule_cache.as_ptr()
    }

    fn string_field(&self, f: StrField) -> &CAtomicOwner<GitCStr> {
        match f {
            StrField::Gitlink => &self.gitlink,
            StrField::Gitdir => &self.gitdir,
            StrField::Commondir => &self.commondir,
            StrField::Workdir => &self.workdir,
            StrField::Namespace => &self.namespace,
        }
    }

    fn read_str<'a>(&self, f: &'a CAtomicOwner<GitCStr>) -> Option<&'a CStr> {
        // A string field is NULL or a string the repository owns, replaced
        // only by `replace_string` and `free`, whose callers guarantee no
        // borrow from `&self` is live (see the module documentation).
        f.get().map(GitCStr::as_c_str)
    }

    /// `repo->gitdir`.
    pub fn gitdir(&self) -> Option<&CStr> {
        self.read_str(&self.gitdir)
    }
    /// `repo->workdir` (also for a bare repository, as the field).
    pub fn workdir_field(&self) -> Option<&CStr> {
        self.read_str(&self.workdir)
    }
    /// `git_repository_workdir`: NULL for a bare repository.
    pub fn workdir(&self) -> Option<&CStr> {
        if self.is_bare() {
            None
        } else {
            self.workdir_field()
        }
    }
    /// `repo->commondir`.
    pub fn commondir(&self) -> Option<&CStr> {
        self.read_str(&self.commondir)
    }
    /// `repo->gitlink`.
    pub fn gitlink(&self) -> Option<&CStr> {
        self.read_str(&self.gitlink)
    }
    /// `repo->namespace`.
    pub fn namespace(&self) -> Option<&CStr> {
        self.read_str(&self.namespace)
    }
    /// `repo->ident_name`.
    pub fn ident_name(&self) -> Option<&CStr> {
        self.read_str(&self.ident_name)
    }
    /// `repo->ident_email`.
    pub fn ident_email(&self) -> Option<&CStr> {
        self.read_str(&self.ident_email)
    }

    /// The raw pointer of a string field (for C's `const char *` results).
    pub(crate) fn string_ptr(&self, f: StrField) -> *const c_char {
        self.string_field(f).as_ptr()
    }

    /// Replace a string field, freeing the old value.
    ///
    /// # Safety
    ///
    /// No string borrowed from this field (through any `&GitRepository`,
    /// or by C) is used after the call.
    pub(crate) unsafe fn replace_string(&self, f: StrField, v: Option<GitCStr>) {
        // SAFETY: no borrow of the old string is used after the call (the
        // contract), so it may be freed.
        drop(unsafe { self.string_field(f).replace(v) });
    }

    /// Set a string field that is still NULL (while the repository is
    /// being opened). A field that is already set is left alone (nothing is
    /// freed that a borrow may still use) and `v` is dropped.
    pub(crate) fn init_string(&self, f: StrField, v: GitCStr) {
        if self.string_field(f).set(v).is_err() {
            debug_assert!(false, "init_string on a set field");
        }
    }

    /// `repo->use_env`.
    pub fn use_env(&self) -> bool {
        self.bits.is_set(USE_ENV, Ordering::Relaxed)
    }
    pub(crate) fn set_use_env(&self, v: bool) {
        let _ = self.bits.set_flag(USE_ENV, v, Ordering::Relaxed);
    }
    /// `repo->is_bare`.
    pub fn is_bare(&self) -> bool {
        self.bits.is_set(IS_BARE, Ordering::Relaxed)
    }
    pub(crate) fn set_is_bare(&self, v: bool) {
        let _ = self.bits.set_flag(IS_BARE, v, Ordering::Relaxed);
    }
    /// `repo->is_worktree`.
    pub fn is_worktree(&self) -> bool {
        self.bits.is_set(IS_WORKTREE, Ordering::Relaxed)
    }
    pub(crate) fn set_is_worktree(&self, v: bool) {
        let _ = self.bits.set_flag(IS_WORKTREE, v, Ordering::Relaxed);
    }

    /// `repo->oid_type` as C stores it.
    pub fn oid_type_raw(&self) -> git_oid_t {
        self.oid_type.load(Ordering::Relaxed)
    }
    /// `repo->oid_type`, defaulting an unknown value to SHA-1.
    pub fn oid_type(&self) -> OidType {
        OidType::from_raw(self.oid_type_raw()).unwrap_or(OidType::DEFAULT)
    }
    pub(crate) fn set_oid_type_raw(&self, t: git_oid_t) {
        self.oid_type.store(t, Ordering::Relaxed)
    }
    /// `repo->refdb_type`.
    pub fn refdb_type(&self) -> c_int {
        self.refdb_type.load(Ordering::Relaxed)
    }
    pub(crate) fn set_refdb_type(&self, t: c_int) {
        self.refdb_type.store(t, Ordering::Relaxed)
    }

    /// `repo->configmap_cache`: C reads and writes it atomically.
    pub(crate) fn configmap_cache(&self) -> &[AtomicIsize; GIT_CONFIGMAP_CACHE_MAX] {
        &self.configmap_cache
    }

    /// `git_repository__configmap_lookup_cache_clear` (config_cache.c):
    /// every entry to `GIT_CONFIGMAP_NOT_CACHED`.
    pub(crate) fn configmap_cache_clear(&self) {
        crate::config_cache::configmap_lookup_cache_clear(self)
    }

    /// `git_repository__configmap_lookup` (config_cache.c). `item` is a
    /// `git_configmap_item` (below `GIT_CONFIGMAP_CACHE_MAX`).
    pub(crate) fn configmap_lookup(&self, item: c_int) -> Result<c_int> {
        crate::config_cache::repository_configmap_lookup(self, item as usize)
    }

    // ----- the lazily created odb, config, refdb and index -----------------

    /// `set_odb`: own (and take a reference of) `odb`, releasing the old.
    ///
    /// # Safety
    ///
    /// The new object, if any, is a live `git_odb` of which the caller
    /// holds a reference ([`Odb::as_raw`], or a pointer C passed); no weak
    /// pointer to the old one (from [`GitRepository::odb_weak`], or C's
    /// `git_repository_odb__weakptr`) is used after the call, and no
    /// borrow of it is live.
    pub(crate) unsafe fn set_odb(&self, odb: Option<NonNull<GitOdb>>) {
        // SAFETY: this function's contract is `set_owned`'s.
        unsafe { self.set_owned(&self.odb, odb) };
    }
    /// `set_refdb`.
    ///
    /// # Safety
    ///
    /// As [`GitRepository::set_odb`], for a `git_refdb`.
    pub(crate) unsafe fn set_refdb(&self, refdb: Option<NonNull<GitRefdb>>) {
        // SAFETY: this function's contract is `set_owned`'s.
        unsafe { self.set_owned(&self.refdb, refdb) };
    }
    /// `set_config`: also clears the configmap cache.
    ///
    /// # Safety
    ///
    /// As [`GitRepository::set_odb`], for a `git_config` (and no
    /// `&GitConfig` from [`GitRepository::config_weak`] is used after).
    pub(crate) unsafe fn set_config(&self, config: Option<NonNull<GitConfig>>) {
        // SAFETY: this function's contract is `set_owned`'s.
        unsafe { self.set_owned(&self.config, config) };
        self.configmap_cache_clear();
    }
    /// `set_index`.
    ///
    /// # Safety
    ///
    /// As [`GitRepository::set_odb`], for a `git_index` of the index
    /// module.
    pub(crate) unsafe fn set_index(&self, index: Option<NonNull<GitIndex>>) {
        // SAFETY: this function's contract is `set_owned`'s.
        unsafe { self.set_owned(&self.index, index) };
    }

    /// Store a new reference to `new` in `slot`, owned by the repository
    /// (C's `GIT_REFCOUNT_OWN` + `GIT_REFCOUNT_INC`); the old object is
    /// disowned and its reference dropped.
    ///
    /// # Safety
    ///
    /// As [`GitRepository::set_odb`].
    pub(crate) unsafe fn set_owned<T: StartsWithRefcount>(
        &self,
        slot: &CAtomicOwner<CShared<T>>,
        new: Option<NonNull<T>>,
    ) {
        // SAFETY: the caller holds a reference to the live object (the
        // contract), so taking another is sound.
        let new = new.and_then(|p| unsafe { CShared::from_borrowed(p.as_ptr()) });
        if let Some(n) = &new {
            // SAFETY: the repository owns the object from now until it
            // clears the owner (below, when the slot is replaced, or in
            // `cleanup`), and outlives it while it does.
            unsafe { T::own(n, self.as_ptr()) };
        }
        // SAFETY: no borrow of the old object (and no weak pointer C keeps)
        // is used, or live, after this (the contract), so the slot may drop
        // its reference.
        let old = unsafe { slot.replace(new) };
        if let Some(old) = old {
            // SAFETY: NULL: the object is no longer the repository's.
            unsafe { T::own(&old, ptr::null_mut()) };
            drop(old);
        }
    }

    /// Publish a freshly made object in `slot` unless another thread was
    /// first (C's `git_atomic_compare_and_swap(&slot, NULL, obj)`): the
    /// loser is disowned and released. Returns the published object.
    pub(crate) fn publish<'s, T: StartsWithRefcount>(
        &self,
        slot: &'s CAtomicOwner<CShared<T>>,
        obj: CShared<T>,
    ) -> &'s CShared<T> {
        // SAFETY: `obj` is a reference the caller holds; the repository
        // owns what it publishes (as in `set_owned`).
        unsafe { T::own(&obj, self.as_ptr()) };
        match slot.try_insert(obj) {
            Ok(won) => won,
            Err((won, loser)) => {
                // SAFETY: NULL: the loser is not the repository's.
                unsafe { T::own(&loser, ptr::null_mut()) };
                drop(loser);
                won
            }
        }
    }

    /// `git_repository_config__weakptr`.
    pub fn config_weak(&self) -> Result<&GitConfig> {
        if let Some(c) = self.config.get() {
            return Ok(c);
        }
        let use_env = self.use_env();
        let mut system = GitStr::new();
        let mut global = GitStr::new();
        let mut xdg = GitStr::new();
        open::config_path_system(&mut system, use_env)?;
        open::config_path_global(&mut global, use_env)?;
        // As C, a missing file is an empty path.
        let _ = crate::config::find_xdg(&mut xdg);
        if global.is_empty() {
            let _ = crate::config::global_location(&mut global);
        }
        let cfg = open::load_config(
            Some(self),
            path_unless_empty(&global).as_deref(),
            path_unless_empty(&xdg).as_deref(),
            path_unless_empty(&system).as_deref(),
        )?;
        // The repository's config, live while it is set (the repository
        // holds a reference); replacing it is the caller's concern, as for
        // C's weak pointers.
        Ok(self.publish(&self.config, cfg.into_shared()))
    }

    /// `git_repository_odb__weakptr`: the odb pointer (not referenced).
    pub fn odb_weak(&self) -> Result<NonNull<GitOdb>> {
        self.odb()?;
        self.odb.get().map(shared_nn).ok_or(ErrorCode::GENERIC)
    }

    /// The repository's odb, created if needed (`git_repository_odb__weakptr`),
    /// borrowed for as long as the repository keeps it (replacing it is the
    /// caller's concern, as for C's weak pointers).
    pub fn odb(&self) -> Result<&GitOdb> {
        if let Some(db) = self.odb.get() {
            return Ok(db);
        }
        let mut path = GitStr::new();
        open::repository_odb_path(&mut path, self)?;
        let odb = new_odb(self.oid_type_raw_or_default())?;
        open::repository_odb_alternates(&odb, self)?;
        let odb = odb.into_shared();
        let db: &GitOdb = &odb;
        // Owned first: `GIT_ODB_CAP_FROM_OWNER` reads the owner's config.
        // SAFETY: the repository owns the odb from here (it is published
        // below, or disowned on failure).
        unsafe { GitOdb::own(&odb, self.as_ptr()) };
        let r = db
            .set_caps(crate::odb::GIT_ODB_CAP_FROM_OWNER)
            .and_then(|()| db.add_default_backends(&cs(&path), false, 0));
        if let Err(e) = r {
            // C frees the odb with its owner still set, which leaks it
            // (DEFECTS.md): released here.
            // SAFETY: NULL: the odb is not the repository's after all.
            unsafe { GitOdb::own(&odb, ptr::null_mut()) };
            return Err(e);
        }
        Ok(self.publish(&self.odb, odb))
    }

    fn oid_type_raw_or_default(&self) -> OidType {
        OidType::from_raw(self.oid_type_raw()).unwrap_or(OidType::DEFAULT)
    }

    /// `git_repository_refdb__weakptr`.
    pub fn refdb_weak(&self) -> Result<NonNull<GitRefdb>> {
        if let Some(db) = self.refdb.get() {
            return Ok(shared_nn(db));
        }
        // SAFETY: this live repository (`Sync`). The refdb and its backend
        // keep the pointer as C's weak `db->repo`, followed only while the
        // repository is live: the repository owns them, and a count taken
        // past its life (C's `git_repository_refdb`, the safe API's
        // `Refdb<'r>`) may not reach the repository after it is freed (C's
        // rule; the safe API's lifetime).
        let me = unsafe { crate::refs::deps::RepoPtr::new(NonNull::from(self)) };
        let db = crate::refdb::open(me)?;
        Ok(shared_nn(self.publish(&self.refdb, db)))
    }

    /// `git_repository_index__weakptr`.
    pub fn index_weak(&self) -> Result<NonNull<GitIndex>> {
        if let Some(ix) = self.index.get() {
            return Ok(shared_nn(ix));
        }
        let mut path = GitStr::new();
        open::repository_index_path(&mut path, self)?;
        let index = crate::index::ops::open_ext(
            Some(&crate::util::fs_path::str_cpath(&path)),
            self.oid_type_raw_or_default(),
        )?;
        // SAFETY: `open_ext` gives up the index's one reference.
        let index = unsafe { CShared::from_raw(index.as_ptr()) }.ok_or(ErrorCode::GENERIC)?;
        let published = shared_nn(self.publish(&self.index, index));
        // SAFETY: the published index, live while the repository holds
        // it; C calls `git_index_set_caps` on it the same way (also when
        // another thread published it).
        let ix = unsafe { IndexPtr::new(published) };
        // Under the safe API's lock: when another thread published the
        // index, it may already be in use through an `Index`.
        crate::index::api::with_index_locked(published, || {
            crate::index::ops::set_caps(ix, GIT_INDEX_CAPABILITY_FROM_OWNER)
        })?;
        Ok(published)
    }

    /// The current `_odb`, `_refdb`, `_config` and `_index` pointers (what
    /// C's weak-pointer functions write out, also on failure).
    pub(crate) fn odb_ptr(&self) -> *mut GitOdb {
        self.odb.as_ptr()
    }
    pub(crate) fn refdb_ptr(&self) -> *mut GitRefdb {
        self.refdb.as_ptr()
    }
    pub(crate) fn config_ptr(&self) -> *mut GitConfig {
        self.config.as_ptr()
    }
    pub(crate) fn index_ptr(&self) -> *mut GitIndex {
        self.index.as_ptr()
    }

    /// `git_repository_head`, for the crate's modules.
    pub(crate) fn head_ref(&self) -> Result<Reference<'_>> {
        head::head(self)
    }

    /// `git_repository_workdir_path`, for the crate's modules.
    pub(crate) fn workdir_path(&self, out: &mut GitStr, path: &[u8]) -> Result<()> {
        head::workdir_path(out, self, path)
    }

    /// `repo->grafts`.
    pub(crate) fn grafts_ptr(&self) -> *mut Grafts {
        self.grafts.as_ptr()
    }
    /// `repo->shallow_grafts`.
    pub(crate) fn shallow_grafts_ptr(&self) -> *mut Grafts {
        self.shallow_grafts.as_ptr()
    }
    /// Set `repo->grafts` if it is still NULL (`g` is dropped otherwise).
    pub(crate) fn init_grafts(&self, g: GraftsBox) {
        let _ = self.grafts.set(g);
    }
    /// Set `repo->shallow_grafts` if it is still NULL.
    pub(crate) fn init_shallow_grafts(&self, g: GraftsBox) {
        let _ = self.shallow_grafts.set(g);
    }

    /// `git_repository_submodule_cache_clear`.
    ///
    /// # Safety
    ///
    /// Nothing else uses the submodule cache during the call (C's rule: a
    /// lookup that loaded the cache pointer would use freed memory).
    pub(crate) unsafe fn submodule_cache_clear(&self) -> c_int {
        // Taken out first (C frees, then clears): two clears cannot free the
        // same cache.
        let cache = self.submodule_cache.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: NULL or the repository's cache, no longer reachable from
        // it and used by nobody else (the contract).
        unsafe { crate::sys::submodule::cache_free(cache) }
    }

    /// `git_repository_set_ident`.
    ///
    /// # Safety
    ///
    /// No string returned by [`ident_name`](Self::ident_name) /
    /// [`ident_email`](Self::ident_email) (or `git_repository_ident`) is
    /// used after the call: the old strings are freed.
    pub(crate) unsafe fn set_ident(&self, name: Option<&CStr>, email: Option<&CStr>) -> Result<()> {
        let n = match name {
            Some(n) => Some(alloc_cstr(n.to_bytes())?),
            None => None,
        };
        let e = match email {
            Some(e) => Some(alloc_cstr(e.to_bytes())?),
            None => None,
        };
        // SAFETY: no borrow of the old strings is used after the call (the
        // contract), so they may be freed (C swaps and frees them too).
        drop(unsafe { self.ident_name.replace(n) });
        // SAFETY: as above.
        drop(unsafe { self.ident_email.replace(e) });
        Ok(())
    }
}

/// `git_repository_open(path)`, owned.
pub(crate) fn open_owned(path: &CStr) -> Result<OwnedRepo> {
    let r = open::open_ext(Some(path), GIT_REPOSITORY_OPEN_NO_SEARCH, None, true)?;
    let r = r.ok_or(ErrorCode::GENERIC)?;
    // SAFETY: the repository `open_ext` just opened, ours.
    Ok(unsafe { OwnedRepo::new(r) })
}

/// `git_odb_new_ext` for `oid_type`, with the C's errors.
pub(crate) fn new_odb(oid_type: OidType) -> Result<Odb<'static>> {
    let opts = crate::odb::GitOdbOptions {
        version: crate::odb::GIT_ODB_OPTIONS_VERSION,
        oid_type: oid_type.raw(),
    };
    Ok(Odb::from_shared(GitOdb::new(Some(&opts))?))
}

/// `git_repository__free_extensions` (settings' shutdown).
pub(crate) fn ffi_free_extensions() {
    open::free_extensions();
}

/// A copy of `s` in `git__allocator`'s memory.
pub(crate) fn alloc_cstr(s: &[u8]) -> Result<GitCStr> {
    // SAFETY: `substrdup` returns NULL (with the error set) or a fresh
    // NUL-terminated copy we own.
    unsafe { GitCStr::from_raw(alloc::substrdup(s)) }.ok_or(ErrorCode::GENERIC)
}

/// `git_str_detach` into a `GitCStr` (an empty string still gives one).
pub(crate) fn detach(s: &mut GitStr) -> Result<GitCStr> {
    match s.detach() {
        Some(c) => Ok(c),
        None => alloc_cstr(b""),
    }
}

/// `path_unless_empty`.
pub(crate) fn path_unless_empty(s: &GitStr) -> Option<std::borrow::Cow<'_, CStr>> {
    if s.is_empty() {
        None
    } else {
        Some(crate::util::fs_path::str_cpath(s))
    }
}

/// The C string of a `git_str` (as C reads `s->ptr`).
pub(crate) fn cs(s: &GitStr) -> std::borrow::Cow<'_, CStr> {
    crate::util::fs_path::str_cpath(s)
}

// ---------------------------------------------------------------------------
// Item paths
// ---------------------------------------------------------------------------

/// `git_repository_item_t`.
pub type ItemT = c_uint;
/// `GIT_REPOSITORY_ITEM_GITDIR`.
pub const ITEM_GITDIR: ItemT = 0;
/// `GIT_REPOSITORY_ITEM_WORKDIR`.
pub const ITEM_WORKDIR: ItemT = 1;
/// `GIT_REPOSITORY_ITEM_COMMONDIR`.
pub const ITEM_COMMONDIR: ItemT = 2;
/// `GIT_REPOSITORY_ITEM_INDEX`.
pub const ITEM_INDEX: ItemT = 3;
/// `GIT_REPOSITORY_ITEM_OBJECTS`.
pub const ITEM_OBJECTS: ItemT = 4;
/// `GIT_REPOSITORY_ITEM_CONFIG`.
pub const ITEM_CONFIG: ItemT = 8;
/// `GIT_REPOSITORY_ITEM_INFO`.
pub const ITEM_INFO: ItemT = 9;
/// `GIT_REPOSITORY_ITEM_WORKTREE_CONFIG`.
pub const ITEM_WORKTREE_CONFIG: ItemT = 14;
/// `GIT_REPOSITORY_ITEM__LAST`.
pub const ITEM_LAST: ItemT = 15;

struct Item {
    parent: ItemT,
    fallback: ItemT,
    name: Option<&'static [u8]>,
    directory: bool,
}

const fn item(
    parent: ItemT,
    fallback: ItemT,
    name: Option<&'static [u8]>,
    directory: bool,
) -> Item {
    Item {
        parent,
        fallback,
        name,
        directory,
    }
}

static ITEMS: [Item; 15] = [
    item(ITEM_GITDIR, ITEM_LAST, None, true),
    item(ITEM_WORKDIR, ITEM_LAST, None, true),
    item(ITEM_COMMONDIR, ITEM_LAST, None, true),
    item(ITEM_GITDIR, ITEM_LAST, Some(b"index"), false),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"objects"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"refs"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"packed-refs"), false),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"remotes"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"config"), false),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"info"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"hooks"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"logs"), true),
    item(ITEM_GITDIR, ITEM_LAST, Some(b"modules"), true),
    item(ITEM_COMMONDIR, ITEM_GITDIR, Some(b"worktrees"), true),
    item(ITEM_GITDIR, ITEM_GITDIR, Some(b"config.worktree"), false),
];

/// The directories `resolved_parent_path` picks from.
#[derive(Clone, Copy, Default, Debug)]
pub struct ItemDirs<'a> {
    /// `git_repository_path`.
    pub gitdir: Option<&'a [u8]>,
    /// `git_repository_workdir`.
    pub workdir: Option<&'a [u8]>,
    /// `git_repository_commondir`.
    pub commondir: Option<&'a [u8]>,
}

impl GitRepository {
    /// The directories of this repository, for [`item_path`].
    pub fn item_dirs(&self) -> ItemDirs<'_> {
        ItemDirs {
            gitdir: self.gitdir().map(CStr::to_bytes),
            workdir: self.workdir().map(CStr::to_bytes),
            commondir: self.commondir().map(CStr::to_bytes),
        }
    }
}

fn resolved_parent_path<'a>(
    get: &impl Fn(ItemT) -> Option<&'a [u8]>,
    item: ItemT,
    fallback: ItemT,
) -> Result<Option<&'a [u8]>> {
    if item > ITEM_COMMONDIR {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid item directory"
        ));
    }
    let parent = get(item);
    if parent.is_none() && fallback != ITEM_LAST {
        return resolved_parent_path(get, fallback, ITEM_LAST);
    }
    Ok(parent)
}

/// `git_repository__item_path`, over the directories `dirs`.
///
/// An `item` beyond the table is rejected with "invalid item directory"
/// (the C reads past its table: see DEFECTS.md).
pub fn item_path(out: &mut GitStr, dirs: &ItemDirs<'_>, item: ItemT) -> Result<()> {
    item_path_with(out, item, |i| match i {
        ITEM_GITDIR => dirs.gitdir,
        ITEM_WORKDIR => dirs.workdir,
        _ => dirs.commondir,
    })
}

/// `git_repository__item_path`, with the directories looked up (lazily) by
/// `get` (`ITEM_GITDIR`, `ITEM_WORKDIR` or `ITEM_COMMONDIR`).
fn item_path_with<'a>(
    out: &mut GitStr,
    item: ItemT,
    get: impl Fn(ItemT) -> Option<&'a [u8]>,
) -> Result<()> {
    let Some(it) = ITEMS.get(item as usize) else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid item directory"
        ));
    };
    // C: a NULL parent (also after "invalid item directory") is reported
    // as "path cannot exist in repository".
    let parent = resolved_parent_path(&get, it.parent, it.fallback)
        .ok()
        .flatten();
    let Some(parent) = parent else {
        let _ = git_error_set!(ErrorClass::INVALID, "path cannot exist in repository");
        return Err(ErrorCode::NOT_FOUND);
    };
    match it.name {
        // C's `git_str_sets(out, parent)` then `git_str_joinpath(out,
        // parent, name)`: the join alone gives the same contents.
        Some(name) => out.joinpath(parent, name)?,
        None => out.sets(parent)?,
    }
    if it.directory {
        crate::util::fs_path::to_dir(out)?;
    }
    Ok(())
}

impl GitRepository {
    /// `git_repository__item_path` of this repository.
    pub fn item_path(&self, out: &mut GitStr, item: ItemT) -> Result<()> {
        item_path_with(out, item, |i| {
            match i {
                ITEM_GITDIR => self.gitdir(),
                ITEM_WORKDIR => self.workdir(),
                _ => self.commondir(),
            }
            .map(CStr::to_bytes)
        })
    }
}

// ---------------------------------------------------------------------------
// The globals of repository.c
// ---------------------------------------------------------------------------

/// `git_repository__validate_ownership` (`bool`): set by
/// `GIT_OPT_SET_OWNER_VALIDATION`, read by C tests too.
#[unsafe(no_mangle)]
pub static git_repository__validate_ownership: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// `git_repository__fsync_gitdir` (`bool`): read by `filebuf`'s callers in
/// C (`GIT_OPT_ENABLE_FSYNC_GITDIR`).
#[unsafe(no_mangle)]
pub static git_repository__fsync_gitdir: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Whether git-directory files are fsync'd (`git_repository__fsync_gitdir`).
pub fn fsync_gitdir() -> bool {
    git_repository__fsync_gitdir.load(core::sync::atomic::Ordering::Relaxed)
}

/// A static `git_str` (`{ ptr, 0, size }`, borrowed) of the reserved names.
#[repr(C)]
pub struct StaticStr {
    ptr: *const c_char,
    asize: usize,
    size: usize,
}

// SAFETY: the strings are immutable statics.
unsafe impl Sync for StaticStr {}

/// `git_repository__reserved_names_win32`: `.git`, `GIT~1`.
#[unsafe(no_mangle)]
pub static git_repository__reserved_names_win32: [StaticStr; 2] = [
    StaticStr {
        ptr: c".git".as_ptr(),
        asize: 0,
        size: 4,
    },
    StaticStr {
        ptr: c"GIT~1".as_ptr(),
        asize: 0,
        size: 5,
    },
];
/// `git_repository__reserved_names_win32_len`.
#[unsafe(no_mangle)]
pub static git_repository__reserved_names_win32_len: usize = 2;

/// `git_repository__reserved_names_posix`: `.git`.
#[unsafe(no_mangle)]
pub static git_repository__reserved_names_posix: [StaticStr; 1] = [StaticStr {
    ptr: c".git".as_ptr(),
    asize: 0,
    size: 4,
}];
/// `git_repository__reserved_names_posix_len`.
#[unsafe(no_mangle)]
pub static git_repository__reserved_names_posix_len: usize = 1;

/// `GIT_OID_DEFAULT`.
pub(crate) const GIT_OID_DEFAULT: git_oid_t = GIT_OID_SHA1;

crate::c_layout_test! {
    headers: ["repository.h", "odb.h", "worktree.h", "annotated_commit.h"],
    types: [
        GitRepository => "git_repository" {
            odb as "_odb", refdb as "_refdb", config as "_config", index as "_index",
            objects, attrcache, diff_drivers, gitlink, gitdir, commondir, workdir,
            namespace, ident_name, ident_email, reserved_names, oid_type, refdb_type,
            lru_counter, grafts, shallow_grafts, attr_session_key, configmap_cache,
            submodule_cache
        },
        StaticStr => "git_str" { ptr, asize, size },
    ],
    asserts: [
        "offsetof(git_worktree, gitlink_path) == 16",
        "offsetof(git_annotated_commit, description) == 56",
        "sizeof(git_cache) == 13 * 8",
        "GIT_CONFIGMAP_CACHE_MAX == 15",
        "GIT_REPOSITORY_ITEM__LAST == 15",
        "GIT_CONFIGMAP_IGNORECASE == 3",
        "GIT_CONFIGMAP_ABBREV == 7",
        "GIT_CONFIGMAP_FSYNCOBJECTFILES == 13",
        "GIT_ABBREV_FALSE == 64",
        "offsetof(git_repository, reserved_names.ptr) == 208",
        "offsetof(git_repository, reserved_names.size) == 216",
        "offsetof(git_repository, reserved_names.asize) == 224",
        "offsetof(git_repository, oid_type) == 236",
    ],
}
