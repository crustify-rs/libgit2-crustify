//! Unit tests of the index's pure Rust (no C: they run under Miri): the
//! entry operations, the map, snapshots' deferred deletion, the on-disk
//! parser over hand-made files, and its failures.

use core::ffi::c_int;

use super::ondisk::{self, ParseState};
use super::*;
use crate::util::hash::{self, HashAlgorithm};

fn index() -> Box<GitIndex> {
    GitIndex::new(OidType::Sha1).unwrap()
}

fn oid(b: u8) -> Oid {
    Oid::from_bytes(&[b; 20], OidType::Sha1).unwrap()
}

/// Insert a stage-`stage` entry at `path` as `git_index_add` would after
/// its checks.
fn add(ix: &mut GitIndex, path: &[u8], stage: c_int, id: u8) -> Result<EntryPtr> {
    let mut e = entry_alloc(path).unwrap();
    e.update(|x| {
        x.mode = GIT_FILEMODE_BLOB;
        x.id = oid(id);
        x.set_stage(stage);
        adjust_namemask(x, path.len());
    });
    ix.sort();
    ix.insert_finish(e, true, true)
}

fn paths(ix: &mut GitIndex) -> Vec<(Vec<u8>, c_int)> {
    ix.sort();
    // SAFETY: live entries of the index.
    ix.entries
        .iter()
        .map(|e| unsafe { eps(as_entry(e)) })
        .map(|(p, s)| (p.to_vec(), s))
        .collect()
}

fn dispose(mut ix: Box<GitIndex>) {
    ix.dispose();
}

#[test]
fn insert_find_remove() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    add(&mut ix, b"b", 0, 1).unwrap();
    add(&mut ix, b"a/x", 0, 2).unwrap();
    add(&mut ix, b"a/y", 0, 3).unwrap();
    // replacing keeps one entry, with the new id
    let e = add(&mut ix, b"b", 0, 9).unwrap();
    // SAFETY: live.
    assert_eq!(unsafe { (*e.as_ptr()).entry.id }, oid(9));
    assert_eq!(
        paths(&mut ix),
        vec![
            (b"a/x".to_vec(), 0),
            (b"a/y".to_vec(), 0),
            (b"b".to_vec(), 0)
        ]
    );
    assert_eq!(ix.find_path(b"a/y"), Some(1));
    assert_eq!(ix.find_prefix(b"a/"), Some(0));
    assert!(ix.get_bypath(b"a/x", 0).is_some());
    assert!(ix.get_bypath(b"A/X", 0).is_none());
    // file/directory collisions replace
    add(&mut ix, b"a", 0, 4).unwrap();
    assert_eq!(paths(&mut ix), vec![(b"a".to_vec(), 0), (b"b".to_vec(), 0)]);
    add(&mut ix, b"b/c", 0, 5).unwrap();
    assert_eq!(
        paths(&mut ix),
        vec![(b"a".to_vec(), 0), (b"b/c".to_vec(), 0)]
    );
    assert!(ix.remove(b"zz", 0).is_err());
    assert_eq!(
        errors::last_message().unwrap(),
        b"index does not contain zz at stage 0"
    );
    ix.remove(b"a", 0).unwrap();
    ix.remove_directory(b"b/", 0).unwrap();
    assert!(ix.entries.is_empty());
    assert!(ix.get_bypath(b"b/c", 0).is_none());
    dispose(ix);
}

#[test]
fn collisions_without_replace_fail() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    add(&mut ix, b"d/f", 0, 1).unwrap();
    let e = entry_alloc(b"d").unwrap();
    ix.sort();
    assert!(ix.insert_finish(e, false, true).is_err());
    assert_eq!(
        errors::last_message().unwrap(),
        b"'d' appears as both a file and a directory"
    );
    dispose(ix);
}

