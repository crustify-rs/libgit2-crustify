//! The safe Rust API of repositories (PORTING.md §5).
//!
//! [`Repository`] owns a `git_repository` and frees it on drop. Opening,
//! discovering and initializing have plain functions and builders
//! ([`Repository::open_ext`], [`RepositoryInitOptions`]). The lazily
//! created odb, configuration, refdb and index come back as owners that
//! hold their own reference ([`Odb`], [`Config`], [`Refdb`], [`Index`]).
//!
//! A `Repository` is `Send`, not `Sync`: libgit2 shares a repository
//! between threads only for some operations, and the C that stays updates
//! some of its caches without synchronisation. The setters that replace
//! the paths take `&mut self`, so no path borrowed from it can dangle.
//!
//! Paths are bytes: they are `&Path` on Unix (`OsStr` bytes).

use core::ffi::{CStr, c_int};
use core::ptr::NonNull;
use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::handles::{Config, Index, Odb, Refdb, Reference};
use super::head::{self, state};
use super::init::{InitOptions, InitOut};
use super::*;
use crate::oid::{Oid, OidType};
use crate::util::errors::{Error, ErrorCode};
use crate::util::str::GitStr;

type Result<T> = core::result::Result<T, Error>;

fn path_c(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| {
        let _ = crate::git_error_set!(
            crate::util::errors::ErrorClass::INVALID,
            "path contains a NUL byte"
        );
        Error::from(ErrorCode::GENERIC)
    })
}

fn to_path(c: &CStr) -> &Path {
    Path::new(OsStr::from_bytes(c.to_bytes()))
}

fn str_path(s: &GitStr) -> PathBuf {
    PathBuf::from(OsStr::from_bytes(crate::util::fs_path::str_c_bytes(s)))
}

/// A tiny bitflags: a `Copy` newtype over the bits with `|`, `contains`,
/// `bits` and `empty`.
macro_rules! bitflags_lite {
    ($(#[$m:meta])* pub struct $name:ident: $t:ty { $($(#[$fm:meta])* $f:ident = $v:expr;)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name($t);
        impl $name {
            $($(#[$fm])* pub const $f: $name = $name($v);)*
            /// No flag.
            pub const fn empty() -> $name { $name(0) }
            /// The raw bits.
            pub const fn bits(self) -> $t { self.0 }
            /// Whether all of `other`'s flags are set.
            pub const fn contains(self, other: $name) -> bool { self.0 & other.0 == other.0 }
        }
        impl core::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, o: $name) -> $name { $name(self.0 | o.0) }
        }
        impl core::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, o: $name) { self.0 |= o.0 }
        }
    };
}

