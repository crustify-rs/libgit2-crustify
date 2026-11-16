//! The safe Rust API of references, the reference database and reflogs
//! (PORTING.md §5): [`Reference`], [`ReferenceType`], [`References`] and
//! [`ReferenceNames`] (with globs), [`Reflog`] and [`ReflogEntry`], and
//! [`RefdbBackend`], a reference database backend written in Rust.
//!
//! It is a thin layer over the same Rust as the C ABI (`refs`, `refdb`,
//! `refdb_fs`, `reflog`). No C type appears in it.
//!
//! The repository it works on is the crate's [`Repository`] (`Sync`: the
//! reference API may be used from several threads on one repository); the
//! committer of reflog entries is a [`Committer`] until WP l-obj's
//! `Signature` lands (PENDING(l-obj)).

use core::ffi::{CStr, c_char, c_int};
use core::marker::PhantomData;
use core::ptr::NonNull;
use std::ffi::CString;

use ffibox::vtable::CTable;

use crate::oid::Oid;
use crate::refdb::{
    self, GIT_REFDB_BACKEND_VERSION, RefIteratorTable, RefdbBackendTable, RefdbPtr,
    git_refdb_backend, git_reference_iterator,
};
use crate::reflog::{self as rl, GitReflog};
pub use crate::refs::ReferenceType;
use crate::refs::{
    self, GIT_REFERENCE_DIRECT, GIT_REFERENCE_SYMBOLIC, GitReference, RefBox, RefIter, RefView,
};
use crate::sys::pending::{RepoPtr, SigBox, git_signature};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode};
use crate::util::fs_path::to_cstring;
use crate::util::pool::GitPool;
use crate::util::wildmatch::{WildmatchFlags, wildmatch};

type Result<T> = core::result::Result<T, Error>;

fn rich<T>(r: errors::Result<T>) -> Result<T> {
    r.map_err(Error::from)
}

/// A name as a C string: an interior NUL makes it an invalid name.
fn cname(name: &[u8]) -> Result<CString> {
    CString::new(name).map_err(|_| {
        Error::new(
            ErrorCode::INVALID_SPEC,
            ErrorClass::REFERENCE,
            format!("the given reference name '{}' is not valid", Bytes(name)),
        )
    })
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
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// A reference of a repository (`git_reference`), freed on drop.
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

/// What a reference can be peeled to (`git_object_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeelTarget {
    /// Whatever the reference ends at, but a tag (`GIT_OBJECT_ANY`).
    Any,
    /// A commit.
    Commit,
    /// A tree.
    Tree,
    /// A blob.
    Blob,
    /// A tag.
    Tag,
}

