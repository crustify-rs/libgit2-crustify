//! Port of `src/libgit2/tree-cache.c`: the index's cache of tree ids (the
//! `TREE` extension), a tree of [`GitTreeCache`] nodes allocated in the
//! index's `tree_pool`.
//!
//! # Types
//!
//! [`GitTreeCache`] is `git_tree_cache`, mirrored (kind 3): C reads its
//! fields (`tree.c`'s `git_tree__write_index` reads `entry_count` and
//! `oid`, the tests read `children_count`), and the node ends in a flexible
//! array (`name`). Nodes and their `children` arrays live in a `git_pool`
//! (the index's), so they are freed together when the pool is cleared;
//! nothing frees a node on its own (`git_tree_cache_free` is declared in the
//! header but was never defined).
//!
//! A *tree* below means: NULL, or a node whose `children` array holds
//! `children_count` pointers to trees, all in one live pool. The parsers
//! only hand out complete trees; the functions that take a tree rely on
//! that (and on nothing writing the nodes meanwhile).
//!
//! The C parser and writer recurse once per level of the extension, so a
//! crafted index with a deeply nested `TREE` extension overflows the C
//! stack; the port walks the levels with an explicit stack (see
//! `rust/DEFECTS.md`).

use core::ffi::{c_char, c_int, c_void};
use core::ptr;

use crate::oid::{Oid, OidType, git_oid_t};
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::pool::GitPool;
use crate::util::str::GitStr;
use crate::util::util::strntol32;
use crate::git_error_set;

/// `git_tree_cache`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitTreeCache {
    /// The subtrees (`children_count` of them).
    pub children: *mut *mut GitTreeCache,
    /// The number of subtrees.
    pub children_count: usize,
    /// The id type of `oid`.
    pub oid_type: git_oid_t,
    /// The number of index entries under this tree; -1 when invalidated.
    pub entry_count: isize,
    /// The tree's id (when `entry_count >= 0`).
    pub oid: Oid,
    /// The length of `name`.
    pub namelen: usize,
    /// The NUL-terminated name (a flexible array).
    pub name: [c_char; 0],
}

/// `"corrupted TREE extension in index"`.
fn corrupted() -> ErrorCode {
    git_error_set!(ErrorClass::INDEX, "corrupted TREE extension in index")
}

impl GitTreeCache {
    /// The node's name.
    ///
    /// # Safety
    ///
    /// `self` is a node made by [`new`] (its `namelen` bytes follow it).
    unsafe fn name_bytes(&self) -> &[u8] {
        // SAFETY: the caller's contract: `namelen` initialized bytes follow
        // the node, in the same allocation, unwritten while borrowed.
        unsafe { core::slice::from_raw_parts(self.name.as_ptr().cast::<u8>(), self.namelen) }
    }

    /// The node's children (NULL slots are possible only while a parser
    /// fills them).
    ///
    /// # Safety
    ///
    /// `self` is a node of a tree (see the module documentation).
    unsafe fn children(&self) -> &[*mut GitTreeCache] {
        if self.children.is_null() || self.children_count == 0 {
            return &[];
        }
        // SAFETY: a tree's `children` holds `children_count` pointers.
        unsafe { core::slice::from_raw_parts(self.children, self.children_count) }
    }
}

/// Where nodes are allocated: a pool (`git_pool_malloc`/`mallocz` of
/// bytes).
pub trait NodeAlloc {
    /// `size` bytes, 8-aligned, zeroed if `zeroed`.
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>>;
}

impl NodeAlloc for GitPool {
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>> {
        if zeroed { self.mallocz(size) } else { self.malloc(size) }
    }
}

/// A pool C passed by pointer, borrowed only for each allocation (the tree
/// lookups between allocations may run C that reaches the pool's index).
pub struct RawPool(*mut GitPool);

impl RawPool {
    /// A handle on `pool`.
    ///
    /// # Safety
    ///
    /// `pool` is a valid, initialized pool that stays live while the
    /// handle is used, and that nothing else uses during an allocation.
    pub unsafe fn new(pool: *mut GitPool) -> RawPool {
        RawPool(pool)
    }
}

impl NodeAlloc for RawPool {
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>> {
        // SAFETY: `new`'s contract; the borrow ends with the allocation.
        unsafe { &mut *self.0 }.alloc(size, zeroed)
    }
}

