//! Port of `src/libgit2/refdb.c`: the reference database (`git_refdb`),
//! the dispatch to its backend (`git_refdb_backend`), and the iteration of
//! references (`git_reference_iterator`).
//!
//! Types (PORTING.md §4.1):
//!
//! - `git_refdb` is mirrored (kind 2, kept whole in `refdb.h`):
//!   `repository.c` takes and drops counts and sets the owner (`rc`),
//!   `refdb_reftable.c` reads `backend`. Rust allocates it (with
//!   `git__calloc`, as C). `rc` is the atomic `git_refcount`; `backend` an
//!   `AtomicPtr` (`git_refdb_set_backend` may replace it while other
//!   threads read it, which C leaves racy).
//! - `git_refdb_backend` and `git_reference_iterator` are public tables
//!   (kind 3, mirrored). The filesystem backend (`refdb_fs`) is a Rust
//!   table C calls into ([`ffibox::CTable`]); `refdb_reftable.c` and user
//!   backends are C tables the Rust calls through the slots.
//!
//! [`RefdbPtr`] is the safe face of a live `git_refdb *`: its methods call
//! the backend's slots and keep C's counting of `ref->db`.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::git_error_set;
use crate::oid::Oid;
use crate::reflog::GitReflog;
use crate::refs::deps::{RepoPtr, configmap, git_repository, git_signature};
use crate::refs::{
    self, GIT_REFERENCE_DIRECT, GIT_REFERENCE_SYMBOLIC, GitReference, RefBox, RefView,
};
use crate::util::alloc;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::thread::GitRefcount;
use crate::util::util::{prefixcmp, strcmp};
use ffibox::CShared;

/// `DEFAULT_NESTING_LEVEL`.
pub const DEFAULT_NESTING_LEVEL: c_int = 5;
/// `MAX_NESTING_LEVEL`.
pub const MAX_NESTING_LEVEL: c_int = 10;

/// `GIT_REFDB_BACKEND_VERSION`.
pub const GIT_REFDB_BACKEND_VERSION: c_uint = 1;
/// `GIT_REFDB_FILES`.
pub const GIT_REFDB_FILES: c_int = 1;
/// `GIT_REFDB_REFTABLE`.
pub const GIT_REFDB_REFTABLE: c_int = 2;
/// `GIT_REFDB_BACKEND_INIT_IS_WORKTREE`.
pub const GIT_REFDB_BACKEND_INIT_IS_WORKTREE: u32 = 1 << 0;
/// `GIT_REFDB_BACKEND_INIT_FORCE_HEAD`.
pub const GIT_REFDB_BACKEND_INIT_FORCE_HEAD: u32 = 1 << 1;

/// `GIT_LOGALLREFUPDATES_FALSE`.
const LOGALL_FALSE: c_int = 0;
/// `GIT_LOGALLREFUPDATES_TRUE`.
const LOGALL_TRUE: c_int = 1;
/// `GIT_LOGALLREFUPDATES_UNSET`.
const LOGALL_UNSET: c_int = 2;
/// `GIT_LOGALLREFUPDATES_ALWAYS`.
const LOGALL_ALWAYS: c_int = 3;

// ---------------------------------------------------------------------------
// The C types
// ---------------------------------------------------------------------------

/// `struct git_refdb`, mirrored.
#[repr(C)]
pub struct GitRefdb {
    /// The count and owner (`GIT_REFCOUNT_*`).
    pub rc: GitRefcount,
    /// The repository (set at creation).
    pub repo: *mut git_repository,
    /// The backend, owned (freed with its `free` slot).
    pub backend: AtomicPtr<git_refdb_backend>,
}

/// `CCounted::c_release`: [`free_db`].
///
/// # Safety
///
/// As [`free_db`].
unsafe fn release_db(db: NonNull<GitRefdb>) {
    // SAFETY: the caller's contract.
    unsafe { free_db(db.as_ptr()) }
}

crate::impl_ccounted!(GitRefdb, crate::util::alloc::GitAlloc, release = release_db);

// SAFETY: C writes only `rc` (GIT_REFCOUNT_*) and `backend` (atomic) while
// the database is shared; `repo` is fixed once made.
unsafe impl ffibox::CShareable for GitRefdb {}

/// `git_refdb_backend.init`.
pub type InitFn =
    unsafe extern "C" fn(*mut git_refdb_backend, *const c_char, libc::mode_t, u32) -> c_int;
/// `git_refdb_backend.exists`.
pub type ExistsFn =
    unsafe extern "C" fn(*mut c_int, *mut git_refdb_backend, *const c_char) -> c_int;
/// `git_refdb_backend.lookup`.
pub type LookupFn =
    unsafe extern "C" fn(*mut *mut GitReference, *mut git_refdb_backend, *const c_char) -> c_int;
/// `git_refdb_backend.iterator`.
pub type IteratorFn = unsafe extern "C" fn(
    *mut *mut git_reference_iterator,
    *mut git_refdb_backend,
    *const c_char,
) -> c_int;
/// `git_refdb_backend.write`.
pub type WriteFn = unsafe extern "C" fn(
    *mut git_refdb_backend,
    *const GitReference,
    c_int,
    *const git_signature,
    *const c_char,
    *const Oid,
    *const c_char,
) -> c_int;
/// `git_refdb_backend.rename`.
pub type RenameFn = unsafe extern "C" fn(
    *mut *mut GitReference,
    *mut git_refdb_backend,
    *const c_char,
    *const c_char,
    c_int,
    *const git_signature,
    *const c_char,
) -> c_int;
/// `git_refdb_backend.del`.
pub type DelFn =
    unsafe extern "C" fn(*mut git_refdb_backend, *const c_char, *const Oid, *const c_char) -> c_int;