#[test]
fn conflicts_and_reuc() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    add(&mut ix, b"c", 1, 1).unwrap();
    add(&mut ix, b"c", 2, 2).unwrap();
    add(&mut ix, b"c", 3, 3).unwrap();
    add(&mut ix, b"n", 0, 4).unwrap();
    assert!(ix.has_conflicts());
    let sides = ix.conflict_get(b"c").unwrap();
    assert!(sides.iter().all(Option::is_some));
    let mut cur = 0;
    assert!(api::conflict_next(&mut ix, &mut cur).is_some());
    assert!(api::conflict_next(&mut ix, &mut cur).is_none());
    ops::conflict_to_reuc(&mut ix, b"c").unwrap();
    assert!(!ix.has_conflicts());
    assert_eq!(ix.reuc.len(), 1);
    assert_eq!(ix.reuc_find(b"c"), Ok(0));
    // a second REUC entry for the path replaces the first
    ix.reuc_add(b"c", [0o100644, 0, 0], [Some(oid(7)), None, None])
        .unwrap();
    assert_eq!(ix.reuc.len(), 1);
    // a mode without an id fails C's assertion
    assert!(
        ix.reuc_add(b"d", [0, 0o100644, 0], [None, None, None])
            .is_err()
    );
    assert_eq!(
        errors::last_message().unwrap(),
        b"unrecoverable internal error: 'our_oid'"
    );
    ix.reuc_remove(0).unwrap();
    assert!(ix.reuc_remove(0).is_err());
    assert!(matches!(ix.conflict_get(b"zz"), Err(e) if e == ErrorCode::NOT_FOUND));
    dispose(ix);
}

#[test]
fn names_and_extensions() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    ix.name_add(Some(c"b"), Some(c"o"), None).unwrap();
    ix.name_add(Some(c"a"), None, Some(c"t")).unwrap();
    ix.sort_names();
    // SAFETY: a name entry of the index.
    let first = unsafe { &*ix.names.get(0).unwrap().cast::<GitIndexNameEntry>() };
    // SAFETY: a string the entry owns.
    assert_eq!(unsafe { opt_cstr(first.ancestor) }, Some(&b"a"[..]));
    ix.name_clear();
    assert!(ix.names.is_empty());

    ix.extension_add(*b"ZZZZ", b"one").unwrap();
    ix.extension_add(*b"AAAA", b"two").unwrap();
    ix.extension_add(*b"ZZZZ", b"three").unwrap();
    assert_eq!(ix.extensions.len(), 2);
    assert_eq!(ix.extensions[0].signature, *b"AAAA");
    assert_eq!(
        ix.extensions[ix.extension_find(b"ZZZZ").unwrap()].data,
        b"three"
    );
    // a short signature compares as C's strncmp
    assert!(ix.extension_find(b"ZZ").is_err());
    ix.extension_remove(b"AAAA").unwrap();
    assert!(ix.extension_remove(b"AAAA").is_err());
    dispose(ix);
}

#[test]
fn removal_is_deferred_while_a_snapshot_reads() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    add(&mut ix, b"x", 0, 1).unwrap();
    add(&mut ix, b"y", 0, 2).unwrap();
    let snap = ix.entries.dup(ix.entries.cmp_fn()).unwrap();
    ix.readers.inc();
    ix.remove(b"x", 0).unwrap();
    assert_eq!(ix.deleted.len(), 1);
    // the snapshot's entries are still readable
    // SAFETY: deferred, so live.
    let p = unsafe { epath(as_entry(snap.get(0).unwrap())) };
    assert_eq!(p, b"x");
    // clear frees nothing while the reader is there
    ix.clear().unwrap();
    assert_eq!(ix.deleted.len(), 2);
    ix.readers.dec();
    drop(snap);
    ix.clear().unwrap();
    assert!(ix.deleted.is_empty());
    dispose(ix);
}

#[test]
fn case_insensitive_index() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    ix.set_ignore_case(true);
    add(&mut ix, b"Dir/File", 0, 1).unwrap();
    assert!(ix.get_bypath(b"dir/file", 0).is_some());
    assert_eq!(ix.find_path(b"DIR/FILE"), Some(0));
    // a new file in the directory takes its case
    let mut e = entry_alloc(b"dir/other").unwrap();
    ix.sort();
    let (_, _, best) = ix.existing_and_best(b"dir/other", 0);
    ix.canonicalize_directory_path(&mut e, best);
    assert_eq!(e.path(), b"Dir/other");
    ix.insert_finish(e, true, false).unwrap();
    // the same path in another case replaces
    add(&mut ix, b"DIR/FILE", 0, 2).unwrap();
    assert_eq!(ix.entries.len(), 2);
    dispose(ix);
}

