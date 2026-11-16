//! The safe API of trees (PORTING.md §5): [`Tree`], its entries
//! ([`TreeEntryRef`], borrowed; [`TreeEntry`], owned), walks
//! ([`Tree::walk`], with a closure), [`TreeBuilder`], and
//! [`Tree::create_updated`].
//!
//! A [`Tree`] holds one reference to a libgit2 tree object, released on
//! drop. Its entries point into the object's data (zero-copy): `Tree<'a>`
//! parsed from bytes borrows them for `'a`; a tree looked up in a
//! repository keeps its odb object, and is tied to the repository's
//! lifetime. Entries borrowed from a tree are tied to the tree's borrow.

use core::ffi::c_int;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::ffi::CString;

use super::{GitTree, GitTreeEntry, GitTreeUpdate, GitTreebuilder, WalkResult};
use crate::commit::ec;
use crate::oid::{Oid, OidType};
use crate::pending_repository::Repository;
use crate::sys::pending::l_commit::{self as pend, git_repository};
use crate::util::errors::{Error, ErrorClass, ErrorCode};
use crate::util::str::GitStr;

fn invalid(msg: &str) -> Error {
    Error::new(ErrorCode::INVALID, ErrorClass::INVALID, msg)
}

fn cstring(s: &[u8], what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| invalid(&format!("{what} contains a NUL byte")))
}

fn err(e: c_int) -> Error {
    Error::from(ec(e))
}

/// The kind of object a tree entry names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A file or symbolic link.
    Blob,
    /// A directory.
    Tree,
    /// A submodule's commit.
    Commit,
}

impl EntryKind {
    fn of(attr: u16) -> EntryKind {
        match super::entry_type(attr) {
            crate::commit::OBJ_COMMIT => EntryKind::Commit,
            crate::commit::OBJ_TREE => EntryKind::Tree,
            _ => EntryKind::Blob,
        }
    }
}

/// An entry borrowed from a tree or a tree builder.
#[derive(Clone, Copy)]
pub struct TreeEntryRef<'t> {
    entry: &'t GitTreeEntry,
}

impl<'t> TreeEntryRef<'t> {
    /// The entry's name.
    pub fn name(&self) -> &'t [u8] {
        // SAFETY: the entry of a live tree or builder: its name lives (and
        // is unwritten) for `'t`.
        unsafe { self.entry.name() }
    }

    /// The id of the object it names.
    pub fn id(&self) -> Oid {
        self.entry.oid
    }

    /// `git_tree_entry_filemode`: the mode, normalised (a legacy `100664`
    /// is `100644`).
    pub fn filemode(&self) -> u32 {
        super::normalize_filemode(u32::from(self.entry.attr))
    }

    /// `git_tree_entry_filemode_raw`: the mode as stored.
    pub fn filemode_raw(&self) -> u32 {
        u32::from(self.entry.attr)
    }

    /// `git_tree_entry_type`.
    pub fn kind(&self) -> EntryKind {
        EntryKind::of(self.entry.attr)
    }

    /// An owned copy (`git_tree_entry_dup`).
    pub fn to_owned(&self) -> Result<TreeEntry, Error> {
        // SAFETY: as in `name`.
        let p = unsafe { super::entry_dup(self.entry) }.map_err(Error::from)?;
        // SAFETY: a fresh entry, ours.
        Ok(unsafe { TreeEntry::from_raw(p) })
    }

    /// `git_tree_entry_cmp`: git's tree order.
    pub fn cmp(&self, other: &TreeEntryRef<'_>) -> core::cmp::Ordering {
        crate::util::util::ordering(super::entry_cmp_parts(
            self.name(),
            self.entry.is_tree(),
            other.name(),
            other.entry.is_tree(),
        ))
    }
}

impl core::fmt::Debug for TreeEntryRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TreeEntry")
            .field("name", &String::from_utf8_lossy(self.name()))
            .field("id", &self.id())
            .field("mode", &format_args!("{:06o}", self.filemode_raw()))
            .finish()
    }
}

