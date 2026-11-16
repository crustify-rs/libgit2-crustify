//! Port of `src/libgit2/revwalk.c`: the revision walker (`git_revwalk`).
//!
//! # The walker and C
//!
//! `git_revwalk` is a shared-prefix type (kind 2, PORTING.md §4.1): the C
//! that stays reads `repo`, `odb` and `commit_pool` (`commit_list.c`,
//! which allocates the nodes and parses them), and `user_input`
//! (`pack-objects.c`), and calls [`ffi::git_revwalk__commit_lookup`]
//! (`merge.c`, `graph.c`, `describe.c`, `commit_list.c`). Those fields are
//! [`RevwalkPrefix`], at offset 0, as `revwalk.h` declares it; the rest of
//! the walk ([`State`]: the iterators, the sorting, the flags, the hide
//! callback, the pathspec) is Rust-native and follows it.
//!
//! The walk is reached through `&GitRevwalk`: the fields C writes while the
//! Rust holds that reference (`commits` and `commit_pool`, through
//! `git_commit_list_parse` → `git_revwalk__commit_lookup`) are in
//! `UnsafeCell`s, and no Rust reference to them lives across a call into C.
//! The commit nodes, which C reads and writes too, are reached through
//! [`NodePtr`] only.
//!
//! # Equivalence
//!
//! The algorithms are the C's, step for step, so that every sorting mode
//! gives the C's output order, ties included: the time queue is C's binary
//! heap (`git_pqueue`, the same sift up and down), the date-ordered lists
//! insert before the first older commit, the topological sort uses the
//! same queue (a heap by time, or a stack reversed once), and the
//! "uninteresting" propagation, `SLOP` and the first-parent and hide
//! callback rules are the C's. Only the containers differ: the walk's
//! internal lists are `VecDeque`s and `Vec`s instead of `git_commit_list`
//! cells (C never sees them); `user_input` stays a `git_commit_list`.

pub mod api;
mod glue;
pub mod node;

use core::cell::{Cell, UnsafeCell};
use core::ffi::{CStr, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use std::collections::VecDeque;

use crate::hashmap_oid::OidPtr;
use crate::oid::Oid;
use crate::sys::pathspec::git_pathspec;
use crate::sys::pending::l_revwalk::{GIT_OBJECT_ANY, GIT_OBJECT_COMMIT, git_odb, git_repository};
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::hashmap::GitHashMap;
use crate::util::pool::GitPool;
use crate::util::str::GitStr;
use crate::{git_assert_arg, git_error_set};

use glue::{Pathspec, Repo};
pub use node::{COMMIT_ALLOC, CommitList, CommitListNode, NodePtr};

/// `GIT_SORT_NONE`.
pub const GIT_SORT_NONE: c_uint = 0;
/// `GIT_SORT_TOPOLOGICAL`.
pub const GIT_SORT_TOPOLOGICAL: c_uint = 1 << 0;
/// `GIT_SORT_TIME`.
pub const GIT_SORT_TIME: c_uint = 1 << 1;
/// `GIT_SORT_REVERSE`.
pub const GIT_SORT_REVERSE: c_uint = 1 << 2;

/// How many uninteresting commits to look at after running out of
/// interesting ones (`SLOP`).
const SLOP: c_int = 5;

/// `git_revwalk_hide_cb`.
pub type HideCb =
    Option<unsafe extern "C" fn(commit_id: *const Oid, payload: *mut c_void) -> c_int>;

/// `git_revwalk_oidmap`: `GIT_HASHMAP_OID_STRUCT(git_revwalk_oidmap,
/// git_commit_list_node *)`, keyed by the nodes' own `oid`.
pub type CommitMap = GitHashMap<OidPtr, NodePtr>;

/// `git_revwalk__push_options` (mirrored: C fills it on its stack).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushOptions {
    /// `uninteresting`: hide rather than push.
    pub uninteresting: c_int,
    /// `from_glob`: skip what does not peel to a commit.
    pub from_glob: c_int,
    /// `insert_by_date`: order `user_input` by date (fetch negotiation).
    pub insert_by_date: c_int,
}

/// The part of `struct git_revwalk` that C sees (`revwalk.h`).
#[repr(C)]
#[derive(Debug)]
pub struct RevwalkPrefix {
    /// `repo`: set at creation, read by C.
    pub repo: *mut git_repository,
    /// `odb`: set at creation, read by C.
    pub odb: *mut git_odb,
    /// `commits`: every node of the walk, by id (only the Rust touches it).
    pub commits: UnsafeCell<CommitMap>,
    /// `commit_pool`: the nodes and their parent arrays; C allocates from
    /// it (`git_commit_list_alloc_node`, `alloc_parents`).
    pub commit_pool: UnsafeCell<GitPool>,
    /// `user_input`: the pushes and hides, a `git_commit_list` that
    /// `pack-objects.c` reads.
    pub user_input: Cell<*mut CommitList>,
}

/// Which `get_next` the walk uses (C's function pointer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Next {
    /// `revwalk_next_unsorted`.
    #[default]
    Unsorted,
    /// `revwalk_next_timesort`.
    Time,
    /// `revwalk_next_toposort`.
    Topo,
    /// `revwalk_next_reverse`.
    Reverse,
}

/// Which `enqueue` the walk uses (C's function pointer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Enqueue {
    /// `revwalk_enqueue_unsorted`.
    #[default]
    Unsorted,
    /// `revwalk_enqueue_timesort`.
    Time,
}

/// C's `git_pqueue` with `git_commit_list_time_cmp`, over (time, node)
/// pairs: newest first, and C's heap layout, so that equal times come out
/// in the C's order.
#[derive(Debug, Default)]
struct TimeHeap {
    v: Vec<(i64, NodePtr)>,
}

