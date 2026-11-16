//! The index's calls into code that is not this module's: the C that stays
//! C (`path.c`, `diff*.c`, `pathspec.c`, `ignore.c`, `submodule.c`, in
//! `crate::sys`) and the modules other work packages are porting
//! (`repository.c`, `refs.c`, `blob.c`, `object.c`, `object_api.c`,
//! `tree.c`: `crate::sys::pending::l_index`, PENDING). Each wrapper is safe
//! to call: its arguments are types whose constructors establish what the
//! C needs, and its `SAFETY` comment says why the call's contract holds.
//!
//! None of these is called while a reference into an index is live (see
//! the [index module](super)): any of them may run code that reaches the
//! index.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use crate::git_error_set;
use crate::oid::Oid;
use crate::strarray::GitStrArray;
use crate::sys::diff as d;
use crate::sys::pending::l_index as p;
use crate::sys::{ignore, path, pathspec, submodule};
use crate::tree_cache::TreeSource;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;

/// A repository: an index's owner (`GIT_REFCOUNT_OWNER`), which outlives
/// the index, or one C passed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repo(NonNull<c_void>);

impl Repo {
    /// A handle on `p`, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// A non-null `p` is a live `git_repository` for as long as the handle
    /// (or a copy) is used.
    pub unsafe fn new(p: *mut c_void) -> Option<Repo> {
        NonNull::new(p).map(Repo)
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut c_void {
        self.0.as_ptr()
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
// repository.c (PENDING(l-repo)), refs.c (PENDING(l-refs))
// ---------------------------------------------------------------------------

/// `GIT_CONFIGMAP_SYMLINKS`.
pub const CONFIGMAP_SYMLINKS: c_int = 2;
/// `GIT_CONFIGMAP_IGNORECASE`.
pub const CONFIGMAP_IGNORECASE: c_int = 3;
/// `GIT_CONFIGMAP_FILEMODE`.
pub const CONFIGMAP_FILEMODE: c_int = 4;

/// `git_repository__configmap_lookup`.
pub fn configmap_lookup(repo: Repo, item: c_int) -> Result<c_int> {
    let mut v = 0;
    // SAFETY: `repo` is live; `v` is a writable int.
    let r = unsafe { p::git_repository__configmap_lookup(&raw mut v, repo.as_ptr(), item) };
    ErrorCode::check(r).map(|_| v)
}

/// Whether `git_repository_workdir` is not NULL.
pub fn has_workdir(repo: Repo) -> bool {
    // SAFETY: `repo` is live.
    !unsafe { p::git_repository_workdir(repo.as_ptr()) }.is_null()
}

/// `git_repository__ensure_not_bare`.
pub fn ensure_not_bare(repo: Repo, operation_name: &str) -> Result<()> {
    // SAFETY: `repo` is live.
    if unsafe { p::git_repository_is_bare(repo.as_ptr()) } == 0 {
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
    // SAFETY: `out` is a valid `git_str`, `rel` a string, `repo` live.
    let r = unsafe { p::git_repository_workdir_path(&raw mut out, repo.as_ptr(), rel.as_ptr()) };
    ErrorCode::check(r).map(|_| out)
}

/// The id of `HEAD` of the repository at `path` (`add_repo_as_submodule`:
/// open, `git_repository_head`, `git_reference_target`).
pub fn head_target_of(path: &CStr) -> Result<Oid> {
    let mut sub = ptr::null_mut();
    // SAFETY: `sub` is writable, `path` a string.
    ErrorCode::check(unsafe { p::git_repository_open(&raw mut sub, path.as_ptr()) })?;
    let mut head = ptr::null_mut();
    // SAFETY: `sub` is the repository just opened; `head` writable.
    let r = unsafe { p::git_repository_head(&raw mut head, sub) };
    if r < 0 {
        // SAFETY: ours (C leaked it here).
        unsafe { p::git_repository_free(sub) };
        return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
    }
    // SAFETY: `head` is a live reference; its target (NULL for a symbolic
    // one) is read before it is freed.
    let target = unsafe { p::git_reference_target(head) };
    let id = if target.is_null() {
        Oid::default()
    } else {
        // SAFETY: as above.
        unsafe { *target }
    };
    // SAFETY: ours, freed once.
    unsafe { p::git_reference_free(head) };
    // SAFETY: as above.
    unsafe { p::git_repository_free(sub) };
    Ok(id)
}

/// `git_repository_index__weakptr`: the repository's index (no reference
/// taken).
pub fn repository_index(repo: Repo) -> Result<NonNull<super::GitIndex>> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` writable, `repo` live.
    ErrorCode::check(unsafe { p::git_repository_index__weakptr(&raw mut out, repo.as_ptr()) })?;
    NonNull::new(out.cast()).ok_or(ErrorCode::GENERIC)
}

// ---------------------------------------------------------------------------
// blob.c, object.c (PENDING(l-obj))
// ---------------------------------------------------------------------------

/// `git_blob__create_from_paths(&id, &st, repo, NULL, rel_path, 0, true)`:
/// write the working-directory file `rel_path` as a blob; its id, and its
/// `stat` in `st`.
pub fn blob_from_workdir(repo: Repo, rel_path: &CStr, st: &mut libc::stat) -> Result<Oid> {
    let mut id = Oid::default();
    // SAFETY: `id` and `st` are writable, `rel_path` a string, `repo` live.
    let r = unsafe {
        p::git_blob__create_from_paths(
            &raw mut id,
            st,
            repo.as_ptr(),
            ptr::null(),
            rel_path.as_ptr(),
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
    unsafe { p::git_object__is_valid(repo.as_ptr(), id, expected_type) }
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
// tree.c (PENDING(l-commit)), object_api.c (PENDING(l-obj))
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

impl TreeSource for CTrees {
    type Tree = CTree;

    fn id(&self, tree: &CTree) -> Oid {
        // SAFETY: a live tree (`CTree`); its id lives as long as it.
        let id = unsafe { p::git_tree_id(tree.ptr.as_ptr()) };
        // SAFETY: as above.
        unsafe { *id }
    }

    fn len(&self, tree: &CTree) -> usize {
        // SAFETY: as above.
        unsafe { p::git_tree_entrycount(tree.ptr.as_ptr()) }
    }

    fn entry<'t>(&self, tree: &'t CTree, i: usize) -> (bool, &'t [u8], Oid) {
        // SAFETY: as above; `i < len` (the caller), so the entry is the
        // tree's, which lives (unwritten) as long as the tree.
        let e = unsafe { p::git_tree_entry_byindex(tree.ptr.as_ptr(), i) };
        // SAFETY: as above.
        unsafe { entry_view(e) }
    }

    fn lookup(&self, id: &Oid) -> Result<CTree> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` writable; `repo` is the tree's live repository.
        ErrorCode::check(unsafe { p::git_tree_lookup(&raw mut out, self.repo, id) })?;
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
unsafe fn entry_view<'a>(e: *const c_void) -> (bool, &'a [u8], Oid) {
    // SAFETY: the caller's contract.
    let is_tree = unsafe { p::git_tree_entry_filemode(e) } == super::GIT_FILEMODE_TREE;
    // SAFETY: as above.
    let name = unsafe { p::git_tree_entry_name(e) };
    // SAFETY: as above: the name is a string of the entry.
    let name = unsafe { crate::util::export::cstr_bytes(name) };
    // SAFETY: as above.
    let id = unsafe { p::git_tree_entry_id(e) };
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
        entry: *const c_void,
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
            let attr = unsafe { p::git_tree_entry_filemode_raw(entry) };
            (pl.0)(root, &TreeEntryView { name, attr, id })
        })
    }
    let mut pl = Payload(f, PhantomData);
    // SAFETY: `tree` is live (`TreeRef`); `cb` only uses `pl` during the
    // walk.
    let r = unsafe { p::git_tree_walk(tree.as_ptr(), 1, cb, (&raw mut pl).cast()) };
    ErrorCode::check(r).map(|_| ())
}

