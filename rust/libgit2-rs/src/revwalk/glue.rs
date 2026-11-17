//! The walk's calls into C: objects, references, commits and trees
//! (the other ported modules, `revwalk::deps`), and revparse, diff
//! and pathspec (C that stays, `sys::{revparse, diff, pathspec}`), as
//! small owners and safe functions over them.

use crate::util::cabi::CStrArg;
use core::ffi::{CStr, c_int, c_uint};
use core::ptr::{self, NonNull};

use super::GitRevwalk;
use super::node::NodePtr;
use crate::oid::Oid;
use crate::revwalk::deps as p;
use crate::sys::diff as d;
use crate::sys::pathspec::{self as ps, git_attr_fnmatch, git_pathspec};
use crate::sys::revparse as rp;
use crate::util::errors::{ErrorCode, Result};
use crate::util::vector::GitVector;

/// A `git_repository *` that the walk was made with.
///
/// Invariant (from [`Repo::new`]): NULL, or a live repository that
/// outlives every use of the `Repo` (C's contract: the repository outlives
/// the walk).
#[derive(Clone, Copy, Debug)]
pub struct Repo(*mut p::git_repository);

impl Repo {
    /// The repository at `r`.
    ///
    /// # Safety
    ///
    /// `r` is NULL or a live repository that outlives every use of the
    /// result and of its copies.
    pub unsafe fn new(r: *mut p::git_repository) -> Repo {
        Repo(r)
    }
}

/// `git_repository_odb`.
pub fn repository_odb(repo: Repo) -> Result<*mut p::git_odb> {
    let mut out = ptr::null_mut();
    // SAFETY: `repo.0` is NULL or a live repository (`Repo`'s invariant),
    // only read through `&` (its state is interior-mutable).
    let repo_ref = unsafe { repo.0.as_ref() };
    // SAFETY: `out` is a writable local; the repository is NULL (which the
    // export rejects with an error) or live.
    ErrorCode::check(unsafe {
        crate::repository::ffi::git_repository_odb(&raw mut out, repo_ref)
    })?;
    Ok(out)
}

/// `git_odb_free`.
///
/// # Safety
///
/// `odb` is NULL or a reference the caller owns, not used after.
pub unsafe fn odb_free(odb: *mut p::git_odb) {
    // SAFETY: NULL or an odb reference the caller owns and gives up, not
    // used after (the contract): what `git_odb_free` releases.
    unsafe { crate::odb::ffi::git_odb_free(odb) }
}

/// `git_commit_list_parse` of a node of `walk` (it is not parsed yet: the
/// Rust checks `parsed` first, as the C does on entry). It writes the node
/// (through `NodePtr`) and allocates from the pool and the map (in their
/// `UnsafeCell`s, borrowed only within the pool and map calls); it does
/// not touch the walk's state.
pub fn commit_list_parse(walk: &GitRevwalk, node: NodePtr) -> Result<()> {
    crate::commit_list::parse(walk, node)
}

/// An owned `git_object *`.
#[derive(Debug)]
pub struct Object(NonNull<p::git_object>);

impl Drop for Object {
    fn drop(&mut self) {
        // SAFETY: we own the object; not used after.
        unsafe { p::git_object_free(self.0.as_ptr()) }
    }
}

impl Object {
    /// Adopt `o` (NULL is `None`).
    ///
    /// # Safety
    ///
    /// A non-null `o` is an object the caller owns and hands over.
    unsafe fn from_raw(o: *mut p::git_object) -> Option<Object> {
        NonNull::new(o).map(Object)
    }

    /// `git_object_id`.
    pub fn id(&self) -> Oid {
        // SAFETY: a live object.
        let id = unsafe { p::git_object_id(self.0.as_ptr()) };
        // SAFETY: its id lives as long as it does, and is copied now.
        unsafe { *id }
    }

    /// `git_object_peel`.
    pub fn peel(&self, kind: c_int) -> Result<Object> {
        let mut out = ptr::null_mut();
        // SAFETY: a live object; `out` is writable.
        ErrorCode::check(unsafe { p::git_object_peel(&raw mut out, self.0.as_ptr(), kind) })?;
        // SAFETY: on success C hands us a new reference.
        unsafe { Object::from_raw(out) }.ok_or(ErrorCode::GENERIC)
    }
}

