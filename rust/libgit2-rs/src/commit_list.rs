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

use crate::util::cabi::Out;
use core::ffi::{c_int, c_void};
use core::ptr;

use ffibox::{CBitfield, CBits};

use crate::commit::deps::{self as pend, git_revwalk};
use crate::commit::{self, CommitSink, SigFields};
use crate::git_error_set;
use crate::oid::{Oid, git_oid_t};
use crate::revwalk::NodePtr;
use crate::sys::commit_graph;
use crate::util::alloc;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};

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
    pub bits: CBitfield<u16>,
    pub in_degree: u16,
    pub out_degree: u16,
    pub parents: *mut *mut GitCommitListNode,
}

/// `unsigned seen:1`.
pub const SEEN: CBits<u16> = CBits::first(1);
/// `unsigned uninteresting:1`.
pub const UNINTERESTING: CBits<u16> = SEEN.next(1);
/// `unsigned topo_delay:1`.
pub const TOPO_DELAY: CBits<u16> = UNINTERESTING.next(1);
/// `unsigned parsed:1`.
pub const PARSED: CBits<u16> = TOPO_DELAY.next(1);
/// `unsigned added:1`.
pub const ADDED: CBits<u16> = PARSED.next(1);
/// `unsigned flags : FLAG_BITS` (4).
pub const FLAGS: CBits<u16> = ADDED.next(4);

impl GitCommitListNode {
    /// `parsed`.
    pub fn parsed(&self) -> bool {
        self.bits.is_set(PARSED)
    }
    /// `flags` (4 bits).
    pub fn flags(&self) -> u32 {
        u32::from(self.bits.get(FLAGS))
    }
    /// Set `flags` (the low 4 bits of `f`, as C's assignment truncates).
    pub fn set_flags(&mut self, f: u32) {
        self.bits.set(FLAGS, (f & 0xf) as u16);
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

/// `alloc_parents`: room for `n` parent pointers: inline after the node for
/// up to two, from the walk's pool otherwise (NULL if out of memory).
fn alloc_parents(walk: &git_revwalk, node: NodePtr, n: usize) -> *mut *mut GitCommitListNode {
    if n <= PARENTS_PER_COMMIT {
        // The node's item holds two pointers after it.
        return node.tail_slots();
    }
    let Some(bytes) = n.checked_mul(size_of::<*mut GitCommitListNode>()) else {
        return ptr::null_mut();
    };
    // As C: `git_pool_malloc(pool, bytes)`, `bytes` items of the pool's size.
    walk.pool_alloc(bytes, false)
        .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
}

/// The parse's last step: `node`'s generation, time and `n` parents
/// (`parent(i)` for each), then `parsed`. Out of memory for the parents'
/// room leaves the node with `out_degree = n` and no parents, as C.
fn fill_node(
    walk: &git_revwalk,
    node: NodePtr,
    generation: u32,
    time: i64,
    n: u16,
    parent: impl FnMut(usize) -> Result<*mut GitCommitListNode>,
) -> Result<()> {
    node.set_generation(generation);
    node.set_time(time);
    let arr = alloc_parents(walk, node, usize::from(n));
    if arr.is_null() {
        node.set_no_parents(n);
        return Err(errors::oom());
    }
    // SAFETY: `arr` is the node's own tail (`n <= 2`: an item of
    // COMMIT_ALLOC bytes, 8-aligned, holds a node and two pointers) or a
    // fresh pool block of `n` pointers, both live as long as the pool and
    // used by nothing else; `parent` gives nodes of this walk (or NULL)
    // from `git_revwalk__commit_lookup`, which allocates new nodes but does
    // not touch this one.
    unsafe { node.set_parents(arr, n, parent) }?;
    node.set_parsed_flag();
    Ok(())
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
fn quick_parse(walk: &git_revwalk, node: NodePtr, data: &[u8], id: &Oid) -> Result<()> {
    // (C dereferences a NULL repository.)
    let repo = walk
        .repository()
        .ok_or_else(|| errors::assert_failed("walk->repo"))?;
    let oid_type: git_oid_t = repo.oid_type_raw();
    let mut sink = QuickSink::default();
    // As C (`if (git_commit__parse_ext(...) < 0) return -1;`): every parse
    // error is -1, the message kept.
    commit::parse(data, oid_type, true, &mut sink).map_err(|_| ErrorCode::GENERIC)?;

    // Perform necessary grafts
    // SAFETY: nothing changes the repository's grafts during the parse (C's
    // rule: they are refreshed only by the repository's own calls), and the
    // graft is used only within this call.
    let graft = unsafe { commit::find_graft(Some(repo), id) };
    let parents: &[Oid] = match graft {
        // SAFETY: a live graft of the unchanged grafts (above), read for
        // this call only.
        Some(g) => unsafe { (*g).parents.as_slice() },
        None => sink.parents(),
    };

    let Ok(n) = u16::try_from(parents.len()) else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "commit has more than 2^16 parents"
        ));
    };
    fill_node(walk, node, 0, sink.time, n, |i| {
        Ok(walk
            .commit_lookup(&parents[i])
            .map_or(ptr::null_mut(), NodePtr::as_ptr))
    })
}

