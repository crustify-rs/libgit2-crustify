//! Tests of the walk's algorithms on graphs built in memory (no C: the
//! nodes are made parsed, so `git_commit_list_parse` is never called), so
//! that they run under Miri. Exact equivalence with the C on real
//! repositories is the differential test's (`rust/difftest/revwalk_diff.c`).

#![allow(clippy::multiple_unsafe_ops_per_block)]

use core::ffi::{c_int, c_void};
use std::collections::{HashMap, HashSet};

use super::*;
use crate::util::pqueue::GitPqueue;
use crate::util::test_support;

/// A walk over an in-memory graph; node `i` has the id `[i, 0, …]`.
struct Graph {
    w: Box<GitRevwalk>,
    nodes: Vec<NodePtr>,
}

impl Drop for Graph {
    fn drop(&mut self) {
        let st = self.st();
        self.w.reset(st);
    }
}

fn oid(i: usize) -> Oid {
    let mut raw = [0u8; 20];
    raw[0] = (i & 0xff) as u8;
    raw[1] = (i >> 8) as u8;
    Oid::from_bytes(&raw, OidType::Sha1).unwrap()
}

use crate::oid::OidType;

impl Graph {
    /// `times[i]` and `parents[i]` (indices) for each node.
    fn new(times: &[i64], parents: &[&[usize]]) -> Graph {
        let w = Box::new(GitRevwalk::with_repo(ptr::null_mut()).unwrap());
        let nodes: Vec<NodePtr> = (0..times.len())
            .map(|i| w.commit_lookup(&oid(i)).unwrap())
            .collect();
        let g = Graph { w, nodes };
        for (i, ps) in parents.iter().enumerate() {
            let ps: Vec<NodePtr> = ps.iter().map(|&p| g.nodes[p]).collect();
            // as C's `alloc_parents`: an array from the pool (one item of
            // COMMIT_ALLOC bytes has room for 11)
            assert!(ps.len() <= COMMIT_ALLOC / 8);
            // SAFETY: the only reference to the pool, for the call.
            let arr = unsafe { &mut *g.w.prefix.commit_pool.get() }
                .malloc(1)
                .unwrap();
            g.nodes[i].set_parsed(times[i], &ps, arr.as_ptr().cast());
        }
        g
    }

    fn st(&self) -> &State {
        self.w.state()
    }

    fn push(&self, i: usize) -> &Self {
        let opts = PushOptions::default();
        self.w.push_node(self.st(), self.nodes[i], &opts).unwrap();
        self
    }

    fn hide(&self, i: usize) -> &Self {
        let opts = PushOptions {
            uninteresting: 1,
            ..PushOptions::default()
        };
        self.w.push_node(self.st(), self.nodes[i], &opts).unwrap();
        self
    }

    fn sort(&self, mode: c_uint) -> &Self {
        self.w.set_sorting(self.st(), mode);
        self
    }

    fn first_parent(&self) -> &Self {
        self.st().first_parent.set(true);
        self
    }

    fn walk(&self) -> Vec<usize> {
        let index: HashMap<Oid, usize> = (0..self.nodes.len()).map(|i| (oid(i), i)).collect();
        let mut out = Vec::new();
        loop {
            match self.w.next(self.st()) {
                Ok(id) => out.push(index[&id]),
                Err(ErrorCode::ITER_OVER) => return out,
                Err(e) => panic!("walk failed: {e}"),
            }
        }
    }
}

/// The graph of `tests/resources/testrepo.git`'s HEAD (see the clar
/// `revwalk::basic`), with times in the same order:
///
/// ```text
///   0 merge
///   |\
///   | 1
///   | 2
///   3 |
///   |/
///   5
///   4
/// ```
fn testrepo() -> Graph {
    Graph::new(
        &[60, 40, 30, 50, 10, 20],
        &[&[3, 1], &[2], &[5], &[5], &[], &[4]],
    )
}

