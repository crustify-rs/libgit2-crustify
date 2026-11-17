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
use crate::refs::deps::{RepoPtr, SigRef, configmap, git_repository, git_signature};
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
/// As [`free_db`]: `db` is a live database whose last count was just
/// dropped.
unsafe fn release_db(db: NonNull<GitRefdb>) {
    // SAFETY: the caller's guarantee is `free_db`'s.
    unsafe { free_db(db.as_ptr()) }
}

crate::impl_ccounted!(GitRefdb, crate::util::alloc::GitAlloc, release = release_db);

// SAFETY: `CShareable` lets `CShared` hand out `&GitRefdb` while the
// database is shared. Its fields: `rc` is the atomic `GitRefcount` (C writes
// it only through `GIT_REFCOUNT_*`, which are atomic); `backend` is an
// `AtomicPtr` (C reads it with plain loads in `refdb_reftable.c`, never
// writes it); `repo` is written once by `new_db`, before the database is
// shared, and only read after.
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
    // SAFETY: a live, aligned database (the contract); `backend` is an
    // atomic, so a load through `&` is fine even if C still held a pointer.
    let backend = unsafe { (*db).backend.load(Ordering::Acquire) };
    // SAFETY: the database owned its backend (NULL or live); nothing
    // reaches it after the last count (the contract), so it is given up.
    unsafe { free_backend(backend) };
    // SAFETY: the database's own block (`git__calloc` of a `GitRefdb`),
    // which nothing uses any more: `git__memzero` as C does.
    unsafe { core::ptr::write_bytes(db.cast::<u8>(), 0, core::mem::size_of::<GitRefdb>()) };
    // SAFETY: as above: from `git__calloc`, freed once.
    unsafe { alloc::free(db.cast()) };
}

/// Free `backend` through its `free` slot (NULL: nothing).
///
/// # Safety
///
/// `backend` is NULL or a live backend the caller owns and gives up.
pub(crate) unsafe fn free_backend(backend: *mut git_refdb_backend) {
    if backend.is_null() {
        return;
    }
    // SAFETY: a non-null `backend` is live (the contract), and its table
    // is aligned and initialized (a `git_refdb_backend` header); `free` is
    // checked by `git_refdb_set_backend` and set by the built-in backends.
    let free = unsafe { (*backend).free };
    if let Some(f) = free {
        // SAFETY: the backend's own `free` slot with the backend, which the
        // caller gives up: called once, nothing uses `backend` after.
        unsafe { f(backend) };
    }
}

/// `GIT_REFCOUNT_DEC(db, git_refdb__free)`.
///
/// # Safety
///
/// `db` is a live database on which the caller holds a count it drops.
pub unsafe fn decref(db: *mut GitRefdb) {
    // SAFETY: a live database on which the caller gives up one count (the
    // contract); the last one frees it through `release_db`.
    unsafe { CShared::decrement_count(db) }
}

// ---------------------------------------------------------------------------
// The database, safely
// ---------------------------------------------------------------------------

