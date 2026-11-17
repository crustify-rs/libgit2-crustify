//! The safe Rust API of the index (PORTING.md §5), over the same Rust as
//! the C ABI.
//!
//! [`Index`] is a reference to a `git_index` (as `git_index *`): `Clone`
//! takes another reference (C refcounts the index) and `Drop` gives it up.
//! The clones are the same index; every operation of the safe API on an
//! index takes the index's lock (shared by all its handles, re-entrant for
//! callbacks), so an `Index` is `Send` and `Sync` (I-1). Entries come out
//! as owned copies ([`IndexEntry`]), so no borrow of the index's memory
//! survives a later call (which may free it). Callbacks are closures; they
//! may use the index again (the lock is re-entrant, and the operations hold
//! no borrow while they run). A panic in a callback stops the operation
//! and is resumed after it.

use core::ffi::{CStr, c_char, c_int};
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr::NonNull;
use ffibox::CShared;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::util::thread::{ReentrantGuard, ReentrantLock, shared_nn};
use std::ffi::CString;
use std::path::{Path, PathBuf};

use super::foreign::{GitStrArrayBuf, Paths};
use super::{
    EntryPtr, GIT_INDEX_ADD_CHECK_PATHSPEC, GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH,
    GIT_INDEX_ADD_FORCE, GIT_INDEX_CAPABILITY_FROM_OWNER, GIT_INDEX_CAPABILITY_IGNORE_CASE,
    GIT_INDEX_CAPABILITY_NO_FILEMODE, GIT_INDEX_CAPABILITY_NO_SYMLINKS, GIT_INDEX_STAGE_ANY,
    GitIndex, GitIndexEntry, GitIndexTime, IndexPtr, epath, ops,
};
use crate::oid::{Oid, OidType};
use crate::util::errors::{Error, ErrorCode};

type Result<T> = core::result::Result<T, Error>;

/// The rich error of a failed operation (its code and the thread's
/// message).
fn err(code: ErrorCode) -> Error {
    Error::from(code)
}

/// A status as a result.
fn check(code: c_int) -> Result<()> {
    if code < 0 {
        Err(err(ErrorCode::from_raw(code).unwrap_or(ErrorCode::GENERIC)))
    } else {
        Ok(())
    }
}

/// A path as the index's bytes.
fn path_bytes(p: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes()
}

/// A NUL-terminated copy of `b`; a NUL within is an invalid path.
fn c_path(b: &[u8]) -> Result<CString> {
    CString::new(b).map_err(|_| {
        Error::with(
            ErrorCode::GENERIC,
            crate::util::errors::ErrorClass::INDEX,
            format!("invalid path: '{}'", String::from_utf8_lossy(b)),
        )
    })
}

/// An index stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Any stage (`GIT_INDEX_STAGE_ANY`), for lookups.
    Any,
    /// A normal staged file (`GIT_INDEX_STAGE_NORMAL`).
    Normal,
    /// The ancestor side of a conflict.
    Ancestor,
    /// The "ours" side of a conflict.
    Ours,
    /// The "theirs" side of a conflict.
    Theirs,
}

impl Stage {
    /// The C value.
    pub(crate) fn raw(self) -> c_int {
        match self {
            Stage::Any => GIT_INDEX_STAGE_ANY,
            Stage::Normal => 0,
            Stage::Ancestor => 1,
            Stage::Ours => 2,
            Stage::Theirs => 3,
        }
    }

    /// The stage of a C value (0 to 3).
    pub(crate) fn from_raw(v: c_int) -> Option<Stage> {
        match v {
            0 => Some(Stage::Normal),
            1 => Some(Stage::Ancestor),
            2 => Some(Stage::Ours),
            3 => Some(Stage::Theirs),
            -1 => Some(Stage::Any),
            _ => None,
        }
    }
}

/// An index entry's time (`git_index_time`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IndexTime {
    /// Seconds (truncated to 32 bits).
    pub seconds: i32,
    /// Nanoseconds.
    pub nanoseconds: u32,
}

impl From<GitIndexTime> for IndexTime {
    fn from(t: GitIndexTime) -> IndexTime {
        IndexTime {
            seconds: t.seconds,
            nanoseconds: t.nanoseconds,
        }
    }
}

impl From<IndexTime> for GitIndexTime {
    fn from(t: IndexTime) -> GitIndexTime {
        GitIndexTime {
            seconds: t.seconds,
            nanoseconds: t.nanoseconds,
        }
    }
}

/// An index entry (`git_index_entry`), owned.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IndexEntry {
    /// The change time.
    pub ctime: IndexTime,
    /// The modification time.
    pub mtime: IndexTime,
    /// `dev`.
    pub dev: u32,
    /// `ino`.
    pub ino: u32,
    /// The file mode.
    pub mode: u32,
    /// `uid`.
    pub uid: u32,
    /// `gid`.
    pub gid: u32,
    /// The size (truncated to 32 bits).
    pub file_size: u32,
    /// The object id.
    pub id: Oid,
    /// `flags`: the name length, stage, `EXTENDED` and `VALID` bits.
    pub flags: u16,
    /// `flags_extended`.
    pub flags_extended: u16,
    /// The path (bytes).
    pub path: Vec<u8>,
}

