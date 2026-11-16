//! The safe API of the index (WP l-index, WP api), used as a Rust consumer
//! would: root items only, no `unsafe`, no explicit init. The paths an
//! index stores are checked by `path.c` (C); under Miri the index uses a
//! Rust stand-in, so the tests that touch no file run there too. The tests
//! with a repository reach more C (diff, attributes, trees): `link-c`, not
//! Miri.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use libgit2_rs::{
    ErrorClass, ErrorCode, Index, IndexAddOption, IndexCapability, IndexEntry, IndexMatch, Oid,
    OidType, Stage,
};

fn id(c: char, t: OidType) -> Oid {
    Oid::from_hex(c.to_string().repeat(t.hexsize()), t).unwrap()
}

fn entry(path: &str, c: char) -> IndexEntry {
    let mut e = IndexEntry::new(path, 0o100644, id(c, OidType::Sha1));
    e.file_size = 3;
    e.mtime.seconds = 1_700_000_000;
    e
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("l_index_api.{}.{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn add_get_remove() {
    let ix = Index::new().unwrap();
    assert!(ix.is_empty());
    ix.add(&entry("b.txt", 'b')).unwrap();
    ix.add(&entry("a/x.txt", 'a')).unwrap();
    assert_eq!(ix.len(), 2);
    assert_eq!(ix.get(0).unwrap().path, b"a/x.txt");
    let e = ix.get_path(b"b.txt", Stage::Normal).unwrap();
    assert_eq!(e.id, id('b', OidType::Sha1));
    assert_eq!(e.stage(), Stage::Normal);
    // the name length is kept in the flags
    assert_eq!(e.flags & 0xfff, 5);
    assert_eq!(ix.find(b"b.txt"), Some(1));
    assert_eq!(ix.find_prefix(b"a/"), Some(0));
    assert!(ix.get_path(b"nope", Stage::Normal).is_none());
    let all: Vec<_> = ix.iter().map(|e| e.path).collect();
    assert_eq!(all, vec![b"a/x.txt".to_vec(), b"b.txt".to_vec()]);
    ix.remove(b"b.txt", Stage::Normal).unwrap();
    let err = ix.remove(b"b.txt", Stage::Normal).unwrap_err();
    assert_eq!(err.code(), ErrorCode::NOT_FOUND);
    assert_eq!(err.message(), "index does not contain b.txt at stage 0");
    ix.remove_dir(b"a", Stage::Normal).unwrap();
    assert!(ix.is_empty());
}

#[test]
fn invalid_paths_and_modes() {
    let ix = Index::new().unwrap();
    let err = ix.add(&entry(".git/config", 'a')).unwrap_err();
    assert_eq!(err.class(), ErrorClass::INDEX);
    assert_eq!(err.message(), "invalid path: '.git/config'");
    assert!(ix.add(&entry("a/../b", 'a')).is_err());
    assert!(ix.add(&entry("nul\0inside", 'a')).is_err());
    let mut bad = entry("m", 'a');
    bad.mode = 0o100600;
    assert_eq!(ix.add(&bad).unwrap_err().message(), "invalid entry mode");
    assert!(ix.is_empty());
}

#[test]
fn snapshot_iteration_survives_changes() {
    let ix = Index::new().unwrap();
    for i in 0..50 {
        ix.add(&entry(&format!("f{i:02}"), 'c')).unwrap();
    }
    let mut it = ix.iter();
    let first = it.next().unwrap();
    // a clone is the same index: changes show in both
    let other = ix.clone();
    other.clear().unwrap();
    assert!(ix.is_empty());
    // the iterator still yields the entries it was made with
    assert_eq!(first.path, b"f00");
    assert_eq!(it.count(), 49);
}

#[test]
fn conflicts() {
    let ix = Index::new().unwrap();
    ix.add(&entry("c", 'f')).unwrap();
    let (a, o, t) = (entry("c", 'a'), entry("c", 'b'), entry("c", 'd'));
    ix.conflict_add(Some(&a), Some(&o), Some(&t)).unwrap();
    assert!(ix.has_conflicts());
    // the stage-0 entry went
    assert!(ix.get_path(b"c", Stage::Normal).is_none());
    let c = ix.conflict_get(b"c").unwrap();
    assert_eq!(c.ancestor.unwrap().stage(), Stage::Ancestor);
    assert_eq!(c.our.unwrap().id, id('b', OidType::Sha1));
    assert_eq!(c.their.unwrap().stage(), Stage::Theirs);
    assert_eq!(ix.conflicts().count(), 1);
    // resolving moves the conflict to the REUC
    ix.remove_path(b"c").unwrap();
    assert!(!ix.has_conflicts());
    assert_eq!(ix.reuc_len(), 1);
    let r = ix.reuc_get(0).unwrap();
    assert_eq!(r.path, b"c");
    assert_eq!(r.mode, [0o100644; 3]);
    assert_eq!(ix.reuc_find(b"c"), Some(0));
    ix.reuc_clear();
    assert_eq!(ix.reuc_len(), 0);
    assert!(ix.conflict_get(b"c").is_err());
}

#[test]
fn names_extensions_caps_version() {
    let ix = Index::new().unwrap();
    ix.name_add(Some(b"anc"), Some(b"ours"), None).unwrap();
    assert!(ix.name_add(Some(b"one"), None, None).is_err());
    assert_eq!(ix.name_get(0).unwrap().ours.unwrap(), b"ours");
    ix.name_clear();
    assert_eq!(ix.name_len(), 0);

    ix.extension_add(b"ZZZZ", b"data").unwrap();
    assert_eq!(ix.extension(b"ZZZZ").unwrap(), b"data");
    ix.extension_remove(b"ZZZZ").unwrap();
    assert!(ix.extension(b"ZZZZ").is_none());

    assert_eq!(ix.version(), 2);
    assert!(ix.set_version(5).is_err());
    ix.set_version(4).unwrap();
    assert_eq!(ix.caps(), IndexCapability::empty());
    ix.set_caps(IndexCapability::IGNORE_CASE).unwrap();
    ix.add(&entry("Mixed/Case", 'a')).unwrap();
    assert!(ix.get_path(b"mixed/case", Stage::Normal).is_some());
    assert!(ix.caps().contains(IndexCapability::IGNORE_CASE));
    assert!(!ix.caps().contains(IndexCapability::NO_SYMLINKS));
    // no repository: no caps from the owner, and no add_all
    assert_eq!(
        ix.set_caps_from_owner().unwrap_err().message(),
        "cannot access repository to set index caps"
    );
    let e = ix.add_all(["*"], IndexAddOption::empty()).unwrap_err();
    assert_eq!(e.code(), ErrorCode::BARE_REPO);
    assert_eq!(
        e.message(),
        "cannot index add all. This operation is not allowed against bare repositories."
    );
    // nor a tree: C's message
    let e = ix.write_tree().unwrap_err();
    assert_eq!(e.class(), ErrorClass::INDEX);
    assert_eq!(
        e.message(),
        "Failed to write tree. the index file is not backed up by an existing repository"
    );
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn write_read_round_trip_all_versions() {
    for t in [OidType::Sha1, OidType::Sha256] {
        for v in 2..=4 {
            let d = tmpdir(&format!("rt{v}{t:?}"));
            let path = d.join("index");
            let ix = Index::open_with_oid_type(&path, t).unwrap();
            for i in 0..20 {
                let mut e = IndexEntry::new(format!("dir{}/file{i}", i % 3), 0o100644, id('e', t));
                e.file_size = i;
                if i == 7 {
                    e.flags_extended = 1 << 13; // intent to add
                }
                ix.add(&e).unwrap();
            }
            ix.reuc_add(
                b"gone",
                [0o100644, 0, 0o100755],
                [id('1', t), id('2', t), id('3', t)],
            )
            .unwrap();
            ix.extension_add(b"QQQQ", b"kept").unwrap();
            ix.set_version(v).unwrap();
            ix.write().unwrap();
            assert!(!ix.is_dirty());
            let back = Index::open_with_oid_type(&path, t).unwrap();
            let a: Vec<_> = ix.iter().collect();
            let b: Vec<_> = back.iter().collect();
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                // version 4 sets no `EXTENDED` flag before writing, so the
                // extended flags of an entry added without it are not
                // written (as C)
                let fx = if v == 4 { 0 } else { x.flags_extended };
                assert_eq!(
                    (&x.path, x.id, x.file_size, fx),
                    (&y.path, y.id, y.file_size, y.flags_extended)
                );
            }
            assert_eq!(back.reuc_len(), 1);
            assert_eq!(back.extension(b"QQQQ").unwrap(), b"kept");
            assert_eq!(back.checksum(), ix.checksum());
            assert_eq!(back.checksum().len(), t.size());
            // version 3 is written only when an entry needs it; 4 is kept
            assert_eq!(back.version(), if v == 4 { 4 } else { 3 });
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn read_errors_and_in_memory() {
    let mem = Index::new().unwrap();
    assert_eq!(
        mem.read(true).unwrap_err().message(),
        "failed to read index: The index is in-memory only"
    );
    assert_eq!(
        mem.write().unwrap_err().message(),
        "failed to write index: The index is in-memory only"
    );
    let d = tmpdir("bad");
    let path = d.join("index");
    std::fs::write(&path, b"DIRC\0\0\0\x09\0\0\0\0xxxxxxxxxxxxxxxxxxxx").unwrap();
    let err = Index::open(&path).unwrap_err();
    assert_eq!(
        err.message(),
        "invalid data in index - incorrect header version"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn read_index_keeps_stat_data() {
    let a = Index::new().unwrap();
    let mut e = entry("same", 'a');
    e.ino = 42;
    a.add(&e).unwrap();
    a.add(&entry("old", 'a')).unwrap();
    let b = Index::new().unwrap();
    let mut e2 = entry("same", 'a');
    e2.ino = 7;
    b.add(&e2).unwrap();
    b.add(&entry("new", 'b')).unwrap();
    a.read_index(&b).unwrap();
    let paths: Vec<_> = a.iter().map(|e| e.path).collect();
    assert_eq!(paths, vec![b"new".to_vec(), b"same".to_vec()]);
    // the unchanged entry keeps its cached stat data
    assert_eq!(a.get_path(b"same", Stage::Normal).unwrap().ino, 42);
    // a new entry gets none
    assert_eq!(a.get_path(b"new", Stage::Normal).unwrap().file_size, 0);
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn index_of_the_resources() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources");
    let ix = Index::open(root.join("testrepo.git/index")).unwrap();
    assert!(!ix.is_empty());
    let v4 = Index::open(root.join("indexv4/.gitted/index")).unwrap();
    assert_eq!(v4.version(), 4);
    let sha256 = Index::open_with_oid_type(root.join("git-sha256.index"), OidType::Sha256).unwrap();
    assert_eq!(sha256.oid_type(), OidType::Sha256);
    assert!(!sha256.is_empty());
}

#[test]
fn remove_all_with_a_reentrant_closure() {
    let ix = Index::new().unwrap();
    for p in ["a/1", "a/2", "b/1", "keep"] {
        ix.add(&entry(p, 'a')).unwrap();
    }
    let other = ix.clone();
    let mut seen = Vec::new();
    ix.remove_all_with(["a/*", "b/*"], |path, matched| {
        seen.push((path.to_vec(), matched.map(<[u8]>::to_vec)));
        // the closure reads and changes the index under the operation
        assert!(other.len() >= 2);
        assert!(other.get_path(path, Stage::Normal).is_some());
        if path == b"a/1" {
            other.remove(b"b/1", Stage::Normal).unwrap();
        }
        IndexMatch::Apply
    })
    .unwrap();
    let left: Vec<_> = ix.iter().map(|e| e.path).collect();
    assert_eq!(left, vec![b"keep".to_vec()]);
    assert_eq!(seen[0], (b"a/1".to_vec(), Some(b"a/*".to_vec())));
    assert_eq!(seen.len(), 2);

    // Skip keeps an entry, Stop ends the operation with Ok
    for p in ["c/1", "c/2", "c/3"] {
        ix.add(&entry(p, 'a')).unwrap();
    }
    let mut n = 0;
    ix.remove_all_with(["c/*"], |path, _| {
        n += 1;
        match path {
            b"c/1" => IndexMatch::Skip,
            b"c/2" => IndexMatch::Apply,
            _ => IndexMatch::Stop,
        }
    })
    .unwrap();
    assert_eq!(n, 3);
    let left: Vec<_> = ix.iter().map(|e| e.path).collect();
    assert_eq!(
        left,
        vec![b"c/1".to_vec(), b"c/3".to_vec(), b"keep".to_vec()]
    );
    // Stop at once: nothing removed
    ix.remove_all_with(["*"], |_, _| IndexMatch::Stop).unwrap();
    assert_eq!(ix.len(), 3);
    // no callback
    ix.remove_all(["c/*"]).unwrap();
    assert_eq!(ix.len(), 1);
}

#[test]
fn a_panicking_closure_is_resumed() {
    let ix = Index::new().unwrap();
    ix.add(&entry("p", 'a')).unwrap();
    ix.add(&entry("q", 'a')).unwrap();
    let mut calls = 0;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ix.remove_all_with(["*"], |_, _| {
            calls += 1;
            panic!("in the closure")
        })
    }));
    let payload = r.unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"in the closure"));
    // the operation stopped at the first call, and changed nothing
    assert_eq!(calls, 1);
    assert_eq!(ix.len(), 2);
    // the index's lock was let go: another thread uses the index
    std::thread::scope(|s| {
        s.spawn(|| ix.add(&entry("r", 'a')).unwrap());
    });
    assert_eq!(ix.len(), 3);
}

#[test]
fn shared_by_threads() {
    let ix = Index::new().unwrap();
    std::thread::scope(|s| {
        for t in 0..4 {
            let ix = &ix;
            s.spawn(move || {
                for i in 0..50 {
                    ix.add(&entry(&format!("t{t}/{i:02}"), 'a')).unwrap();
                    let _ = ix.get(i);
                    let _: Vec<_> = ix.iter().take(3).collect();
                }
            });
        }
        // a clone moved to another thread
        let c = ix.clone();
        s.spawn(move || {
            for _ in 0..50 {
                let _ = c.find_prefix(b"t1/");
                let _ = c.conflicts().count();
            }
        });
    });
    assert_eq!(ix.len(), 200);
    let paths: Vec<_> = ix.iter().map(|e| e.path).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn reuc_by_path() {
    let ix = Index::new().unwrap();
    let t = OidType::Sha1;
    ix.reuc_add(
        "b",
        [0o100644, 0o100644, 0],
        [id('1', t), id('2', t), Oid::zero(t)],
    )
    .unwrap();
    ix.reuc_add(
        "a",
        [0, 0o100755, 0o100644],
        [Oid::zero(t), id('3', t), id('4', t)],
    )
    .unwrap();
    let r = ix.reuc_get_path("b").unwrap();
    assert_eq!(r.path, b"b");
    assert_eq!(r.mode, [0o100644, 0o100644, 0]);
    assert_eq!(r.oid[1], id('2', t));
    assert_eq!(ix.reuc_get_path(b"a").unwrap().mode[1], 0o100755);
    assert!(ix.reuc_get_path("c").is_none());
    assert_eq!(ix.reuc_find("b"), Some(1));
    ix.reuc_remove(0).unwrap();
    assert!(ix.reuc_get_path("a").is_none());
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn path_of_the_file() {
    let d = tmpdir("path");
    let path = d.join("index");
    let ix = Index::open(&path).unwrap();
    assert_eq!(ix.path(), Some(path));
    assert_eq!(Index::new().unwrap().path(), None);
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(feature = "link-c")]
mod with_a_repository {
    use super::*;
    use libgit2_rs::{ObjectType, Repository, Tree};

    fn repo(name: &str) -> (PathBuf, Repository) {
        let d = tmpdir(name);
        let repo = Repository::init(d.join("w"), false).unwrap();
        (d, repo)
    }

    #[test]
    #[cfg_attr(miri, ignore = "C")]
    fn write_tree_read_tree_round_trip() {
        let (d, repo) = repo("trees");
        let index = repo.index().unwrap();
        for (p, data) in [("a.txt", "a\n"), ("dir/b.txt", "b\n"), ("dir/sub/c", "c\n")] {
            index
                .add_from_buffer(
                    &IndexEntry::new(p, 0o100644, Oid::zero(OidType::Sha1)),
                    data.as_bytes(),
                )
                .unwrap();
        }
        // the repository stays usable with its index
        let tree_id = index.write_tree().unwrap();
        let tree = Tree::lookup(&repo, &tree_id).unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(
            *tree.get_name(b"a.txt").unwrap().id(),
            index.get_path("a.txt", Stage::Normal).unwrap().id
        );
        // the same tree again (the tree cache)
        assert_eq!(index.write_tree().unwrap(), tree_id);

        // an index made alone reads it, and writes it to a repository
        let alone = Index::new().unwrap();
        alone.read_tree(&tree).unwrap();
        let a: Vec<_> = alone.iter().map(|e| (e.path, e.id, e.mode)).collect();
        let b: Vec<_> = index.iter().map(|e| (e.path, e.id, e.mode)).collect();
        assert_eq!(a, b);
        assert_eq!(alone.write_tree_to(&repo).unwrap(), tree_id);
        assert!(alone.write_tree().is_err());
        // a conflict cannot be written
        let side = |c| {
            let mut e = entry("dir/b.txt", c);
            e.id = index.get_path("a.txt", Stage::Normal).unwrap().id;
            e
        };
        alone
            .conflict_add(Some(&side('1')), Some(&side('2')), None)
            .unwrap();
        let e = alone.write_tree_to(&repo).unwrap_err();
        assert_eq!(e.class(), ErrorClass::INDEX);
        assert_eq!(
            e.message(),
            "cannot create a tree from a not fully merged index."
        );
        // read_tree replaces the entries
        alone.read_tree(&tree).unwrap();
        assert!(!alone.has_conflicts());
        assert_eq!(alone.len(), 3);
        // an empty index is the empty tree
        let empty = Index::new().unwrap();
        assert_eq!(
            empty.write_tree_to(&repo).unwrap().to_string(),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
        drop(index);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[cfg_attr(miri, ignore = "C")]
    fn add_all_with_a_reentrant_closure() {
        let (d, repo) = repo("addall");
        let w = d.join("w");
        for f in ["a.txt", "b.txt", "skip.txt", "sub/c.txt"] {
            std::fs::create_dir_all(w.join(f).parent().unwrap()).unwrap();
            std::fs::write(w.join(f), f).unwrap();
        }
        let index = repo.index().unwrap();
        let mut seen = Vec::new();
        index
            .add_all_with(["*.txt"], IndexAddOption::empty(), |path, matched| {
                // the same index, read from the closure (the lock is
                // re-entrant), and the repository too
                let before = index.len();
                seen.push((path.to_vec(), matched.map(<[u8]>::to_vec), before));
                assert!(index.get_path(path, Stage::Normal).is_none());
                assert!(repo.head_unborn().unwrap());
                if path == b"skip.txt" {
                    IndexMatch::Skip
                } else {
                    IndexMatch::Apply
                }
            })
            .unwrap();
        let paths: Vec<_> = index.iter().map(|e| e.path).collect();
        assert_eq!(
            paths,
            vec![b"a.txt".to_vec(), b"b.txt".to_vec(), b"sub/c.txt".to_vec()]
        );
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0], (b"a.txt".to_vec(), Some(b"*.txt".to_vec()), 0));
        // the closure saw the entries added before it
        assert_eq!(seen[1].2, 1);
        let blob = index.get_path("a.txt", Stage::Normal).unwrap().id;
        assert!(repo.odb().unwrap().exists(&blob));
        assert_eq!(
            repo.odb().unwrap().read(&blob).unwrap().kind(),
            ObjectType::Blob
        );

        // Stop: Ok, and nothing after
        index.clear().unwrap();
        let mut n = 0;
        index
            .add_all_with(["*"], IndexAddOption::empty(), |_, _| {
                n += 1;
                if n == 2 {
                    IndexMatch::Stop
                } else {
                    IndexMatch::Apply
                }
            })
            .unwrap();
        assert_eq!((n, index.len()), (2, 1));

        // update_all: a changed file, a removed one
        index.add_all(["*"], IndexAddOption::empty()).unwrap();
        assert_eq!(index.len(), 4);
        let old = index.get_path("b.txt", Stage::Normal).unwrap().id;
        std::fs::write(w.join("b.txt"), "changed").unwrap();
        std::fs::remove_file(w.join("skip.txt")).unwrap();
        let mut updated = Vec::new();
        index
            .update_all_with(["*"], |p, _| {
                updated.push(p.to_vec());
                IndexMatch::Apply
            })
            .unwrap();
        assert!(updated.contains(&b"b.txt".to_vec()));
        assert_ne!(index.get_path("b.txt", Stage::Normal).unwrap().id, old);
        assert!(index.get_path("skip.txt", Stage::Normal).is_none());
        index.update_all(["*"]).unwrap();
        assert_eq!(index.len(), 3);

        // a panic is resumed, the index stays usable
        std::fs::write(w.join("new.txt"), "new").unwrap();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            index.add_all_with(["*"], IndexAddOption::FORCE, |_, _| panic!("boom"))
        }));
        assert!(r.is_err());
        assert_eq!(index.len(), 3);
        assert!(index.get_path("new.txt", Stage::Normal).is_none());
        index.write().unwrap();
        assert_eq!(Index::open(d.join("w/.git/index")).unwrap().len(), 3);
        drop(index);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[cfg_attr(miri, ignore = "C")]
    fn index_and_repository_from_threads() {
        let (d, repo) = repo("threads");
        let index = repo.index().unwrap();
        std::fs::write(d.join("w/f"), "data").unwrap();
        std::thread::scope(|s| {
            s.spawn(|| {
                for i in 0..20 {
                    let id = repo
                        .odb()
                        .unwrap()
                        .write(format!("{i}").as_bytes(), ObjectType::Blob)
                        .unwrap();
                    index
                        .add(&IndexEntry::new(format!("n{i:02}"), 0o100644, id))
                        .unwrap();
                }
            });
            s.spawn(|| {
                for _ in 0..20 {
                    index.add_path("f").unwrap();
                    let _ = repo
                        .hashfile(d.join("w/f"), ObjectType::Blob, None)
                        .unwrap();
                }
            });
            s.spawn(|| {
                for _ in 0..20 {
                    let _ = repo.index().unwrap().len();
                    let _ = index.write_tree();
                }
            });
        });
        assert_eq!(index.len(), 21);
        let tree = Tree::lookup(&repo, &index.write_tree().unwrap()).unwrap();
        assert_eq!(tree.len(), 21);
        drop(index);
        let _ = std::fs::remove_dir_all(&d);
    }
}
