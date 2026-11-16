//! The safe API of the revision walker and of similarity signatures, on
//! the test repositories. Expected orders are the C reference's.

#![cfg(all(feature = "link-c", not(miri)))]
#![forbid(unsafe_code)]

use std::path::PathBuf;

use libgit2_rs::revwalk::api::Repository;
use libgit2_rs::util::test_support;
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
    let _g = test_support::init();
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    let mut rev = HEAD_ORDER;
    rev.reverse();
    for bits in 0..8 {
        let sort = Sort::from_bits(bits);
        walk.sorting(sort).unwrap().push_head().unwrap();
        let want: &[&str] = if sort.contains(Sort::REVERSE) {
            &rev
        } else {
            &HEAD_ORDER
        };
        assert_eq!(short(&mut walk), want, "sort {bits}");
    }
    walk.simplify_first_parent().unwrap().push_head().unwrap();
    assert_eq!(
        short(&mut walk),
        [
            "a65fedf", "be3563a", "9fd738e", "4a202b3", "5b5b025", "8496071"
        ]
    );
}

#[test]
fn globs_hides_and_ranges() {
    let _g = test_support::init();
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
    walk.sorting(Sort::TIME).unwrap().push_glob("tags").unwrap();
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
        .unwrap()
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
    let _g = test_support::init();
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
    let _g = test_support::init();
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
    let _g = test_support::init();
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
#[should_panic(expected = "from the hide callback")]
fn hide_callback_panic_resumes() {
    let _g = test_support::init();
    let repo = testrepo();
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.set_hide_callback(|_| panic!("from the hide callback"));
    walk.push_head().unwrap();
    let _ = walk.next();
}

#[test]
fn hashsig_api() {
    let _g = test_support::init();
    let dir = resources().join("renames");
    let a = HashSig::from_file(&dir.join("songof7cities.txt"), HashSigOptions::NORMAL).unwrap();
    let b = HashSig::from_file(&dir.join("untimely.txt"), HashSigOptions::NORMAL).unwrap();
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
    let e = HashSig::from_file(&dir.join("does-not-exist"), HashSigOptions::NORMAL).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    let both = HashSigOptions::IGNORE_WHITESPACE | HashSigOptions::SMART_WHITESPACE;
    assert_eq!(
        HashSig::new(&data, both).unwrap_err().class(),
        ErrorClass::INTERNAL
    );
}
