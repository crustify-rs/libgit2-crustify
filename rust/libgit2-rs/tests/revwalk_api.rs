//! The safe API of the revision walker and of similarity signatures, on
//! the test repositories (root items only, no `unsafe`, no init).
//! Expected orders are the C reference's.

#![cfg(all(feature = "link-c", not(miri)))]
#![forbid(unsafe_code)]

use std::path::PathBuf;

use libgit2_rs::Repository;
use libgit2_rs::{ErrorClass, ErrorCode, HashSig, HashSigOptions, Oid, Revwalk, Sort};

fn resources() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn testrepo() -> Repository {
    Repository::open(resources().join("testrepo.git")).unwrap()
}

/// The ids of a walk, abbreviated to 7 digits.
fn short(walk: &mut Revwalk<'_>) -> Vec<String> {
    walk.map(|id| id.unwrap().to_string()[..7].to_string())
        .collect()
}

const HEAD_ORDER: [&str; 7] = [
    "a65fedf", "be3563a", "c47800c", "9fd738e", "4a202b3", "5b5b025", "8496071",
];

#[test]
fn every_sorting_mode() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    let mut rev = HEAD_ORDER;
    rev.reverse();
    for bits in 0..8 {
        let sort = Sort::from_bits(bits).unwrap();
        walk.sorting(sort).push_head().unwrap();
        let want: &[&str] = if sort.contains(Sort::REVERSE) {
            &rev
        } else {
            &HEAD_ORDER
        };
        assert_eq!(short(&mut walk), want, "sort {bits}");
    }
    walk.simplify_first_parent().push_head().unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "a65fedf", "be3563a", "9fd738e", "4a202b3", "5b5b025", "8496071"
        ]
    );
}

#[test]
fn globs_hides_and_ranges() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.push_glob("heads").unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "41bc8c6", "5001298", "8496071", "e90810b", "6dcf9bf", "9fd738e", "763d71a", "c47800c",
            "4a202b3", "a65fedf", "258f0e2", "a4a7dce", "be3563a", "5b5b025"
        ]
    );
    walk.sorting(Sort::TIME).push_glob("tags").unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "a65fedf", "e90810b", "6dcf9bf", "be3563a", "c47800c", "9fd738e", "4a202b3", "5b5b025",
            "8496071"
        ]
    );
    walk.push_head()
        .unwrap()
        .hide_ref("refs/heads/br2")
        .unwrap();
    assert_eq!(short(&mut walk), ["a65fedf", "be3563a"]);
    walk.push_range("be3563a..HEAD").unwrap();
    assert_eq!(short(&mut walk), ["a65fedf"]);
    walk.sorting(Sort::TOPOLOGICAL)
        .push_glob("heads")
        .unwrap()
        .hide_ref("refs/heads/packed-test")
        .unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "41bc8c6", "5001298", "e90810b", "6dcf9bf", "763d71a", "a65fedf", "258f0e2", "be3563a",
            "a4a7dce", "9fd738e", "c47800c"
        ]
    );
}

#[test]
fn push_and_hide_ids() {
    let repo = testrepo();
    let head = Oid::from_hex(
        b"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        libgit2_rs::OidType::Sha1,
    )
    .unwrap();
    let c4 = Oid::from_hex(
        b"c47800c7266a2be04c571c04d5a6614691ea99bd",
        libgit2_rs::OidType::Sha1,
    )
    .unwrap();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.push(&head).unwrap().hide(&c4).unwrap();
    assert_eq!(
        short(&mut walk),
        ["a65fedf", "be3563a", "9fd738e", "4a202b3"]
    );
    // the walk is over and reset: nothing pushed
    assert!(walk.next().is_none());
}

#[test]
fn errors() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    let e = walk.push_range("HEAD~1...HEAD").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(
        e.message(),
        "symmetric differences not implemented in revwalk"
    );
    let e = walk.push_range("HEAD").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.message(), "invalid revspec: range not provided");
    let e = walk.push_ref("refs/heads/nope").unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.message(), "reference 'refs/heads/nope' not found");
    let tree = Oid::from_hex(
        b"944c0f6e4dfa41595e6eb3ceecdb14f50fe18162",
        libgit2_rs::OidType::Sha1,
    )
    .unwrap();
    assert!(walk.push(&tree).is_err());
    let e = walk.push_ref("refs/heads/a\0b").unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
    // the walk still works
    walk.push_head().unwrap();
    assert_eq!(short(&mut walk).len(), 7);
}

