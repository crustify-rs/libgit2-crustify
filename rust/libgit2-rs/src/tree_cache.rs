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

use crate::git_error_set;
use crate::oid::{Oid, OidType, git_oid_t};
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::pool::GitPool;
use crate::util::str::GitStr;
use crate::util::util::strntol32;

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

/// A node's name (through the node's pointer, whose provenance covers the
/// name that follows the node).
///
/// # Safety
///
/// `node` is a node made by [`new`] (its `namelen` bytes follow it), not
/// written during `'a`.
unsafe fn name_of<'a>(node: *const GitTreeCache) -> &'a [u8] {
    // SAFETY: a live node (this fn's contract); one field read.
    let n = unsafe { (*node).namelen };
    let p = node.cast::<u8>().wrapping_add(size_of::<GitTreeCache>());
    // SAFETY: a `new` node: `namelen` initialized bytes follow it in the
    // same block, which the node's pointer covers (so within `isize::MAX`),
    // unwritten during `'a` (this fn's contract).
    unsafe { core::slice::from_raw_parts(p, n) }
}

/// `"corrupted TREE extension in index"`.
fn corrupted() -> ErrorCode {
    git_error_set!(ErrorClass::INDEX, "corrupted TREE extension in index")
}

impl GitTreeCache {
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
        // SAFETY: `self` is a node of a tree (this fn's contract), whose
        // non-NULL `children` holds `children_count` initialized pointers in
        // one pool block, unwritten while `self` is borrowed.
        unsafe { core::slice::from_raw_parts(self.children, self.children_count) }
    }
}

/// Where nodes are allocated: a pool (`git_pool_malloc`/`mallocz` of
/// bytes).
///
/// # Safety
///
/// `alloc` returns a fresh block of at least `size` bytes, 8-aligned,
/// writable (and zeroed when asked), that stays valid as long as the
/// allocator; the tree cache writes nodes through it.
pub(crate) unsafe trait NodeAlloc {
    /// `size` bytes, 8-aligned, zeroed if `zeroed`.
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>>;
}

// SAFETY: `GitPool`'s blocks are fresh, 8-aligned (its rounding), of the
// size asked, valid until the pool is cleared.
unsafe impl NodeAlloc for GitPool {
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>> {
        if zeroed {
            self.mallocz(size)
        } else {
            self.malloc(size)
        }
    }
}

/// A pool C passed by pointer, borrowed only for each allocation (the tree
/// lookups between allocations may run C that reaches the pool's index).
pub(crate) struct RawPool(*mut GitPool);

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

// SAFETY: as the pool it points to (`RawPool::new`'s contract).
unsafe impl NodeAlloc for RawPool {
    fn alloc(&mut self, size: usize, zeroed: bool) -> Result<core::ptr::NonNull<u8>> {
        // SAFETY: a valid, initialized pool, live while the handle is and
        // used by nothing else during an allocation (`new`'s contract); the
        // borrow ends with the allocation.
        unsafe { &mut *self.0 }.alloc(size, zeroed)
    }
}

/// `git_tree_cache_new`: a node named `name`, with no children and an
/// `entry_count` of 0, in `pool`.
pub fn new(name: &[u8], oid_type: git_oid_t, pool: &mut GitPool) -> Result<*mut GitTreeCache> {
    new_in(name, oid_type, pool)
}

/// [`new`] in any [`NodeAlloc`].
pub(crate) fn new_in<A: NodeAlloc + ?Sized>(
    name: &[u8],
    oid_type: git_oid_t,
    pool: &mut A,
) -> Result<*mut GitTreeCache> {
    let alloc_size = crate::util::integer::alloc_add(size_of::<GitTreeCache>(), name.len())
        .and_then(|n| crate::util::integer::alloc_add(n, 1))?;
    // pool blocks are 8-aligned (the pool's rounding)
    #[allow(clippy::cast_ptr_alignment)]
    let p = pool
        .alloc(alloc_size, false)?
        .as_ptr()
        .cast::<GitTreeCache>();
    let node = GitTreeCache {
        children: ptr::null_mut(),
        children_count: 0,
        oid_type,
        entry_count: 0,
        oid: Oid::default(),
        namelen: name.len(),
        name: [],
    };
    const { assert!(align_of::<GitTreeCache>() <= 8) };
    // SAFETY: `p` is a fresh, writable block of `alloc_size` bytes (the
    // `NodeAlloc` contract), aligned to 8 (enough: asserted above), room for
    // the node, `name` and its NUL.
    unsafe { p.write(node) };
    // the bytes right after the node, within the block
    let name_ptr = p.cast::<u8>().wrapping_add(size_of::<GitTreeCache>());
    // SAFETY: as above: the name's room after the node, in the block;
    // `name` is a distinct allocation.
    unsafe { ptr::copy_nonoverlapping(name.as_ptr(), name_ptr, name.len()) };
    // SAFETY: as above: the last byte of the block.
    unsafe { name_ptr.wrapping_add(name.len()).write(0) };
    Ok(p)
}

