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
//! are `AtomicPtr`, the integers atomics, the embedded `git_cache` an
//! `UnsafeCell`. So a `&GitRepository` is sound while C (on this or another
//! thread) writes any field, and the type is `Sync` with real atomics.
//!
//! The bitfield `unsigned use_env:1, is_bare:1, is_worktree:1` is one
//! `unsigned` at offset 232: `use_env` is bit 0, `is_bare` bit 1,
//! `is_worktree` bit 2 (x86-64 SysV: the first declared is the least
//! significant bit).
//!
//! The string fields (`gitdir`, `workdir`, …) are NULL or NUL-terminated
//! strings of `git__allocator`, owned by the repository. Only
//! [`GitRepository::replace_string`] (`set_workdir`, `set_bare`,
//! `set_namespace`) and `git_repository_free` free them, so a string
//! borrowed from a `&GitRepository` stays valid until one of those runs;
//! their callers guarantee no such borrow is live (C's rule: these setters
//! are not called while the repository is used elsewhere).

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicPtr, AtomicU32, Ordering};

use crate::git_error_set;
use crate::oid::{GIT_OID_SHA1, OidType, git_oid_t};
use crate::sys::pending::l_repo as c;
use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::thread::{self, Atomic32};

pub mod api;
pub mod ffi;
pub mod handles;
mod head;
mod init;
mod open;
#[cfg(test)]
mod tests;

pub use api::{Repository, RepositoryInitOptions, RepositoryOpenFlags, RepositoryState};
pub use handles::{Config, Index, Odb, Refdb, Reference};

use handles::{ConfigRef, check};

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

/// `GIT_REFDB_FILES`.
pub const GIT_REFDB_FILES: c_int = 1;
/// `GIT_REFDB_REFTABLE`.
pub const GIT_REFDB_REFTABLE: c_int = 2;

/// `GIT_CONFIGMAP_CACHE_MAX`: the entries of `configmap_cache`.
pub const GIT_CONFIGMAP_CACHE_MAX: usize = 15;
/// `GIT_CONFIGMAP_IGNORECASE`.
pub const GIT_CONFIGMAP_IGNORECASE: c_int = 3;
/// `GIT_CONFIGMAP_ABBREV`.
pub const GIT_CONFIGMAP_ABBREV: c_int = 7;
/// `GIT_ABBREV_MINIMUM`.
pub const GIT_ABBREV_MINIMUM: c_int = 4;
/// `GIT_ABBREV_FALSE` (`GIT_OID_MAX_HEXSIZE`).
pub const GIT_ABBREV_FALSE: c_int = 64;

/// `GIT_CONFIG_LEVEL_SYSTEM`.
pub const GIT_CONFIG_LEVEL_SYSTEM: c_int = 2;
/// `GIT_CONFIG_LEVEL_XDG`.
pub const GIT_CONFIG_LEVEL_XDG: c_int = 3;
/// `GIT_CONFIG_LEVEL_GLOBAL`.
pub const GIT_CONFIG_LEVEL_GLOBAL: c_int = 4;
/// `GIT_CONFIG_LEVEL_LOCAL`.
pub const GIT_CONFIG_LEVEL_LOCAL: c_int = 5;
/// `GIT_CONFIG_LEVEL_WORKTREE`.
pub const GIT_CONFIG_LEVEL_WORKTREE: c_int = 6;

/// `GIT_ODB_CAP_FROM_OWNER`.
const GIT_ODB_CAP_FROM_OWNER: c_int = -1;
/// `GIT_INDEX_CAPABILITY_FROM_OWNER`.
const GIT_INDEX_CAPABILITY_FROM_OWNER: c_int = -1;

/// Bit of `use_env` in the bitfield.
const BIT_USE_ENV: u32 = 1 << 0;
/// Bit of `is_bare`.
const BIT_IS_BARE: u32 = 1 << 1;
/// Bit of `is_worktree`.
const BIT_IS_WORKTREE: u32 = 1 << 2;