/// A live `git_refdb`, not counted by this value. Every method keeps C's
/// contract with the backend.
///
/// Invariant (from [`new`](Self::new), [`from_raw`](Self::from_raw) or an
/// export's parameter, which share the contract): the database and its
/// backend are live, and its repository (if set) is live, while this
/// value, a copy, or anything made through it (references, iterators,
/// reflogs that point back to the database) is used.
///
/// `#[repr(transparent)]` over the pointer, so an export may take an
/// `Option<RefdbPtr>` where C passes a `git_refdb *` (NULL is `None`);
/// such a parameter's contract is [`new`](Self::new)'s, for the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct RefdbPtr(NonNull<GitRefdb>);

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
    /// `db` is a live database whose backend (if set) is live, and whose
    /// repository (if set) is live, while this value (or a copy, or what is
    /// made through it) is used, on the threads that use it. A database on
    /// which the user holds a count (a reference's, an iterator's, a
    /// repository's) stays live; its backend changes only through
    /// [`set_backend`], whose callers keep other users away.
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
        // SAFETY: the repository holds a count on its database (and the
        // database owns its backend) while the repository lives, which
        // `RepoPtr`'s contract extends over every use of what is made
        // through it; `git_repository_set_refdb` (an `unsafe fn`) is the
        // only way to drop that count early.
        Ok(unsafe { RefdbPtr::new(db) })
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut GitRefdb {
        self.0.as_ptr()
    }

    fn db(&self) -> &GitRefdb {
        // SAFETY: a live, aligned database (the type's invariant), for the
        // borrow of `self`; its mutable fields are atomics and `repo` is
        // fixed, so a shared reference meets no write (`CShareable`).
        unsafe { self.0.as_ref() }
    }

    /// `db->repo`.
    pub fn repo(self) -> Option<RepoPtr> {
        // SAFETY: the repository of the database is live while this value
        // and what is made through it are used (the type's invariant).
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
        // SAFETY: a live database (the type's invariant); the count is
        // atomic.
        unsafe { CShared::increment_count(self.as_ptr()) }
    }

    /// The backend (one load of `db->backend`) and its slot `f`: the call
    /// passes that same backend, so a concurrent `git_refdb_set_backend`
    /// cannot pair one table's slot with another table (C loads the
    /// pointer twice).
    ///
    /// Every call of a slot `f` from `(b, f)` below is sound for the same
    /// reasons, which its `SAFETY` comment abbreviates as "`b`'s slot": `b`
    /// is the live backend (the type's invariant), `f` its own slot (set by
    /// the backend, unchanged while it is in use), called with `b` and with
    /// arguments of the types the slot takes: C strings from `&CStr`s and
    /// out-parameters that are our locals, all live for the call.
    fn slot<T: Copy>(
        self,
        f: impl FnOnce(&git_refdb_backend) -> Option<T>,
    ) -> Result<(*mut git_refdb_backend, T)> {
        let b = self.backend_checked()?;
        // SAFETY: `b` is the non-null, live backend (the type's invariant),
        // whose table (a `git_refdb_backend` header, aligned) nothing writes
        // while it is in use; the reference lives only for `f`.
        let s = f(unsafe { &*b });
        // C calls the slot unconditionally: a NULL one crashes there.
        Ok((b, s.expect("refdb backend slot is NULL")))
    }

    /// The backend (one load) and its optional slot `f`, for the calls
    /// that check the slot (C: `if (db->backend->x)`). A NULL backend is
    /// `None` here (C dereferences it).
    fn opt_slot<T: Copy>(
        self,
        f: impl FnOnce(&git_refdb_backend) -> Option<T>,
    ) -> (*mut git_refdb_backend, Option<T>) {
        let b = self.backend();
        // SAFETY: as in `slot`, for a non-null `b`.
        let s = unsafe { b.as_ref() }.and_then(f);
        (b, s)
    }

    /// `git_refdb_init`.
    pub fn init(self, head_target: Option<&CStr>, mode: libc::mode_t, flags: u32) -> Result<()> {
        let b = self.backend_checked()?;
        // SAFETY: `b` is non-null and live, its table aligned and unwritten
        // (as in `slot`); `init` is a plain field read.
        let Some(init) = (unsafe { (*b).init }) else {
            return Ok(());
        };
        // SAFETY: `b`'s slot (see `slot`); `head_target` NULL or a string.
        ErrorCode::check(unsafe { init(b, opt_c(head_target), mode, flags) }).map(drop)
    }

    /// `git_refdb_exists`.
    pub fn exists(self, name: &CStr) -> Result<bool> {
        let (b, f) = self.slot(|b| b.exists)?;
        let mut exists = 0;
        // SAFETY: `b`'s slot (see `slot`); `exists` our local.
        ErrorCode::check(unsafe { f(&raw mut exists, b, c(name)) })?;
        Ok(exists != 0)
    }

    /// `git_refdb_lookup`: the reference, with `ref->db` this database.
    pub fn lookup(self, name: &CStr) -> Result<RefBox> {
        let (b, f) = self.slot(|b| b.lookup)?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `b`'s slot (see `slot`); `out` our local.
        let r = unsafe { f(&raw mut out, b, c(name)) };
        // SAFETY: `out` is NULL or the reference the backend made
        // (`git_reference__alloc*`) and handed over to us alone.
        let r_ref = unsafe { RefBox::from_raw(out) };
        ErrorCode::check(r)?;
        let mut r_ref = r_ref.ok_or_else(|| {
            // C dereferences the NULL reference.
            git_error_set!(ErrorClass::REFERENCE, "refdb backend returned no reference")
        })?;
        r_ref.attach_db(self);
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
        let (b, f) = self.opt_slot(|t| t.iterator);
        let Some(f) = f else {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "this backend doesn't support iterators"
            ));
        };
        let mut out = core::ptr::null_mut();
        // SAFETY: `b`'s slot (see `slot`); `out` our local; `glob` NULL or
        // a string.
        ErrorCode::check(unsafe { f(&raw mut out, b, opt_c(glob)) })?;
        let it = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
        self.incref();
        // SAFETY: the backend's fresh iterator, handed over to us alone: a
        // live, aligned `git_reference_iterator` header whose `db` we set,
        // as `git_refdb_iterator` does.
        unsafe { (*it.as_ptr()).db = self.as_ptr() };
        Ok(it)
    }

    /// `git_refdb_write`: `ref->db` becomes this database (counted), then
    /// the backend writes.
    pub fn write(
        self,
        r: &mut RefBox,
        force: bool,
        who: Option<SigRef<'_>>,
        message: Option<&CStr>,
        old_id: Option<&Oid>,
        old_target: Option<&CStr>,
    ) -> Result<()> {
        let (b, f) = self.slot(|b| b.write)?;
        r.attach_db(self);
        // SAFETY: `f` is the `write` slot of the backend `b` (one load, see
        // `slot`), live by `RefdbPtr`'s contract; `r` is our live reference
        // (borrowed mutably, so nothing else touches it during the call),
        // `who` NULL or a valid signature (`SigRef`), the strings and oid
        // NULL or valid for the call (references), as the slot takes them.
        ErrorCode::check(unsafe {
            f(
                b,
                r.as_ptr(),
                c_int::from(force),
                SigRef::opt_ptr(who),
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
        who: Option<SigRef<'_>>,
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
        // SAFETY: `b`'s slot (see `slot`); `outp` NULL or our local; `who`
        // NULL or a valid signature (`SigRef`).
        let r = unsafe {
            f(
                outp,
                b,
                c(old_name),
                c(new_name),
                c_int::from(force),
                SigRef::opt_ptr(who),
                opt_c(message),
            )
        };
        // SAFETY: `out` is NULL or the reference the backend made and
        // handed over to us alone.
        let made = unsafe { RefBox::from_raw(out) };
        ErrorCode::check(r)?;
        match made {
            Some(mut m) => {
                m.attach_db(self);
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
        // SAFETY: `b`'s slot (see `slot`); `old_id` NULL or a live oid.
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
        // C dereferences a NULL backend: here, nothing to compress.
        let (b, f) = self.opt_slot(|b| b.compress);
        let Some(f) = f else {
            return Ok(());
        };
        // SAFETY: `b`'s slot (see `slot`).
        ErrorCode::check(unsafe { f(b) }).map(drop)
    }

    /// `git_refdb_reflog_read`: the reflog, with a count on this database.
    pub fn reflog_read(self, name: &CStr) -> Result<NonNull<GitReflog>> {
        let (b, f) = self.slot(|b| b.reflog_read)?;
        let mut out = core::ptr::null_mut();
        // SAFETY: `b`'s slot (see `slot`); `out` our local.
        ErrorCode::check(unsafe { f(&raw mut out, b, c(name)) })?;
        let log = NonNull::new(out).ok_or(ErrorCode::GENERIC)?;
        self.incref();
        // SAFETY: the backend's fresh reflog (a `git_reflog`, aligned),
        // handed over to us alone; we set its `db`, as C does.
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
        // SAFETY: `b`'s slot (see `slot`), with the caller's live reflog,
        // which nothing else uses during the call (the contract).
        ErrorCode::check(unsafe { f(b, log) }).map(drop)
    }

    /// `backend->reflog_rename`.
    pub fn reflog_rename(self, old_name: &CStr, new_name: &CStr) -> Result<()> {
        let (b, f) = self.slot(|b| b.reflog_rename)?;
        // SAFETY: `b`'s slot (see `slot`).
        ErrorCode::check(unsafe { f(b, c(old_name), c(new_name)) }).map(drop)
    }

    /// `backend->reflog_delete`.
    pub fn reflog_delete(self, name: &CStr) -> Result<()> {
        let (b, f) = self.slot(|b| b.reflog_delete)?;
        // SAFETY: `b`'s slot (see `slot`).
        ErrorCode::check(unsafe { f(b, c(name)) }).map(drop)
    }

    /// `git_refdb_has_log`: the backend's answer (1, 0, or an error).
    pub fn has_log(self, name: &CStr) -> Result<bool> {
        let (b, f) = self.slot(|b| b.has_log)?;
        // SAFETY: `b`'s slot (see `slot`).
        Ok(ErrorCode::check(unsafe { f(b, c(name)) })? != 0)
    }

    /// `git_refdb_has_log` as C returns it.
    pub fn has_log_raw(self, name: &CStr) -> Result<c_int> {
        let (b, f) = self.slot(|b| b.has_log)?;
        // SAFETY: `b`'s slot (see `slot`).
        Ok(unsafe { f(b, c(name)) })
    }

    /// `git_refdb_ensure_log`.
    pub fn ensure_log(self, name: &CStr) -> Result<c_int> {
        let (b, f) = self.slot(|b| b.ensure_log)?;
        // SAFETY: `b`'s slot (see `slot`).
        Ok(unsafe { f(b, c(name)) })
    }

    /// `git_refdb_lock`.
    pub fn lock(self, name: &CStr) -> Result<*mut c_void> {
        let (b, f) = self.opt_slot(|b| b.lock);
        let Some(f) = f else {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "backend does not support locking"
            ));
        };
        let mut payload = core::ptr::null_mut();
        // SAFETY: `b`'s slot (see `slot`); `payload` our local.
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
    // SAFETY: a live repository (`RepoPtr`'s invariant), which the backend
    // keeps as a weak pointer: the backend belongs to the repository's
    // refdb, freed before the repository; `out` our local.
    ErrorCode::check(unsafe { git_refdb_backend_reftable(&raw mut out, repo.as_ptr()) })?;
    Ok(out)
}

/// `git_refdb_set_backend`: replace (and free) the backend, after checking
/// the version and the required slots.
///
/// # Safety
///
/// `db` is a live database; `backend` NULL or a live backend whose
/// ownership passes to the database on success. The old backend is freed
/// at once (C keeps no count on a backend): nothing may be using it, on
/// any thread, during or after the call (no call through a `RefdbPtr` of
/// `db` in flight, no iterator of it alive).
pub unsafe fn set_backend(db: RefdbPtr, backend: *mut git_refdb_backend) -> Result<()> {
    // SAFETY: `backend` is NULL or a live, aligned backend table (the
    // contract), which nothing writes during the call.
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
    // SAFETY: the database owned the old backend (NULL or live); the swap
    // unpublished it and nothing else uses it (the contract), so it is
    // given up here.
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
    // SAFETY: `iter` is a live iterator (the contract): its table is
    // readable, and C does not change its slots while it is used.
    let f = unsafe { (*iter).next }.expect("iterator next is NULL");
    // SAFETY: the `next` slot of that iterator, with the iterator itself
    // (used by nobody else during the call, the contract) and our writable
    // local `out`.
    let r = unsafe { f(&raw mut out, iter) };
    // SAFETY: `out` is NULL or the reference the backend made and handed
    // over to us (a `git_reference__alloc*` block nothing else holds).
    let made = unsafe { RefBox::from_raw(out) };
    ErrorCode::check(r)?;
    let mut made = made.ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a live iterator (as above); `db` is a plain field that only
    // `git_refdb_iterator` writes, before the iterator is handed out.
    let db = unsafe { (*iter).db };
    // C increments a NULL `db` (a crash): the panic becomes an error.
    let db = NonNull::new(db).expect("iterator without a database");
    // SAFETY: the iterator holds a count on its database, which keeps it
    // live for this call; only its count is touched.
    made.attach_db(unsafe { RefdbPtr::new(db) });
    Ok(made)
}

/// `git_refdb_iterator_next_name`: the raw status and name.
///
/// # Safety
///
/// As [`iterator_next`]; `out` is writable for a `const char *` (the
/// backend writes the name there).
pub unsafe fn iterator_next_name(
    out: *mut *const c_char,
    iter: *mut git_reference_iterator,
) -> c_int {
    // SAFETY: `iter` is a live iterator (the contract): its table is
    // readable, and C does not change its slots while it is used.
    let f = unsafe { (*iter).next_name }.expect("iterator next_name is NULL");
    // SAFETY: the `next_name` slot of that iterator, with the iterator
    // (used by nobody else during the call) and `out` (writable, the
    // contract).
    unsafe { f(out, iter) }
}

/// `git_refdb_iterator_free`: drop the iterator's count on its database,
/// then free it.
///
/// # Safety
///
/// `iter` is a live iterator from `git_refdb_iterator`, given up.
pub unsafe fn iterator_free(iter: *mut git_reference_iterator) {
    // SAFETY: a live, aligned iterator header (the contract), unwritten
    // during the call.
    let db = unsafe { (*iter).db };
    // SAFETY: as above.
    let free = unsafe { (*iter).free };
    if !db.is_null() {
        // SAFETY: the iterator holds a count on its live database, given up
        // here (C's order: the count, then the iterator).
        unsafe { decref(db) };
    }
    if let Some(f) = free {
        // SAFETY: the iterator's own `free` slot with the iterator, which
        // the caller gives up: called once, nothing uses `iter` after.
        unsafe { f(iter) };
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `refdb.c`.
///
/// Every `git_refdb *` is NULL (rejected as an invalid argument, where C
/// crashes on some) or a live database (from `git_refdb_new` /
/// `git_refdb_open`, or a repository's), which, with its backend and its
/// repository, stays live for the call ([`RefdbPtr::new`]'s contract);
/// its backend is not replaced during the call. libgit2's threading rules
/// hold for it. Pointers passed through to a backend slot unread are the
/// slot's contract.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};

    /// The database of a `git_refdb *` argument, checked as
    /// `GIT_ASSERT_ARG(db)`.
    fn dbp(db: Option<RefdbPtr>) -> Result<RefdbPtr> {
        db.ok_or_else(|| errors::assert_arg_failed("db"))
    }

    /// One load of `db->backend`, checked as `GIT_ASSERT_ARG(<text>)`
    /// (C loads it again for the call; here the checked pointer is the one
    /// called, see [`RefdbPtr::slot`]).
    fn backend_arg(d: RefdbPtr, text: &'static str) -> Result<*mut git_refdb_backend> {
        let b = d.backend();
        if b.is_null() {
            return Err(errors::assert_arg_failed(text));
        }
        Ok(b)
    }

    /// The slot `f` of the live, non-null backend `b`; a NULL slot panics
    /// (C calls it unconditionally: a crash; the barrier makes it an
    /// error).
    fn slot_of<T: Copy>(
        b: *mut git_refdb_backend,
        f: impl FnOnce(&git_refdb_backend) -> Option<T>,
    ) -> T {
        // SAFETY: `b` is a non-null backend loaded from a live database
        // (the exports' contract: a live database has a live backend),
        // whose table nothing writes while it is in use; the reference
        // lives only for `f`.
        f(unsafe { &*b }).expect("refdb backend slot is NULL")
    }

    /// `git_refdb_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository, which
    /// stays live while the database is used (the database keeps it as a
    /// weak pointer).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_new(
        mut out: Out<'_, *mut GitRefdb>,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            crate::git_assert_arg!(!repo.is_null(), "repo", -1);
            match new_db(repo) {
                Ok(db) => {
                    out.write(CShared::into_raw(db));
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
        mut out: Out<'_, *mut GitRefdb>,
        repo: Option<RepoPtr>,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            let repo = git_arg!(repo, "repo", -1);
            out.write(core::ptr::null_mut());
            match open(repo) {
                Ok(db) => {
                    out.write(CShared::into_raw(db));
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
    /// `db` is NULL or a live database; `backend` NULL or a live backend
    /// whose ownership passes to `db` on success. Nothing may use the old
    /// backend during or after the call ([`set_backend`]).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_set_backend(
        db: Option<RefdbPtr>,
        backend: *mut git_refdb_backend,
    ) -> c_int {
        barrier(|| {
            let r = dbp(db).and_then(|d| {
                // SAFETY: this export's contract is `set_backend`'s.
                unsafe { set_backend(d, backend) }
            });
            status(r)
        })
    }

    /// `git_refdb_compress`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_compress(db: Option<RefdbPtr>) -> c_int {
        barrier(|| status(dbp(db).and_then(RefdbPtr::compress)))
    }

    /// `git_refdb__free`.
    ///
    /// # Safety
    ///
    /// `db` is a live database whose last count is dropped: nothing uses it
    /// after the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb__free(db: *mut GitRefdb) {
        // SAFETY: this export's contract is `free_db`'s.
        barrier(|| unsafe { free_db(db) })
    }

    /// `git_refdb_free`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database on which the caller gives up one
    /// count.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_free(db: *mut GitRefdb) {
        // SAFETY: NULL or a live `CShared` database (made by `new_db`)
        // whose count the caller gives up, as `release` takes it.
        barrier(|| unsafe { CShared::release(db) })
    }

    /// `git_refdb_init`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module); `head_target`
    /// NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_init(
        db: Option<RefdbPtr>,
        head_target: CStrArg<'_>,
        mode: libc::mode_t,
        flags: u32,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "refdb", -1);
            crate::git_assert_arg!(!d.backend().is_null(), "refdb->backend", -1);
            status(d.init(head_target.get(), mode, flags))
        })
    }

    /// `git_refdb_exists`.
    ///
    /// # Safety
    ///
    /// `exists` is NULL or writable for an `int`; `db` NULL or a live
    /// database (see the module); `ref_name` what the backend's `exists`
    /// accepts (NULL or a NUL-terminated string for the crate's backends).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_exists(
        exists: *mut c_int,
        db: Option<RefdbPtr>,
        ref_name: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!exists.is_null(), "exists", -1);
            let d = git_arg!(db, "refdb", -1);
            let b = match backend_arg(d, "refdb->backend") {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            let f = slot_of(b, |t| t.exists);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(exists, b, ref_name) }
        })
    }

    /// `git_refdb_lookup`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `db` NULL or a live database (see the
    /// module); `ref_name` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_lookup(
        out: *mut *mut GitReference,
        db: *mut GitRefdb,
        ref_name: *const c_char,
    ) -> c_int {
        // SAFETY: `out` NULL or writable, with no reference to `*out` (this
        // export's contract).
        let mut out = unsafe { Out::from_ptr(out) };
        // SAFETY: `ref_name` NULL or a string unwritten during the call.
        let ref_name = unsafe { CStrArg::from_ptr(ref_name) };
        // SAFETY: `db` NULL or a live database for the call (this export's
        // contract is `RefdbPtr::new`'s).
        let db = NonNull::new(db).map(|d| unsafe { RefdbPtr::new(d) });
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            crate::git_assert_arg!(!out.is_null(), "out", -1);
            let name = git_arg!(ref_name.get(), "ref_name", -1);
            match d.lookup(name) {
                Ok(r) => {
                    out.write(r.into_raw());
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
    /// `out` is NULL or writable; `db` NULL or a live database (see the
    /// module); `ref_name` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_resolve(
        mut out: Out<'_, *mut GitReference>,
        db: Option<RefdbPtr>,
        ref_name: CStrArg<'_>,
        max_nesting: c_int,
    ) -> c_int {
        barrier(|| {
            out.write(core::ptr::null_mut());
            let d = git_arg!(db, "db", -1);
            crate::git_assert_arg!(!d.backend().is_null(), "db->backend", -1);
            let name = git_arg!(ref_name.get(), "ref_name", -1);
            match d.resolve(name, max_nesting) {
                Ok(r) => {
                    out.write(r.into_raw());
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
    /// `out` is NULL or writable; `db` NULL or a live database (see the
    /// module); `glob` NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator(
        mut out: Out<'_, *mut git_reference_iterator>,
        db: Option<RefdbPtr>,
        glob: CStrArg<'_>,
    ) -> c_int {
        barrier(|| match dbp(db).and_then(|d| d.iterator(glob.get())) {
            Ok(it) => {
                out.write(it.as_ptr());
                0
            }
            Err(e) => e.raw(),
        })
    }

    /// `git_refdb_iterator_next`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `iter` a live iterator from
    /// `git_refdb_iterator`, used by nobody else during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_next(
        out: *mut *mut GitReference,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        // SAFETY: `out` NULL or writable, with no reference to `*out` (this
        // export's contract).
        let mut out = unsafe { Out::from_ptr(out) };
        barrier(|| {
            // SAFETY: this export's contract on `iter` is `iterator_next`'s.
            match unsafe { iterator_next(iter) } {
                Ok(r) => {
                    out.write(r.into_raw());
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
    /// As [`git_refdb_iterator_next`]; `out` writable (the backend writes
    /// it).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_next_name(
        out: *mut *const c_char,
        iter: *mut git_reference_iterator,
    ) -> c_int {
        // SAFETY: this export's contract is `iterator_next_name`'s.
        barrier(|| unsafe { iterator_next_name(out, iter) })
    }

    /// `git_refdb_iterator_free`.
    ///
    /// # Safety
    ///
    /// `iter` is a live iterator from `git_refdb_iterator` that the caller
    /// gives up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_iterator_free(iter: *mut git_reference_iterator) {
        // SAFETY: this export's contract is `iterator_free`'s.
        barrier(|| unsafe { iterator_free(iter) })
    }

    /// `git_refdb_write`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module); `r` a live
    /// reference (C dereferences NULL) that nothing else uses during the
    /// call (its `db` is set); `who`, `message`, `old_id` and `old_target`
    /// what the backend's `write` accepts (NULL or valid).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_write(
        db: Option<RefdbPtr>,
        r: *mut GitReference,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let b = match backend_arg(d, "db->backend") {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            // C dereferences a NULL reference: the panic becomes an error.
            assert!(!r.is_null(), "NULL reference");
            d.incref();
            // SAFETY: `r` is a live, aligned reference that nothing else
            // uses during the call (this export's contract); C sets `db`
            // without dropping a count it held.
            unsafe { (*r).db = d.as_ptr() };
            let f = slot_of(b, |t| t.write);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(b, r, force, who, message, old_id, old_target) }
        })
    }

    /// `git_refdb_rename`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable (and readable after the backend writes
    /// it); `db` NULL or a live database (see the module); the other
    /// pointers what the backend's `rename` accepts (NULL or valid).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_rename(
        out: *mut *mut GitReference,
        db: Option<RefdbPtr>,
        old_name: *const c_char,
        new_name: *const c_char,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let b = match backend_arg(d, "db->backend") {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            let f = slot_of(b, |t| t.rename);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            let r = unsafe { f(out, b, old_name, new_name, force, who, message) };
            if r < 0 {
                return r;
            }
            if !out.is_null() {
                // SAFETY: `out` is non-null, readable (the contract), and on
                // success the backend set it.
                let made = unsafe { *out };
                if !made.is_null() {
                    d.incref();
                    // SAFETY: the backend's fresh reference (aligned, live),
                    // which nobody else uses yet; we set its `db`, as C does.
                    unsafe { (*made).db = d.as_ptr() };
                }
            }
            0
        })
    }

    /// `git_refdb_delete`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module); the other
    /// pointers what the backend's `del` accepts (NULL or valid).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_delete(
        db: Option<RefdbPtr>,
        ref_name: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let b = match backend_arg(d, "db->backend") {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            let f = slot_of(b, |t| t.del);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(b, ref_name, old_id, old_target) }
        })
    }

    /// `git_refdb_reflog_read`.
    ///
    /// # Safety
    ///
    /// `out` is writable and readable after the backend writes it; `db`
    /// NULL or a live database (see the module); `name` what the backend's
    /// `reflog_read` accepts.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_reflog_read(
        out: *mut *mut GitReflog,
        db: Option<RefdbPtr>,
        name: *const c_char,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let b = match backend_arg(d, "db->backend") {
                Ok(b) => b,
                Err(e) => return e.raw(),
            };
            let f = slot_of(b, |t| t.reflog_read);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            let r = unsafe { f(out, b, name) };
            if r < 0 {
                return r;
            }
            d.incref();
            // SAFETY: on success the backend set `*out` (readable, the
            // contract) to a fresh reflog.
            let log = unsafe { *out };
            // SAFETY: that fresh reflog (aligned, live), which nobody else
            // uses yet; we set its `db`, as C does (C dereferences NULL).
            unsafe { (*log).db = d.as_ptr() };
            0
        })
    }

    /// `git_refdb_should_write_reflog`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `db` NULL or a live database with a live
    /// repository (see the module); `r` NULL or a live reference,
    /// unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_should_write_reflog(
        mut out: Out<'_, c_int>,
        db: Option<RefdbPtr>,
        r: Option<RefView<'_>>,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let v = git_arg!(r, "ref", -1);
            match d.should_write_reflog(v) {
                Ok(w) => {
                    out.write(c_int::from(w));
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
        mut out: Out<'_, c_int>,
        db: Option<RefdbPtr>,
        r: Option<RefView<'_>>,
    ) -> c_int {
        barrier(|| {
            out.write(0);
            let d = git_arg!(db, "db", -1);
            let v = git_arg!(r, "ref", -1);
            match d.should_write_head_reflog(v) {
                Ok(w) => {
                    out.write(c_int::from(w));
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
    /// `db` is NULL or a live database (see the module); `refname` NULL or
    /// a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_has_log(
        db: Option<RefdbPtr>,
        refname: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let name = git_arg!(refname.get(), "refname", -1);
            match d.has_log_raw(name) {
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
        db: Option<RefdbPtr>,
        refname: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            let name = git_arg!(refname.get(), "refname", -1);
            match d.ensure_log(name) {
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
        mut backend: Out<'_, git_refdb_backend>,
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
            backend.write(tmpl);
            0
        })
    }

    /// `git_refdb_lock`.
    ///
    /// # Safety
    ///
    /// `payload` is NULL or writable (the backend writes it); `db` NULL or
    /// a live database (see the module); `refname` NULL or what the
    /// backend's `lock` accepts.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_lock(
        payload: *mut *mut c_void,
        db: Option<RefdbPtr>,
        refname: *const c_char,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!payload.is_null(), "payload", -1);
            let d = git_arg!(db, "db", -1);
            crate::git_assert_arg!(!refname.is_null(), "refname", -1);
            // C dereferences a NULL backend: the panic becomes an error.
            let b = d.backend();
            assert!(!b.is_null(), "NULL refdb backend");
            // SAFETY: `b` is the non-null backend of a live database (this
            // export's contract), its table aligned and unwritten.
            let Some(f) = (unsafe { (*b).lock }) else {
                let _ = git_error_set!(ErrorClass::REFERENCE, "backend does not support locking");
                return -1;
            };
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(payload, b, refname) }
        })
    }

    /// `git_refdb_unlock`.
    ///
    /// # Safety
    ///
    /// `db` is NULL or a live database (see the module); the other
    /// arguments as the backend's `unlock` takes them (`payload` from its
    /// `lock`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_unlock(
        db: Option<RefdbPtr>,
        payload: *mut c_void,
        success: c_int,
        update_reflog: c_int,
        r: *const GitReference,
        sig: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        barrier(|| {
            let d = git_arg!(db, "db", -1);
            // C dereferences a NULL backend: the panic becomes an error.
            let b = d.backend();
            assert!(!b.is_null(), "NULL refdb backend");
            let f = slot_of(b, |t| t.unlock);
            // SAFETY: the backend's own slot with the backend and C's
            // arguments, passed on as C does (this export's contract).
            unsafe { f(b, payload, success, update_reflog, r, sig, message) }
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