/// `git_refdb_backend.compress`.
pub type CompressFn = unsafe extern "C" fn(*mut git_refdb_backend) -> c_int;
/// `git_refdb_backend.has_log` / `ensure_log` / `reflog_delete`.
pub type NameFn = unsafe extern "C" fn(*mut git_refdb_backend, *const c_char) -> c_int;
/// `git_refdb_backend.free`.
pub type FreeFn = unsafe extern "C" fn(*mut git_refdb_backend);
/// `git_refdb_backend.reflog_read`.
pub type ReflogReadFn =
    unsafe extern "C" fn(*mut *mut GitReflog, *mut git_refdb_backend, *const c_char) -> c_int;
/// `git_refdb_backend.reflog_write`.
pub type ReflogWriteFn = unsafe extern "C" fn(*mut git_refdb_backend, *mut GitReflog) -> c_int;
/// `git_refdb_backend.reflog_rename`.
pub type ReflogRenameFn =
    unsafe extern "C" fn(*mut git_refdb_backend, *const c_char, *const c_char) -> c_int;
/// `git_refdb_backend.lock`.
pub type LockFn =
    unsafe extern "C" fn(*mut *mut c_void, *mut git_refdb_backend, *const c_char) -> c_int;
/// `git_refdb_backend.unlock`.
pub type UnlockFn = unsafe extern "C" fn(
    *mut git_refdb_backend,
    *mut c_void,
    c_int,
    c_int,
    *const GitReference,
    *const git_signature,
    *const c_char,
) -> c_int;

/// `struct git_refdb_backend` (include/git2/sys/refdb_backend.h), mirrored.
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Default)]
pub struct git_refdb_backend {
    /// `version`.
    pub version: c_uint,
    /// `init`.
    pub init: Option<InitFn>,
    /// `exists`.
    pub exists: Option<ExistsFn>,
    /// `lookup`.
    pub lookup: Option<LookupFn>,
    /// `iterator`.
    pub iterator: Option<IteratorFn>,
    /// `write`.
    pub write: Option<WriteFn>,
    /// `rename`.
    pub rename: Option<RenameFn>,
    /// `del`.
    pub del: Option<DelFn>,
    /// `compress`.
    pub compress: Option<CompressFn>,
    /// `has_log`.
    pub has_log: Option<NameFn>,
    /// `ensure_log`.
    pub ensure_log: Option<NameFn>,
    /// `free`.
    pub free: Option<FreeFn>,
    /// `reflog_read`.
    pub reflog_read: Option<ReflogReadFn>,
    /// `reflog_write`.
    pub reflog_write: Option<ReflogWriteFn>,
    /// `reflog_rename`.
    pub reflog_rename: Option<ReflogRenameFn>,
    /// `reflog_delete`.
    pub reflog_delete: Option<NameFn>,
    /// `lock`.
    pub lock: Option<LockFn>,
    /// `unlock`.
    pub unlock: Option<UnlockFn>,
}

/// `git_reference_iterator.next`.
pub type IterNextFn =
    unsafe extern "C" fn(*mut *mut GitReference, *mut git_reference_iterator) -> c_int;
/// `git_reference_iterator.next_name`.
pub type IterNextNameFn =
    unsafe extern "C" fn(*mut *const c_char, *mut git_reference_iterator) -> c_int;
/// `git_reference_iterator.free`.
pub type IterFreeFn = unsafe extern "C" fn(*mut git_reference_iterator);

/// `struct git_reference_iterator` (include/git2/sys/refdb_backend.h),
/// mirrored.
#[allow(non_camel_case_types)]
#[repr(C)]
pub struct git_reference_iterator {
    /// The database, with a count the iterator holds (set by
    /// `git_refdb_iterator`).
    pub db: *mut GitRefdb,
    /// `next`.
    pub next: Option<IterNextFn>,
    /// `next_name`.
    pub next_name: Option<IterNextNameFn>,
    /// `free`.
    pub free: Option<IterFreeFn>,
}

impl Default for git_reference_iterator {
    fn default() -> Self {
        git_reference_iterator {
            db: core::ptr::null_mut(),
            next: None,
            next_name: None,
            free: None,
        }
    }
}

ffibox::define_ctype!(
    /// The layout type of `git_refdb_backend`, for [`ffibox::CTable`].
    RefdbBackendTable,
    RefdbBackendTableRef,
    RefdbBackendTableMut,
    git_refdb_backend
);
ffibox::define_ctype!(
    /// The layout type of `git_reference_iterator`, for [`ffibox::CTable`].
    RefIteratorTable,
    RefIteratorTableRef,
    RefIteratorTableMut,
    git_reference_iterator
);

// ---------------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------------

/// `git_refdb__free`: free the backend (its `free` slot) and the database.
///
/// # Safety
///
/// `db` is a live database whose last count is being dropped: nothing
/// uses it again.
pub unsafe fn free_db(db: *mut GitRefdb) {
    // SAFETY: per the contract: a live database (its fields are atomics).
    let backend = unsafe { (*db).backend.load(Ordering::Acquire) };
    // SAFETY: the database owned its backend; nothing reaches it now.
    unsafe { free_backend(backend) };
    // SAFETY: our block, which nothing uses any more.
    unsafe { core::ptr::write_bytes(db.cast::<u8>(), 0, core::mem::size_of::<GitRefdb>()) };
    // SAFETY: as above: from `git__calloc`, freed once.
    unsafe { alloc::free(db.cast()) };
}

/// Free `backend` through its `free` slot (NULL: nothing).
///
/// # Safety
///
/// `backend` is NULL or a live backend the caller owns and gives up.
unsafe fn free_backend(backend: *mut git_refdb_backend) {
    if backend.is_null() {
        return;
    }
    // SAFETY: a live backend; `free` is required (checked by
    // `git_refdb_set_backend`, set by the built-in backends).
    let free = unsafe { (*backend).free };
    if let Some(f) = free {
        // SAFETY: the backend's own `free` slot, called once.
        unsafe { f(backend) };
    }
}