/// `git_commit_list_parse`.
pub(crate) fn parse(walk: &git_revwalk, node: NodePtr) -> Result<()> {
    if node.parsed() {
        return Ok(());
    }
    let odb: *mut pend::git_odb = walk.odb_ptr();
    // SAFETY: the walk's odb is NULL or live while the walk is, only read
    // through `&` (the odb module's docs).
    let odb_ref = unsafe { odb.as_ref() };
    let id = node.oid();

    // Let's try to use the commit graph first.
    let mut file: *mut c_void = ptr::null_mut();
    // SAFETY: the walk's odb (NULL or its live reference); `file` a local.
    // (Its status is ignored, as the C's.)
    let _ =
        unsafe { crate::odb::ffi::git_odb__get_commit_graph_file(Out::from(&mut file), odb_ref) };
    let file = file.cast::<pend::git_commit_graph_file>();
    if !file.is_null() {
        // (C dereferences a NULL repository.)
        let repo = walk
            .repository()
            .ok_or_else(|| errors::assert_failed("walk->repo"))?;
        let oid_type: git_oid_t = repo.oid_type_raw();
        // SAFETY: the odb's commit-graph file, live with the odb (which
        // the walk holds) for the call.
        let e = unsafe { commit_graph::entry_find(file, &id, crate::oid::oid_size(oid_type)) };
        if let Ok(e) = e
            && let Ok(n) = u16::try_from(e.parent_count)
        {
            return fill_node(walk, node, e.generation as u32, e.commit_time, n, |i| {
                // SAFETY: the file is live (as above); `e` came from it.
                let parent =
                    unsafe { commit_graph::entry_parent(file, &e, i) }.map_err(commit::ec)?;
                Ok(walk
                    .commit_lookup(&parent.sha1)
                    .map_or(ptr::null_mut(), NodePtr::as_ptr))
            });
        }
    }

    let mut obj = ptr::null_mut();
    // SAFETY: the walk's odb (NULL, which C rejects, or live); `obj` a
    // writable local; `id` readable.
    ErrorCode::check_zero(unsafe {
        crate::odb::ffi::git_odb_read(Out::from(&mut obj), odb_ref, Some(&id))
    })?;
    // SAFETY: the object just read, live until freed below, not written
    // once shared but for its atomic count.
    let obj_ref = unsafe { obj.as_ref() };
    // SAFETY: the object just read, live until freed below.
    let r = if unsafe { crate::odb::ffi::git_odb_object_type(obj_ref) } != commit::OBJ_COMMIT {
        Err(git_error_set!(
            ErrorClass::INVALID,
            "object is no commit object"
        ))
    } else {
        // SAFETY: as above.
        let p =
            unsafe { crate::odb::ffi::git_odb_object_data(obj_ref) }.cast::<core::ffi::c_char>();
        // SAFETY: as above.
        let n = unsafe { crate::odb::ffi::git_odb_object_size(obj_ref) };
        let data = if p.is_null() {
            &[][..]
        } else {
            // SAFETY: the object's data is readable for its size and
            // unchanged until it is freed below.
            unsafe { crate::util::export::bytes(p, n) }
        };
        // SAFETY: the object is live.
        let id = unsafe { crate::odb::ffi::git_odb_object_id(obj_ref) };
        // SAFETY: its id, live with it.
        let id = unsafe { &*id };
        quick_parse(walk, node, data, id)
    };
    // SAFETY: the object read above, freed once, not used after.
    unsafe { crate::odb::ffi::git_odb_object_free(obj) };
    r
}