/// `sizeof(git_cache)` in `u64` words (l-odb's type, embedded by value).
const GIT_CACHE_WORDS: usize = 13;

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
    odb: AtomicPtr<c_void>,
    refdb: AtomicPtr<c_void>,
    config: AtomicPtr<c_void>,
    index: AtomicPtr<c_void>,
    /// `git_cache objects` (PENDING(l-odb): `crate::cache::GitCache`):
    /// only `cache.c` touches it, through the pointer.
    objects: UnsafeCell<MaybeUninit<[u64; GIT_CACHE_WORDS]>>,
    attrcache: AtomicPtr<c_void>,
    diff_drivers: AtomicPtr<c_void>,
    gitlink: AtomicPtr<c_char>,
    gitdir: AtomicPtr<c_char>,
    commondir: AtomicPtr<c_char>,
    workdir: AtomicPtr<c_char>,
    namespace: AtomicPtr<c_char>,
    ident_name: AtomicPtr<c_char>,
    ident_email: AtomicPtr<c_char>,
    reserved_names: UnsafeCell<ReservedNames>,
    bits: AtomicU32,
    oid_type: AtomicU32,
    refdb_type: AtomicI32,
    lru_counter: AtomicU32,
    grafts: AtomicPtr<c_void>,
    shallow_grafts: AtomicPtr<c_void>,
    attr_session_key: Atomic32,
    configmap_cache: [AtomicIsize; GIT_CONFIGMAP_CACHE_MAX],
    submodule_cache: AtomicPtr<c_void>,
}

// SAFETY: every field is an atomic or behind an `UnsafeCell` that only C
// (or this module, under C's rules) touches; the objects it points to are
// released from any thread.
unsafe impl Send for GitRepository {}
// SAFETY: as above: shared access goes through atomics.
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