/// `GIT_REFCOUNT_DEC(db, git_refdb__free)`.
///
/// # Safety
///
/// `db` is a live database on which the caller holds a count it drops.
pub unsafe fn decref(db: *mut GitRefdb) {
    // SAFETY: per the contract.
    unsafe { CShared::decrement_count(db) }
}

// ---------------------------------------------------------------------------
// The database, safely
// ---------------------------------------------------------------------------

/// A live `git_refdb`, not counted by this value. Every method keeps C's
/// contract with the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefdbPtr(NonNull<GitRefdb>);

// SAFETY: the database's counts and backend pointer are atomics, and its
// backends are used from several threads (`tests/libgit2/threads`); the
// contract of `RefdbPtr::new` covers each use.
unsafe impl Send for RefdbPtr {}
// SAFETY: as above.
unsafe impl Sync for RefdbPtr {}

fn c(name: &CStr) -> *const c_char {
    name.as_ptr()
}

fn opt_c(name: Option<&CStr>) -> *const c_char {
    name.map_or(core::ptr::null(), CStr::as_ptr)
}

impl RefdbPtr {
    /// The database at `db`.
    ///
    /// # Safety
    ///
    /// `db` is a live database whose backend (if set) is live, both
    /// staying live while this value (or a copy) is used, on the threads
    /// that use it.
    pub unsafe fn new(db: NonNull<GitRefdb>) -> RefdbPtr {
        RefdbPtr(db)
    }

    /// The database at `db`, which C dereferences unchecked (the barrier
    /// turns the panic of a NULL into an error).
    ///
    /// # Safety
    ///
    /// As [`new`](Self::new), for a non-NULL `db`.
    pub unsafe fn from_raw(db: *mut GitRefdb) -> RefdbPtr {
        RefdbPtr(NonNull::new(db).expect("NULL refdb"))
    }

