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

use crate::commit::{self, CommitSink, SigFields};
use crate::git_error_set;
use crate::oid::{Oid, git_oid_t};
use crate::sys::commit_graph;
use crate::sys::pending::l_commit::{self as pend, git_revwalk};
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
    // SAFETY: the pool is a `git_pool` by value at that offset of the walk;
    // nothing else touches it during `f` (contract), and `f` calls no C.
    let pool = unsafe { &mut *pend::field_ptr::<GitPool, _>(walk, pend::REVWALK_COMMIT_POOL) };
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
    let repo: *mut pend::git_repository = unsafe { pend::field(walk, pend::REVWALK_REPO) };
    // SAFETY: a live repository.
    let oid_type: git_oid_t = unsafe { pend::field(repo, pend::REPO_OID_TYPE) };
    let mut sink = QuickSink::default();
    commit::parse(data, oid_type, true, &mut sink)?;

    // Perform necessary grafts
    // SAFETY: a live repository, whose grafts nothing changes meanwhile.
    let graft = unsafe { commit::find_graft(repo, id) };
    // SAFETY: the graft is the repository's, live during the call.
    let parents: &[Oid] = match graft {
        Some(g) => unsafe { (*g).parents.as_slice() },
        None => sink.parents(),
    };

    let Ok(n) = u16::try_from(parents.len()) else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "commit has more than 2^16 parents"
        ));
    };
    // SAFETY: the node is the walk's; written through the pointer.
    unsafe {
        (*node).generation = 0;
        (*node).time = sink.time;
        (*node).out_degree = n;
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
        let parent = unsafe { pend::git_revwalk__commit_lookup(walk, p) };
        // SAFETY: `i < n` slots of the parents' room.
        unsafe { pp.add(i).write(parent) };
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
    // SAFETY: a live walk.
    let (repo, odb): (*mut pend::git_repository, *mut pend::git_odb) = unsafe {
        (
            pend::field(walk, pend::REVWALK_REPO),
            pend::field(walk, pend::REVWALK_ODB),
        )
    };
    // SAFETY: as above.
    let id = unsafe { (*node).oid };

    // Let's try to use the commit graph first.
    let mut file = ptr::null_mut();
    // SAFETY: the walk's odb; `file` writable. (Its status is ignored, as
    // the C's.)
    let _ = unsafe { pend::git_odb__get_commit_graph_file(&mut file, odb) };
    if !file.is_null() {
        // SAFETY: a live repository.
        let oid_type: git_oid_t = unsafe { pend::field(repo, pend::REPO_OID_TYPE) };
        // SAFETY: the odb's commit-graph file, live with the odb.
        let e = unsafe { commit_graph::entry_find(file, &id, crate::oid::oid_size(oid_type)) };
        if let Ok(e) = e {
            if let Ok(n) = u16::try_from(e.parent_count) {
                // SAFETY: the walk's node, written through the pointer.
                unsafe {
                    (*node).generation = e.generation as u32;
                    (*node).time = e.commit_time;
                    (*node).out_degree = n;
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
                    let p = unsafe { pend::git_revwalk__commit_lookup(walk, &parent.sha1) };
                    // SAFETY: `i < n` slots.
                    unsafe { pp.add(i).write(p) };
                }
                // SAFETY: as above.
                unsafe { (*node).bits |= BIT_PARSED };
                return Ok(());
            }
        }
    }

    let mut obj = ptr::null_mut();
    // SAFETY: the walk's odb; `obj` writable.
    ErrorCode::check_zero(unsafe { pend::git_odb_read(&mut obj, odb, &id) })?;
    // SAFETY: the object just read, live until freed below: its data is
    // readable for its size and unchanged.
    let r = unsafe {
        if pend::git_odb_object_type(obj) != commit::OBJ_COMMIT {
            Err(git_error_set!(
                ErrorClass::INVALID,
                "object is no commit object"
            ))
        } else {
            let p = pend::git_odb_object_data(obj).cast::<core::ffi::c_char>();
            let n = pend::git_odb_object_size(obj);
            let data = if p.is_null() {
                &[][..]
            } else {
                crate::util::export::bytes(p, n)
            };
            quick_parse(walk, node, data, &*pend::git_odb_object_id(obj))
        }
    };
    // SAFETY: freed once.
    unsafe { pend::git_odb_object_free(obj) };
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
        // SAFETY: live nodes (contract), read through the pointers.
        unsafe { generation_cmp(((*a).generation, (*a).time), ((*b).generation, (*b).time)) }
    }

    /// `git_commit_list_time_cmp`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are live nodes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_time_cmp(a: *const c_void, b: *const c_void) -> c_int {
        let (a, b) = (a.cast::<GitCommitListNode>(), b.cast::<GitCommitListNode>());
        // SAFETY: live nodes (contract), read through the pointers.
        unsafe { time_cmp((*a).time, (*b).time) }
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
        // SAFETY: a valid head (contract).
        unsafe {
            let new_list = git_commit_list_create(item, list_p.read());
            list_p.write(new_list);
            new_list
        }
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
        // SAFETY: a valid list of live nodes (contract), walked through the
        // raw pointers.
        unsafe {
            loop {
                let p = pp.read();
                if p.is_null() || git_commit_list_time_cmp((*p).item.cast(), item.cast()) > 0 {
                    break;
                }
                pp = ptr::addr_of_mut!((*p).next);
            }
            git_commit_list_insert(item, pp)
        }
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
        // SAFETY: the caller's contract.
        unsafe {
            let mut list = list_p.read();
            if list.is_null() {
                return;
            }
            while !list.is_null() {
                let temp = list;
                list = (*temp).next;
                alloc::free(temp.cast());
            }
            list_p.write(ptr::null_mut());
        }
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
        // SAFETY: the caller's contract.
        unsafe {
            let top = stack.read();
            if top.is_null() {
                return ptr::null_mut();
            }
            let item = (*top).item;
            stack.write((*top).next);
            alloc::free(top.cast());
            item
        }
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
                ffi::git_commit_list_insert_by_date(base.add(i), &mut list);
            }
            let mut times = vec![];
            while !list.is_null() {
                times.push((*ffi::git_commit_list_pop(&mut list)).time);
            }
            assert_eq!(times, [9, 5, 3, 3]);
            ffi::git_commit_list_insert(base, &mut list);
            ffi::git_commit_list_insert(base.add(1), &mut list);
            assert_eq!((*(*list).item).time, 3);
            ffi::git_commit_list_free(&mut list);
            assert!(list.is_null());
            assert!(ffi::git_commit_list_pop(&mut list).is_null());
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