#[inline]
fn load<T>(p: &AtomicPtr<T>) -> *mut T {
    p.load(Ordering::Acquire)
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
        // alignment (8, `git__allocator`'s); all-zero bytes are a valid
        // `GitRepository` (NULL pointers, zero integers).
        let r = unsafe { &*repo };
        // SAFETY: the zeroed `git_cache` of a repository no one else sees.
        if unsafe { c::git_cache_init(r.objects_ptr()) } < 0 {
            // SAFETY: as above; the cache was not set up, and disposing a
            // zeroed one is what the C does.
            unsafe { c::git_cache_dispose(r.objects_ptr()) };
            // SAFETY: the allocation above, referenced by nothing else.
            unsafe { alloc::free(repo.cast()) };
            return ptr::null_mut();
        }
        let names = alloc::mallocarray(4, size_of::<GitStr>()).cast::<GitStr>();
        if names.is_null() {
            // SAFETY: as above.
            unsafe { c::git_cache_dispose(r.objects_ptr()) };
            // SAFETY: the allocation above, referenced by nothing else.
            unsafe { alloc::free(repo.cast()) };
            return ptr::null_mut();
        }
        // SAFETY: the array is the repository's alone.
        unsafe {
            *r.reserved_names.get() = ReservedNames {
                ptr: names,
                size: 0,
                asize: 4,
            }
        };
        r.configmap_cache_clear();
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
        {
            // SAFETY: the caller's contract.
            let r = unsafe { &*repo };
            let _ = r.cleanup();
            // SAFETY: the repository's cache, which nothing else uses now.
            unsafe { c::git_cache_dispose(r.objects_ptr()) };
            // SAFETY: the registry, set by C with compare-and-swap, no
            // longer used.
            unsafe {
                crate::sys::diff_driver::registry_free(
                    r.diff_drivers.swap(ptr::null_mut(), Ordering::AcqRel),
                )
            };
            // SAFETY: the repository's own array, no longer used.
            let names = unsafe { &mut *r.reserved_names.get() };
            for i in 0..names.size {
                // SAFETY: entries `0..size` are initialized strings.
                let name = unsafe { names.ptr.add(i) };
                // SAFETY: as above; nothing else references it.
                let name = unsafe { &mut *name };
                name.dispose();
            }
            // SAFETY: the array's buffer, from `git__allocator`.
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
                // SAFETY: NULL or the repository's own string.
                unsafe { alloc::free(f.swap(ptr::null_mut(), Ordering::AcqRel).cast()) };
            }
        }
        // SAFETY: the whole allocation, no longer referenced; then freed
        // (`git__memzero` + `git__free`).
        unsafe { ptr::write_bytes(repo.cast::<u8>(), 0, size_of::<GitRepository>()) };
        // SAFETY: as above.
        unsafe { alloc::free(repo.cast()) };
    }

    /// `git_repository__cleanup`: drop the caches, grafts and the
    /// odb, config, refdb and index.
    pub(crate) fn cleanup(&self) -> Result<()> {
        let _ = self.submodule_cache_clear();
        // SAFETY: the repository's own cache.
        unsafe { c::git_cache_clear(self.objects_ptr()) };
        let _ = crate::sys::attrcache::flush(self);
        // SAFETY: the grafts are the repository's; taken out first.
        unsafe { c::git_grafts_free(self.grafts.swap(ptr::null_mut(), Ordering::AcqRel)) };
        // SAFETY: as above.
        unsafe { c::git_grafts_free(self.shallow_grafts.swap(ptr::null_mut(), Ordering::AcqRel)) };
        self.set_config(None);
        self.set_index(None);
        self.set_odb(None);
        self.set_refdb(None);
        Ok(())
    }

    /// The `git_repository *` for C.
    #[inline]
    pub fn as_ptr(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }

    /// The `git_cache *` of `objects`.
    #[inline]
    pub(crate) fn objects_ptr(&self) -> *mut c_void {
        self.objects.get().cast()
    }

    /// `&repo->submodule_cache`, for C to set.
    #[inline]
    pub(crate) fn submodule_cache_slot(&self) -> *mut *mut c_void {
        self.submodule_cache.as_ptr()
    }

    fn string_field(&self, f: StrField) -> &AtomicPtr<c_char> {
        match f {
            StrField::Gitlink => &self.gitlink,
            StrField::Gitdir => &self.gitdir,
            StrField::Commondir => &self.commondir,
            StrField::Workdir => &self.workdir,
            StrField::Namespace => &self.namespace,
        }
    }

    fn read_str(&self, f: &AtomicPtr<c_char>) -> Option<&CStr> {
        let p = load(f);
        if p.is_null() {
            None
        } else {
            // SAFETY: a string field is NULL or a NUL-terminated string the
            // repository owns, freed only by `replace_string` and `free`,
            // whose callers guarantee no borrow from `&self` is live (see
            // the module documentation).
            Some(unsafe { CStr::from_ptr(p) })
        }
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
        load(self.string_field(f))
    }

    /// Replace a string field, freeing the old value.
    ///
    /// # Safety
    ///
    /// No string borrowed from this field (through any `&GitRepository`,
    /// or by C) is used after the call.
    pub(crate) unsafe fn replace_string(&self, f: StrField, v: Option<GitCStr>) {
        let new = v.map_or(ptr::null_mut(), GitCStr::into_raw);
        let old = self.string_field(f).swap(new, Ordering::AcqRel);
        // SAFETY: NULL or the field's old string, owned by the repository,
        // which the caller guarantees nothing uses.
        unsafe { alloc::free(old.cast()) };
    }

    /// Set a string field that is still NULL (while the repository is
    /// being opened, before anyone else sees it).
    pub(crate) fn init_string(&self, f: StrField, v: GitCStr) {
        let old = self.string_field(f).swap(v.into_raw(), Ordering::AcqRel);
        debug_assert!(old.is_null());
        // SAFETY: NULL or an unreferenced string of the repository.
        unsafe { alloc::free(old.cast()) };
    }

    #[inline]
    fn bit(&self, b: u32) -> bool {
        self.bits.load(Ordering::Relaxed) & b != 0
    }
    #[inline]
    fn set_bit(&self, b: u32, v: bool) {
        if v {
            self.bits.fetch_or(b, Ordering::Relaxed);
        } else {
            self.bits.fetch_and(!b, Ordering::Relaxed);
        }
    }
    /// `repo->use_env`.
    pub fn use_env(&self) -> bool {
        self.bit(BIT_USE_ENV)
    }
    pub(crate) fn set_use_env(&self, v: bool) {
        self.set_bit(BIT_USE_ENV, v)
    }
    /// `repo->is_bare`.
    pub fn is_bare(&self) -> bool {
        self.bit(BIT_IS_BARE)
    }
    pub(crate) fn set_is_bare(&self, v: bool) {
        self.set_bit(BIT_IS_BARE, v)
    }
    /// `repo->is_worktree`.
    pub fn is_worktree(&self) -> bool {
        self.bit(BIT_IS_WORKTREE)
    }
    pub(crate) fn set_is_worktree(&self, v: bool) {
        self.set_bit(BIT_IS_WORKTREE, v)
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

    /// `git_repository__configmap_lookup_cache_clear` (config_cache.c,
    /// PENDING(l-config)): every entry to `GIT_CONFIGMAP_NOT_CACHED`.
    pub(crate) fn configmap_cache_clear(&self) {
        // SAFETY: a live repository; C stores to its configmap cache
        // (atomics on this side).
        unsafe { c::git_repository__configmap_lookup_cache_clear(self.as_ptr()) }
    }

    /// `git_repository__configmap_lookup` (PENDING(l-config)).
    pub(crate) fn configmap_lookup(&self, item: c_int) -> Result<c_int> {
        let mut out: c_int = 0;
        // SAFETY: a live repository and a local out-parameter.
        check(unsafe { c::git_repository__configmap_lookup(&raw mut out, self.as_ptr(), item) })?;
        Ok(out)
    }

    // ----- the lazily created odb, config, refdb and index -----------------

    /// `set_odb`: own (and take a reference of) `odb`, releasing the old.
    ///
    /// The new object, when given, is a live `git_odb`: [`Odb::as_nonnull`]
    /// or a pointer C passed (the export's contract).
    pub(crate) fn set_odb(&self, odb: Option<NonNull<c_void>>) {
        self.set_owned(&self.odb, odb, |p| {
            // SAFETY: the reference the repository held.
            unsafe { c::git_odb_free(p) }
        });
    }
    /// `set_refdb` (as [`GitRepository::set_odb`]).
    pub(crate) fn set_refdb(&self, refdb: Option<NonNull<c_void>>) {
        self.set_owned(&self.refdb, refdb, |p| {
            // SAFETY: the reference the repository held.
            unsafe { c::git_refdb_free(p) }
        });
    }
    /// `set_config`: also clears the configmap cache.
    pub(crate) fn set_config(&self, config: Option<NonNull<c_void>>) {
        self.set_owned(&self.config, config, |p| {
            // SAFETY: the reference the repository held.
            unsafe { c::git_config_free(p) }
        });
        self.configmap_cache_clear();
    }
    /// `set_index`.
    pub(crate) fn set_index(&self, index: Option<NonNull<c_void>>) {
        self.set_owned(&self.index, index, |p| {
            // SAFETY: the reference the repository held.
            unsafe { c::git_index_free(p) }
        });
    }

    fn set_owned(
        &self,
        slot: &AtomicPtr<c_void>,
        new: Option<NonNull<c_void>>,
        free: impl FnOnce(*mut c_void),
    ) {
        if let Some(p) = new {
            // SAFETY: a live refcounted object (the caller's owner holds a
            // reference).
            let rc = unsafe { handles::refcount(p) };
            rc.own(self.as_ptr());
            rc.inc();
        }
        let old = thread::swap(slot, new.map_or(ptr::null_mut(), NonNull::as_ptr));
        if let Some(old) = NonNull::new(old) {
            // SAFETY: the object the repository owned and held a reference
            // of.
            unsafe { handles::refcount(old) }.own(ptr::null_mut());
            free(old.as_ptr());
        }
    }

    /// Publish a freshly made object in `slot` unless another thread was
    /// first (C's `git_atomic_compare_and_swap(&slot, NULL, obj)`): the
    /// loser is released. Returns the published object.
    fn publish(
        &self,
        slot: &AtomicPtr<c_void>,
        obj: NonNull<c_void>,
        free: impl FnOnce(*mut c_void),
    ) -> NonNull<c_void> {
        // SAFETY: `obj` is the live object the caller made.
        let rc = unsafe { handles::refcount(obj) };
        rc.own(self.as_ptr());
        let prev = thread::compare_and_swap(slot, ptr::null_mut(), obj.as_ptr());
        match NonNull::new(prev) {
            Some(prev) => {
                rc.own(ptr::null_mut());
                free(obj.as_ptr());
                prev
            }
            None => obj,
        }
    }

    /// `git_repository_config__weakptr`.
    pub fn config_weak(&self) -> Result<ConfigRef<'_>> {
        let mut p = load(&self.config);
        if p.is_null() {
            let use_env = self.use_env();
            let mut system = GitStr::new();
            let mut global = GitStr::new();
            let mut xdg = GitStr::new();
            open::config_path_system(&mut system, use_env)?;
            open::config_path_global(&mut global, use_env)?;
            // SAFETY: an exclusive local `git_str`.
            unsafe { c::git_config__find_xdg(&raw mut xdg) };
            if global.is_empty() {
                // SAFETY: as above.
                unsafe { c::git_config__global_location(&raw mut global) };
            }
            let cfg = open::load_config(
                Some(self),
                path_unless_empty(&global).as_deref(),
                path_unless_empty(&xdg).as_deref(),
                path_unless_empty(&system).as_deref(),
            )?;
            p = self
                .publish(&self.config, cfg.into_nonnull(), |p| {
                    // SAFETY: the loser's reference, released once.
                    unsafe { c::git_config_free(p) }
                })
                .as_ptr();
        }
        let p = NonNull::new(p).ok_or(ErrorCode::GENERIC)?;
        // SAFETY: the repository's config, live while it is set; replacing
        // it is the caller's concern, as for C's weak pointers.
        Ok(unsafe { ConfigRef::from_ptr(p) })
    }

    /// `git_repository_odb__weakptr`: the odb pointer (not referenced).
    pub fn odb_weak(&self) -> Result<NonNull<c_void>> {
        let p = load(&self.odb);
        if let Some(p) = NonNull::new(p) {
            return Ok(p);
        }
        let mut path = GitStr::new();
        open::repository_odb_path(&mut path, self)?;
        let odb = Odb::new(self.oid_type_raw_or_default())?;
        open::repository_odb_alternates(&odb, self)?;
        // SAFETY: the live odb we made.
        let rc = unsafe { handles::refcount(odb.as_nonnull()) };
        // Owned first: `GIT_ODB_CAP_FROM_OWNER` reads the owner's config.
        rc.own(self.as_ptr());
        let r = odb
            .set_caps(GIT_ODB_CAP_FROM_OWNER)
            .and_then(|()| odb.add_default_backends(&cs(&path)));
        if let Err(e) = r {
            // C frees the odb with its owner still set, which leaks it
            // (DEFECTS.md): released here.
            rc.own(ptr::null_mut());
            return Err(e);
        }
        Ok(self.publish(&self.odb, odb.into_nonnull(), |p| {
            // SAFETY: the loser's reference, released once.
            unsafe { c::git_odb_free(p) }
        }))
    }

    fn oid_type_raw_or_default(&self) -> OidType {
        OidType::from_raw(self.oid_type_raw()).unwrap_or(OidType::DEFAULT)
    }

    /// `git_repository_refdb__weakptr`.
    pub fn refdb_weak(&self) -> Result<NonNull<c_void>> {
        let mut p = load(&self.refdb);
        if p.is_null() {
            let mut out = ptr::null_mut();
            // SAFETY: a live repository and a local out-parameter.
            check(unsafe { c::git_refdb_open(&raw mut out, self.as_ptr()) })?;
            let out = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
            p = self
                .publish(&self.refdb, out, |p| {
                    // SAFETY: the loser's reference, released once.
                    unsafe { c::git_refdb_free(p) }
                })
                .as_ptr();
        }
        NonNull::new(p).ok_or(ErrorCode::GENERIC)
    }

    /// `git_repository_index__weakptr`.
    pub fn index_weak(&self) -> Result<NonNull<c_void>> {
        let mut p = load(&self.index);
        let mut error = Ok(());
        if p.is_null() {
            let mut path = GitStr::new();
            open::repository_index_path(&mut path, self)?;
            let index = Index::open(
                &crate::util::fs_path::str_cpath(&path),
                self.oid_type_raw_or_default(),
            )?;
            p = self
                .publish(&self.index, index.into_nonnull(), |p| {
                    // SAFETY: the loser's reference, released once.
                    unsafe { c::git_index_free(p) }
                })
                .as_ptr();
            // SAFETY: the published index.
            error = check(unsafe { c::git_index_set_caps(p, GIT_INDEX_CAPABILITY_FROM_OWNER) })
                .map(drop);
        }
        error?;
        NonNull::new(p).ok_or(ErrorCode::GENERIC)
    }

    /// The current `_odb`, `_refdb`, `_config` and `_index` pointers (what
    /// C's weak-pointer functions write out, also on failure).
    pub(crate) fn refdb_ptr(&self) -> *mut c_void {
        load(&self.refdb)
    }
    pub(crate) fn config_ptr(&self) -> *mut c_void {
        load(&self.config)
    }
    pub(crate) fn index_ptr(&self) -> *mut c_void {
        load(&self.index)
    }

    /// `repo->grafts`.
    pub(crate) fn grafts_ptr(&self) -> *mut c_void {
        load(&self.grafts)
    }
    /// `repo->shallow_grafts`.
    pub(crate) fn shallow_grafts_ptr(&self) -> *mut c_void {
        load(&self.shallow_grafts)
    }
    pub(crate) fn grafts_slot(&self) -> *mut *mut c_void {
        self.grafts.as_ptr()
    }
    pub(crate) fn shallow_grafts_slot(&self) -> *mut *mut c_void {
        self.shallow_grafts.as_ptr()
    }

    /// `git_repository_submodule_cache_clear`.
    pub(crate) fn submodule_cache_clear(&self) -> c_int {
        let cache = self.submodule_cache.load(Ordering::Acquire);
        // SAFETY: the repository's cache, no longer reachable from it after
        // the store below (as the C, which frees then clears).
        let r = unsafe { crate::sys::submodule::cache_free(cache) };
        self.submodule_cache
            .store(ptr::null_mut(), Ordering::Release);
        r
    }

    /// `git_repository_set_ident`.
    pub(crate) fn set_ident(&self, name: Option<&CStr>, email: Option<&CStr>) -> Result<()> {
        let n = match name {
            Some(n) => Some(alloc_cstr(n.to_bytes())?),
            None => None,
        };
        let e = match email {
            Some(e) => Some(alloc_cstr(e.to_bytes())?),
            None => None,
        };
        let old_n = self.ident_name.swap(
            n.map_or(ptr::null_mut(), GitCStr::into_raw),
            Ordering::AcqRel,
        );
        let old_e = self.ident_email.swap(
            e.map_or(ptr::null_mut(), GitCStr::into_raw),
            Ordering::AcqRel,
        );
        // SAFETY: the old strings, owned by the repository (C swaps and
        // frees them the same way).
        unsafe { alloc::free(old_n.cast()) };
        // SAFETY: as above.
        unsafe { alloc::free(old_e.cast()) };
        Ok(())
    }
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
    dirs: &ItemDirs<'a>,
    item: ItemT,
    fallback: ItemT,
) -> Result<Option<&'a [u8]>> {
    let parent = match item {
        ITEM_GITDIR => dirs.gitdir,
        ITEM_WORKDIR => dirs.workdir,
        ITEM_COMMONDIR => dirs.commondir,
        _ => {
            return Err(git_error_set!(
                ErrorClass::INVALID,
                "invalid item directory"
            ));
        }
    };
    if parent.is_none() && fallback != ITEM_LAST {
        return resolved_parent_path(dirs, fallback, ITEM_LAST);
    }
    Ok(parent)
}