impl IndexEntry {
    /// An entry for `path` with mode `mode` and id `id`, the rest zero.
    pub fn new(path: impl Into<Vec<u8>>, mode: u32, id: Oid) -> IndexEntry {
        IndexEntry {
            ctime: IndexTime::default(),
            mtime: IndexTime::default(),
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size: 0,
            id,
            flags: 0,
            flags_extended: 0,
            path: path.into(),
        }
    }

    /// The stage (`git_index_entry_stage`).
    pub fn stage(&self) -> Stage {
        Stage::from_raw(c_int::from(
            (self.flags & super::GIT_INDEX_ENTRY_STAGEMASK) >> super::GIT_INDEX_ENTRY_STAGESHIFT,
        ))
        .unwrap_or(Stage::Normal)
    }

    /// Set the stage (`GIT_INDEX_ENTRY_STAGE_SET`); `Any` sets 3, as C's
    /// mask of -1.
    pub fn set_stage(&mut self, stage: Stage) {
        let s = (stage.raw() & 3) as u16;
        self.flags = (self.flags & !super::GIT_INDEX_ENTRY_STAGEMASK)
            | (s << super::GIT_INDEX_ENTRY_STAGESHIFT);
    }

    /// Whether the entry is a side of a conflict
    /// (`git_index_entry_is_conflict`).
    pub fn is_conflict(&self) -> bool {
        !matches!(self.stage(), Stage::Normal)
    }

    /// A copy of an index entry.
    fn from_ptr(e: EntryPtr) -> IndexEntry {
        // SAFETY: the callers pass a live entry of an index, which nothing
        // writes during the copy (no foreign call).
        let r = unsafe { (*e.as_ptr()).entry };
        // SAFETY: as above.
        let path = unsafe { epath(e) };
        IndexEntry {
            ctime: r.ctime.into(),
            mtime: r.mtime.into(),
            dev: r.dev,
            ino: r.ino,
            mode: r.mode,
            uid: r.uid,
            gid: r.gid,
            file_size: r.file_size,
            id: r.id,
            flags: r.flags,
            flags_extended: r.flags_extended,
            path: path.to_vec(),
        }
    }

    /// The C entry over `path` (a NUL-terminated copy of `self.path`).
    fn raw(&self, path: &CStr) -> GitIndexEntry {
        GitIndexEntry {
            ctime: self.ctime.into(),
            mtime: self.mtime.into(),
            dev: self.dev,
            ino: self.ino,
            mode: self.mode,
            uid: self.uid,
            gid: self.gid,
            file_size: self.file_size,
            id: self.id,
            flags: self.flags,
            flags_extended: self.flags_extended,
            path: path.as_ptr(),
        }
    }
}

crate::api::flags::bitflags! {
    /// The capabilities of an index (`git_index_capability_t`).
    pub struct IndexCapability: u32 {
        /// Paths compare without case (`GIT_INDEX_CAPABILITY_IGNORE_CASE`).
        IGNORE_CASE = GIT_INDEX_CAPABILITY_IGNORE_CASE as u32;
        /// File modes are not trusted (`GIT_INDEX_CAPABILITY_NO_FILEMODE`).
        NO_FILEMODE = GIT_INDEX_CAPABILITY_NO_FILEMODE as u32;
        /// Symbolic links are not supported
        /// (`GIT_INDEX_CAPABILITY_NO_SYMLINKS`).
        NO_SYMLINKS = GIT_INDEX_CAPABILITY_NO_SYMLINKS as u32;
    }
}

crate::api::flags::bitflags! {
    /// Flags of [`Index::add_all`] (`git_index_add_option_t`).
    pub struct IndexAddOption: u32 {
        /// Add ignored files too (`GIT_INDEX_ADD_FORCE`).
        FORCE = GIT_INDEX_ADD_FORCE;
        /// Match the pathspec exactly
        /// (`GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH`).
        DISABLE_PATHSPEC_MATCH = GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH;
        /// Fail on a pathspec naming an ignored file
        /// (`GIT_INDEX_ADD_CHECK_PATHSPEC`).
        CHECK_PATHSPEC = GIT_INDEX_ADD_CHECK_PATHSPEC;
    }
}

/// The three sides of a conflict.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictEntries {
    /// The common ancestor.
    pub ancestor: Option<IndexEntry>,
    /// Our side.
    pub our: Option<IndexEntry>,
    /// Their side.
    pub their: Option<IndexEntry>,
}

impl ConflictEntries {
    fn from_ptrs(s: [Option<EntryPtr>; 3]) -> ConflictEntries {
        ConflictEntries {
            ancestor: s[0].map(IndexEntry::from_ptr),
            our: s[1].map(IndexEntry::from_ptr),
            their: s[2].map(IndexEntry::from_ptr),
        }
    }
}

/// A resolve-undo record (`git_index_reuc_entry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReucEntry {
    /// The path.
    pub path: Vec<u8>,
    /// The ancestor's, ours and theirs modes (0: absent).
    pub mode: [u32; 3],
    /// Their ids.
    pub oid: [Oid; 3],
}

/// A conflict-name record (`git_index_name_entry`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameEntry {
    /// The ancestor's path.
    pub ancestor: Option<Vec<u8>>,
    /// Our path.
    pub ours: Option<Vec<u8>>,
    /// Their path.
    pub theirs: Option<Vec<u8>>,
}