/// `git_tree_cache_new`: a node named `name`, with no children and an
/// `entry_count` of 0, in `pool`.
pub fn new(name: &[u8], oid_type: git_oid_t, pool: &mut GitPool) -> Result<*mut GitTreeCache> {
    new_in(name, oid_type, pool)
}

/// [`new`] in any [`NodeAlloc`].
pub fn new_in<A: NodeAlloc + ?Sized>(name: &[u8], oid_type: git_oid_t, pool: &mut A) -> Result<*mut GitTreeCache> {
    let alloc_size = crate::util::integer::alloc_add(size_of::<GitTreeCache>(), name.len())
        .and_then(|n| crate::util::integer::alloc_add(n, 1))?;
    // pool blocks are 8-aligned (the pool's rounding)
    #[allow(clippy::cast_ptr_alignment)]
    let p = pool.alloc(alloc_size, false)?.as_ptr().cast::<GitTreeCache>();
    let node = GitTreeCache {
        children: ptr::null_mut(),
        children_count: 0,
        oid_type,
        entry_count: 0,
        oid: Oid::default(),
        namelen: name.len(),
        name: [],
    };
    // SAFETY: `p` is a fresh pool block of `alloc_size` bytes, aligned to 8
    // (the pool's rounding), room for the node and `name` and its NUL.
    unsafe { p.write(node) };
    // the bytes right after the node, within the block
    let name_ptr = p.cast::<u8>().wrapping_add(size_of::<GitTreeCache>());
    // SAFETY: as above; `name` is a distinct allocation.
    unsafe { ptr::copy_nonoverlapping(name.as_ptr(), name_ptr, name.len()) };
    // SAFETY: the last byte of the block.
    unsafe { name_ptr.add(name.len()).write(0) };
    Ok(p)
}

/// `find_child`: the child of `tree` named `name`.
///
/// # Safety
///
/// `tree` is a non-null node of a tree.
unsafe fn find_child(tree: *const GitTreeCache, name: &[u8]) -> *mut GitTreeCache {
    // SAFETY: the caller's contract.
    let children = unsafe { (*tree).children() };
    for &child in children {
        if child.is_null() {
            continue;
        }
        // SAFETY: a child of a tree is a node of it.
        let c = unsafe { &*child };
        // SAFETY: as above.
        if c.namelen == name.len() && name == unsafe { c.name_bytes() } {
            return child;
        }
    }
    ptr::null_mut()
}

/// `git_tree_cache_invalidate_path`: mark `tree` and the subtrees on the
/// way to `path` (up to its last directory) as invalid.
///
/// # Safety
///
/// `tree` is a tree (see the module documentation) that nothing else
/// accesses meanwhile.
pub unsafe fn invalidate_path(tree: *mut GitTreeCache, path: &[u8]) {
    let mut tree = tree;
    if tree.is_null() {
        return;
    }
    // SAFETY: a node of the tree, not otherwise accessed (the contract).
    unsafe { (*tree).entry_count = -1 };
    let mut rest = path;
    while let Some(slash) = memchr::memchr(b'/', rest) {
        // SAFETY: `tree` is a non-null node.
        tree = unsafe { find_child(tree, &rest[..slash]) };
        if tree.is_null() {
            return;
        }
        // SAFETY: as above.
        unsafe { (*tree).entry_count = -1 };
        rest = &rest[slash + 1..];
    }
}

/// `git_tree_cache_get`: the subtree of `tree` at `path` (a directory
/// path, with or without a trailing slash).
///
/// # Safety
///
/// `tree` is a tree (see the module documentation).
pub unsafe fn get(tree: *const GitTreeCache, path: &[u8]) -> *const GitTreeCache {
    let mut tree = tree;
    if tree.is_null() {
        return ptr::null();
    }
    let mut rest = path;
    loop {
        let slash = memchr::memchr(b'/', rest);
        let name = match slash {
            Some(s) => &rest[..s],
            None => rest,
        };
        // SAFETY: `tree` is a non-null node of the tree.
        tree = unsafe { find_child(tree, name) };
        if tree.is_null() {
            return ptr::null();
        }
        // C tests `*end + 1 == '\0'`, which is never true for '/': a
        // trailing slash is followed by an empty name, which no child has.
        match slash {
            None => return tree,
            Some(s) => rest = &rest[s + 1..],
        }
    }
}

