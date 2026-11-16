//! Unit tests of tree parsing, name search, modes, ordering, tree builders
//! (unvalidated inserts and serialisation) and walks: pure Rust (run under
//! Miri).

use super::*;
use crate::util::test_support;

fn sha1() -> git_oid_t {
    OidType::Sha1.raw()
}

fn raw_tree(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
    let mut v = Vec::new();
    for (mode, name, idbyte) in entries {
        v.extend_from_slice(mode.as_bytes());
        v.push(b' ');
        v.extend_from_slice(name);
        v.push(0);
        v.extend_from_slice(&[*idbyte; 20]);
    }
    v
}

/// A zeroed tree, as `object.c` allocates one.
struct Fresh(*mut GitTree);

impl Fresh {
    fn new() -> Fresh {
        let p = alloc::calloc(1, size_of::<GitTree>()).cast::<GitTree>();
        assert!(!p.is_null());
        Fresh(p)
    }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        // SAFETY: the tree made in `new` (its odb object is NULL), freed
        // once.
        unsafe { ffi::git_tree__free(self.0.cast()) }
    }
}

fn names(t: *const GitTree) -> Vec<Vec<u8>> {
    // SAFETY: a live tree parsed from data that outlives this call.
    unsafe { entries(t) }
        .iter()
        .map(|e| unsafe { e.name() }.to_vec())
        .collect()
}

#[test]
fn parse_zero_copy() {
    let _g = test_support::init();
    let data = raw_tree(&[
        ("100644", b"a", 1),
        ("40000", b"a.b", 2),
        ("40000", b"a", 3),
        ("160000", b"sub", 4),
        ("100664", b"z", 5),
    ]);
    let t = Fresh::new();
    // SAFETY: a fresh tree; `data` outlives it.
    unsafe {
        assert_eq!(
            ffi::git_tree__parse_raw(t.0.cast(), data.as_ptr().cast(), data.len(), sha1()),
            0
        )
    };
    // SAFETY: a live tree.
    let es = unsafe { entries(t.0) };
    assert_eq!(es.len(), 5);
    assert_eq!(es[1].attr, 0o40000);
    assert_eq!(es[1].filename_len, 3);
    // The names point into the data.
    let range = data.as_ptr_range();
    assert!(es.iter().all(|e| range.contains(&e.filename.cast())));
    assert_eq!(es[3].oid.as_bytes(), [4u8; 20]);
    assert_eq!(names(t.0)[4], b"z");
    assert_eq!(normalize_filemode(u32::from(es[4].attr)), FILEMODE_BLOB);
    assert_eq!(entry_type(es[3].attr), OBJ_COMMIT);
    assert_eq!(entry_type(es[2].attr), OBJ_TREE);
    assert!(es[2].is_tree() && !es[3].is_tree());
}

#[test]
fn parse_errors() {
    let _g = test_support::init();
    let cases: &[(&[u8], &str)] = &[
        (
            b" 100644 a\0aaaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse filemode",
        ),
        (
            b"x a\0aaaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse filemode",
        ),
        (
            b"-1 a\0aaaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse filemode",
        ),
        (
            b"200000 a\0aaaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse filemode",
        ),
        (
            b"100644",
            "failed to parse tree: missing space after filemode",
        ),
        (
            b"100644\0",
            "failed to parse tree: missing space after filemode",
        ),
        (b"100644 a", "failed to parse tree: object is corrupted"),
        (
            b"100644 \0aaaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse filename",
        ),
        (
            b"100644 a\0aaaaaaaaaaaaaaaaaaa",
            "failed to parse tree: can't parse OID",
        ),
    ];
    for &(d, msg) in cases {
        let mut a = GitArray::new();
        assert_eq!(
            parse_entries(&mut a, d, sha1()).unwrap_err(),
            ErrorCode::INVALID,
            "{:?}",
            String::from_utf8_lossy(d)
        );
        assert_eq!(errors::last_message().unwrap(), msg.as_bytes());
        assert_eq!(errors::last_class(), Some(ErrorClass::TREE));
    }
    // Modes as git__strntol32 reads them: leading zeros, a sign, many digits.
    for (m, attr) in [
        ("0100644", 0o100644u16),
        ("+644", 0o644),
        ("00000000000000100644", 0o100644),
        ("177777", 0o177777),
    ] {
        let d = raw_tree(&[(m, b"n", 1)]);
        let mut a = GitArray::new();
        parse_entries(&mut a, &d, sha1()).unwrap();
        assert_eq!(a.as_slice()[0].attr, attr, "{m}");
    }
    // Empty trees are fine.
    let mut a = GitArray::new();
    parse_entries(&mut a, b"", sha1()).unwrap();
    assert!(a.is_empty());
}