/// An owned tree entry (`git_tree_entry_dup`, `git_tree_entry_bypath`).
pub struct TreeEntry {
    raw: NonNull<GitTreeEntry>,
}

impl TreeEntry {
    /// # Safety
    ///
    /// `p` is a non-NULL `alloc_entry` block, handed over.
    unsafe fn from_raw(p: *mut GitTreeEntry) -> TreeEntry {
        // SAFETY: non-NULL (contract).
        TreeEntry {
            raw: unsafe { NonNull::new_unchecked(p) },
        }
    }

    /// The entry, borrowed.
    pub fn as_ref(&self) -> TreeEntryRef<'_> {
        // SAFETY: the block this value owns; nothing writes it.
        TreeEntryRef {
            entry: unsafe { self.raw.as_ref() },
        }
    }
}

impl Clone for TreeEntry {
    fn clone(&self) -> Self {
        self.as_ref()
            .to_owned()
            .expect("out of memory duplicating a tree entry")
    }
}

impl Drop for TreeEntry {
    fn drop(&mut self) {
        // SAFETY: the block this value owns, freed once.
        unsafe { crate::util::alloc::free(self.raw.as_ptr().cast()) }
    }
}

impl core::fmt::Debug for TreeEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// The order of a walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeWalkMode {
    /// A directory's entry before its contents.
    PreOrder,
    /// A directory's contents before its entry.
    PostOrder,
}

/// What a walk's closure says about an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeWalk {
    /// Go on.
    Continue,
    /// (Pre-order) do not descend into this directory.
    Skip,
    /// Stop the walk ([`Tree::walk`] returns `GIT_EUSER`).
    Stop,
}

/// An update for [`Tree::create_updated`].
#[derive(Clone, Copy, Debug)]
pub enum TreeUpdate<'a> {
    /// Add or replace the entry at the path.
    Upsert {
        path: &'a [u8],
        id: Oid,
        filemode: u32,
    },
    /// Remove the entry at the path.
    Remove { path: &'a [u8] },
}

/// A tree (`git_tree`).
pub struct Tree<'a> {
    raw: NonNull<GitTree>,
    _data: PhantomData<&'a [u8]>,
}