#[test]
fn case_flag_change_leaves_no_dangling_key() {
    // C: with "x/a" put before "x/A", a switch to case-insensitive and a
    // removal of "x/A" removed "x/a"'s key and left "x/A"'s, then freed
    // it (see DEFECTS.md); the next lookup read freed memory.
    let _g = crate::util::test_support::init();
    let mut ix = index();
    add(&mut ix, b"x/a", 0, 1).unwrap();
    add(&mut ix, b"x/A", 0, 2).unwrap();
    ix.set_ignore_case(true);
    ix.remove_directory(b"x/", 0).unwrap();
    assert!(ix.entries.is_empty());
    // Miri would report a use after free here
    assert!(ix.get_bypath(b"x/a", 0).is_none());
    dispose(ix);
}

#[test]
fn tree_cache_is_invalidated_by_changes() {
    let _g = crate::util::test_support::init();
    let mut ix = index();
    let mut data = b"\x002 1\n".to_vec();
    data.extend_from_slice(&[1; 20]);
    data.extend_from_slice(b"d\x001 0\n");
    data.extend_from_slice(&[2; 20]);
    ix.tree = tree_cache::read(&data, OidType::Sha1, &mut ix.tree_pool).unwrap();
    add(&mut ix, b"top", 0, 1).unwrap();
    ix.remove(b"top", 0).unwrap();
    // SAFETY: the index's tree.
    assert_eq!(unsafe { (*ix.tree).entry_count }, -1);
    // SAFETY: as above.
    let d = unsafe { tree_cache::get(ix.tree, b"d") };
    // SAFETY: as above.
    assert_eq!(unsafe { (*d).entry_count }, 1);
    dispose(ix);
}

// ----- the parser -----

struct Ent<'a> {
    path: &'a [u8],
    flags_extended: u16,
    stage: u16,
}