#[test]
fn modes() {
    for (raw, norm) in [
        (0o040000, FILEMODE_TREE),
        (0o040755, FILEMODE_TREE),
        (0o100644, FILEMODE_BLOB),
        (0o100664, FILEMODE_BLOB),
        (0o100600, FILEMODE_BLOB),
        (0o100755, FILEMODE_BLOB_EXECUTABLE),
        (0o100700, FILEMODE_BLOB_EXECUTABLE),
        (0o120000, FILEMODE_LINK),
        (0o160000, FILEMODE_COMMIT),
        (0o160755, FILEMODE_BLOB_EXECUTABLE),
        (0, FILEMODE_BLOB),
    ] {
        assert_eq!(normalize_filemode(raw), norm, "{raw:o}");
    }
    assert!(valid_filemode(FILEMODE_LINK) && !valid_filemode(FILEMODE_BLOB_GROUP_WRITABLE));
    assert_eq!(type_from_filemode(0o100664), OBJ_INVALID);
}

#[test]
fn order_and_search() {
    let _g = test_support::init();
    // git's order: "a" (tree) sorts as "a/", after "a.b", before "a0".
    assert!(entry_cmp_parts(b"a.b", true, b"a", true) < 0);
    assert!(entry_cmp_parts(b"a", true, b"a0", false) < 0);
    assert!(entry_cmp_parts(b"a", false, b"a.b", false) < 0);
    assert_eq!(entry_cmp_parts(b"x", false, b"x", false), 0);

    let names_sorted: [(&str, &[u8]); 7] = [
        ("100644", b"a"),
        ("100644", b"a-b"),
        ("40000", b"a.b"),
        ("40000", b"a"),
        ("100644", b"a0"),
        ("40000", b"b"),
        ("100644", b"c"),
    ];
    // Not a valid tree (two "a"), but tree_key_search finds each by its
    // homing search and linear scan.
    let data = raw_tree(
        &names_sorted
            .iter()
            .enumerate()
            .map(|(i, (m, n))| (*m, *n, i as u8))
            .collect::<Vec<_>>(),
    );
    let mut a = GitArray::new();
    parse_entries(&mut a, &data, sha1()).unwrap();
    let es = a.as_slice();
    // SAFETY: entries of `data`, which outlives the lookups.
    let find = |n: &[u8]| key_search(es, n, |e| unsafe { e.name() });
    assert_eq!(find(b"a"), Some(3));
    assert_eq!(find(b"a.b"), Some(2));
    assert_eq!(find(b"a0"), Some(4));
    assert_eq!(find(b"c"), Some(6));
    assert_eq!(find(b"b"), Some(5));
    assert_eq!(find(b"a-"), None);
    assert_eq!(find(b""), None);
    assert_eq!(find(b"zz"), None);
}

#[test]
fn entries_alloc_dup() {
    let _g = test_support::init();
    let id = Oid::from_bytes(&[7; 20], OidType::Sha1).unwrap();
    let e = alloc_entry(b"name.txt", &id);
    assert!(!e.is_null());
    // SAFETY: the entry just made.
    unsafe {
        (*e).attr = 0o100755;
        assert_eq!((*e).name(), b"name.txt");
        assert_eq!(CStr::from_ptr((*e).filename).to_bytes(), b"name.txt");
        let mut d = ptr::null_mut();
        assert_eq!(ffi::git_tree_entry_dup(&mut d, e), 0);
        assert_eq!(
            ((*d).attr, (*d).oid, (*d).name()),
            (0o100755, id, &b"name.txt"[..])
        );
        assert_eq!(ffi::git_tree_entry_cmp(e, d), 0);
        assert_eq!(ffi::git_tree_entry_filemode(d), FILEMODE_BLOB_EXECUTABLE);
        assert_eq!(ffi::git_tree_entry_type(d), OBJ_BLOB);
        ffi::git_tree_entry_free(d);
        ffi::git_tree_entry_free(e);
        ffi::git_tree_entry_free(ptr::null_mut());
        assert!(ffi::git_tree_entry_name(ptr::null()).is_null());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid argument: 'entry'"
        );
    }
}

