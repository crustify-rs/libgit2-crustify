//! Regression tests of review api-r1 (`/work/reviews/api-r1.md`) that are
//! not a component's: S-4 (the library initializes itself, and no owner
//! outlives it), S-5 (readers racing a writer of HEAD), I-1 (the index and
//! the repository used together, from several threads). The components'
//! own regressions (S-2, S-3, S-6*) are in their test files; S-1 is the
//! `compile_fail` doctests of `src/lib.rs`.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use libgit2_rs::{
    Commit, IndexEntry, Libgit2, Object, ObjectType, Odb, Oid, OidType, Repository, Stage,
};

const HEAD_COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";

fn testrepo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources/testrepo.git")
}

fn oid(s: &str) -> Oid {
    s.parse().unwrap()
}

/// A writable copy of testrepo.git.
fn copy_repo(name: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("api-r1-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    let st = std::process::Command::new("cp")
        .arg("-r")
        .arg(testrepo())
        .arg(&dst)
        .status()
        .unwrap();
    assert!(st.success());
    dst
}

/// S-4 (`odb_outlives_library_guard`): the reviewer's odb with a pack
/// backend outlived the last `Libgit2` guard, whose shutdown disposed the
/// pack cache under it (a leak, under LSan). The safe API never shuts the
/// library down: the odb still reads after the "guard" is gone.
#[test]
#[cfg_attr(miri, ignore)]
fn odb_outlives_libgit2_value() {
    let g = Libgit2::init().unwrap();
    let odb = Odb::open(testrepo().join("objects"), OidType::Sha1).unwrap();
    let o = odb.read(&oid(HEAD_COMMIT)).unwrap();
    assert_eq!(o.kind(), Some(ObjectType::Commit));
    drop(o);
    drop(g);
    let o = odb.read(&oid(HEAD_COMMIT)).unwrap();
    assert!(o.data().starts_with(b"tree "));
}

/// S-5 (`shared_repository_threads_move_head`): readers of HEAD failed
/// spuriously ("could not read (expected 41 bytes, read 23)", "corrupted
/// loose reference file: HEAD") while another thread moved it, because
/// the size came from a `stat` of the path and the bytes from the file
/// opened after it.
#[test]
#[cfg_attr(miri, ignore)]
fn readers_while_head_moves() {
    let path = copy_repo("head-moves");
    let repo = Repository::open(&path).unwrap();
    let head = oid(HEAD_COMMIT);
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                for _ in 0..300 {
                    let r = repo.head().unwrap();
                    assert_eq!(r.target_peel().or(r.target()), Some(head));
                    let o = Object::lookup(&repo, &head, None).unwrap();
                    assert_eq!(o.kind(), ObjectType::Commit);
                    let c = Commit::lookup(&repo, &head).unwrap();
                    let _ = c.tree().unwrap();
                }
            });
        }
        s.spawn(|| {
            for i in 0..300 {
                if i % 2 == 0 {
                    repo.set_head_detached(&head).unwrap();
                } else {
                    repo.set_head("refs/heads/master").unwrap();
                }
            }
        });
    });
    drop(repo);
    let _ = std::fs::remove_dir_all(&path);
}

/// I-1: the repository's index and its objects and references are used
/// together, and the index from several threads at once.
#[test]
#[cfg_attr(miri, ignore)]
fn index_with_repository_across_threads() {
    let path = copy_repo("index-threads");
    let repo = Repository::open(&path).unwrap();
    let index = repo.index().unwrap();
    let head = repo.head().unwrap();
    let commit = Commit::lookup(&repo, &head.target().unwrap()).unwrap();
    index.read_tree(&commit.tree().unwrap()).unwrap();
    let base = index.len();
    std::thread::scope(|s| {
        for t in 0..4 {
            let index = &index;
            let repo = &repo;
            s.spawn(move || {
                let blob = commit_blob(repo);
                for i in 0..50 {
                    index
                        .add(&IndexEntry::new(format!("t{t}/f{i}"), 0o100644, blob))
                        .unwrap();
                    assert!(
                        index
                            .get_path(format!("t{t}/f{i}"), Stage::Normal)
                            .is_some()
                    );
                    let _ = repo.head().unwrap();
                }
            });
        }
        s.spawn(|| {
            for _ in 0..50 {
                let _ = index.iter().count();
            }
        });
    });
    assert_eq!(index.len(), base + 200);
    let tree = index.write_tree().unwrap();
    let t = libgit2_rs::Tree::lookup(&repo, &tree).unwrap();
    assert!(t.get_path(b"t3/f49").is_ok());
    drop(t);
    drop(index);
    drop(commit);
    drop(head);
    drop(repo);
    let _ = std::fs::remove_dir_all(&path);
}

fn commit_blob(repo: &Repository) -> Oid {
    repo.odb()
        .unwrap()
        .write(b"content\n", ObjectType::Blob)
        .unwrap()
}
