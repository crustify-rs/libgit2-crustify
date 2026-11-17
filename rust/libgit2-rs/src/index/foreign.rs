//! The index's calls into code that is not this module's: the C that stays
//! C (`path.c`, `diff*.c`, `pathspec.c`, `ignore.c`, `submodule.c`, in
//! `crate::sys`) and the modules other work packages are porting
//! and the other ported modules (`crate::{blob, object, object_api, tree,
//! repository}`, whose definitions of the C ABI it calls). Each wrapper is safe
//! to call: its arguments are types whose constructors establish what the
//! C needs, and its `SAFETY` comment says why the call's contract holds.
//!
//! None of these is called while a reference into an index is live (see
//! the [index module](super)): any of them may run code that reaches the
//! index.

use crate::util::cabi::{CStrArg, Out};
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use super::IndexPtr;
use crate::git_error_set;
use crate::oid::Oid;
use crate::repository::GitRepository;
use crate::strarray::GitStrArray;
use crate::sys::diff as d;
use crate::sys::{ignore, path, pathspec, submodule};
use crate::tree::GitTreeEntry;
use crate::tree::ffi as t;
use crate::tree_cache::TreeSource;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;

/// A repository: an index's owner (`GIT_REFCOUNT_OWNER`), which outlives
/// the index, or one C passed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repo(NonNull<GitRepository>);

impl Repo {
    /// A handle on `p`, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// A non-null `p` is a live `git_repository` for as long as the handle
    /// (or a copy) is used.
    pub unsafe fn new(p: *mut c_void) -> Option<Repo> {
        NonNull::new(p.cast()).map(Repo)
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr().cast()
    }

    /// The repository.
    pub fn get(&self) -> &GitRepository {
        // SAFETY: live while the handle is used (`new`'s contract); a
        // `GitRepository` is `Sync` (every field atomic or locked), so a
        // shared reference is sound while C uses it too.
        unsafe { self.0.as_ref() }
    }
}

/// The raw pointer of an optional repository.
fn raw(repo: Option<Repo>) -> *mut c_void {
    repo.map_or(ptr::null_mut(), Repo::as_ptr)
}

// ---------------------------------------------------------------------------
// path.c
// ---------------------------------------------------------------------------

/// `git_path_is_valid` of `path` (a NUL within it makes it invalid).
///
/// Under Miri (which cannot call C), a Rust stand-in: the filesystem
/// checks of the flags (`fs_path`'s port) and a case-insensitive `.git`
/// component for `GIT_PATH_REJECT_DOT_GIT`, so that the index's own logic
/// runs under Miri; the repository's NTFS/HFS rules are not modelled.
#[cfg(miri)]
pub fn path_is_valid(repo: Option<Repo>, path: &[u8], file_mode: u16, flags: c_uint) -> bool {
    let _ = (repo, file_mode);
    let fs_flags = flags & (path::GIT_FS_PATH_REJECT_MAX * 2 - 1);
    let mut dotgit =
        |c: &[u8]| flags & path::GIT_PATH_REJECT_DOT_GIT == 0 || !c.eq_ignore_ascii_case(b".git");
    crate::util::fs_path::str_is_valid_ext(
        path,
        true,
        fs_flags,
        crate::util::fs_path::ValidateCallbacks {
            char_cb: None,
            component_cb: Some(&mut dotgit),
        },
    )
}

/// `git_path_is_valid` of `path` (a NUL within it makes it invalid).
#[cfg(not(miri))]
pub fn path_is_valid(repo: Option<Repo>, path: &[u8], file_mode: u16, flags: c_uint) -> bool {
    let s = path::ConstStr {
        ptr: path.as_ptr().cast(),
        asize: 0,
        size: path.len(),
    };
    // SAFETY: `s` describes `path`, readable for the call, which C only
    // reads; `repo` is NULL or live (`Repo`).
    unsafe { path::git_path_str_is_valid(raw(repo), &raw const s, file_mode, flags) }
}

// ---------------------------------------------------------------------------
// repository.c (`crate::repository`)
// ---------------------------------------------------------------------------

/// `GIT_CONFIGMAP_SYMLINKS`.
pub const CONFIGMAP_SYMLINKS: c_int = 2;
/// `GIT_CONFIGMAP_IGNORECASE`.
pub const CONFIGMAP_IGNORECASE: c_int = 3;
/// `GIT_CONFIGMAP_FILEMODE`.
pub const CONFIGMAP_FILEMODE: c_int = 4;

