//! The safe Rust API of repositories (PORTING.md §5).
//!
//! [`Repository`] owns a `git_repository` and frees it on drop. Opening,
//! discovering and initializing have plain functions and builders
//! ([`Repository::open_ext`], [`RepositoryInitOptions`]). The lazily
//! created odb, configuration, refdb and index come back as owners that
//! hold their own reference ([`Odb`], [`Config`], [`Refdb`], [`Index`]).
//!
//! A `Repository` is `Send` and `Sync`: libgit2 lets several threads use
//! one repository for reference, object and configuration work
//! (`tests/libgit2/threads`): its lazily made odb, refdb and config are
//! published with compare-and-swap, its caches are locked or atomic, and
//! the refdb, odb and config lock their own state; the index serialises
//! its users with its own lock (see [`Index`]). What is single-threaded
//! takes `&mut self`: the setters that replace paths, strings or the odb,
//! config and index (so no borrow of the old one can dangle),
//! [`Repository::cleanup`], the submodule cache, and
//! [`Repository::reinit_filesystem`] (submodules, which C caches without
//! synchronisation).
//!
//! File-system paths are `impl AsRef<Path>` in and `&Path`/`PathBuf` out
//! (`OsStr` bytes on Unix); names and messages are bytes.
//!
//! The owners a repository hands out point back to it in C (an odb uses
//! its owner's object cache, config file backends keep the repository for
//! their include conditions, an index its owner's capabilities), so they
//! carry its lifetime, and so do their clones, levels, snapshots and
//! iterators: none outlives the repository.
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/nonexistent").unwrap();
//! let odb = repo.odb().unwrap().clone();
//! drop(repo); // error: `repo` is still borrowed by the clone of its odb
//! drop(odb);
//! ```
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/nonexistent").unwrap();
//! let cfg = repo.config().unwrap();
//! let level = cfg.clone().snapshot().unwrap();
//! drop(cfg);
//! drop(repo); // error: `repo` is still borrowed by the snapshot
//! drop(level);
//! ```
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/nonexistent").unwrap();
//! let cfg = repo.config().unwrap();
//! // error: the configuration borrows `repo`, so it is not `'static`
//! std::thread::spawn(move || drop(cfg));
//! ```
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/nonexistent").unwrap();
//! let index = repo.index().unwrap().clone();
//! drop(repo); // error: `repo` is still borrowed by the index's clone
//! drop(index);
//! ```
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/nonexistent").unwrap();
//! let index = repo.index().unwrap();
//! // error: the index borrows `repo`, so it is not `'static`
//! std::thread::spawn(move || drop(index));
//! ```
//!
//! The index and the repository are usable together, and from several
//! threads (I-1):
//!
//! ```no_run
//! # use libgit2_rs::Repository;
//! let repo = Repository::open("/some/repo").unwrap();
//! let index = repo.index().unwrap();
//! let head = repo.head().unwrap(); // fine: the repository is shared
//! std::thread::scope(|s| {
//!     s.spawn(|| index.len());
//!     s.spawn(|| repo.is_empty());
//! });
//! # drop(head);
//! ```
//!
//! The setters (`set_odb`, `set_config`, `set_index`, `wrap_odb`) take the object over: it must be made alone and be its only
//! handle, because the repository becomes its owner (the object then reads
//! the repository), so no handle that can outlive the repository may be
//! left. A handle of another repository's object is refused at run time
//! (that repository owns it); one of this repository's cannot be passed to
//! a `&mut self` setter while it borrows the repository:
//!
//! ```compile_fail
//! # use libgit2_rs::Repository;
//! let mut repo = Repository::open("/nonexistent").unwrap();
//! let index = repo.index().unwrap();
//! repo.set_index(index); // error: `index` borrows `repo`
//! ```

use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::handles::Refdb;
use super::head::{self, state};
use super::init::{InitOptions, InitOut};
use super::*;
use crate::config::Config;
use crate::index::Index;
use crate::odb::Odb;
use crate::oid::{Oid, OidType};
use crate::refs_api::Reference;
use crate::util::errors::{Error, ErrorCode};
use crate::util::str::GitStr;

type Result<T> = core::result::Result<T, Error>;

/// The error for an argument with an interior NUL byte (class `INVALID`,
/// "<what> contains a NUL byte"), with the thread's message set.
fn nul_error(what: &str) -> Error {
    let e = crate::git_error_set!(
        crate::util::errors::ErrorClass::INVALID,
        "{what} contains a NUL byte"
    );
    Error::from(e)
}

/// `b` as a C string, or [`nul_error`]`(what)`.
fn c_string(b: impl Into<Vec<u8>>, what: &str) -> Result<CString> {
    CString::new(b).map_err(|_| nul_error(what))
}

/// The ceiling directories as C's `:`-separated list.
fn ceiling_list(dirs: &[&Path]) -> Result<Option<CString>> {
    if dirs.is_empty() {
        return Ok(None);
    }
    let joined = dirs
        .iter()
        .map(|d| d.as_os_str().as_bytes())
        .collect::<Vec<_>>()
        .join(&crate::util::fs_path::GIT_PATH_LIST_SEPARATOR);
    c_string(joined, "ceiling directory").map(Some)
}