#[test]
fn testrepo_modes() {
    let _g = test_support::init();
    let g = testrepo();
    assert_eq!(g.push(0).walk(), [0, 3, 1, 2, 5, 4]);
    assert_eq!(g.sort(GIT_SORT_TIME).push(0).walk(), [0, 3, 1, 2, 5, 4]);
    // the C's choice of the two topological orders
    assert_eq!(
        g.sort(GIT_SORT_TOPOLOGICAL).push(0).walk(),
        [0, 1, 2, 3, 5, 4]
    );
    assert_eq!(
        g.sort(GIT_SORT_TOPOLOGICAL | GIT_SORT_TIME).push(0).walk(),
        [0, 3, 1, 2, 5, 4]
    );
    assert_eq!(
        g.sort(GIT_SORT_TIME | GIT_SORT_REVERSE).push(0).walk(),
        [4, 5, 2, 1, 3, 0]
    );
    assert_eq!(
        g.sort(GIT_SORT_TOPOLOGICAL | GIT_SORT_REVERSE)
            .push(0)
            .walk(),
        [4, 5, 3, 2, 1, 0]
    );
    assert_eq!(g.sort(GIT_SORT_REVERSE).push(0).walk(), [4, 5, 2, 1, 3, 0]);
    assert_eq!(g.first_parent().push(0).walk(), [0, 3, 5, 4]);
    // the walk resets at its end: first-parent is gone
    assert_eq!(g.push(1).walk(), [1, 2, 5, 4]);
}

#[test]
fn testrepo_hides() {
    let _g = test_support::init();
    let g = testrepo();
    assert_eq!(g.push(0).hide(3).walk(), [0, 1, 2]);
    assert_eq!(g.push(0).hide(1).walk(), [0, 3]);
    assert_eq!(g.push(1).hide(3).walk(), [1, 2]);
    // a hide before the push of the same commit wins; after it too
    assert_eq!(g.hide(0).push(0).walk(), Vec::<usize>::new());
    assert_eq!(g.push(1).hide(1).walk(), Vec::<usize>::new());
    // only hides: the walk is over at once
    g.hide(2);
    assert_eq!(g.w.next(g.st()), Err(ErrorCode::ITER_OVER));
    // which does not reset: the hide of 2 (and so of 5 and 4) is still
    // there
    assert_eq!(g.push(0).walk(), [0, 3, 1]);
    for mode in 0..8 {
        let got = g.sort(mode).push(0).hide(5).walk();
        let mut set = got.clone();
        set.sort_unstable();
        assert_eq!(set, [0, 1, 2, 3], "mode {mode}: {got:?}");
    }
}

unsafe extern "C" fn hide_index(id: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: the walk passes a node's id and our payload, a
    // `Cell<usize>`.
    let (id, which) = unsafe { (&*id, (*payload.cast::<core::cell::Cell<usize>>()).get()) };
    c_int::from(*id == oid(which))
}

#[test]
fn hide_callback() {
    let _g = test_support::init();
    let g = testrepo();
    let which = core::cell::Cell::new(3usize);
    let payload = ptr::from_ref(&which).cast_mut().cast::<c_void>();
    g.w.set_hide_cb(g.st(), Some(hide_index), payload);
    // a hidden commit is left out, and what is reachable only through it
    assert_eq!(g.push(0).walk(), [0, 1, 2, 5, 4]);
    which.set(1);
    assert_eq!(g.push(0).walk(), [0, 3, 5, 4]);
    // with first-parent, a hidden first parent lets the second through
    which.set(3);
    assert_eq!(g.first_parent().push(0).walk(), [0, 1, 2, 5, 4]);
    // The end of a walk resets `limited` (not the callback): unlimited,
    // only parents are asked about, so a hidden tip is still shown (C).
    which.set(0);
    assert_eq!(g.push(0).walk(), [0, 3, 1, 2, 5, 4]);
    // Limited (set again), limit_list asks about every commit it keeps,
    // the pushed ones too: a hidden tip is left out, its parents walked.
    g.w.set_hide_cb(g.st(), Some(hide_index), payload);
    assert_eq!(g.push(0).walk(), [3, 1, 2, 5, 4]);
    g.w.set_hide_cb(g.st(), None, ptr::null_mut());
    assert_eq!(g.push(0).walk(), [0, 3, 1, 2, 5, 4]);
}

