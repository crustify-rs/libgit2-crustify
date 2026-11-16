//! The safe Rust API of references, the reference database and reflogs
//! (PORTING.md §5): [`Reference`], [`ReferenceType`], [`References`] and
//! [`ReferenceNames`] (with globs), [`Reflog`] and [`ReflogEntry`],
//! [`Refdb`] (the database handle), and [`RefdbBackend`], a reference
//! database backend written in Rust.
//!
//! It is a thin layer over the same Rust as the C ABI (`refs`, `refdb`,
//! `refdb_fs`, `reflog`). No C type appears in it. Reference names, globs
//! and log messages are bytes (`impl AsRef<[u8]>` in, `&[u8]` out): git
//! allows names that are not UTF-8. A NUL inside one is an error, never a
//! truncation.
//!
//! The repository it works on is the crate's [`Repository`] (`Sync`: the
//! reference API may be used from several threads on one repository); the
//! committer of reflog entries is a [`SignatureRef`].
//!
//! ```no_run
//! use libgit2_rs::{Reference, Repository};
//!
//! let repo = Repository::open("/path/to/repo")?;
//! let head = Reference::lookup(&repo, "HEAD")?.resolve()?;
//! let id = head.target().expect("a direct reference");
//! Reference::create(&repo, "refs/heads/copy", &id, false, None, Some(b"copy"))?;
//! assert_eq!(Reference::name_to_id(&repo, "refs/heads/copy")?, id);
//! # Ok::<(), libgit2_rs::Error>(())
//! ```

use core::ffi::{CStr, c_char, c_int, c_void};
use core::marker::PhantomData;
use core::ptr::NonNull;
use std::ffi::CString;

use ffibox::vtable::CTable;

use crate::odb::ObjectType;
use crate::oid::Oid;
use crate::refdb::{
    self, GIT_REFDB_BACKEND_VERSION, RefIteratorTable, RefdbBackendTable, RefdbPtr,
    git_refdb_backend, git_reference_iterator,
};
use crate::reflog::{self as rl, GitReflog};
pub use crate::refs::ReferenceType;
use crate::refs::deps::{RepoPtr, git_signature};
use crate::refs::{
    self, GIT_REFERENCE_DIRECT, GIT_REFERENCE_SYMBOLIC, GitReference, RefBox, RefIter, RefView,
};
pub use crate::repository::handles::Refdb;
pub use crate::signature::{Signature, SignatureRef};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode};
use crate::util::pool::GitPool;
use crate::util::wildmatch::{WildmatchFlags, wildmatch};

type Result<T> = core::result::Result<T, Error>;

fn rich<T>(r: errors::Result<T>) -> Result<T> {
    r.map_err(Error::from)
}

/// A name as a C string: an interior NUL makes it an invalid name.
fn cname(name: &[u8]) -> Result<CString> {
    CString::new(name).map_err(|_| {
        Error::with(
            ErrorCode::INVALID_SPEC,
            ErrorClass::REFERENCE,
            format!("the given reference name '{}' is not valid", Bytes(name)),
        )
    })
}

/// A log message as a C string: an interior NUL is refused (C would see
/// a shorter message).
fn cmsg(message: Option<&[u8]>) -> Result<Option<CString>> {
    message
        .map(|m| {
            CString::new(m).map_err(|_| {
                Error::with(
                    ErrorCode::INVALID,
                    ErrorClass::INVALID,
                    "the log message contains a NUL byte",
                )
            })
        })
        .transpose()
}

// ---------------------------------------------------------------------------
// The repository
// ---------------------------------------------------------------------------

pub use crate::repository::Repository;

impl Repository {
    /// The repository as the refs modules take it.
    pub(crate) fn ptr(&self) -> RepoPtr {
        // SAFETY: an owner's repository pointer is not NULL.
        let p = unsafe { NonNull::new_unchecked(self.raw_ptr()) };
        // SAFETY: the repository this owner holds, live while `self` is
        // borrowed (the references and iterators borrow it); it is `Sync`,
        // so the refs code may use it from the threads that share `self`.
        unsafe { RepoPtr::new(p) }
    }

    /// Install `backend` as the backend of the repository's reference
    /// database (`git_refdb_set_backend` on `git_repository_refdb`); the
    /// previous backend is freed.
    ///
    /// It takes the repository exclusively: the old backend is freed at
    /// once (as in C, which keeps no count on a backend), so no other
    /// thread, and no [`Reference`], iterator or [`Refdb`] of the
    /// repository, may be using it.
    ///
    /// ```compile_fail
    /// # use libgit2_rs::*;
    /// # fn f<B: RefdbBackend>(mut repo: Repository, b: B) {
    /// let head = Reference::lookup(&repo, "HEAD").unwrap();
    /// repo.set_refdb_backend(b).unwrap(); // `repo` is borrowed by `head`
    /// drop(head);
    /// # }
    /// ```
    pub fn set_refdb_backend<B: RefdbBackend>(&mut self, backend: B) -> Result<()> {
        let db = RefdbPtr::of_repo(self.ptr()).map_err(Error::from)?;
        // SAFETY: the repository's database, which `&mut self` keeps from
        // every other user (all of them borrow the repository).
        unsafe { install_rust_backend(db, backend, self.oid_type()) }
    }

    /// Install a new files backend (`git_refdb_backend_fs`, the backend
    /// of a repository's `refs/` and `packed-refs`) in the repository's
    /// reference database, e.g. to restore it after
    /// [`set_refdb_backend`](Self::set_refdb_backend). The previous backend
    /// is freed; as there, the repository is taken exclusively.
    pub fn set_refdb_backend_fs(&mut self) -> Result<()> {
        let repo = self.ptr();
        let db = RefdbPtr::of_repo(repo).map_err(Error::from)?;
        // SAFETY: as in `set_refdb_backend`.
        unsafe { install_fs_backend(db, repo) }
    }
}

/// Install a Rust backend in `db`.
///
/// # Safety
///
/// `db` is live, and nothing uses its current backend now or later (it is
/// freed).
pub(crate) unsafe fn install_rust_backend<B: RefdbBackend>(
    db: RefdbPtr,
    backend: B,
    oid_type: crate::oid::OidType,
) -> Result<()> {
    let table = CTable::<RefdbBackendTable, RustBackend<B>>::new(RustBackend(
        crate::api::NoUnwindDrop::new(backend),
        oid_type,
    ));
    let raw = table.into_c().as_ptr();
    // SAFETY: a live database and a complete backend, whose ownership
    // passes to the database; the old backend is unused (the contract).
    let r = unsafe { refdb::set_backend(db, raw) };
    if let Err(e) = r {
        // SAFETY: the table was not taken: release it through its slot.
        unsafe { CTable::<RefdbBackendTable, RustBackend<B>>::release(raw) };
        return Err(Error::from(e));
    }
    Ok(())
}

/// Install a new files backend of `repo` in `db`.
///
/// # Safety
///
/// As [`install_rust_backend`]; `db` belongs to `repo`.
pub(crate) unsafe fn install_fs_backend(db: RefdbPtr, repo: RepoPtr) -> Result<()> {
    let b = rich(crate::refdb_fs::backend_fs(repo))?;
    // SAFETY: a live database and a complete backend, whose ownership
    // passes to the database; the old backend is unused (the contract).
    if let Err(e) = unsafe { refdb::set_backend(db, b) } {
        // SAFETY: the backend was not taken: free it (once).
        unsafe { refdb::free_backend(b) };
        return Err(Error::from(e));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// A reference of a repository (`git_reference`), freed on drop.
///
/// It is a snapshot: the methods that change the reference return the
/// updated one.
pub struct Reference<'repo> {
    inner: RefBox,
    _repo: PhantomData<&'repo Repository>,
}

impl core::fmt::Debug for Reference<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("Reference");
        d.field("name", &Bytes(self.name()));
        match self.kind() {
            ReferenceType::Direct => d.field("target", &self.target()),
            ReferenceType::Symbolic => {
                d.field("symbolic_target", &self.symbolic_target().map(Bytes))
            }
        };
        d.finish()
    }
}

crate::api::flags::bitflags! {
    /// The flags of [`Reference::normalize_name`]
    /// (`git_reference_format_t`).
    pub struct ReferenceFormat: u32 {
        /// `GIT_REFERENCE_FORMAT_NORMAL`: no flag.
        NORMAL = 0;
        /// `GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL`: names without a `/`
        /// (`HEAD`, `ORIG_HEAD`, …).
        ALLOW_ONELEVEL = refs::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL;
        /// `GIT_REFERENCE_FORMAT_REFSPEC_PATTERN`: one `*` in the name.
        REFSPEC_PATTERN = refs::GIT_REFERENCE_FORMAT_REFSPEC_PATTERN;
        /// `GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND`: a short name
        /// (`main`) is valid.
        REFSPEC_SHORTHAND = refs::GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND;
    }
}