/// `git_repository__configmap_lookup`.
pub fn configmap_lookup(repo: Repo, item: c_int) -> Result<c_int> {
    repo.get().configmap_lookup(item)
}

/// Whether `git_repository_workdir` is not NULL.
pub fn has_workdir(repo: Repo) -> bool {
    repo.get().workdir().is_some()
}

/// `git_repository__ensure_not_bare`.
pub fn ensure_not_bare(repo: Repo, operation_name: &str) -> Result<()> {
    if !repo.get().is_bare() {
        return Ok(());
    }
    let _ = git_error_set!(
        ErrorClass::REPOSITORY,
        "cannot {operation_name}. This operation is not allowed against bare repositories."
    );
    Err(ErrorCode::BARE_REPO)
}

/// `git_repository_workdir_path`.
pub fn workdir_path(repo: Repo, rel: &CStr) -> Result<GitStr> {
    let mut out = GitStr::new();
    repo.get().workdir_path(&mut out, rel.to_bytes())?;
    Ok(out)
}

/// The id of `HEAD` of the repository at `path` (`add_repo_as_submodule`:
/// open, `git_repository_head`, `git_reference_target`). The repository
/// is freed also on failure (C leaks it then).
pub fn head_target_of(path: &CStr) -> Result<Oid> {
    let sub = crate::repository::open_owned(path)?;
    let head = sub.get().head_ref()?;
    // `git_reference_target`: NULL (all zeros here) for a symbolic one.
    Ok(head.target().copied().unwrap_or_default())
}

/// `git_repository_index__weakptr`: the repository's index (no reference
/// taken).
pub fn repository_index(repo: Repo) -> Result<NonNull<super::GitIndex>> {
    repo.get().index_weak()
}

// ---------------------------------------------------------------------------
// blob.c, object.c (`crate::blob`, `crate::object`)
// ---------------------------------------------------------------------------

/// `git_blob__create_from_paths(&id, &st, repo, NULL, rel_path, 0, true)`:
/// write the working-directory file `rel_path` as a blob; its id, and its
/// `stat` in `st`.
pub fn blob_from_workdir(repo: Repo, rel_path: &CStr, st: &mut libc::stat) -> Result<Oid> {
    let mut id = Oid::default();
    // SAFETY: `id` and `st` are writable, `rel_path` a string, `repo` live.
    let r = unsafe {
        p::git_blob__create_from_paths(
            Out::from(&mut id),
            st,
            repo.as_ptr(),
            CStrArg::NULL,
            rel_path.into(),
            0,
            true,
        )
    };
    ErrorCode::check(r).map(|_| id)
}

/// `git_blob_create_from_buffer`.
pub fn blob_from_buffer(repo: Repo, buf: &[u8]) -> Result<Oid> {
    let mut id = Oid::default();
    // SAFETY: `id` writable, `buf` readable for its length, `repo` live.
    let r = unsafe {
        p::git_blob_create_from_buffer(&raw mut id, repo.as_ptr(), buf.as_ptr().cast(), buf.len())
    };
    ErrorCode::check(r).map(|_| id)
}

/// `git_object__type_from_filemode`.
pub fn type_from_filemode(mode: u32) -> c_int {
    match mode {
        super::GIT_FILEMODE_TREE => 2,
        super::GIT_FILEMODE_COMMIT => 1,
        super::GIT_FILEMODE_BLOB
        | super::GIT_FILEMODE_BLOB_EXECUTABLE
        | super::GIT_FILEMODE_LINK => 3,
        _ => -1,
    }
}

/// `git_object__is_valid`.
pub fn object_is_valid(repo: Repo, id: &Oid, expected_type: c_int) -> bool {
    // SAFETY: `id` readable, `repo` live.
    unsafe { p::git_object__is_valid(repo.as_ptr(), Some(id), expected_type) }
}

// ---------------------------------------------------------------------------
// submodule.c
// ---------------------------------------------------------------------------

/// A submodule from `git_submodule_lookup`, freed on drop.
pub struct Submodule(NonNull<c_void>);

impl Drop for Submodule {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { submodule::git_submodule_free(self.0.as_ptr()) };
    }
}