    /// The database a repository holds (`git_repository_refdb__weakptr`).
    pub fn of_repo(repo: RepoPtr) -> Result<RefdbPtr> {
        let db = repo.refdb_weak()?;
        let db = NonNull::new(db.cast::<GitRefdb>()).ok_or(ErrorCode::GENERIC)?;
        // SAFETY: the repository holds the database while it lives, which
        // the contract of `repo` covers.
        Ok(unsafe { RefdbPtr::new(db) })
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut GitRefdb {
        self.0.as_ptr()
    }

    fn db(&self) -> &GitRefdb {
        // SAFETY: a live database (the constructor's contract); its
        // mutable fields are atomics.
        unsafe { self.0.as_ref() }
    }

    /// `db->repo`.
    pub fn repo(self) -> Option<RepoPtr> {
        // SAFETY: the database's repository outlives it (C's contract).
        NonNull::new(self.db().repo).map(|r| unsafe { RepoPtr::new(r) })
    }

    /// `db->backend`.
    pub fn backend(self) -> *mut git_refdb_backend {
        self.db().backend.load(Ordering::Acquire)
    }

    /// The backend, or the `GIT_ASSERT_ARG(db->backend)` error.
    fn backend_checked(self) -> Result<*mut git_refdb_backend> {
        let b = self.backend();
        crate::git_assert_arg!(!b.is_null(), "db->backend");
        Ok(b)
    }

    /// `GIT_REFCOUNT_INC(db)`.
    pub fn incref(self) {
        // SAFETY: a live database (the constructor's contract), which a
        // count someone holds keeps live.
        unsafe { CShared::increment_count(self.as_ptr()) }
    }

    /// The backend (one load of `db->backend`) and its slot `f`: the call
    /// passes that same backend, so a concurrent `git_refdb_set_backend`
    /// cannot pair one table's slot with another table (C loads the
    /// pointer twice).
    fn slot<T: Copy>(
        self,
        f: impl FnOnce(&git_refdb_backend) -> Option<T>,
    ) -> Result<(*mut git_refdb_backend, T)> {
        let b = self.backend_checked()?;
        // SAFETY: a live backend (the constructor's contract), whose slots
        // C does not change while it is in use.
        let s = f(unsafe { &*b });
        // C calls the slot unconditionally: a NULL one crashes there.
        Ok((b, s.expect("refdb backend slot is NULL")))
    }

    /// `git_refdb_init`.
    pub fn init(self, head_target: Option<&CStr>, mode: libc::mode_t, flags: u32) -> Result<()> {
        let b = self.backend_checked()?;
        // SAFETY: as in `slot`.
        let Some(init) = (unsafe { (*b).init }) else {
            return Ok(());
        };
        // SAFETY: the backend's own slot with its table and C's arguments.
        ErrorCode::check(unsafe { init(b, opt_c(head_target), mode, flags) }).map(drop)
    }

    /// `git_refdb_exists`.
    pub fn exists(self, name: &CStr) -> Result<bool> {
        let (b, f) = self.slot(|b| b.exists)?;
        let mut exists = 0;
        // SAFETY: as in `init`; `exists` writable.
        ErrorCode::check(unsafe { f(&raw mut exists, b, c(name)) })?;
        Ok(exists != 0)
    }

    /// `git_refdb_lookup`: the reference, with `ref->db` this database.
    pub fn lookup(self, name: &CStr) -> Result<RefBox> {
        let (b, f) = self.slot(|b| b.lookup)?;
        let mut out = core::ptr::null_mut();
        // SAFETY: as in `init`; `out` writable.
        let r = unsafe { f(&raw mut out, b, c(name)) };
        // SAFETY: the backend hands over the reference it made, if any.
        let r_ref = unsafe { RefBox::from_raw(out) };
        ErrorCode::check(r)?;
        let mut r_ref = r_ref.ok_or_else(|| {
            // C dereferences the NULL reference.
            git_error_set!(ErrorClass::REFERENCE, "refdb backend returned no reference")
        })?;
        // SAFETY: a live database.
        unsafe { r_ref.attach_db(self.as_ptr()) };
        Ok(r_ref)
    }

    /// `git_refdb_resolve`: follow symbolic references up to
    /// `max_nesting` deep (default 5, at most 10); a dangling symbolic
    /// reference is returned as it is.
    pub fn resolve(self, name: &CStr, max_nesting: c_int) -> Result<RefBox> {
        let max_nesting = if max_nesting > MAX_NESTING_LEVEL {
            MAX_NESTING_LEVEL
        } else if max_nesting < 0 {
            DEFAULT_NESTING_LEVEL
        } else {
            max_nesting
        };
        let mut r = self.lookup(name)?;
        for _ in 0..max_nesting {
            if r.view().raw_kind() == GIT_REFERENCE_DIRECT {
                break;
            }
            let target = r.view().symbolic_target_cstr().unwrap_or(c"").to_owned();
            match self.lookup(&target) {
                Ok(resolved) => r = resolved,
                Err(ErrorCode::NOT_FOUND) => return Ok(r),
                Err(e) => return Err(e),
            }
        }
        if r.view().raw_kind() != GIT_REFERENCE_DIRECT && max_nesting != 0 {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "cannot resolve reference (>{} levels deep)",
                max_nesting
            ));
        }
        Ok(r)
    }

    /// `git_refdb_iterator`: a backend iterator holding a count on this
    /// database.
    pub fn iterator(self, glob: Option<&CStr>) -> Result<NonNull<git_reference_iterator>> {
        let b = self.backend();
        // SAFETY: as in `slot`.
        let f = unsafe { b.as_ref() }.and_then(|t| t.iterator);
        let Some(f) = f else {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "this backend doesn't support iterators"
            ));
        };
        let mut out = core::ptr::null_mut();
        // SAFETY: as in `init`; `out` writable.
        ErrorCode::check(unsafe { f(&raw mut out, b, opt_c(glob)) })?;
        let it = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
        self.incref();
        // SAFETY: the backend's fresh iterator, ours.
        unsafe { (*it.as_ptr()).db = self.as_ptr() };
        Ok(it)
    }

    /// `git_refdb_write`: `ref->db` becomes this database (counted), then
    /// the backend writes.
    ///
    /// # Safety
    ///
    /// `r` is a live reference the caller owns, and `who` NULL or a valid
    /// signature, for the call.
    pub unsafe fn write(
        self,
        r: *mut GitReference,
        force: bool,
        who: *const git_signature,
        message: Option<&CStr>,
        old_id: Option<&Oid>,
        old_target: Option<&CStr>,
    ) -> Result<()> {
        let (b, f) = self.slot(|b| b.write)?;
        self.incref();
        // SAFETY: per the contract.
        unsafe { (*r).db = self.as_ptr() };
        // SAFETY: as in `init`, with the caller's pointers.
        ErrorCode::check(unsafe {
            f(
                b,
                r,
                c_int::from(force),
                who,
                opt_c(message),
                old_id.map_or(core::ptr::null(), |o| o as *const Oid),
                opt_c(old_target),
            )
        })
        .map(drop)
    }

    /// `git_refdb_rename`.
    pub(crate) fn rename(
        self,
        old_name: &CStr,
        new_name: &CStr,
        force: bool,
        who: *const git_signature,
        message: Option<&CStr>,
        want: bool,
    ) -> Result<Option<RefBox>> {
        let (b, f) = self.slot(|b| b.rename)?;
        let mut out = core::ptr::null_mut();
        let outp = if want {
            &raw mut out
        } else {
            core::ptr::null_mut()
        };
        // SAFETY: as in `init`; `who` is NULL or valid (callers' contract).
        let r = unsafe {
            f(
                outp,
                b,
                c(old_name),
                c(new_name),
                c_int::from(force),
                who,
                opt_c(message),
            )
        };
        // SAFETY: what the backend handed over, if anything.
        let made = unsafe { RefBox::from_raw(out) };
        ErrorCode::check(r)?;
        match made {
            Some(mut m) => {
                // SAFETY: a live database.
                unsafe { m.attach_db(self.as_ptr()) };
                Ok(Some(m))
            }
            None => Ok(None),
        }
    }

    /// `git_refdb_delete`.
    pub fn delete(
        self,
        name: &CStr,
        old_id: Option<&Oid>,
        old_target: Option<&CStr>,
    ) -> Result<()> {
        let (b, f) = self.slot(|b| b.del)?;
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe {
            f(
                b,
                c(name),
                old_id.map_or(core::ptr::null(), |o| o as *const Oid),
                opt_c(old_target),
            )
        })
        .map(drop)
    }

    /// `git_refdb_compress`.
    pub fn compress(self) -> Result<()> {
        let b = self.backend();
        // SAFETY: as in `slot` (C dereferences a NULL backend).
        let Some(f) = (unsafe { b.as_ref() }).and_then(|b| b.compress) else {
            return Ok(());
        };
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe { f(b) }).map(drop)
    }

    /// `git_refdb_reflog_read`: the reflog, with a count on this database.
    pub fn reflog_read(self, name: &CStr) -> Result<NonNull<GitReflog>> {
        let (b, f) = self.slot(|b| b.reflog_read)?;
        let mut out = core::ptr::null_mut();
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe { f(&raw mut out, b, c(name)) })?;
        let log = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
        self.incref();
        // SAFETY: the backend's fresh reflog, ours.
        unsafe { (*log.as_ptr()).db = self.as_ptr() };
        Ok(log)
    }

    /// `git_reflog_write` through this database's backend.
    ///
    /// # Safety
    ///
    /// `log` is a live reflog, not used elsewhere during the call.
    pub unsafe fn reflog_write(self, log: *mut GitReflog) -> Result<()> {
        let (b, f) = self.slot(|b| b.reflog_write)?;
        // SAFETY: as in `init`, with the caller's reflog.
        ErrorCode::check(unsafe { f(b, log) }).map(drop)
    }

    /// `backend->reflog_rename`.
    pub fn reflog_rename(self, old_name: &CStr, new_name: &CStr) -> Result<()> {
        let (b, f) = self.slot(|b| b.reflog_rename)?;
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe { f(b, c(old_name), c(new_name)) }).map(drop)
    }

    /// `backend->reflog_delete`.
    pub fn reflog_delete(self, name: &CStr) -> Result<()> {
        let (b, f) = self.slot(|b| b.reflog_delete)?;
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe { f(b, c(name)) }).map(drop)
    }

    /// `git_refdb_has_log`: the backend's answer (1, 0, or an error).
    pub fn has_log(self, name: &CStr) -> Result<bool> {
        let (b, f) = self.slot(|b| b.has_log)?;
        // SAFETY: as in `init`.
        Ok(ErrorCode::check(unsafe { f(b, c(name)) })? != 0)
    }

    /// `git_refdb_has_log` as C returns it.
    pub fn has_log_raw(self, name: &CStr) -> Result<c_int> {
        let (b, f) = self.slot(|b| b.has_log)?;
        // SAFETY: as in `init`.
        Ok(unsafe { f(b, c(name)) })
    }

    /// `git_refdb_ensure_log`.
    pub fn ensure_log(self, name: &CStr) -> Result<c_int> {
        let (b, f) = self.slot(|b| b.ensure_log)?;
        // SAFETY: as in `init`.
        Ok(unsafe { f(b, c(name)) })
    }

    /// `git_refdb_lock`.
    pub fn lock(self, name: &CStr) -> Result<*mut c_void> {
        let b = self.backend();
        // SAFETY: as in `slot`.
        let Some(f) = (unsafe { b.as_ref() }).and_then(|b| b.lock) else {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "backend does not support locking"
            ));
        };
        let mut payload = core::ptr::null_mut();
        // SAFETY: as in `init`.
        ErrorCode::check(unsafe { f(&raw mut payload, b, c(name)) })?;
        Ok(payload)
    }

    /// `git_refdb_should_write_reflog` (`core.logAllRefUpdates`).
    pub fn should_write_reflog(self, r: RefView<'_>) -> Result<bool> {
        let repo = self.repo().ok_or(ErrorCode::GENERIC)?;
        let mut logall = repo.configmap(configmap::LOGALLREFUPDATES)?;
        if logall == LOGALL_UNSET {
            logall = c_int::from(!repo.is_bare());
        }
        Ok(match logall {
            LOGALL_FALSE => false,
            LOGALL_TRUE => {
                let name = r.name();
                // C ORs `git_refdb_has_log`'s int: an error (< 0) is true.
                !matches!(self.has_log_raw(r.name_cstr()), Ok(0))
                    || prefixcmp(name, refs::GIT_REFS_HEADS_DIR) == 0
                    || strcmp(name, refs::GIT_HEAD_REF) == 0
                    || prefixcmp(name, refs::GIT_REFS_REMOTES_DIR) == 0
                    || prefixcmp(name, refs::GIT_REFS_NOTES_DIR) == 0
            }
            LOGALL_ALWAYS => true,
            _ => false,
        })
    }

    /// `git_refdb_should_write_head_reflog`: whether `r` is what HEAD
    /// (symbolically) points to.
    pub fn should_write_head_reflog(self, r: RefView<'_>) -> Result<bool> {
        if r.raw_kind() == GIT_REFERENCE_SYMBOLIC {
            return Ok(false);
        }
        let head = self.lookup(c"HEAD")?;
        if head.view().raw_kind() == GIT_REFERENCE_DIRECT {
            return Ok(false);
        }
        let head_target = head.view().symbolic_target_cstr().unwrap_or(c"").to_owned();
        let name: Vec<u8> = match self.resolve(&head_target, -1) {
            Err(ErrorCode::NOT_FOUND) => head_target.to_bytes().to_vec(),
            Err(e) => return Err(e),
            Ok(resolved) => {
                if resolved.view().raw_kind() == GIT_REFERENCE_SYMBOLIC {
                    resolved
                        .view()
                        .symbolic_target()
                        .unwrap_or_default()
                        .to_vec()
                } else {
                    resolved.view().name().to_vec()
                }
            }
        };
        Ok(strcmp(&name, r.name()) == 0)
    }
}

