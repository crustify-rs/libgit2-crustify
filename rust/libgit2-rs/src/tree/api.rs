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
use crate::commit::deps::{self as pend, git_repository};
use crate::commit::ec;
use crate::object::{Object, ObjectType};
use crate::oid::{Oid, OidType};
use crate::repository::Repository;
use crate::util::errors::{Error, ErrorClass, ErrorCode};
use crate::util::str::GitStr;

fn invalid(msg: &str) -> Error {
    Error::with(ErrorCode::INVALID, ErrorClass::INVALID, msg)
}

fn cstring(s: &[u8], what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| invalid(&format!("{what} contains a NUL byte")))
}

fn err(e: c_int) -> Error {
    Error::from(ec(e))
}

/// An entry borrowed from a tree or a tree builder.
#[derive(Clone, Copy)]
pub struct TreeEntryRef<'t> {
    entry: &'t GitTreeEntry,
}

// SAFETY: the entry is read only while borrowed (`'t`): a tree's entries
// are immutable once parsed, a builder's are only changed through
// `&mut TreeBuilder`, which the borrow excludes. Its name pointer points
// into the tree's data or the entry's own block, both as immutable.
unsafe impl Send for TreeEntryRef<'_> {}
// SAFETY: as above; every method only reads.
unsafe impl Sync for TreeEntryRef<'_> {}

impl<'t> TreeEntryRef<'t> {
    /// The entry's name.
    pub fn name(&self) -> &'t [u8] {
        // SAFETY: the entry of a live tree or builder: its name lives (and
        // is unwritten) for `'t`.
        unsafe { self.entry.name() }
    }

    /// `git_tree_entry_id`: the id of the object it names.
    pub fn id(&self) -> &'t Oid {
        &self.entry.oid
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

    /// `git_tree_entry_type`: the type of the object it names (from the
    /// mode: a submodule's commit, a tree, else a blob; never invalid).
    pub fn kind(&self) -> ObjectType {
        ObjectType::from_raw(super::entry_type(self.entry.attr)).unwrap_or(ObjectType::Blob)
    }

    /// `git_tree_entry_to_object`: the object it names, looked up in
    /// `repo`.
    pub fn to_object<'r>(&self, repo: &'r Repository) -> Result<Object<'r>, Error> {
        Object::lookup(repo, &self.entry.oid, None)
    }

    /// An owned copy (`git_tree_entry_dup`).
    pub fn to_owned(&self) -> Result<TreeEntry, Error> {
        // SAFETY: as in `name`.
        let p = unsafe { super::entry_dup(self.entry) }.map_err(Error::from)?;
        // SAFETY: a fresh entry, ours.
        Ok(unsafe { TreeEntry::from_raw(p) })
    }

    /// `git_tree_entry_cmp`: git's tree order.
    pub fn cmp_tree_order(&self, other: &TreeEntryRef<'_>) -> core::cmp::Ordering {
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

// SAFETY: the value owns its `git__allocator` block (the entry and its
// name), which nothing else reaches; any thread may free it.
unsafe impl Send for TreeEntry {}
// SAFETY: `&self` methods only read the block.
unsafe impl Sync for TreeEntry {}

impl TreeEntry {
    /// # Safety
    ///
    /// `p` is a non-NULL `alloc_entry` block, handed over.
    unsafe fn from_raw(p: *mut GitTreeEntry) -> TreeEntry {
        TreeEntry {
            // SAFETY: non-NULL (contract).
            raw: unsafe { NonNull::new_unchecked(p) },
        }
    }

    /// The entry, borrowed.
    pub fn as_ref(&self) -> TreeEntryRef<'_> {
        TreeEntryRef {
            // SAFETY: the block this value owns; nothing writes it.
            entry: unsafe { self.raw.as_ref() },
        }
    }

    /// `git_tree_entry_name`.
    pub fn name(&self) -> &[u8] {
        self.as_ref().name()
    }

    /// `git_tree_entry_id`.
    pub fn id(&self) -> &Oid {
        self.as_ref().id()
    }

    /// `git_tree_entry_filemode`.
    pub fn filemode(&self) -> u32 {
        self.as_ref().filemode()
    }

    /// `git_tree_entry_filemode_raw`.
    pub fn filemode_raw(&self) -> u32 {
        self.as_ref().filemode_raw()
    }

    /// `git_tree_entry_type`.
    pub fn kind(&self) -> ObjectType {
        self.as_ref().kind()
    }

    /// `git_tree_entry_to_object`: the object it names, looked up in
    /// `repo`.
    pub fn to_object<'r>(&self, repo: &'r Repository) -> Result<Object<'r>, Error> {
        self.as_ref().to_object(repo)
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