impl PeelTarget {
    fn raw(self) -> c_int {
        match self {
            PeelTarget::Any => -2,
            PeelTarget::Commit => 1,
            PeelTarget::Tree => 2,
            PeelTarget::Blob => 3,
            PeelTarget::Tag => 4,
        }
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
    pub fn lookup(repo: &'repo Repository, name: &[u8]) -> Result<Reference<'repo>> {
        cname(name)?;
        rich(refs::lookup(repo.ptr(), name)).map(Self::wrap)
    }

    /// `git_reference_lookup_resolved`: follow symbolic references up to
    /// `max_nesting` deep (negative: the default).
    pub fn lookup_resolved(
        repo: &'repo Repository,
        name: &[u8],
        max_nesting: i32,
    ) -> Result<Reference<'repo>> {
        cname(name)?;
        rich(refs::lookup_resolved(repo.ptr(), name, max_nesting)).map(Self::wrap)
    }

    /// `git_reference_dwim`: look a short name up as git does.
    pub fn dwim(repo: &'repo Repository, shorthand: &[u8]) -> Result<Reference<'repo>> {
        cname(shorthand)?;
        rich(refs::dwim(repo.ptr(), shorthand)).map(Self::wrap)
    }

    /// `git_reference_create_matching`: create (or with `force`,
    /// overwrite) `name` pointing at `id`; with `current`, only if the
    /// reference is now at that id.
    pub fn create(
        repo: &'repo Repository,
        name: &[u8],
        id: &Oid,
        force: bool,
        current: Option<&Oid>,
        log_message: Option<&str>,
    ) -> Result<Reference<'repo>> {
        cname(name)?;
        let msg = log_message.map(|m| to_cstring(m.as_bytes()));
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

    /// `git_reference_symbolic_create_matching`.
    pub fn symbolic_create(
        repo: &'repo Repository,
        name: &[u8],
        target: &[u8],
        force: bool,
        current: Option<&[u8]>,
        log_message: Option<&str>,
    ) -> Result<Reference<'repo>> {
        cname(name)?;
        cname(target)?;
        let msg = log_message.map(|m| to_cstring(m.as_bytes()));
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

    /// The full name.
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

    /// Direct or symbolic.
    pub fn kind(&self) -> ReferenceType {
        self.view().kind().unwrap_or(ReferenceType::Direct)
    }

    /// The target of a direct reference.
    pub fn target(&self) -> Option<Oid> {
        self.view().target().copied()
    }

    /// The peeled target of a direct reference, when the database knows it
    /// (`git_reference_target_peel`).
    pub fn target_peel(&self) -> Option<Oid> {
        let v = self.view();
        (v.raw_kind() == GIT_REFERENCE_DIRECT && !v.peel().is_zero()).then(|| *v.peel())
    }

    /// The target of a symbolic reference.
    pub fn symbolic_target(&self) -> Option<&[u8]> {
        self.view().symbolic_target()
    }

    /// Under `refs/heads/`.
    pub fn is_branch(&self) -> bool {
        refs::is_branch(self.name())
    }

    /// Under `refs/remotes/`.
    pub fn is_remote(&self) -> bool {
        refs::is_remote(self.name())
    }

    /// Under `refs/tags/`.
    pub fn is_tag(&self) -> bool {
        refs::is_tag(self.name())
    }

    /// Under `refs/notes/`.
    pub fn is_note(&self) -> bool {
        refs::is_note(self.name())
    }

    /// The direct reference this one ends at (`git_reference_resolve`).
    pub fn resolve(&self) -> Result<Reference<'repo>> {
        rich(refs::resolve(self.view())).map(Self::wrap)
    }

    /// `git_reference_set_target`: point this direct reference at `id`
    /// (if it has not moved); the updated reference.
    pub fn set_target(&self, id: &Oid, log_message: Option<&str>) -> Result<Reference<'repo>> {
        let msg = log_message.map(|m| to_cstring(m.as_bytes()));
        rich(refs::set_target(self.view(), id, msg.as_deref())).map(Self::wrap)
    }