/// `git_repository__item_path`, over the directories `dirs`.
///
/// An `item` beyond the table is rejected with "invalid item directory"
/// (the C reads past its table: see DEFECTS.md).
pub fn item_path(out: &mut GitStr, dirs: &ItemDirs<'_>, item: ItemT) -> Result<()> {
    let Some(it) = ITEMS.get(item as usize) else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid item directory"
        ));
    };
    // C: a NULL parent (also after "invalid item directory") is reported
    // as "path cannot exist in repository".
    let parent = resolved_parent_path(dirs, it.parent, it.fallback)
        .ok()
        .flatten();
    let Some(parent) = parent else {
        let _ = git_error_set!(ErrorClass::INVALID, "path cannot exist in repository");
        return Err(ErrorCode::NOT_FOUND);
    };
    out.sets(parent)?;
    if let Some(name) = it.name {
        out.joinpath(parent, name)?;
    }
    if it.directory {
        crate::util::fs_path::to_dir(out)?;
    }
    Ok(())
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
        c::ConfigEntryRaw => "git_config_entry" { name, value, backend_type, origin_path, include_depth, level },
        c::OidTypeOptions => "git_odb_options" { version, oid_type },
        c::ObjectIdOptions => "git_object_id_options" { version, object_type, oid_type, filters },
    ],
    asserts: [
        "offsetof(git_odb, options) + offsetof(git_odb_options, oid_type) == 60",
        "offsetof(git_worktree, gitlink_path) == 16",
        "offsetof(git_annotated_commit, description) == 56",
        "sizeof(git_cache) == 13 * 8",
        "GIT_CONFIGMAP_CACHE_MAX == 15",
        "GIT_REPOSITORY_ITEM__LAST == 15",
        "GIT_CONFIGMAP_IGNORECASE == 3",
        "GIT_CONFIGMAP_ABBREV == 7",
        "GIT_ABBREV_FALSE == 64",
        "sizeof(git_index_options) == 8 && offsetof(git_index_options, oid_type) == 4",
        "offsetof(git_repository, reserved_names.ptr) == 208",
        "offsetof(git_repository, reserved_names.size) == 216",
        "offsetof(git_repository, reserved_names.asize) == 224",
        "offsetof(git_repository, oid_type) == 236",
    ],
}