/// `git_refdb_new`: a database of `repo`, without a backend, with one count.
pub fn new_db(repo: *mut git_repository) -> Result<CShared<GitRefdb>> {
    CShared::try_new(GitRefdb {
        rc: GitRefcount::one(),
        repo,
        backend: AtomicPtr::default(),
    })
    .map_err(|_| errors::oom())
}

/// `git_refdb_open`: a database of `repo` with the backend of its
/// reference storage format.
pub fn open(repo: RepoPtr) -> Result<CShared<GitRefdb>> {
    let db = new_db(repo.as_ptr())?;
    let backend = match repo.refdb_type() {
        GIT_REFDB_FILES => crate::refdb_fs::backend_fs(repo),
        GIT_REFDB_REFTABLE => reftable_backend(repo),
        _ => {
            let _ = git_error_set!(ErrorClass::REFERENCE, "unknown reference storage format");
            Err(ErrorCode::INVALID)
        }
    };
    match backend {
        Ok(b) => {
            db.backend.store(b, Ordering::Release);
            Ok(db)
        }
        Err(e) => Err(e),
    }
}

// The repository is a Rust type C sees as opaque.
#[allow(improper_ctypes)]
unsafe extern "C" {
    /// `git_refdb_backend_reftable` (refdb_reftable.c, C that stays C).
    fn git_refdb_backend_reftable(
        out: *mut *mut git_refdb_backend,
        repo: *mut git_repository,
    ) -> c_int;
}

