//! The exports of `revwalk.c` and `hashsig.c`, called as C calls them.

#![cfg(all(feature = "link-c", not(miri)))]
// The tests call the exports in sequences, as C does; the SAFETY comment of
// each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{c_char, c_int, c_void};
use core::ptr;

use libgit2_rs::hashsig::ffi as hs;
use libgit2_rs::revwalk::ffi as rw;
use libgit2_rs::revwalk::node::{CommitListNode, PARSED};
use libgit2_rs::revwalk::{GIT_SORT_TIME, GitRevwalk, PushOptions};
use libgit2_rs::sys::pending::l_revwalk as p;
use libgit2_rs::util::errors;
use libgit2_rs::util::test_support;
use libgit2_rs::{Oid, OidType};

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

fn open() -> *mut p::git_repository {
    let path = format!(
        "{}/../../tests/resources/testrepo.git\0",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut r = ptr::null_mut();
    // SAFETY: a writable out-parameter and a string.
    let e = unsafe { p::git_repository_open(&mut r, path.as_ptr().cast()) };
    assert_eq!(e, 0);
    r
}

fn hex(s: &str) -> Oid {
    Oid::from_hex(s.as_bytes(), OidType::Sha1).unwrap()
}

fn drain(w: *mut GitRevwalk) -> Vec<String> {
    let mut out = Vec::new();
    let mut id = Oid::NONE;
    loop {
        // SAFETY: a live walk, a writable oid.
        let e = unsafe { rw::git_revwalk_next(&mut id, w) };
        if e != 0 {
            break;
        }
        out.push(id.to_string()[..7].to_string());
    }
    out
}

#[test]
fn walk_lifecycle_and_arguments() {
    let _g = test_support::init();
    let repo = open();
    // SAFETY: as C calls them.
    unsafe {
        let mut w = ptr::null_mut();
        assert_eq!(rw::git_revwalk_new(&mut w, repo), 0);
        assert_eq!(rw::git_revwalk_repository(w), repo);
        assert_eq!((*w).prefix.repo, repo);
        assert!(!(*w).prefix.odb.is_null());

        // nothing pushed
        let mut id = Oid::NONE;
        assert_eq!(rw::git_revwalk_next(&mut id, w), -31);
        assert!(errors::last_message().is_none());

        assert_eq!(rw::git_revwalk_push_head(w), 0);
        assert_eq!(rw::git_revwalk_sorting(w, GIT_SORT_TIME), 0);
        // sorting before walking keeps the pushes
        assert_eq!(drain(w).len(), 7);

        // arguments
        assert_eq!(rw::git_revwalk_next(ptr::null_mut(), w), -1);
        assert_eq!(msg(), "invalid argument: 'oid'");
        assert_eq!(rw::git_revwalk_next(&mut id, ptr::null_mut()), -1);
        assert_eq!(msg(), "invalid argument: 'walk'");
        assert_eq!(rw::git_revwalk_push(w, ptr::null()), -1);
        assert_eq!(msg(), "invalid argument: 'oid'");
        assert_eq!(rw::git_revwalk_push_ref(w, ptr::null()), -1);
        assert_eq!(msg(), "invalid argument: 'refname'");
        assert_eq!(rw::git_revwalk_push_glob(w, ptr::null()), -1);
        assert_eq!(msg(), "invalid argument: 'glob'");
        assert_eq!(rw::git_revwalk_sorting(ptr::null_mut(), 0), -1);
        assert_eq!(rw::git_revwalk_reset(ptr::null_mut()), -1);
        assert_eq!(
            rw::git_revwalk_add_hide_cb(ptr::null_mut(), None, ptr::null_mut()),
            -1
        );
        assert_eq!(
            rw::git_revwalk_pathspec(ptr::null_mut(), ptr::null_mut()),
            -1
        );
        assert!(rw::git_revwalk_repository(ptr::null_mut()).is_null());
        assert_eq!(msg(), "invalid argument: 'walk'");

        // errors of pushes
        assert_eq!(
            rw::git_revwalk_push_range(w, c"HEAD~1...HEAD".as_ptr()),
            -12
        );
        assert_eq!(msg(), "symmetric differences not implemented in revwalk");
        assert_eq!(rw::git_revwalk_push_range(w, c"HEAD".as_ptr()), -12);
        assert_eq!(msg(), "invalid revspec: range not provided");
        let tree = hex("944c0f6e4dfa41595e6eb3ceecdb14f50fe18162");
        assert_eq!(rw::git_revwalk_push(w, &tree), -12);
        assert_eq!(msg(), "object is not a committish");
        assert_eq!(rw::git_revwalk_push_ref(w, c"refs/heads/nope".as_ptr()), -1);
        assert_eq!(msg(), "reference 'refs/heads/nope' not found");

        rw::git_revwalk_free(w);
        rw::git_revwalk_free(ptr::null_mut());

        // a NULL repository: git_repository_odb's argument check, and -1
        let mut w2 = std::ptr::dangling_mut::<GitRevwalk>();
        assert_eq!(rw::git_revwalk_new(&mut w2, ptr::null_mut()), -1);
        assert_eq!(msg(), "invalid argument: 'repo'");
        assert_eq!(w2, std::ptr::dangling_mut::<GitRevwalk>());
        p::git_repository_free(repo);
    }
}

unsafe extern "C" fn hide_c4(id: *const Oid, payload: *mut c_void) -> c_int {
    // SAFETY: a node's id and our counter.
    unsafe {
        *payload.cast::<usize>() += 1;
        c_int::from((*id).to_string().starts_with("c47800c"))
    }
}

#[test]
fn hide_cb_and_internal_pushes() {
    let _g = test_support::init();
    let repo = open();
    // SAFETY: as C calls them.
    unsafe {
        let mut w = ptr::null_mut();
        assert_eq!(rw::git_revwalk_new(&mut w, repo), 0);
        let mut calls = 0usize;
        assert_eq!(
            rw::git_revwalk_add_hide_cb(w, Some(hide_c4), (&raw mut calls).cast()),
            0
        );
        assert_eq!(rw::git_revwalk_push_head(w), 0);
        assert_eq!(
            drain(w),
            [
                "a65fedf", "be3563a", "9fd738e", "4a202b3", "5b5b025", "8496071"
            ]
        );
        assert!(calls > 0);

        // fetch negotiation's push: every ref, by date, into user_input
        let opts = PushOptions {
            insert_by_date: 1,
            ..PushOptions::default()
        };
        assert_eq!(rw::git_revwalk__push_glob(w, c"refs/*".as_ptr(), &opts), 0);
        let mut times = Vec::new();
        let mut l = (*w).prefix.user_input.get();
        while !l.is_null() {
            times.push((*(*l).item).time);
            l = (*l).next;
        }
        assert!(times.len() > 5);
        assert!(times.windows(2).all(|t| t[0] >= t[1]), "{times:?}");
        assert_eq!(rw::git_revwalk_reset(w), 0);
        assert!((*w).prefix.user_input.get().is_null());

        // from_glob skips what is missing
        let opts = PushOptions {
            from_glob: 1,
            ..PushOptions::default()
        };
        assert_eq!(
            rw::git_revwalk__push_ref(w, c"refs/heads/nope".as_ptr(), &opts),
            0
        );
        assert_eq!(
            rw::git_revwalk__push_glob(w, c"tags".as_ptr(), ptr::null()),
            0
        );
        let head = hex("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
        assert_eq!(
            rw::git_revwalk__push_commit(w, &head, &PushOptions::default()),
            0
        );
        assert!(!drain(w).is_empty());
        rw::git_revwalk_free(w);
        p::git_repository_free(repo);
    }
}

#[test]
fn commit_nodes_are_cs() {
    let _g = test_support::init();
    let repo = open();
    // SAFETY: as C calls them.
    unsafe {
        let mut w = ptr::null_mut();
        assert_eq!(rw::git_revwalk_new(&mut w, repo), 0);
        let head = hex("a65fedf39aefe402d3bb6e24df4d4f5fe4547750");
        let n = rw::git_revwalk__commit_lookup(w, &head);
        assert!(!n.is_null());
        assert_eq!(rw::git_revwalk__commit_lookup(w, &head), n);
        assert_eq!((*n).oid, head);
        // C parses it: parents, time, the parsed bit where the Rust reads it
        assert_eq!(p::git_commit_list_parse(w.cast(), n.cast()), 0);
        assert_eq!((*n).bits & PARSED, PARSED);
        assert_eq!((*n).out_degree, 1);
        assert_eq!((*n).time, 1_312_943_626);
        let parent: *mut CommitListNode = *(*n).parents;
        assert_eq!(rw::git_revwalk__commit_lookup(w, &(*parent).oid), parent);
        assert_eq!(
            (*parent).oid.to_string(),
            "be3563ae3f795b2b4353bcce3a527ad0a4f7f644"
        );

        // a node the Rust marks parsed: C does not read the (missing) object
        let bogus = hex("4242424242424242424242424242424242424242");
        let b = rw::git_revwalk__commit_lookup(w, &bogus);
        assert_eq!(p::git_commit_list_parse(w.cast(), b.cast()), -3);
        (*b).bits |= PARSED;
        assert_eq!(p::git_commit_list_parse(w.cast(), b.cast()), 0);

        assert!(rw::git_revwalk__commit_lookup(w, ptr::null()).is_null());
        rw::git_revwalk_free(w);
        p::git_repository_free(repo);
    }
}

#[test]
fn hashsig_exports() {
    let _g = test_support::init();
    let text = b"one\ntwo\nthree\nfour\nfive\nsix\n";
    // SAFETY: as C calls them.
    unsafe {
        let mut a = ptr::null_mut();
        let mut b = ptr::null_mut();
        assert_eq!(
            hs::git_hashsig_create(&mut a, text.as_ptr().cast(), text.len(), 0),
            0
        );
        let path = format!(
            "{}/../../tests/resources/renames/songof7cities.txt\0",
            env!("CARGO_MANIFEST_DIR")
        );
        assert_eq!(
            hs::git_hashsig_create_fromfile(&mut b, path.as_ptr().cast::<c_char>(), 0),
            0
        );
        assert_eq!(hs::git_hashsig_compare(a, a), 100);
        assert_eq!(hs::git_hashsig_compare(a, b), 0);
        hs::git_hashsig_free(a);
        hs::git_hashsig_free(b);
        hs::git_hashsig_free(ptr::null_mut());

        let mut c = ptr::null_mut();
        assert_eq!(hs::git_hashsig_create(&mut c, c"a\n".as_ptr(), 2, 0), -6);
        assert_eq!(msg(), "file too small for similarity signature calculation");
        assert!(c.is_null());
        assert_eq!(
            hs::git_hashsig_create_fromfile(&mut c, c"/nonexistent/x".as_ptr(), 0),
            -3
        );
        assert_eq!(
            msg(),
            "could not find '/nonexistent/x' to open: No such file or directory"
        );
        assert_eq!(
            hs::git_hashsig_create(&mut c, text.as_ptr().cast(), text.len(), 3),
            -1
        );
        assert!(msg().starts_with("unrecoverable internal error"));
    }

    // under an allocation limit: the C's out-of-memory error
    let g = test_support::exclusive();
    let r = test_support::with_alloc_limit(&g, 0, || {
        let mut c = ptr::null_mut();
        // SAFETY: as above.
        unsafe { hs::git_hashsig_create(&mut c, text.as_ptr().cast(), text.len(), 0) }
    });
    assert_eq!(r, -1);
}
