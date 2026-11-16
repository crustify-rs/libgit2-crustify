//! libgit2-rs as a dependent crate uses it (review api-r1 A-3, A-4, S-4):
//! only the public API, linked through what cargo passes on from the build
//! script (this crate has no build script of its own), and no
//! initialization call anywhere: the library initializes itself.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use libgit2_rs::{
    Backend, Commit, Config, ConfigLevel, Index, IndexEntry, ObjectType, Odb, Oid, OidType,
    Repository, Revwalk, Signature, Sort, Time, Tree,
};

fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources")
}

fn testrepo() -> PathBuf {
    resources().join("testrepo.git")
}

const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";

fn oid(s: &str) -> Oid {
    s.parse().unwrap()
}

/// What a consumer does first: open, read HEAD, walk the history.
#[test]
#[cfg_attr(miri, ignore)]
fn open_walk_read() {
    let repo = Repository::open(testrepo()).unwrap();
    let head = repo.head().unwrap();
    assert_eq!(head.target(), Some(oid(HEAD)));
    let c = Commit::lookup(&repo, &oid(HEAD)).unwrap();
    assert_eq!(c.id(), &oid(HEAD));
    let tree: Tree<'_> = c.tree().unwrap();
    assert!(!tree.is_empty());
    let mut walk = Revwalk::new(&repo).unwrap();
    walk.sorting(Sort::TOPOLOGICAL | Sort::TIME);
    walk.push(&oid(HEAD)).unwrap();
    let n = walk.map(|r| r.unwrap()).count();
    assert!(n > 1, "{n}");
    let cfg = repo.config().unwrap();
    assert!(cfg.get_bool("core.bare").unwrap());
}

/// Objects written in memory, a commit built and parsed back.
#[test]
#[cfg_attr(miri, ignore)]
fn write_in_memory() {
    let odb = Odb::new().unwrap();
    let (backend, mempack) = Backend::mempack();
    odb.add_backend(backend, 1).unwrap();
    let repo = Repository::wrap_odb(odb).unwrap();
    let blob = repo
        .odb()
        .unwrap()
        .write(b"hello\n", ObjectType::Blob)
        .unwrap();
    assert!(mempack.contains(&blob));
    let index = Index::new().unwrap();
    index
        .add(&IndexEntry::new("hello.txt", 0o100644, blob))
        .unwrap();
    let tree = index.write_tree_to(&repo).unwrap();
    let sig = Signature::new("A U Thor", "author@example.com", Time::new(1234567890, 60)).unwrap();
    let commit = libgit2_rs::CommitBuilder::new(tree, &sig, &sig, b"message\n")
        .write(&repo)
        .unwrap();
    let c = Commit::lookup(&repo, &commit).unwrap();
    assert_eq!(c.tree_id(), &tree);
    assert_eq!(c.author().name(), b"A U Thor");
    assert_eq!(c.author().when().offset_minutes(), 60);
}

/// A standalone configuration and odb (owners made without a repository).
#[test]
#[cfg_attr(miri, ignore)]
fn standalone_owners() {
    let cfg = Config::new();
    cfg.add_memory(b"[a]\n\tb = 7\n", ConfigLevel::App, false)
        .unwrap();
    assert_eq!(cfg.get_i32("a.b").unwrap(), 7);
    let odb = Odb::open(testrepo().join("objects"), OidType::Sha1).unwrap();
    let o = odb.read(&oid(HEAD)).unwrap();
    assert_eq!(o.kind(), ObjectType::Commit);
}

/// The constructors, each the first thing a fresh process does (S-4: none
/// may fail because the library was not initialized). The test re-runs
/// this binary once per case with `CONSUMER_FIRST_CALL` set; the child runs
/// only `first_call_child`.
#[test]
#[cfg_attr(miri, ignore)]
fn first_call_in_a_fresh_process() {
    if std::env::var_os("CONSUMER_FIRST_CALL").is_some() {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    for case in CASES {
        let out = Command::new(&exe)
            .args([
                "--exact",
                "first_call_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("CONSUMER_FIRST_CALL", case)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

const CASES: &[&str] = &[
    "repository_open",
    "odb_new",
    "odb_open",
    "index_new",
    "config_new",
    "signature_now",
    "commit_parse",
    "tree_parse",
    "backend_mempack",
    "oid_from_hex",
];

#[test]
#[cfg_attr(miri, ignore)]
fn first_call_child() {
    let Some(case) = std::env::var_os("CONSUMER_FIRST_CALL") else {
        return;
    };
    let commit = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
        author a <b> 1 +0000\ncommitter a <b> 1 +0000\n\nm\n";
    match case.to_str().unwrap() {
        "repository_open" => drop(Repository::open(testrepo()).unwrap()),
        "odb_new" => {
            let odb = Odb::new().unwrap();
            let (b, _m) = Backend::mempack();
            odb.add_backend(b, 1).unwrap();
            odb.write(b"x", ObjectType::Blob).unwrap();
        }
        "odb_open" => {
            let odb = Odb::open(testrepo().join("objects"), OidType::Sha1).unwrap();
            odb.read(&oid(HEAD)).unwrap();
        }
        "index_new" => {
            let ix = Index::new().unwrap();
            ix.add(&IndexEntry::new("a", 0o100644, oid(HEAD))).unwrap();
        }
        "config_new" => {
            let c = Config::new();
            c.add_memory(b"[a]\nb = 1\n", ConfigLevel::Local, false)
                .unwrap();
            assert_eq!(c.get_i32("a.b").unwrap(), 1);
        }
        "signature_now" => drop(Signature::now("a", "b@c").unwrap()),
        "commit_parse" => drop(Commit::parse(commit, OidType::Sha1).unwrap()),
        "tree_parse" => drop(Tree::parse(b"", OidType::Sha1).unwrap()),
        "backend_mempack" => drop(Backend::mempack()),
        "oid_from_hex" => drop(Oid::from_hex(HEAD, OidType::Sha1).unwrap()),
        other => panic!("unknown case {other}"),
    }
}