impl<'a> Tree<'a> {
    /// # Safety
    ///
    /// `raw` is a non-NULL parsed tree whose reference the caller hands over,
    /// and whose data and repository outlive `'a`.
    unsafe fn from_raw(raw: *mut GitTree) -> Tree<'a> {
        // SAFETY: non-NULL (contract).
        Tree {
            raw: unsafe { NonNull::new_unchecked(raw) },
            _data: PhantomData,
        }
    }

    /// `git_tree_lookup` in a C repository.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a repository that outlives `'a`.
    pub(crate) unsafe fn lookup_raw(
        repo: *mut git_repository,
        id: &Oid,
    ) -> Result<Tree<'a>, Error> {
        if repo.is_null() {
            // A parsed object has no repository (the C would crash).
            return Err(Error::from(crate::util::errors::assert_arg_failed("repo")));
        }
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; the repository per the contract (the C
        // asserts it is not NULL).
        let e = unsafe { pend::git_tree_lookup(&mut raw, repo, id) };
        if e < 0 {
            return Err(err(e));
        }
        // SAFETY: a tree the lookup returned, ours to release.
        Ok(unsafe { Tree::from_raw(raw) })
    }

    /// `git_tree_lookup`.
    pub fn lookup(repo: &'a Repository, id: &Oid) -> Result<Tree<'a>, Error> {
        // SAFETY: the repository lives for `'a`.
        unsafe { Tree::lookup_raw(repo.raw(), id) }
    }

    /// Parse a tree from its bytes (`git_object__from_raw`), outside any
    /// repository; its entries borrow `data`.
    pub fn parse(data: &'a [u8], oid_type: OidType) -> Result<Tree<'a>, Error> {
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; `data` readable, and borrowed by the tree
        // for `'a`.
        let e = unsafe {
            pend::git_object__from_raw(
                &mut raw,
                data.as_ptr().cast(),
                data.len(),
                crate::commit::OBJ_TREE,
                oid_type.raw(),
            )
        };
        if e < 0 {
            return Err(err(e));
        }
        // SAFETY: a tree just made, ours to release.
        Ok(unsafe { Tree::from_raw(raw.cast()) })
    }

    fn t(&self) -> *const GitTree {
        self.raw.as_ptr()
    }

    fn repo(&self) -> *mut git_repository {
        // SAFETY: a live tree.
        unsafe { crate::commit::object_owner(self.t()) }.unwrap_or(ptr::null_mut())
    }

    fn entries(&self) -> &[GitTreeEntry] {
        // SAFETY: a live tree; its entries are unchanged while it lives.
        unsafe { super::entries(self.t()) }
    }

    /// The tree's id.
    pub fn id(&self) -> Oid {
        // SAFETY: a live tree.
        unsafe { super::ffi::tree_id(self.t()) }
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Whether it has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// `git_tree_entry_byindex`.
    pub fn get(&self, i: usize) -> Option<TreeEntryRef<'_>> {
        self.entries().get(i).map(|entry| TreeEntryRef { entry })
    }

    /// The entries, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = TreeEntryRef<'_>> + '_ {
        self.entries().iter().map(|entry| TreeEntryRef { entry })
    }

    /// `git_tree_entry_byname`.
    pub fn get_name(&self, name: &[u8]) -> Option<TreeEntryRef<'_>> {
        // SAFETY: a live tree.
        unsafe { super::entry_fromname(self.t(), name) }.map(|entry| TreeEntryRef { entry })
    }

    /// `git_tree_entry_byid`: the first entry naming `id`.
    pub fn get_id(&self, id: &Oid) -> Option<TreeEntryRef<'_>> {
        self.entries()
            .iter()
            .find(|e| e.oid == *id)
            .map(|entry| TreeEntryRef { entry })
    }

    /// `git_tree_entry_bypath`: the entry at a `/`-separated path, looking
    /// the subtrees up in the tree's repository.
    pub fn get_path(&self, path: &[u8]) -> Result<TreeEntry, Error> {
        let repo = self.repo();
        let mut lookup = |id: &Oid| -> crate::util::errors::Result<*mut GitTree> {
            if repo.is_null() {
                return Err(crate::util::errors::assert_arg_failed("repo"));
            }
            let mut t = ptr::null_mut();
            // SAFETY: the tree's repository (or NULL: the C's assertion).
            ErrorCode::check(unsafe { pend::git_tree_lookup(&mut t, repo, id) })?;
            Ok(t)
        };
        // SAFETY: trees from `lookup`, released once.
        let mut free = |t: *mut GitTree| unsafe { pend::git_tree_free(t) };
        // SAFETY: a live tree.
        let p = unsafe { super::entry_bypath(self.t(), path, &mut lookup, &mut free) }
            .map_err(Error::from)?;
        // SAFETY: a fresh entry, ours.
        Ok(unsafe { TreeEntry::from_raw(p) })
    }

    /// `git_tree_walk`: `f(root, entry)` for every entry under the tree
    /// (`root` is the directory of the entry, with a trailing `/`, empty at
    /// the top). Subtrees are looked up in the tree's repository.
    pub fn walk(
        &self,
        mode: TreeWalkMode,
        mut f: impl FnMut(&[u8], TreeEntryRef<'_>) -> TreeWalk,
    ) -> Result<(), Error> {
        let repo = self.repo();
        let mut lookup = |id: &Oid| -> crate::util::errors::Result<*mut GitTree> {
            if repo.is_null() {
                return Err(crate::util::errors::assert_arg_failed("repo"));
            }
            let mut t = ptr::null_mut();
            // SAFETY: the tree's repository.
            ErrorCode::check(unsafe { pend::git_tree_lookup(&mut t, repo, id) })?;
            Ok(t)
        };
        // SAFETY: trees from `lookup`, released once.
        let mut free = |t: *mut GitTree| unsafe { pend::git_tree_free(t) };
        let mut cb = |root: &GitStr, entry: &GitTreeEntry| match f(
            root.as_bytes(),
            TreeEntryRef { entry },
        ) {
            TreeWalk::Continue => WalkResult::Continue,
            TreeWalk::Skip => WalkResult::Skip,
            TreeWalk::Stop => WalkResult::Stop(ErrorCode::USER.raw()),
        };
        let mut path = GitStr::new();
        // SAFETY: a live tree; `lookup` returns live trees, released by
        // `free`.
        let e = unsafe {
            super::walk(
                self.t(),
                &mut path,
                mode == TreeWalkMode::PreOrder,
                &mut lookup,
                &mut free,
                &mut cb,
            )
        };
        if e < 0 { Err(err(e)) } else { Ok(()) }
    }

    /// `git_tree_create_updated`: write the tree `baseline` (or an empty
    /// tree) with `updates` applied, and its subtrees, to `repo`.
    pub fn create_updated(
        repo: &Repository,
        baseline: Option<&Tree<'_>>,
        updates: &[TreeUpdate<'_>],
    ) -> Result<Oid, Error> {
        let mut paths = Vec::with_capacity(updates.len());
        for u in updates {
            let p = match u {
                TreeUpdate::Upsert { path, .. } | TreeUpdate::Remove { path } => path,
            };
            paths.push(cstring(p, "path")?);
        }
        let ups: Vec<GitTreeUpdate> = updates
            .iter()
            .zip(&paths)
            .map(|(u, p)| match *u {
                TreeUpdate::Upsert { id, filemode, .. } => GitTreeUpdate {
                    action: 0,
                    id,
                    filemode,
                    path: p.as_ptr(),
                },
                TreeUpdate::Remove { .. } => GitTreeUpdate {
                    action: 1,
                    id: Oid::default(),
                    filemode: 0,
                    path: p.as_ptr(),
                },
            })
            .collect();
        let base = baseline.map_or(ptr::null_mut(), |t| t.raw.as_ptr());
        let mut id = Oid::default();
        // SAFETY: a live repository; `base` NULL or a live tree; the updates'
        // paths live (in `paths`) during the call.
        unsafe { super::create_updated(&mut id, repo.raw(), base, &ups) }.map_err(Error::from)?;
        Ok(id)
    }
}

impl Clone for Tree<'_> {
    fn clone(&self) -> Self {
        let mut raw = ptr::null_mut();
        // SAFETY: a live object; `git_object_dup` only counts a reference.
        unsafe { pend::git_object_dup(&mut raw, self.raw.as_ptr().cast()) };
        // SAFETY: the reference just taken.
        unsafe { Tree::from_raw(raw.cast()) }
    }
}