#[test]
fn octopus_and_reset() {
    let _g = test_support::init();
    // 0 has four parents (an array outside the node), 1..4 share root 5
    let g = Graph::new(
        &[100, 90, 80, 95, 70, 10],
        &[&[1, 2, 3, 4], &[5], &[5], &[5], &[5], &[]],
    );
    assert_eq!(g.push(0).walk(), [0, 3, 1, 2, 4, 5]);
    assert_eq!(
        g.sort(GIT_SORT_TOPOLOGICAL).push(0).walk(),
        [0, 4, 3, 2, 1, 5]
    );
    assert_eq!(g.first_parent().push(0).walk(), [0, 1, 5]);
    // a partial walk, then reset
    g.push(0);
    assert_eq!(g.w.next(g.st()), Ok(oid(0)));
    g.w.reset(g.st());
    assert!(g.w.prefix.user_input.get().is_null());
    assert_eq!(g.push(2).walk(), [2, 5]);
}

#[test]
fn sorting_resets_a_walk_in_progress() {
    let _g = test_support::init();
    let g = testrepo();
    g.push(0);
    assert_eq!(g.w.next(g.st()), Ok(oid(0)));
    g.sort(GIT_SORT_TIME);
    // the push is gone with the reset
    assert_eq!(g.w.next(g.st()), Err(ErrorCode::ITER_OVER));
}

/// A deterministic pseudo-random sequence.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A random DAG of `n` nodes: node `i`'s parents are older nodes (higher
/// indices); times decrease with the index, with ties.
fn random_dag(rng: &mut Lcg, n: usize) -> (Vec<i64>, Vec<Vec<usize>>) {
    let times: Vec<i64> = (0..n).map(|i| ((n - i) / 3) as i64).collect();
    let parents = (0..n)
        .map(|i| {
            if i + 1 >= n {
                return vec![];
            }
            let k = match rng.below(10) {
                0 => 0,
                1..=6 => 1,
                7 | 8 => 2,
                _ => 3,
            };
            let mut ps = Vec::new();
            for _ in 0..k {
                let p = i + 1 + rng.below((n - i - 1).min(12));
                if !ps.contains(&p) {
                    ps.push(p);
                }
            }
            ps
        })
        .collect();
    (times, parents)
}

fn reachable(parents: &[Vec<usize>], from: &[usize]) -> HashSet<usize> {
    let mut seen = HashSet::new();
    let mut stack: Vec<usize> = from.to_vec();
    while let Some(i) = stack.pop() {
        if seen.insert(i) {
            stack.extend(&parents[i]);
        }
    }
    seen
}

