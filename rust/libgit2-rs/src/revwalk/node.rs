//! `git_commit_list_node` and `git_commit_list` (l-commit's types, from
//! `crate::commit_list`), for the walk.
//!
//! The nodes live in the walk's pool (`git_revwalk.commit_pool`) and C
//! reads and writes them (`git_commit_list_parse`, `merge.c`, `graph.c`,
//! `describe.c` set `parents`, `time`, `flags`, …), so the walk reaches
//! them only through [`NodePtr`], a raw pointer whose accessors read or
//! write one field at a time: no Rust reference to a node is ever formed.

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

/// `git_commit_list_node`: l-commit's type (`commit_list.c`, which owns
/// it). C allocates it from the walk's pool through
/// `git_commit_list_alloc_node`, and reads and writes its fields.
pub use crate::commit_list::GitCommitListNode as CommitListNode;
use ffibox::{CBitfield, CBits};

/// The node's bitfields: l-commit's declaration (`commit_list.rs`).
pub use crate::commit_list::{ADDED, FLAGS, PARSED, SEEN, TOPO_DELAY, UNINTERESTING};

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
    fn bits(self) -> CBitfield<u16> {
        // SAFETY: a live node (the invariant); one field read, no
        // reference formed.
        unsafe { field!(self.0.as_ptr(), bits: CBitfield<u16>).read() }
    }

    #[inline(always)]
    fn set_bits(self, bits: CBitfield<u16>) {
        // SAFETY: as in `bits`, for a write: nothing else reaches the node
        // during the write (single thread, no reference to it).
        unsafe { field!(self.0.as_ptr(), bits: CBitfield<u16>).write(bits) }
    }

    #[inline(always)]
    fn set_flag(self, field: CBits<u16>, on: bool) {
        let mut b = self.bits();
        b.set_flag(field, on);
        self.set_bits(b);
    }

    /// `seen`.
    #[inline(always)]
    pub fn seen(self) -> bool {
        self.bits().is_set(SEEN)
    }
    /// `seen = 1`.
    #[inline(always)]
    pub fn set_seen(self) {
        self.set_flag(SEEN, true);
    }
    /// `uninteresting`.
    #[inline(always)]
    pub fn uninteresting(self) -> bool {
        self.bits().is_set(UNINTERESTING)
    }
    /// `uninteresting = on`.
    #[inline(always)]
    pub fn set_uninteresting(self, on: bool) {
        self.set_flag(UNINTERESTING, on);
    }
    /// `parsed`.
    #[inline(always)]
    pub fn parsed(self) -> bool {
        self.bits().is_set(PARSED)
    }
    /// `added`.
    #[inline(always)]
    pub fn added(self) -> bool {
        self.bits().is_set(ADDED)
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
        let mut b = self.bits();
        for f in [SEEN, UNINTERESTING, TOPO_DELAY, ADDED, FLAGS] {
            b.set(f, 0);
        }
        self.set_bits(b);
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
    #[cfg(test)]
    #[inline(always)]
    pub fn as_void(self) -> *mut core::ffi::c_void {
        self.0.as_ptr().cast()
    }

    /// `time = v`.
    #[inline(always)]
    pub fn set_time(self, v: i64) {
        // SAFETY: as in `set_bits`.
        unsafe { field!(self.0.as_ptr(), time: i64).write(v) }
    }

    /// `generation = v`.
    #[inline(always)]
    pub fn set_generation(self, v: u32) {
        // SAFETY: as in `set_bits`.
        unsafe { field!(self.0.as_ptr(), generation: u32).write(v) }
    }

    /// `parsed = 1`.
    #[inline(always)]
    pub fn set_parsed_flag(self) {
        self.set_flag(PARSED, true);
    }

    /// The room for [`PARENTS_PER_COMMIT`] parent pointers after the node
    /// (a node is an item of [`COMMIT_ALLOC`] bytes).
    #[inline(always)]
    pub fn tail_slots(self) -> *mut *mut CommitListNode {
        self.0.as_ptr().wrapping_add(1).cast()
    }

    /// `parents = NULL; out_degree = n`: what a parse that ran out of
    /// memory for the parents leaves (C's `out_degree` set, `parents` NULL).
    #[inline]
    pub fn set_no_parents(self, n: u16) {
        let p = self.0.as_ptr();
        // SAFETY: as in `set_bits`.
        unsafe { field!(p, parents: *mut *mut CommitListNode).write(ptr::null_mut()) };
        // SAFETY: as in `set_bits` (with no array, any count is valid).
        unsafe { field!(p, out_degree: u16).write(n) };
    }

    /// `parents = arr; out_degree = n`, then `parents[i] = parent(i)` for
    /// each `i < n`, as `git_commit_list_parse` fills a node. If `parent`
    /// fails at `i`, the slots from `i` on are NULL (C leaves them
    /// unwritten, and a pool block is uninitialized: `parent` reads NULL
    /// as no parent) and the error is returned.
    ///
    /// # Safety
    ///
    /// `arr` is aligned and valid for writes and reads of `n` node pointers
    /// for as long as the walk's pool lives (the node's
    /// [`tail_slots`](Self::tail_slots) for `n <= 2`, or a block of the
    /// pool), and nothing else uses it; `parent` returns nodes of the same
    /// walk (or NULL), and does not touch this node or `arr`.
    #[inline]
    pub unsafe fn set_parents(
        self,
        arr: *mut *mut CommitListNode,
        n: u16,
        mut parent: impl FnMut(usize) -> Result<*mut CommitListNode>,
    ) -> Result<()> {
        let p = self.0.as_ptr();
        // SAFETY: as in `set_bits`.
        unsafe { field!(p, parents: *mut *mut CommitListNode).write(arr) };
        // SAFETY: as in `set_bits`.
        unsafe { field!(p, out_degree: u16).write(n) };
        let n = usize::from(n);
        for i in 0..n {
            match parent(i) {
                // SAFETY: `i < n`: a slot of `arr`, writable, used by
                // nothing else (the contract).
                Ok(q) => unsafe { arr.wrapping_add(i).write(q) },
                Err(e) => {
                    for j in i..n {
                        // SAFETY: as above, for `j < n`.
                        unsafe { arr.wrapping_add(j).write(ptr::null_mut()) };
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
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

/// `git_commit_list`: l-commit's type, a singly-linked list cell from
/// `git__allocator`. The walk's `user_input` is one (`pack-objects.c`
/// reads it); the walk's other lists are Rust-native.
pub use crate::commit_list::GitCommitList as CommitList;

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