#[test]
fn builder_serialize_and_filter() {
    let _g = test_support::init();
    let mut b = GitTreebuilder::empty(ptr::null_mut());
    let ids: Vec<Oid> = (1..=4u8)
        .map(|i| Oid::from_bytes(&[i; 20], OidType::Sha1).unwrap())
        .collect();
    // SAFETY: NUL-terminated names; no validation, so no repository is used.
    unsafe {
        b.append_entry(c"b".as_ptr(), &ids[0], FILEMODE_BLOB, false)
            .unwrap();
        b.append_entry(c"a".as_ptr(), &ids[1], FILEMODE_TREE, false)
            .unwrap();
        b.append_entry(c"a.c".as_ptr(), &ids[2], FILEMODE_BLOB_EXECUTABLE, false)
            .unwrap();
        // A duplicate replaces (and frees) the earlier entry.
        b.append_entry(c"b".as_ptr(), &ids[3], FILEMODE_LINK, false)
            .unwrap();
    }
    assert_eq!(b.len(), 3);
    let mut buf = GitStr::new();
    b.serialize(&mut buf, 20).unwrap();
    let mut expect = Vec::new();
    for (m, n, i) in [("100755", "a.c", 2), ("40000", "a", 1), ("120000", "b", 3)] {
        expect.extend_from_slice(format!("{m} {n}\0").as_bytes());
        expect.extend_from_slice(ids[i].as_bytes());
    }
    assert_eq!(buf.as_bytes(), expect.as_slice());
    assert!(buf.capacity() >= 3 * 72);

    // Through the C ABI: get, filter, remove, clear.
    let p = &mut b as *mut GitTreebuilder;
    unsafe extern "C" fn drop_trees(e: *const GitTreeEntry, _: *mut c_void) -> c_int {
        // SAFETY: a live entry, given by the filter.
        c_int::from(unsafe { (*e).is_tree() })
    }
    // SAFETY: a live builder used by nothing else; NUL-terminated names.
    unsafe {
        assert!(!ffi::git_treebuilder_get(p, c"a".as_ptr()).is_null());
        assert_eq!(
            ffi::git_treebuilder_filter(p, Some(drop_trees), ptr::null_mut()),
            0
        );
        assert!(ffi::git_treebuilder_get(p, c"a".as_ptr()).is_null());
        assert_eq!(ffi::git_treebuilder_entrycount(p), 2);
        assert_eq!(ffi::git_treebuilder_remove(p, c"nope".as_ptr()), -1);
        assert_eq!(
            errors::last_message().unwrap(),
            b"failed to remove entry: file isn't in the tree - nope"
        );
        assert_eq!(ffi::git_treebuilder_remove(p, c"b".as_ptr()), 0);
        assert_eq!(ffi::git_treebuilder_entrycount(p), 1);
        assert_eq!(ffi::git_treebuilder_clear(p), 0);
        assert_eq!(ffi::git_treebuilder_entrycount(p), 0);
    }
}