/// The header of one node at `buf[*pos..]` (`read_tree_internal` up to the
/// children): the node, its children array allocated and zeroed.
fn read_node(
    buf: &[u8],
    pos: &mut usize,
    oid_type: OidType,
    pool: &mut GitPool,
) -> core::result::Result<*mut GitTreeCache, (ErrorCode, bool)> {
    // The second member: whether the failure is a corruption (C's
    // `goto corrupted`) rather than a failed allocation.
    let corrupt = || Err((corrupted(), true));
    let start = *pos;
    let Some(nul) = memchr::memchr(0, &buf[start..]) else {
        return corrupt();
    };
    let name = &buf[start..start + nul];
    let mut p = start + nul + 1;
    if p >= buf.len() {
        return corrupt();
    }
    let tree = new(name, oid_type.raw(), pool).map_err(|e| (e, false))?;

    // Blank-terminated ASCII decimal number of entries in this tree
    let Ok((count, used)) = strntol32(&buf[p..], 10) else {
        return corrupt();
    };
    p += used;
    // SAFETY: `tree` is the fresh node.
    unsafe { (*tree).entry_count = count as isize };
    if buf.get(p) != Some(&b' ') {
        return corrupt();
    }
    p += 1;
    if p >= buf.len() {
        return corrupt();
    }

    // Number of children of the tree, newline-terminated
    let children_count = match strntol32(&buf[p..], 10) {
        Ok((c, used)) if c >= 0 => {
            p += used;
            c as usize
        }
        _ => return corrupt(),
    };
    // SAFETY: as above.
    unsafe { (*tree).children_count = children_count };
    if buf.get(p) != Some(&b'\n') {
        return corrupt();
    }
    p += 1;

    // The OID is only there if it's not invalidated
    if count >= 0 {
        let oid_size = oid_type.size();
        if p + oid_size > buf.len() {
            return corrupt();
        }
        let oid = Oid::from_bytes(&buf[p..p + oid_size], oid_type).unwrap_or_default();
        // SAFETY: as above.
        unsafe { (*tree).oid = oid };
        p += oid_size;
    }

    if children_count > 0 {
        let bufsize =
            crate::util::integer::alloc_multiply(children_count, size_of::<*mut GitTreeCache>())
                .map_err(|e| (e, false))?;
        let children = pool.mallocz(bufsize).map_err(|e| (e, false))?;
        // SAFETY: as above.
        unsafe { (*tree).children = children.as_ptr().cast() };
    }
    *pos = p;
    Ok(tree)
}

/// `git_tree_cache_read`: parse a `TREE` extension's data into a tree in
/// `pool`.
pub fn read(buf: &[u8], oid_type: OidType, pool: &mut GitPool) -> Result<*mut GitTreeCache> {
    let mut t = ptr::null_mut();
    read_into(&mut t, buf, oid_type, pool)?;
    Ok(t)
}

/// [`read`] into `*out`, which is set once the whole tree is read, even
/// when trailing data then fails the read (as C).
pub fn read_into(
    out: &mut *mut GitTreeCache,
    buf: &[u8],
    oid_type: OidType,
    pool: &mut GitPool,
) -> Result<()> {
    let mut pos = 0usize;
    // The C recursion (`read_tree_internal`), with an explicit stack: a
    // failure below the root is reported as a corruption by every level
    // above it, so only the root's own allocation failures keep their
    // message.
    let root = match read_node(buf, &mut pos, oid_type, pool) {
        Ok(t) => t,
        Err((e, _)) => return Err(e),
    };
    let mut stack: Vec<(*mut GitTreeCache, usize)> = Vec::new();
    stack.try_reserve(1).map_err(|_| errors::oom())?;
    stack.push((root, 0));
    while let Some(&mut (node, ref mut next)) = stack.last_mut() {
        // SAFETY: `node` is a node made by `read_node`, in `pool`.
        let count = unsafe { (*node).children_count };
        if *next == count {
            stack.pop();
            continue;
        }
        let i = *next;
        *next += 1;
        let child = match read_node(buf, &mut pos, oid_type, pool) {
            Ok(c) => c,
            Err(_) => return Err(corrupted()),
        };
        // SAFETY: `node` is a node of `pool`, only accessed here.
        let slots = unsafe { (*node).children };
        // SAFETY: `node`'s children array has `count` slots, `i < count`.
        unsafe { slots.add(i).write(child) };
        if stack.try_reserve(1).is_err() {
            return Err(corrupted());
        }
        stack.push((child, 0));
    }
    *out = root;
    if pos < buf.len() {
        return Err(git_error_set!(
            ErrorClass::INDEX,
            "corrupted TREE extension in index (unexpected trailing data)"
        ));
    }
    Ok(())
}