#[test]
fn random_dags_properties() {
    let _g = test_support::init();
    let mut rng = Lcg(42);
    let rounds = if cfg!(miri) { 3 } else { 40 };
    for round in 0..rounds {
        let n = if cfg!(miri) { 40 } else { 150 };
        let (times, parents) = random_dag(&mut rng, n);
        let ps: Vec<&[usize]> = parents.iter().map(Vec::as_slice).collect();
        let g = Graph::new(&times, &ps);
        let pushes: Vec<usize> = (0..1 + rng.below(3)).map(|_| rng.below(n / 2)).collect();
        let all = reachable(&parents, &pushes);

        for mode in 0..8u32 {
            g.sort(mode);
            for &p in &pushes {
                g.push(p);
            }
            let out = g.walk();
            let set: HashSet<usize> = out.iter().copied().collect();
            assert_eq!(
                set.len(),
                out.len(),
                "round {round} mode {mode}: duplicates"
            );
            assert_eq!(set, all, "round {round} mode {mode}");

            let fwd = if mode & GIT_SORT_REVERSE != 0 {
                out.iter().rev().copied().collect::<Vec<_>>()
            } else {
                out.clone()
            };
            let pos: HashMap<usize, usize> = fwd.iter().enumerate().map(|(i, &c)| (c, i)).collect();
            if mode & GIT_SORT_TOPOLOGICAL != 0 {
                for &c in &fwd {
                    for p in &parents[c] {
                        assert!(
                            pos[&c] < pos[p],
                            "round {round} mode {mode}: {p} before its child {c}"
                        );
                    }
                }
            } else if mode & GIT_SORT_TIME != 0 {
                assert!(
                    fwd.windows(2).all(|w| times[w[0]] >= times[w[1]]),
                    "round {round} mode {mode}"
                );
            }
        }

        // the reverse of a mode is that mode reversed
        for mode in 0..4u32 {
            g.sort(mode);
            pushes.iter().for_each(|&p| {
                g.push(p);
            });
            let fwd = g.walk();
            g.sort(mode | GIT_SORT_REVERSE);
            pushes.iter().for_each(|&p| {
                g.push(p);
            });
            let mut rev = g.walk();
            rev.reverse();
            assert_eq!(fwd, rev, "round {round} mode {mode}");
        }

        // first-parent from one push: the first-parent chain
        g.first_parent().push(pushes[0]);
        let out = g.walk();
        let mut chain = vec![pushes[0]];
        while let Some(&p) = parents[*chain.last().unwrap()].first() {
            chain.push(p);
        }
        assert_eq!(out, chain, "round {round}");

        // hides: what is left is reachable from the pushes, and covers
        // what is not reachable from the hides
        let hide = n / 2 + rng.below(n / 2);
        let hidden = reachable(&parents, &[hide]);
        for mode in [0, GIT_SORT_TIME, GIT_SORT_TOPOLOGICAL] {
            g.sort(mode);
            for &p in &pushes {
                g.push(p);
            }
            g.hide(hide);
            let set: HashSet<usize> = g.walk().into_iter().collect();
            assert!(set.is_subset(&all));
            let want: HashSet<usize> = all.difference(&hidden).copied().collect();
            assert!(want.is_subset(&set), "round {round} mode {mode}");
            assert!(!set.contains(&hide));
        }
    }
}

#[test]
fn time_heap_is_cs_pqueue() {
    let _g = test_support::init();
    let mut rng = Lcg(7);
    let n = if cfg!(miri) { 60 } else { 600 };
    let times: Vec<i64> = (0..n).map(|_| rng.below(20) as i64).collect();
    let ps: Vec<&[usize]> = vec![&[]; n];
    let g = Graph::new(&times, &ps);

    let mut heap = TimeHeap::default();
    let mut pq = GitPqueue::new(0, 8, None).unwrap();
    // git_commit_list_time_cmp
    let mut cmp = |a: *mut c_void, b: *mut c_void| -> c_int {
        // SAFETY: the queue holds nodes of the graph.
        let (a, b) = unsafe {
            (
                NodePtr::new(NonNull::new_unchecked(a.cast())),
                NodePtr::new(NonNull::new_unchecked(b.cast())),
            )
        };
        match a.time().cmp(&b.time()) {
            core::cmp::Ordering::Less => 1,
            core::cmp::Ordering::Greater => -1,
            core::cmp::Ordering::Equal => 0,
        }
    };
    let mut next = 0;
    while next < n || !heap.v.is_empty() {
        if next < n && rng.below(3) != 0 {
            heap.insert(g.nodes[next]).unwrap();
            pq.insert_by(g.nodes[next].as_void(), &mut cmp).unwrap();
            next += 1;
        } else {
            let a = heap.pop().map_or(ptr::null_mut(), NodePtr::as_void);
            let b = pq.pop_by(&mut cmp);
            assert_eq!(a, b);
        }
    }
    assert!(pq.is_empty());
}

#[test]
fn user_input_is_a_c_list() {
    let _g = test_support::init();
    let g = testrepo();
    g.push(0).hide(3).push(2);
    // pushes are inserted at the head
    // SAFETY: the walk's list, unchanged while read.
    let items: Vec<NodePtr> = unsafe { node::list_items(g.w.prefix.user_input.get()) }.collect();
    assert_eq!(items, [g.nodes[2], g.nodes[3], g.nodes[0]]);
    g.w.reset(g.st());

    // by date, as fetch negotiation pushes
    let opts = PushOptions {
        insert_by_date: 1,
        ..PushOptions::default()
    };
    for i in [4, 0, 2, 3] {
        g.w.push_node(g.st(), g.nodes[i], &opts).unwrap();
    }
    // SAFETY: as above.
    let items: Vec<NodePtr> = unsafe { node::list_items(g.w.prefix.user_input.get()) }.collect();
    assert_eq!(items, [g.nodes[0], g.nodes[3], g.nodes[2], g.nodes[4]]);
}