impl TimeHeap {
    // `git_commit_list_time_cmp(a, b) > 0` is `a.time < b.time`.

    /// `git_pqueue_insert`.
    fn insert(&mut self, n: NodePtr) -> Result<()> {
        self.v.try_reserve(1).map_err(|_| errors::oom())?;
        self.v.push((n.time(), n));
        // pqueue_up
        let v = &mut self.v;
        let mut el = v.len() - 1;
        let kid = v[el];
        while el > 0 {
            let parent_el = (el - 1) >> 1;
            let parent = v[parent_el];
            if parent.0 >= kid.0 {
                break;
            }
            v[el] = parent;
            el = parent_el;
        }
        v[el] = kid;
        Ok(())
    }

    /// `git_pqueue_pop`.
    fn pop(&mut self) -> Option<NodePtr> {
        let top = *self.v.first()?;
        let last = self.v.pop()?;
        if !self.v.is_empty() {
            // pqueue_down from the top, with the last element
            let v = &mut self.v;
            let n = v.len();
            let mut el = 0;
            loop {
                let mut kid_el = (el << 1) + 1;
                if kid_el >= n {
                    break;
                }
                let mut kid = v[kid_el];
                if kid_el + 1 < n && kid.0 < v[kid_el + 1].0 {
                    kid = v[kid_el + 1];
                    kid_el += 1;
                }
                if last.0 >= kid.0 {
                    break;
                }
                v[el] = kid;
                el = kid_el;
            }
            v[el] = last;
        }
        Some(top.1)
    }

    fn clear(&mut self) {
        self.v.clear();
    }
}

/// The queue of `sort_in_topological_order`: C's `git_pqueue` by time
/// (with `GIT_SORT_TIME`) or without comparison (a stack).
enum TopoQueue {
    Time(TimeHeap),
    Stack(Vec<NodePtr>),
}

impl TopoQueue {
    fn insert(&mut self, n: NodePtr) -> Result<()> {
        match self {
            TopoQueue::Time(h) => h.insert(n),
            TopoQueue::Stack(s) => {
                s.try_reserve(1).map_err(|_| errors::oom())?;
                s.push(n);
                Ok(())
            }
        }
    }

    fn pop(&mut self) -> Option<NodePtr> {
        match self {
            TopoQueue::Time(h) => h.pop(),
            TopoQueue::Stack(s) => s.pop(),
        }
    }
}

/// The walk's own state, after the prefix (Rust-native; C never sees it).
#[derive(Debug, Default)]
pub struct State {
    iterator_topo: VecDeque<NodePtr>,
    iterator_rand: VecDeque<NodePtr>,
    /// A stack (C pushes at the head and pops from it).
    iterator_reverse: Vec<NodePtr>,
    iterator_time: TimeHeap,
    get_next: Next,
    enqueue: Enqueue,
    walking: bool,
    first_parent: bool,
    did_hide: bool,
    did_push: bool,
    limited: bool,
    sorting: c_uint,
    hide_cb: HideCb,
    hide_cb_payload: *mut c_void,
    pathspec: Option<Pathspec>,
    pathspec_wildcard: bool,
}

/// `struct git_revwalk`: the C-visible prefix, then the Rust state.
#[repr(C)]
#[derive(Debug)]
pub struct GitRevwalk {
    /// What C sees.
    pub prefix: RevwalkPrefix,
    state: UnsafeCell<State>,
}

/// What the walk's steps read of the state (copied out, so that the
/// iterators can be borrowed beside it).
#[derive(Clone, Copy)]
struct Ctx<'w> {
    w: &'w GitRevwalk,
    hide_cb: HideCb,
    payload: *mut c_void,
    first_parent: bool,
    limited: bool,
    pathspec: Option<Pathspec>,
    pathspec_wildcard: bool,
}

impl<'w> Ctx<'w> {
    fn new(w: &'w GitRevwalk, st: &State) -> Ctx<'w> {
        Ctx {
            w,
            hide_cb: st.hide_cb,
            payload: st.hide_cb_payload,
            first_parent: st.first_parent,
            limited: st.limited,
            pathspec: st.pathspec,
            pathspec_wildcard: st.pathspec_wildcard,
        }
    }

    /// `walk->hide_cb && walk->hide_cb(&commit->oid, walk->hide_cb_payload)`.
    #[inline]
    fn hidden(&self, n: NodePtr) -> bool {
        match self.hide_cb {
            None => false,
            // SAFETY: the callback and payload the user registered with
            // `git_revwalk_add_hide_cb`, called as C does, with the node's
            // id (valid for the call). Its contract forbids it to use the
            // walk.
            Some(cb) => unsafe { cb(n.oid_ptr().as_ptr(), self.payload) != 0 },
        }
    }
}

impl GitRevwalk {
    /// A walk over `repo` with an empty state and no odb (the first half of
    /// `git_revwalk_new`).
    fn with_repo(repo: *mut git_repository) -> Result<GitRevwalk> {
        Ok(GitRevwalk {
            prefix: RevwalkPrefix {
                repo,
                odb: ptr::null_mut(),
                commits: UnsafeCell::new(CommitMap::new()),
                commit_pool: UnsafeCell::new(GitPool::with_item_size(COMMIT_ALLOC)?),
                user_input: Cell::new(ptr::null_mut()),
            },
            state: UnsafeCell::new(State::default()),
        })
    }

    /// The state.
    ///
    /// # Safety
    ///
    /// No other reference to the state lives while the result does: only
    /// this module makes one, once per entry point, and an entry point is
    /// never re-entered while another runs on the same walk (C that stays
    /// only calls `git_revwalk__commit_lookup`, which does not touch the
    /// state; the hide callback may not use the walk; a walk is used by one
    /// thread at a time).
    #[allow(clippy::mut_from_ref)]
    unsafe fn state(&self) -> &mut State {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.state.get() }
    }