/// The next conflict from position `*cur` (`git_index_conflict_next`).
pub(crate) fn conflict_next(
    index: &mut GitIndex,
    cur: &mut usize,
) -> Option<[Option<EntryPtr>; 3]> {
    while *cur < index.entries.len() {
        index.sort();
        let e = index.entry_at(*cur)?;
        // SAFETY: a live entry of the index.
        if unsafe { super::estage(e) } > 0 {
            let (sides, len) = index.conflict_at(*cur);
            *cur += len;
            return Some(sides);
        }
        *cur += 1;
    }
    None
}

/// The safe API's lock of each live index, by address (S-1, I-1).
///
/// The lock is kept beside the index, not in it: holders reach the index's
/// memory through `&mut GitIndex` under the lock, so the lock itself must
/// not be part of that memory. An index's entry lives while a handle (or
/// an operation) holds its lock; a later handle of the same index makes a
/// new one if none is left (no handle, so no holder, of the old one).
fn api_lock_of(ix: NonNull<GitIndex>) -> Arc<ReentrantLock> {
    static LOCKS: Mutex<Option<HashMap<usize, Weak<ReentrantLock>>>> = Mutex::new(None);
    let mut g = LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
    let map = g.get_or_insert_with(HashMap::new);
    let key = ix.as_ptr().addr();
    if let Some(l) = map.get(&key).and_then(Weak::upgrade) {
        return l;
    }
    map.retain(|_, w| w.strong_count() > 0);
    let l = Arc::new(ReentrantLock::new());
    map.insert(key, Arc::downgrade(&l));
    l
}

/// Run `f` with the safe API's lock of the index `ix` held (for the
/// repository's own changes to an index that handles may share, such as
/// the capabilities it sets when it loads the index).
pub(crate) fn with_index_locked<R>(ix: NonNull<GitIndex>, f: impl FnOnce() -> R) -> R {
    let lock = api_lock_of(ix);
    let _g = lock.lock();
    f()
}

/// Run `f` with the safe API's lock of `repo`'s index held (loading the
/// index if it is not yet): for the safe API's operations whose C reads
/// the repository's index (attributes for filters, …), so that they do not
/// run while another thread changes it through an [`Index`]. If the index
/// cannot be loaded, the C cannot reach one either; `f` runs unlocked.
pub(crate) fn with_repository_index_locked<R>(
    repo: &crate::repository::GitRepository,
    f: impl FnOnce() -> R,
) -> R {
    let saved = crate::util::errors::save();
    let lock = repo.index_weak().ok().map(api_lock_of);
    crate::util::errors::restore(saved);
    let _g = lock.as_ref().map(|l| l.lock());
    // The same operations' C (attributes, ignores) reads the search paths:
    // hold their setters off too.
    crate::sys::sysdir::reading(f)
}

/// An index (`git_index`): a reference to it, freed with the last one.
///
/// `Clone` takes another reference to the same index (C refcounts it).
/// The handles of one index, and the safe API's operations that read a
/// repository's index, serialise on one lock, so an `Index` is `Send` and
/// `Sync`: threads may share it (each call is atomic with respect to the
/// others). An index from [`Repository::index`](crate::Repository::index)
/// borrows the repository (`'r`); one made alone is `Index<'static>`.
pub struct Index<'r> {
    /// The reference; taken and dropped with `lock` held (`Clone`,
    /// `Drop`), since the count is in the index's memory, which the
    /// safe API's operations borrow exclusively under the lock.
    ix: ManuallyDrop<CShared<GitIndex>>,
    lock: Arc<ReentrantLock>,
    _repo: PhantomData<&'r ()>,
}

impl Clone for Index<'_> {
    fn clone(&self) -> Self {
        let ix = {
            let _g = self.api();
            CShared::clone(&self.ix)
        };
        Index {
            ix: ManuallyDrop::new(ix),
            lock: Arc::clone(&self.lock),
            _repo: PhantomData,
        }
    }
}

impl Drop for Index<'_> {
    fn drop(&mut self) {
        let lock = &self.lock;
        let _g = lock.lock();
        // SAFETY: dropped once, here; the field is not used after.
        unsafe { ManuallyDrop::drop(&mut self.ix) };
    }
}

// SAFETY: the index is released (its count dropped, and freed with the
// last one) from any thread (`GitRefcount` is atomic). Every access of the
// safe API to the index's memory happens with `lock` held, which every
// handle of the same index shares (`api_lock_of`), and so do the safe
// API's other operations that reach the repository's index from C
// (`with_repository_index_locked`); the C ABI keeps C's rule (an index is
// used by one thread at a time).
unsafe impl Send for Index<'_> {}
// SAFETY: as above: `&Index` methods take the lock too.
unsafe impl Sync for Index<'_> {}

impl core::fmt::Debug for Index<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Index")
            .field("entries", &self.len())
            .finish()
    }
}

/// The closure of [`Index::add_all_with`] and friends.
type PathCallback<'a> = dyn FnMut(&[u8], Option<&[u8]>) -> IndexMatch + 'a;

/// What a path callback of [`Index::add_all`], [`Index::update_all`] and
/// [`Index::remove_all`] decides for a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexMatch {
    /// Do it for this path.
    Apply,
    /// Skip this path.
    Skip,
    /// Stop (the operation returns `Ok`).
    Stop,
}

