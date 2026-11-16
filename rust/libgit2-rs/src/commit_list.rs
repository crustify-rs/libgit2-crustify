//! `src/libgit2/commit_list.c`: the commit nodes of a revision walk, and
//! the linked lists and comparisons that `revwalk.c`, `merge.c`, `graph.c`
//! and `describe.c` (C that stays C, or l-revwalk's) build from them.
//!
//! [`GitCommitListNode`] and [`GitCommitList`] are *mirrored* types
//! (PORTING.md §4.1): the C allocates them (nodes from the walk's pool, with
//! `COMMIT_ALLOC` bytes each; lists with `git__malloc`), reads and writes
//! their fields, and sorts them.
//!
//! The nodes belong to a `git_revwalk`, which libgit2 does not share
//! between threads; but the C that stays holds pointers to them across the
//! calls here (and [`parse`] calls back into `git_revwalk__commit_lookup`,
//! which allocates more nodes from the same pool). So this module never
//! keeps a Rust reference to a node or to the pool across a call into C: it
//! writes the fields through the raw pointer, and borrows the pool only for
//! the extent of one allocation.

use core::ffi::{c_int, c_void};
use core::ptr;

use crate::commit::deps::{self as pend, git_revwalk};
use crate::commit::{self, CommitSink, SigFields};
use crate::git_error_set;
use crate::oid::{Oid, git_oid_t};
use crate::sys::commit_graph;
use crate::util::alloc;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::pool::GitPool;

/// `PARENT1`.
pub const PARENT1: u32 = 1 << 0;
/// `PARENT2`.
pub const PARENT2: u32 = 1 << 1;
/// `RESULT`.
pub const RESULT: u32 = 1 << 2;
/// `STALE`.
pub const STALE: u32 = 1 << 3;
/// `PARENTS_PER_COMMIT`: parents stored inline after a node.
pub const PARENTS_PER_COMMIT: usize = 2;
/// `COMMIT_ALLOC`: the pool item size of a walk (a node and two parents).
pub const COMMIT_ALLOC: usize =
    size_of::<GitCommitListNode>() + PARENTS_PER_COMMIT * size_of::<*mut GitCommitListNode>();

/// `struct git_commit_list_node` (`commit_list.h`).
///
/// The C bitfields `seen:1, uninteresting:1, topo_delay:1, parsed:1,
/// added:1, flags:4` share an `unsigned int` with `in_degree`: they are the
/// low 9 bits of [`bits`](Self::bits) (x86-64 SysV: the first declared is
/// the least significant), at offset 52; `in_degree` is the next 16 bits.
#[repr(C)]
pub struct GitCommitListNode {
    pub oid: Oid,
    pub time: i64,
    pub generation: u32,
    pub bits: u16,
    pub in_degree: u16,
    pub out_degree: u16,
    pub parents: *mut *mut GitCommitListNode,
}

/// The bit of `seen`.
pub const BIT_SEEN: u16 = 1 << 0;
/// The bit of `uninteresting`.
pub const BIT_UNINTERESTING: u16 = 1 << 1;
/// The bit of `topo_delay`.
pub const BIT_TOPO_DELAY: u16 = 1 << 2;
/// The bit of `parsed`.
pub const BIT_PARSED: u16 = 1 << 3;
/// The bit of `added`.
pub const BIT_ADDED: u16 = 1 << 4;
/// The shift of `flags` (4 bits).
pub const FLAGS_SHIFT: u16 = 5;

impl GitCommitListNode {
    /// `parsed`.
    pub fn parsed(&self) -> bool {
        self.bits & BIT_PARSED != 0
    }
    /// `flags` (4 bits).
    pub fn flags(&self) -> u32 {
        u32::from((self.bits >> FLAGS_SHIFT) & 0xf)
    }
    /// Set `flags` (the low 4 bits of `f`).
    pub fn set_flags(&mut self, f: u32) {
        self.bits = (self.bits & !(0xf << FLAGS_SHIFT)) | (((f & 0xf) as u16) << FLAGS_SHIFT);
    }
    /// The parents (`out_degree` of them).
    ///
    /// # Safety
    ///
    /// The node was parsed (its `parents` holds `out_degree` pointers), and
    /// nothing writes them while the slice lives.
    pub unsafe fn parent_slice(&self) -> &[*mut GitCommitListNode] {
        if self.parents.is_null() || self.out_degree == 0 {
            return &[];
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(self.parents, usize::from(self.out_degree)) }
    }
}