/// `git_refdb_backend_reftable`.
fn reftable_backend(repo: RepoPtr) -> Result<*mut git_refdb_backend> {
    let mut out = core::ptr::null_mut();
    // SAFETY: a live repository; `out` writable.
    ErrorCode::check(unsafe { git_refdb_backend_reftable(&raw mut out, repo.as_ptr()) })?;
    Ok(out)
}

/// `git_refdb_set_backend`: replace (and free) the backend, after checking
/// the version and the required slots.
///
/// # Safety
///
/// `db` is a live database; `backend` NULL or a live backend whose
/// ownership passes to the database on success.
pub unsafe fn set_backend(db: RefdbPtr, backend: *mut git_refdb_backend) -> Result<()> {
    // SAFETY: per the contract.
    let b = unsafe { backend.as_ref() };
    if let Some(b) = b
        && (b.version == 0 || b.version > GIT_REFDB_BACKEND_VERSION)
    {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid version {} on git_refdb_backend",
            b.version
        ));
    }
    let complete = b.is_some_and(|b| {
        b.exists.is_some()
            && b.lookup.is_some()
            && b.iterator.is_some()
            && b.write.is_some()
            && b.rename.is_some()
            && b.del.is_some()
            && b.has_log.is_some()
            && b.ensure_log.is_some()
            && b.free.is_some()
            && b.reflog_read.is_some()
            && b.reflog_write.is_some()
            && b.reflog_rename.is_some()
            && b.reflog_delete.is_some()
            && !(b.lock.is_some() && b.unlock.is_none())
    });
    if !complete {
        let _ = git_error_set!(
            ErrorClass::REFERENCE,
            "incomplete refdb backend implementation"
        );
        return Err(ErrorCode::INVALID);
    }
    let old = db.db().backend.swap(backend, Ordering::AcqRel);
    // SAFETY: the database owned the old backend; nothing reaches it now.
    unsafe { free_backend(old) };
    Ok(())
}