#[test]
fn commit_lookup_is_stable() {
    let _g = test_support::init();
    let w = GitRevwalk::with_repo(ptr::null_mut()).unwrap();
    let a = w.commit_lookup(&oid(1)).unwrap();
    let b = w.commit_lookup(&oid(2)).unwrap();
    assert_ne!(a, b);
    assert_eq!(w.commit_lookup(&oid(1)), Some(a));
    assert_eq!(a.oid(), oid(1));
    assert!(!a.parsed() && !a.has_parents() && a.time() == 0);
    // many nodes: the map grows, the pool pages, the nodes stay put
    let nodes: Vec<NodePtr> = (0..3000)
        .map(|i| w.commit_lookup(&oid(i)).unwrap())
        .collect();
    for (i, n) in nodes.iter().enumerate() {
        assert_eq!(w.commit_lookup(&oid(i)), Some(*n));
        assert_eq!(n.oid(), oid(i));
    }
}

#[test]
fn still_interesting_rules() {
    let _g = test_support::init();
    let g = Graph::new(&[30, 20, 10], &[&[], &[], &[]]);
    let l = |ix: &[usize]| ix.iter().map(|&i| g.nodes[i]).collect::<VecDeque<_>>();
    assert_eq!(still_interesting(&l(&[]), 5, 3), 0);
    // an older-or-equal source time than the head: reset to SLOP
    assert_eq!(still_interesting(&l(&[1]), 20, 3), SLOP);
    // an interesting commit in the list
    assert_eq!(still_interesting(&l(&[1, 2]), 25, 3), SLOP);
    g.nodes[1].set_uninteresting(true);
    g.nodes[2].set_uninteresting(true);
    assert_eq!(still_interesting(&l(&[1, 2]), 25, 3), 2);
}

/// What a re-entrant hide callback does on its first call.
#[derive(Clone, Copy)]
enum Reenter {
    FirstParent,
    ReplaceCb,
    Reset,
    SortTopo,
    PushNode(usize),
}

struct ReState {
    walk: *mut GitRevwalk,
    nodes: *const Vec<NodePtr>,
    what: Reenter,
    calls: core::cell::Cell<usize>,
    second: core::cell::Cell<usize>,
}

/// The replacement callback: hides node 1, counts its calls.
unsafe extern "C" fn second_cb(id: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: the walk passes a node's id and our `ReState`.
    let st = unsafe { &*payload.cast::<ReState>() };
    st.second.set(st.second.get() + 1);
    // SAFETY: as above.
    c_int::from(unsafe { *id } == oid(1))
}

/// A callback that calls back into the walk, through its exports, on its
/// first call (as a C user may).
unsafe extern "C" fn reentrant_cb(_id: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: our `ReState`, which outlives the walk.
    let st = unsafe { &*payload.cast::<ReState>() };
    st.calls.set(st.calls.get() + 1);
    if st.calls.get() == 1 {
        // SAFETY: a live walk; the exports support being called from its
        // hide callback (no borrow of the walk's state lives across it).
        unsafe {
            match st.what {
                Reenter::FirstParent => {
                    assert_eq!(ffi::git_revwalk_simplify_first_parent(st.walk), 0);
                }
                Reenter::ReplaceCb => {
                    let e = ffi::git_revwalk_add_hide_cb(st.walk, Some(second_cb), payload);
                    assert_eq!(e, 0);
                }
                Reenter::Reset => assert_eq!(ffi::git_revwalk_reset(st.walk), 0),
                Reenter::SortTopo => {
                    assert_eq!(ffi::git_revwalk_sorting(st.walk, GIT_SORT_TOPOLOGICAL), 0);
                }
                Reenter::PushNode(i) => {
                    let w = &*st.walk;
                    let n = (&*st.nodes)[i];
                    w.push_node(w.state(), n, &PushOptions::default()).unwrap();
                }
            }
        }
    }
    0
}