/// `git_submodule_lookup`.
pub fn submodule_lookup(repo: Repo, name: &CStr) -> Result<Submodule> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` writable, `name` a string, `repo` live.
    let r = unsafe { submodule::git_submodule_lookup(&raw mut out, repo.as_ptr(), name.as_ptr()) };
    ErrorCode::check(r)?;
    NonNull::new(out).map(Submodule).ok_or(ErrorCode::GENERIC)
}

/// `git_submodule_add_to_index(sm, false)`.
pub fn submodule_add_to_index(sm: &Submodule) -> Result<()> {
    // SAFETY: `sm` is live.
    ErrorCode::check(unsafe { submodule::git_submodule_add_to_index(sm.0.as_ptr(), 0) }).map(|_| ())
}

// ---------------------------------------------------------------------------
// tree.c (`crate::tree`), object_api.c (`crate::object_api`)
// ---------------------------------------------------------------------------

/// A `git_tree`: borrowed, or looked up (and freed on drop).
pub struct CTree {
    ptr: NonNull<c_void>,
    owned: bool,
}

impl Drop for CTree {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: a tree we looked up, freed once.
            unsafe { p::git_tree_free(self.ptr.as_ptr()) };
        }
    }
}

/// The trees of one repository, through `tree.c`'s accessors.
pub struct CTrees {
    repo: *mut c_void,
}

impl CTrees {
    /// The trees of `tree`'s repository, and `tree` itself (borrowed).
    ///
    /// # Safety
    ///
    /// `tree` is a live `git_tree` for as long as both results are used.
    pub unsafe fn borrow(tree: *const c_void) -> (CTrees, CTree) {
        // SAFETY: the caller's contract.
        let repo = unsafe { p::git_tree_owner(tree) };
        (
            CTrees { repo },
            CTree {
                ptr: NonNull::new(tree.cast_mut()).unwrap_or(NonNull::dangling()),
                owned: false,
            },
        )
    }
}

// SAFETY: a `git_tree` is immutable while it lives: its entry count and
// entries do not change.
unsafe impl TreeSource for CTrees {
    type Tree = CTree;

    fn id(&self, tree: &CTree) -> Oid {
        // SAFETY: a live tree (`CTree`); its id lives as long as it.
        let id = unsafe { p::git_tree_id(tree.ptr.as_ptr()) };
        // SAFETY: as above.
        unsafe { *id }
    }

    fn len(&self, tree: &CTree) -> usize {
        // SAFETY: as above.
        unsafe { t::git_tree_entrycount(tree.ptr.as_ptr().cast()) }
    }

    fn entry<'t>(&self, tree: &'t CTree, i: usize) -> (bool, &'t [u8], Oid) {
        // SAFETY: as above; `i < len` (the caller), so the entry is the
        // tree's, which lives (unwritten) as long as the tree.
        let e = unsafe { t::git_tree_entry_byindex(tree.ptr.as_ptr().cast(), i) };
        // SAFETY: as above.
        unsafe { entry_view(e) }
    }

    fn is_tree(&self, tree: &CTree, i: usize) -> bool {
        // SAFETY: as in `entry`.
        let e = unsafe { t::git_tree_entry_byindex(tree.ptr.as_ptr().cast(), i) };
        // SAFETY: as above.
        unsafe { t::git_tree_entry_filemode(e) == super::GIT_FILEMODE_TREE }
    }

    fn lookup(&self, id: &Oid) -> Result<CTree> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` writable; `repo` is the tree's live repository.
        ErrorCode::check(unsafe { p::git_tree_lookup(&raw mut out, self.repo, Some(id)) })?;
        Ok(CTree {
            ptr: NonNull::new(out).ok_or(ErrorCode::GENERIC)?,
            owned: true,
        })
    }
}

/// A tree entry's kind (`GIT_FILEMODE_TREE`), name and id.
///
/// # Safety
///
/// `e` is a live tree entry, unwritten for `'a`.
unsafe fn entry_view<'a>(e: *const GitTreeEntry) -> (bool, &'a [u8], Oid) {
    // SAFETY: the caller's contract.
    let is_tree = unsafe { t::git_tree_entry_filemode(e) } == super::GIT_FILEMODE_TREE;
    // SAFETY: as above.
    let name = unsafe { t::git_tree_entry_name(e) };
    // SAFETY: as above: the name is a string of the entry.
    let name = unsafe { crate::util::export::cstr_bytes(name) };
    // SAFETY: as above.
    let id = unsafe { t::git_tree_entry_id(e) };
    // SAFETY: as above.
    (is_tree, name, unsafe { *id })
}