impl<'repo> Reference<'repo> {
    pub(crate) fn wrap(inner: RefBox) -> Reference<'repo> {
        Reference {
            inner,
            _repo: PhantomData,
        }
    }

    fn view(&self) -> RefView<'_> {
        self.inner.view()
    }

    /// `git_reference_lookup`.
    pub fn lookup(repo: &'repo Repository, name: impl AsRef<[u8]>) -> Result<Reference<'repo>> {
        let name = name.as_ref();
        cname(name)?;
        rich(refs::lookup(repo.ptr(), name)).map(Self::wrap)
    }

    /// `git_reference_lookup_resolved`: follow symbolic references up to
    /// `max_nesting` deep (`None`: the default, 5).
    pub fn lookup_resolved(
        repo: &'repo Repository,
        name: impl AsRef<[u8]>,
        max_nesting: Option<u32>,
    ) -> Result<Reference<'repo>> {
        let name = name.as_ref();
        cname(name)?;
        let n = max_nesting.map_or(-1, |n| c_int::try_from(n).unwrap_or(c_int::MAX));
        rich(refs::lookup_resolved(repo.ptr(), name, n)).map(Self::wrap)
    }

    /// `git_reference_dwim`: look a short name up as git does.
    pub fn dwim(repo: &'repo Repository, shorthand: impl AsRef<[u8]>) -> Result<Reference<'repo>> {
        let shorthand = shorthand.as_ref();
        cname(shorthand)?;
        rich(refs::dwim(repo.ptr(), shorthand)).map(Self::wrap)
    }

    /// `git_reference_create_matching` (`git_reference_create` without
    /// `current`): create (or with `force`, overwrite) `name` pointing at
    /// `id`; with `current`, only if the reference is now at that id
    /// ([`ErrorCode::MODIFIED`] otherwise).
    pub fn create(
        repo: &'repo Repository,
        name: impl AsRef<[u8]>,
        id: &Oid,
        force: bool,
        current: Option<&Oid>,
        log_message: Option<&[u8]>,
    ) -> Result<Reference<'repo>> {
        let name = name.as_ref();
        cname(name)?;
        let msg = cmsg(log_message)?;
        rich(refs::create_matching(
            repo.ptr(),
            name,
            id,
            force,
            current,
            msg.as_deref(),
        ))
        .map(Self::wrap)
    }

    /// `git_reference_symbolic_create_matching` (`…_symbolic_create`
    /// without `current`).
    pub fn symbolic_create(
        repo: &'repo Repository,
        name: impl AsRef<[u8]>,
        target: impl AsRef<[u8]>,
        force: bool,
        current: Option<&[u8]>,
        log_message: Option<&[u8]>,
    ) -> Result<Reference<'repo>> {
        let (name, target) = (name.as_ref(), target.as_ref());
        cname(name)?;
        cname(target)?;
        let msg = cmsg(log_message)?;
        let cur = current.map(cname).transpose()?;
        rich(refs::symbolic_create_matching(
            repo.ptr(),
            name,
            target,
            force,
            cur.as_deref(),
            msg.as_deref(),
        ))
        .map(Self::wrap)
    }

    /// The full name (`git_reference_name`).
    pub fn name(&self) -> &[u8] {
        self.view().name()
    }

    /// The full name, if it is UTF-8.
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(self.name()).ok()
    }

    /// The short name (`git_reference_shorthand`).
    pub fn shorthand(&self) -> &[u8] {
        let n = self.name();
        &n[refs::shorthand_offset(n)..]
    }

    /// Direct or symbolic (`git_reference_type`).
    pub fn kind(&self) -> ReferenceType {
        self.view().kind().unwrap_or(ReferenceType::Direct)
    }

    /// The target of a direct reference (`git_reference_target`).
    pub fn target(&self) -> Option<Oid> {
        self.view().target().copied()
    }

    /// The peeled target of a direct reference, when the database knows it
    /// (`git_reference_target_peel`).
    pub fn target_peel(&self) -> Option<Oid> {
        let v = self.view();
        (v.raw_kind() == GIT_REFERENCE_DIRECT && !v.peel().is_zero()).then(|| *v.peel())
    }

    /// The target of a symbolic reference
    /// (`git_reference_symbolic_target`).
    pub fn symbolic_target(&self) -> Option<&[u8]> {
        self.view().symbolic_target()
    }

    /// Under `refs/heads/` (`git_reference_is_branch`).
    pub fn is_branch(&self) -> bool {
        refs::is_branch(self.name())
    }

    /// Under `refs/remotes/` (`git_reference_is_remote`).
    pub fn is_remote(&self) -> bool {
        refs::is_remote(self.name())
    }

    /// Under `refs/tags/` (`git_reference_is_tag`).
    pub fn is_tag(&self) -> bool {
        refs::is_tag(self.name())
    }

    /// Under `refs/notes/` (`git_reference_is_note`).
    pub fn is_note(&self) -> bool {
        refs::is_note(self.name())
    }

    /// The direct reference this one ends at (`git_reference_resolve`).
    pub fn resolve(&self) -> Result<Reference<'repo>> {
        rich(refs::resolve(self.view())).map(Self::wrap)
    }

    /// `git_reference_set_target`: point this direct reference at `id`
    /// (if it has not moved); the updated reference.
    pub fn set_target(&self, id: &Oid, log_message: Option<&[u8]>) -> Result<Reference<'repo>> {
        let msg = cmsg(log_message)?;
        rich(refs::set_target(self.view(), id, msg.as_deref())).map(Self::wrap)
    }

    /// `git_reference_symbolic_set_target`.
    pub fn symbolic_set_target(
        &self,
        target: impl AsRef<[u8]>,
        log_message: Option<&[u8]>,
    ) -> Result<Reference<'repo>> {
        let target = target.as_ref();
        cname(target)?;
        let msg = cmsg(log_message)?;
        rich(refs::symbolic_set_target(
            self.view(),
            target,
            msg.as_deref(),
        ))
        .map(Self::wrap)
    }

    /// `git_reference_rename`: the renamed reference (worktrees' HEADs
    /// pointing at it follow).
    pub fn rename(
        &self,
        new_name: impl AsRef<[u8]>,
        force: bool,
        log_message: Option<&[u8]>,
    ) -> Result<Reference<'repo>> {
        let new_name = new_name.as_ref();
        cname(new_name)?;
        let msg = cmsg(log_message)?;
        let mut out = None;
        let r = refs::rename(self.view(), new_name, force, msg.as_deref(), &mut out);
        rich(r)?;
        out.map(Self::wrap)
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))
    }

    /// `git_reference_delete`: delete the reference (and its reflog), if
    /// it still has the value this snapshot has ([`ErrorCode::MODIFIED`]
    /// otherwise). HEAD cannot be deleted.
    pub fn delete(self) -> Result<()> {
        let v = self.view();
        if v.name() == b"HEAD" {
            return Err(Error::with(
                ErrorCode::GENERIC,
                ErrorClass::REFERENCE,
                "cannot delete HEAD",
            ));
        }
        let (old_id, old_target) = if v.raw_kind() == GIT_REFERENCE_DIRECT {
            (Some(v.oid_field()), None)
        } else {
            (None, v.symbolic_target_cstr())
        };
        let db = NonNull::new(v.db()).ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: a reference's database lives while the reference does.
        rich(unsafe { RefdbPtr::new(db) }.delete(v.name_cstr(), old_id, old_target))
    }

    /// `git_reference_peel`: the object the reference ends at, peeled
    /// until it is of type `kind` (`None`: until it is not a tag).
    pub fn peel(&self, kind: Option<ObjectType>) -> Result<crate::object::Object<'repo>> {
        let o = rich(refs::peel(self.view(), ObjectType::raw_or_any(kind)))?;
        let raw = NonNull::new(o.into_raw().cast::<crate::object::GitObject>())
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: the object the peel looked up, whose reference passes to
        // the result; of this reference's repository, which outlives
        // `'repo`.
        Ok(unsafe { crate::object::Object::adopt(raw) })
    }

    /// Whether this is HEAD and its branch has no commit yet
    /// (`git_reference__is_unborn_head`).
    pub fn is_unborn_head(&self) -> Result<bool> {
        let v = self.view();
        let db = NonNull::new(v.db()).ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: a reference's database lives while the reference does.
        let repo = unsafe { RefdbPtr::new(db) }
            .repo()
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        rich(refs::is_unborn_head(v, repo))
    }

    /// `git_reference_name_to_id`: the id `name` resolves to.
    pub fn name_to_id(repo: &Repository, name: impl AsRef<[u8]>) -> Result<Oid> {
        let name = name.as_ref();
        cname(name)?;
        rich(refs::name_to_id(repo.ptr(), name))
    }

    /// `git_reference_remove`: delete `name` (and its reflog), whatever its
    /// value.
    pub fn remove(repo: &Repository, name: impl AsRef<[u8]>) -> Result<()> {
        let n = cname(name.as_ref())?;
        rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.delete(&n, None, None)))
    }

    /// `git_reference_list`: every reference name.
    pub fn list(repo: &Repository) -> Result<Vec<Vec<u8>>> {
        ReferenceNames::new(repo)?.collect()
    }

    /// `git_reference_has_log`: whether `name` has a reflog.
    pub fn has_log(repo: &Repository, name: impl AsRef<[u8]>) -> Result<bool> {
        let n = cname(name.as_ref())?;
        rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.has_log(&n)))
    }

    /// `git_reference_ensure_log`: make sure `name` has a reflog (the next
    /// update of it is logged).
    pub fn ensure_log(repo: &Repository, name: impl AsRef<[u8]>) -> Result<()> {
        let n = cname(name.as_ref())?;
        rich(
            RefdbPtr::of_repo(repo.ptr())
                .and_then(|db| ErrorCode::check(db.ensure_log(&n)?).map(drop)),
        )
    }

    /// `git_reference_normalize_name`: `name` in its canonical form, if it
    /// is valid under `format` ([`ErrorCode::INVALID_SPEC`] otherwise).
    pub fn normalize_name(name: impl AsRef<[u8]>, format: ReferenceFormat) -> Result<Vec<u8>> {
        crate::libgit2::ensure_init()?;
        let name = name.as_ref();
        cname(name)?;
        let mut buf = crate::util::str::GitStr::new();
        rich(refs::normalize_name(Some(&mut buf), name, format.raw()))?;
        Ok(buf.as_bytes().to_vec())
    }

    /// `git_reference_name_is_valid`: whether `name` is a valid reference
    /// name (one-level names such as `HEAD` included).
    pub fn is_valid_name(name: impl AsRef<[u8]>) -> bool {
        let name = name.as_ref();
        crate::libgit2::ensure_init().is_ok()
            && !name.contains(&0)
            && matches!(
                refs::name_is_valid(name, refs::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL),
                Ok(true)
            )
    }

    /// `git_reference_foreach_glob` (or `_foreach_name` without a glob):
    /// call `f` with each name (matching `glob`), until it returns
    /// `false` (then `Ok`).
    pub fn foreach_name(
        repo: &Repository,
        glob: Option<&[u8]>,
        mut f: impl FnMut(&[u8]) -> bool,
    ) -> Result<()> {
        let g = glob.map(cname).transpose()?;
        let mut cb = |n: &CStr| -> c_int { if f(n.to_bytes()) { 0 } else { 1 } };
        let action = if g.is_some() {
            "git_reference_foreach_glob"
        } else {
            "git_reference_foreach_name"
        };
        match refs::foreach_name(repo.ptr(), g.as_deref(), action, &mut cb) {
            Ok(()) => Ok(()),
            // A stop asked by `f` is not an error.
            Err(e) if e.raw() == 1 => {
                errors::clear();
                Ok(())
            }
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl Clone for Reference<'_> {
    /// `git_reference_dup`.
    fn clone(&self) -> Self {
        let d = RefBox::dup(self.view()).expect("out of memory duplicating a reference");
        Self::wrap(d)
    }
}

impl PartialEq for Reference<'_> {
    /// `git_reference_cmp(a, b) == 0`.
    fn eq(&self, other: &Self) -> bool {
        refs::cmp(self.view(), other.view()) == 0
    }
}