    fn repo(&self) -> Repo {
        // SAFETY: the repository the walk was made with, which C requires
        // to outlive it.
        unsafe { Repo::new(self.prefix.repo) }
    }

    /// `git_revwalk__commit_lookup`: the node of `oid`, made if new; `None`
    /// (with the error set) if out of memory.
    pub fn commit_lookup(&self, oid: &Oid) -> Option<NodePtr> {
        // SAFETY: no other reference to the map lives: they are made only
        // here and in `reset`, for the extent of a call that does not reach
        // C or the user.
        let map = unsafe { &mut *self.prefix.commits.get() };
        // SAFETY: a key for the lookup alone: `oid` outlives it.
        let key = unsafe { OidPtr::new(NonNull::from(oid)) };
        if let Some(n) = map.get(&key) {
            return Some(*n);
        }

        // SAFETY: as for the map: no other reference to the pool lives (C
        // allocates from it only inside `git_commit_list_parse`, which is
        // not running now).
        let pool = unsafe { &mut *self.prefix.commit_pool.get() };
        // git_commit_list_alloc_node: one zeroed item of COMMIT_ALLOC bytes
        let p = pool.mallocz(1).ok()?.cast::<CommitListNode>();
        // SAFETY: a fresh, zeroed (so initialized: every field is valid
        // as zero) and 8-aligned node of the walk's pool, which lives as
        // long as the walk; the walk is used from one thread.
        let node = unsafe { NodePtr::new(p) };
        // SAFETY: the node's oid field, written once before it is shared.
        unsafe { node.oid_ptr().as_ptr().write(*oid) };
        // SAFETY: the key points into the node, which lives (and keeps its
        // oid unchanged) as long as the map.
        let key = unsafe { OidPtr::new(node.oid_ptr()) };
        map.put(key, node).ok()?;
        Some(node)
    }

    /// `git_commit_list_parse`, skipping the call when parsed.
    #[inline]
    fn parse(&self, n: NodePtr) -> Result<()> {
        if n.parsed() {
            return Ok(());
        }
        glue::commit_list_parse(self, n)
    }

    // -- pushing and hiding ------------------------------------------------

    /// `git_revwalk__push_commit`.
    fn push_commit(&self, st: &mut State, oid: &Oid, opts: &PushOptions) -> Result<()> {
        let oobj = glue::object_lookup(self.repo(), oid, GIT_OBJECT_ANY)?;
        let obj = oobj.peel(GIT_OBJECT_COMMIT);
        drop(oobj);
        let obj = match obj {
            Err(e)
                if [
                    ErrorCode::NOT_FOUND,
                    ErrorCode::INVALID_SPEC,
                    ErrorCode::PEEL,
                ]
                .contains(&e) =>
            {
                // If this comes from e.g. push_glob("tags"), ignore this
                if opts.from_glob != 0 {
                    return Ok(());
                }
                let _ = git_error_set!(ErrorClass::INVALID, "object is not a committish");
                return Err(e);
            }
            r => r?,
        };
        let commit_id = obj.id();
        drop(obj);

        // the error is set by the failed lookup
        let commit = self.commit_lookup(&commit_id).ok_or(ErrorCode::GENERIC)?;
        self.push_node(st, commit, opts)
    }

    /// The second half of `git_revwalk__push_commit`, for a node.
    fn push_node(&self, st: &mut State, commit: NodePtr, opts: &PushOptions) -> Result<()> {
        // A previous hide already told us we don't want this commit
        if commit.uninteresting() {
            return Ok(());
        }

        if opts.uninteresting != 0 {
            st.limited = true;
            st.did_hide = true;
        } else {
            st.did_push = true;
        }

        // `commit->uninteresting = opts->uninteresting`: a 1-bit field
        commit.set_uninteresting(opts.uninteresting & 1 != 0);
        let mut list = self.prefix.user_input.get();

        // To insert by date, we need to parse so we know the date.
        if opts.insert_by_date != 0 {
            self.parse(commit)?;
        }

        // SAFETY: `user_input` is the walk's well-formed list, which
        // nothing else reaches during the call.
        let by_date = opts.insert_by_date != 0
            && unsafe { node::list_insert_by_date(commit, &mut list) }.is_ok();
        if !by_date && node::list_insert(commit, &mut list).is_err() {
            return Err(errors::oom());
        }
        self.prefix.user_input.set(list);
        Ok(())
    }

    /// `git_revwalk__push_ref`.
    fn push_ref(
        &self,
        st: &mut State,
        refname: *const core::ffi::c_char,
        opts: &PushOptions,
    ) -> Result<()> {
        match glue::reference_name_to_id(self.repo(), refname) {
            Err(e)
                if opts.from_glob != 0
                    && [
                        ErrorCode::NOT_FOUND,
                        ErrorCode::INVALID_SPEC,
                        ErrorCode::PEEL,
                    ]
                    .contains(&e) =>
            {
                Ok(())
            }
            Err(_) => Err(ErrorCode::GENERIC),
            Ok(oid) => self.push_commit(st, &oid, opts),
        }
    }