fn path_c(p: &Path) -> Result<CString> {
    c_string(p.as_os_str().as_bytes(), "path")
}

fn to_path(c: &CStr) -> &Path {
    Path::new(OsStr::from_bytes(c.to_bytes()))
}

fn str_path(s: &GitStr) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(crate::util::fs_path::str_c_bytes(s)))
}

crate::api::flags::bitflags! {
    /// `git_repository_open_flag_t`.
    pub struct RepositoryOpenFlags: u32 {
        /// `GIT_REPOSITORY_OPEN_NO_SEARCH`: only open the given path.
        NO_SEARCH = GIT_REPOSITORY_OPEN_NO_SEARCH;
        /// `GIT_REPOSITORY_OPEN_CROSS_FS`: search across filesystems.
        CROSS_FS = GIT_REPOSITORY_OPEN_CROSS_FS;
        /// `GIT_REPOSITORY_OPEN_BARE`: open as bare.
        BARE = GIT_REPOSITORY_OPEN_BARE;
        /// `GIT_REPOSITORY_OPEN_NO_DOTGIT`: do not append `/.git`.
        NO_DOTGIT = GIT_REPOSITORY_OPEN_NO_DOTGIT;
        /// `GIT_REPOSITORY_OPEN_FROM_ENV`: honour `GIT_DIR` and friends.
        FROM_ENV = GIT_REPOSITORY_OPEN_FROM_ENV;
    }
}

crate::api::flags::bitflags! {
    /// `git_repository_init_flag_t`.
    pub struct RepositoryInitFlags: u32 {
        /// `GIT_REPOSITORY_INIT_BARE`.
        BARE = GIT_REPOSITORY_INIT_BARE;
        /// `GIT_REPOSITORY_INIT_NO_REINIT`.
        NO_REINIT = GIT_REPOSITORY_INIT_NO_REINIT;
        /// `GIT_REPOSITORY_INIT_MKDIR`.
        MKDIR = GIT_REPOSITORY_INIT_MKDIR;
        /// `GIT_REPOSITORY_INIT_MKPATH`.
        MKPATH = GIT_REPOSITORY_INIT_MKPATH;
        /// `GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE`.
        EXTERNAL_TEMPLATE = GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE;
        /// `GIT_REPOSITORY_INIT_RELATIVE_GITLINK`.
        RELATIVE_GITLINK = GIT_REPOSITORY_INIT_RELATIVE_GITLINK;
    }
}

/// The shared mode of a new repository (`git_repository_init_mode_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepositoryInitMode {
    /// `GIT_REPOSITORY_INIT_SHARED_UMASK`.
    #[default]
    SharedUmask,
    /// `GIT_REPOSITORY_INIT_SHARED_GROUP`.
    SharedGroup,
    /// `GIT_REPOSITORY_INIT_SHARED_ALL`.
    SharedAll,
    /// Explicit permission bits.
    Mode(u32),
}

impl RepositoryInitMode {
    fn raw(self) -> u32 {
        match self {
            RepositoryInitMode::SharedUmask => GIT_REPOSITORY_INIT_SHARED_UMASK,
            RepositoryInitMode::SharedGroup => GIT_REPOSITORY_INIT_SHARED_GROUP,
            RepositoryInitMode::SharedAll => GIT_REPOSITORY_INIT_SHARED_ALL,
            RepositoryInitMode::Mode(m) => m,
        }
    }
}

/// The reference storage of a new repository (`git_refdb_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefStorage {
    /// Loose and packed refs (`GIT_REFDB_FILES`).
    #[default]
    Files,
    /// Reftables (`GIT_REFDB_REFTABLE`).
    Reftable,
}

/// Options of [`Repository::init_opts`] (`git_repository_init_options`):
/// a by-value builder.
///
/// ```no_run
/// # use libgit2_rs::{Repository, RepositoryInitOptions};
/// let o = RepositoryInitOptions::new().bare(true).initial_head("main");
/// let repo = Repository::init_opts("/tmp/x.git", &o).unwrap();
/// # drop(repo);
/// ```
#[derive(Clone, Debug)]
pub struct RepositoryInitOptions {
    flags: RepositoryInitFlags,
    mode: RepositoryInitMode,
    workdir_path: Option<PathBuf>,
    description: Option<Vec<u8>>,
    template_path: Option<PathBuf>,
    initial_head: Option<Vec<u8>>,
    origin_url: Option<Vec<u8>>,
    oid_type: Option<OidType>,
    ref_storage: Option<RefStorage>,
}

impl Default for RepositoryInitOptions {
    fn default() -> RepositoryInitOptions {
        RepositoryInitOptions::new()
    }
}