impl Eq for Reference<'_> {}

impl PartialOrd for Reference<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Reference<'_> {
    /// `git_reference_cmp`: by name, symbolic before direct, then target.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        refs::cmp(self.view(), other.view()).cmp(&0)
    }
}

// ---------------------------------------------------------------------------
// The database
// ---------------------------------------------------------------------------

impl<'r> Refdb<'r> {
    /// `git_refdb_new`: a reference database of `repo` without a backend
    /// (not the repository's own: see [`Repository::refdb`]). Give it one
    /// with [`set_backend`](Self::set_backend) or
    /// [`set_backend_fs`](Self::set_backend_fs).
    pub fn new(repo: &'r Repository) -> Result<Refdb<'r>> {
        crate::libgit2::ensure_init()?;
        let db = rich(refdb::new_db(repo.raw_ptr().cast()))?;
        Ok(Refdb::from_shared(db))
    }

    /// `git_refdb_open`: a new reference database of `repo` with the
    /// backend of its storage format (the files backend, or reftable),
    /// beside the repository's own.
    pub fn open(repo: &'r Repository) -> Result<Refdb<'r>> {
        crate::libgit2::ensure_init()?;
        let db = rich(refdb::open(repo.ptr()))?;
        Ok(Refdb::from_shared(db))
    }

    fn db(&self) -> RefdbPtr {
        // SAFETY: the live database this handle counts, whose backend the
        // database owns.
        unsafe { RefdbPtr::new(self.as_nonnull()) }
    }

    /// `git_refdb_compress`: let the backend optimise its storage (the
    /// files backend packs the loose references into `packed-refs`). A
    /// database without a backend has nothing to do.
    pub fn compress(&self) -> Result<()> {
        rich(self.db().compress())
    }

    /// Whether the database has a backend.
    pub fn has_backend(&self) -> bool {
        !self.db().backend().is_null()
    }

    /// The database, if this handle is its only owner (so nothing else
    /// can be using its backend).
    fn exclusive(&mut self) -> Result<RefdbPtr> {
        if !self.is_unique() {
            return Err(Error::with(
                ErrorCode::LOCKED,
                ErrorClass::REFERENCE,
                "the reference database is shared: its backend cannot be replaced",
            ));
        }
        Ok(self.db())
    }

    /// `git_refdb_set_backend` of a Rust backend: the previous backend is
    /// freed. Only for a database this handle alone owns (one made by
    /// [`new`](Self::new) or [`open`](Self::open) and not cloned): a shared
    /// one gives [`ErrorCode::LOCKED`]. For the repository's own database,
    /// see [`Repository::set_refdb_backend`].
    pub fn set_backend<B: RefdbBackend>(&mut self, backend: B) -> Result<()> {
        let db = self.exclusive()?;
        let repo = db.repo().ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        let oid_type = crate::oid::git_oid_t::try_from(repo.oid_type())
            .ok()
            .and_then(crate::oid::OidType::from_raw)
            .unwrap_or(crate::oid::OidType::Sha1);
        // SAFETY: the database is this handle's alone (`&mut self` and a
        // count of one): nothing else uses its backend.
        unsafe { install_rust_backend(db, backend, oid_type) }
    }

    /// `git_refdb_backend_fs` + `git_refdb_set_backend`: give the database
    /// a new files backend of its repository; as
    /// [`set_backend`](Self::set_backend), only for an unshared database.
    pub fn set_backend_fs(&mut self) -> Result<()> {
        let db = self.exclusive()?;
        let repo = db.repo().ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: as in `set_backend`.
        unsafe { install_fs_backend(db, repo) }
    }
}

// ---------------------------------------------------------------------------
// Iteration
// ---------------------------------------------------------------------------

/// The references of a repository (`git_reference_iterator_new`), loose
/// ones first (in path order), then the packed ones not shadowed.
pub struct References<'repo> {
    it: RefIter,
    _repo: PhantomData<&'repo Repository>,
}

impl core::fmt::Debug for References<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("References").finish_non_exhaustive()
    }
}

impl<'repo> References<'repo> {
    /// Every reference.
    pub fn new(repo: &'repo Repository) -> Result<References<'repo>> {
        Self::make(repo, None)
    }

    /// The references whose names match `glob`
    /// (`git_reference_iterator_glob_new`, `wildmatch`).
    pub fn glob(repo: &'repo Repository, glob: impl AsRef<[u8]>) -> Result<References<'repo>> {
        let g = cname(glob.as_ref())?;
        Self::make(repo, Some(&g))
    }

    fn make(repo: &'repo Repository, glob: Option<&CStr>) -> Result<References<'repo>> {
        Ok(References {
            it: rich(RefIter::new(repo.ptr(), glob))?,
            _repo: PhantomData,
        })
    }
}

impl<'repo> Iterator for References<'repo> {
    type Item = Result<Reference<'repo>>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.it.next_ref() {
            Ok(r) => Some(Ok(Reference::wrap(r))),
            Err(ErrorCode::ITER_OVER) => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}

/// The names of the references of a repository
/// (`git_reference_next_name`).
pub struct ReferenceNames<'repo> {
    it: RefIter,
    _repo: PhantomData<&'repo Repository>,
}

impl core::fmt::Debug for ReferenceNames<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReferenceNames").finish_non_exhaustive()
    }
}