/// A tree (`git_tree`): an [`Object`] of type tree, of a repository or
/// parsed from bytes ([`Tree::parse`], whose entries borrow them).
#[derive(Clone)]
#[repr(transparent)]
pub struct Tree<'a>(Object<'a>);

impl<'a> TryFrom<Object<'a>> for Tree<'a> {
    type Error = Object<'a>;

    /// The object as a tree, or the object back if it is not one.
    fn try_from(obj: Object<'a>) -> Result<Tree<'a>, Object<'a>> {
        if obj.kind() == ObjectType::Tree {
            Ok(Tree(obj))
        } else {
            Err(obj)
        }
    }
}

impl<'a> From<Tree<'a>> for Object<'a> {
    fn from(t: Tree<'a>) -> Object<'a> {
        t.0
    }
}

impl<'a> Tree<'a> {
    /// An object known to be a tree.
    pub(crate) fn wrap(obj: Object<'a>) -> Tree<'a> {
        debug_assert_eq!(obj.kind(), ObjectType::Tree);
        Tree(obj)
    }

    /// An object of type tree, borrowed as a tree.
    pub(crate) fn from_object_ref<'s>(obj: &'s Object<'a>) -> &'s Tree<'a> {
        debug_assert_eq!(obj.kind(), ObjectType::Tree);
        // SAFETY: `Tree` is a `repr(transparent)` wrapper of `Object`, whose
        // only invariant (the object is a tree) the caller checked.
        unsafe { &*ptr::from_ref(obj).cast::<Tree<'a>>() }
    }

    /// The `git_tree *`, live while `self` is (crate-internal).
    pub(crate) fn as_raw_tree(&self) -> NonNull<GitTree> {
        self.0.as_ptr().cast()
    }

    /// `git_tree_lookup`.
    pub fn lookup(repo: &'a Repository, id: &Oid) -> Result<Tree<'a>, Error> {
        Ok(Tree(Object::lookup(repo, id, Some(ObjectType::Tree))?))
    }

    /// `git_tree_lookup_prefix`: by the first `len` hex digits of `id`.
    pub fn lookup_prefix(repo: &'a Repository, id: &Oid, len: usize) -> Result<Tree<'a>, Error> {
        Ok(Tree(Object::lookup_prefix(
            repo,
            id,
            len,
            Some(ObjectType::Tree),
        )?))
    }

    /// Parse a tree from its bytes (`git_object__from_raw`), outside any
    /// repository; its entries borrow `data` (and
    /// [`get_path`](Self::get_path) and [`walk`](Self::walk) cannot descend
    /// into subtrees).
    pub fn parse(data: &'a [u8], oid_type: OidType) -> Result<Tree<'a>, Error> {
        crate::libgit2::ensure_init()?;
        // SAFETY: `data` readable for its length, and borrowed by the tree
        // for `'a`.
        let raw = unsafe {
            crate::object::from_raw(
                data.as_ptr().cast(),
                data.len(),
                crate::commit::OBJ_TREE,
                oid_type.raw(),
            )
        }?;
        // SAFETY: a tree just made, with its reference, of no repository;
        // its data lives for `'a`.
        Ok(Tree(unsafe { Object::adopt(raw) }))
    }

    /// The tree as an object.
    pub fn as_object(&self) -> &Object<'a> {
        &self.0
    }

    /// The tree as an object.
    pub fn into_object(self) -> Object<'a> {
        self.0
    }

    fn t(&self) -> *const GitTree {
        self.as_raw_tree().as_ptr()
    }

    fn repo(&self) -> *mut git_repository {
        self.0.repo_raw().cast()
    }

    fn entries(&self) -> &[GitTreeEntry] {
        // SAFETY: a live tree; its entries are unchanged while it lives.
        unsafe { super::entries(self.t()) }
    }

    /// `git_tree_id`.
    pub fn id(&self) -> &Oid {
        self.0.id()
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
    pub fn get_name(&self, name: impl AsRef<[u8]>) -> Option<TreeEntryRef<'_>> {
        // SAFETY: a live tree.
        unsafe { super::entry_fromname(self.t(), name.as_ref()) }
            .map(|entry| TreeEntryRef { entry })
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
    pub fn get_path(&self, path: impl AsRef<[u8]>) -> Result<TreeEntry, Error> {
        let path = path.as_ref();
        let repo = self.repo();
        let mut lookup = |id: &Oid| -> crate::util::errors::Result<*mut GitTree> {
            if repo.is_null() {
                return Err(crate::util::errors::assert_arg_failed("repo"));
            }
            let mut t = ptr::null_mut();
            // SAFETY: the tree's repository (or NULL: the C's assertion).
            ErrorCode::check(unsafe { pend::git_tree_lookup(&raw mut t, repo, Some(id)) })?;
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
            ErrorCode::check(unsafe { pend::git_tree_lookup(&raw mut t, repo, Some(id)) })?;
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
        let base = baseline.map_or(ptr::null_mut(), |t| t.as_raw_tree().as_ptr());
        let mut id = Oid::default();
        // SAFETY: a live repository; `base` NULL or a live tree; the updates'
        // paths live (in `paths`) during the call.
        unsafe { super::create_updated(&mut id, repo.raw_ptr(), base, &ups) }
            .map_err(Error::from)?;
        Ok(id)
    }
}

impl core::fmt::Debug for Tree<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tree")
            .field("id", self.id())
            .field("entries", &self.len())
            .finish()
    }
}

/// A tree builder (`git_treebuilder`).
pub struct TreeBuilder<'r> {
    raw: NonNull<GitTreebuilder>,
    _repo: PhantomData<&'r Repository>,
}

// SAFETY: the value owns its builder (a Rust map of `git__allocator`
// entries, and a buffer), which nothing else reaches; its repository is
// `Sync` and outlives `'r`. Any thread may free the entries.
unsafe impl Send for TreeBuilder<'_> {}
// SAFETY: `&self` methods (`len`, `is_empty`, `get`) only read the map;
// every change takes `&mut self`.
unsafe impl Sync for TreeBuilder<'_> {}

impl<'r> TreeBuilder<'r> {
    /// `git_treebuilder_new`: a builder for `repo`, holding `source`'s
    /// entries.
    pub fn new(repo: &'r Repository, source: Option<&Tree<'_>>) -> Result<TreeBuilder<'r>, Error> {
        let src = source.map_or(ptr::null(), |t| t.t());
        // SAFETY: a live repository; `src` NULL or a live tree.
        let raw = unsafe { GitTreebuilder::new_raw(repo.raw_ptr(), src) }.map_err(Error::from)?;
        Ok(TreeBuilder {
            // SAFETY: `new_raw` returns a non-NULL builder on success.
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
    pub fn get(&self, name: impl AsRef<[u8]>) -> Option<TreeEntryRef<'_>> {
        let name = CString::new(name.as_ref()).ok()?;
        self.b().get(&name).map(|e| TreeEntryRef {
            // SAFETY: an entry the builder owns, unchanged while `self` is
            // borrowed shared.
            entry: unsafe { &*e },
        })
    }

    /// `git_treebuilder_insert`: add or replace the entry `name` (checked:
    /// a valid mode, name and non-zero id of an existing object of the
    /// mode's type, under strict object validation).
    pub fn insert(
        &mut self,
        name: impl AsRef<[u8]>,
        id: &Oid,
        filemode: u32,
    ) -> Result<TreeEntryRef<'_>, Error> {
        let name = cstring(name.as_ref(), "name")?;
        // SAFETY: the builder's repository outlives `'r`.
        let e = unsafe { self.b_mut().insert(&name, id, filemode) }.map_err(Error::from)?;
        Ok(TreeEntryRef {
            // SAFETY: an entry the builder owns, unchanged while `self` is
            // borrowed.
            entry: unsafe { &*e },
        })
    }

    /// `git_treebuilder_remove`.
    pub fn remove(&mut self, name: impl AsRef<[u8]>) -> Result<(), Error> {
        let name = cstring(name.as_ref(), "name")?;
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
            if f(TreeEntryRef {
                // SAFETY: an entry the builder owns, not freed before the
                // callback returns.
                entry: unsafe { &*entry },
            }) && let Some((_, e)) = b.map.remove(&key)
            {
                // SAFETY: the entry left the map with its key.
                unsafe { crate::util::alloc::free(e.cast()) };
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