impl RepositoryInitOptions {
    /// The defaults: `MKPATH` (as `git_repository_init`).
    pub fn new() -> RepositoryInitOptions {
        RepositoryInitOptions {
            flags: RepositoryInitFlags::MKPATH,
            mode: RepositoryInitMode::SharedUmask,
            workdir_path: None,
            description: None,
            template_path: None,
            initial_head: None,
            origin_url: None,
            oid_type: None,
            ref_storage: None,
        }
    }
    /// The flags (replacing those set so far).
    pub fn flags(mut self, f: RepositoryInitFlags) -> Self {
        self.flags = f;
        self
    }
    /// A bare repository (`GIT_REPOSITORY_INIT_BARE`).
    pub fn bare(mut self, bare: bool) -> Self {
        self.flags.set(RepositoryInitFlags::BARE, bare);
        self
    }
    /// The shared mode.
    pub fn mode(mut self, m: RepositoryInitMode) -> Self {
        self.mode = m;
        self
    }
    /// The working directory.
    pub fn workdir_path(mut self, p: impl AsRef<Path>) -> Self {
        self.workdir_path = Some(p.as_ref().to_owned());
        self
    }
    /// The description file's contents.
    pub fn description(mut self, d: impl AsRef<[u8]>) -> Self {
        self.description = Some(d.as_ref().to_vec());
        self
    }
    /// The template directory.
    pub fn template_path(mut self, p: impl AsRef<Path>) -> Self {
        self.template_path = Some(p.as_ref().to_owned());
        self
    }
    /// The initial HEAD (a branch name, or a `refs/…` name).
    pub fn initial_head(mut self, h: impl AsRef<[u8]>) -> Self {
        self.initial_head = Some(h.as_ref().to_vec());
        self
    }
    /// Add an `origin` remote with this URL.
    pub fn origin_url(mut self, u: impl AsRef<[u8]>) -> Self {
        self.origin_url = Some(u.as_ref().to_vec());
        self
    }
    /// The object id type.
    pub fn oid_type(mut self, t: OidType) -> Self {
        self.oid_type = Some(t);
        self
    }
    /// The reference storage.
    pub fn ref_storage(mut self, r: RefStorage) -> Self {
        self.ref_storage = Some(r);
        self
    }

    fn to_raw(&self) -> Result<InitOptions> {
        let bytes = |b: &Option<Vec<u8>>, what: &str| -> Result<Option<CString>> {
            b.as_ref().map(|v| c_string(v.clone(), what)).transpose()
        };
        let path = |p: &Option<PathBuf>| p.as_deref().map(path_c).transpose();
        Ok(InitOptions {
            flags: self.flags.bits(),
            mode: self.mode.raw(),
            workdir_path: path(&self.workdir_path)?,
            description: bytes(&self.description, "description")?,
            template_path: path(&self.template_path)?,
            initial_head: bytes(&self.initial_head, "initial head")?,
            origin_url: bytes(&self.origin_url, "origin URL")?,
            oid_type: self.oid_type.map_or(0, OidType::raw),
            refdb_type: match self.ref_storage {
                None => 0,
                Some(RefStorage::Files) => GIT_REFDB_FILES,
                Some(RefStorage::Reftable) => GIT_REFDB_REFTABLE,
            },
        })
    }
}

/// `git_repository_state_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryState {
    /// No operation in progress.
    Clean,
    /// A merge.
    Merge,
    /// A revert.
    Revert,
    /// A sequence of reverts.
    RevertSequence,
    /// A cherry-pick.
    CherryPick,
    /// A sequence of cherry-picks.
    CherryPickSequence,
    /// A bisect.
    Bisect,
    /// A rebase.
    Rebase,
    /// An interactive rebase.
    RebaseInteractive,
    /// A merge-based rebase.
    RebaseMerge,
    /// `git am`.
    ApplyMailbox,
    /// `git am` or a rebase.
    ApplyMailboxOrRebase,
}

impl RepositoryState {
    fn from_raw(s: c_int) -> RepositoryState {
        match s {
            state::MERGE => RepositoryState::Merge,
            state::REVERT => RepositoryState::Revert,
            state::REVERT_SEQUENCE => RepositoryState::RevertSequence,
            state::CHERRYPICK => RepositoryState::CherryPick,
            state::CHERRYPICK_SEQUENCE => RepositoryState::CherryPickSequence,
            state::BISECT => RepositoryState::Bisect,
            state::REBASE => RepositoryState::Rebase,
            state::REBASE_INTERACTIVE => RepositoryState::RebaseInteractive,
            state::REBASE_MERGE => RepositoryState::RebaseMerge,
            state::APPLY_MAILBOX => RepositoryState::ApplyMailbox,
            state::APPLY_MAILBOX_OR_REBASE => RepositoryState::ApplyMailboxOrRebase,
            _ => RepositoryState::Clean,
        }
    }
}

/// An item of the repository's layout (`git_repository_item_t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RepositoryItem {
    /// The git directory.
    Gitdir = 0,
    /// The working directory.
    Workdir,
    /// The common directory.
    Commondir,
    /// The index file.
    Index,
    /// The object directory.
    Objects,
    /// The references directory.
    Refs,
    /// The packed-refs file.
    PackedRefs,
    /// The remotes directory.
    Remotes,
    /// The configuration file.
    Config,
    /// The info directory.
    Info,
    /// The hooks directory.
    Hooks,
    /// The logs directory.
    Logs,
    /// The modules directory.
    Modules,
    /// The worktrees directory.
    Worktrees,
    /// The worktree's own configuration file.
    WorktreeConfig,
}