impl<'repo> ReferenceNames<'repo> {
    /// Every reference's name.
    pub fn new(repo: &'repo Repository) -> Result<ReferenceNames<'repo>> {
        Ok(ReferenceNames {
            it: rich(RefIter::new(repo.ptr(), None))?,
            _repo: PhantomData,
        })
    }

    /// The names matching `glob`.
    pub fn glob(repo: &'repo Repository, glob: impl AsRef<[u8]>) -> Result<ReferenceNames<'repo>> {
        let g = cname(glob.as_ref())?;
        Ok(ReferenceNames {
            it: rich(RefIter::new(repo.ptr(), Some(&g)))?,
            _repo: PhantomData,
        })
    }
}

impl Iterator for ReferenceNames<'_> {
    type Item = Result<Vec<u8>>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.it.next_name() {
            Ok(n) => Some(Ok(n.to_bytes().to_vec())),
            Err(ErrorCode::ITER_OVER) => None,
            Err(e) => Some(Err(Error::from(e))),
        }
    }
}

// ---------------------------------------------------------------------------
// Reflogs
// ---------------------------------------------------------------------------

/// The committer of the C signature at `sig` (NULL: none).
fn committer_of(sig: *const git_signature) -> Option<Signature> {
    if sig.is_null() {
        return None;
    }
    // SAFETY: a valid signature (an entry's committer, alive with the
    // entry).
    Some(unsafe { Signature::dup_raw(sig) }.expect("out of memory duplicating a signature"))
}

/// The reflog of a reference (`git_reflog`), freed on drop. Index 0 is
/// the newest entry.
pub struct Reflog<'repo> {
    inner: rl::Reflog,
    _repo: PhantomData<&'repo Repository>,
}

/// One entry of a [`Reflog`].
#[derive(Clone, Copy)]
pub struct ReflogEntry<'a> {
    ptr: NonNull<rl::GitReflogEntry>,
    _log: PhantomData<&'a Reflog<'a>>,
}

impl<'a> ReflogEntry<'a> {
    fn get(&self) -> &'a rl::GitReflogEntry {
        // SAFETY: an entry of a reflog borrowed for `'a`, which changes
        // only through `&mut Reflog`.
        unsafe { self.ptr.as_ref() }
    }

    /// The previous value (`git_reflog_entry_id_old`).
    pub fn id_old(&self) -> Oid {
        self.get().oid_old
    }

    /// The new value (`git_reflog_entry_id_new`).
    pub fn id_new(&self) -> Oid {
        self.get().oid_cur
    }

    /// The message, if any (`git_reflog_entry_message`).
    pub fn message(&self) -> Option<&'a [u8]> {
        let m = self.get().msg;
        if m.is_null() {
            return None;
        }
        // SAFETY: the entry's NUL-terminated message, alive for `'a`.
        Some(unsafe { CStr::from_ptr(m) }.to_bytes())
    }

    /// The message, if there is one and it is UTF-8.
    pub fn message_str(&self) -> Option<&'a str> {
        self.message().and_then(|m| core::str::from_utf8(m).ok())
    }

    /// Who made the change (`git_reflog_entry_committer`).
    pub fn committer(&self) -> Option<SignatureRef<'a>> {
        let p = self.get().committer;
        // SAFETY: NULL or the entry's signature, whose strings live (and are
        // not written) as long as the reflog the entry borrows.
        unsafe { p.as_ref() }.map(|s| unsafe { SignatureRef::from_raw(s) })
    }
}

impl core::fmt::Debug for ReflogEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReflogEntry")
            .field("old", &self.id_old())
            .field("new", &self.id_new())
            .field("message", &self.message().map(Bytes))
            .finish()
    }
}

impl<'repo> Reflog<'repo> {
    /// `git_reflog_read`: the reflog of `name` (empty if it has none, in
    /// which case the filesystem backend creates the file).
    pub fn read(repo: &'repo Repository, name: impl AsRef<[u8]>) -> Result<Reflog<'repo>> {
        let n = cname(name.as_ref())?;
        let log = rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.reflog_read(&n)))?;
        // SAFETY: the backend's fresh reflog, ours.
        let inner = unsafe { rl::Reflog::from_raw(log.as_ptr()) }
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        Ok(Reflog {
            inner,
            _repo: PhantomData,
        })
    }

    /// The reference's name.
    pub fn name(&self) -> &[u8] {
        self.inner.name().to_bytes()
    }

    /// The number of entries (`git_reflog_entrycount`).
    pub fn len(&self) -> usize {
        // SAFETY: our live reflog.
        unsafe { (*self.inner.as_ptr()).entries.len() }
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entry at `idx` (0: the newest; `git_reflog_entry_byindex`).
    pub fn get(&self, idx: usize) -> Option<ReflogEntry<'_>> {
        // SAFETY: our live reflog.
        let e = unsafe { rl::entry_byindex(self.inner.as_ptr(), idx) };
        NonNull::new(e).map(|ptr| ReflogEntry {
            ptr,
            _log: PhantomData,
        })
    }

    /// The entries, newest first.
    pub fn iter(&self) -> impl Iterator<Item = ReflogEntry<'_>> + '_ {
        (0..self.len()).filter_map(move |i| self.get(i))
    }

    /// `git_reflog_append`: a new newest entry to `id` (the message's
    /// newlines become spaces; a NUL in it is refused). Kept in memory
    /// until [`write`](Self::write).
    pub fn append<'s>(
        &mut self,
        id: &Oid,
        committer: impl Into<SignatureRef<'s>>,
        message: Option<&[u8]>,
    ) -> Result<()> {
        let msg = cmsg(message)?;
        let committer = committer.into().to_signature()?;
        // SAFETY: our reflog, exclusively borrowed; a valid signature.
        rich(unsafe {
            rl::append(
                self.inner.as_ptr(),
                id,
                committer.as_raw(),
                msg.as_ref().map(|m| m.to_bytes()),
            )
        })
    }

    /// `git_reflog_drop`: remove entry `idx`; with `rewrite_previous`, keep
    /// the chain of old ids consistent.
    pub fn remove(&mut self, idx: usize, rewrite_previous: bool) -> Result<()> {
        // SAFETY: as above.
        rich(unsafe { rl::drop_entry(self.inner.as_ptr(), idx, rewrite_previous) })
    }

    /// `git_reflog_write`.
    pub fn write(&mut self) -> Result<()> {
        // SAFETY: our live reflog.
        let db = unsafe { (*self.inner.as_ptr()).db };
        let db =
            NonNull::new(db).ok_or_else(|| Error::from(errors::assert_arg_failed("reflog->db")))?;
        // SAFETY: the reflog's database, counted by it.
        let db = unsafe { RefdbPtr::new(db) };
        // SAFETY: our reflog.
        rich(unsafe { db.reflog_write(self.inner.as_ptr()) })
    }

    /// `git_reflog_rename`.
    pub fn rename(
        repo: &Repository,
        old_name: impl AsRef<[u8]>,
        new_name: impl AsRef<[u8]>,
    ) -> Result<()> {
        let (o, n) = (cname(old_name.as_ref())?, cname(new_name.as_ref())?);
        rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.reflog_rename(&o, &n)))
    }

    /// `git_reflog_delete`.
    pub fn delete(repo: &Repository, name: impl AsRef<[u8]>) -> Result<()> {
        let n = cname(name.as_ref())?;
        rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.reflog_delete(&n)))
    }
}

impl core::fmt::Debug for Reflog<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

// ---------------------------------------------------------------------------
// Backends written in Rust
// ---------------------------------------------------------------------------

/// The value of a reference in a [`RefdbBackend`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefValue {
    /// A direct reference, with its peeled target if known.
    Direct {
        /// The target.
        id: Oid,
        /// The peeled target.
        peel: Option<Oid>,
    },
    /// A symbolic reference.
    Symbolic(Vec<u8>),
}

/// One reflog entry of a [`RefdbBackend`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflogRecord {
    /// The previous value.
    pub old: Oid,
    /// The new value.
    pub new: Oid,
    /// Who made the change.
    pub committer: Signature,
    /// The message (cut at a NUL, if it has one, as C reads it).
    pub message: Option<Vec<u8>>,
}

/// What a [`RefdbBackend::lock`] returns: the backend's own record of the
/// lock, handed back to [`RefdbBackend::unlock`].
pub type RefdbLock = Box<dyn core::any::Any + Send>;