    /// `git_revwalk__push_glob`.
    fn push_glob(&self, st: &mut State, glob: &CStr, given: Option<&PushOptions>) -> Result<()> {
        let mut opts = given.copied().unwrap_or_default();
        let g = glob.to_bytes();

        // refs/ is implied if not given in the glob
        let mut buf = GitStr::new();
        if !g.starts_with(b"refs/") {
            let _ = buf.joinpath(b"refs/", g);
        } else {
            let _ = buf.puts(g);
        }
        if buf.is_oom() {
            return Err(ErrorCode::GENERIC);
        }

        // If no '?', '*' or '[' exist, we append '/*' to the glob
        if !g.iter().any(|c| matches!(c, b'?' | b'*' | b'[')) {
            let _ = buf.put(b"/*");
        }
        let Some(pattern) = buf.as_c_str() else {
            return Err(ErrorCode::GENERIC);
        };

        let mut iter = glue::RefIter::glob(self.repo(), pattern)?;
        opts.from_glob = 1;
        let r = loop {
            match iter.next() {
                Ok(r) => {
                    let e = self.push_ref(st, r.name_ptr(), &opts);
                    drop(r);
                    if let Err(e) = e {
                        break Err(e);
                    }
                }
                Err(ErrorCode::ITER_OVER) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        drop(iter);
        r
    }

    /// `git_revwalk_push_range`.
    fn push_range(&self, st: &mut State, range: *const core::ffi::c_char) -> Result<()> {
        let spec = glue::revparse(self.repo(), range)?;

        let Some(to) = spec.to.as_ref() else {
            let _ = git_error_set!(ErrorClass::INVALID, "invalid revspec: range not provided");
            return Err(ErrorCode::INVALID_SPEC);
        };

        if spec.flags & glue::GIT_REVSPEC_MERGE_BASE != 0 {
            // TODO: support "<commit>...<commit>"
            let _ = git_error_set!(
                ErrorClass::INVALID,
                "symmetric differences not implemented in revwalk"
            );
            return Err(ErrorCode::INVALID_SPEC);
        }

        // A range always has its left side (C dereferences it).
        let Some(from) = spec.from.as_ref() else {
            let _ = git_error_set!(ErrorClass::INVALID, "invalid revspec: range not provided");
            return Err(ErrorCode::INVALID_SPEC);
        };

        let mut opts = PushOptions {
            uninteresting: 1,
            ..PushOptions::default()
        };
        self.push_commit(st, &from.id(), &opts)?;
        opts.uninteresting = 0;
        self.push_commit(st, &to.id(), &opts)
    }

    // -- the walk ----------------------------------------------------------

    /// `git_revwalk_reset`.
    fn reset(&self, st: &mut State) {
        {
            // SAFETY: as in `commit_lookup`: the only reference to the map,
            // for a loop that does not reach C.
            let map = unsafe { &*self.prefix.commits.get() };
            for n in map.values() {
                n.reset();
            }
        }
        st.iterator_time.clear();
        st.iterator_topo.clear();
        st.iterator_rand.clear();
        st.iterator_reverse.clear();
        let mut list = self.prefix.user_input.replace(ptr::null_mut());
        // SAFETY: the walk's own list, not used after.
        unsafe { node::list_free(&mut list) };
        st.first_parent = false;
        st.walking = false;
        st.limited = false;
        st.did_push = false;
        st.did_hide = false;
        st.sorting = GIT_SORT_NONE;
    }

    /// `git_revwalk_sorting`.
    fn set_sorting(&self, st: &mut State, mode: c_uint) {
        if st.walking {
            self.reset(st);
        }
        st.sorting = mode;
        if mode & GIT_SORT_TIME != 0 {
            st.get_next = Next::Time;
            st.enqueue = Enqueue::Time;
        } else {
            st.get_next = Next::Unsorted;
            st.enqueue = Enqueue::Unsorted;
        }
        if mode != GIT_SORT_NONE {
            st.limited = true;
        }
    }

    /// `git_revwalk_pathspec`.
    fn set_pathspec(&self, st: &mut State, spec: Option<Pathspec>) {
        if st.walking {
            self.reset(st);
        }
        if let Some(spec) = spec {
            st.pathspec = Some(spec);
            st.pathspec_wildcard = spec.has_wildcard();
            st.limited = true;
        }
    }

    /// `git_revwalk_add_hide_cb`.
    fn set_hide_cb(&self, st: &mut State, cb: HideCb, payload: *mut c_void) {
        if st.walking {
            self.reset(st);
        }
        st.hide_cb = cb;
        st.hide_cb_payload = payload;
        if cb.is_some() {
            st.limited = true;
        }
    }

    /// `git_revwalk_next`.
    fn next(&self, st: &mut State) -> Result<Oid> {
        if !st.walking {
            prepare_walk(self, st)?;
        }
        match get_next(self, st) {
            Ok(n) => Ok(n.oid()),
            Err(ErrorCode::ITER_OVER) => {
                self.reset(st);
                errors::clear();
                Err(ErrorCode::ITER_OVER)
            }
            Err(e) => Err(e),
        }
    }
}

/// `insert_by_date` into a Rust list: before the first older commit.
#[inline]
fn insert_by_date(list: &mut VecDeque<NodePtr>, n: NodePtr) -> Result<()> {
    let t = n.time();
    let pos = list.iter().position(|p| p.time() < t).unwrap_or(list.len());
    list.try_reserve(1).map_err(|_| errors::oom())?;
    list.insert(pos, n);
    Ok(())
}

/// `mark_parents_uninteresting`.
fn mark_parents_uninteresting(commit: NodePtr) -> Result<()> {
    // C's list: parents inserted at the head, popped from it (a stack).
    let mut parents: Vec<NodePtr> = Vec::new();
    let push = |s: &mut Vec<NodePtr>, n: NodePtr| -> Result<()> {
        s.try_reserve(1).map_err(|_| errors::oom())?;
        s.push(n);
        Ok(())
    };
    for i in 0..commit.out_degree() {
        if let Some(p) = commit.parent(i) {
            push(&mut parents, p)?;
        }
    }

    while let Some(start) = parents.pop() {
        let mut c = Some(start);
        while let Some(n) = c {
            if n.uninteresting() {
                break;
            }
            n.set_uninteresting(true);
            // If we've reached this commit some other way already, we
            // need to mark its parents uninteresting as well.
            if !n.has_parents() {
                break;
            }
            for i in 0..n.out_degree() {
                if let Some(p) = n.parent(i) {
                    push(&mut parents, p)?;
                }
            }
            // `commit->parents[0]`: for a root commit, C reads the zeroed
            // first slot of the node's tail, NULL.
            c = n.parent(0);
        }
    }
    Ok(())
}

/// `add_parents_to_list`.
fn add_parents_to_list(ctx: Ctx<'_>, commit: NodePtr, list: &mut VecDeque<NodePtr>) -> Result<()> {
    if commit.added() {
        return Ok(());
    }
    commit.set_added();

    // Go full on in the uninteresting case as we want to include as many
    // of these as we can. Usually we haven't parsed the parent of a
    // parent, but if we have it we reached it via other means so we want
    // to mark its parents recursively too.
    if commit.uninteresting() {
        for i in 0..commit.out_degree() {
            let Some(p) = commit.parent(i) else { continue };
            p.set_uninteresting(true);

            // git does it gently here, but we don't like missing objects
            ctx.w.parse(p)?;

            if p.has_parents() {
                mark_parents_uninteresting(p)?;
            }

            p.set_seen();
            insert_by_date(list, p)?;
        }
        return Ok(());
    }

    // Now on to what we do if the commit is indeed interesting. Here we
    // do want things like first-parent take effect as this is what we'll
    // be showing.
    for i in 0..commit.out_degree() {
        let Some(p) = commit.parent(i) else { continue };

        ctx.w.parse(p)?;

        if ctx.hidden(p) {
            continue;
        }

        if !p.seen() {
            p.set_seen();
            insert_by_date(list, p)?;
        }

        if ctx.first_parent {
            break;
        }
    }
    Ok(())
}

/// `still_interesting`.
fn still_interesting(list: &VecDeque<NodePtr>, time: i64, slop: c_int) -> c_int {
    // The empty list is pretty boring
    let Some(first) = list.front() else { return 0 };

    // If the destination list has commits with an earlier date than our
    // source, we want to reset the slop counter as we're not done.
    if time <= first.time() {
        return SLOP;
    }

    // If the destination list still contains interesting commits we want
    // to continue looking.
    if list.iter().any(|n| !n.uninteresting() || n.time() > time) {
        return SLOP;
    }

    // Everything's uninteresting, reduce the count
    slop - 1
}

/// `limit_list`.
fn limit_list(ctx: Ctx<'_>, mut list: VecDeque<NodePtr>) -> Result<VecDeque<NodePtr>> {
    let mut slop = SLOP;
    let mut time = i64::MAX;
    let mut newlist = VecDeque::new();

    while let Some(commit) = list.pop_front() {
        add_parents_to_list(ctx, commit, &mut list)?;

        if commit.uninteresting() {
            mark_parents_uninteresting(commit)?;

            slop = still_interesting(&list, time, slop);
            if slop != 0 {
                continue;
            }
            break;
        }

        if let Some(spec) = ctx.pathspec
            && !glue::include_path(ctx.w.repo(), spec, ctx.pathspec_wildcard, commit)
        {
            continue;
        }

        if ctx.hidden(commit) {
            continue;
        }

        time = commit.time();
        newlist.try_reserve(1).map_err(|_| errors::oom())?;
        newlist.push_back(commit);
    }

    Ok(newlist)
}

/// `get_revision`.
#[inline]
fn get_revision(ctx: Ctx<'_>, list: &mut VecDeque<NodePtr>) -> Result<NodePtr> {
    let Some(commit) = list.pop_front() else {
        errors::clear();
        return Err(ErrorCode::ITER_OVER);
    };

    // If we did not run limit_list, we must add parents to the list
    // ourselves.
    if !ctx.limited {
        add_parents_to_list(ctx, commit, list)?;
    }
    Ok(commit)
}

/// `sort_in_topological_order`.
fn sort_in_topological_order(
    sorting: c_uint,
    list: VecDeque<NodePtr>,
) -> Result<VecDeque<NodePtr>> {
    let mut queue = if sorting & GIT_SORT_TIME != 0 {
        TopoQueue::Time(TimeHeap::default())
    } else {
        TopoQueue::Stack(Vec::new())
    };

    // Start by resetting the in-degree to 1 for the commits in our list.
    for n in &list {
        n.set_in_degree(1);
    }

    // Count up how many children each commit has. We limit ourselves to
    // those commits in the original list (in-degree of 1) avoiding setting
    // it for any parent that was hidden.
    for n in &list {
        for i in 0..n.out_degree() {
            if let Some(p) = n.parent(i)
                && p.in_degree() != 0
            {
                p.set_in_degree(p.in_degree().wrapping_add(1));
            }
        }
    }

    // Now we find the tips i.e. those not reachable from any other node
    // i.e. those which still have an in-degree of 1.
    for n in &list {
        if n.in_degree() == 1 {
            queue.insert(*n)?;
        }
    }

    // We need to output the tips in the order that they came out of the
    // traversal, so if we're not doing time-sorting, we need to reverse
    // the queue in order to get them to come out as we inserted them.
    if let TopoQueue::Stack(s) = &mut queue {
        s.reverse();
    }

    let mut out = VecDeque::new();
    out.try_reserve(list.len()).map_err(|_| errors::oom())?;
    while let Some(next) = queue.pop() {
        for i in 0..next.out_degree() {
            let Some(p) = next.parent(i) else { continue };
            if p.in_degree() == 0 {
                continue;
            }
            let d = p.in_degree() - 1;
            p.set_in_degree(d);
            if d == 1 {
                queue.insert(p)?;
            }
        }

        // All the children of 'item' have been emitted (since we got to it
        // via the priority queue)
        next.set_in_degree(0);

        out.try_reserve(1).map_err(|_| errors::oom())?;
        out.push_back(next);
    }
    Ok(out)
}

/// `walk->get_next(&next, walk)`.
fn get_next(w: &GitRevwalk, st: &mut State) -> Result<NodePtr> {
    let ctx = Ctx::new(w, st);
    match st.get_next {
        Next::Time => {
            while let Some(n) = st.iterator_time.pop() {
                // Some commits might become uninteresting after being added
                // to the list
                if !n.uninteresting() {
                    return Ok(n);
                }
            }
            errors::clear();
            Err(ErrorCode::ITER_OVER)
        }
        Next::Unsorted => loop {
            let n = get_revision(ctx, &mut st.iterator_rand)?;
            if !n.uninteresting() {
                return Ok(n);
            }
        },
        Next::Topo => loop {
            let n = get_revision(ctx, &mut st.iterator_topo)?;
            if !n.uninteresting() {
                return Ok(n);
            }
        },
        Next::Reverse => st.iterator_reverse.pop().ok_or(ErrorCode::ITER_OVER),
    }
}

/// `prepare_walk`.
fn prepare_walk(w: &GitRevwalk, st: &mut State) -> Result<()> {
    // If there were no pushes, we know that the walk is already over
    if !st.did_push {
        errors::clear();
        return Err(ErrorCode::ITER_OVER);
    }

    // Keep the order of the pushes (`insert_by_date` for fetch
    // negotiation).
    let mut commits = VecDeque::new();
    // SAFETY: the walk's well-formed list; nothing changes it during the
    // loop (parsing and marking touch nodes, not the list).
    for commit in unsafe { node::list_items(w.prefix.user_input.get()) } {
        w.parse(commit)?;

        if commit.uninteresting() {
            mark_parents_uninteresting(commit)?;
        }

        if !commit.seen() {
            commit.set_seen();
            commits.try_reserve(1).map_err(|_| errors::oom())?;
            commits.push_back(commit);
        }
    }

    if st.limited {
        commits = limit_list(Ctx::new(w, st), commits)?;
    }

    if st.sorting & GIT_SORT_TOPOLOGICAL != 0 {
        st.iterator_topo = sort_in_topological_order(st.sorting, commits)?;
        st.get_next = Next::Topo;
    } else if st.sorting & GIT_SORT_TIME != 0 {
        for c in commits {
            match st.enqueue {
                Enqueue::Time => st.iterator_time.insert(c)?,
                Enqueue::Unsorted => {
                    st.iterator_rand.try_reserve(1).map_err(|_| errors::oom())?;
                    st.iterator_rand.push_front(c);
                }
            }
        }
    } else {
        st.iterator_rand = commits;
        st.get_next = Next::Unsorted;
    }

    if st.sorting & GIT_SORT_REVERSE != 0 {
        loop {
            match get_next(w, st) {
                Ok(n) => {
                    st.iterator_reverse
                        .try_reserve(1)
                        .map_err(|_| errors::oom())?;
                    st.iterator_reverse.push(n);
                }
                Err(ErrorCode::ITER_OVER) => break,
                Err(e) => return Err(e),
            }
        }
        st.get_next = Next::Reverse;
    }

    st.walking = true;
    Ok(())
}

/// Allocate a walk in `git__allocator`'s storage (C's `git__calloc`).
fn alloc_walk(repo: *mut git_repository) -> Result<NonNull<GitRevwalk>> {
    let p = crate::util::alloc::calloc(1, size_of::<GitRevwalk>()).cast::<GitRevwalk>();
    let p = NonNull::new(p).ok_or(ErrorCode::GENERIC)?;
    match GitRevwalk::with_repo(repo) {
        Ok(w) => {
            // SAFETY: a fresh allocation of the size of `GitRevwalk`,
            // 8-aligned (git__allocator), enough for it.
            unsafe { p.as_ptr().write(w) };
            Ok(p)
        }
        Err(e) => {
            // SAFETY: our allocation, uninitialized, not used after.
            unsafe { crate::util::alloc::free(p.as_ptr().cast()) };
            Err(e)
        }
    }
}

/// `git_revwalk_new`.
fn new_walk(repo: *mut git_repository) -> Result<NonNull<GitRevwalk>> {
    let p = alloc_walk(repo)?;
    // SAFETY: the walk is ours alone: a shared reference for the call.
    let r = glue::repository_odb(unsafe { p.as_ref() }.repo());
    match r {
        Ok(odb) => {
            // SAFETY: ours alone; set before anything else sees it.
            unsafe { (*p.as_ptr()).prefix.odb = odb };
            Ok(p)
        }
        Err(_) => {
            // SAFETY: ours, not used after.
            unsafe { free_walk(p) };
            Err(ErrorCode::GENERIC)
        }
    }
}

/// `git_revwalk_free`.
///
/// # Safety
///
/// `p` is a walk from [`alloc_walk`], which nothing uses after (and no
/// reference to which lives).
unsafe fn free_walk(p: NonNull<GitRevwalk>) {
    {
        // SAFETY: a live walk (the contract), for the extent of the block.
        let w = unsafe { p.as_ref() };
        // SAFETY: nothing else uses the walk now.
        let st = unsafe { w.state() };
        w.reset(st);
        // SAFETY: the walk's reference, not used after.
        unsafe { glue::odb_free(w.prefix.odb) };
    }
    // SAFETY: a live walk that nothing uses after; its state (NodePtrs)
    // drops before its pool, and nothing reads the nodes in between.
    unsafe { ptr::drop_in_place(p.as_ptr()) };
    // SAFETY: the walk's storage, from git__allocator.
    unsafe { crate::util::alloc::free(p.as_ptr().cast()) };
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `revwalk.c`.
///
/// Every export that takes a `git_revwalk *` requires a walk from
/// `git_revwalk_new` that is not freed, used by one thread at a time, and
/// not re-entered from the hide callback; and a repository that outlives
/// it.
pub mod ffi {
    use core::ffi::{c_char, c_int, c_uint, c_void};
    use core::ptr::{self, NonNull};

    use super::*;
    use crate::util::export::{barrier, cstr, status};

    /// The walk at `walk`, and its state.
    ///
    /// # Safety
    ///
    /// The contract of the module's exports: a live walk, not used
    /// concurrently or re-entrantly.
    #[inline(always)]
    unsafe fn walk_of<'a>(walk: *mut GitRevwalk) -> (&'a GitRevwalk, &'a mut State) {
        // SAFETY: a live walk (the contract); a shared reference, since C
        // writes only the `UnsafeCell` parts while it lives.
        let w = unsafe { &*walk };
        // SAFETY: the only reference to the state: an export is never
        // re-entered on the same walk (the contract).
        let st = unsafe { w.state() };
        (w, st)
    }

    /// `git_revwalk__commit_lookup`: the walk's node for `oid`, made if
    /// new; NULL if out of memory.
    ///
    /// # Safety
    ///
    /// A live walk (which C may call this on while the walk runs
    /// `git_commit_list_parse`); `oid` is readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk__commit_lookup(
        walk: *mut GitRevwalk,
        oid: *const Oid,
    ) -> *mut CommitListNode {
        barrier(|| {
            if walk.is_null() || oid.is_null() {
                // C dereferences both
                let _ = errors::assert_arg_failed(if walk.is_null() { "walk" } else { "oid" });
                return ptr::null_mut();
            }
            // SAFETY: a live walk, of which only the map and the pool are
            // used (no state reference: this may run inside another export).
            let w = unsafe { &*walk };
            // SAFETY: a readable oid (the contract), copied.
            let oid = unsafe { *oid };
            w.commit_lookup(&oid)
                .map_or(ptr::null_mut(), NodePtr::as_ptr)
        })
    }

    /// `git_revwalk__push_commit`.
    ///
    /// # Safety
    ///
    /// The module's contract; `oid` and `opts` are readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk__push_commit(
        walk: *mut GitRevwalk,
        oid: *const Oid,
        opts: *const PushOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!oid.is_null(), "oid", -1);
            git_assert_arg!(!opts.is_null(), "opts", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            // SAFETY: readable (the contract), copied.
            let (oid, opts) = unsafe { (*oid, *opts) };
            status(w.push_commit(st, &oid, &opts))
        })
    }

    /// Push or hide `oid` (`git_revwalk_push` / `git_revwalk_hide`).
    ///
    /// # Safety
    ///
    /// The module's contract; `oid` is readable.
    unsafe fn push_oid(walk: *mut GitRevwalk, oid: *const Oid, uninteresting: c_int) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            // SAFETY: readable, copied.
            let oid = unsafe { *oid };
            let opts = PushOptions {
                uninteresting,
                ..PushOptions::default()
            };
            status(w.push_commit(st, &oid, &opts))
        })
    }

    /// `git_revwalk_push`.
    ///
    /// # Safety
    ///
    /// The module's contract; `oid` is readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_push(walk: *mut GitRevwalk, oid: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { push_oid(walk, oid, 0) }
    }

    /// `git_revwalk_hide`.
    ///
    /// # Safety
    ///
    /// The module's contract; `oid` is readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_hide(walk: *mut GitRevwalk, oid: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { push_oid(walk, oid, 1) }
    }

    /// `git_revwalk__push_ref`.
    ///
    /// # Safety
    ///
    /// The module's contract; `refname` is a string; `opts` is readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk__push_ref(
        walk: *mut GitRevwalk,
        refname: *const c_char,
        opts: *const PushOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!opts.is_null(), "opts", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            // SAFETY: readable, copied.
            let opts = unsafe { *opts };
            status(w.push_ref(st, refname, &opts))
        })
    }

    /// Push or hide a reference (`git_revwalk_push_ref`, `_head`, …).
    ///
    /// # Safety
    ///
    /// The module's contract; `refname` is a string.
    unsafe fn push_refname(
        walk: *mut GitRevwalk,
        refname: *const c_char,
        uninteresting: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!refname.is_null(), "refname", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            let opts = PushOptions {
                uninteresting,
                ..PushOptions::default()
            };
            status(w.push_ref(st, refname, &opts))
        })
    }

    /// `git_revwalk_push_ref`.
    ///
    /// # Safety
    ///
    /// The module's contract; `refname` is a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_push_ref(
        walk: *mut GitRevwalk,
        refname: *const c_char,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { push_refname(walk, refname, 0) }
    }

    /// `git_revwalk_hide_ref`.
    ///
    /// # Safety
    ///
    /// The module's contract; `refname` is a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_hide_ref(
        walk: *mut GitRevwalk,
        refname: *const c_char,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { push_refname(walk, refname, 1) }
    }

    /// `git_revwalk_push_head`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_push_head(walk: *mut GitRevwalk) -> c_int {
        // SAFETY: forwarded, with a static string.
        unsafe { push_refname(walk, c"HEAD".as_ptr(), 0) }
    }

    /// `git_revwalk_hide_head`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_hide_head(walk: *mut GitRevwalk) -> c_int {
        // SAFETY: forwarded, with a static string.
        unsafe { push_refname(walk, c"HEAD".as_ptr(), 1) }
    }

    /// `git_revwalk__push_glob`.
    ///
    /// # Safety
    ///
    /// The module's contract; `glob` is a string; `given_opts` is NULL or
    /// readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk__push_glob(
        walk: *mut GitRevwalk,
        glob: *const c_char,
        given_opts: *const PushOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!glob.is_null(), "glob", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            // SAFETY: a string (the contract), unchanged during the call.
            let Some(glob) = (unsafe { cstr(glob) }) else {
                return -1;
            };
            // SAFETY: NULL or readable, copied.
            let opts = unsafe { given_opts.as_ref() }.copied();
            status(w.push_glob(st, glob, opts.as_ref()))
        })
    }

    /// `git_revwalk_push_glob`.
    ///
    /// # Safety
    ///
    /// The module's contract; `glob` is a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_push_glob(
        walk: *mut GitRevwalk,
        glob: *const c_char,
    ) -> c_int {
        if walk.is_null() || glob.is_null() {
            let _ = errors::assert_arg_failed(if walk.is_null() { "walk" } else { "glob" });
            return -1;
        }
        let opts = PushOptions::default();
        // SAFETY: forwarded.
        unsafe { git_revwalk__push_glob(walk, glob, &opts) }
    }

    /// `git_revwalk_hide_glob`.
    ///
    /// # Safety
    ///
    /// The module's contract; `glob` is a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_hide_glob(
        walk: *mut GitRevwalk,
        glob: *const c_char,
    ) -> c_int {
        if walk.is_null() || glob.is_null() {
            let _ = errors::assert_arg_failed(if walk.is_null() { "walk" } else { "glob" });
            return -1;
        }
        let opts = PushOptions {
            uninteresting: 1,
            ..PushOptions::default()
        };
        // SAFETY: forwarded.
        unsafe { git_revwalk__push_glob(walk, glob, &opts) }
    }

    /// `git_revwalk_push_range`.
    ///
    /// # Safety
    ///
    /// The module's contract; `range` is a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_push_range(
        walk: *mut GitRevwalk,
        range: *const c_char,
    ) -> c_int {
        barrier(|| {
            // C dereferences `walk` unchecked
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            status(w.push_range(st, range))
        })
    }

    /// `git_revwalk_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `repo` is NULL or a live repository that
    /// outlives the walk.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_new(
        out: *mut *mut GitRevwalk,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            // C writes `*out` unchecked
            git_assert_arg!(!out.is_null(), "revwalk_out", -1);
            match new_walk(repo) {
                Ok(p) => {
                    // SAFETY: writable (the contract).
                    unsafe { out.write(p.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_revwalk_free`.
    ///
    /// # Safety
    ///
    /// `walk` is NULL or a live walk, not used after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_free(walk: *mut GitRevwalk) {
        barrier(|| {
            if let Some(p) = NonNull::new(walk) {
                // SAFETY: the contract.
                unsafe { free_walk(p) };
            }
        })
    }

    /// `git_revwalk_repository`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_repository(walk: *mut GitRevwalk) -> *mut git_repository {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", ptr::null_mut());
            // SAFETY: a live walk; `repo` is never written after creation.
            unsafe { (*walk).prefix.repo }
        })
    }

    /// `git_revwalk_sorting`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_sorting(
        walk: *mut GitRevwalk,
        sort_mode: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            w.set_sorting(st, sort_mode);
            0
        })
    }

    /// `git_revwalk_simplify_first_parent`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_simplify_first_parent(walk: *mut GitRevwalk) -> c_int {
        barrier(|| {
            // C dereferences `walk` unchecked
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (_, st) = unsafe { walk_of(walk) };
            st.first_parent = true;
            0
        })
    }

    /// `git_revwalk_next`.
    ///
    /// # Safety
    ///
    /// The module's contract; `oid` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_next(oid: *mut Oid, walk: *mut GitRevwalk) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            match w.next(st) {
                Ok(id) => {
                    // SAFETY: writable (the contract).
                    unsafe { oid.write(id) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_revwalk_pathspec`.
    ///
    /// # Safety
    ///
    /// The module's contract; `pathspec` is NULL or a live pathspec that
    /// outlives the walk's use of it and is not changed meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_pathspec(
        walk: *mut GitRevwalk,
        pathspec: *mut git_pathspec,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            // SAFETY: the contract.
            let spec = NonNull::new(pathspec).map(|p| unsafe { Pathspec::new(p) });
            w.set_pathspec(st, spec);
            0
        })
    }

    /// `git_revwalk_reset`.
    ///
    /// # Safety
    ///
    /// The module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_reset(walk: *mut GitRevwalk) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            w.reset(st);
            0
        })
    }

    /// `git_revwalk_add_hide_cb`.
    ///
    /// # Safety
    ///
    /// The module's contract; `hide_cb` is NULL or sound to call with a
    /// commit id and `payload` during the walks, and does not use the walk.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_revwalk_add_hide_cb(
        walk: *mut GitRevwalk,
        hide_cb: HideCb,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!walk.is_null(), "walk", -1);
            // SAFETY: the contract.
            let (w, st) = unsafe { walk_of(walk) };
            w.set_hide_cb(st, hide_cb, payload);
            0
        })
    }
}

crate::c_layout_test! {
    headers: ["revwalk.h", "commit_list.h"],
    types: [
        RevwalkPrefix => "git_revwalk" { repo, odb, commits, commit_pool, user_input },
        PushOptions => "git_revwalk__push_options" { uninteresting, from_glob, insert_by_date },
        CommitListNode => "git_commit_list_node" { oid, time, generation, in_degree, out_degree, parents },
        CommitList => "git_commit_list" { item, next },
        CommitMap => "git_revwalk_oidmap" { },
    ],
    asserts: [
        "COMMIT_ALLOC == 88",
        "PARENTS_PER_COMMIT == 2",
        "FLAG_BITS == 4",
        "GIT_SORT_NONE == 0 && GIT_SORT_TOPOLOGICAL == 1 && GIT_SORT_TIME == 2 && GIT_SORT_REVERSE == 4",
    ],
}