#[test]
fn walks() {
    let _g = test_support::init();
    // root: README, ab/ (4.txt, c/ (3.txt)), z
    let c = raw_tree(&[("100644", b"3.txt", 3)]);
    let ab = raw_tree(&[("100644", b"4.txt", 4), ("40000", b"c", 0xc)]);
    let root = raw_tree(&[
        ("100644", b"README", 1),
        ("40000", b"ab", 0xab),
        ("100644", b"z", 9),
    ]);
    let trees: Vec<(u8, Fresh)> = [(0xab, &ab), (0xc, &c), (0, &root)]
        .into_iter()
        .map(|(k, d)| {
            let t = Fresh::new();
            // SAFETY: a fresh tree; the data outlives it.
            unsafe {
                assert_eq!(
                    ffi::git_tree__parse_raw(t.0.cast(), d.as_ptr().cast(), d.len(), sha1()),
                    0
                )
            };
            (k, t)
        })
        .collect();
    let by_id = |id: &Oid| {
        trees
            .iter()
            .find(|(k, _)| id.as_bytes() == [*k; 20])
            .map(|(_, t)| t.0)
    };
    let run = |pre: bool, skip: &[u8], stop: &[u8]| {
        let mut seen = Vec::new();
        let mut lookup = |id: &Oid| by_id(id).ok_or(ErrorCode::NOT_FOUND);
        let mut free = |_t: *mut GitTree| {};
        let mut cb = |root: &GitStr, e: &GitTreeEntry| {
            // SAFETY: an entry of a live tree.
            let n = unsafe { e.name() };
            seen.push(format!(
                "{}{}",
                String::from_utf8_lossy(root.as_bytes()),
                String::from_utf8_lossy(n)
            ));
            if n == skip {
                WalkResult::Skip
            } else if n == stop {
                WalkResult::Stop(-42)
            } else {
                WalkResult::Continue
            }
        };
        let mut path = GitStr::new();
        let root = trees[2].1.0;
        // SAFETY: live trees from `lookup`.
        let r = unsafe { walk(root, &mut path, pre, &mut lookup, &mut free, &mut cb) };
        (r, seen)
    };
    let (r, pre) = run(true, b"-", b"-");
    assert_eq!(r, 0);
    assert_eq!(pre, ["README", "ab", "ab/4.txt", "ab/c", "ab/c/3.txt", "z"]);
    let (_, post) = run(false, b"-", b"-");
    assert_eq!(
        post,
        ["README", "ab/4.txt", "ab/c/3.txt", "ab/c", "ab", "z"]
    );
    let (_, skipped) = run(true, b"c", b"-");
    assert_eq!(skipped, ["README", "ab", "ab/4.txt", "ab/c", "z"]);
    let (r, stopped) = run(true, b"-", b"4.txt");
    assert_eq!(r, -42);
    assert_eq!(stopped.len(), 3);
    assert_eq!(
        errors::last_message().unwrap(),
        b"git_tree_walk callback returned -42"
    );

    // By path.
    let root = trees[2].1.0;
    let mut lookup = |id: &Oid| by_id(id).ok_or(ErrorCode::NOT_FOUND);
    let mut free = |_t: *mut GitTree| {};
    // SAFETY: live trees.
    unsafe {
        let e = entry_bypath(root, b"ab/c/3.txt", &mut lookup, &mut free).unwrap();
        assert_eq!((*e).name(), b"3.txt");
        alloc::free(e.cast());
        let e = entry_bypath(root, b"ab/", &mut lookup, &mut free).unwrap();
        assert_eq!((*e).name(), b"ab");
        alloc::free(e.cast());
        assert_eq!(
            entry_bypath(root, b"README/x", &mut lookup, &mut free).unwrap_err(),
            ErrorCode::NOT_FOUND
        );
        assert_eq!(
            errors::last_message().unwrap(),
            b"the path 'README' exists but is not a tree"
        );
        assert_eq!(
            entry_bypath(root, b"ab/nope", &mut lookup, &mut free).unwrap_err(),
            ErrorCode::NOT_FOUND
        );
        assert_eq!(
            errors::last_message().unwrap(),
            b"the path 'nope' does not exist in the given tree"
        );
        assert_eq!(
            entry_bypath(root, b"/x", &mut lookup, &mut free).unwrap_err(),
            ErrorCode::NOT_FOUND
        );
        assert_eq!(errors::last_message().unwrap(), b"invalid tree path given");
    }
    // SAFETY: live trees; NUL-terminated names.
    unsafe {
        assert_eq!(ffi::git_tree_entrycount(root), 3);
        let e = ffi::git_tree_entry_byname(root, c"z".as_ptr());
        assert_eq!(e, ffi::git_tree_entry_byindex(root, 2));
        assert!(ffi::git_tree_entry_byindex(root, 3).is_null());
        let id = Oid::from_bytes(&[0xab; 20], OidType::Sha1).unwrap();
        assert_eq!(
            ffi::git_tree_entry_byid(root, &id),
            ffi::git_tree_entry_byindex(root, 1)
        );
        assert_eq!(ffi::git_tree_walk(root, 5, None, ptr::null_mut()), -1);
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid walking mode for tree walk"
        );
    }
}

#[test]
fn update_helpers() {
    assert_eq!(count_slashes(b"a/b/c"), 2);
    assert_eq!(next_component(b"a/b"), Some(&b"a"[..]));
    assert_eq!(next_component(b"b"), None);
    assert_eq!(subpath_len(b"ab/c"), 2);
    assert!(left_dir(b"win32mmap.c", b"win32"));
    assert!(!left_dir(b"win32/mmap.c", b"win32"));
    assert!(!left_dir(b"x", b""));
}