/// How a transaction ends for a reference a [`RefdbBackend`] locked
/// (`git_refdb_backend.unlock`'s `success`).
#[derive(Debug)]
#[non_exhaustive]
pub enum RefdbUnlock<'a> {
    /// The transaction leaves the reference as it was (`success == 0`).
    Discard,
    /// Write the reference (`success == 1`).
    Write {
        /// The name.
        name: &'a [u8],
        /// The new value.
        value: RefValue,
        /// Whether to append to its reflog.
        update_reflog: bool,
        /// Who made the change, if the transaction says.
        committer: Option<SignatureRef<'a>>,
        /// The reflog message.
        message: Option<&'a [u8]>,
    },
    /// Delete the reference (`success == 2`).
    Delete {
        /// The name.
        name: &'a [u8],
    },
}

/// The error of a [`RefdbBackend`] method that is not implemented.
fn unsupported(what: &str) -> Error {
    Error::with(
        ErrorCode::GENERIC,
        ErrorClass::REFERENCE,
        format!("the reference database backend does not support {what}"),
    )
}

/// A reference database backend written in Rust (`git_refdb_backend`),
/// installed with [`Repository::set_refdb_backend`] or
/// [`Refdb::set_backend`].
///
/// libgit2 may call it from several threads at once, and re-entrantly
/// (writing a reference may look references up): it takes `&self`, and is
/// `Send + Sync`. A panic in a method is caught and becomes an error of
/// class [`ErrorClass::INTERNAL`]. Every method has a default, so that the
/// trait can grow: by default the backend is empty and read-only. Its
/// `Drop` may panic: the panic is caught (and its payload leaked), as C
/// frees a backend where it cannot unwind.
///
/// Names and values it returns must not contain NUL (an error of code
/// [`ErrorCode::INVALID_SPEC`] otherwise). Errors with code
/// [`ErrorCode::NOT_FOUND`] mean "no such reference"; a success code or
/// [`ErrorCode::ITER_OVER`] in an error becomes a plain failure.
pub trait RefdbBackend: Send + Sync + 'static {
    /// Whether `name` exists; by default, `false`.
    fn exists(&self, name: &[u8]) -> core::result::Result<bool, Error> {
        let _ = name;
        Ok(false)
    }
    /// The value of `name`, `None` if there is none; by default, `None`.
    fn lookup(&self, name: &[u8]) -> core::result::Result<Option<RefValue>, Error> {
        let _ = name;
        Ok(None)
    }
    /// Every reference, in the order iteration returns them (the glob
    /// filtering is done for the backend); by default, none.
    fn list(&self) -> core::result::Result<Vec<(Vec<u8>, RefValue)>, Error> {
        Ok(Vec::new())
    }
    /// Write `name`. Unless `force`, it must not exist; with `old_id` /
    /// `old_target`, it must currently have that value
    /// ([`ErrorCode::MODIFIED`] otherwise). By default, an error.
    fn write(
        &self,
        name: &[u8],
        value: &RefValue,
        force: bool,
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
        message: Option<&[u8]>,
    ) -> core::result::Result<(), Error> {
        let _ = (name, value, force, old_id, old_target, message);
        Err(unsupported("writing references"))
    }
    /// Rename `old_name` to `new_name`, returning its value; by default,
    /// an error.
    fn rename(
        &self,
        old_name: &[u8],
        new_name: &[u8],
        force: bool,
    ) -> core::result::Result<RefValue, Error> {
        let _ = (old_name, new_name, force);
        Err(unsupported("renaming references"))
    }
    /// Delete `name` (and its reflog), with the same conditions as
    /// [`write`](Self::write); by default, an error.
    fn delete(
        &self,
        name: &[u8],
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> core::result::Result<(), Error> {
        let _ = (name, old_id, old_target);
        Err(unsupported("deleting references"))
    }
    /// Optimise storage (`git_refdb_compress`); by default, nothing.
    fn compress(&self) -> core::result::Result<(), Error> {
        Ok(())
    }
    /// Whether `name` has a reflog; by default, `false`.
    fn has_log(&self, name: &[u8]) -> bool {
        let _ = name;
        false
    }
    /// Make sure `name` has a reflog; by default, nothing.
    fn ensure_log(&self, name: &[u8]) -> core::result::Result<(), Error> {
        let _ = name;
        Ok(())
    }
    /// The reflog of `name`, oldest first; by default, empty.
    fn reflog_read(&self, name: &[u8]) -> core::result::Result<Vec<ReflogRecord>, Error> {
        let _ = name;
        Ok(Vec::new())
    }
    /// Replace the reflog of `name`; by default, nothing.
    fn reflog_write(
        &self,
        name: &[u8],
        entries: &[ReflogRecord],
    ) -> core::result::Result<(), Error> {
        let _ = (name, entries);
        Ok(())
    }
    /// Rename a reflog; by default, nothing.
    fn reflog_rename(&self, old_name: &[u8], new_name: &[u8]) -> core::result::Result<(), Error> {
        let _ = (old_name, new_name);
        Ok(())
    }
    /// Delete a reflog; by default, nothing.
    fn reflog_delete(&self, name: &[u8]) -> core::result::Result<(), Error> {
        let _ = name;
        Ok(())
    }
    /// Lock `name` for a transaction (`git_transaction_lock_ref`, which
    /// stays C): what the backend returns comes back to
    /// [`unlock`](Self::unlock), once. By default, locking is not
    /// supported (the error C gives for a backend without `lock`).
    fn lock(&self, name: &[u8]) -> core::result::Result<RefdbLock, Error> {
        let _ = name;
        Err(Error::with(
            ErrorCode::GENERIC,
            ErrorClass::REFERENCE,
            "backend does not support locking",
        ))
    }
    /// End the transaction on a reference [`lock`](Self::lock) locked:
    /// write it, delete it, or leave it. By default, nothing.
    fn unlock(&self, lock: RefdbLock, how: RefdbUnlock<'_>) -> core::result::Result<(), Error> {
        let _ = (lock, how);
        Ok(())
    }
}

/// The state behind a Rust backend's table.
pub(crate) struct RustBackend<B>(crate::api::NoUnwindDrop<B>, crate::oid::OidType);

fn fail(e: &Error) -> c_int {
    e.set_last().raw()
}

fn status<T>(r: core::result::Result<T, Error>) -> c_int {
    match r {
        Ok(_) => 0,
        Err(e) => fail(&e),
    }
}

fn busy() -> c_int {
    let _ = errors::set_str(
        ErrorClass::REFERENCE,
        "the reference database is being freed",
    );
    -1
}

fn panicked() -> c_int {
    errors::set_panic_error();
    -1
}

/// # Safety
///
/// `p` is NULL or a NUL-terminated string.
unsafe fn bytes_of<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: per the contract.
    unsafe { crate::util::export::cstr_bytes(p) }
}

/// # Safety
///
/// `p` is NULL or a NUL-terminated string.
unsafe fn opt_bytes<'a>(p: *const c_char) -> Option<&'a [u8]> {
    // SAFETY: per the contract.
    unsafe { crate::util::export::cstr(p) }.map(CStr::to_bytes)
}

/// A name or target from a Rust backend: a NUL would cut it short in C
/// (S-6e), so it is refused.
fn check_no_nul(b: &[u8], what: &str) -> errors::Result<()> {
    if b.contains(&0) {
        let _ = crate::git_error_set!(
            ErrorClass::REFERENCE,
            "the {what} '{}' from the backend contains a NUL byte",
            Bytes(&b[..memchr::memchr(0, b).unwrap_or(b.len())])
        );
        return Err(ErrorCode::INVALID_SPEC);
    }
    Ok(())
}

fn make_ref(name: &[u8], v: &RefValue) -> errors::Result<RefBox> {
    check_no_nul(name, "reference name")?;
    if let RefValue::Symbolic(t) = v {
        check_no_nul(t, "symbolic target")?;
    }
    match v {
        RefValue::Direct { id, peel } => RefBox::new_direct(name, id, peel.as_ref()),
        RefValue::Symbolic(t) => RefBox::new_symbolic(name, t),
    }
}

fn value_of(r: RefView<'_>) -> RefValue {
    if r.raw_kind() == GIT_REFERENCE_SYMBOLIC {
        RefValue::Symbolic(r.symbolic_target().unwrap_or_default().to_vec())
    } else {
        let peel = (!r.peel().is_zero()).then(|| *r.peel());
        RefValue::Direct {
            id: *r.oid_field(),
            peel,
        }
    }
}

/// The iterator of a Rust backend: a snapshot of its references.
pub(crate) struct VecIter {
    pool: GitPool,
    items: Vec<(NonNull<c_char>, RefValue)>,
    pos: usize,
}

// SAFETY: the names are blocks of the iterator's own pool, read-only.
unsafe impl Send for VecIter {}
// SAFETY: as above.
unsafe impl Sync for VecIter {}