/// The tree objects behind a [`read_tree`] (`tree.c`'s accessors).
pub trait TreeSource {
    /// A tree object (released on drop).
    type Tree;
    /// The tree's id (`git_tree_id`).
    fn id(&self, tree: &Self::Tree) -> Oid;
    /// The number of entries (`git_tree_entrycount`).
    fn len(&self, tree: &Self::Tree) -> usize;
    /// Entry `i`: whether it is a tree (`git_tree_entry_filemode` is
    /// `GIT_FILEMODE_TREE`), its name and id.
    fn entry<'t>(&self, tree: &'t Self::Tree, i: usize) -> (bool, &'t [u8], Oid);
    /// Look a subtree up by id (`git_tree_lookup`).
    fn lookup(&self, id: &Oid) -> Result<Self::Tree>;
}

/// `read_tree_recursive`.
fn read_tree_recursive<S: TreeSource, A: NodeAlloc + ?Sized>(
    src: &S,
    cache: *mut GitTreeCache,
    tree: &S::Tree,
    pool: &mut A,
) -> Result<()> {
    // SAFETY (for every access to `*cache` and `*child` below): the nodes
    // are fresh blocks of `pool`, made by `new`, that nothing else reaches
    // until the tree is returned.
    unsafe { (*cache).oid = src.id(tree) };
    let nentries = src.len(tree);
    let ntrees = (0..nentries).filter(|&i| src.entry(tree, i).0).count();
    let alloc_size =
        crate::util::integer::alloc_multiply(ntrees, size_of::<*mut GitTreeCache>())?;
    // SAFETY: as above.
    let oid_type = unsafe { (*cache).oid_type };
    // SAFETY: as above.
    unsafe { (*cache).children_count = ntrees };
    // pool blocks are 8-aligned (the pool's rounding)
    #[allow(clippy::cast_ptr_alignment)]
    let children = pool.alloc(alloc_size, true)?.as_ptr().cast::<*mut GitTreeCache>();
    // SAFETY: as above.
    unsafe { (*cache).children = children };
    let mut j = 0;
    for i in 0..nentries {
        let (is_tree, name, eid) = src.entry(tree, i);
        if !is_tree {
            // SAFETY: as above.
            unsafe { (*cache).entry_count += 1 };
            continue;
        }
        let child = new_in(name, oid_type, pool)?;
        // SAFETY: `children` has `ntrees` slots and `j < ntrees`.
        unsafe { children.add(j).write(child) };
        let subtree = src.lookup(&eid)?;
        let r = read_tree_recursive(src, child, &subtree, pool);
        drop(subtree);
        // SAFETY: as above.
        let n = unsafe { (*child).entry_count };
        // SAFETY: as above.
        unsafe { (*cache).entry_count += n };
        j += 1;
        r?;
    }
    Ok(())
}

/// `git_tree_cache_read_tree`: the tree cache of the tree object `tree`
/// (as `git read-tree` makes it), in `pool`.
pub fn read_tree<S: TreeSource, A: NodeAlloc + ?Sized>(
    src: &S,
    tree: &S::Tree,
    oid_type: git_oid_t,
    pool: &mut A,
) -> Result<*mut GitTreeCache> {
    let cache = new_in(b"", oid_type, pool)?;
    read_tree_recursive(src, cache, tree, pool)?;
    Ok(cache)
}