/// An owned repository (`git_repository`), freed on drop.
///
/// The library is initialized on first use and stays so (see
/// [`Libgit2`](crate::Libgit2)).
pub struct Repository {
    repo: NonNull<GitRepository>,
}

// SAFETY: the repository and what it owns are released from any thread.
unsafe impl Send for Repository {}
// SAFETY: every field of `GitRepository` is atomic or behind its own lock
// (it is `Sync`), and what `&self` reaches is libgit2's thread-safe part:
// the odb, refdb and config (made lazily and published with compare-and-
// swap; each locks its own state), the object cache (an rwlock), the
// configmap cache (atomics), and the files of the git directory; the
// index is reached under its safe-API lock (`Index`, and
// `with_repository_index_locked` for the C that reads it). What is not
// thread-safe is reached through `&mut self` only: the path and string
// setters, `set_odb`/`set_config`/`set_index`, `cleanup`,
// the submodule cache, `reinit_filesystem`.
unsafe impl Sync for Repository {}

impl Drop for Repository {
    fn drop(&mut self) {
        // SAFETY: the repository this owner holds alone.
        unsafe { GitRepository::free(self.repo.as_ptr()) }
    }
}

impl core::fmt::Debug for Repository {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Repository")
            .field("path", &self.path())
            .finish()
    }
}

impl Repository {
    fn from_nonnull(repo: NonNull<GitRepository>) -> Repository {
        Repository { repo }
    }

    /// The `git_repository *` (crate-internal: the commit and tree APIs
    /// hand it to their C-shaped internals), live while `self` is.
    pub(crate) fn raw_ptr(&self) -> *mut GitRepository {
        self.repo.as_ptr()
    }

    pub(crate) fn raw(&self) -> &GitRepository {
        // SAFETY: the repository this owner holds, live until drop.
        unsafe { self.repo.as_ref() }
    }

    /// `git_repository_open`: open the repository at `path` (its git
    /// directory, or a working directory with `.git`).
    pub fn open(path: impl AsRef<Path>) -> Result<Repository> {
        Repository::open_ext(Some(path.as_ref()), RepositoryOpenFlags::NO_SEARCH, &[])
    }

    /// `git_repository_open_ext`.
    pub fn open_ext(
        path: Option<&Path>,
        flags: RepositoryOpenFlags,
        ceiling_dirs: &[&Path],
    ) -> Result<Repository> {
        let p = path.map(path_c).transpose()?;
        let ceil = ceiling_list(ceiling_dirs)?;
        crate::libgit2::ensure_init()?;
        match open::open_ext(p.as_deref(), flags.bits(), ceil.as_deref(), true)? {
            Some(r) => Ok(Repository::from_nonnull(r)),
            None => Err(ErrorCode::GENERIC.into()),
        }
    }

    /// `git_repository_open_ext(NULL, GIT_REPOSITORY_OPEN_FROM_ENV, NULL)`:
    /// open as `git` would, from `GIT_DIR`, `GIT_WORK_TREE` and the rest of
    /// the environment.
    pub fn open_from_env() -> Result<Repository> {
        Repository::open_ext(None, RepositoryOpenFlags::FROM_ENV, &[])
    }

    /// `git_repository_open_bare`.
    pub fn open_bare(path: impl AsRef<Path>) -> Result<Repository> {
        let p = path_c(path.as_ref())?;
        crate::libgit2::ensure_init()?;
        Ok(Repository::from_nonnull(open::open_bare(&p)?))
    }

    /// `git_repository_discover`: the git directory of the repository that
    /// contains `start`.
    pub fn discover(
        start: impl AsRef<Path>,
        across_fs: bool,
        ceiling_dirs: &[&Path],
    ) -> Result<PathBuf> {
        let s = path_c(start.as_ref())?;
        let ceil = ceiling_list(ceiling_dirs)?;
        crate::libgit2::ensure_init()?;
        let g = open::discover(&s, across_fs, ceil.as_deref())?;
        Ok(str_path(&g))
    }

    /// `git_repository_init`.
    pub fn init(path: impl AsRef<Path>, bare: bool) -> Result<Repository> {
        Repository::init_opts(path, &RepositoryInitOptions::new().bare(bare))
    }

    /// `git_repository_init_ext`.
    pub fn init_opts(path: impl AsRef<Path>, opts: &RepositoryInitOptions) -> Result<Repository> {
        let p = path_c(path.as_ref())?;
        let mut raw = opts.to_raw()?;
        crate::libgit2::ensure_init()?;
        let (r, out) = init::init_ext(&p, &mut raw);
        let repo = match out {
            InitOut::Repo(p) => Some(Repository::from_nonnull(p)),
            _ => None,
        };
        r?;
        repo.ok_or_else(|| ErrorCode::GENERIC.into())
    }

