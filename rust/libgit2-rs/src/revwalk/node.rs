//! `git_commit_list_node` and `git_commit_list` as `commit_list.h` defines
//! them, for the walk. PENDING(l-commit): `commit_list.c` is ported by
//! l-commit, which owns these types; at integration, these mirrors give
//! way to its definitions (the layout test pins them to the header).
//!
//! The nodes live in the walk's pool (`git_revwalk.commit_pool`) and C
//! reads and writes them (`git_commit_list_parse`, `merge.c`, `graph.c`,
//! `describe.c` set `parents`, `time`, `flags`, …), so the walk reaches
//! them only through [`NodePtr`], a raw pointer whose accessors read or
//! write one field at a time: no Rust reference to a node is ever formed.

use core::ffi::c_void;
use core::ptr::{self, NonNull};

use crate::oid::Oid;
use crate::util::alloc;
use crate::util::errors::{self, Result};

/// A pointer to field `$f` (of type `$t`) of the node at `$p`: safe
/// pointer arithmetic, in bounds for a live node.
macro_rules! field {
    ($p:expr, $f:ident: $t:ty) => {
        $p.wrapping_byte_add(core::mem::offset_of!(CommitListNode, $f))
            .cast::<$t>()
    };
}

/// `git_commit_list_node` (mirrored, kind 3: C allocates it from the
/// walk's pool through `git_commit_list_alloc_node`, and reads and writes
/// its fields).
#[repr(C)]
#[derive(Debug)]
pub struct CommitListNode {
    /// `oid`.
    pub oid: Oid,
    /// `time`: the committer time.
    pub time: i64,
    /// `generation`: from the commit-graph, or 0.
    pub generation: u32,
    /// The bitfields `unsigned int seen:1, uninteresting:1, topo_delay:1,
    /// parsed:1, added:1, flags:4`, from the least significant bit (x86-64
    /// SysV). They take 9 bits, so the next (non-bitfield) member,
    /// `in_degree`, starts 2 bytes after them: they are a `u16` here.
    pub bits: u16,
    /// `in_degree`.
    pub in_degree: u16,
    /// `out_degree`: the number of parents.
    pub out_degree: u16,
    /// `parents`: `out_degree` node pointers; NULL until parsed.
    pub parents: *mut *mut CommitListNode,
}

/// `seen` (bit 0).
pub const SEEN: u16 = 1 << 0;
/// `uninteresting` (bit 1).
pub const UNINTERESTING: u16 = 1 << 1;
/// `topo_delay` (bit 2).
pub const TOPO_DELAY: u16 = 1 << 2;
/// `parsed` (bit 3).
pub const PARSED: u16 = 1 << 3;
/// `added` (bit 4).
pub const ADDED: u16 = 1 << 4;
/// `flags` (bits 5 to 8, `FLAG_BITS` = 4).
pub const FLAGS_MASK: u16 = 0xf << 5;

/// `PARENTS_PER_COMMIT`.
pub const PARENTS_PER_COMMIT: usize = 2;
/// `COMMIT_ALLOC`: the pool's item size, a node and room for two parents.
pub const COMMIT_ALLOC: usize =
    size_of::<CommitListNode>() + PARENTS_PER_COMMIT * size_of::<*mut CommitListNode>();

/// A node of a walk.
///
/// Invariant (from [`NodePtr::new`], the only constructor): the pointer is
/// to a live, initialized node of a walk's pool, and every use of the
/// `NodePtr` and of its copies happens while that pool lives, on the
/// thread that uses the walk (a walk is not thread-safe), and while no Rust
/// reference covers the node. The walk keeps `NodePtr`s only in its own
/// state, which it drops before its pool, so a `NodePtr` never outlives the
/// node; the type is private to the walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub(crate) struct NodePtr(NonNull<CommitListNode>);

impl NodePtr {
    /// The node at `p`.
    ///
    /// # Safety
    ///
    /// As the type's invariant: `p` is a live node of a walk's pool, and
    /// the `NodePtr` and its copies are used only while the pool lives,
    /// from the walk's thread, with no Rust reference to the node.
    #[inline(always)]
    pub unsafe fn new(p: NonNull<CommitListNode>) -> NodePtr {
        NodePtr(p)
    }

    /// The node at `p`, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// As [`NodePtr::new`], for a non-null `p`.
    #[inline(always)]
    pub unsafe fn from_raw(p: *mut CommitListNode) -> Option<NodePtr> {
        NonNull::new(p).map(NodePtr)
    }

    /// The pointer, for C.
    #[inline(always)]
    pub fn as_ptr(self) -> *mut CommitListNode {
        self.0.as_ptr()
    }