/// `git_tree_cache_write`: append the extension's data for `tree`.
///
/// # Safety
///
/// `tree` is a non-null tree (see the module documentation).
pub unsafe fn write(out: &mut GitStr, tree: *const GitTreeCache) -> Result<()> {
    // `write_tree`, pre-order, with an explicit stack.
    let mut stack: Vec<(*const GitTreeCache, usize)> = Vec::new();
    let mut node = tree;
    let mut i = 0usize;
    loop {
        if i == 0 {
            // SAFETY: a node of the tree (the contract).
            let n = unsafe { &*node };
            // SAFETY: as above.
            let name = unsafe { n.name_bytes() };
            // C: "%s%c%"PRIdZ" %"PRIuZ"\n" (12 characters), `%s` stopping at
            // the name's NUL.
            let name = memchr::memchr(0, name).map_or(name, |z| &name[..z]);
            let _ = out.printf(
                12,
                format_args!(
                    "{}\0{} {}\n",
                    crate::util::errors::Bytes(name),
                    n.entry_count,
                    n.children_count
                ),
            );
            if n.entry_count != -1 {
                let size = crate::oid::oid_size(n.oid_type);
                let _ = out.put(&n.oid.raw_id()[..size.min(crate::oid::GIT_OID_MAX_SIZE)]);
            }
        }
        // SAFETY: as above.
        let children = unsafe { (*node).children() };
        if i < children.len() {
            if stack.try_reserve(1).is_err() {
                return Err(errors::oom());
            }
            stack.push((node, i + 1));
            node = children[i];
            i = 0;
            continue;
        }
        match stack.pop() {
            Some((n, next)) => {
                node = n;
                i = next;
            }
            None => break,
        }
    }
    if out.is_oom() { Err(ErrorCode::GENERIC) } else { Ok(()) }
}