    /// `git_repository_new_ext`: a repository with no backends.
    pub fn new_in_memory(oid_type: OidType) -> Result<Repository> {
        crate::libgit2::ensure_init()?;
        Ok(Repository::from_nonnull(open::new_ext(Some(oid_type))?))
    }

    /// `git_repository_path`: the git directory (with a trailing slash).
    pub fn path(&self) -> &Path {
        self.raw().gitdir().map_or(Path::new(""), to_path)
    }

    /// `git_repository_workdir`: the working directory, `None` if bare.
    pub fn workdir(&self) -> Option<&Path> {
        self.raw().workdir().map(to_path)
    }

    /// `git_repository_commondir`.
    pub fn commondir(&self) -> Option<&Path> {
        self.raw().commondir().map(to_path)
    }

    /// `git_repository_is_bare`.
    pub fn is_bare(&self) -> bool {
        self.raw().is_bare()
    }

    /// `git_repository_is_worktree`.
    pub fn is_worktree(&self) -> bool {
        self.raw().is_worktree()
    }

    /// `git_repository_oid_type`.
    pub fn oid_type(&self) -> OidType {
        self.raw().oid_type()
    }

    /// `git_repository_is_empty`.
    pub fn is_empty(&self) -> Result<bool> {
        let r = head::is_empty(self.raw());
        ErrorCode::check(r)?;
        Ok(r != 0)
    }

    /// `git_repository_is_shallow`.
    pub fn is_shallow(&self) -> Result<bool> {
        Ok(head::is_shallow(self.raw())?)
    }