/// `git_object_lookup`.
pub fn object_lookup(repo: Repo, id: &Oid, kind: c_int) -> Result<Object> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` writable; `repo` NULL or live (the invariant); `id`
    // readable.
    ErrorCode::check(unsafe { p::git_object_lookup(&raw mut out, repo.0, Some(id), kind) })?;
    // SAFETY: on success C hands us a new reference.
    unsafe { Object::from_raw(out) }.ok_or(ErrorCode::GENERIC)
}

/// `git_reference_name_to_id`.
pub fn reference_name_to_id(repo: Repo, name: &CStr) -> Result<Oid> {
    let mut out = Oid::NONE;
    // SAFETY: `out` writable; `repo` NULL or live; `name` a string.
    ErrorCode::check(unsafe {
        crate::refs::ffi::git_reference_name_to_id(&raw mut out, repo.0, name.as_ptr())
    })?;
    Ok(out)
}

/// An owned `git_reference *`.
#[derive(Debug)]
pub struct Reference(NonNull<p::git_reference>);

impl Drop for Reference {
    fn drop(&mut self) {
        // SAFETY: we own the reference; not used after.
        unsafe { crate::refs::ffi::git_reference_free(self.0.as_ptr()) }
    }
}

impl Reference {
    /// `git_reference_name`.
    pub fn name(&self) -> &CStr {
        // SAFETY: a live reference.
        let p = unsafe { crate::refs::ffi::git_reference_name(self.0.as_ptr()) };
        // SAFETY: a reference's name is a string that lives, unchanged, as
        // long as the reference (borrowed from `self`).
        unsafe { CStr::from_ptr(p) }
    }
}

/// An owned `git_reference_iterator *`.
#[derive(Debug)]
pub struct RefIter(NonNull<p::git_reference_iterator>);

impl Drop for RefIter {
    fn drop(&mut self) {
        // SAFETY: we own the iterator; not used after.
        unsafe { crate::refs::ffi::git_reference_iterator_free(self.0.as_ptr()) }
    }
}

impl RefIter {
    /// `git_reference_iterator_glob_new`.
    pub fn glob(repo: Repo, glob: &CStr) -> Result<RefIter> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` writable; `repo` NULL or live; `glob` a string.
        ErrorCode::check(unsafe {
            crate::refs::ffi::git_reference_iterator_glob_new(&raw mut out, repo.0, glob.into())
        })?;
        NonNull::new(out).map(RefIter).ok_or(ErrorCode::GENERIC)
    }

    /// `git_reference_next`: `Err(ITER_OVER)` at the end.
    pub fn next(&mut self) -> Result<Reference> {
        let mut out = ptr::null_mut();
        // SAFETY: a live iterator; `out` writable.
        ErrorCode::check(unsafe {
            crate::refs::ffi::git_reference_next(&raw mut out, self.0.as_ptr())
        })?;
        NonNull::new(out).map(Reference).ok_or(ErrorCode::GENERIC)
    }
}

/// `git_revspec`, owning its objects.
#[derive(Debug)]
pub struct Revspec {
    /// `from`.
    pub from: Option<Object>,
    /// `to`.
    pub to: Option<Object>,
    /// `flags`.
    pub flags: c_uint,
}

/// `git_revparse`. Any non-zero status is an error (as the walk tests it).
pub fn revparse(repo: Repo, spec: &CStr) -> Result<Revspec> {
    let mut out = rp::git_revspec {
        from: ptr::null_mut(),
        to: ptr::null_mut(),
        flags: 0,
    };
    // SAFETY: `out` writable; `repo` NULL or live; `spec` a string.
    let e = unsafe { rp::git_revparse(&raw mut out, repo.0, spec.as_ptr()) };
    // SAFETY: what C stored in `out` is ours (NULL on failure).
    let from = unsafe { Object::from_raw(out.from) };
    // SAFETY: as above.
    let to = unsafe { Object::from_raw(out.to) };
    ErrorCode::check_zero(e)?;
    Ok(Revspec {
        from,
        to,
        flags: out.flags,
    })
}

/// `GIT_REVSPEC_MERGE_BASE`.
pub const GIT_REVSPEC_MERGE_BASE: c_uint = rp::GIT_REVSPEC_MERGE_BASE;

// ---------------------------------------------------------------------------
// Pathspec limiting
// ---------------------------------------------------------------------------

/// A `git_pathspec *` the walk was given.
///
/// Invariant (from [`Pathspec::new`]): a live pathspec that nothing
/// changes or frees while the `Pathspec` is used (C's contract for
/// `git_revwalk_pathspec`: it outlives the walks it limits).
#[derive(Clone, Copy, Debug)]
pub struct Pathspec(NonNull<git_pathspec>);

impl Pathspec {
    /// The pathspec at `p`.
    ///
    /// # Safety
    ///
    /// As the type's invariant.
    pub unsafe fn new(p: NonNull<git_pathspec>) -> Pathspec {
        Pathspec(p)
    }

    /// The pointer.
    pub fn as_ptr(self) -> *mut git_pathspec {
        self.0.as_ptr()
    }

    fn vector(&self) -> *const GitVector {
        // SAFETY: a live pathspec (the invariant): a field projection.
        unsafe { &raw const (*self.0.as_ptr()).pathspec }
    }