/// `struct git_commit_list` (`commit_list.h`).
#[repr(C)]
pub struct GitCommitList {
    pub item: *mut GitCommitListNode,
    pub next: *mut GitCommitList,
}

/// `git_commit_list_time_cmp` over two times: the newer first.
#[inline]
pub fn time_cmp(a: i64, b: i64) -> c_int {
    (a < b) as c_int - (a > b) as c_int
}

/// `git_commit_list_generation_cmp` over `(generation, time)`: the higher
/// generation first, by time when either has none.
#[inline]
pub fn generation_cmp(a: (u32, i64), b: (u32, i64)) -> c_int {
    if a.0 == 0 || b.0 == 0 {
        // Fall back to comparing by timestamps if at least one commit lacks
        // a generation.
        return time_cmp(a.1, b.1);
    }
    (a.0 < b.0) as c_int - (a.0 > b.0) as c_int
}

/// The walk's pool, for one allocation.
///
/// # Safety
///
/// `walk` is a live walk that nothing else uses during `f`.
unsafe fn with_pool<R>(walk: *mut git_revwalk, f: impl FnOnce(&mut GitPool) -> R) -> R {
    // SAFETY: a live walk's pool, a `git_pool` by value (contract).
    let pool = unsafe { (*walk).prefix.commit_pool.get() };
    // SAFETY: nothing else touches the pool during `f` (contract), and `f`
    // calls no C.
    let pool = unsafe { &mut *pool };
    f(pool)
}

/// `alloc_parents`: room for `n` parent pointers: inline after the node for
/// up to two, from the walk's pool otherwise.
///
/// # Safety
///
/// `walk` is live; `node` one of its nodes (allocated with `COMMIT_ALLOC`
/// bytes).
unsafe fn alloc_parents(
    walk: *mut git_revwalk,
    node: *mut GitCommitListNode,
    n: usize,
) -> *mut *mut GitCommitListNode {
    if n <= PARENTS_PER_COMMIT {
        // The node's item holds two pointers after it.
        return node.wrapping_add(1).cast();
    }
    let Some(bytes) = n.checked_mul(size_of::<*mut GitCommitListNode>()) else {
        return ptr::null_mut();
    };
    // As C: `git_pool_malloc(pool, bytes)`, `bytes` items of the pool's size.
    // SAFETY: the walk is live and unused meanwhile (contract).
    unsafe {
        with_pool(walk, |p| {
            p.malloc(bytes)
                .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
        })
    }
}

/// The parents and the commit time of a quick parse.
#[derive(Default)]
struct QuickSink {
    inline: [Oid; PARENTS_PER_COMMIT],
    n: usize,
    spill: Vec<Oid>,
    time: i64,
}

impl QuickSink {
    fn parents(&self) -> &[Oid] {
        if self.n <= PARENTS_PER_COMMIT {
            &self.inline[..self.n]
        } else {
            &self.spill
        }
    }
}

impl<'a> CommitSink<'a> for QuickSink {
    fn tree(&mut self, _id: Oid) {}
    #[inline]
    fn parent(&mut self, id: Oid) -> Result<()> {
        if self.n < PARENTS_PER_COMMIT {
            self.inline[self.n] = id;
        } else {
            if self.n == PARENTS_PER_COMMIT {
                self.spill.try_reserve(4).map_err(|_| errors::oom())?;
                self.spill.extend_from_slice(&self.inline);
            }
            self.spill.try_reserve(1).map_err(|_| errors::oom())?;
            self.spill.push(id);
        }
        self.n += 1;
        Ok(())
    }
    fn author(&mut self, _sig: &SigFields<'a>) -> Result<()> {
        Ok(())
    }
    #[inline]
    fn committer(&mut self, sig: &SigFields<'a>) -> Result<()> {
        self.time = sig.when.time;
        Ok(())
    }
    fn encoding(&mut self, _value: &'a [u8]) -> Result<()> {
        Ok(())
    }
    fn header_and_message(&mut self, _header: &'a [u8], _message: Option<&'a [u8]>) -> Result<()> {
        Ok(())
    }
}