    /// `git_repository_head`.
    pub fn head(&self) -> Result<Reference<'_>> {
        let h = head::head(self.raw())?;
        // SAFETY: the reference `git_repository_head` returned, ours to own.
        let b = unsafe { crate::refs::RefBox::from_raw(h.into_raw()) };
        Ok(Reference::wrap(b.ok_or(ErrorCode::GENERIC)?))
    }

    /// `git_repository_head_detached`.
    pub fn head_detached(&self) -> Result<bool> {
        let r = head::head_detached(self.raw());
        ErrorCode::check(r)?;
        Ok(r != 0)
    }

    /// `git_repository_head_unborn`.
    pub fn head_unborn(&self) -> Result<bool> {
        let r = head::head_unborn(self.raw());
        ErrorCode::check(r)?;
        Ok(r != 0)
    }

    /// `git_repository_set_head`: make HEAD point at `refname` (a branch
    /// that need not exist yet, or any other reference, which detaches
    /// HEAD at what it points to).
    pub fn set_head(&self, refname: impl AsRef<[u8]>) -> Result<()> {
        let n = c_string(refname.as_ref(), "reference name")?;
        Ok(head::set_head(self.raw(), &n)?)
    }

    /// `git_repository_set_head_detached`.
    pub fn set_head_detached(&self, id: &Oid) -> Result<()> {
        Ok(head::set_head_detached(self.raw(), Some(id))?)
    }

    /// `git_repository_detach_head`.
    pub fn detach_head(&self) -> Result<()> {
        Ok(head::detach_head(self.raw())?)
    }

    /// `git_repository_state`.
    pub fn state(&self) -> Result<RepositoryState> {
        Ok(RepositoryState::from_raw(head::repo_state(self.raw())?))
    }

    /// `git_repository_state_cleanup`.
    pub fn state_cleanup(&self) -> Result<()> {
        Ok(head::state_cleanup(self.raw())?)
    }

    /// `git_repository_message`: the prepared commit message (`MERGE_MSG`).
    pub fn message(&self) -> Result<Vec<u8>> {
        let mut s = GitStr::new();
        head::message(&mut s, self.raw())?;
        Ok(s.as_bytes().to_vec())
    }

    /// `git_repository_message_remove`.
    pub fn remove_message(&self) -> Result<()> {
        if head::message_remove(self.raw()) < 0 {
            return Err(ErrorCode::GENERIC.into());
        }
        Ok(())
    }

    /// `git_repository_item_path`.
    pub fn item_path(&self, item: RepositoryItem) -> Result<PathBuf> {
        let mut s = GitStr::new();
        self.raw().item_path(&mut s, item as ItemT)?;
        Ok(str_path(&s))
    }

    /// `git_repository_odb`.
    pub fn odb(&self) -> Result<Odb<'_>> {
        let p = self.raw().odb_weak()?;
        // SAFETY: the repository's live odb; a new reference, bound to the
        // repository.
        Ok(unsafe { Odb::new_ref(p) })
    }

    /// `git_repository_config`.
    pub fn config(&self) -> Result<Config<'_>> {
        let c = self.raw().config_weak()?;
        // SAFETY: the repository's live config; a new reference, bound to
        // the repository.
        Ok(unsafe { Config::new_ref(NonNull::from(c)) })
    }

    /// `git_repository_config_snapshot`.
    pub fn config_snapshot(&self) -> Result<Config<'_>> {
        let snap = self.raw().config_weak()?.snapshot()?;
        Ok(Config::from_box(snap))
    }

    /// `git_repository_refdb`.
    pub fn refdb(&self) -> Result<Refdb<'_>> {
        let p = self.raw().refdb_weak()?;
        // SAFETY: the repository's live refdb; a new reference.
        Ok(unsafe { Refdb::new_ref(p) })
    }

    /// `git_repository_index`: the repository's index (loaded on first
    /// use). It borrows the repository, which stays usable meanwhile: the
    /// index serialises its own operations, and those of the repository
    /// whose C reads the index (I-1).
    pub fn index(&self) -> Result<Index<'_>> {
        let p = self.raw().index_weak()?;
        // SAFETY: the repository's live index; a new reference, bound to the
        // borrow of the repository (the index uses its owner).
        Ok(unsafe { Index::new_ref(p) })
    }

    /// `git_repository_set_workdir`.
    pub fn set_workdir(&mut self, workdir: impl AsRef<Path>, update_gitlink: bool) -> Result<()> {
        let w = path_c(workdir.as_ref())?;
        // SAFETY: `&mut self`: no path borrowed from the repository, and no
        // handle of it, is live.
        Ok(unsafe { init::set_workdir(self.raw(), &w, update_gitlink) }?)
    }

    /// `git_repository_set_bare`.
    pub fn set_bare(&mut self) -> Result<()> {
        // SAFETY: as `set_workdir`.
        Ok(unsafe { init::set_bare(self.raw()) }?)
    }

    /// `git_repository_reinit_filesystem`.
    ///
    /// `&mut self`: it walks the submodules (which read the index).
    pub fn reinit_filesystem(&mut self, recurse: bool) -> Result<()> {
        Ok(init::reinit_filesystem(self.raw(), recurse)?)
    }

    /// `git_repository_get_namespace`.
    pub fn namespace(&self) -> Option<&[u8]> {
        self.raw().namespace().map(CStr::to_bytes)
    }

    /// `git_repository_set_namespace`.
    pub fn set_namespace(&mut self, ns: Option<&[u8]>) -> Result<()> {
        if ns.is_some_and(|n| n.contains(&0)) {
            return Err(nul_error("namespace"));
        }
        let v = ns.map(super::alloc_cstr).transpose()?;
        // SAFETY: `&mut self`: no namespace borrowed from this repository
        // is live.
        unsafe { self.raw().replace_string(StrField::Namespace, v) };
        Ok(())
    }

    /// `git_repository_ident`: the identity for reflogs.
    pub fn ident(&self) -> (Option<&[u8]>, Option<&[u8]>) {
        (
            self.raw().ident_name().map(CStr::to_bytes),
            self.raw().ident_email().map(CStr::to_bytes),
        )
    }

    /// `git_repository_set_ident`: the identity for reflogs (`None`: from
    /// the configuration), as [`ident`](Self::ident) returns it.
    pub fn set_ident(&mut self, name: Option<&[u8]>, email: Option<&[u8]>) -> Result<()> {
        let n = name.map(|n| c_string(n, "name")).transpose()?;
        let e = email.map(|e| c_string(e, "email")).transpose()?;
        Ok(self.raw().set_ident(n.as_deref(), e.as_deref())?)
    }

    /// `git_repository_hashfile`: the id the file at `path` would get as
    /// an object of type `kind`, filtered as the working-directory path
    /// `as_path` (`Some(b"")`: no filters; `None`: as `path`, which is then
    /// relative to the working directory).
    pub fn hashfile(
        &self,
        path: impl AsRef<Path>,
        kind: crate::ObjectType,
        as_path: Option<&[u8]>,
    ) -> Result<Oid> {
        let p = path_c(path.as_ref())?;
        let a = as_path.map(|a| c_string(a, "path")).transpose()?;
        // The filters read attributes, which C may read from the index.
        Ok(crate::index::api::with_repository_index_locked(
            self.raw(),
            || head::hashfile(self.raw(), &p, kind.raw(), a.as_deref()),
        )?)
    }

    /// `git_repository_wrap_odb`: a repository with no directory over
    /// `odb` (its id type that of the odb; as C, not marked bare).
    ///
    /// The repository takes `odb` over: it must be made alone (`'static`)
    /// and be its only handle (no clone left),
    /// since the odb then reads the repository's object cache, which no
    /// handle that can outlive the repository may reach.
    pub fn wrap_odb(odb: Odb<'static>) -> Result<Repository> {
        // SAFETY: `odb` holds a reference to the live odb for the call.
        unsafe { sole(odb.as_raw(), "odb") }?;
        crate::libgit2::ensure_init()?;
        Ok(Repository::from_nonnull(open::wrap_odb(&odb)?))
    }

    /// `git_repository_set_odb`: replace the repository's object database.
    ///
    /// `odb` must be made alone (`'static`: an odb of a repository is that
    /// repository's) and be its only handle (no clone left): the repository
    /// owns it from
    /// now on (the odb reads the repository's object cache), so no handle
    /// that may outlive the repository may be left. Get it back with
    /// [`odb`](Self::odb). `&mut self`: no handle of the old odb is live.
    pub fn set_odb(&mut self, odb: Odb<'static>) -> Result<()> {
        let p = odb.as_raw();
        // SAFETY: `odb` holds a reference to the live odb for the call.
        unsafe { sole(p, "odb") }?;
        // SAFETY: `odb` is live (we hold its only reference until the end
        // of the call); `&mut self`: no borrow of the repository, so no
        // handle or weak pointer of the old odb, is live.
        unsafe { self.raw().set_odb(Some(p)) };
        Ok(())
    }

    /// `git_repository_set_config`: replace the repository's configuration.
    ///
    /// As [`set_odb`](Self::set_odb): `config` must be made alone
    /// (`'static`: a repository's configuration, and its snapshots, keep
    /// that repository for their include conditions) and be its only
    /// handle; its owner is the repository from now on (setting a value
    /// clears the repository's configuration cache).
    pub fn set_config(&mut self, config: Config<'static>) -> Result<()> {
        // The pointer with the allocation's provenance: the repository
        // keeps it and frees through it when it drops the last count (one
        // made from `config.inner()`, a `&GitConfig`, may not be freed
        // through).
        let config = config.into_shared();
        let p = NonNull::new(ffibox::CShared::as_ptr(&config)).expect("a CShared is not NULL");
        // SAFETY: `config` holds a reference to the live config for the
        // call.
        unsafe { sole(p, "config") }?;
        // SAFETY: as in `set_odb`.
        unsafe { self.raw().set_config(Some(p)) };
        Ok(())
    }

    /// `git_repository_set_index`: replace the repository's index.
    ///
    /// As [`set_odb`](Self::set_odb): `index` must be made alone
    /// ([`Index::new`], [`Index::open`]) and be its only handle (no clone
    /// or iterator left): the index reads
    /// its owner's working directory and configuration, so no handle that
    /// may outlive the repository may be left. Get it back with
    /// [`index`](Self::index).
    pub fn set_index(&mut self, index: Index<'static>) -> Result<()> {
        let p = index.as_nonnull();
        // SAFETY: `index` holds a reference to the live index for the call.
        unsafe { sole(p, "index") }?;
        // The safe API's lock of the index is held for the change of owner.
        index.locked(|_| {
            // SAFETY: as in `set_odb`; and no other thread reaches the
            // index (ours is its only reference, and its lock is held).
            unsafe { self.raw().set_index(Some(p)) }
        });
        Ok(())
    }

    /// `git_repository__cleanup`: drop the repository's caches, grafts and
    /// its odb, configuration, refdb and index (made again on next use).
    ///
    /// `&mut self`: no handle of what it drops is live.
    pub fn cleanup(&mut self) -> Result<()> {
        // SAFETY: `&mut self`: no borrow of the repository, so no handle,
        // weak pointer or cache entry of it, is live or used after.
        Ok(unsafe { self.raw().cleanup() }?)
    }

    /// `git_repository_submodule_cache_all`: load the submodules into a
    /// cache that later submodule lookups use (until
    /// [`submodule_cache_clear`](Self::submodule_cache_clear)).
    ///
    /// `&mut self`: C fills the cache without synchronisation, and reads
    /// the index.
    pub fn submodule_cache_all(&mut self) -> Result<()> {
        let r = crate::sys::submodule::cache_init(self.raw());
        ErrorCode::check(r)?;
        Ok(())
    }

    /// `git_repository_submodule_cache_clear`.
    ///
    /// `&mut self`: no lookup may use the cache meanwhile.
    pub fn submodule_cache_clear(&mut self) -> Result<()> {
        ErrorCode::check(self.raw().submodule_cache_clear())?;
        Ok(())
    }

    /// `git_repository_fetchhead_foreach`: `f(ref_name, remote_url, id,
    /// is_merge)` for each line of `FETCH_HEAD` (the names `None` when the
    /// line has none); `false` stops (and the call returns `Ok`). A panic
    /// in `f` stops and is resumed after.
    pub fn fetchhead_foreach<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(Option<&[u8]>, Option<&[u8]>, &Oid, bool) -> bool,
    {
        type Each<'a> = dyn FnMut(Option<&[u8]>, Option<&[u8]>, &Oid, bool) -> bool + 'a;
        struct State<'a> {
            f: &'a mut Each<'a>,
            cb: CallbackState,
        }
        unsafe extern "C" fn each(
            ref_name: *const c_char,
            remote_url: *const c_char,
            oid: *const Oid,
            is_merge: core::ffi::c_uint,
            payload: *mut core::ffi::c_void,
        ) -> c_int {
            // SAFETY: the payload is the `State` of the running call, used
            // by nothing else during the callback.
            let st = unsafe { &mut *payload.cast::<State<'_>>() };
            // SAFETY: C passes NULL or strings, live for the call.
            let n = unsafe { crate::util::export::cstr(ref_name) }.map(CStr::to_bytes);
            // SAFETY: as above.
            let u = unsafe { crate::util::export::cstr(remote_url) }.map(CStr::to_bytes);
            // SAFETY: C passes a valid id, live for the call.
            let id = unsafe { &*oid };
            let f = &mut st.f;
            st.cb.call(|| f(n, u, id, is_merge != 0))
        }
        let mut st = State {
            f: &mut f,
            cb: CallbackState::default(),
        };
        // SAFETY: `each` treats the payload as the `State`, which outlives
        // the call.
        let r = unsafe {
            crate::sys::fetchhead::fetchhead_foreach(self.raw(), each, (&raw mut st).cast())
        };
        st.cb.finish(r)
    }

    /// `git_repository_mergehead_foreach`: `f(id)` for each id of
    /// `MERGE_HEAD`; `false` stops (and the call returns `Ok`). A panic in
    /// `f` stops and is resumed after.
    pub fn mergehead_foreach<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(&Oid) -> bool,
    {
        struct State<'a> {
            f: &'a mut dyn FnMut(&Oid) -> bool,
            cb: CallbackState,
        }
        unsafe extern "C" fn each(oid: *const Oid, payload: *mut core::ffi::c_void) -> c_int {
            // SAFETY: the payload is the `State` of the running call, used
            // by nothing else during the callback.
            let st = unsafe { &mut *payload.cast::<State<'_>>() };
            // SAFETY: C passes a valid id, live for the call.
            let id = unsafe { &*oid };
            let f = &mut st.f;
            st.cb.call(|| f(id))
        }
        let mut st = State {
            f: &mut f,
            cb: CallbackState::default(),
        };
        // SAFETY: `each` treats the payload as the `State`, which outlives
        // the call.
        let r =
            unsafe { crate::sys::merge::mergehead_foreach(self.raw(), each, (&raw mut st).cast()) };
        st.cb.finish(r)
    }

    /// `git_repository_head_for_worktree`: `f` gets the HEAD of the
    /// worktree `name`, resolved, while that worktree's repository is open
    /// (C returns the reference after closing that repository, which it
    /// still points to; so the reference is lent to `f` only).
    pub fn head_for_worktree<T>(
        &self,
        name: impl AsRef<[u8]>,
        f: impl FnOnce(&Reference<'_>) -> T,
    ) -> Result<T> {
        let n = c_string(name.as_ref(), "worktree name")?;
        let r = head::with_head_for_worktree(self.raw(), &n, |h| {
            // SAFETY: the reference `with_head_for_worktree` lends, ours to
            // own for the extent of `f` (its repository is open meanwhile).
            unsafe { crate::refs::RefBox::from_raw(h.into_raw()) }.map(|b| f(&Reference::wrap(b)))
        })?;
        r.ok_or_else(|| ErrorCode::GENERIC.into())
    }

    /// `git_repository_head_detached_for_worktree`.
    pub fn head_detached_for_worktree(&self, name: impl AsRef<[u8]>) -> Result<bool> {
        let n = c_string(name.as_ref(), "worktree name")?;
        Ok(head::head_detached_for_worktree(self.raw(), &n)?)
    }
}