impl VecIter {
    fn name(&self, i: usize) -> &CStr {
        // SAFETY: a string of our pool, alive while we are.
        unsafe { CStr::from_ptr(self.items[i].0.as_ptr()) }
    }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a `git_refdb_backend` its
    // backend in the argument marked `@`, and calls only the slots set
    // here (`free` once; `unlock` once per `lock` that succeeded, with
    // the payload it made); `init` is NULL, which the header allows.
    unsafe impl<B: RefdbBackend> CVtable<RustBackend<B>> for RefdbBackendTable {}
    base = git_refdb_backend { version: GIT_REFDB_BACKEND_VERSION, ..Default::default() };
    fn exists(out: *mut c_int, @backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: `name` is a string.
        |s: &RustBackend<B>| match s.0.exists(unsafe { bytes_of(name) }) {
            Ok(e) => {
                // SAFETY: `out` is writable.
                unsafe { *out = c_int::from(e) };
                0
            }
            Err(e) => fail(&e),
        },
    }
    fn lookup(out: *mut *mut GitReference, @backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: `name` is a string.
            let n = unsafe { bytes_of(name) };
            match s.0.lookup(n) {
                Ok(Some(v)) => match make_ref(n, &v) {
                    Ok(r) => {
                        // SAFETY: `out` is writable.
                        unsafe { *out = r.into_raw() };
                        0
                    }
                    Err(e) => e.raw(),
                },
                Ok(None) => {
                    let _ = crate::git_error_set!(ErrorClass::REFERENCE, "reference '{}' not found", Bytes(n));
                    ErrorCode::NOT_FOUND.raw()
                }
                Err(e) => fail(&e),
            }
        },
    }
    fn iterator(out: *mut *mut git_reference_iterator, @backend, glob: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            let list = match s.0.list() {
                Ok(l) => l,
                Err(e) => return fail(&e),
            };
            // SAFETY: `glob` is NULL or a string.
            let glob = unsafe { opt_bytes(glob) };
            let mut it = match GitPool::with_item_size(1) {
                Ok(pool) => VecIter { pool, items: Vec::new(), pos: 0 },
                Err(e) => return e.raw(),
            };
            for (name, v) in list {
                if let Err(e) = check_no_nul(&name, "reference name") {
                    return e.raw();
                }
                if let RefValue::Symbolic(t) = &v
                    && let Err(e) = check_no_nul(t, "symbolic target")
                {
                    return e.raw();
                }
                if glob.is_some_and(|g| !wildmatch(g, &name, WildmatchFlags::default()).is_match()) {
                    continue;
                }
                match it.pool.strndup(&name) {
                    Ok(n) => it.items.push((n, v)),
                    Err(e) => return e.raw(),
                }
            }
            let t = CTable::<RefIteratorTable, VecIter>::new(it);
            // SAFETY: `out` is writable.
            unsafe { *out = t.into_c().as_ptr() };
            0
        },
    }
    fn write(
        @backend,
        r: *const GitReference,
        force: c_int,
        _who: *const git_signature,
        message: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: `r` is a live reference.
            let v = unsafe { RefView::from_raw(r) };
            // SAFETY: NULL or strings.
            let (msg, ot) = (unsafe { opt_bytes(message) }, unsafe { opt_bytes(old_target) });
            // SAFETY: NULL or a valid oid.
            let oid = unsafe { old_id.as_ref() };
            status(s.0.write(v.name(), &value_of(v), force != 0, oid, ot, msg))
        },
    }
    fn rename(
        out: *mut *mut GitReference,
        @backend,
        old_name: *const c_char,
        new_name: *const c_char,
        force: c_int,
        _who: *const git_signature,
        _message: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: strings.
            let (o, n) = (unsafe { bytes_of(old_name) }, unsafe { bytes_of(new_name) });
            match s.0.rename(o, n, force != 0) {
                Ok(v) => match make_ref(n, &v) {
                    Ok(r) => {
                        if !out.is_null() {
                            // SAFETY: NULL or writable.
                            unsafe { *out = r.into_raw() };
                        }
                        0
                    }
                    Err(e) => e.raw(),
                },
                Err(e) => fail(&e),
            }
        },
    }
    fn del(@backend, name: *const c_char, old_id: *const Oid, old_target: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: a string; the others NULL or valid.
            let (n, old, ot) = (unsafe { bytes_of(name) }, unsafe { old_id.as_ref() }, unsafe { opt_bytes(old_target) });
            status(s.0.delete(n, old, ot))
        },
    }
    fn compress(@backend) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| status(s.0.compress()),
    }
    fn has_log(@backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: a string.
        |s: &RustBackend<B>| c_int::from(s.0.has_log(unsafe { bytes_of(name) })),
    }
    fn ensure_log(@backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: a string.
        |s: &RustBackend<B>| status(s.0.ensure_log(unsafe { bytes_of(name) })),
    }
    fn reflog_read(out: *mut *mut GitReflog, @backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: a string.
            let n = unsafe { crate::util::export::cstr(name) }.unwrap_or(c"");
            let records = match s.0.reflog_read(n.to_bytes()) {
                Ok(r) => r,
                Err(e) => return fail(&e),
            };
            let log = match rl::Reflog::alloc(n, s.1 as c_int) {
                Ok(l) => l,
                Err(e) => return e.raw(),
            };
            for rec in &records {
                // SAFETY: our reflog; a valid signature.
                if let Err(e) = unsafe { rl::append(log.as_ptr(), &rec.new, rec.committer.as_raw(), rec.message.as_deref()) } {
                    return e.raw();
                }
                // `append` chains the old id; keep the record's.
                // SAFETY: our reflog.
                let e = unsafe { rl::entry_byindex(log.as_ptr(), 0) };
                // SAFETY: the entry just appended, of our reflog.
                unsafe { (*e).oid_old = rec.old };
            }
            // SAFETY: `out` is writable.
            unsafe { *out = log.into_raw().as_ptr() };
            0
        },
    }
    fn reflog_write(@backend, log: *mut GitReflog) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: a live reflog for the call.
            let (name, entries) = (unsafe { rl::name_of(log) }, unsafe { rl::entries_of(log) });
            let mut records = Vec::new();
            for e in entries {
                // SAFETY: a live entry of the reflog.
                let e = unsafe { &*e };
                // SAFETY: NULL or the entry's message.
                let message = unsafe { opt_bytes(e.msg) }.map(<[u8]>::to_vec);
                records.push(ReflogRecord {
                    old: e.oid_old,
                    new: e.oid_cur,
                    committer: match committer_of(e.committer) {
                        Some(c) => c,
                        None => Signature::empty().expect("out of memory"),
                    },
                    message,
                });
            }
            status(s.0.reflog_write(name.to_bytes(), &records))
        },
    }
    fn reflog_rename(@backend, old_name: *const c_char, new_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: strings.
            let (o, n) = (unsafe { bytes_of(old_name) }, unsafe { bytes_of(new_name) });
            status(s.0.reflog_rename(o, n))
        },
    }
    fn reflog_delete(@backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: a string.
        |s: &RustBackend<B>| status(s.0.reflog_delete(unsafe { bytes_of(name) })),
    }
    fn lock(payload_out: *mut *mut c_void, @backend, name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            // SAFETY: a string.
            match s.0.lock(unsafe { bytes_of(name) }) {
                Ok(lock) => {
                    let p = Box::into_raw(Box::new(lock));
                    // SAFETY: `payload_out` is writable; C hands the
                    // payload back to `unlock` once.
                    unsafe { payload_out.write(p.cast()) };
                    0
                }
                Err(e) => fail(&e),
            }
        },
    }
    fn unlock(
        @backend,
        payload: *mut c_void,
        success: c_int,
        update_reflog: c_int,
        r: *const GitReference,
        sig: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &RustBackend<B>| {
            if payload.is_null() {
                return errors::assert_arg_failed("payload").raw();
            }
            // SAFETY: the payload our `lock` made (`Box::into_raw`), which
            // C hands back once.
            let lock = *unsafe { Box::from_raw(payload.cast::<RefdbLock>()) };
            let how = if success == 0 || r.is_null() {
                RefdbUnlock::Discard
            } else {
                // SAFETY: a live reference for the call.
                let v = unsafe { RefView::from_raw(r) };
                if success == 2 {
                    RefdbUnlock::Delete { name: v.name() }
                } else {
                    RefdbUnlock::Write {
                        name: v.name(),
                        value: value_of(v),
                        update_reflog: update_reflog != 0,
                        // SAFETY: NULL or a live signature for the call.
                        committer: unsafe { sig.as_ref() }
                            .map(|g| unsafe { SignatureRef::from_raw(g) }),
                        // SAFETY: NULL or a string.
                        message: unsafe { opt_bytes(message) },
                    }
                }
            };
            status(s.0.unlock(lock, how))
        },
    }
    release free;
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a `git_reference_iterator` the
    // iterator, and calls `next`, `next_name` and `free` (once), all set.
    unsafe impl CVtable<VecIter> for RefIteratorTable {}
    base = git_reference_iterator::default();
    fn next(out: *mut *mut GitReference, @iter) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &mut VecIter| {
            if s.pos >= s.items.len() {
                return ErrorCode::ITER_OVER.raw();
            }
            let i = s.pos;
            s.pos += 1;
            match make_ref(s.name(i).to_bytes(), &s.items[i].1) {
                Ok(r) => {
                    // SAFETY: `out` is writable.
                    unsafe { *out = r.into_raw() };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    fn next_name(out: *mut *const c_char, @iter) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &mut VecIter| {
            if s.pos >= s.items.len() {
                return ErrorCode::ITER_OVER.raw();
            }
            let n = s.items[s.pos].0;
            s.pos += 1;
            // SAFETY: `out` is writable; the name lives in our pool until
            // the iterator is freed.
            unsafe { *out = n.as_ptr() };
            0
        },
    }
    release free;
}

#[cfg(test)]
// The test calls the tables' slots in sequence, as libgit2 would.
#[allow(clippy::multiple_unsafe_ops_per_block)]
mod tests {
    //! The Rust backend's tables, called as libgit2 calls them (Miri).
    use super::*;
    use crate::util::test_support;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mem(Mutex<Vec<(Vec<u8>, RefValue)>>);

    impl RefdbBackend for Mem {
        fn exists(&self, name: &[u8]) -> Result<bool> {
            Ok(self.0.lock().unwrap().iter().any(|(n, _)| n == name))
        }
        fn lookup(&self, name: &[u8]) -> Result<Option<RefValue>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone()))
        }
        fn list(&self) -> Result<Vec<(Vec<u8>, RefValue)>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn write(
            &self,
            name: &[u8],
            value: &RefValue,
            _force: bool,
            _old_id: Option<&Oid>,
            _old_target: Option<&[u8]>,
            _message: Option<&[u8]>,
        ) -> Result<()> {
            self.0.lock().unwrap().push((name.to_vec(), value.clone()));
            Ok(())
        }
        fn rename(&self, _old: &[u8], _new: &[u8], _force: bool) -> Result<RefValue> {
            Err(Error::with(
                ErrorCode::NOT_FOUND,
                ErrorClass::REFERENCE,
                "nope",
            ))
        }
        fn delete(&self, name: &[u8], _: Option<&Oid>, _: Option<&[u8]>) -> Result<()> {
            if name == b"panic" {
                panic!("delete");
            }
            self.0.lock().unwrap().retain(|(n, _)| n != name);
            Ok(())
        }
    }

    #[test]
    fn rust_backend_slots() {
        let _g = test_support::init();
        let id = Oid::parse_hex(
            b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            crate::OidType::Sha1,
        )
        .unwrap();
        let t = CTable::<RefdbBackendTable, RustBackend<Mem>>::new(RustBackend(
            crate::api::NoUnwindDrop::new(Mem::default()),
            crate::OidType::Sha1,
        ));
        let b = t.into_c().as_ptr();
        // SAFETY: libgit2's contract with a backend: its own slots, its
        // table, valid arguments, `free` once.
        unsafe {
            let tb = &*b;
            assert!(tb.lock.is_some() && tb.unlock.is_some() && tb.init.is_none());
            let r = RefBox::new_direct(b"refs/heads/a", &id, None).unwrap();
            assert_eq!(
                (tb.write.unwrap())(
                    b,
                    r.as_ptr(),
                    0,
                    core::ptr::null(),
                    core::ptr::null(),
                    core::ptr::null(),
                    core::ptr::null()
                ),
                0
            );
            let s = RefBox::new_symbolic(b"HEAD", b"refs/heads/a").unwrap();
            assert_eq!(
                (tb.write.unwrap())(
                    b,
                    s.as_ptr(),
                    0,
                    core::ptr::null(),
                    c"m".as_ptr(),
                    core::ptr::null(),
                    core::ptr::null()
                ),
                0
            );
            let mut e = 0;
            assert_eq!((tb.exists.unwrap())(&raw mut e, b, c"HEAD".as_ptr()), 0);
            assert_eq!(e, 1);
            let mut out = core::ptr::null_mut();
            assert_eq!(
                (tb.lookup.unwrap())(&raw mut out, b, c"refs/heads/a".as_ptr()),
                0
            );
            let got = RefBox::from_raw(out).unwrap();
            assert_eq!(got.view().target(), Some(&id));
            assert_eq!(
                (tb.lookup.unwrap())(&raw mut out, b, c"refs/heads/z".as_ptr()),
                -3
            );
            assert_eq!(
                errors::last_message().unwrap(),
                b"reference 'refs/heads/z' not found"
            );
            assert_eq!(
                (tb.rename.unwrap())(
                    &raw mut out,
                    b,
                    c"a".as_ptr(),
                    c"b".as_ptr(),
                    0,
                    core::ptr::null(),
                    core::ptr::null()
                ),
                -3
            );
            assert_eq!(errors::last_message().unwrap(), b"nope");
            assert_eq!(
                (tb.del.unwrap())(b, c"panic".as_ptr(), core::ptr::null(), core::ptr::null()),
                -1
            );
            assert_eq!(errors::last_class(), Some(ErrorClass::INTERNAL));
            assert_eq!((tb.compress.unwrap())(b), 0);
            assert_eq!((tb.has_log.unwrap())(b, c"HEAD".as_ptr()), 0);

            let mut it: *mut git_reference_iterator = core::ptr::null_mut();
            assert_eq!(
                (tb.iterator.unwrap())(&raw mut it, b, c"refs/*".as_ptr()),
                0
            );
            let mut name = core::ptr::null();
            assert_eq!(((*it).next_name.unwrap())(&raw mut name, it), 0);
            assert_eq!(CStr::from_ptr(name).to_bytes(), b"refs/heads/a");
            assert_eq!(
                ((*it).next.unwrap())(&raw mut out, it),
                ErrorCode::ITER_OVER.raw()
            );
            // The name stays valid until the iterator is freed.
            assert_eq!(CStr::from_ptr(name).to_bytes(), b"refs/heads/a");
            ((*it).free.unwrap())(it);
            assert_eq!((tb.iterator.unwrap())(&raw mut it, b, core::ptr::null()), 0);
            assert_eq!(((*it).next.unwrap())(&raw mut out, it), 0);
            drop(RefBox::from_raw(out));
            ((*it).free.unwrap())(it);

            assert_eq!(
                (tb.del.unwrap())(b, c"HEAD".as_ptr(), core::ptr::null(), core::ptr::null()),
                0
            );
            assert_eq!((tb.exists.unwrap())(&raw mut e, b, c"HEAD".as_ptr()), 0);
            assert_eq!(e, 0);
            (tb.free.unwrap())(b);
        }
    }

    /// A table pointer handed to the test's threads.
    #[derive(Clone, Copy)]
    struct SendPtr(*mut git_refdb_backend);
    // SAFETY: the table's state is `Send + Sync` (the slots it is used
    // through are shared ones).
    unsafe impl Send for SendPtr {}
    // SAFETY: as above.
    unsafe impl Sync for SendPtr {}
    impl SendPtr {
        fn get(self) -> *mut git_refdb_backend {
            self.0
        }
    }

    /// Two threads in the slots of one table at once, as libgit2's
    /// threads do (`tests/libgit2/threads/refdb.c`).
    #[test]
    fn rust_backend_two_threads() {
        let _g = test_support::init();
        let id = Oid::parse_hex(
            b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            crate::OidType::Sha1,
        )
        .unwrap();
        let t = CTable::<RefdbBackendTable, RustBackend<Mem>>::new(RustBackend(
            crate::api::NoUnwindDrop::new(Mem::default()),
            crate::OidType::Sha1,
        ));
        let b = SendPtr(t.into_c().as_ptr());
        let n = if cfg!(miri) { 4 } else { 200 };
        std::thread::scope(|s| {
            for k in 0..2 {
                s.spawn(move || {
                    let b = b.get();
                    let name = std::ffi::CString::new(format!("refs/heads/t{k}")).unwrap();
                    for _ in 0..n {
                        // SAFETY: a live table (freed after the scope), its
                        // slots called as libgit2 calls them.
                        unsafe {
                            let tb = &*b;
                            let r = RefBox::new_direct(name.to_bytes(), &id, None).unwrap();
                            let w = (tb.write.unwrap())(
                                b,
                                r.as_ptr(),
                                1,
                                core::ptr::null(),
                                core::ptr::null(),
                                core::ptr::null(),
                                core::ptr::null(),
                            );
                            assert_eq!(w, 0);
                            let mut e = 0;
                            assert_eq!((tb.exists.unwrap())(&raw mut e, b, name.as_ptr()), 0);
                            assert_eq!(e, 1);
                            let mut out = core::ptr::null_mut();
                            assert_eq!((tb.lookup.unwrap())(&raw mut out, b, name.as_ptr()), 0);
                            drop(RefBox::from_raw(out));
                            let mut it: *mut git_reference_iterator = core::ptr::null_mut();
                            assert_eq!(
                                (tb.iterator.unwrap())(&raw mut it, b, core::ptr::null()),
                                0
                            );
                            let mut nm = core::ptr::null();
                            while ((*it).next_name.unwrap())(&raw mut nm, it) == 0 {
                                assert!(!CStr::from_ptr(nm).to_bytes().is_empty());
                            }
                            ((*it).free.unwrap())(it);
                            assert_eq!(
                                (tb.del.unwrap())(
                                    b,
                                    name.as_ptr(),
                                    core::ptr::null(),
                                    core::ptr::null()
                                ),
                                0
                            );
                        }
                    }
                });
            }
        });
        let b = b.get();
        // SAFETY: the table, released once, no slot running.
        unsafe { ((*b).free.unwrap())(b) };
    }

    /// A backend whose slot releases its own table (C freeing the backend
    /// from inside a call): the release is deferred until the slot exits.
    struct Reentrant {
        table: std::sync::atomic::AtomicPtr<git_refdb_backend>,
        dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for Reentrant {
        fn drop(&mut self) {
            self.dropped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl RefdbBackend for Reentrant {
        fn exists(&self, _: &[u8]) -> Result<bool> {
            let t = self.table.load(std::sync::atomic::Ordering::SeqCst);
            // SAFETY: our own live table, released once (here).
            unsafe { ((*t).free.unwrap())(t) };
            // Still alive: the release waits for this slot to exit.
            assert!(!self.dropped.load(std::sync::atomic::Ordering::SeqCst));
            Ok(true)
        }
        fn lookup(&self, _: &[u8]) -> Result<Option<RefValue>> {
            Ok(None)
        }
        fn list(&self) -> Result<Vec<(Vec<u8>, RefValue)>> {
            Ok(Vec::new())
        }
        fn write(
            &self,
            _: &[u8],
            _: &RefValue,
            _: bool,
            _: Option<&Oid>,
            _: Option<&[u8]>,
            _: Option<&[u8]>,
        ) -> Result<()> {
            Ok(())
        }
        fn rename(&self, _: &[u8], _: &[u8], _: bool) -> Result<RefValue> {
            Ok(RefValue::Symbolic(b"x".to_vec()))
        }
        fn delete(&self, _: &[u8], _: Option<&Oid>, _: Option<&[u8]>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rust_backend_reentrant_free() {
        let _g = test_support::init();
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let st = Reentrant {
            table: std::sync::atomic::AtomicPtr::new(core::ptr::null_mut()),
            dropped: dropped.clone(),
        };
        let t = CTable::<RefdbBackendTable, RustBackend<Reentrant>>::new(RustBackend(
            crate::api::NoUnwindDrop::new(st),
            crate::OidType::Sha1,
        ));
        // Tell the state where its table is.
        t.state()
            .unwrap()
            .0
            .table
            .store(t.as_c_ptr(), std::sync::atomic::Ordering::SeqCst);
        let b = t.into_c().as_ptr();
        let mut e = 0;
        // SAFETY: the table's own slot; it releases the table inside.
        let r = unsafe { ((*b).exists.unwrap())(&raw mut e, b, c"x".as_ptr()) };
        assert_eq!(r, 0);
        assert_eq!(e, 1);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }
}

/// The transaction hooks of a Rust backend ([`RefdbBackend::lock`] /
/// [`RefdbBackend::unlock`]), driven by C's `git_transaction` (which stays
/// C, and has no safe counterpart).
#[cfg(all(test, feature = "link-c", not(miri)))]
#[allow(clippy::multiple_unsafe_ops_per_block)]
mod transaction_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// `git_transaction`, opaque.
    #[repr(C)]
    struct Tx {
        _p: [u8; 0],
    }

    unsafe extern "C" {
        fn git_transaction_new(out: *mut *mut Tx, repo: *mut c_void) -> c_int;
        fn git_transaction_lock_ref(tx: *mut Tx, name: *const c_char) -> c_int;
        fn git_transaction_set_target(
            tx: *mut Tx,
            name: *const c_char,
            target: *const Oid,
            sig: *const git_signature,
            msg: *const c_char,
        ) -> c_int;
        fn git_transaction_remove(tx: *mut Tx, name: *const c_char) -> c_int;
        fn git_transaction_commit(tx: *mut Tx) -> c_int;
        fn git_transaction_free(tx: *mut Tx);
    }

    /// What the backend saw.
    #[derive(Default)]
    struct Seen(Mutex<Vec<String>>);

    struct Locking(Arc<Seen>);

    impl RefdbBackend for Locking {
        fn lock(&self, name: &[u8]) -> Result<RefdbLock> {
            let n = String::from_utf8_lossy(name).into_owned();
            self.0.0.lock().unwrap().push(format!("lock {n}"));
            Ok(Box::new(n))
        }
        fn unlock(&self, lock: RefdbLock, how: RefdbUnlock<'_>) -> Result<()> {
            let n = lock.downcast::<String>().unwrap();
            let what = match how {
                RefdbUnlock::Discard => "discard".to_owned(),
                RefdbUnlock::Delete { name } => format!("delete {}", Bytes(name)),
                RefdbUnlock::Write {
                    name,
                    value,
                    update_reflog,
                    committer,
                    message,
                } => format!(
                    "write {} {:?} {update_reflog} {:?} {:?}",
                    Bytes(name),
                    value,
                    committer.map(|c| c.name().to_vec()),
                    message.map(Bytes)
                ),
            };
            self.0.0.lock().unwrap().push(format!("unlock {n}: {what}"));
            Ok(())
        }
    }

    struct NoLocks;
    impl RefdbBackend for NoLocks {}

    fn repo() -> (Repository, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!(
            "refs-tx-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        (Repository::init(&d, true).unwrap(), d)
    }

    #[test]
    fn transactions_reach_the_backend() {
        let (mut repo, dir) = repo();
        let seen = Arc::new(Seen::default());
        repo.set_refdb_backend(Locking(seen.clone())).unwrap();
        let id = Oid::from_hex(
            "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            crate::OidType::Sha1,
        )
        .unwrap();
        let who = Signature::new(b"me", b"me@x", crate::Time::new(0, 0)).unwrap();
        let mut tx = core::ptr::null_mut();
        // SAFETY: C's transaction API, on a live repository, with live
        // arguments; the transaction freed once.
        unsafe {
            assert_eq!(git_transaction_new(&raw mut tx, repo.raw_ptr().cast()), 0);
            assert_eq!(git_transaction_lock_ref(tx, c"refs/heads/a".as_ptr()), 0);
            assert_eq!(git_transaction_lock_ref(tx, c"refs/heads/b".as_ptr()), 0);
            assert_eq!(git_transaction_lock_ref(tx, c"refs/heads/c".as_ptr()), 0);
            assert_eq!(
                git_transaction_set_target(
                    tx,
                    c"refs/heads/a".as_ptr(),
                    &raw const id,
                    who.as_raw(),
                    c"moved".as_ptr()
                ),
                0
            );
            assert_eq!(git_transaction_remove(tx, c"refs/heads/b".as_ptr()), 0);
            assert_eq!(git_transaction_commit(tx), 0);
            git_transaction_free(tx);
        }
        let seen = seen.0.lock().unwrap().clone();
        assert_eq!(
            &seen[..3],
            [
                "lock refs/heads/a",
                "lock refs/heads/b",
                "lock refs/heads/c"
            ]
        );
        let mut rest = seen[3..].to_vec();
        rest.sort();
        assert_eq!(
            rest,
            [
                format!(
                    "unlock refs/heads/a: write refs/heads/a Direct {{ id: {id:?}, peel: None }} true Some([109, 101]) Some(\"moved\")"
                ),
                "unlock refs/heads/b: delete refs/heads/b".to_owned(),
                "unlock refs/heads/c: discard".to_owned(),
            ]
        );
        drop(repo);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backend_without_locks_refuses_transactions() {
        let (mut repo, dir) = repo();
        repo.set_refdb_backend(NoLocks).unwrap();
        let mut tx = core::ptr::null_mut();
        // SAFETY: as above.
        unsafe {
            assert_eq!(git_transaction_new(&raw mut tx, repo.raw_ptr().cast()), 0);
            assert_eq!(git_transaction_lock_ref(tx, c"refs/heads/a".as_ptr()), -1);
            git_transaction_free(tx);
        }
        assert_eq!(
            errors::last_message().unwrap(),
            b"backend does not support locking"
        );
        assert_eq!(errors::last_class(), Some(ErrorClass::REFERENCE));
        drop(repo);
        let _ = std::fs::remove_dir_all(dir);
    }
}