    /// The `git_attr_fnmatch *` items.
    fn matches(&self) -> impl Iterator<Item = *const git_attr_fnmatch> + '_ {
        // SAFETY: the vector of a live pathspec that nothing writes while
        // we use it (the invariant).
        let v = unsafe { &*self.vector() };
        v.iter().map(|m| m.cast_const().cast::<git_attr_fnmatch>())
    }

    /// `pathspec_has_wildcard`.
    pub fn has_wildcard(&self) -> bool {
        self.matches().any(|m| {
            // SAFETY: the items are live `git_attr_fnmatch`es of the
            // pathspec (built by `git_pathspec__vinit`).
            unsafe { (*m).flags & ps::GIT_ATTR_FNMATCH_HASWILD != 0 }
        })
    }

    fn patterns(&self) -> impl Iterator<Item = CStrArg<'_>> + '_ {
        self.matches().map(|m| {
            // SAFETY: as in `has_wildcard`.
            let p = unsafe { (*m).pattern };
            // SAFETY: a pattern string of the live pathspec, which `self`
            // borrows and nothing writes while it lives.
            unsafe { CStrArg::from_ptr(p) }
        })
    }

    /// `git_pathspec__match(&ps->pathspec, path, false, false, NULL, NULL)`.
    fn match_path(&self, path: CStrArg<'_>) -> bool {
        // SAFETY: a live vector of patterns; `path` is a string of a live
        // diff delta; the out-parameters are NULL, which C accepts.
        unsafe {
            ps::git_pathspec__match(
                self.vector(),
                path.as_ptr(),
                false,
                false,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        }
    }
}

/// An owned `git_commit *`.
struct Commit(NonNull<p::git_commit>);

impl Drop for Commit {
    fn drop(&mut self) {
        // SAFETY: we own the commit; not used after.
        unsafe { p::git_commit_free(self.0.as_ptr()) }
    }
}