/// `commit_quick_parse` (with `git_commit__parse_ext`'s grafts): the
/// parents and committer time of the commit in `data`, as the walk needs
/// them, without allocating a `git_commit`.
///
/// # Safety
///
/// `walk` is live and unused by anything else during the call (except the
/// C it calls back); `node` is one of its nodes.
unsafe fn quick_parse(
    walk: *mut git_revwalk,
    node: *mut GitCommitListNode,
    data: &[u8],
    id: &Oid,
) -> Result<()> {
    // SAFETY: a live walk's repository (contract).
    let repo: *mut pend::git_repository = unsafe { (*walk).prefix.repo };
    // SAFETY: a live repository.
    let oid_type: git_oid_t = unsafe { (*repo).oid_type_raw() };
    let mut sink = QuickSink::default();
    // As C (`if (git_commit__parse_ext(...) < 0) return -1;`): every parse
    // error is -1, the message kept.
    commit::parse(data, oid_type, true, &mut sink).map_err(|_| ErrorCode::GENERIC)?;

    // Perform necessary grafts
    // SAFETY: a live repository, whose grafts nothing changes meanwhile.
    let graft = unsafe { commit::find_graft(repo, id) };
    let parents: &[Oid] = match graft {
        // SAFETY: the graft is the repository's, live and unchanged during
        // the call.
        Some(g) => unsafe { (*g).parents.as_slice() },
        None => sink.parents(),
    };

    let Ok(n) = u16::try_from(parents.len()) else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "commit has more than 2^16 parents"
        ));
    };
    {
        // SAFETY: the walk's live node; nothing else reaches it while this
        // borrow lives (no C is called).
        let nd = unsafe { &mut *node };
        nd.generation = 0;
        nd.time = sink.time;
        nd.out_degree = n;
    }
    // SAFETY: the walk and the node, per the contract.
    let pp = unsafe { alloc_parents(walk, node, usize::from(n)) };
    if pp.is_null() {
        return Err(errors::oom());
    }
    // SAFETY: as above.
    unsafe { (*node).parents = pp };
    for (i, p) in parents.iter().enumerate() {
        // SAFETY: the walk is live; `p` readable. The lookup may allocate
        // from the pool: no borrow of it is held here.
        let parent = unsafe { crate::revwalk::ffi::git_revwalk__commit_lookup(walk, p) };
        // SAFETY: `i < n`: within the parents' room.
        let slot = unsafe { pp.add(i) };
        // SAFETY: a writable slot of that room.
        unsafe { slot.write(parent) };
    }
    // SAFETY: as above.
    unsafe { (*node).bits |= BIT_PARSED };
    Ok(())
}