    /// `git_reference_symbolic_set_target`.
    pub fn symbolic_set_target(
        &self,
        target: &[u8],
        log_message: Option<&str>,
    ) -> Result<Reference<'repo>> {
        cname(target)?;
        let msg = log_message.map(|m| to_cstring(m.as_bytes()));
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
        new_name: &[u8],
        force: bool,
        log_message: Option<&str>,
    ) -> Result<Reference<'repo>> {
        cname(new_name)?;
        let msg = log_message.map(|m| to_cstring(m.as_bytes()));
        let mut out = None;
        let r = refs::rename(self.view(), new_name, force, msg.as_deref(), &mut out);
        rich(r)?;
        out.map(Self::wrap)
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))
    }

    /// `git_reference_delete` (not HEAD).
    pub fn delete(&self) -> Result<()> {
        let v = self.view();
        if v.name() == b"HEAD" {
            return Err(Error::new(
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

    /// The id of the object this reference peels to
    /// (`git_reference_peel`).
    pub fn peel_to_id(&self, target: PeelTarget) -> Result<Oid> {
        rich(refs::peel(self.view(), target.raw())).map(|o| o.id())
    }

    /// Whether this is HEAD and its branch has no commit yet
    /// (`git_reference__is_unborn_head`).
    pub fn is_unborn_head(&self, repo: &'repo Repository) -> Result<bool> {
        rich(refs::is_unborn_head(self.view(), repo.ptr()))
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

/// `git_reference_name_to_id`: the id `name` resolves to.
pub fn name_to_id(repo: &Repository, name: &[u8]) -> Result<Oid> {
    cname(name)?;
    rich(refs::name_to_id(repo.ptr(), name))
}

/// `git_reference_remove`: delete `name` (and its reflog).
pub fn remove(repo: &Repository, name: &[u8]) -> Result<()> {
    let n = cname(name)?;
    rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.delete(&n, None, None)))
}

/// `git_reference_list`: every reference name.
pub fn list(repo: &Repository) -> Result<Vec<Vec<u8>>> {
    ReferenceNames::new(repo)?.collect()
}

/// `git_reference_has_log`.
pub fn has_log(repo: &Repository, name: &[u8]) -> Result<bool> {
    let n = cname(name)?;
    rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.has_log(&n)))
}

/// `git_reference_ensure_log`.
pub fn ensure_log(repo: &Repository, name: &[u8]) -> Result<()> {
    let n = cname(name)?;
    rich(
        RefdbPtr::of_repo(repo.ptr()).and_then(|db| ErrorCode::check(db.ensure_log(&n)?).map(drop)),
    )
}

/// `git_refdb_compress`: pack the loose references.
pub fn compress(repo: &Repository) -> Result<()> {
    rich(RefdbPtr::of_repo(repo.ptr()).and_then(RefdbPtr::compress))
}

/// The flags of [`normalize_name`] (`git_reference_format_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReferenceFormat(u32);

impl ReferenceFormat {
    /// `GIT_REFERENCE_FORMAT_NORMAL`.
    pub const NORMAL: ReferenceFormat = ReferenceFormat(0);
    /// `GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL`.
    pub const ALLOW_ONELEVEL: ReferenceFormat =
        ReferenceFormat(refs::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL);
    /// `GIT_REFERENCE_FORMAT_REFSPEC_PATTERN`.
    pub const REFSPEC_PATTERN: ReferenceFormat =
        ReferenceFormat(refs::GIT_REFERENCE_FORMAT_REFSPEC_PATTERN);
    /// `GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND`.
    pub const REFSPEC_SHORTHAND: ReferenceFormat =
        ReferenceFormat(refs::GIT_REFERENCE_FORMAT_REFSPEC_SHORTHAND);
}

impl core::ops::BitOr for ReferenceFormat {
    type Output = ReferenceFormat;
    fn bitor(self, o: ReferenceFormat) -> ReferenceFormat {
        ReferenceFormat(self.0 | o.0)
    }
}

/// `git_reference_normalize_name`.
pub fn normalize_name(name: &[u8], format: ReferenceFormat) -> Result<Vec<u8>> {
    cname(name)?;
    let mut buf = crate::util::str::GitStr::new();
    rich(refs::normalize_name(Some(&mut buf), name, format.0))?;
    Ok(buf.as_bytes().to_vec())
}

/// `git_reference_name_is_valid`.
pub fn is_valid_name(name: &[u8]) -> bool {
    !name.contains(&0)
        && matches!(
            refs::name_is_valid(name, refs::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL),
            Ok(true)
        )
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

impl<'repo> References<'repo> {
    /// Every reference.
    pub fn new(repo: &'repo Repository) -> Result<References<'repo>> {
        Self::make(repo, None)
    }

    /// The references whose names match `glob` (`wildmatch`).
    pub fn glob(repo: &'repo Repository, glob: &[u8]) -> Result<References<'repo>> {
        let g = cname(glob)?;
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

impl<'repo> ReferenceNames<'repo> {
    /// Every reference's name.
    pub fn new(repo: &'repo Repository) -> Result<ReferenceNames<'repo>> {
        Ok(ReferenceNames {
            it: rich(RefIter::new(repo.ptr(), None))?,
            _repo: PhantomData,
        })
    }

    /// The names matching `glob`.
    pub fn glob(repo: &'repo Repository, glob: &[u8]) -> Result<ReferenceNames<'repo>> {
        let g = cname(glob)?;
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

/// `git_reference_foreach_glob` (or `_foreach_name` without a glob): call
/// `f` with each name, until it returns `false`.
pub fn for_each_name(
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

// ---------------------------------------------------------------------------
// Reflogs
// ---------------------------------------------------------------------------

/// Who made a reflog entry, and when. PENDING(l-obj): becomes `Signature`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committer {
    /// The name.
    pub name: Vec<u8>,
    /// The email.
    pub email: Vec<u8>,
    /// Seconds since the epoch.
    pub time: i64,
    /// The timezone offset, in minutes.
    pub offset_minutes: i32,
}

impl Committer {
    /// A committer.
    pub fn new(name: &[u8], email: &[u8], time: i64, offset_minutes: i32) -> Committer {
        Committer {
            name: name.to_vec(),
            email: email.to_vec(),
            time,
            offset_minutes,
        }
    }

    fn to_sig(&self) -> Result<SigBox> {
        let (n, e) = (cname(&self.name)?, cname(&self.email)?);
        rich(SigBox::new(&n, &e, self.time, self.offset_minutes))
    }

    /// From a C signature.
    fn from_sig(sig: *const git_signature) -> Option<Committer> {
        // SAFETY: NULL or a valid signature (an entry's committer, alive
        // with the entry).
        let s = unsafe { sig.as_ref() }?;
        let str_of = |p: *const c_char| -> Vec<u8> {
            if p.is_null() {
                return Vec::new();
            }
            // SAFETY: the signature's NUL-terminated strings.
            unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
        };
        Some(Committer {
            name: str_of(s.name),
            email: str_of(s.email),
            time: s.when.time,
            offset_minutes: s.when.offset,
        })
    }
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

    /// The previous value.
    pub fn id_old(&self) -> Oid {
        self.get().oid_old
    }

    /// The new value.
    pub fn id_new(&self) -> Oid {
        self.get().oid_cur
    }

    /// The message, if any.
    pub fn message(&self) -> Option<&'a [u8]> {
        let m = self.get().msg;
        if m.is_null() {
            return None;
        }
        // SAFETY: the entry's NUL-terminated message, alive for `'a`.
        Some(unsafe { CStr::from_ptr(m) }.to_bytes())
    }

    /// Who made the change.
    pub fn committer(&self) -> Option<Committer> {
        Committer::from_sig(self.get().committer)
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
    pub fn read(repo: &'repo Repository, name: &[u8]) -> Result<Reflog<'repo>> {
        let n = cname(name)?;
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

    /// The number of entries.
    pub fn len(&self) -> usize {
        // SAFETY: our live reflog.
        unsafe { (*self.inner.as_ptr()).entries.len() }
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entry at `idx` (0: the newest).
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
    /// newlines become spaces).
    pub fn append(
        &mut self,
        id: &Oid,
        committer: &Committer,
        message: Option<&[u8]>,
    ) -> Result<()> {
        let sig = committer.to_sig()?;
        let msg = message.map(crate::util::fs_path::c_str);
        // SAFETY: our reflog, exclusively borrowed; a valid signature.
        rich(unsafe { rl::append(self.inner.as_ptr(), id, sig.as_ptr(), msg) })
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
    pub fn rename(repo: &Repository, old_name: &[u8], new_name: &[u8]) -> Result<()> {
        let (o, n) = (cname(old_name)?, cname(new_name)?);
        rich(RefdbPtr::of_repo(repo.ptr()).and_then(|db| db.reflog_rename(&o, &n)))
    }

    /// `git_reflog_delete`.
    pub fn delete(repo: &Repository, name: &[u8]) -> Result<()> {
        let n = cname(name)?;
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
    pub committer: Committer,
    /// The message.
    pub message: Option<Vec<u8>>,
}

/// A reference database backend written in Rust (`git_refdb_backend`).
///
/// libgit2 may call it from several threads at once, and re-entrantly
/// (writing a reference may look references up): it takes `&self`, and is
/// `Send + Sync`. A panic is turned into an error (class
/// `GIT_ERROR_INTERNAL`). Transactions (`lock` / `unlock`) are not offered.
pub trait RefdbBackend: Send + Sync + 'static {
    /// Whether `name` exists.
    fn exists(&self, name: &[u8]) -> core::result::Result<bool, Error>;
    /// The value of `name`, `None` if there is none.
    fn lookup(&self, name: &[u8]) -> core::result::Result<Option<RefValue>, Error>;
    /// Every reference, in the order iteration returns them (the glob
    /// filtering is done for the backend).
    fn list(&self) -> core::result::Result<Vec<(Vec<u8>, RefValue)>, Error>;
    /// Write `name`. Unless `force`, it must not exist; with `old_id` /
    /// `old_target`, it must currently have that value
    /// ([`ErrorCode::MODIFIED`] otherwise).
    fn write(
        &self,
        name: &[u8],
        value: &RefValue,
        force: bool,
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
        message: Option<&[u8]>,
    ) -> core::result::Result<(), Error>;
    /// Rename `old_name` to `new_name`, returning its value.
    fn rename(
        &self,
        old_name: &[u8],
        new_name: &[u8],
        force: bool,
    ) -> core::result::Result<RefValue, Error>;
    /// Delete `name` (and its reflog), with the same conditions as
    /// [`write`](Self::write).
    fn delete(
        &self,
        name: &[u8],
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> core::result::Result<(), Error>;
    /// Optimise storage (`git_refdb_compress`).
    fn compress(&self) -> core::result::Result<(), Error> {
        Ok(())
    }
    /// Whether `name` has a reflog.
    fn has_log(&self, _name: &[u8]) -> bool {
        false
    }
    /// Make sure `name` has a reflog.
    fn ensure_log(&self, _name: &[u8]) -> core::result::Result<(), Error> {
        Ok(())
    }
    /// The reflog of `name`, oldest first.
    fn reflog_read(&self, _name: &[u8]) -> core::result::Result<Vec<ReflogRecord>, Error> {
        Ok(Vec::new())
    }
    /// Replace the reflog of `name`.
    fn reflog_write(
        &self,
        _name: &[u8],
        _entries: &[ReflogRecord],
    ) -> core::result::Result<(), Error> {
        Ok(())
    }
    /// Rename a reflog.
    fn reflog_rename(&self, _old_name: &[u8], _new_name: &[u8]) -> core::result::Result<(), Error> {
        Ok(())
    }
    /// Delete a reflog.
    fn reflog_delete(&self, _name: &[u8]) -> core::result::Result<(), Error> {
        Ok(())
    }
}

/// Install `backend` as the reference database backend of `repo`
/// (`git_refdb_set_backend` on the repository's database); the previous
/// backend is freed.
///
/// It takes the repository exclusively: the old backend is freed at once
/// (as in C, which keeps no count on a backend), so no other thread, and
/// no [`Reference`] or iterator of the repository, may be using it.
///
/// ```compile_fail
/// # use libgit2_rs::refs_api::*;
/// # fn f<B: RefdbBackend>(mut repo: Repository, b: B) {
/// let head = Reference::lookup(&repo, b"HEAD").unwrap();
/// set_refdb_backend(&mut repo, b).unwrap(); // `repo` is borrowed by `head`
/// drop(head);
/// # }
/// ```
pub fn set_refdb_backend<B: RefdbBackend>(repo: &mut Repository, backend: B) -> Result<()> {
    let db = RefdbPtr::of_repo(repo.ptr()).map_err(Error::from)?;
    let table = CTable::<RefdbBackendTable, RustBackend<B>>::new(RustBackend(backend));
    let raw = table.into_c().as_ptr();
    // SAFETY: a live database and a complete backend, whose ownership
    // passes to the database.
    let r = unsafe { refdb::set_backend(db, raw) };
    if let Err(e) = r {
        // SAFETY: the table was not taken: release it through its slot.
        unsafe { CTable::<RefdbBackendTable, RustBackend<B>>::release(raw) };
        return Err(Error::from(e));
    }
    Ok(())
}

/// The state behind a Rust backend's table.
pub struct RustBackend<B>(B);

fn fail(e: &Error) -> c_int {
    let _ = errors::set_str(e.class(), e.message_bytes());
    e.code().raw()
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

fn make_ref(name: &[u8], v: &RefValue) -> errors::Result<RefBox> {
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
pub struct VecIter {
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
    // here (`free` once); `lock`, `unlock`, `init` are NULL, which the
    // header allows.
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
            let log = match rl::Reflog::alloc(n, crate::oid::OidType::Sha1 as c_int) {
                Ok(l) => l,
                Err(e) => return e.raw(),
            };
            for rec in &records {
                let sig = match rec.committer.to_sig() {
                    Ok(s) => s,
                    Err(e) => return fail(&e),
                };
                // SAFETY: our reflog; a valid signature.
                if let Err(e) = unsafe { rl::append(log.as_ptr(), &rec.new, sig.as_ptr(), rec.message.as_deref()) } {
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
                    committer: Committer::from_sig(e.committer).unwrap_or_else(|| Committer::new(b"", b"", 0, 0)),
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
            Err(Error::new(
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
        let id = Oid::from_hex(
            b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            crate::OidType::Sha1,
        )
        .unwrap();
        let t = CTable::<RefdbBackendTable, RustBackend<Mem>>::new(RustBackend(Mem::default()));
        let b = t.into_c().as_ptr();
        // SAFETY: libgit2's contract with a backend: its own slots, its
        // table, valid arguments, `free` once.
        unsafe {
            let tb = &*b;
            assert!(tb.lock.is_none() && tb.unlock.is_none() && tb.init.is_none());
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
        let id = Oid::from_hex(
            b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            crate::OidType::Sha1,
        )
        .unwrap();
        let t = CTable::<RefdbBackendTable, RustBackend<Mem>>::new(RustBackend(Mem::default()));
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
        let t = CTable::<RefdbBackendTable, RustBackend<Reentrant>>::new(RustBackend(st));
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