impl Commit {
    fn lookup(repo: Repo, id: &Oid) -> Option<Commit> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` writable; `repo` live; `id` readable.
        if unsafe { p::git_commit_lookup(&raw mut out, repo.0, Some(id)) } != 0 {
            return None;
        }
        NonNull::new(out).map(Commit)
    }

    fn tree(&self) -> Option<Tree> {
        let mut out = ptr::null_mut();
        // SAFETY: a live commit; `out` writable.
        if unsafe { crate::commit::ffi::git_commit_tree(&raw mut out, self.0.as_ptr()) } != 0 {
            return None;
        }
        NonNull::new(out).map(Tree)
    }

    fn parentcount(&self) -> c_uint {
        // SAFETY: a live commit.
        unsafe { crate::commit::ffi::git_commit_parentcount(self.0.as_ptr()) }
    }

    fn parent(&self, n: c_uint) -> Option<Commit> {
        let mut out = ptr::null_mut();
        // SAFETY: a live commit; `out` writable.
        if unsafe { crate::commit::ffi::git_commit_parent(&raw mut out, self.0.as_ptr(), n) } != 0 {
            return None;
        }
        NonNull::new(out).map(Commit)
    }
}

/// An owned `git_tree *`.
struct Tree(NonNull<p::git_tree>);

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: we own the tree; not used after.
        unsafe { p::git_tree_free(self.0.as_ptr()) }
    }
}

/// An owned `git_tree_entry *`.
struct TreeEntry(NonNull<p::git_tree_entry>);

impl Drop for TreeEntry {
    fn drop(&mut self) {
        // SAFETY: we own the entry; not used after.
        unsafe { crate::tree::ffi::git_tree_entry_free(self.0.as_ptr()) }
    }
}

impl Tree {
    /// `git_tree_entry_bypath`; `None` on any failure.
    fn entry_bypath(&self, path: CStrArg<'_>) -> Option<TreeEntry> {
        let mut out = ptr::null_mut();
        // SAFETY: a live tree; `out` writable; `path` a pattern string of a
        // live pathspec.
        if unsafe { crate::tree::ffi::git_tree_entry_bypath(&raw mut out, self.0.as_ptr(), path) }
            != 0
        {
            return None;
        }
        NonNull::new(out).map(TreeEntry)
    }
}

impl TreeEntry {
    fn id(&self) -> Oid {
        // SAFETY: a live entry.
        let id = unsafe { crate::tree::ffi::git_tree_entry_id(self.0.as_ptr()) };
        // SAFETY: its id is copied while it lives.
        unsafe { *id }
    }
}

/// An owned `git_diff *`.
struct Diff(NonNull<d::git_diff>);

impl Drop for Diff {
    fn drop(&mut self) {
        // SAFETY: we own the diff; not used after.
        unsafe { d::git_diff_free(self.0.as_ptr().cast()) }
    }
}

/// `include_path_delta`.
fn include_path_delta(
    repo: Repo,
    spec: Pathspec,
    commit_tree: &Tree,
    parent_tree: Option<&Tree>,
    opts: &d::git_diff_options,
) -> bool {
    let mut out = ptr::null_mut();
    let old = parent_tree.map_or(ptr::null_mut(), |t| t.0.as_ptr());
    // SAFETY: `out` writable; `repo` live; the trees are live (or NULL for
    // the old side, which C accepts); `opts` is a valid options struct
    // whose pathspec strings outlive the call.
    if unsafe {
        d::git_diff_tree_to_tree(
            &raw mut out,
            repo.0,
            old,
            commit_tree.0.as_ptr(),
            core::ptr::from_ref(opts).cast(),
        )
    } != 0
    {
        return false;
    }
    let Some(diff) = NonNull::new(out).map(Diff) else {
        return false;
    };
    // SAFETY: a live diff.
    let n = unsafe { d::git_diff_num_deltas(diff.0.as_ptr().cast()) };
    for i in 0..n {
        // SAFETY: a live diff, `i` in range.
        let delta = unsafe { d::git_diff_get_delta(diff.0.as_ptr().cast(), i) };
        if delta.is_null() {
            continue;
        }
        // SAFETY: a live delta of the diff; its paths live as long as it.
        let newp = unsafe { (*delta).new_file.path };
        // SAFETY: as above.
        let oldp = unsafe { (*delta).old_file.path };
        // SAFETY: NULL or a path string of the delta, which lives, unwritten,
        // as long as the diff (held to the end of the loop).
        let newp = unsafe { CStrArg::from_ptr(newp) };
        // SAFETY: as above.
        let oldp = unsafe { CStrArg::from_ptr(oldp) };
        if (!newp.is_null() && spec.match_path(newp)) || (!oldp.is_null() && spec.match_path(oldp))
        {
            return true;
        }
    }
    false
}

/// `include_path_wildcard`.
fn include_path_wildcard(repo: Repo, spec: Pathspec, commit: &Commit, tree: &Tree) -> bool {
    let parents = commit.parentcount();
    let mut opts = d::git_diff_options::INIT;
    // SAFETY: a live pathspec (the invariant).
    let mut prefix = unsafe { (*spec.as_ptr()).prefix };
    // Narrow the diff to the common prefix only, as the C does.
    if !prefix.is_null() {
        opts.pathspec.strings = &raw mut prefix;
        opts.pathspec.count = 1;
    }

    if parents == 0 {
        return include_path_delta(repo, spec, tree, None, &opts);
    }
    // included only if it matches against every parent
    let mut include = true;
    let mut i = 0;
    while i < parents && include {
        include = false;
        if let Some(parent) = commit.parent(i)
            && let Some(ptree) = parent.tree()
            && include_path_delta(repo, spec, tree, Some(&ptree), &opts)
        {
            include = true;
        }
        i += 1;
    }
    include
}

/// `include_path_exact_root`.
fn include_path_exact_root(spec: Pathspec, tree: &Tree) -> bool {
    spec.patterns().any(|pat| tree.entry_bypath(pat).is_some())
}

/// `include_path_exact_parent`.
fn include_path_exact_parent(spec: Pathspec, tree: &Tree, ptree: &Tree) -> bool {
    for pat in spec.patterns() {
        let ce = tree.entry_bypath(pat);
        let pe = ptree.entry_bypath(pat);
        // added or removed; or both there with different ids
        let include = match (&ce, &pe) {
            (Some(c), Some(p)) => c.id() != p.id(),
            (None, None) => false,
            _ => true,
        };
        if include {
            return true;
        }
    }
    false
}

/// `include_path_exact`.
fn include_path_exact(spec: Pathspec, commit: &Commit, tree: &Tree) -> bool {
    let parents = commit.parentcount();
    if parents == 0 {
        return include_path_exact_root(spec, tree);
    }
    let mut include = true;
    let mut p = 0;
    while p < parents && include {
        if let Some(parent) = commit.parent(p)
            && let Some(ptree) = parent.tree()
            && !include_path_exact_parent(spec, tree, &ptree)
        {
            include = false;
        }
        p += 1;
    }
    include
}

/// `include_path`: whether `node`'s commit touches the pathspec.
pub fn include_path(repo: Repo, spec: Pathspec, wildcard: bool, node: NodePtr) -> bool {
    let id = node.oid();
    let Some(commit) = Commit::lookup(repo, &id) else {
        return false;
    };
    let Some(tree) = commit.tree() else {
        return false;
    };
    if wildcard {
        include_path_wildcard(repo, spec, &commit, &tree)
    } else {
        include_path_exact(spec, &commit, &tree)
    }
}