/// A version-`v` index file of `ents` (SHA-1), with `ext` appended.
fn encode(v: u32, ents: &[Ent<'_>], ext: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"DIRC");
    out.extend_from_slice(&v.to_be_bytes());
    out.extend_from_slice(&(ents.len() as u32).to_be_bytes());
    let mut last: &[u8] = b"";
    for e in ents {
        let start = out.len();
        for i in 0..10u32 {
            out.extend_from_slice(&(i + 1).to_be_bytes());
        }
        out.extend_from_slice(&[0xab; 20]);
        let mut flags = (e.path.len().min(0xfff) as u16) | (e.stage << 12);
        if e.flags_extended != 0 {
            flags |= GIT_INDEX_ENTRY_EXTENDED;
        }
        out.extend_from_slice(&flags.to_be_bytes());
        if e.flags_extended != 0 {
            out.extend_from_slice(&e.flags_extended.to_be_bytes());
        }
        if v >= 4 {
            let same = e.path.iter().zip(last).take_while(|(a, b)| a == b).count();
            let mut vb = [0u8; 16];
            let n = crate::util::varint::encode(Some(&mut vb), (last.len() - same) as u64) as usize;
            out.extend_from_slice(&vb[..n]);
            out.extend_from_slice(&e.path[same..]);
            out.push(0);
            last = e.path;
        } else {
            out.extend_from_slice(e.path);
            let len = out.len() - start;
            let padded = (len + 8) & !7;
            out.resize(start + padded, 0);
        }
    }
    out.extend_from_slice(ext);
    let h = hash::hash_buf(&out, HashAlgorithm::Sha1).unwrap();
    out.extend_from_slice(h.as_bytes());
    out
}

fn parse(buf: &[u8]) -> (Box<GitIndex>, Result<()>) {
    let mut ix = index();
    let mut st: ParseState = ix.take_parse_state();
    let mut validate = |p: &[u8]| !p.is_empty() && !p.starts_with(b".git/");
    let r = ondisk::parse(&mut st, buf, &mut validate);
    ix.put_parse_state(st);
    (ix, r)
}

#[test]
fn parses_versions_2_3_4() {
    let _g = crate::util::test_support::init();
    let ents = [
        Ent {
            path: b"a/b/c",
            flags_extended: 0,
            stage: 0,
        },
        Ent {
            path: b"a/b/d",
            flags_extended: GIT_INDEX_ENTRY_INTENT_TO_ADD,
            stage: 0,
        },
        Ent {
            path: b"z",
            flags_extended: 0,
            stage: 2,
        },
    ];
    for v in [2, 3, 4] {
        let file = encode(v, &ents, b"");
        let (mut ix, r) = parse(&file);
        r.unwrap();
        assert_eq!(ix.version, v);
        assert!(!ix.dirty());
        assert_eq!(
            paths(&mut ix),
            vec![
                (b"a/b/c".to_vec(), 0),
                (b"a/b/d".to_vec(), 0),
                (b"z".to_vec(), 2)
            ],
            "version {v}"
        );
        let ent = *ix.get_bypath(b"a/b/d", 0).unwrap().entry();
        assert_eq!(ent.flags_extended, GIT_INDEX_ENTRY_INTENT_TO_ADD);
        assert_eq!(
            (ent.ctime.seconds, ent.mtime.nanoseconds, ent.file_size),
            (1, 4, 10)
        );
        assert_eq!(&ix.checksum[..20], &file[file.len() - 20..]);
        dispose(ix);
    }
}

#[test]
fn parses_extensions() {
    let _g = crate::util::test_support::init();
    let mut ext = Vec::new();
    // TREE
    let mut tree = b"\x001 0\n".to_vec();
    tree.extend_from_slice(&[3; 20]);
    ext.extend_from_slice(b"TREE");
    ext.extend_from_slice(&(tree.len() as u32).to_be_bytes());
    ext.extend_from_slice(&tree);
    // REUC
    let mut reuc = b"f\x00100644\x000\x00100755\x00".to_vec();
    reuc.extend_from_slice(&[4; 20]);
    reuc.extend_from_slice(&[5; 20]);
    ext.extend_from_slice(b"REUC");
    ext.extend_from_slice(&(reuc.len() as u32).to_be_bytes());
    ext.extend_from_slice(&reuc);
    // NAME
    let name = b"anc\x00\x00theirs\x00";
    ext.extend_from_slice(b"NAME");
    ext.extend_from_slice(&(name.len() as u32).to_be_bytes());
    ext.extend_from_slice(name);
    // two unknown optional ones, one duplicated
    for (sig, d) in [(b"ZZZZ", &b"zz"[..]), (b"QQQQ", b"q"), (b"ZZZZ", b"dup")] {
        ext.extend_from_slice(sig);
        ext.extend_from_slice(&(d.len() as u32).to_be_bytes());
        ext.extend_from_slice(d);
    }
    let file = encode(
        2,
        &[Ent {
            path: b"f",
            flags_extended: 0,
            stage: 0,
        }],
        &ext,
    );
    let (ix, r) = parse(&file);
    r.unwrap();
    assert!(!ix.tree.is_null());
    assert_eq!(ix.reuc.len(), 1);
    // SAFETY: the REUC entry.
    let re = unsafe { &*ix.reuc.get(0).unwrap().cast::<GitIndexReucEntry>() };
    assert_eq!(re.mode, [0o100644, 0, 0o100755]);
    assert_eq!(re.oid[2], Oid::from_bytes(&[5; 20], OidType::Sha1).unwrap());
    assert_eq!(ix.names.len(), 1);
    assert_eq!(ix.extensions.len(), 2);
    assert_eq!(ix.extensions[1].data, b"zz");
    // the duplicate's message is left, as C's `extension_ondup` set it
    assert_eq!(
        errors::last_message().unwrap(),
        b"found duplicate extension 'ZZZZ'"
    );
    dispose(ix);
}

#[test]
fn rejects_corrupt_files() {
    let _g = crate::util::test_support::init();
    let one = [Ent {
        path: b"f",
        flags_extended: 0,
        stage: 0,
    }];
    let good = encode(2, &one, b"");
    let cases: Vec<(Vec<u8>, &[u8])> = vec![
        (
            good[..20].to_vec(),
            b"invalid data in index - insufficient buffer space",
        ),
        (
            {
                let mut b = good.clone();
                b[0] = b'X';
                b
            },
            b"invalid data in index - incorrect header signature",
        ),
        (
            encode(5, &one, b""),
            b"invalid data in index - incorrect header version",
        ),
        (
            {
                let mut b = good.clone();
                let n = b.len();
                b[n - 1] ^= 1;
                b
            },
            b"invalid data in index - calculated checksum does not match expected",
        ),
        (
            {
                let mut b = encode(2, &one, b"");
                b[11] = 2;
                let n = b.len() - 20;
                let h = hash::hash_buf(&b[..n], HashAlgorithm::Sha1).unwrap();
                b[n..].copy_from_slice(h.as_bytes());
                b
            },
            b"invalid data in index - header entries changed while parsing",
        ),
        (
            encode(
                2,
                &[Ent {
                    path: b".git/x",
                    flags_extended: 0,
                    stage: 0,
                }],
                b"",
            ),
            b"invalid data in index - invalid entry",
        ),
        (
            encode(2, &one, b"abcd\0\0\0\x01x"),
            b"unsupported mandatory extension: 'abcd'",
        ),
        (
            encode(2, &one, b"ABCD\0\0\x10\0x"),
            b"invalid data in index - extension is truncated",
        ),
        (
            encode(2, &one, b"TREE\0\0\0\x02xx"),
            b"corrupted TREE extension in index",
        ),
        (
            encode(2, &one, b"REUC\0\0\0\x02xx"),
            b"invalid data in index - reading reuc entries",
        ),
        (
            encode(2, &one, b"NAME\0\0\0\x02x\0"),
            b"invalid data in index - reading conflict name entries",
        ),
    ];
    for (buf, msg) in cases {
        let (ix, r) = parse(&buf);
        assert!(r.is_err());
        assert_eq!(errors::last_message().unwrap(), msg);
        dispose(ix);
    }
    // a zero checksum (index.skipHash) is accepted
    let mut b = good.clone();
    let n = b.len();
    b[n - 20..].fill(0);
    let (ix, r) = parse(&b);
    r.unwrap();
    dispose(ix);
}

#[test]
fn v4_prefix_errors() {
    let _g = crate::util::test_support::init();
    // a strip length longer than the previous path
    let mut file = encode(
        4,
        &[Ent {
            path: b"ab",
            flags_extended: 0,
            stage: 0,
        }],
        b"",
    );
    // the varint is right after the flags (offset 12 + 62)
    file[12 + 62] = 5;
    let n = file.len() - 20;
    let h = hash::hash_buf(&file[..n], HashAlgorithm::Sha1).unwrap();
    file[n..].copy_from_slice(h.as_bytes());
    let (ix, r) = parse(&file);
    assert!(r.is_err());
    assert_eq!(
        errors::last_message().unwrap(),
        b"invalid data in index - invalid entry"
    );
    dispose(ix);
}

#[test]
fn truncated_entries_at_every_length() {
    let _g = crate::util::test_support::init();
    let ents = [
        Ent {
            path: b"a",
            flags_extended: 0,
            stage: 0,
        },
        Ent {
            path: b"bb",
            flags_extended: GIT_INDEX_ENTRY_SKIP_WORKTREE,
            stage: 0,
        },
    ];
    for v in [2, 4] {
        let file = encode(v, &ents, b"");
        for cut in (12..file.len() - 20).step_by(7) {
            let mut b = file[..cut].to_vec();
            let h = hash::hash_buf(&b, HashAlgorithm::Sha1).unwrap();
            b.extend_from_slice(h.as_bytes());
            let (ix, r) = parse(&b);
            assert!(r.is_err(), "v{v} cut {cut}");
            dispose(ix);
        }
    }
}