/// `find_child`: the child of `tree` named `name`.
///
/// # Safety
///
/// `tree` is a non-null node of a tree, which nothing writes during the
/// call.
unsafe fn find_child(tree: *const GitTreeCache, name: &[u8]) -> *mut GitTreeCache {
    // SAFETY: a non-NULL node of a tree, unwritten during the call (this
    // fn's contract).
    let t = unsafe { &*tree };
    // SAFETY: as above: a node of a tree.
    let children = unsafe { t.children() };
    for &child in children {
        if child.is_null() {
            continue;
        }
        // SAFETY: a non-NULL child of a tree's node is a node of the tree
        // (live, unwritten).
        let c = unsafe { &*child };
        // SAFETY: as above.
        if c.namelen == name.len() && name == unsafe { name_of(child) } {
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
    // SAFETY: the tree's non-NULL root, which nothing else accesses during
    // the call (this fn's contract): written in place.
    unsafe { (*tree).entry_count = -1 };
    let mut rest = path;
    while let Some(slash) = memchr::memchr(b'/', rest) {
        // SAFETY: `tree` is a non-null node of the tree, unwritten during
        // the lookup.
        tree = unsafe { find_child(tree, &rest[..slash]) };
        if tree.is_null() {
            return;
        }
        // SAFETY: a non-NULL node of the tree, which nothing else accesses
        // (as above): written in place.
        unsafe { (*tree).entry_count = -1 };
        rest = &rest[slash + 1..];
    }
}

/// `git_tree_cache_get`: the subtree of `tree` at `path` (a directory
/// path, with or without a trailing slash).
///
/// # Safety
///
/// `tree` is a tree (see the module documentation) that nothing writes
/// during the call.
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
        // SAFETY: `tree` is a non-null node of the tree (this fn's
        // contract, or a child found below), unwritten during the call.
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
    // SAFETY: `tree` is the fresh node `new` just made, which only this
    // function reaches: written in place (here and below).
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

    // Every child takes at least 6 bytes ("\0-1 0\n"): a count the data
    // cannot hold is a corruption, found before C's allocation (and zeroing)
    // of up to 16 GiB for it (see DEFECTS.md).
    if children_count > (buf.len() - p) / 6 {
        return corrupt();
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
        // SAFETY: `node` is a node made by `read_node`, in `pool`, which
        // only this function reaches until it returns.
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
        // SAFETY: as above.
        let slots = unsafe { (*node).children };
        // SAFETY: `node`'s children array is a zeroed pool block of `count`
        // slots (`read_node`), and `i < count`.
        unsafe { slots.wrapping_add(i).write(child) };
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
///
/// # Safety
///
/// `len` and `entry` give the same answers for a tree as long as it lives
/// (the subtree count decides the size of the array the entries fill).
pub(crate) unsafe trait TreeSource {
    /// A tree object (released on drop).
    type Tree;
    /// The tree's id (`git_tree_id`).
    fn id(&self, tree: &Self::Tree) -> Oid;
    /// The number of entries (`git_tree_entrycount`).
    fn len(&self, tree: &Self::Tree) -> usize;
    /// Entry `i`: whether it is a tree (`git_tree_entry_filemode` is
    /// `GIT_FILEMODE_TREE`), its name and id.
    fn entry<'t>(&self, tree: &'t Self::Tree, i: usize) -> (bool, &'t [u8], Oid);
    /// Whether entry `i` is a tree (the first part of [`entry`](Self::entry)).
    fn is_tree(&self, tree: &Self::Tree, i: usize) -> bool {
        self.entry(tree, i).0
    }
    /// Look a subtree up by id (`git_tree_lookup`).
    fn lookup(&self, id: &Oid) -> Result<Self::Tree>;
}

/// `read_tree_recursive`.
///
/// # Safety
///
/// `cache` is a node made by [`new_in`] in `pool`, with no children yet,
/// that nothing else reaches during the call.
unsafe fn read_tree_recursive<S: TreeSource, A: NodeAlloc + ?Sized>(
    src: &S,
    cache: *mut GitTreeCache,
    tree: &S::Tree,
    pool: &mut A,
) -> Result<()> {
    let id = src.id(tree);
    // SAFETY: (this and every access to `*cache` and `*child` below) the
    // nodes are fresh blocks of `pool` made by `new_in` (this fn's
    // contract, and `new_in` below), that nothing else reaches until the
    // tree is returned; no reference to them is held across the accesses.
    unsafe { (*cache).oid = id };
    let nentries = src.len(tree);
    let ntrees = (0..nentries).filter(|&i| src.is_tree(tree, i)).count();
    let alloc_size = crate::util::integer::alloc_multiply(ntrees, size_of::<*mut GitTreeCache>())?;
    // SAFETY: as above.
    let oid_type = unsafe { (*cache).oid_type };
    // SAFETY: as above.
    unsafe { (*cache).children_count = ntrees };
    // pool blocks are 8-aligned (the pool's rounding)
    #[allow(clippy::cast_ptr_alignment)]
    let children = pool
        .alloc(alloc_size, true)?
        .as_ptr()
        .cast::<*mut GitTreeCache>();
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
        if j >= ntrees {
            // the source changed its answer (its contract forbids it)
            break;
        }
        let child = new_in(name, oid_type, pool)?;
        // SAFETY: `children` is a fresh, zeroed block of `ntrees` slots, and
        // `j < ntrees`.
        unsafe { children.wrapping_add(j).write(child) };
        let subtree = src.lookup(&eid)?;
        // SAFETY: `child` is the node just made in `pool`, which only this
        // call reaches (its parent's slot is not read meanwhile).
        let r = unsafe { read_tree_recursive(src, child, &subtree, pool) };
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
pub(crate) fn read_tree<S: TreeSource, A: NodeAlloc + ?Sized>(
    src: &S,
    tree: &S::Tree,
    oid_type: git_oid_t,
    pool: &mut A,
) -> Result<*mut GitTreeCache> {
    let cache = new_in(b"", oid_type, pool)?;
    // SAFETY: the node just made in `pool`, which only this call reaches.
    unsafe { read_tree_recursive(src, cache, tree, pool) }?;
    Ok(cache)
}

/// `git_tree_cache_write`: append the extension's data for `tree`.
///
/// # Safety
///
/// `tree` is a non-null tree (see the module documentation) that nothing
/// writes during the call.
pub unsafe fn write(out: &mut GitStr, tree: *const GitTreeCache) -> Result<()> {
    // `write_tree`, pre-order, with an explicit stack.
    let mut stack: Vec<(*const GitTreeCache, usize)> = Vec::new();
    let mut node = tree;
    let mut i = 0usize;
    loop {
        if i == 0 {
            // SAFETY: a node of the tree (this fn's contract: the root, or
            // a child of a node), live and unwritten during the call.
            let n = unsafe { &*node };
            // SAFETY: as above: a `new` node.
            let name = unsafe { name_of(node) };
            // C: "%s%c%"PRIdZ" %"PRIuZ"\n" (12 characters), `%s` stopping at
            // the name's NUL.
            let name = memchr::memchr(0, name).map_or(name, |z| &name[..z]);
            // the name's bytes as they are (never through a `Display`, which
            // would rewrite bytes that are not UTF-8); only the numbers are
            // formatted
            let _ = out.put(name);
            let _ = out.putc(0);
            let _ = out.printf(6, format_args!("{} {}\n", n.entry_count, n.children_count));
            if n.entry_count != -1 {
                let size = crate::oid::oid_size(n.oid_type);
                let _ = out.put(&n.oid.raw_id()[..size.min(crate::oid::GIT_OID_MAX_SIZE)]);
            }
        }
        // SAFETY: as above.
        let nref = unsafe { &*node };
        // SAFETY: as above.
        let children = unsafe { nref.children() };
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
    if out.is_oom() {
        Err(ErrorCode::GENERIC)
    } else {
        Ok(())
    }
}

/// The C ABI of `tree-cache.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};

    /// `git_tree_cache_write`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_str` that nothing else uses during the
    /// call; `tree` is a non-null tree cache (made by these functions, in a
    /// live pool), not written meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_write(
        out: Option<&mut GitStr>,
        tree: *mut GitTreeCache,
    ) -> c_int {
        barrier(|| {
            let out = git_arg!(out, "out", -1);
            // SAFETY: a non-NULL tree cache, unwritten during the call (this
            // export's contract).
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
        mut tree: Out<'_, *mut GitTreeCache>,
        buffer: *const c_char,
        buffer_size: usize,
        oid_type: git_oid_t,
        pool: Option<&mut GitPool>,
    ) -> c_int {
        barrier(|| {
            let Some(t) = OidType::from_raw(oid_type) else {
                return corrupted().raw();
            };
            // SAFETY: `buffer_size` readable bytes in one object (so within
            // `isize::MAX`), unwritten during the call (this export's
            // contract).
            let buf = unsafe { crate::util::export::bytes(buffer, buffer_size) };
            let pool = git_arg!(pool, "pool", -1);
            let mut t_out = ptr::null_mut();
            let r = read_into(&mut t_out, buf, t, pool);
            if !t_out.is_null() {
                tree.write(t_out);
            }
            status(r)
        })
    }

    /// `git_tree_cache_invalidate_path`.
    ///
    /// # Safety
    ///
    /// `tree` is NULL or a tree cache not otherwise accessed meanwhile;
    /// `path` is NULL or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_invalidate_path(
        tree: *mut GitTreeCache,
        path: CStrArg<'_>,
    ) {
        barrier(|| {
            let path = path.to_bytes();
            // SAFETY: NULL or a tree cache that nothing else accesses during
            // the call (this export's contract).
            unsafe { invalidate_path(tree, path) }
        })
    }

    /// `git_tree_cache_get`.
    ///
    /// # Safety
    ///
    /// `tree` is NULL or a tree cache, not written meanwhile; `path` is NULL
    /// or a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_cache_get(
        tree: *const GitTreeCache,
        path: CStrArg<'_>,
    ) -> *const GitTreeCache {
        barrier(|| {
            let path = path.to_bytes();
            // SAFETY: NULL or a tree cache, unwritten during the call (this
            // export's contract).
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
        mut out: Out<'_, *mut GitTreeCache>,
        name: CStrArg<'_>,
        oid_type: git_oid_t,
        pool: Option<&mut GitPool>,
    ) -> c_int {
        barrier(|| {
            let name = name.to_bytes();
            let pool = git_arg!(pool, "pool", -1);
            match new(name, oid_type, pool) {
                Ok(t) => {
                    out.write(t);
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
        mut out: Out<'_, *mut GitTreeCache>,
        tree: *const c_void,
        oid_type: git_oid_t,
        pool: *mut GitPool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live tree for the call (this export's contract),
            // which `src` and `root` do not outlive.
            let (src, root) = unsafe { crate::index::foreign::CTrees::borrow(tree) };
            // SAFETY: a valid, initialized pool, live for the call, that
            // nothing else uses meanwhile, C included (this export's
            // contract).
            let mut pool = unsafe { RawPool::new(pool) };
            match read_tree(&src, &root, oid_type, &mut pool) {
                Ok(t) => {
                    out.write(t);
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
        GitTreeCache => "git_tree_cache" { children, children_count, oid_type, entry_count, oid, namelen },
    ],
    asserts: ["offsetof(git_tree_cache, name) == 80"],
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
        // SAFETY: (all below) a tree just read, in `p`, which outlives it.
        let node = |x: *const GitTreeCache| unsafe { &*x };
        assert_eq!(node(t).entry_count, 3);
        assert_eq!(node(t).children_count, 1);
        // SAFETY: as above.
        let sub = unsafe { get(t, b"sub") };
        assert!(!sub.is_null());
        assert_eq!(node(sub).entry_count, -1);
        // SAFETY: as above.
        let deep = unsafe { get(t, b"sub/deep") };
        assert_eq!(node(deep).entry_count, 2);
        // SAFETY: as above.
        assert!(unsafe { get(t, b"sub/") }.is_null());
        // SAFETY: as above.
        assert!(unsafe { get(t, b"nope") }.is_null());
        let mut out = GitStr::new();
        // SAFETY: as above.
        unsafe { write(&mut out, t) }.unwrap();
        assert_eq!(out.as_bytes(), &data[..]);
        // SAFETY: as above; no reference is live.
        unsafe { invalidate_path(t, b"sub/deep/file") };
        assert_eq!(node(t).entry_count, -1);
        assert_eq!(node(deep).entry_count, -1);
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
    fn names_are_written_byte_for_byte() {
        let _g = crate::util::test_support::init();
        let mut p = pool();
        let data = b"\x00-1 1\n\xff\xfe\x00-1 0\n".to_vec();
        let t = read(&data, OidType::Sha1, &mut p).unwrap();
        let mut out = GitStr::new();
        // SAFETY: a tree just read.
        unsafe { write(&mut out, t) }.unwrap();
        assert_eq!(out.as_bytes(), &data[..]);
    }

    #[test]
    fn name_follows_the_node() {
        assert_eq!(core::mem::offset_of!(GitTreeCache, name), 80);
    }

    #[test]
    fn deep_nesting_does_not_recurse() {
        let _g = crate::util::test_support::init();
        let mut data = Vec::new();
        for _ in 0..if cfg!(miri) { 300 } else { 200_000 } {
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