/// `git_commit_list_parse`.
///
/// # Safety
///
/// As [`quick_parse`].
unsafe fn parse(walk: *mut git_revwalk, node: *mut GitCommitListNode) -> Result<()> {
    // SAFETY: a live node (contract).
    if unsafe { (*node).bits } & BIT_PARSED != 0 {
        return Ok(());
    }
    // SAFETY: a live walk's `repo` field.
    let repo: *mut pend::git_repository = unsafe { (*walk).prefix.repo };
    // SAFETY: a live walk's `odb` field.
    let odb: *mut pend::git_odb = unsafe { (*walk).prefix.odb };
    // SAFETY: as above.
    let id = unsafe { (*node).oid };

    // Let's try to use the commit graph first.
    let mut file: *mut pend::git_commit_graph_file = ptr::null_mut();
    // SAFETY: the walk's odb; `file` writable. (Its status is ignored, as
    // the C's.)
    let _ = unsafe { crate::odb::ffi::git_odb__get_commit_graph_file((&raw mut file).cast(), odb) };
    if !file.is_null() {
        // SAFETY: a live repository.
        let oid_type: git_oid_t = unsafe { (*repo).oid_type_raw() };
        // SAFETY: the odb's commit-graph file, live with the odb.
        let e = unsafe { commit_graph::entry_find(file, &id, crate::oid::oid_size(oid_type)) };
        if let Ok(e) = e
            && let Ok(n) = u16::try_from(e.parent_count)
        {
            {
                // SAFETY: the walk's live node; nothing else reaches it
                // while this borrow lives (no C is called).
                let nd = unsafe { &mut *node };
                nd.generation = e.generation as u32;
                nd.time = e.commit_time;
                nd.out_degree = n;
            }
            // SAFETY: the walk and its node.
            let pp = unsafe { alloc_parents(walk, node, usize::from(n)) };
            if pp.is_null() {
                return Err(errors::oom());
            }
            // SAFETY: as above.
            unsafe { (*node).parents = pp };
            for i in 0..usize::from(n) {
                // SAFETY: the file is live; `e` came from it.
                let parent =
                    unsafe { commit_graph::entry_parent(file, &e, i) }.map_err(commit::ec)?;
                // SAFETY: the walk is live; no borrow of the pool held.
                let p = unsafe {
                    crate::revwalk::ffi::git_revwalk__commit_lookup(walk, &raw const parent.sha1)
                };
                // SAFETY: `i < n`: within the parents' room.
                let slot = unsafe { pp.add(i) };
                // SAFETY: a writable slot of that room.
                unsafe { slot.write(p) };
            }
            // SAFETY: as above.
            unsafe { (*node).bits |= BIT_PARSED };
            return Ok(());
        }
    }

    let mut obj = ptr::null_mut();
    // SAFETY: the walk's odb; `obj` writable.
    ErrorCode::check_zero(unsafe {
        crate::odb::ffi::git_odb_read(&raw mut obj, odb, &raw const id)
    })?;
    // SAFETY: the object just read, live until freed below.
    let r = if unsafe { crate::odb::ffi::git_odb_object_type(obj) } != commit::OBJ_COMMIT {
        Err(git_error_set!(
            ErrorClass::INVALID,
            "object is no commit object"
        ))
    } else {
        // SAFETY: as above.
        let p = unsafe { crate::odb::ffi::git_odb_object_data(obj) }.cast::<core::ffi::c_char>();
        // SAFETY: as above.
        let n = unsafe { crate::odb::ffi::git_odb_object_size(obj) };
        let data = if p.is_null() {
            &[][..]
        } else {
            // SAFETY: the object's data is readable for its size and
            // unchanged until it is freed below.
            unsafe { crate::util::export::bytes(p, n) }
        };
        // SAFETY: the object is live.
        let id = unsafe { crate::odb::ffi::git_odb_object_id(obj) };
        // SAFETY: its id, live with it.
        let id = unsafe { &*id };
        // SAFETY: the caller's contract on the walk and node.
        unsafe { quick_parse(walk, node, data, id) }
    };
    // SAFETY: freed once.
    unsafe { crate::odb::ffi::git_odb_object_free(obj) };
    r
}

pub mod ffi {
    //! The exports of `commit_list.c`.

    use super::*;
    use crate::util::export::barrier;