/// The extensions this library supports (`GIT_OPT_GET_EXTENSIONS`; the
/// public form is in `opts`).
pub(crate) fn extensions() -> Vec<Vec<u8>> {
    open::extensions().unwrap_or_default()
}

/// Set the extensions this library supports beyond the built-in ones (a
/// leading `!` rejects one) (`GIT_OPT_SET_EXTENSIONS`; the public form is in
/// `opts`).
pub(crate) fn set_extensions(exts: &[&[u8]]) {
    open::set_extensions(exts);
}

/// Whether `p`'s reference (held by the caller) is the only one and no
/// repository owns the object; otherwise the error of the setters.
///
/// # Safety
///
/// `p` is a live refcounted object (a `git_refcount` at offset 0) on which
/// the caller holds a reference for the call.
unsafe fn sole<T>(p: NonNull<T>, what: &str) -> Result<()> {
    // SAFETY: the caller's guarantee: a live object whose count (at offset
    // 0) we only read.
    if unsafe { crate::util::thread::GitRefcount::sole(crate::util::thread::refcount_of(p)) } {
        return Ok(());
    }
    Err(Error::with(
        ErrorCode::GENERIC,
        crate::util::errors::ErrorClass::INVALID,
        format!("the {what} is shared: it has another handle, or a repository owns it"),
    ))
}

/// The state of a closure called by C: whether it stopped, and its panic.
#[derive(Default)]
struct CallbackState {
    stopped: bool,
    panic: Option<Box<dyn core::any::Any + Send>>,
}

impl CallbackState {
    /// Run the closure (`false` stops), as C's callback status.
    fn call(&mut self, f: impl FnOnce() -> bool) -> c_int {
        if self.stopped || self.panic.is_some() {
            return ErrorCode::USER.raw();
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(true) => 0,
            Ok(false) => {
                self.stopped = true;
                ErrorCode::USER.raw()
            }
            Err(p) => {
                self.panic = Some(p);
                ErrorCode::USER.raw()
            }
        }
    }

    /// The operation's result `r`: the panic resumed, `Ok` if stopped.
    fn finish(self, r: c_int) -> Result<()> {
        if let Some(p) = self.panic {
            std::panic::resume_unwind(p);
        }
        if self.stopped {
            crate::util::errors::clear();
            return Ok(());
        }
        ErrorCode::check(r)?;
        Ok(())
    }
}