impl<'r> Index<'r> {
    /// A handle of a reference `ix` the caller holds (and gives up), for
    /// an index no other safe user reaches yet, or with `lock` (the
    /// index's) held since the reference was taken.
    fn wrap_locked(ix: CShared<GitIndex>, lock: Arc<ReentrantLock>) -> Index<'r> {
        Index {
            ix: ManuallyDrop::new(ix),
            lock,
            _repo: PhantomData,
        }
    }

    fn wrap(ix: CShared<GitIndex>) -> Index<'r> {
        let lock = api_lock_of(shared_nn(&ix));
        Index::wrap_locked(ix, lock)
    }

    fn from_ptr(ptr: IndexPtr) -> Index<'r> {
        // SAFETY: `open_ext` gives up a reference to a live index.
        let ix = unsafe { CShared::from_raw(ptr.as_ptr()) }.expect("non-null");
        Index::wrap(ix)
    }

    /// The handle's lock, held for one operation.
    fn api(&self) -> ReentrantGuard<'_> {
        self.lock.lock()
    }

    /// The index, for the C-shaped operations (`ops`); used with the lock
    /// held ([`api`](Self::api)).
    fn ptr(&self) -> IndexPtr {
        // SAFETY: the index this reference keeps live; the callers hold the
        // lock, so no other safe user reaches it meanwhile.
        unsafe { IndexPtr::new(shared_nn(&self.ix)) }
    }

    /// A new reference to `raw`, as a handle.
    ///
    /// # Safety
    ///
    /// `raw` is a live index of this module, which stays live while the
    /// handle's lifetime `'r` (it is the repository's, borrowed for `'r`).
    pub(crate) unsafe fn new_ref(raw: NonNull<GitIndex>) -> Index<'r> {
        // The count is taken under the index's lock: other handles of the
        // index may be in use on other threads.
        let lock = api_lock_of(raw);
        let ix = {
            let _g = lock.lock();
            // SAFETY: the caller's contract.
            unsafe { CShared::from_borrowed(raw.as_ptr()) }.expect("non-null")
        };
        Index::wrap_locked(ix, lock)
    }

    /// The `git_index *` (crate-internal), live while `self` is; used with
    /// [`locked`](Self::locked).
    pub(crate) fn as_nonnull(&self) -> NonNull<GitIndex> {
        shared_nn(&self.ix)
    }

    /// Run `f` with this index's lock held (crate-internal: other modules'
    /// operations on an index, such as writing a tree from it).
    pub(crate) fn locked<R>(&self, f: impl FnOnce(NonNull<GitIndex>) -> R) -> R {
        let _g = self.api();
        f(self.as_nonnull())
    }

    fn with<R>(&self, f: impl FnOnce(&mut GitIndex) -> R) -> R {
        let _g = self.api();
        // SAFETY: `f` is this module's code over the index's own methods,
        // which make no foreign call and do not reenter this API; the lock
        // is held, so no other safe user reaches the index, and no borrow
        // outlives the call.
        f(unsafe { self.ptr().get() })
    }
    /// An in-memory index for SHA-1 ids (`git_index_new`).
    pub fn new() -> Result<Index<'static>> {
        Self::with_oid_type(OidType::Sha1)
    }

    /// An in-memory index for `oid_type` ids (`git_index_new_ext`).
    pub fn with_oid_type(oid_type: OidType) -> Result<Index<'static>> {
        crate::libgit2::ensure_init()?;
        ops::open_ext(None, oid_type)
            .map(Index::from_ptr)
            .map_err(err)
    }

    /// The index file at `path`, read if it exists, for SHA-1 ids
    /// (`git_index_open`).
    pub fn open(path: impl AsRef<Path>) -> Result<Index<'static>> {
        Self::open_with_oid_type(path, OidType::Sha1)
    }

    /// [`open`](Self::open) for `oid_type` ids (`git_index_open_ext`).
    pub fn open_with_oid_type(path: impl AsRef<Path>, oid_type: OidType) -> Result<Index<'static>> {
        crate::libgit2::ensure_init()?;
        let p = c_path(path_bytes(path.as_ref()))?;
        ops::open_ext(Some(&p), oid_type)
            .map(Index::from_ptr)
            .map_err(err)
    }

    /// The file's path (`git_index_path`), or `None` in memory.
    pub fn path(&self) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        self.with(|i| {
            i.file_path()
                .map(|p| PathBuf::from(std::ffi::OsStr::from_bytes(p.to_bytes())))
        })
    }

    /// The id type (`git_index_oid_type`).
    pub fn oid_type(&self) -> OidType {
        self.with(|i| i.oid_kind())
    }

    /// Re-read the file (`git_index_read`); with `force`, even when it did
    /// not change, discarding unsaved changes.
    pub fn read(&self, force: bool) -> Result<()> {
        let _g = self.api();
        ops::read(self.ptr(), force).map(|_| ()).map_err(err)
    }

    /// Write the index to its file (`git_index_write`).
    pub fn write(&self) -> Result<()> {
        let _g = self.api();
        ops::write(self.ptr()).map_err(err)
    }

    /// Replace the entries by those of `tree` (`git_index_read_tree`).
    pub fn read_tree(&self, tree: &crate::Tree<'_>) -> Result<()> {
        let _g = self.api();
        let t = tree.as_raw_tree().cast::<core::ffi::c_void>();
        // SAFETY: `tree` is live for the call (it is borrowed).
        ops::read_tree(self.ptr(), unsafe { super::foreign::TreeRef::new(t) }).map_err(err)
    }

    /// Write the entries as trees into the repository the index belongs
    /// to; the root tree's id (`git_index_write_tree`). An index made
    /// alone has no repository: use [`write_tree_to`](Self::write_tree_to).
    pub fn write_tree(&self) -> Result<Oid> {
        let _g = self.api();
        let ix = self.ptr();
        // SAFETY: the owner of the live index is NULL or its live
        // repository: the safe API's handles of an owned index borrow that
        // repository, and it clears the owner before it goes.
        let repo = unsafe { super::foreign::Repo::new(ix.owner()) };
        let Some(repo) = repo else {
            return Err(err(crate::util::errors::set_str(
                crate::util::errors::ErrorClass::INDEX,
                "Failed to write tree. the index file is not backed up by an existing repository",
            )));
        };
        // SAFETY: the lock is held and no reference into the index is live.
        unsafe { super::foreign::tree_write_index(ix, repo) }.map_err(err)
    }

    /// [`write_tree`](Self::write_tree) into `repo`'s object database
    /// (`git_index_write_tree_to`).
    pub fn write_tree_to(&self, repo: &crate::Repository) -> Result<Oid> {
        let _g = self.api();
        let ix = self.ptr();
        // SAFETY: a live repository, borrowed for the call.
        let repo = unsafe { super::foreign::Repo::new(repo.raw_ptr().cast()) }
            .ok_or_else(|| err(crate::util::errors::assert_arg_failed("repo")))?;
        // SAFETY: the lock is held and no reference into the index is live.
        unsafe { super::foreign::tree_write_index(ix, repo) }.map_err(err)
    }

    /// The number of entries (`git_index_entrycount`).
    pub fn len(&self) -> usize {
        self.with(|i| i.entries.len())
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.with(|i| i.dirty())
    }

    /// The `n`th entry, in path order (`git_index_get_byindex`).
    pub fn get(&self, n: usize) -> Option<IndexEntry> {
        self.with(|i| {
            i.sort();
            i.entry_at(n).map(IndexEntry::from_ptr)
        })
    }

    /// The entry for `path` at `stage` (`git_index_get_bypath`).
    pub fn get_path(&self, path: impl AsRef<[u8]>, stage: Stage) -> Option<IndexEntry> {
        let path = path.as_ref();
        self.with(|i| i.get_bypath(path, stage.raw()).map(IndexEntry::from_ptr))
    }

    /// The position of `path` (`git_index_find`).
    pub fn find(&self, path: impl AsRef<[u8]>) -> Option<usize> {
        let path = path.as_ref();
        self.with(|i| i.find_path(path))
    }

    /// The position of the first entry under `prefix`
    /// (`git_index_find_prefix`).
    pub fn find_prefix(&self, prefix: impl AsRef<[u8]>) -> Option<usize> {
        let prefix = prefix.as_ref();
        self.with(|i| i.find_prefix(prefix))
    }

    /// The entries, in path order, as of this call (`git_index_iterator`):
    /// later changes to the index do not show.
    pub fn iter(&self) -> IndexEntries<'r> {
        let _g = self.api();
        let snap = ops::snapshot_new(self.ptr()).ok();
        IndexEntries {
            index: self.clone(),
            snap,
            cur: 0,
        }
    }

    /// Add or replace an entry (`git_index_add`).
    pub fn add(&self, entry: &IndexEntry) -> Result<()> {
        let p = c_path(&entry.path)?;
        let _g = self.api();
        ops::add(self.ptr(), &entry.raw(&p), p.as_bytes()).map_err(err)
    }

    /// Add an entry for `entry.path` with `data` written as a blob
    /// (`git_index_add_from_buffer`).
    pub fn add_from_buffer(&self, entry: &IndexEntry, data: &[u8]) -> Result<()> {
        let p = c_path(&entry.path)?;
        let _g = self.api();
        ops::add_from_buffer(self.ptr(), &entry.raw(&p), p.as_bytes(), Ok(data)).map_err(err)
    }

    /// Add the working-directory file at `path` (relative to the working
    /// directory; `git_index_add_bypath`).
    pub fn add_path(&self, path: impl AsRef<[u8]>) -> Result<()> {
        let p = c_path(path.as_ref())?;
        let _g = self.api();
        ops::add_bypath(self.ptr(), &p).map_err(err)
    }

    /// Remove the entry for `path` at `stage` (`git_index_remove`).
    pub fn remove(&self, path: impl AsRef<[u8]>, stage: Stage) -> Result<()> {
        let path = path.as_ref();
        self.with(|i| i.remove(path, stage.raw())).map_err(err)
    }

    /// Remove `path`'s entries and its conflict (`git_index_remove_bypath`).
    pub fn remove_path(&self, path: impl AsRef<[u8]>) -> Result<()> {
        let path = path.as_ref();
        self.with(|i| ops::remove_bypath(i, path)).map_err(err)
    }

    /// Remove the entries under directory `dir` at `stage`
    /// (`git_index_remove_directory`).
    pub fn remove_dir(&self, dir: impl AsRef<[u8]>, stage: Stage) -> Result<()> {
        let mut pfx = crate::util::str::GitStr::new();
        pfx.sets(dir.as_ref()).map_err(err)?;
        crate::util::fs_path::to_dir(&mut pfx).map_err(err)?;
        self.with(|i| i.remove_directory(pfx.as_bytes(), stage.raw()))
            .map_err(err)
    }

    /// Remove every entry (`git_index_clear`).
    pub fn clear(&self) -> Result<()> {
        self.with(|i| i.clear()).map_err(err)
    }

    fn with_callback<I>(
        &self,
        pathspecs: I,
        mut cb: Option<&mut PathCallback<'_>>,
        op: impl FnOnce(IndexPtr, Paths<'_>, Option<ops::MatchedPathCb<'_>>) -> c_int,
    ) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        let owned: Vec<CString> = pathspecs
            .into_iter()
            .map(|p| c_path(p.as_ref()))
            .collect::<Result<_>>()?;
        let refs: Vec<&CStr> = owned.iter().map(|c| c.as_c_str()).collect();
        let arr = GitStrArrayBuf::new(&refs);
        let has_cb = cb.is_some();
        let mut panic = None;
        let mut stopped = false;
        let mut adapter = |p: *const c_char, m: *const c_char| -> c_int {
            let Some(f) = cb.as_mut() else { return 0 };
            if panic.is_some() || stopped {
                return ErrorCode::USER.raw();
            }
            // the strings are copied before the closure runs (it may change
            // the index, and free what they point to)
            // SAFETY: the operation passes a string and a NULL or string,
            // live for the call.
            let path = unsafe { crate::util::export::cstr_bytes(p) }.to_vec();
            // SAFETY: as above.
            let matched = unsafe { crate::util::export::cstr(m) }.map(|m| m.to_bytes().to_vec());
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                f(&path, matched.as_deref())
            })) {
                Ok(IndexMatch::Apply) => 0,
                Ok(IndexMatch::Skip) => 1,
                Ok(IndexMatch::Stop) => {
                    stopped = true;
                    ErrorCode::USER.raw()
                }
                Err(e) => {
                    panic = Some(e);
                    ErrorCode::USER.raw()
                }
            }
        };
        let r = {
            let _g = self.api();
            let cbr: Option<ops::MatchedPathCb<'_>> =
                if has_cb { Some(&mut adapter) } else { None };
            op(self.ptr(), Paths::from_ptrs(&arr), cbr)
        };
        // A panic in the callback resumes here, after the operation let go
        // of the index (as the other closures of the API).
        if let Some(p) = panic {
            std::panic::resume_unwind(p);
        }
        if stopped {
            crate::util::errors::clear();
            return Ok(());
        }
        check(r)
    }

    /// Add the working-directory files that match `pathspecs` (all if
    /// empty), as `git add` (`git_index_add_all`).
    pub fn add_all<I>(&self, pathspecs: I, flags: IndexAddOption) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, None, |ix, paths, cb| {
            ops::add_all(ix, paths, flags.raw(), cb)
        })
    }

    /// [`add_all`](Self::add_all), asking `f(path, matched_pathspec)`
    /// about each path.
    pub fn add_all_with<I>(
        &self,
        pathspecs: I,
        flags: IndexAddOption,
        mut f: impl FnMut(&[u8], Option<&[u8]>) -> IndexMatch,
    ) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, Some(&mut f), |ix, paths, cb| {
            ops::add_all(ix, paths, flags.raw(), cb)
        })
    }

    /// Update the entries that match `pathspecs` from the working directory
    /// (removing those whose file is gone; `git_index_update_all`).
    pub fn update_all<I>(&self, pathspecs: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, None, ops::update_all)
    }

    /// [`update_all`](Self::update_all) with a callback, as
    /// [`add_all_with`](Self::add_all_with).
    pub fn update_all_with<I>(
        &self,
        pathspecs: I,
        mut f: impl FnMut(&[u8], Option<&[u8]>) -> IndexMatch,
    ) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, Some(&mut f), ops::update_all)
    }

    /// Remove the entries that match `pathspecs` (`git_index_remove_all`).
    pub fn remove_all<I>(&self, pathspecs: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, None, ops::remove_all)
    }

    /// [`remove_all`](Self::remove_all) with a callback, as
    /// [`add_all_with`](Self::add_all_with).
    pub fn remove_all_with<I>(
        &self,
        pathspecs: I,
        mut f: impl FnMut(&[u8], Option<&[u8]>) -> IndexMatch,
    ) -> Result<()>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.with_callback(pathspecs, Some(&mut f), ops::remove_all)
    }

    /// Whether any entry is a conflict (`git_index_has_conflicts`).
    pub fn has_conflicts(&self) -> bool {
        self.with(|i| i.has_conflicts())
    }

    /// Record a conflict (`git_index_conflict_add`): at least one side.
    pub fn conflict_add(
        &self,
        ancestor: Option<&IndexEntry>,
        ours: Option<&IndexEntry>,
        theirs: Option<&IndexEntry>,
    ) -> Result<()> {
        let paths: Vec<Option<CString>> = [ancestor, ours, theirs]
            .iter()
            .map(|e| e.map(|e| c_path(&e.path)).transpose())
            .collect::<Result<_>>()?;
        let raws: Vec<Option<GitIndexEntry>> = [ancestor, ours, theirs]
            .iter()
            .zip(&paths)
            .map(|(e, p)| e.zip(p.as_ref()).map(|(e, p)| e.raw(p)))
            .collect();
        let side = |i: usize| {
            raws[i]
                .as_ref()
                .zip(paths[i].as_ref())
                .map(|(r, p)| (r, p.as_bytes()))
        };
        let _g = self.api();
        ops::conflict_add(self.ptr(), [side(0), side(1), side(2)]).map_err(err)
    }

    /// The sides of `path`'s conflict (`git_index_conflict_get`).
    pub fn conflict_get(&self, path: impl AsRef<[u8]>) -> Result<ConflictEntries> {
        let path = path.as_ref();
        self.with(|i| i.conflict_get(path).map(ConflictEntries::from_ptrs))
            .map_err(err)
    }

    /// Remove `path`'s conflict (`git_index_conflict_remove`).
    pub fn conflict_remove(&self, path: impl AsRef<[u8]>) -> Result<()> {
        let path = path.as_ref();
        self.with(|i| i.conflict_remove(Some(path))).map_err(err)
    }

    /// Remove every conflict (`git_index_conflict_cleanup`).
    pub fn conflict_cleanup(&self) -> Result<()> {
        self.with(|i| i.conflict_remove(None)).map_err(err)
    }

    /// The conflicts (`git_index_conflict_iterator`), read as the iteration
    /// goes.
    pub fn conflicts(&self) -> IndexConflicts<'r> {
        IndexConflicts {
            index: self.clone(),
            cur: 0,
        }
    }

    /// The on-disk format version (`git_index_version`).
    pub fn version(&self) -> u32 {
        self.with(|i| i.version)
    }

    /// Set the format version, 2 to 4 (`git_index_set_version`).
    pub fn set_version(&self, version: u32) -> Result<()> {
        if !(super::INDEX_VERSION_NUMBER_LB..=super::INDEX_VERSION_NUMBER_UB).contains(&version) {
            let code = crate::git_error_set!(
                crate::util::errors::ErrorClass::INDEX,
                "invalid version number"
            );
            return Err(err(code));
        }
        self.with(|i| i.version = version);
        Ok(())
    }

    /// The capabilities (`git_index_caps`).
    pub fn caps(&self) -> IndexCapability {
        self.with(|i| {
            let mut c = IndexCapability::empty();
            c.set(IndexCapability::IGNORE_CASE, i.ignore_case());
            c.set(IndexCapability::NO_FILEMODE, i.distrust_filemode());
            c.set(IndexCapability::NO_SYMLINKS, i.no_symlinks());
            c
        })
    }

    /// Set the capabilities (`git_index_set_caps`).
    pub fn set_caps(&self, caps: IndexCapability) -> Result<()> {
        let _g = self.api();
        ops::set_caps(self.ptr(), caps.raw() as c_int).map_err(err)
    }

    /// Take the capabilities from the repository's configuration
    /// (`git_index_set_caps(GIT_INDEX_CAPABILITY_FROM_OWNER)`).
    pub fn set_caps_from_owner(&self) -> Result<()> {
        let _g = self.api();
        ops::set_caps(self.ptr(), GIT_INDEX_CAPABILITY_FROM_OWNER).map_err(err)
    }

    /// The checksum of the file as last read or written
    /// (`git_index_checksum`).
    pub fn checksum(&self) -> Vec<u8> {
        self.with(|i| i.checksum[..i.oid_kind().size()].to_vec())
    }

    /// Make this index's entries those of `other` (`git_index_read_index`).
    pub fn read_index(&self, other: &Index<'_>) -> Result<()> {
        let _g = self.api();
        let _o = other.api();
        ops::read_index(self.ptr(), other.ptr()).map_err(err)
    }

    /// The number of resolve-undo records (`git_index_reuc_entrycount`).
    pub fn reuc_len(&self) -> usize {
        self.with(|i| i.reuc.len())
    }

    /// The `n`th resolve-undo record (`git_index_reuc_get_byindex`).
    pub fn reuc_get(&self, n: usize) -> Option<ReucEntry> {
        self.with(|i| {
            let r = i.reuc.get(n)?.cast::<super::GitIndexReucEntry>();
            Some(reuc_entry(r))
        })
    }

    /// The resolve-undo record of `path` (`git_index_reuc_get_bypath`).
    pub fn reuc_get_path(&self, path: impl AsRef<[u8]>) -> Option<ReucEntry> {
        let path = path.as_ref();
        self.with(|i| {
            let n = i.reuc_find(path).ok()?;
            let r = i.reuc.get(n)?.cast::<super::GitIndexReucEntry>();
            Some(reuc_entry(r))
        })
    }

    /// The position of `path`'s resolve-undo record
    /// (`git_index_reuc_find`).
    pub fn reuc_find(&self, path: impl AsRef<[u8]>) -> Option<usize> {
        let path = path.as_ref();
        self.with(|i| i.reuc_find(path).ok())
    }

    /// Add a resolve-undo record (`git_index_reuc_add`): the ancestor's,
    /// ours and theirs modes (0: absent) and ids.
    pub fn reuc_add(&self, path: impl AsRef<[u8]>, modes: [u32; 3], ids: [Oid; 3]) -> Result<()> {
        let path = path.as_ref();
        let modes = modes.map(|m| m as c_int);
        self.with(|i| i.reuc_add(path, modes, ids.map(Some)))
            .map_err(err)
    }

    /// Remove the `n`th resolve-undo record (`git_index_reuc_remove`).
    pub fn reuc_remove(&self, n: usize) -> Result<()> {
        self.with(|i| i.reuc_remove(n)).map_err(err)
    }

    /// Remove every resolve-undo record (`git_index_reuc_clear`).
    pub fn reuc_clear(&self) {
        self.with(|i| i.reuc_clear())
    }

    /// The number of conflict name records (`git_index_name_entrycount`).
    pub fn name_len(&self) -> usize {
        self.with(|i| i.names.len())
    }

    /// The `n`th conflict name record (`git_index_name_get_byindex`).
    pub fn name_get(&self, n: usize) -> Option<NameEntry> {
        self.with(|i| {
            i.sort_names();
            let ne = i.names.get(n)?.cast::<super::GitIndexNameEntry>();
            // SAFETY: a live name entry of the index; its fields NULL or
            // strings.
            let n = unsafe { &*ne };
            let f = |p| {
                // SAFETY: as above.
                unsafe { crate::util::export::cstr(p) }.map(|s: &CStr| s.to_bytes().to_vec())
            };
            Some(NameEntry {
                ancestor: f(n.ancestor),
                ours: f(n.ours),
                theirs: f(n.theirs),
            })
        })
    }

    /// Add a conflict name record (`git_index_name_add`): at least two of
    /// the three.
    pub fn name_add(
        &self,
        ancestor: Option<&[u8]>,
        ours: Option<&[u8]>,
        theirs: Option<&[u8]>,
    ) -> Result<()> {
        let n = [ancestor, ours, theirs]
            .iter()
            .filter(|x| x.is_some())
            .count();
        if n < 2 {
            let code = crate::util::errors::assert_arg_failed(
                "(ancestor && ours) || (ancestor && theirs) || (ours && theirs)",
            );
            return Err(err(code));
        }
        let c = |x: Option<&[u8]>| x.map(c_path).transpose();
        let (a, o, t) = (c(ancestor)?, c(ours)?, c(theirs)?);
        self.with(|i| i.name_add(a.as_deref(), o.as_deref(), t.as_deref()))
            .map_err(err)
    }

    /// Remove every conflict name record (`git_index_name_clear`).
    pub fn name_clear(&self) {
        self.with(|i| i.name_clear())
    }

    /// The data of the extension `signature` (a 4-byte tag), if present.
    pub fn extension(&self, signature: &[u8; 4]) -> Option<Vec<u8>> {
        let key = &signature[..signature.iter().position(|&b| b == 0).unwrap_or(4)];
        self.with(|i| {
            i.extension_find(key)
                .ok()
                .map(|p| i.extensions[p].data.clone())
        })
    }

    /// Add or replace an extension (written with the index).
    pub fn extension_add(&self, signature: &[u8; 4], data: &[u8]) -> Result<()> {
        if data.len() >= u32::MAX as usize {
            return Err(err(crate::util::errors::assert_failed(
                "data_len < UINT32_MAX",
            )));
        }
        self.with(|i| i.extension_add(*signature, data))
            .map_err(err)
    }

    /// Remove an extension.
    pub fn extension_remove(&self, signature: &[u8; 4]) -> Result<()> {
        let key = &signature[..signature.iter().position(|&b| b == 0).unwrap_or(4)];
        self.with(|i| i.extension_remove(key)).map_err(err)
    }
}