/// A tree object the caller passed (`git_index_read_tree`).
#[derive(Clone, Copy)]
pub struct TreeRef<'a>(NonNull<c_void>, PhantomData<&'a ()>);

impl<'a> TreeRef<'a> {
    /// A handle on `tree`.
    ///
    /// # Safety
    ///
    /// `tree` is a live `git_tree` for `'a`.
    pub unsafe fn new(tree: NonNull<c_void>) -> TreeRef<'a> {
        TreeRef(tree, PhantomData)
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *const c_void {
        self.0.as_ptr()
    }
}

/// A tree entry handed to a [`tree_walk`] callback.
pub struct TreeEntryView<'a> {
    /// `filename`.
    pub name: &'a [u8],
    /// `attr` (`git_tree_entry_filemode_raw`).
    pub attr: u32,
    /// The id.
    pub id: Oid,
}

/// `git_tree_walk(tree, GIT_TREEWALK_POST, …)` with a closure, given the
/// root (the entry's directory, with a trailing slash) and the entry;
/// non-zero stops the walk.
pub fn tree_walk_post(
    tree: TreeRef<'_>,
    f: &mut dyn FnMut(&[u8], &TreeEntryView<'_>) -> c_int,
) -> Result<()> {
    struct Payload<'a, 'b>(
        &'a mut dyn FnMut(&[u8], &TreeEntryView<'_>) -> c_int,
        PhantomData<&'b ()>,
    );
    unsafe extern "C" fn cb(
        root: *const c_char,
        entry: *const GitTreeEntry,
        payload: *mut c_void,
    ) -> c_int {
        crate::util::export::barrier(|| {
            // SAFETY: `payload` is the `Payload` below, live for the walk;
            // `root` a string and `entry` a live tree entry, for the call.
            let pl = unsafe { &mut *payload.cast::<Payload<'_, '_>>() };
            // SAFETY: as above.
            let root = unsafe { crate::util::export::cstr_bytes(root) };
            // SAFETY: as above.
            let (_, name, id) = unsafe { entry_view(entry) };
            // SAFETY: as above.
            let attr = unsafe { t::git_tree_entry_filemode_raw(entry) };
            (pl.0)(root, &TreeEntryView { name, attr, id })
        })
    }
    let mut pl = Payload(f, PhantomData);
    // SAFETY: `tree` is live (`TreeRef`); `cb` only uses `pl` during the
    // walk.
    let r = unsafe { t::git_tree_walk(tree.as_ptr().cast(), 1, Some(cb), (&raw mut pl).cast()) };
    ErrorCode::check(r).map(|_| ())
}

/// `git_tree__write_index`.
///
/// # Safety
///
/// No reference into the index is live during the call, and nothing else
/// uses the index meanwhile (the C that the tree write calls included:
/// it borrows the entries' paths across those calls, as C does).
pub unsafe fn tree_write_index(index: IndexPtr, repo: Repo) -> Result<Oid> {
    let mut id = Oid::default();
    // SAFETY: `id` writable; `repo` live (`Repo`); the index is live
    // (`IndexPtr`), and the rest is this fn's contract.
    let r = unsafe { t::git_tree__write_index(Out::from(&mut id), Some(index), repo.0.as_ptr()) };
    ErrorCode::check(r).map(|_| id)
}

// ---------------------------------------------------------------------------
// pathspec.c, ignore.c
// ---------------------------------------------------------------------------

/// The strings of a `git_strarray` argument, valid for `'a` (or none).
#[derive(Clone, Copy)]
pub struct Paths<'a>(*const GitStrArray, PhantomData<&'a ()>);

impl<'a> Paths<'a> {
    /// The array at `raw` (NULL for none).
    ///
    /// # Safety
    ///
    /// A non-null `raw` is a valid `git_strarray`, its strings NULL or
    /// NUL-terminated, all readable and unwritten for `'a`.
    pub unsafe fn from_raw(raw: *const GitStrArray) -> Paths<'a> {
        Paths(raw, PhantomData)
    }

    /// The array `arr` (whose strings live for `'a`).
    pub fn from_ptrs(arr: &'a GitStrArrayBuf<'a>) -> Paths<'a> {
        Paths(&raw const arr.raw, PhantomData)
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *const GitStrArray {
        self.0
    }
}