    #[inline(always)]
    fn bits(self) -> u16 {
        // SAFETY: a live node (the invariant); one field read, no
        // reference formed.
        unsafe { field!(self.0.as_ptr(), bits: u16).read() }
    }

    #[inline(always)]
    fn set_bits(self, bits: u16) {
        // SAFETY: as in `bits`, for a write: nothing else reaches the node
        // during the write (single thread, no reference to it).
        unsafe { field!(self.0.as_ptr(), bits: u16).write(bits) }
    }

    #[inline(always)]
    fn set_flag(self, mask: u16, on: bool) {
        let b = self.bits();
        self.set_bits(if on { b | mask } else { b & !mask });
    }

    /// `seen`.
    #[inline(always)]
    pub fn seen(self) -> bool {
        self.bits() & SEEN != 0
    }
    /// `seen = 1`.
    #[inline(always)]
    pub fn set_seen(self) {
        self.set_flag(SEEN, true);
    }
    /// `uninteresting`.
    #[inline(always)]
    pub fn uninteresting(self) -> bool {
        self.bits() & UNINTERESTING != 0
    }
    /// `uninteresting = on`.
    #[inline(always)]
    pub fn set_uninteresting(self, on: bool) {
        self.set_flag(UNINTERESTING, on);
    }
    /// `parsed`.
    #[inline(always)]
    pub fn parsed(self) -> bool {
        self.bits() & PARSED != 0
    }
    /// `added`.
    #[inline(always)]
    pub fn added(self) -> bool {
        self.bits() & ADDED != 0
    }
    /// `added = 1`.
    #[inline(always)]
    pub fn set_added(self) {
        self.set_flag(ADDED, true);
    }

    /// What `git_revwalk_reset` clears: `seen`, `in_degree`, `topo_delay`,
    /// `uninteresting`, `added` and `flags` (not `parsed`).
    #[inline]
    pub fn reset(self) {
        self.set_bits(self.bits() & !(SEEN | UNINTERESTING | TOPO_DELAY | ADDED | FLAGS_MASK));
        self.set_in_degree(0);
    }

    /// `time`.
    #[inline(always)]
    pub fn time(self) -> i64 {
        // SAFETY: as in `bits`.
        unsafe { field!(self.0.as_ptr(), time: i64).read() }
    }

    /// `&oid`, for C and for the map's key.
    #[inline(always)]
    pub fn oid_ptr(self) -> NonNull<Oid> {
        // SAFETY: a field of a live node: in bounds, non-null.
        unsafe { NonNull::new_unchecked(field!(self.0.as_ptr(), oid: Oid)) }
    }

    /// `oid`, copied.
    #[inline(always)]
    pub fn oid(self) -> Oid {
        // SAFETY: as in `bits`.
        unsafe { self.oid_ptr().as_ptr().read() }
    }

    /// `in_degree`.
    #[inline(always)]
    pub fn in_degree(self) -> u16 {
        // SAFETY: as in `bits`.
        unsafe { field!(self.0.as_ptr(), in_degree: u16).read() }
    }
    /// `in_degree = v`.
    #[inline(always)]
    pub fn set_in_degree(self, v: u16) {
        // SAFETY: as in `set_bits`.
        unsafe { field!(self.0.as_ptr(), in_degree: u16).write(v) }
    }

    /// `out_degree`.
    #[inline(always)]
    pub fn out_degree(self) -> usize {
        // SAFETY: as in `bits`.
        usize::from(unsafe { field!(self.0.as_ptr(), out_degree: u16).read() })
    }

    /// Whether `parents` is set (the node was parsed, or C set it).
    #[inline(always)]
    pub fn has_parents(self) -> bool {
        // SAFETY: as in `bits`.
        !unsafe { field!(self.0.as_ptr(), parents: *mut *mut CommitListNode).read() }.is_null()
    }

    /// `parents[i]`, for `i < out_degree`; `None` if the array is unset or
    /// the slot is NULL (C stores NULL when a parent's allocation failed).
    #[inline(always)]
    pub fn parent(self, i: usize) -> Option<NodePtr> {
        // SAFETY: as in `bits`.
        let parents = unsafe { field!(self.0.as_ptr(), parents: *mut *mut CommitListNode).read() };
        if parents.is_null() || i >= self.out_degree() {
            return None;
        }
        // SAFETY: a set `parents` holds `out_degree` slots (what
        // `git_commit_list_parse` allocates: in the node's tail or in the
        // pool), and `i < out_degree`.
        let p = unsafe { parents.wrapping_add(i).read() };
        // SAFETY: the slots point to nodes of the same pool
        // (`git_revwalk__commit_lookup`), or are NULL.
        unsafe { NodePtr::from_raw(p) }
    }

    /// The node as C's `void *`.
    #[inline(always)]
    pub fn as_void(self) -> *mut c_void {
        self.0.as_ptr().cast()
    }