bitflags_lite! {
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


bitflags_lite! {
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

/// Options of [`Repository::init_opts`] (`git_repository_init_options`).
#[derive(Clone, Debug, Default)]
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

impl RepositoryInitOptions {
    /// The defaults: `MKPATH` (as `git_repository_init`).
    pub fn new() -> RepositoryInitOptions {
        RepositoryInitOptions {
            flags: RepositoryInitFlags::MKPATH,
            ..Default::default()
        }
    }
    /// The flags.
    pub fn flags(&mut self, f: RepositoryInitFlags) -> &mut Self {
        self.flags = f;
        self
    }
    /// A bare repository.
    pub fn bare(&mut self, bare: bool) -> &mut Self {
        if bare {
            self.flags |= RepositoryInitFlags::BARE;
        } else {
            self.flags = RepositoryInitFlags(self.flags.0 & !GIT_REPOSITORY_INIT_BARE);
        }
        self
    }
    /// The shared mode.
    pub fn mode(&mut self, m: RepositoryInitMode) -> &mut Self {
        self.mode = m;
        self
    }
    /// The working directory.
    pub fn workdir_path(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.workdir_path = Some(p.as_ref().to_owned());
        self
    }
    /// The description file's contents.
    pub fn description(&mut self, d: impl AsRef<[u8]>) -> &mut Self {
        self.description = Some(d.as_ref().to_vec());
        self
    }
    /// The template directory.
    pub fn template_path(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.template_path = Some(p.as_ref().to_owned());
        self
    }
    /// The initial HEAD (a branch name, or a `refs/…` name).
    pub fn initial_head(&mut self, h: impl AsRef<[u8]>) -> &mut Self {
        self.initial_head = Some(h.as_ref().to_vec());
        self
    }
    /// Add an `origin` remote with this URL.
    pub fn origin_url(&mut self, u: impl AsRef<[u8]>) -> &mut Self {
        self.origin_url = Some(u.as_ref().to_vec());
        self
    }
    /// The object id type.
    pub fn oid_type(&mut self, t: OidType) -> &mut Self {
        self.oid_type = Some(t);
        self
    }
    /// The reference storage.
    pub fn ref_storage(&mut self, r: RefStorage) -> &mut Self {
        self.ref_storage = Some(r);
        self
    }

    fn to_raw(&self) -> Result<InitOptions> {
        let bytes = |b: &Option<Vec<u8>>| -> Result<Option<CString>> {
            b.as_ref()
                .map(|v| CString::new(v.clone()).map_err(|_| Error::from(ErrorCode::GENERIC)))
                .transpose()
        };
        let path = |p: &Option<PathBuf>| p.as_deref().map(path_c).transpose();
        Ok(InitOptions {
            flags: self.flags.bits(),
            mode: self.mode.raw(),
            workdir_path: path(&self.workdir_path)?,
            description: bytes(&self.description)?,
            template_path: path(&self.template_path)?,
            initial_head: bytes(&self.initial_head)?,
            origin_url: bytes(&self.origin_url)?,
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
pub struct Repository(NonNull<GitRepository>);

// SAFETY: the repository and what it owns are released from any thread,
// and it is used by one owner at a time (it is not `Sync`).
unsafe impl Send for Repository {}

impl Drop for Repository {
    fn drop(&mut self) {
        // SAFETY: the repository this owner holds alone.
        unsafe { GitRepository::free(self.0.as_ptr()) }
    }
}

impl core::fmt::Debug for Repository {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Repository").field("path", &self.path()).finish()
    }
}

impl Repository {
    fn from_nonnull(p: NonNull<GitRepository>) -> Repository {
        Repository(p)
    }

    fn raw(&self) -> &GitRepository {
        // SAFETY: the repository this owner holds, live until drop.
        unsafe { self.0.as_ref() }
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
        let ceil = if ceiling_dirs.is_empty() {
            None
        } else {
            let mut v = Vec::new();
            for (i, d) in ceiling_dirs.iter().enumerate() {
                if i > 0 {
                    v.push(crate::util::fs_path::GIT_PATH_LIST_SEPARATOR);
                }
                v.extend_from_slice(d.as_os_str().as_bytes());
            }
            Some(CString::new(v).map_err(|_| Error::from(ErrorCode::GENERIC))?)
        };
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
        Ok(Repository::from_nonnull(open::open_bare(&p)?))
    }

    /// `git_repository_discover`: the git directory of the repository that
    /// contains `start`.
    pub fn discover(start: impl AsRef<Path>, across_fs: bool, ceiling_dirs: &[&Path]) -> Result<PathBuf> {
        let s = path_c(start.as_ref())?;
        let ceil = if ceiling_dirs.is_empty() {
            None
        } else {
            let joined = ceiling_dirs
                .iter()
                .map(|d| d.as_os_str().as_bytes())
                .collect::<Vec<_>>()
                .join(&b':');
            Some(CString::new(joined).map_err(|_| Error::from(ErrorCode::GENERIC))?)
        };
        let g = open::discover(&s, across_fs, ceil.as_deref())?;
        Ok(str_path(&g))
    }

    /// `git_repository_init`.
    pub fn init(path: impl AsRef<Path>, bare: bool) -> Result<Repository> {
        let mut o = RepositoryInitOptions::new();
        o.bare(bare);
        Repository::init_opts(path, &o)
    }

    /// `git_repository_init_ext`.
    pub fn init_opts(path: impl AsRef<Path>, opts: &RepositoryInitOptions) -> Result<Repository> {
        let p = path_c(path.as_ref())?;
        let mut raw = opts.to_raw()?;
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
        Ok(Repository::from_nonnull(open::new_ext(Some(oid_type))?))
    }

    /// `git_repository_wrap_odb`: a repository over an object database.
    pub fn wrap_odb(odb: &Odb) -> Result<Repository> {
        Ok(Repository::from_nonnull(open::wrap_odb(odb)?))
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
    pub fn head(&self) -> Result<Reference> {
        Ok(head::head(self.raw())?)
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

    /// `git_repository_set_head`.
    pub fn set_head(&self, refname: &str) -> Result<()> {
        let n = CString::new(refname).map_err(|_| Error::from(ErrorCode::GENERIC))?;
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
        item_path(&mut s, &self.raw().item_dirs(), item as ItemT)?;
        Ok(str_path(&s))
    }

    /// `git_repository_odb`.
    pub fn odb(&self) -> Result<Odb> {
        let p = self.raw().odb_weak()?;
        // SAFETY: the repository's live odb; a new reference.
        Ok(unsafe { Odb::new_ref(p) })
    }

    /// `git_repository_config`.
    pub fn config(&self) -> Result<Config> {
        let c = self.raw().config_weak()?;
        // SAFETY: the repository's live config; a new reference.
        Ok(unsafe { Config::new_ref(NonNull::new_unchecked(c.as_ptr())) })
    }

    /// `git_repository_config_snapshot`.
    pub fn config_snapshot(&self) -> Result<Config> {
        Ok(self.raw().config_weak()?.snapshot()?)
    }

    /// `git_repository_refdb`.
    pub fn refdb(&self) -> Result<Refdb> {
        let p = self.raw().refdb_weak()?;
        // SAFETY: the repository's live refdb; a new reference.
        Ok(unsafe { Refdb::new_ref(p) })
    }

    /// `git_repository_index`.
    pub fn index(&self) -> Result<Index> {
        let p = self.raw().index_weak()?;
        // SAFETY: the repository's live index; a new reference.
        Ok(unsafe { Index::new_ref(p) })
    }

    /// `git_repository_set_odb`.
    pub fn set_odb(&self, odb: &Odb) {
        self.raw().set_odb(Some(odb.as_nonnull()));
    }

    /// `git_repository_set_config`.
    pub fn set_config(&self, config: &Config) {
        self.raw().set_config(Some(config.as_nonnull()));
    }

    /// `git_repository_set_index` (`None` drops the index).
    pub fn set_index(&self, index: Option<&Index>) {
        self.raw().set_index(index.map(Index::as_nonnull));
    }

    /// `git_repository_set_workdir`.
    pub fn set_workdir(&mut self, workdir: impl AsRef<Path>, update_gitlink: bool) -> Result<()> {
        let w = path_c(workdir.as_ref())?;
        Ok(init::set_workdir(self.raw(), &w, update_gitlink)?)
    }

    /// `git_repository_set_bare`.
    pub fn set_bare(&mut self) -> Result<()> {
        Ok(init::set_bare(self.raw())?)
    }

    /// `git_repository_reinit_filesystem`.
    pub fn reinit_filesystem(&self, recurse: bool) -> Result<()> {
        Ok(init::reinit_filesystem(self.raw(), recurse)?)
    }

    /// `git_repository_get_namespace`.
    pub fn namespace(&self) -> Option<&[u8]> {
        self.raw().namespace().map(CStr::to_bytes)
    }

    /// `git_repository_set_namespace`.
    pub fn set_namespace(&mut self, ns: Option<&[u8]>) -> Result<()> {
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

    /// `git_repository_set_ident`.
    pub fn set_ident(&mut self, name: Option<&str>, email: Option<&str>) -> Result<()> {
        let n = name.map(CString::new).transpose().map_err(|_| Error::from(ErrorCode::GENERIC))?;
        let e = email.map(CString::new).transpose().map_err(|_| Error::from(ErrorCode::GENERIC))?;
        Ok(self.raw().set_ident(n.as_deref(), e.as_deref())?)
    }

    /// `git_repository_hashfile`: the id `path` would get as an object of
    /// type `object_type` (`GIT_OBJECT_BLOB` = 3), filtered as `as_path`
    /// (`Some("")`: no filters; `None`: as `path`).
    pub fn hashfile(&self, path: impl AsRef<Path>, object_type: i32, as_path: Option<&Path>) -> Result<Oid> {
        let p = path_c(path.as_ref())?;
        let a = as_path.map(path_c).transpose()?;
        Ok(head::hashfile(self.raw(), &p, object_type, a.as_deref())?)
    }

    /// The `git_repository *`, for C (still owned).
    pub(crate) fn as_raw(&self) -> &GitRepository {
        self.raw()
    }
}

/// The extensions this library supports (`GIT_OPT_GET_EXTENSIONS`).
pub fn extensions() -> Vec<Vec<u8>> {
    open::extensions().unwrap_or_default()
}

/// Set the extensions this library supports beyond the built-in ones (a
/// leading `!` rejects one) (`GIT_OPT_SET_EXTENSIONS`).
pub fn set_extensions(exts: &[&[u8]]) {
    open::set_extensions(exts);
}

#[allow(dead_code)]
fn _assert_send() {
    fn s<T: Send>() {}
    s::<Repository>();
}
