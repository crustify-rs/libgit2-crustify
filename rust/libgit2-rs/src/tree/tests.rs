//! Unit tests of tree parsing, name search, modes, ordering, tree builders
//! (unvalidated inserts and serialisation) and walks: pure Rust (run under
//! Miri).

use super::*;
use crate::util::cabi::Out;
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
        assert_eq!(ffi::git_tree_entry_dup(Out::from(&mut d), e.as_ref()), 0);
        assert_eq!(
            ((*d).attr, (*d).oid, (*d).name()),
            (0o100755, id, &b"name.txt"[..])
        );
        assert_eq!(ffi::git_tree_entry_cmp(e.as_ref(), d.as_ref()), 0);
        assert_eq!(
            ffi::git_tree_entry_filemode(d.as_ref()),
            FILEMODE_BLOB_EXECUTABLE
        );
        assert_eq!(ffi::git_tree_entry_type(d), OBJ_BLOB);
        ffi::git_tree_entry_free(d);
        ffi::git_tree_entry_free(e);
        ffi::git_tree_entry_free(ptr::null_mut());
        assert!(ffi::git_tree_entry_name(None).is_null());
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
    let p = &raw mut b;
    /// # Safety
    ///
    /// `e` is a live entry (the filter's).
    unsafe extern "C" fn drop_trees(e: *const GitTreeEntry, _: *mut c_void) -> c_int {
        // SAFETY: a live entry, given by the filter.
        c_int::from(unsafe { (*e).is_tree() })
    }
    // SAFETY: a live builder used by nothing else; NUL-terminated names.
    unsafe {
        assert!(!ffi::git_treebuilder_get(p.as_ref(), c"a".into()).is_null());
        assert_eq!(
            ffi::git_treebuilder_filter(p, Some(drop_trees), ptr::null_mut()),
            0
        );
        assert!(ffi::git_treebuilder_get(p.as_ref(), c"a".into()).is_null());
        assert_eq!(ffi::git_treebuilder_entrycount(p.as_ref()), 2);
        assert_eq!(ffi::git_treebuilder_remove(p.as_mut(), c"nope".into()), -1);
        assert_eq!(
            errors::last_message().unwrap(),
            b"failed to remove entry: file isn't in the tree - nope"
        );
        assert_eq!(ffi::git_treebuilder_remove(p.as_mut(), c"b".into()), 0);
        assert_eq!(ffi::git_treebuilder_entrycount(p.as_ref()), 1);
        assert_eq!(ffi::git_treebuilder_clear(p.as_mut()), 0);
        assert_eq!(ffi::git_treebuilder_entrycount(p.as_ref()), 0);
    }
}