impl Drop for Tree<'_> {
    fn drop(&mut self) {
        // SAFETY: the reference this value holds, released once.
        unsafe { pend::git_object_free(self.raw.as_ptr().cast()) }
    }
}

impl core::fmt::Debug for Tree<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tree")
            .field("id", &self.id())
            .field("entries", &self.len())
            .finish()
    }
}

/// A tree builder (`git_treebuilder`).
pub struct TreeBuilder<'r> {
    raw: NonNull<GitTreebuilder>,
    _repo: PhantomData<&'r Repository>,
}

impl<'r> TreeBuilder<'r> {
    /// `git_treebuilder_new`: a builder for `repo`, holding `source`'s
    /// entries.
    pub fn new(repo: &'r Repository, source: Option<&Tree<'_>>) -> Result<TreeBuilder<'r>, Error> {
        let src = source.map_or(ptr::null(), |t| t.t());
        // SAFETY: a live repository; `src` NULL or a live tree.
        let raw = unsafe { GitTreebuilder::new_raw(repo.raw(), src) }.map_err(Error::from)?;
        // SAFETY: non-NULL on success.
        Ok(TreeBuilder {
            raw: unsafe { NonNull::new_unchecked(raw) },
            _repo: PhantomData,
        })
    }

    fn b(&self) -> &GitTreebuilder {
        // SAFETY: the builder this value owns; only it reaches the builder.
        unsafe { self.raw.as_ref() }
    }

    fn b_mut(&mut self) -> &mut GitTreebuilder {
        // SAFETY: as above, exclusively.
        unsafe { self.raw.as_mut() }
    }

    /// `git_treebuilder_entrycount`.
    pub fn len(&self) -> usize {
        self.b().len()
    }

    /// Whether it has no entries.
    pub fn is_empty(&self) -> bool {
        self.b().is_empty()
    }

    /// `git_treebuilder_get`.
    pub fn get(&self, name: &[u8]) -> Option<TreeEntryRef<'_>> {
        let name = CString::new(name).ok()?;
        // SAFETY: an entry the builder owns, unchanged while `self` is
        // borrowed shared.
        self.b().get(&name).map(|e| TreeEntryRef {
            entry: unsafe { &*e },
        })
    }

    /// `git_treebuilder_insert`: add or replace the entry `name` (checked:
    /// a valid mode, name and non-zero id of an existing object of the
    /// mode's type, under strict object validation).
    pub fn insert(
        &mut self,
        name: &[u8],
        id: &Oid,
        filemode: u32,
    ) -> Result<TreeEntryRef<'_>, Error> {
        let name = cstring(name, "name")?;
        // SAFETY: the builder's repository outlives `'r`.
        let e = unsafe { self.b_mut().insert(&name, id, filemode) }.map_err(Error::from)?;
        // SAFETY: an entry the builder owns, unchanged while `self` is
        // borrowed.
        Ok(TreeEntryRef {
            entry: unsafe { &*e },
        })
    }

    /// `git_treebuilder_remove`.
    pub fn remove(&mut self, name: &[u8]) -> Result<(), Error> {
        let name = cstring(name, "name")?;
        if self.b().get(&name).is_none() {
            return Err(Error::from(super::tree_error(
                "failed to remove entry: file isn't in the tree",
                Some(name.to_bytes()),
            )));
        }
        self.b_mut().remove_found(&name);
        Ok(())
    }

    /// `git_treebuilder_clear`.
    pub fn clear(&mut self) {
        self.b_mut().clear();
    }

    /// `git_treebuilder_filter`: remove the entries for which `f` is true.
    pub fn filter(&mut self, mut f: impl FnMut(TreeEntryRef<'_>) -> bool) {
        let b = self.b_mut();
        let mut it = 0;
        while let Some((key, entry)) = b.map.iterate(&mut it).map(|(k, v)| (*k, *v)) {
            // SAFETY: an entry the builder owns.
            if f(TreeEntryRef {
                entry: unsafe { &*entry },
            }) {
                if let Some((_, e)) = b.map.remove(&key) {
                    // SAFETY: the entry left the map with its key.
                    unsafe { crate::util::alloc::free(e.cast()) };
                }
            }
        }
    }

    /// `git_treebuilder_write`: write the tree; its id.
    pub fn write(&mut self) -> Result<Oid, Error> {
        // SAFETY: the builder's repository outlives `'r`.
        unsafe { self.b_mut().write() }.map_err(Error::from)
    }
}

impl Drop for TreeBuilder<'_> {
    fn drop(&mut self) {
        // SAFETY: the builder this value owns, freed once.
        unsafe { super::free_raw(self.raw.as_ptr()) }
    }
}