    /// `git_commit_list_generation_cmp`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are live nodes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_generation_cmp(
        a: *const c_void,
        b: *const c_void,
    ) -> c_int {
        let (a, b) = (a.cast::<GitCommitListNode>(), b.cast::<GitCommitListNode>());
        // SAFETY: a live node (contract), read only during the call.
        let a = unsafe { &*a };
        // SAFETY: as above.
        let b = unsafe { &*b };
        generation_cmp((a.generation, a.time), (b.generation, b.time))
    }

    /// `git_commit_list_time_cmp`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are live nodes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_time_cmp(a: *const c_void, b: *const c_void) -> c_int {
        let (a, b) = (a.cast::<GitCommitListNode>(), b.cast::<GitCommitListNode>());
        // SAFETY: a live node (contract), read only during the call.
        let a = unsafe { &*a };
        // SAFETY: as above.
        let b = unsafe { &*b };
        time_cmp(a.time, b.time)
    }

    /// `git_commit_list_create`: a new list cell (NULL on allocation
    /// failure).
    #[unsafe(no_mangle)]
    pub extern "C" fn git_commit_list_create(
        item: *mut GitCommitListNode,
        next: *mut GitCommitList,
    ) -> *mut GitCommitList {
        barrier(|| {
            let p = alloc::malloc(size_of::<GitCommitList>()).cast::<GitCommitList>();
            if !p.is_null() {
                // SAFETY: a fresh allocation of a cell.
                unsafe { p.write(GitCommitList { item, next }) };
            }
            p
        })
    }

    /// `git_commit_list_insert`: push `item` on `*list_p`.
    ///
    /// # Safety
    ///
    /// `list_p` is a writable list head (NULL-terminated list).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_insert(
        item: *mut GitCommitListNode,
        list_p: *mut *mut GitCommitList,
    ) -> *mut GitCommitList {
        // SAFETY: a valid, readable head (contract).
        let new_list = git_commit_list_create(item, unsafe { list_p.read() });
        // SAFETY: a writable head (contract).
        unsafe { list_p.write(new_list) };
        new_list
    }

    /// `git_commit_list_insert_by_date`: insert before the first cell that
    /// is not newer.
    ///
    /// # Safety
    ///
    /// `list_p` is a writable list head whose cells hold live nodes; `item`
    /// is a live node.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_insert_by_date(
        item: *mut GitCommitListNode,
        list_p: *mut *mut GitCommitList,
    ) -> *mut GitCommitList {
        let mut pp = list_p;
        loop {
            // SAFETY: `pp` is the head or a live cell's `next` (contract).
            let p = unsafe { pp.read() };
            if p.is_null() {
                break;
            }
            // SAFETY: a live cell of the list.
            let cell_item = unsafe { (*p).item };
            // SAFETY: live nodes (contract).
            if unsafe { git_commit_list_time_cmp(cell_item.cast(), item.cast()) } > 0 {
                break;
            }
            // SAFETY: a live cell of the list.
            pp = unsafe { &raw mut (*p).next };
        }
        // SAFETY: `pp` is a writable link of the list.
        unsafe { git_commit_list_insert(item, pp) }
    }

    /// `git_commit_list_alloc_node`: a zeroed node from the walk's pool
    /// (NULL on allocation failure).
    ///
    /// # Safety
    ///
    /// `walk` is a live walk that nothing else uses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_alloc_node(
        walk: *mut git_revwalk,
    ) -> *mut GitCommitListNode {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe {
                with_pool(walk, |p| {
                    p.mallocz(1).map_or(ptr::null_mut(), |p| p.as_ptr().cast())
                })
            }
        })
    }

    /// `git_commit_list_free`.
    ///
    /// # Safety
    ///
    /// `list_p` is a writable list head; its cells are `git__malloc`ed and
    /// not used after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_free(list_p: *mut *mut GitCommitList) {
        // SAFETY: a readable head (contract).
        let mut list = unsafe { list_p.read() };
        if list.is_null() {
            return;
        }
        while !list.is_null() {
            let temp = list;
            // SAFETY: a live cell of the list.
            list = unsafe { (*temp).next };
            // SAFETY: a `git__malloc`ed cell, not used after (contract).
            unsafe { alloc::free(temp.cast()) };
        }
        // SAFETY: a writable head (contract).
        unsafe { list_p.write(ptr::null_mut()) };
    }

    /// `git_commit_list_pop`.
    ///
    /// # Safety
    ///
    /// As [`git_commit_list_free`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_pop(
        stack: *mut *mut GitCommitList,
    ) -> *mut GitCommitListNode {
        // SAFETY: a readable head (contract).
        let top = unsafe { stack.read() };
        if top.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: the live first cell, read before it is freed.
        let GitCommitList { item, next } = unsafe { top.read() };
        // SAFETY: a writable head (contract).
        unsafe { stack.write(next) };
        // SAFETY: a `git__malloc`ed cell, not used after (contract).
        unsafe { alloc::free(top.cast()) };
        item
    }

    /// `git_commit_list_parse`: fill `commit`'s parents and time, from the
    /// commit-graph if it has the commit, from the odb otherwise.
    ///
    /// # Safety
    ///
    /// `walk` is a live walk, used by nothing else during the call; `commit`
    /// one of its nodes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_parse(
        walk: *mut git_revwalk,
        commit: *mut GitCommitListNode,
    ) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| commit::code(unsafe { parse(walk, commit) }))
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)] // each block's SAFETY covers its sequence
mod tests {
    use super::*;