/// `git_tree__write_index`.
///
/// # Safety
///
/// `index` is a live index that no reference covers during the call.
pub unsafe fn tree_write_index(index: *mut super::GitIndex, repo: Repo) -> Result<Oid> {
    let mut id = Oid::default();
    // SAFETY: `id` writable; the caller's contract for `index`; `repo`
    // live.
    let r = unsafe { p::git_tree__write_index(&raw mut id, index.cast(), repo.as_ptr()) };
    ErrorCode::check(r).map(|_| id)
}

// ---------------------------------------------------------------------------
// pathspec.c, ignore.c
// ---------------------------------------------------------------------------

/// The strings of a `git_strarray` argument, valid for `'a` (or none).
#[derive(Clone, Copy)]
pub struct Paths<'a>(*const GitStrArray, PhantomData<&'a ()>);

impl<'a> Paths<'a> {
    /// No paths (a NULL `git_strarray *`).
    pub fn none() -> Paths<'a> {
        Paths(ptr::null(), PhantomData)
    }

    /// The array at `raw` (NULL for none).
    ///
    /// # Safety
    ///
    /// A non-null `raw` is a valid `git_strarray`, its strings NULL or
    /// NUL-terminated, all readable and unwritten for `'a`.
    pub unsafe fn from_raw(raw: *const GitStrArray) -> Paths<'a> {
        Paths(raw, PhantomData)
    }

    /// An array over `ptrs` (which must stay as they are for `'a`).
    pub fn from_ptrs(arr: &'a GitStrArrayBuf) -> Paths<'a> {
        Paths(&raw const arr.raw, PhantomData)
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *const GitStrArray {
        self.0
    }
}

/// A `git_strarray` over borrowed strings.
pub struct GitStrArrayBuf {
    raw: GitStrArray,
    _ptrs: Vec<*mut c_char>,
}

impl GitStrArrayBuf {
    /// An array of the `strings` (which must outlive it: their pointers are
    /// taken).
    pub fn new(strings: &[&CStr]) -> GitStrArrayBuf {
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
        }
    }
}

/// A parsed pathspec (`git_pathspec`), cleared on drop.
pub struct Pathspec(Box<core::mem::MaybeUninit<pathspec::GitPathspec>>);

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

    /// Whether there is no delta.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