/// The C ABI of `tree-cache.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr_bytes, status};

    /// `git_tree_cache_write`.
    ///
    /// # Safety
    ///
    /// `out` is a valid `git_str`; `tree` is a non-null tree cache (made by
    /// these functions, in a live pool), not written meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_write(out: *mut GitStr, tree: *mut GitTreeCache) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract: a valid, unaliased `git_str`.
            let out = unsafe { &mut *out };
            // SAFETY: the caller's contract.
            status(unsafe { write(out, tree) })
        })
    }

    /// `git_tree_cache_read`.
    ///
    /// # Safety
    ///
    /// `tree` is writable; `buffer` is readable for `buffer_size` bytes;
    /// `pool` is a valid, initialized `git_pool` that nothing else uses
    /// meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_read(
        tree: *mut *mut GitTreeCache,
        buffer: *const c_char,
        buffer_size: usize,
        oid_type: git_oid_t,
        pool: *mut GitPool,
    ) -> c_int {
        barrier(|| {
            let Some(t) = OidType::from_raw(oid_type) else {
                return corrupted().raw();
            };
            // SAFETY: the caller's contract.
            let buf = unsafe { crate::util::export::bytes(buffer, buffer_size) };
            // SAFETY: as above.
            let pool = unsafe { &mut *pool };
            let mut t_out = ptr::null_mut();
            let r = read_into(&mut t_out, buf, t, pool);
            if !t_out.is_null() {
                // SAFETY: `tree` is writable (the contract).
                unsafe { tree.write(t_out) };
            }
            status(r)
        })
    }

    /// `git_tree_cache_invalidate_path`.
    ///
    /// # Safety
    ///
    /// `tree` is NULL or a tree cache not otherwise accessed meanwhile;
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_invalidate_path(tree: *mut GitTreeCache, path: *const c_char) {
        barrier(|| {
            // SAFETY: the caller's contract.
            let path = unsafe { cstr_bytes(path) };
            // SAFETY: as above.
            unsafe { invalidate_path(tree, path) }
        })
    }

    /// `git_tree_cache_get`.
    ///
    /// # Safety
    ///
    /// `tree` is NULL or a tree cache; `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_get(
        tree: *const GitTreeCache,
        path: *const c_char,
    ) -> *const GitTreeCache {
        barrier(|| {
            // SAFETY: the caller's contract.
            let path = unsafe { cstr_bytes(path) };
            // SAFETY: as above.
            unsafe { get(tree, path) }
        })
    }

    /// `git_tree_cache_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `name` is a NUL-terminated string; `pool` is a
    /// valid, initialized `git_pool` not used meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_new(
        out: *mut *mut GitTreeCache,
        name: *const c_char,
        oid_type: git_oid_t,
        pool: *mut GitPool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let name = unsafe { cstr_bytes(name) };
            // SAFETY: as above.
            let pool = unsafe { &mut *pool };
            match new(name, oid_type, pool) {
                Ok(t) => {
                    // SAFETY: as above.
                    unsafe { out.write(t) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_tree_cache_read_tree`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `tree` is a valid `git_tree`; `pool` is a valid,
    /// initialized `git_pool` not used meanwhile (the tree lookups may run
    /// other C, which must not use it either).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_read_tree(
        out: *mut *mut GitTreeCache,
        tree: *const c_void,
        oid_type: git_oid_t,
        pool: *mut GitPool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let (src, root) = unsafe { crate::index::foreign::CTrees::borrow(tree) };
            // SAFETY: as above.
            let mut pool = unsafe { RawPool::new(pool) };
            match read_tree(&src, &root, oid_type, &mut pool) {
                Ok(t) => {
                    // SAFETY: as above.
                    unsafe { out.write(t) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["tree-cache.h"],
    types: [
        GitTreeCache => "git_tree_cache" { children, children_count, oid_type, entry_count, oid, namelen, name },
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> GitPool {
        GitPool::with_item_size(1).unwrap()
    }

    fn ext(root_count: &str, children: &[&[u8]]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"\0");
        v.extend_from_slice(root_count.as_bytes());
        v.extend_from_slice(b"\n");
        for c in children {
            v.extend_from_slice(c);
        }
        v
    }

    #[test]
    fn read_write_roundtrip() {
        let _g = crate::util::test_support::init();
        let mut p = pool();
        let mut data = ext("3 1", &[]);
        data.extend_from_slice(&[0xaa; 20]);
        data.extend_from_slice(b"sub\0-1 1\n");
        data.extend_from_slice(b"deep\x002 0\n");
        data.extend_from_slice(&[0xbb; 20]);
        let t = read(&data, OidType::Sha1, &mut p).unwrap();
        // SAFETY: a tree just read, in `p`.
        unsafe {
            assert_eq!((*t).entry_count, 3);
            assert_eq!((*t).children_count, 1);
            let sub = get(t, b"sub");
            assert!(!sub.is_null());
            assert_eq!((*sub).entry_count, -1);
            let deep = get(t, b"sub/deep");
            assert_eq!((*deep).entry_count, 2);
            assert!(get(t, b"sub/").is_null());
            assert!(get(t, b"nope").is_null());
            let mut out = GitStr::new();
            write(&mut out, t).unwrap();
            assert_eq!(out.as_bytes(), &data[..]);
            invalidate_path(t, b"sub/deep/file");
            assert_eq!((*t).entry_count, -1);
            assert_eq!((*deep).entry_count, -1);
        }
    }

    #[test]
    fn corrupt_inputs() {
        let _g = crate::util::test_support::init();
        for bad in [
            &b""[..],
            b"abc",
            b"\0",
            b"\0x 0\n",
            b"\x001 0\n",
            b"\x001 -1\n",
            b"\x00-1 1\n",
            b"\x00-1 0",
            b"\x00-1 0x",
        ] {
            let mut p = pool();
            assert!(read(bad, OidType::Sha1, &mut p).is_err(), "{bad:?}");
            assert_eq!(
                errors::last_message().unwrap(),
                b"corrupted TREE extension in index"
            );
        }
        let mut p = pool();
        assert!(read(b"\x00-1 0\nX", OidType::Sha1, &mut p).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"corrupted TREE extension in index (unexpected trailing data)"
        );
    }

    #[test]
    fn deep_nesting_does_not_recurse() {
        let _g = crate::util::test_support::init();
        let mut data = Vec::new();
        for _ in 0..200_000 {
            data.extend_from_slice(b"d\0-1 1\n");
        }
        data.extend_from_slice(b"d\0-1 0\n");
        let mut p = pool();
        let t = read(&data, OidType::Sha1, &mut p).unwrap();
        let mut out = GitStr::new();
        // SAFETY: a tree just read.
        unsafe { write(&mut out, t).unwrap() };
        assert_eq!(out.as_bytes(), &data[..]);
    }
}