/// A copy of a resolve-undo record.
fn reuc_entry(r: *const super::GitIndexReucEntry) -> ReucEntry {
    // SAFETY: a live REUC entry of the index (the lock is held).
    let path = unsafe { super::reuc_path(r) }.to_vec();
    // SAFETY: as above.
    let rr = unsafe { &*r };
    ReucEntry {
        path,
        mode: rr.mode,
        oid: rr.oid,
    }
}

/// The entries of an index, as of [`Index::iter`] (a snapshot: the index
/// keeps the entries it removes meanwhile until the snapshot ends).
pub struct IndexEntries<'r> {
    index: Index<'r>,
    snap: Option<crate::util::vector::GitVector>,
    cur: usize,
}

// SAFETY: the snapshot is a vector of entry pointers into the index, read
// only with the index's lock held (`next`) and released with it held
// (`drop`); the index handle is `Send + Sync`.
unsafe impl Send for IndexEntries<'_> {}
// SAFETY: `&IndexEntries` reaches nothing (every method takes `&mut`).
unsafe impl Sync for IndexEntries<'_> {}

impl Iterator for IndexEntries<'_> {
    type Item = IndexEntry;

    fn next(&mut self) -> Option<IndexEntry> {
        let _g = self.index.api();
        let e = self.snap.as_ref()?.get(self.cur)?;
        self.cur += 1;
        // a snapshot's entries stay live: the index defers their removal
        Some(IndexEntry::from_ptr(super::as_entry(e)))
    }
}

impl Drop for IndexEntries<'_> {
    fn drop(&mut self) {
        if let Some(snap) = self.snap.take() {
            let _g = self.index.api();
            ops::snapshot_release(snap, self.index.ptr());
        }
    }
}

impl core::fmt::Debug for IndexEntries<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IndexEntries")
            .field("cur", &self.cur)
            .finish()
    }
}

/// The conflicts of an index ([`Index::conflicts`]).
#[derive(Debug)]
pub struct IndexConflicts<'r> {
    index: Index<'r>,
    cur: usize,
}

impl Iterator for IndexConflicts<'_> {
    type Item = ConflictEntries;

    fn next(&mut self) -> Option<ConflictEntries> {
        let cur = &mut self.cur;
        self.index
            .with(|i| conflict_next(i, cur).map(ConflictEntries::from_ptrs))
    }
}
