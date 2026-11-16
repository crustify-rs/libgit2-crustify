//! The safe API of the index (WP l-index), used as a Rust consumer would:
//! no `unsafe`. The paths an index stores are checked by `path.c` (C), so
//! these tests need `--features link-c`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use libgit2_rs::index::{IndexAddOption, IndexCapability};
use libgit2_rs::util::test_support;
use libgit2_rs::{ErrorClass, ErrorCode, Index, IndexEntry, Oid, OidType, Stage};

fn id(c: char, t: OidType) -> Oid {
    Oid::from_str_hex(&c.to_string().repeat(t.hexsize()), t).unwrap()
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
#[cfg_attr(miri, ignore = "path checks are C")]
fn add_get_remove() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
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
#[cfg_attr(miri, ignore = "path checks are C")]
fn invalid_paths_and_modes() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
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
#[cfg_attr(miri, ignore = "path checks are C")]
fn snapshot_iteration_survives_changes() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
    for i in 0..50 {
        ix.add(&entry(&format!("f{i:02}"), 'c')).unwrap();
    }
    let mut it = ix.iter();
    let first = it.next().unwrap();
    // a clone is the same index: changes show in both
    let mut other = ix.clone();
    other.clear().unwrap();
    assert!(ix.is_empty());
    // the iterator still yields the entries it was made with
    assert_eq!(first.path, b"f00");
    assert_eq!(it.count(), 49);
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn conflicts() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
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
#[cfg_attr(miri, ignore = "path checks are C")]
fn names_extensions_caps_version() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
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
    assert_eq!(ix.caps(), IndexCapability::default());
    ix.set_caps(IndexCapability {
        ignore_case: true,
        ..Default::default()
    })
    .unwrap();
    ix.add(&entry("Mixed/Case", 'a')).unwrap();
    assert!(ix.get_path(b"mixed/case", Stage::Normal).is_some());
    assert!(ix.caps().ignore_case);
    // no repository: no caps from the owner, and no add_all
    assert_eq!(
        ix.set_caps_from_owner().unwrap_err().message(),
        "cannot access repository to set index caps"
    );
    let e = ix
        .add_all(
            &[b"*"],
            IndexAddOption::default(),
            None::<fn(&[u8], Option<&[u8]>) -> i32>,
        )
        .unwrap_err();
    assert_eq!(e.code(), ErrorCode::BARE_REPO);
}

#[test]
#[cfg_attr(miri, ignore = "path checks are C")]
fn write_read_round_trip_all_versions() {
    let _g = test_support::init();
    for t in [OidType::Sha1, OidType::Sha256] {
        for v in 2..=4 {
            let d = tmpdir(&format!("rt{v}{t:?}"));
            let path = d.join("index");
            let mut ix = Index::open_with(&path, t).unwrap();
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
            let back = Index::open_with(&path, t).unwrap();
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
    let _g = test_support::init();
    let mut mem = Index::new().unwrap();
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
#[cfg_attr(miri, ignore = "path checks are C")]
fn read_index_keeps_stat_data() {
    let _g = test_support::init();
    let mut a = Index::new().unwrap();
    let mut e = entry("same", 'a');
    e.ino = 42;
    a.add(&e).unwrap();
    a.add(&entry("old", 'a')).unwrap();
    let mut b = Index::new().unwrap();
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
    let _g = test_support::init();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources");
    let ix = Index::open(&root.join("testrepo.git/index")).unwrap();
    assert!(!ix.is_empty());
    let v4 = Index::open(&root.join("indexv4/.gitted/index")).unwrap();
    assert_eq!(v4.version(), 4);
    let sha256 = Index::open_with(&root.join("git-sha256.index"), OidType::Sha256).unwrap();
    assert_eq!(sha256.oid_type(), OidType::Sha256);
    assert!(!sha256.is_empty());
}

#[test]
#[cfg_attr(miri, ignore = "path checks and pathspecs are C")]
fn remove_all_with_a_reentrant_closure() {
    let _g = test_support::init();
    let mut ix = Index::new().unwrap();
    for p in ["a/1", "a/2", "b/1", "keep"] {
        ix.add(&entry(p, 'a')).unwrap();
    }
    let mut other = ix.clone();
    let mut seen = Vec::new();
    ix.remove_all(
        &[b"a/*", b"b/*"],
        Some(|path: &[u8], matched: Option<&[u8]>| {
            seen.push((path.to_vec(), matched.map(<[u8]>::to_vec)));
            // the closure changes the index under the operation
            if path == b"a/1" {
                let _ = other.remove(b"b/1", Stage::Normal);
            }
            0
        }),
    )
    .unwrap();
    let left: Vec<_> = ix.iter().map(|e| e.path).collect();
    assert_eq!(left, vec![b"keep".to_vec()]);
    assert_eq!(seen[0], (b"a/1".to_vec(), Some(b"a/*".to_vec())));
    // a negative result stops, with C's message
    ix.add(&entry("a/3", 'a')).unwrap();
    let err = ix
        .remove_all(&[b"a/*"], Some(|_: &[u8], _: Option<&[u8]>| -7))
        .unwrap_err();
    assert_eq!(err.message(), "git_index_remove_all callback returned -7");
    assert_eq!(ix.len(), 2);
}