    /// What `git_commit_list_parse` does to a node, for the tests' graphs:
    /// set `time`, `out_degree`, `parents` (`slots`: the node's tail for
    /// up to two parents, else an array that outlives the node) and
    /// `parsed`.
    #[cfg(test)]
    pub fn set_parsed(self, time: i64, parents: &[NodePtr], slots: *mut *mut CommitListNode) {
        let tail = self.0.as_ptr().wrapping_add(1);
        let arr = if parents.len() <= PARENTS_PER_COMMIT {
            tail.cast::<*mut CommitListNode>()
        } else {
            slots
        };
        for (i, p) in parents.iter().enumerate() {
            // SAFETY: `arr` has room for the parents (the node's tail of
            // COMMIT_ALLOC, or the caller's array).
            unsafe { arr.wrapping_add(i).write(p.as_ptr()) };
        }
        let n = self.0.as_ptr();
        // SAFETY: as in `set_bits`.
        unsafe { field!(n, time: i64).write(time) };
        // SAFETY: as in `set_bits`.
        unsafe { field!(n, out_degree: u16).write(parents.len() as u16) };
        // SAFETY: as in `set_bits`.
        unsafe { field!(n, parents: *mut *mut CommitListNode).write(arr) };
        self.set_flag(PARSED, true);
    }
}

/// `git_commit_list` (mirrored): a singly-linked list cell from
/// `git__allocator`. The walk's `user_input` is one (`pack-objects.c`
/// reads it); the walk's other lists are Rust-native.
#[repr(C)]
#[derive(Debug)]
pub struct CommitList {
    /// `item`.
    pub item: *mut CommitListNode,
    /// `next`.
    pub next: *mut CommitList,
}

/// `git_commit_list_insert`: a new cell holding `item` in front of `*at`.
pub(crate) fn list_insert(item: NodePtr, at: &mut *mut CommitList) -> Result<()> {
    let cell = alloc::malloc(size_of::<CommitList>()).cast::<CommitList>();
    if cell.is_null() {
        return Err(errors::oom());
    }
    // SAFETY: a fresh allocation of a `CommitList` (git__allocator's
    // alignment is 8).
    unsafe {
        cell.write(CommitList {
            item: item.as_ptr(),
            next: *at,
        })
    };
    *at = cell;
    Ok(())
}

/// `git_commit_list_insert_by_date`: insert `item` before the first cell
/// whose commit is older (`git_commit_list_time_cmp(p->item, item) > 0`).
///
/// # Safety
///
/// `*head` is a well-formed list (NULL-terminated, of live cells holding
/// [`NodePtr`]-valid nodes) that nothing else reaches during the call.
pub(crate) unsafe fn list_insert_by_date(item: NodePtr, head: &mut *mut CommitList) -> Result<()> {
    let time = item.time();
    let mut pp: *mut *mut CommitList = head;
    loop {
        // SAFETY: `pp` is `head` or the `next` of a live cell.
        let p = unsafe { *pp };
        if p.is_null() {
            break;
        }
        // SAFETY: a live cell.
        let item_p = unsafe { (*p).item };
        // SAFETY: its item is a valid node (the contract).
        let Some(n) = (unsafe { NodePtr::from_raw(item_p) }) else {
            break;
        };
        if n.time() < time {
            break;
        }
        // SAFETY: a live cell.
        pp = unsafe { &raw mut (*p).next };
    }
    // SAFETY: `pp` is valid and unaliased (the contract).
    list_insert(item, unsafe { &mut *pp })
}

/// `git_commit_list_free`.
///
/// # Safety
///
/// `*head` is a well-formed list whose cells nothing else uses after.
pub(crate) unsafe fn list_free(head: &mut *mut CommitList) {
    let mut p = core::mem::replace(head, ptr::null_mut());
    while !p.is_null() {
        // SAFETY: a live cell, read before it is freed.
        let next = unsafe { (*p).next };
        // SAFETY: a cell of git__allocator, not used after.
        unsafe { alloc::free(p.cast()) };
        p = next;
    }
}

/// The items of a list, in order.
///
/// # Safety
///
/// `head` is a well-formed list that stays unchanged while the iterator
/// is used, whose items are [`NodePtr`]-valid nodes.
pub(crate) unsafe fn list_items(head: *const CommitList) -> impl Iterator<Item = NodePtr> {
    let mut p = head;
    core::iter::from_fn(move || {
        if p.is_null() {
            return None;
        }
        // SAFETY: a live cell (the contract).
        let item = unsafe { (*p).item };
        // SAFETY: as above.
        let next = unsafe { (*p).next };
        p = next;
        // SAFETY: a valid node (the contract); C never stores NULL items.
        unsafe { NodePtr::from_raw(item) }
    })
}