/// The bytes of a `git_reference *`'s name, for messages.
pub fn name_of(r: RefView<'_>) -> Bytes<'_> {
    Bytes(r.name())
}

/// `git_refdb_iterator_next`: the next reference, with `ref->db` the
/// iterator's database (counted).
///
/// # Safety
///
/// `iter` is a live iterator from `git_refdb_iterator`, used by nobody
/// else during the call.
pub unsafe fn iterator_next(iter: *mut git_reference_iterator) -> Result<RefBox> {
    let mut out = core::ptr::null_mut();
    // per the contract; `next` is set by every backend.
    let r = {
        // SAFETY: a live table, whose slots C does not change while it is used.
        let f = unsafe { (*iter).next }.expect("iterator next is NULL");
        // SAFETY: the slot of that table, with its own table and C's arguments.
        unsafe { f(&raw mut out, iter) }
    };
    // SAFETY: what the backend handed over, if anything.
    let made = unsafe { RefBox::from_raw(out) };
    ErrorCode::check(r)?;
    let mut made = made.ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a live iterator.
    let db = unsafe { (*iter).db };
    // SAFETY: the iterator's database is live (it holds a count).
    unsafe { made.attach_db(db) };
    Ok(made)
}

/// `git_refdb_iterator_next_name`: the raw status and name.
///
/// # Safety
///
/// As [`iterator_next`]; `out` writable.
pub unsafe fn iterator_next_name(
    out: *mut *const c_char,
    iter: *mut git_reference_iterator,
) -> c_int {
    // per the contract.
    {
        // SAFETY: a live table, whose slots C does not change while it is used.
        let f = unsafe { (*iter).next_name }.expect("iterator next_name is NULL");
        // SAFETY: the slot of that table, with its own table and C's arguments.
        unsafe { f(out, iter) }
    }
}

/// `git_refdb_iterator_free`: drop the iterator's count on its database,
/// then free it.
///
/// # Safety
///
/// `iter` is a live iterator from `git_refdb_iterator`, given up.
pub unsafe fn iterator_free(iter: *mut git_reference_iterator) {
    // SAFETY: per the contract: a live iterator.
    let db = unsafe { (*iter).db };
    // SAFETY: as above.
    let free = unsafe { (*iter).free };
    if !db.is_null() {
        // SAFETY: the iterator holds a count on its live database.
        unsafe { decref(db) };
    }
    if let Some(f) = free {
        // SAFETY: the iterator's own `free` slot, called once.
        unsafe { f(iter) };
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `refdb.c`.
///
/// Every `git_refdb *` is NULL (where C checks it) or a live database
/// (from `git_refdb_new` / `git_refdb_open`, or a repository's), whose
/// backend is live; libgit2's threading rules hold for it.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr, status};

    /// The database behind `db`, checked as `GIT_ASSERT_ARG(db)`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module).
    unsafe fn dbp(db: *mut GitRefdb) -> Result<RefdbPtr> {
        crate::git_assert_arg!(!db.is_null(), "db");
        // SAFETY: per the contract.
        Ok(unsafe { RefdbPtr::from_raw(db) })
    }

    /// `git_refdb_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_new(
        out: *mut *mut GitRefdb,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            match new_db(repo) {
                Ok(db) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = CShared::into_raw(db) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_open`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_new`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_open(
        out: *mut *mut GitRefdb,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            // SAFETY: `out` writable.
            unsafe { *out = core::ptr::null_mut() };
            // SAFETY: a live repository (the caller's contract).
            let repo = unsafe { RepoPtr::from_raw(repo) };
            match open(repo) {
                Ok(db) => {
                    // SAFETY: as above.
                    unsafe { *out = CShared::into_raw(db) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_set_backend`.
    ///
    /// # Safety
    ///
    /// `db` is a live database; `backend` NULL or a live backend whose
    /// ownership passes to `db` on success.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_set_backend(
        db: *mut GitRefdb,
        backend: *mut git_refdb_backend,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract (C dereferences a NULL `db`).
            let r = unsafe { dbp(db) }.and_then(|d| unsafe { set_backend(d, backend) });
            status(r)
        })
    }

    /// `git_refdb_compress`.
    ///
    /// # Safety
    ///
    /// See the module.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_compress(db: *mut GitRefdb) -> c_int {
        // SAFETY: per the contract.
        barrier(|| status(unsafe { dbp(db) }.and_then(RefdbPtr::compress)))
    }

    /// `git_refdb__free`.
    ///
    /// # Safety
    ///
    /// `db` is a live database whose last count is dropped.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb__free(db: *mut GitRefdb) {
        // SAFETY: per the contract.
        barrier(|| unsafe { free_db(db) })
    }

    /// `git_refdb_free`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database on which the caller drops a count.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_free(db: *mut GitRefdb) {
        // SAFETY: per the contract.
        barrier(|| unsafe { CShared::release(db) })
    }

    /// `git_refdb_init`.
    ///
    /// # Safety
    ///
    /// See the module; `head_target` NULL or a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_init(
        db: *mut GitRefdb,
        head_target: *const c_char,
        mode: libc::mode_t,
        flags: u32,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "refdb", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "refdb->backend", -1);
            // SAFETY: per the contract.
            status(d.init(unsafe { cstr(head_target) }, mode, flags))
        })
    }

    /// `git_refdb_exists`.
    ///
    /// # Safety
    ///
    /// See the module; `exists` writable; `ref_name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_exists(
        exists: *mut c_int,
        db: *mut GitRefdb,
        ref_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!exists.is_null(), "exists", -1);
            crate::git_assert_arg!(!db.is_null(), "refdb", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "refdb->backend", -1);
            let b = d.backend();
            // a live backend and the caller's pointers, passed on
            // as C does.
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).exists }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(exists, b, ref_name) }
            }
        })
    }

    /// `git_refdb_lookup`.
    ///
    /// # Safety
    ///
    /// See the module; `out` writable; `ref_name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_lookup(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        ref_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            crate::git_assert_arg!(!ref_name.is_null(), "ref_name", -1);
            // SAFETY: a string (the contract).
            let name = unsafe { CStr::from_ptr(ref_name) };
            match d.lookup(name) {
                Ok(r) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = r.into_raw() };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_resolve`.
    ///
    /// # Safety
    ///
    /// See the module; `out` writable; `ref_name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_resolve(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        ref_name: *const c_char,
        max_nesting: c_int,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `out` writable.
            unsafe { *out = core::ptr::null_mut() };
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            crate::git_assert_arg!(!ref_name.is_null(), "ref_name", -1);
            // SAFETY: a string.
            let name = unsafe { CStr::from_ptr(ref_name) };
            match d.resolve(name, max_nesting) {
                Ok(r) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = r.into_raw() };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_iterator`.
    ///
    /// # Safety
    ///
    /// See the module; `out` writable; `glob` NULL or a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator(
        out: *mut *mut git_reference_iterator,
        db: *mut GitRefdb,
        glob: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract (C dereferences a NULL `db`).
            let r = unsafe { dbp(db) }.and_then(|d| d.iterator(unsafe { cstr(glob) }));
            match r {
                Ok(it) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = it.as_ptr() };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_iterator_next`.
    ///
    /// # Safety
    ///
    /// `out` writable; `iter` a live iterator from `git_refdb_iterator`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_next(
        out: *mut *mut GitReference,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract.
            match unsafe { iterator_next(iter) } {
                Ok(r) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = r.into_raw() };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_iterator_next_name`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_iterator_next`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_next_name(
        out: *mut *const c_char,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        // SAFETY: per the contract.
        barrier(|| unsafe { iterator_next_name(out, iter) })
    }

    /// `git_refdb_iterator_free`.
    ///
    /// # Safety
    ///
    /// `iter` a live iterator from `git_refdb_iterator`, given up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_free(iter: *mut git_reference_iterator) {
        // SAFETY: per the contract.
        barrier(|| unsafe { iterator_free(iter) })
    }

    /// `git_refdb_write`.
    ///
    /// # Safety
    ///
    /// See the module; `ref` a live reference; the other pointers NULL or
    /// valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_write(
        db: *mut GitRefdb,
        r: *mut GitReference,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            d.incref();
            // SAFETY: a live reference (C dereferences NULL).
            unsafe { (*r).db = db };
            let b = d.backend();
            // the backend's slot, with C's arguments passed on.
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).write }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(b, r, force, who, message, old_id, old_target) }
            }
        })
    }

    /// `git_refdb_rename`.
    ///
    /// # Safety
    ///
    /// See the module; `out` NULL or writable; names strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_rename(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        old_name: *const c_char,
        new_name: *const c_char,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            let b = d.backend();
            // as in `git_refdb_write`.
            let r = {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).rename }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(out, b, old_name, new_name, force, who, message) }
            };
            if r < 0 {
                return r;
            }
            if !out.is_null() {
                // SAFETY: on success the backend set `*out`.
                let made = unsafe { *out };
                if !made.is_null() {
                    d.incref();
                    // SAFETY: the backend's fresh reference.
                    unsafe { (*made).db = db };
                }
            }
            0
        })
    }

    /// `git_refdb_delete`.
    ///
    /// # Safety
    ///
    /// See the module.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_delete(
        db: *mut GitRefdb,
        ref_name: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            let b = d.backend();
            // as in `git_refdb_write`.
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).del }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(b, ref_name, old_id, old_target) }
            }
        })
    }

    /// `git_refdb_reflog_read`.
    ///
    /// # Safety
    ///
    /// See the module; `out` writable; `name` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_reflog_read(
        out: *mut *mut GitReflog,
        db: *mut GitRefdb,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            let b = d.backend();
            // as in `git_refdb_write`.
            let r = {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).reflog_read }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(out, b, name) }
            };
            if r < 0 {
                return r;
            }
            d.incref();
            // SAFETY: on success the backend set `*out` to a fresh reflog.
            let log = unsafe { *out };
            // SAFETY: as above.
            unsafe { (*log).db = db };
            0
        })
    }

    /// `git_refdb_should_write_reflog`.
    ///
    /// # Safety
    ///
    /// See the module; `out` writable; `ref` a live reference.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_should_write_reflog(
        out: *mut c_int,
        db: *mut GitRefdb,
        r: *const GitReference,
    ) -> c_int {
        barrier(|| {
            // SAFETY: per the contract (C dereferences NULLs).
            let d = unsafe { RefdbPtr::from_raw(db) };
            // SAFETY: a live reference for the call.
            let v = unsafe { RefView::from_raw(r) };
            match d.should_write_reflog(v) {
                Ok(w) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = c_int::from(w) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_should_write_head_reflog`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_should_write_reflog`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_should_write_head_reflog(
        out: *mut c_int,
        db: *mut GitRefdb,
        r: *const GitReference,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `out` writable.
            unsafe { *out = 0 };
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            // SAFETY: a live reference for the call.
            let v = unsafe { RefView::from_raw(r) };
            match d.should_write_head_reflog(v) {
                Ok(w) => {
                    // SAFETY: `out` writable.
                    unsafe { *out = c_int::from(w) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_has_log`.
    ///
    /// # Safety
    ///
    /// See the module; `refname` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_has_log(db: *mut GitRefdb, refname: *const c_char) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            crate::git_assert_arg!(!refname.is_null(), "refname", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            // SAFETY: a string.
            match d.has_log_raw(unsafe { CStr::from_ptr(refname) }) {
                Ok(v) => v,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_ensure_log`.
    ///
    /// # Safety
    ///
    /// As [`git_refdb_has_log`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_ensure_log(
        db: *mut GitRefdb,
        refname: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            crate::git_assert_arg!(!refname.is_null(), "refname", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            // SAFETY: a string.
            match d.ensure_log(unsafe { CStr::from_ptr(refname) }) {
                Ok(v) => v,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_refdb_init_backend`.
    ///
    /// # Safety
    ///
    /// `backend` is writable for a whole `git_refdb_backend`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_init_backend(
        backend: *mut git_refdb_backend,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            if version == 0 || version > GIT_REFDB_BACKEND_VERSION {
                let _ = git_error_set!(
                    ErrorClass::INVALID,
                    "invalid version {} on git_refdb_backend",
                    version
                );
                return -1;
            }
            let tmpl = git_refdb_backend {
                version: GIT_REFDB_BACKEND_VERSION,
                ..Default::default()
            };
            // SAFETY: `backend` writable (the contract).
            unsafe { backend.write(tmpl) };
            0
        })
    }

    /// `git_refdb_lock`.
    ///
    /// # Safety
    ///
    /// See the module; `payload` writable; `refname` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_lock(
        payload: *mut *mut c_void,
        db: *mut GitRefdb,
        refname: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!payload.is_null(), "payload", -1);
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            crate::git_assert_arg!(!refname.is_null(), "refname", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            let b = d.backend();
            // SAFETY: a live backend (C dereferences NULL).
            let Some(f) = (unsafe { (*b).lock }) else {
                let _ = git_error_set!(ErrorClass::REFERENCE, "backend does not support locking");
                return -1;
            };
            // SAFETY: the backend's slot, with C's arguments.
            unsafe { f(payload, b, refname) }
        })
    }

    /// `git_refdb_unlock`.
    ///
    /// # Safety
    ///
    /// See the module; the arguments as the backend's `unlock` takes them.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_unlock(
        db: *mut GitRefdb,
        payload: *mut c_void,
        success: c_int,
        update_reflog: c_int,
        r: *const GitReference,
        sig: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!db.is_null(), "db", -1);
            // SAFETY: per the contract.
            let d = unsafe { RefdbPtr::from_raw(db) };
            let b = d.backend();
            // the backend's slot (C calls it unconditionally).
            {
                // SAFETY: a live table, whose slots C does not change while it is used.
                let f = unsafe { (*b).unlock }.expect("refdb backend slot is NULL");
                // SAFETY: the slot of that table, with its own table and C's arguments.
                unsafe { f(b, payload, success, update_reflog, r, sig, message) }
            }
        })
    }

    #[allow(unused_imports)]
    use errors as _;
}

crate::c_layout_test! {
    headers: ["refdb.h", "git2/sys/refdb_backend.h"],
    types: [
        GitRefdb => "git_refdb" { rc, repo, backend },
        git_refdb_backend => "git_refdb_backend" {
            version, init, exists, lookup, iterator, write, rename, del, compress,
            has_log, ensure_log, free, reflog_read, reflog_write, reflog_rename,
            reflog_delete, lock, unlock
        },
        git_reference_iterator => "git_reference_iterator" { db, next, next_name, free },
    ],
    asserts: [
        "offsetof(git_repository, refdb_type) == 240",
        "GIT_REFDB_FILES == 1",
        "GIT_REFDB_REFTABLE == 2",
        "GIT_REFDB_BACKEND_VERSION == 1",
        "GIT_LOGALLREFUPDATES_UNSET == 2",
        "GIT_LOGALLREFUPDATES_ALWAYS == 3",
        "GIT_CONFIGMAP_IGNORECASE == 3",
        "GIT_CONFIGMAP_PRECOMPOSE == 8",
        "GIT_CONFIGMAP_LOGALLREFUPDATES == 10",
        "GIT_CONFIGMAP_FSYNCOBJECTFILES == 13",
        "sizeof(git_signature) == 32",
        "offsetof(git_signature, when) == 16",
        "sizeof(git_time) == 16",
    ],
}