    #[test]
    fn comparisons() {
        assert_eq!(time_cmp(1, 2), 1);
        assert_eq!(time_cmp(2, 1), -1);
        assert_eq!(time_cmp(2, 2), 0);
        assert_eq!(generation_cmp((1, 5), (2, 1)), 1);
        assert_eq!(generation_cmp((3, 5), (2, 1)), -1);
        assert_eq!(generation_cmp((0, 5), (2, 1)), -1);
        assert_eq!(generation_cmp((2, 5), (2, 1)), 0);
    }

    #[test]
    fn node_bits() {
        let mut n = GitCommitListNode {
            oid: Oid::default(),
            time: 0,
            generation: 0,
            bits: BIT_SEEN | BIT_PARSED,
            in_degree: 0,
            out_degree: 0,
            parents: ptr::null_mut(),
        };
        assert!(n.parsed());
        n.set_flags(0x1f);
        assert_eq!(n.flags(), 0xf);
        assert_eq!(n.bits & 0x1f, BIT_SEEN | BIT_PARSED);
        assert_eq!(COMMIT_ALLOC, 88);
    }

    #[test]
    fn lists() {
        let _g = crate::util::test_support::init();
        let mut nodes: Vec<GitCommitListNode> = [5i64, 3, 9, 3]
            .iter()
            .map(|&t| GitCommitListNode {
                oid: Oid::default(),
                time: t,
                generation: 0,
                bits: 0,
                in_degree: 0,
                out_degree: 0,
                parents: ptr::null_mut(),
            })
            .collect();
        let base = nodes.as_mut_ptr();
        let mut list: *mut GitCommitList = ptr::null_mut();
        // SAFETY: the nodes outlive the list; the list head is ours.
        unsafe {
            for i in 0..4 {
                ffi::git_commit_list_insert_by_date(base.add(i), &raw mut list);
            }
            let mut times = vec![];
            while !list.is_null() {
                times.push((*ffi::git_commit_list_pop(&raw mut list)).time);
            }
            assert_eq!(times, [9, 5, 3, 3]);
            ffi::git_commit_list_insert(base, &raw mut list);
            ffi::git_commit_list_insert(base.add(1), &raw mut list);
            assert_eq!((*(*list).item).time, 3);
            ffi::git_commit_list_free(&raw mut list);
            assert!(list.is_null());
            assert!(ffi::git_commit_list_pop(&raw mut list).is_null());
        }
    }
}

/// Miri tests of the quick parse over a walk of the revwalk module and a
/// zeroed repository with empty grafts.
#[cfg(all(test, miri))]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)] // each block's SAFETY covers its sequence
mod miri_quick {
    use super::*;
    use crate::oid::OidType;

    /// A walk (the revwalk's own, without an odb) over a zeroed repository
    /// whose grafts (and shallow grafts) are one empty `Grafts`. The
    /// repository and the grafts are raw allocations, so that the walk's pointer to
    /// the repository stays valid (a `Box` that moves would retag it).
    struct Fake {
        walk: Box<git_revwalk>,
        repo: *mut crate::repository::GitRepository,
        grafts: *mut crate::grafts::Grafts,
    }