/// `git_treebuilder_insert` with the id of the builder's own entry for the
/// name (re-adding an entry with a new mode): the call overwrites that id,
/// so `id` must not be a reference argument (a protected `&Oid` written
/// during the call is undefined behaviour). Needs the C path checks.
#[test]
#[cfg(all(feature = "link-c", not(miri)))]
fn builder_insert_own_entry_id() {
    let _g = test_support::init();
    let mut b = GitTreebuilder::empty(ptr::null_mut());
    let id = Oid::from_bytes(&[7; 20], OidType::Sha1).unwrap();
    // SAFETY: a NUL-terminated name; no validation, so no repository is used.
    unsafe { b.append_entry(c"m".as_ptr(), &id, FILEMODE_COMMIT, false) }.unwrap();
    let p = &raw mut b;
    // SAFETY: a live builder used by nothing else; `own` is the id of its
    // entry "m", live until the entry is replaced or freed.
    unsafe {
        let e = ffi::git_treebuilder_get(p.as_ref(), c"m".into());
        assert!(!e.is_null());
        let own = &raw const (*e).oid;
        assert_eq!(
            ffi::git_treebuilder_insert(Out::null(), p.as_mut(), c"m".into(), own, FILEMODE_COMMIT),
            0
        );
        // Another name with the same entry's id.
        assert_eq!(
            ffi::git_treebuilder_insert(Out::null(), p.as_mut(), c"n".into(), own, FILEMODE_COMMIT),
            0
        );
        assert_eq!(ffi::git_treebuilder_entrycount(p.as_ref()), 2);
        assert_eq!((*e).oid, id);
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

    // The callback gets pointers into the trees' arrays (LC-1): kept past
    // the walk, they are the by-index entries.
    {
        let root = trees[2].1.0;
        let mut saved: Vec<*const GitTreeEntry> = Vec::new();
        let mut lookup = |id: &Oid| by_id(id).ok_or(ErrorCode::NOT_FOUND);
        let mut free = |_t: *mut GitTree| {};
        let mut cb = |r: &GitStr, e: &GitTreeEntry| {
            if r.is_empty() {
                saved.push(ptr::from_ref(e));
            }
            WalkResult::Continue
        };
        let mut path = GitStr::new();
        // SAFETY: live trees from `lookup`.
        let r = unsafe { walk(root, &mut path, true, &mut lookup, &mut free, &mut cb) };
        assert_eq!(r, 0);
        for (i, p) in saved.iter().enumerate() {
            // SAFETY: a live tree.
            assert_eq!(*p, unsafe { ffi::git_tree_entry_byindex(root.as_ref(), i) });
        }
        assert_eq!(saved.len(), 3);
    }

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
        assert_eq!(ffi::git_tree_entrycount(root.as_ref()), 3);
        let e = ffi::git_tree_entry_byname(root.as_ref(), c"z".into());
        assert_eq!(e, ffi::git_tree_entry_byindex(root.as_ref(), 2));
        assert!(ffi::git_tree_entry_byindex(root.as_ref(), 3).is_null());
        let id = Oid::from_bytes(&[0xab; 20], OidType::Sha1).unwrap();
        assert_eq!(
            ffi::git_tree_entry_byid(root.as_ref(), Some(&id)),
            ffi::git_tree_entry_byindex(root.as_ref(), 1)
        );
        assert_eq!(
            ffi::git_tree_walk(root.as_ref(), 5, None, ptr::null_mut()),
            -1
        );
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

/// A C filter callback that re-enters the builder: it removes the current
/// entry itself, removes another one, and reads the count, while
/// `git_treebuilder_filter` iterates.
#[test]
fn filter_reentrant_callback() {
    let _g = test_support::init();
    let mut b = GitTreebuilder::empty(ptr::null_mut());
    let names: Vec<std::ffi::CString> = (0..40)
        .map(|i| std::ffi::CString::new(format!("n{i}")).unwrap())
        .collect();
    for (i, n) in names.iter().enumerate() {
        let id = Oid::from_bytes(&[i as u8 + 1; 20], OidType::Sha1).unwrap();
        // SAFETY: NUL-terminated; no validation (no repository used).
        unsafe { b.append_entry(n.as_ptr(), &id, FILEMODE_BLOB, false) }.unwrap();
    }
    struct Ctx {
        bld: *mut GitTreebuilder,
        calls: usize,
    }
    /// # Safety
    ///
    /// `payload` is the `Ctx` below and `e` a live entry (the filter's).
    unsafe extern "C" fn cb(e: *const GitTreeEntry, payload: *mut c_void) -> c_int {
        // SAFETY: the context passed below, live during the filter.
        let ctx = unsafe { &mut *payload.cast::<Ctx>() };
        ctx.calls += 1;
        // SAFETY: a live entry; its name is NUL-terminated.
        let name = unsafe { CStr::from_ptr((*e).filename) }.to_bytes().to_vec();
        let n: usize = String::from_utf8_lossy(&name[1..]).parse().unwrap();
        // SAFETY: the builder, re-entered through the C ABI only.
        unsafe {
            assert!(ffi::git_treebuilder_entrycount(ctx.bld.as_ref()) > 0);
            match n % 4 {
                // remove itself, then ask to remove it again
                0 => {
                    let own = std::ffi::CString::new(name).unwrap();
                    assert_eq!(
                        ffi::git_treebuilder_remove(ctx.bld.as_mut(), own.as_c_str().into()),
                        0
                    );
                    1
                }
                // remove the next one (which may already have been visited)
                1 => {
                    let next = std::ffi::CString::new(format!("n{}", n + 1)).unwrap();
                    let _ = ffi::git_treebuilder_remove(ctx.bld.as_mut(), next.as_c_str().into());
                    0
                }
                // plain removal by the filter
                2 => 1,
                _ => 0,
            }
        }
    }
    let p = &raw mut b;
    let mut ctx = Ctx { bld: p, calls: 0 };
    // SAFETY: a live builder; the callback and its context are valid.
    unsafe {
        assert_eq!(
            ffi::git_treebuilder_filter(p, Some(cb), (&raw mut ctx).cast()),
            0
        );
    }
    // n%4 == 0 removed by itself, 2 by the filter; 1 removed n+1 (= 2 mod 4).
    // What is left are the n%4 == 1 and 3 that no callback removed.
    let left = b.len();
    assert!(left <= 20 && left > 0, "{left}");
    for (_, &e) in b.map.iter() {
        // SAFETY: an entry the builder owns.
        let n = unsafe { CStr::from_ptr((*e).filename) };
        let k: usize = n.to_str().unwrap()[1..].parse().unwrap();
        assert!(k % 2 == 1, "{k}");
    }
    assert!(ctx.calls >= 20);
}