pub mod ffi {
    //! The exports of `commit_list.c`.

    use super::*;
    use crate::git_arg;
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
        // SAFETY: `a` is a live, aligned node (the contract), not written
        // during the call: its fields are read in place, without a
        // reference (the walk reaches nodes through raw pointers only).
        let ga = unsafe { (*a).generation };
        // SAFETY: as above.
        let ta = unsafe { (*a).time };
        // SAFETY: as for `a`.
        let gb = unsafe { (*b).generation };
        // SAFETY: as for `a`.
        let tb = unsafe { (*b).time };
        generation_cmp((ga, ta), (gb, tb))
    }

    /// `git_commit_list_time_cmp`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are live nodes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_time_cmp(a: *const c_void, b: *const c_void) -> c_int {
        let (a, b) = (a.cast::<GitCommitListNode>(), b.cast::<GitCommitListNode>());
        // SAFETY: as in `git_commit_list_generation_cmp`.
        let ta = unsafe { (*a).time };
        // SAFETY: as for `a`.
        let tb = unsafe { (*b).time };
        time_cmp(ta, tb)
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
    /// `walk` is NULL (C dereferences it: here an invalid argument) or a
    /// live walk, not used by another thread during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_alloc_node(
        walk: Option<&git_revwalk>,
    ) -> *mut GitCommitListNode {
        barrier(|| {
            let walk = git_arg!(walk, "walk", ptr::null_mut());
            walk.pool_alloc(1, true)
                .map_or(ptr::null_mut(), |p| p.as_ptr().cast())
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
    /// `walk` is NULL or a live walk, not used by another thread during the
    /// call; `commit` NULL or one of its nodes, which no Rust reference
    /// covers (C NULLs: here invalid arguments, C dereferences them).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_list_parse(
        walk: Option<&git_revwalk>,
        commit: *mut GitCommitListNode,
    ) -> c_int {
        barrier(|| {
            let walk = git_arg!(walk, "walk", -1);
            // SAFETY: NULL or a live node of the walk's pool, which lives
            // for the call, used from the walk's thread and covered by no
            // reference (this export's contract): `NodePtr`'s invariant.
            let node = git_arg!(unsafe { NodePtr::from_raw(commit) }, "commit", -1);
            commit::code(parse(walk, node))
        })
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
            bits: CBitfield::new(SEEN.mask() | PARSED.mask()),
            in_degree: 0,
            out_degree: 0,
            parents: ptr::null_mut(),
        };
        assert!(n.parsed());
        n.set_flags(0x1f);
        assert_eq!(n.flags(), 0xf);
        assert_eq!(n.bits.into_inner() & 0x1f, SEEN.mask() | PARSED.mask());
        assert_eq!(n.bits.into_inner() >> 5, 0xf);
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
                bits: CBitfield::new(0),
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

/// Tests of the quick parse over a walk of the revwalk module and a zeroed
/// repository with empty grafts (no C: Miri runs them too).
#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)] // each block's SAFETY covers its sequence
mod miri_quick {
    use super::*;
    use crate::oid::OidType;

    /// A walk (the revwalk's own, without an odb) over a zeroed repository
    /// whose grafts and shallow grafts are empty. The repository is a raw
    /// allocation, so that the walk's pointer to it stays valid (a `Box`
    /// that moves would retag it).
    struct Fake {
        walk: Box<git_revwalk>,
        repo: *mut crate::repository::GitRepository,
    }

    /// Empty grafts (`git_grafts_new`).
    fn empty_grafts() -> crate::repository::GraftsBox {
        let mut g = core::ptr::null_mut();
        // SAFETY: a local out-parameter.
        let r = unsafe {
            crate::grafts::ffi::git_grafts_new(
                crate::util::cabi::Out::from(&mut g),
                OidType::Sha1.raw(),
            )
        };
        assert_eq!(r, 0);
        // SAFETY: the grafts just made, ours.
        unsafe { crate::repository::GraftsBox::from_raw(g) }.unwrap()
    }

    impl Fake {
        fn new() -> Fake {
            // SAFETY: all-zero is a valid `GitRepository` (atomics, NULLs,
            // empty owners, an empty cache).
            let repo: Box<crate::repository::GitRepository> =
                unsafe { Box::new_zeroed().assume_init() };
            repo.set_oid_type_raw(OidType::Sha1.raw());
            repo.init_grafts(empty_grafts());
            repo.init_shallow_grafts(empty_grafts());
            let repo = Box::into_raw(repo);
            // SAFETY: `repo` lives until `Fake` drops, after the walk's
            // last use (the walk does not use it when dropped).
            let walk = Box::new(unsafe { git_revwalk::with_repo(repo) }.unwrap());
            Fake { walk, repo }
        }
        fn walk(&self) -> &git_revwalk {
            &self.walk
        }
        /// A fresh node of the walk's pool.
        fn node(&self) -> NodePtr {
            // SAFETY: a live walk, used by this thread only.
            let raw = unsafe { ffi::git_commit_list_alloc_node(Some(self.walk())) };
            // SAFETY: a fresh zeroed node of the walk's pool, which outlives
            // the test's uses of it; no reference covers it.
            unsafe { NodePtr::from_raw(raw) }.unwrap()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            // SAFETY: the allocation of `new`, freed once (its grafts with
            // it; the walk, which points to the repository, is dropped after
            // this body, but does not use it when dropped).
            unsafe { drop(Box::from_raw(self.repo)) };
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
            let f = Fake::new();
            let node = f.node();
            let data = commit(np, "committer C <c@d> 1234 +0100");
            quick_parse(f.walk(), node, &data, &Oid::default()).unwrap();
            assert!(node.parsed());
            assert_eq!(node.time(), 1234);
            assert_eq!(node.out_degree(), np);
            for i in 0..np {
                // The walk's own node for each parent.
                let n = node.parent(i).unwrap();
                assert_eq!(n.oid().hex().as_str(), format!("{:040x}", i + 1));
                assert_eq!(f.walk().commit_lookup(&n.oid()), Some(n));
            }
            assert!(node.parent(np).is_none());
            // Up to two parents live inline after the node.
            // SAFETY: a live node; its `parents` field read in place.
            let parents = unsafe { (*node.as_ptr()).parents };
            assert_eq!(parents == node.tail_slots(), np <= PARENTS_PER_COMMIT);
        }
    }

    #[test]
    fn quick_parse_errors_are_minus_one() {
        let _g = crate::util::test_support::init();
        let f = Fake::new();
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
            let node = f.node();
            assert_eq!(
                quick_parse(f.walk(), node, &data, &Oid::default()).unwrap_err(),
                ErrorCode::GENERIC
            );
            assert_eq!(errors::last_message().unwrap(), msg.as_bytes());
            assert!(!node.parsed());
        }
    }

    #[test]
    fn null_arguments_are_invalid() {
        let _g = crate::util::test_support::init();
        // SAFETY: NULL arguments (C dereferences them).
        assert!(unsafe { ffi::git_commit_list_alloc_node(None) }.is_null());
        let f = Fake::new();
        // SAFETY: a live walk; a NULL node.
        let r = unsafe { ffi::git_commit_list_parse(Some(f.walk()), ptr::null_mut()) };
        assert_eq!(r, -1);
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid argument: 'commit'"
        );
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