    impl Fake {
        fn new() -> Fake {
            // SAFETY: all-zero is a valid `GitRepository` (atomics, NULLs,
            // an empty cache).
            let repo: Box<crate::repository::GitRepository> =
                unsafe { Box::new_zeroed().assume_init() };
            let repo = Box::into_raw(repo);
            let grafts = Box::into_raw(Box::new(crate::grafts::Grafts::new(OidType::Sha1)));
            // SAFETY: the fresh repository, ours alone; its own slots.
            unsafe {
                (*repo).set_oid_type_raw(OidType::Sha1.raw());
                (*repo).grafts_slot().write(grafts);
                (*repo).shallow_grafts_slot().write(grafts);
            }
            let walk = Box::new(git_revwalk::with_repo(repo).unwrap());
            Fake { walk, repo, grafts }
        }
        fn walk(&mut self) -> *mut git_revwalk {
            &raw mut *self.walk
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            // SAFETY: the allocations of `new`, freed once (the walk, which
            // points to the repository, is dropped after this body, but
            // does not use it when dropped).
            unsafe {
                drop(Box::from_raw(self.repo));
                drop(Box::from_raw(self.grafts));
            }
        }
    }

    const T: &str = "1810dff58d8a660512d4832e740f692884338ccd";

    fn commit(parents: usize, committer: &str) -> Vec<u8> {
        let mut s = format!("tree {T}\n");
        for i in 0..parents {
            s.push_str(&format!("parent {:040x}\n", i + 1));
        }
        s.push_str(&format!("author A <a@b> 1 +0000\n{committer}\n\nmsg\n"));
        s.into_bytes()
    }

    #[test]
    fn quick_parse_nodes() {
        let _g = crate::util::test_support::init();
        for np in [0usize, 1, 2, 3, 7] {
            let mut f = Fake::new();
            let walk = f.walk();
            // SAFETY: the fake walk; the node is allocated from its pool.
            unsafe {
                let node = ffi::git_commit_list_alloc_node(walk);
                let data = commit(np, "committer C <c@d> 1234 +0100");
                quick_parse(walk, node, &data, &Oid::default()).unwrap();
                assert!((*node).parsed());
                assert_eq!((*node).time, 1234);
                assert_eq!(usize::from((*node).out_degree), np);
                let ps = (*node).parent_slice();
                assert_eq!(ps.len(), np);
                for (i, &n) in ps.iter().enumerate() {
                    // The walk's own node for each parent.
                    assert_eq!((*n).oid.hex().as_str(), format!("{:040x}", i + 1));
                }
                // Up to two parents live inline after the node.
                if np <= PARENTS_PER_COMMIT {
                    assert_eq!((*node).parents, node.add(1).cast());
                }
            }
        }
    }

    #[test]
    fn quick_parse_errors_are_minus_one() {
        let _g = crate::util::test_support::init();
        let mut f = Fake::new();
        let walk = f.walk();
        for (data, msg) in [
            (
                commit(1, "committer C c@d> 1 +0000"),
                "failed to parse signature - malformed e-mail",
            ),
            (
                commit(0, "x"),
                "failed to parse signature - expected prefix doesn't match actual",
            ),
            (b"tree 12\n".to_vec(), "failed to parse bad commit object"),
        ] {
            // SAFETY: the fake walk.
            unsafe {
                let node = ffi::git_commit_list_alloc_node(walk);
                assert_eq!(
                    quick_parse(walk, node, &data, &Oid::default()).unwrap_err(),
                    ErrorCode::GENERIC
                );
                assert_eq!(errors::last_message().unwrap(), msg.as_bytes());
                assert!(!(*node).parsed());
            }
        }
    }
}

crate::c_layout_test! {
    headers: ["commit_list.h"],
    types: [
        GitCommitListNode => "git_commit_list_node" { oid, time, generation, in_degree, out_degree, parents },
        GitCommitList => "git_commit_list" { item, next },
    ],
    asserts: [
        "COMMIT_ALLOC == 88",
        "PARENT1 == 1", "PARENT2 == 2", "RESULT == 4", "STALE == 8", "FLAG_BITS == 4",
    ],
}