/// A `git_strarray` over strings borrowed for `'a`.
pub struct GitStrArrayBuf<'a> {
    raw: GitStrArray,
    _ptrs: Vec<*mut c_char>,
    _strings: PhantomData<&'a CStr>,
}

impl<'a> GitStrArrayBuf<'a> {
    /// An array of the `strings`, borrowed for `'a`.
    pub fn new(strings: &[&'a CStr]) -> GitStrArrayBuf<'a> {
        let mut ptrs: Vec<*mut c_char> = strings.iter().map(|s| s.as_ptr().cast_mut()).collect();
        GitStrArrayBuf {
            raw: GitStrArray {
                strings: if ptrs.is_empty() {
                    ptr::null_mut()
                } else {
                    ptrs.as_mut_ptr()
                },
                count: ptrs.len(),
            },
            _ptrs: ptrs,
            _strings: PhantomData,
        }
    }
}

/// A parsed pathspec (`git_pathspec`), cleared on drop.
#[cfg(not(miri))]
pub struct Pathspec(Box<core::mem::MaybeUninit<pathspec::GitPathspec>>);

/// Under Miri (which cannot call C), a stand-in for `pathspec.c`: the
/// patterns, matched with `wildmatch`'s port (with `WM_PATHNAME`) or as a
/// directory prefix; no pattern matches everything. Enough to drive the
/// index's pathspec operations and their callbacks.
#[cfg(miri)]
pub struct Pathspec(Vec<std::ffi::CString>);

#[cfg(miri)]
impl Pathspec {
    /// The stand-in of `git_pathspec__init`.
    pub fn new(paths: Paths<'_>) -> Result<Pathspec> {
        let mut v = Vec::new();
        if !paths.0.is_null() {
            // SAFETY: a valid `git_strarray` (`Paths`).
            let raw = unsafe { *paths.0 };
            for i in 0..raw.count {
                // SAFETY: as above: `count` pointers, each NULL or a string.
                let p = unsafe { *raw.strings.add(i) };
                // SAFETY: as above.
                if let Some(c) = unsafe { crate::util::export::cstr(p) }
                    && !c.is_empty()
                {
                    v.push(c.to_owned());
                }
            }
        }
        Ok(Pathspec(v))
    }

    /// The stand-in of `git_pathspec__match`.
    pub fn matches(&self, path: &CStr, casefold: bool) -> Option<*const c_char> {
        if self.0.is_empty() {
            return Some(ptr::null());
        }
        let p = path.to_bytes();
        self.0
            .iter()
            .find(|pat| {
                let pat = pat.to_bytes();
                let flags = crate::util::wildmatch::WildmatchFlags {
                    casefold,
                    pathname: true,
                };
                crate::util::wildmatch::wildmatch(pat, p, flags)
                    == crate::util::wildmatch::WildMatch::Match
                    || (p.starts_with(pat) && p.get(pat.len()) == Some(&b'/'))
            })
            .map(|pat| pat.as_ptr())
    }

    /// Ignore checks need the repository: none under Miri.
    pub fn check_exact_ignores(&mut self, repo: Repo, no_fnmatch: bool) -> Result<()> {
        let _ = (repo, no_fnmatch);
        Ok(())
    }
}

#[cfg(not(miri))]
impl Pathspec {
    /// `git_pathspec__init`.
    pub fn new(paths: Paths<'_>) -> Result<Pathspec> {
        let mut ps = Box::new(core::mem::MaybeUninit::<pathspec::GitPathspec>::uninit());
        // SAFETY: `ps` is writable storage for a `git_pathspec`, which the
        // C initializes (memset first); `paths` is NULL or valid.
        let r = unsafe { pathspec::git_pathspec__init(ps.as_mut_ptr(), paths.as_ptr()) };
        if r < 0 {
            // the C cleared it already; nothing is owned
            core::mem::forget(ps);
            return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
        }
        Ok(Pathspec(ps))
    }

    /// The parsed patterns (`&ps.pathspec`).
    fn vector(&self) -> *const crate::util::vector::GitVector {
        // SAFETY: initialized by `new`; only a raw pointer is formed.
        unsafe { &raw const (*self.0.as_ptr()).pathspec }
    }

    /// `git_pathspec__match`: `Some(matched pattern, or NULL)` if `path`
    /// matches.
    pub fn matches(&self, path: &CStr, casefold: bool) -> Option<*const c_char> {
        let mut m = ptr::null();
        // SAFETY: the vector is the initialized pathspec's; `path` a
        // string; `m` writable.
        let ok = unsafe {
            pathspec::git_pathspec__match(
                self.vector(),
                path.as_ptr(),
                false,
                casefold,
                &raw mut m,
                ptr::null_mut(),
            )
        };
        ok.then_some(m)
    }

    /// `git_ignore__check_pathspec_for_exact_ignores`.
    pub fn check_exact_ignores(&mut self, repo: Repo, no_fnmatch: bool) -> Result<()> {
        // SAFETY: initialized by `new`; only the field's address is taken.
        let v = unsafe { &raw mut (*self.0.as_mut_ptr()).pathspec };
        // SAFETY: as in `matches`; `repo` live. C reads the vector.
        let r = unsafe {
            ignore::git_ignore__check_pathspec_for_exact_ignores(repo.as_ptr(), v, no_fnmatch)
        };
        ErrorCode::check(r).map(|_| ())
    }
}

#[cfg(not(miri))]
impl Drop for Pathspec {
    fn drop(&mut self) {
        // SAFETY: initialized by `new`, cleared once (the C zeroes it, and
        // it is never dropped as a Rust value).
        unsafe { pathspec::git_pathspec__clear(self.0.as_mut_ptr()) };
    }
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

/// A `git_diff`, freed on drop.
pub struct Diff(NonNull<c_void>);

impl Drop for Diff {
    fn drop(&mut self) {
        // SAFETY: ours, freed once.
        unsafe { d::git_diff_free(self.0.as_ptr()) };
    }
}

/// A delta of a [`Diff`], copied out.
pub struct DeltaView<'a> {
    /// `old_file.path`.
    pub old_path: &'a CStr,
    /// `new_file.path`.
    pub new_path: &'a CStr,
    /// `new_file.flags`.
    pub new_flags: u32,
}

/// The view of a delta.
///
/// # Safety
///
/// `delta` is a live delta whose paths are strings, for `'a`.
unsafe fn view<'a>(delta: *const d::GitDiffDelta) -> DeltaView<'a> {
    // SAFETY: the caller's contract.
    let d = unsafe { &*delta };
    DeltaView {
        // SAFETY: as above.
        old_path: unsafe { CStr::from_ptr(d.old_file.path) },
        // SAFETY: as above.
        new_path: unsafe { CStr::from_ptr(d.new_file.path) },
        new_flags: d.new_file.flags,
    }
}

impl Diff {
    /// `git_diff_index_to_workdir` with `flags` and `pathspec`.
    ///
    /// # Safety
    ///
    /// `index` is a live index that no reference covers during the call.
    pub unsafe fn index_to_workdir(
        repo: Repo,
        index: *mut super::GitIndex,
        flags: u32,
        pathspec: Option<(*mut *mut c_char, usize)>,
    ) -> Result<Diff> {
        let mut opts = d::GitDiffOptions::new();
        opts.flags = flags;
        if let Some((strings, count)) = pathspec {
            opts.pathspec = GitStrArray { strings, count };
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` writable; `opts` a valid options struct whose
        // pathspec strings the caller keeps live; `repo` live; the
        // caller's contract for `index`.
        let r = unsafe {
            d::git_diff_index_to_workdir(&raw mut out, repo.as_ptr(), index.cast(), &raw const opts)
        };
        ErrorCode::check(r)?;
        NonNull::new(out).map(Diff).ok_or(ErrorCode::GENERIC)
    }

    /// `git_diff_num_deltas`.
    pub fn len(&self) -> usize {
        // SAFETY: a live diff.
        unsafe { d::git_diff_num_deltas(self.0.as_ptr()) }
    }

    /// The old path of delta `i` (`git_diff_get_delta`), copied.
    pub fn old_path(&self, i: usize) -> Option<Vec<u8>> {
        // SAFETY: a live diff; the delta lives as long as it, and its path
        // is a string.
        let delta = unsafe { d::git_diff_get_delta(self.0.as_ptr(), i) };
        if delta.is_null() {
            return None;
        }
        // SAFETY: as above.
        Some(unsafe { view(delta) }.old_path.to_bytes().to_vec())
    }

    /// `git_diff_foreach` with a file callback only.
    pub fn foreach_file(&self, f: &mut dyn FnMut(&DeltaView<'_>) -> c_int) -> c_int {
        struct Payload<'a>(&'a mut dyn FnMut(&DeltaView<'_>) -> c_int);
        unsafe extern "C" fn cb(
            delta: *const d::GitDiffDelta,
            _progress: f32,
            payload: *mut c_void,
        ) -> c_int {
            crate::util::export::barrier(|| {
                // SAFETY: `payload` is the `Payload` below, live for the
                // call; `delta` a live delta for the callback.
                let pl = unsafe { &mut *payload.cast::<Payload<'_>>() };
                // SAFETY: as above.
                (pl.0)(&unsafe { view(delta) })
            })
        }
        let mut pl = Payload(f);
        // SAFETY: a live diff; `cb` uses `pl` only during the call.
        unsafe {
            d::git_diff_foreach(
                self.0.as_ptr(),
                Some(cb),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                (&raw mut pl).cast(),
            )
        }
    }
}

/// `git_error_set_after_callback(error)` in function `func`.
pub fn set_after_callback(code: c_int, func: &str) -> c_int {
    errors::set_after_callback(code, func)
}

#[allow(dead_code)]
fn _unused(_: Bytes<'_>) {}

/// The object modules' definitions the index calls (blob creation, object
/// validation, tree lookup), with the untyped repository and tree pointers
/// this module holds (named as the C functions).
#[allow(non_snake_case)]
mod p {
    use crate::util::cabi::CStrArg;
    use crate::util::cabi::Out;
    use core::ffi::{c_int, c_void};

    use crate::oid::Oid;

    /// `git_blob__create_from_paths`.
    ///
    /// # Safety
    ///
    /// As the export's.
    pub unsafe fn git_blob__create_from_paths(
        out_oid: Out<'_, Oid>,
        out_st: *mut libc::stat,
        repo: *mut c_void,
        full_path: CStrArg<'_>,
        hint_path: CStrArg<'_>,
        hint_mode: libc::mode_t,
        apply_filters: bool,
    ) -> c_int {
        // SAFETY: the caller's contract (`posix::Stat` is `libc::stat`).
        unsafe {
            crate::blob::ffi::git_blob__create_from_paths(
                out_oid,
                out_st.cast(),
                repo.cast(),
                full_path,
                hint_path,
                hint_mode,
                apply_filters,
            )
        }
    }

    /// `git_blob_create_from_buffer`.
    ///
    /// # Safety
    ///
    /// As the export's.
    pub unsafe fn git_blob_create_from_buffer(
        id: *mut Oid,
        repo: *mut c_void,
        buffer: *const c_void,
        len: usize,
    ) -> c_int {
        // SAFETY: the caller's contract.
        unsafe { crate::blob::ffi::git_blob_create_from_buffer(id, repo.cast(), buffer, len) }
    }

    /// `git_object__is_valid`.
    ///
    /// # Safety
    ///
    /// As the export's.
    pub unsafe fn git_object__is_valid(
        repo: *mut c_void,
        id: Option<&Oid>,
        expected_type: c_int,
    ) -> bool {
        // SAFETY: the caller's contract.
        unsafe { crate::object::ffi::git_object__is_valid(repo.cast(), id, expected_type) }
    }

    pub use crate::object_api::ffi::{git_tree_free, git_tree_id};

    /// `git_tree_owner`.
    ///
    /// # Safety
    ///
    /// As the export's.
    pub unsafe fn git_tree_owner(tree: *const c_void) -> *mut c_void {
        // SAFETY: the caller's contract.
        unsafe { crate::object_api::ffi::git_tree_owner(tree) }.cast()
    }

    /// `git_tree_lookup`.
    ///
    /// # Safety
    ///
    /// As the export's.
    pub unsafe fn git_tree_lookup(
        out: *mut *mut c_void,
        repo: *mut c_void,
        id: Option<&Oid>,
    ) -> c_int {
        // SAFETY: the caller's contract.
        unsafe { crate::object_api::ffi::git_tree_lookup(out, repo.cast(), id) }
    }
}