fn reentrant_walk(what: Reenter, sort: c_uint) -> (Vec<usize>, usize, usize) {
    let g = testrepo();
    let st = ReState {
        walk: ptr::from_ref(&*g.w).cast_mut(),
        nodes: &raw const g.nodes,
        what,
        calls: core::cell::Cell::new(0),
        second: core::cell::Cell::new(0),
    };
    let payload = ptr::from_ref(&st).cast_mut().cast::<c_void>();
    g.w.set_hide_cb(g.st(), Some(reentrant_cb), payload);
    g.sort(sort);
    let out = g.push(0).walk();
    (out, st.calls.get(), st.second.get())
}

#[test]
fn reentrant_hide_callbacks() {
    let _g = test_support::init();
    // first-parent from the callback takes effect at once: 1 is not added
    let (out, calls, _) = reentrant_walk(Reenter::FirstParent, 0);
    assert_eq!(out, [0, 3, 5, 4]);
    assert!(calls > 1);
    // the replacement callback is used from its next call (and hides 1);
    // the first one is not called again
    let (out, calls, second) = reentrant_walk(Reenter::ReplaceCb, 0);
    assert_eq!(out, [0, 3, 5, 4]);
    assert_eq!(calls, 1);
    assert!(second > 0);
    // a sorting set from the callback is the one prepare_walk sees after
    // limit_list
    let (out, _, _) = reentrant_walk(Reenter::SortTopo, 0);
    assert_eq!(out, [0, 1, 2, 3, 5, 4]);
    // a reset from the callback (inside limit_list): what the C gives
    let (out, _, _) = reentrant_walk(Reenter::Reset, 0);
    assert_eq!(out, [0, 3, 1, 2, 5, 4]);
    for sort in 0..8 {
        let (out, _, _) = reentrant_walk(Reenter::Reset, sort);
        assert!(!out.is_empty(), "sort {sort}");
    }
    // a push from the callback: into user_input, read at the next prepare
    let (out, _, _) = reentrant_walk(Reenter::PushNode(2), 0);
    assert_eq!(out, [0, 3, 1, 2, 5, 4]);
}

/// Without `link-c`, the C that the walk can reach is absent: these
/// definitions stand in for it in this test binary (never in the library),
/// so that it links. The tests' nodes are parsed and no pathspec is set,
/// so none is ever called.
///
/// Stand-ins for the C that stays (diff, pathspec) and for l-obj's
/// object_api.c (PENDING(l-obj)) when the C rest is not linked.
#[cfg(not(feature = "link-c"))]
mod c_stubs {
    use core::ffi::{c_char, c_int};

    use crate::oid::Oid;
    use crate::sys::diff::{GitDiffDelta as git_diff_delta, git_diff, git_diff_options};
    use crate::sys::pending::l_revwalk::{git_commit, git_repository, git_tree};
    use crate::util::vector::GitVector;

    macro_rules! stubs {
        ($(fn $name:ident($($a:ident: $t:ty),*) -> $r:ty;)*) => {$(
            #[unsafe(no_mangle)]
            extern "C" fn $name($(_: $t),*) -> $r {
                unreachable!(concat!(stringify!($name), " is C: not linked"))
            }
        )*};
    }

    stubs! {
        fn git_commit_lookup(o: *mut *mut git_commit, r: *mut git_repository, i: *const Oid) -> c_int;
        fn git_commit_free(c: *mut git_commit) -> ();
        fn git_tree_free(t: *mut git_tree) -> ();
        fn git_diff_tree_to_tree(d: *mut *mut git_diff, r: *mut git_repository, a: *mut git_tree, b: *mut git_tree, o: *const git_diff_options) -> c_int;
        fn git_diff_num_deltas(d: *const git_diff) -> usize;
        fn git_diff_get_delta(d: *const git_diff, i: usize) -> *const git_diff_delta;
        fn git_diff_free(d: *mut git_diff) -> ();
        fn git_pathspec__match(v: *const GitVector, p: *const c_char, a: bool, b: bool, m: *mut *const c_char, at: *mut usize) -> bool;
    }
}