#[test]
fn hide_callback() {
    let mut asked = Vec::new();
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.set_hide_callback(|id| {
        asked.push(*id);
        id.to_string().starts_with("c47800c")
    });
    walk.push_head().unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "a65fedf", "be3563a", "9fd738e", "4a202b3", "5b5b025", "8496071"
        ]
    );
    walk.clear_hide_callback();
    drop(walk);
    assert!(!asked.is_empty());
}

#[test]
fn hide_callback_panic_is_resumed() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    let mut calls = 0;
    walk.set_hide_callback(move |_| {
        calls += 1;
        if calls == 2 {
            panic!("from the hide callback");
        }
        false
    });
    walk.push_head().unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| walk.next()));
    let p = r.unwrap_err();
    assert_eq!(p.downcast_ref::<&str>(), Some(&"from the hide callback"));
    // the walk was reset: nothing is pushed any more
    assert!(walk.next().is_none());
    // and it works again
    walk.clear_hide_callback().push_head().unwrap();
    assert_eq!(short(&mut walk), HEAD_ORDER);
    assert!(std::ptr::eq(walk.repository(), &raw const repo));
}

#[test]
fn sort_flags() {
    assert_eq!(Sort::default(), Sort::NONE);
    assert!(Sort::NONE.is_empty());
    let s = Sort::TOPOLOGICAL | Sort::TIME;
    assert!(s.contains(Sort::TIME) && !s.contains(Sort::REVERSE));
    assert_eq!(s.bits(), 3);
    assert_eq!(Sort::all().bits(), 7);
    assert_eq!(Sort::from_bits(8), None);
    assert_eq!(Sort::from_bits_truncate(9), Sort::TOPOLOGICAL);
    assert_eq!(s - Sort::TIME, Sort::TOPOLOGICAL);
    assert_eq!(format!("{s:?}"), "Sort(TOPOLOGICAL | TIME)");
    let mut m = Sort::NONE;
    m |= Sort::REVERSE;
    assert_eq!(!m, Sort::TOPOLOGICAL | Sort::TIME);
}

#[test]
fn reset_forgets_everything() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.sorting(Sort::REVERSE)
        .simplify_first_parent()
        .push_head()
        .unwrap();
    walk.reset().push_head().unwrap();
    assert_eq!(short(&mut walk), HEAD_ORDER);
    walk.push_head().unwrap().hide_head().unwrap();
    assert!(walk.next().is_none());
    walk.push_glob("heads/m*")
        .unwrap()
        .hide_ref("refs/heads/br2")
        .unwrap();
    assert_eq!(short(&mut walk), ["a65fedf", "be3563a"]);
    // Everything is reachable from a tag.
    walk.push_head().unwrap().hide_glob("tags").unwrap();
    assert!(walk.next().is_none());
}

#[test]
fn byte_names() {
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.push_ref(b"refs/heads/master").unwrap();
    assert_eq!(short(&mut walk), HEAD_ORDER);
    let e = walk.push_ref(b"refs/heads/\xff").unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
}

#[test]
fn hashsig_api() {
    let dir = resources().join("renames");
    let a = HashSig::from_file(dir.join("songof7cities.txt"), HashSigOptions::NORMAL).unwrap();
    let b = HashSig::from_file(dir.join("untimely.txt"), HashSigOptions::NORMAL).unwrap();
    assert_eq!(a.compare(&a), 100);
    let ab = a.compare(&b);
    assert!((0..100).contains(&ab));
    assert_eq!(b.compare(&a), b.compare(&a));
    let data = std::fs::read(dir.join("songof7cities.txt")).unwrap();
    let m = HashSig::new(&data, HashSigOptions::NORMAL).unwrap();
    assert_eq!(m.compare(&a), 100);
    let e = HashSig::new(b"x\n", HashSigOptions::NORMAL).unwrap_err();
    assert_eq!(e.code(), ErrorCode::BUFS);
    assert!(HashSig::new(b"x\n", HashSigOptions::ALLOW_SMALL_FILES).is_ok());
    let e = HashSig::from_file(dir.join("does-not-exist"), HashSigOptions::NORMAL).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let ws = HashSig::new(&data, HashSigOptions::IGNORE_WHITESPACE).unwrap();
    assert_eq!(ws.compare(&ws), 100);
    let smart = HashSig::new(&data, HashSigOptions::SMART_WHITESPACE).unwrap();
    assert!(smart.compare(&m) <= 100);
    assert_eq!(HashSigOptions::default(), HashSigOptions::NORMAL);
    assert_eq!(HashSigOptions::from_bits(8), None);
    let both = HashSigOptions::IGNORE_WHITESPACE | HashSigOptions::SMART_WHITESPACE;
    assert_eq!(
        HashSig::new(&data, both).unwrap_err().class(),
        ErrorClass::INTERNAL
    );
}
