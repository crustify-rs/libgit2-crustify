//! Tests of the pure path functions (run under Miri) and of their exports;
//! the expected values are the C's (`rust/difftest/fs_diff.c` compares
//! the two on ~200k inputs). Filesystem tests are in `tests/util_fs.rs`.

use super::*;
use crate::util::test_support;

fn s(b: &[u8]) -> GitStr {
    GitStr::from_bytes(b).unwrap()
}

#[test]
fn dirname_and_basename() {
    let _g = test_support::init();
    let cases: &[(&[u8], &[u8], &[u8])] = &[
        (b"", b".", b"."),
        (b"a", b".", b"a"),
        (b"/", b"/", b"/"),
        (b"//", b"/", b"/"),
        (b"/usr", b"/", b"usr"),
        (b"/usr/", b"/", b"usr"),
        (b"/usr/lib//", b"/usr", b"lib"),
        (b"usr/lib", b"usr", b"lib"),
        (b".git/", b".", b".git"),
        (b"a//b//", b"a", b"b"),
        (b"../../a", b"../..", b"a"),
        (b"C:a", b".", b"C:a"),
        (b"C:/", b".", b"C:"),
        (b"/..", b"/", b".."),
        (b"http://a/b/../c", b"http://a/b/..", b"c"),
    ];
    for &(p, d, b) in cases {
        let mut buf = GitStr::new();
        assert_eq!(
            dirname_r(Some(&mut buf), Some(p)),
            Ok(d.len() as c_int),
            "{p:?}"
        );
        assert_eq!(buf.as_bytes(), d, "dirname {p:?}");
        assert_eq!(basename_r(Some(&mut buf), Some(p)), Ok(b.len() as c_int));
        assert_eq!(buf.as_bytes(), b, "basename {p:?}");
        assert_eq!(dirname(Some(p)).unwrap().as_bytes(), d);
        assert_eq!(basename(Some(p)).unwrap().as_bytes(), b);
        let mut me = s(p);
        dirname_self(&mut me).unwrap();
        assert_eq!(me.as_bytes(), d);
    }
    assert_eq!(dirname_r(None, None), Ok(1));
    assert_eq!(basename_piece(None), Piece::Static(b"."));
    // A NUL ends the path, as in C.
    assert_eq!(basename_piece(Some(c_str(b"a/bc\0/d"))), Piece::Range(2, 2));
}

#[test]
fn roots_and_dirs() {
    let _g = test_support::init();
    assert_eq!(root(b"/a"), 0);
    assert_eq!(root(b"a"), -1);
    assert_eq!(root(b""), -1);
    assert_eq!(root(b"C:/a"), 2);
    assert_eq!(root(b"C:a"), -1);
    assert_eq!(root(b"\xc3\xa4:/x"), 3);
    assert_eq!(root(b"\x80\x80\x80\x80:"), -1);
    assert_eq!(root(b"\x80:/"), 2);
    assert_eq!(dirlen(b"a//"), 1);
    assert_eq!(dirlen(b"//"), 1);
    assert_eq!(dirlen(b""), 0);
    assert!(is_root(b"/") && !is_root(b"//"));
    assert!(is_dot_or_dotdot(b".") && is_dot_or_dotdot(b"..") && !is_dot_or_dotdot(b"..."));
    assert!(is_relative(b"./x") && is_relative(b"../x") && !is_relative(b".x"));

    let mut b = s(b"a//b//");
    assert_eq!(basename_offset(Some(&b)), 3);
    to_dir(&mut b).unwrap();
    assert_eq!(b.as_bytes(), b"a//b//");
    let mut b = s(b"../../a");
    to_dir(&mut b).unwrap();
    assert_eq!((b.as_bytes(), b.capacity()), (&b"../../a/"[..], 16));
    let mut e = GitStr::new();
    to_dir(&mut e).unwrap();
    assert!(e.is_empty());

    let mut buf = *b"../../a\0\0";
    string_to_dir(&mut buf, 7);
    assert_eq!(&buf, b"../../a\0\0");
    string_to_dir(&mut buf, 8);
    assert_eq!(&buf, b"../../a/\0");
    // Never past the slice, whatever `size` says.
    let mut full = *b"ab\0";
    string_to_dir(&mut full, 10);
    assert_eq!(&full, b"ab\0");
}

#[test]
fn join_squash_resolve() {
    let _g = test_support::init();
    let mut out = GitStr::new();
    assert_eq!(join_unrooted(&mut out, b"a//b//", Some(b"/base")), Ok(5));
    assert_eq!(out.as_bytes(), b"/base/a//b//");
    assert_eq!(join_unrooted(&mut out, b"/a/b", Some(b"/a/")), Ok(2));
    assert_eq!(out.as_bytes(), b"/a/b");
    assert_eq!(join_unrooted(&mut out, b"/a/b", Some(b"/x")), Ok(0));
    assert_eq!(join_unrooted(&mut out, b"x", None), Ok(0));

    let mut b = s(b"//a//b///c");
    squash_slashes(&mut b);
    assert_eq!(b.as_bytes(), b"/a/b/c");
    let mut b = s(b"http://a/b/../c");
    squash_slashes(&mut b);
    assert_eq!(b.as_bytes(), b"http:/a/b/../c");

    let resolve = |p: &[u8], ceiling: usize| {
        let mut b = s(p);
        resolve_relative(&mut b, ceiling).map(|()| b.as_bytes().to_vec())
    };
    assert_eq!(resolve(b"a//b//", 0).unwrap(), b"a/b/");
    assert_eq!(resolve(b"a//b//", 1).unwrap(), b"ab/");
    assert_eq!(resolve(b"a//b//", 3).unwrap(), b"a//b/");
    assert_eq!(resolve(b"../../a", 0).unwrap(), b"../../a");
    assert_eq!(resolve(b"a/b/../../c", 0).unwrap(), b"c");
    assert_eq!(resolve(b"http://a/b/../c", 0).unwrap(), b"http://a/c");
    assert_eq!(resolve(b"/..", 0), Err(ErrorCode::GENERIC));
    assert_eq!(
        errors::last_message().unwrap(),
        b"cannot strip root component off url"
    );
    // Partially rewritten on failure, as C.
    let mut b = s(b"a/b/../../c");
    assert!(resolve_relative(&mut b, 1).is_err());
    assert_eq!(b.as_bytes(), b"ab//../../c");

    let mut b = s(b"base/dir");
    apply_relative(&mut b, b"../../a").unwrap();
    assert_eq!(b.as_bytes(), b"a");
    let mut b = s(b"/base/dir/");
    apply_relative(&mut b, b"http://a/b/../c").unwrap();
    assert_eq!(b.as_bytes(), b"/base/dir/http:/a/c");
}

#[test]
fn relative_and_common() {
    let _g = test_support::init();
    let rel = |p: &[u8], parent: &[u8]| {
        let mut b = s(p);
        make_relative(&mut b, parent).map(|()| b.as_bytes().to_vec())
    };
    assert_eq!(rel(b"/a/b/c", b"/a/b").unwrap(), b"c");
    assert_eq!(rel(b"/a/b/c", b"/a/b/").unwrap(), b"c");
    assert_eq!(rel(b"/a/b", b"/a/b").unwrap(), b"");
    assert_eq!(rel(b"/a/b", b"/a/b/c/d").unwrap(), b"../../");
    assert_eq!(rel(b"/a/x/y", b"/a/b/c").unwrap(), b"../../x/y");
    assert_eq!(rel(b"a", b"b"), Err(ErrorCode::NOT_FOUND));
    assert_eq!(errors::last_message().unwrap(), b"b is not a parent of a");

    assert_eq!(common_dirlen(b"a/b/c", b"a/b/d"), 4);
    assert_eq!(common_dirlen(b"a/b", b"a/c"), 2);
    assert_eq!(common_dirlen(b"ab", b"ab"), 0);
    assert_eq!(
        equal_or_prefixed(b"a/b", b"a/b"),
        (GIT_FS_PATH_EQUAL, Some(3))
    );
    assert_eq!(
        equal_or_prefixed(b"a/b", b"a/b/c"),
        (GIT_FS_PATH_PREFIX, Some(3))
    );
    assert_eq!(
        equal_or_prefixed(b"a/", b"a/b"),
        (GIT_FS_PATH_PREFIX, Some(1))
    );
    assert_eq!(
        equal_or_prefixed(b"a/b", b"a/bc"),
        (GIT_FS_PATH_NOTEQUAL, None)
    );

    let strncmp = |a: &[u8], b: &[u8], n: usize| crate::util::util::strncmp(a, b, n);
    assert_eq!(cmp(b"a", 1, false, b"a/b", 3, false, strncmp), -1);
    assert_eq!(cmp(b"a", 1, true, b"a/b", 3, false, strncmp), 0);
    assert_eq!(cmp(b"a.", 2, false, b"a", 1, true, strncmp), -1);
    assert_eq!(cmp(b"b", 1, false, b"a", 1, false, strncmp), 1);
}

#[test]
fn urls() {
    let _g = test_support::init();
    let mut b = GitStr::new();
    percent_decode(&mut b, b"%41%42%4").unwrap();
    assert_eq!(b.as_bytes(), b"AB%4");
    percent_decode(&mut b, b"a%20b%zz%").unwrap();
    assert_eq!(b.as_bytes(), b"a b%zz%");
    assert!(is_local_file_url(b"file:///tmp") && is_local_file_url(b"file://localhost/x"));
    assert!(!is_local_file_url(b"file://other/x") && !is_local_file_url(b"file://"));
    fromurl(&mut b, b"file:///tmp/a%20b").unwrap();
    assert_eq!(b.as_bytes(), b"/tmp/a b");
    fromurl(&mut b, b"file://localhost/x/y").unwrap();
    assert_eq!(b.as_bytes(), b"/x/y");
    assert!(fromurl(&mut b, b"file:///").is_err());
    assert_eq!(
        errors::last(),
        (
            ErrorClass::CONFIG,
            b"'file:///' is not a valid local file URI".to_vec()
        )
    );
    from_url_or_path(&mut b, b"rel/x").unwrap();
    assert_eq!(b.as_bytes(), b"rel/x");
    let mut n = s(b"pre\\");
    normalize_slashes(&mut n, b"a\\b").unwrap();
    assert_eq!(n.as_bytes(), b"pre/a/b");
}

#[test]
fn validity() {
    let _g = test_support::init();
    let all = (1 << 10) - 1;
    assert!(is_valid(b"anything/../x", 0));
    assert!(!is_valid(b"a/../b", GIT_FS_PATH_REJECT_TRAVERSAL));
    assert!(!is_valid(b"a//b", GIT_FS_PATH_REJECT_EMPTY_COMPONENT));
    assert!(!is_valid(b"a/", GIT_FS_PATH_REJECT_EMPTY_COMPONENT));
    assert!(is_valid(b"a/b", GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS));
    assert!(!is_valid(b"a/b", GIT_FS_PATH_REJECT_SLASH));
    assert!(!is_valid(b"a\\b", GIT_FS_PATH_REJECT_BACKSLASH));
    assert!(!is_valid(b"a.", GIT_FS_PATH_REJECT_TRAILING_DOT));
    assert!(!is_valid(b"a ", GIT_FS_PATH_REJECT_TRAILING_SPACE));
    assert!(!is_valid(b"a:", GIT_FS_PATH_REJECT_TRAILING_COLON));
    assert!(!is_valid(b"a:b", GIT_FS_PATH_REJECT_NT_CHARS));
    assert!(!is_valid(b"a\x01", GIT_FS_PATH_REJECT_NT_CHARS));
    assert!(is_valid(b"a\x80", GIT_FS_PATH_REJECT_NT_CHARS));
    for bad in [
        &b"CON"[..],
        b"con.txt",
        b"aux",
        b"AUX:",
        b"COM1",
        b"lpt9.x",
        b"x/NUL",
    ] {
        assert!(!is_valid(bad, GIT_FS_PATH_REJECT_DOS_PATHS), "{bad:?}");
    }
    for good in [&b"COM0"[..], b"CONx", b"co", b"nul ", b"COM"] {
        assert!(is_valid(good, GIT_FS_PATH_REJECT_DOS_PATHS), "{good:?}");
    }
    // A sized path with a NUL inside is invalid; a C string ends there.
    let cb = || ValidateCallbacks {
        char_cb: None,
        component_cb: None,
    };
    assert!(!str_is_valid_ext(b"ab\0c", true, all, cb()));
    assert!(str_is_valid_ext(b"ab\0c", false, all, cb()));
    // The callbacks see every byte and every component.
    let (mut chars, mut comps) = (0, Vec::new());
    let ok = str_is_valid_ext(
        b"a/bad/c",
        true,
        GIT_FS_PATH_REJECT_TRAVERSAL,
        ValidateCallbacks {
            char_cb: Some(&mut |_| {
                chars += 1;
                true
            }),
            component_cb: Some(&mut |c: &[u8]| {
                comps.push(c.to_vec());
                c != b"bad"
            }),
        },
    );
    assert!(!ok);
    assert_eq!(chars, 6);
    assert_eq!(comps, [b"a".to_vec(), b"bad".to_vec()]);
}

#[test]
fn walking_up() {
    let _g = test_support::init();
    let walk = |p: &[u8], ceiling: Option<&[u8]>| {
        let mut seen = Vec::new();
        let r = walk_up(p, ceiling, |step| {
            seen.push(match step {
                WalkStep::Prefix(n) => p[..n].to_vec(),
                WalkStep::Empty => b"".to_vec(),
            });
            0
        });
        (r, seen)
    };
    let v = |xs: &[&[u8]]| xs.iter().map(|x| x.to_vec()).collect::<Vec<_>>();
    assert_eq!(
        walk(b"a/b/c", None),
        (0, v(&[b"a/b/c", b"a/b/", b"a/", b""]))
    );
    assert_eq!(
        walk(b"/a/b/c/", None),
        (0, v(&[b"/a/b/c/", b"/a/b/", b"/a/", b"/"]))
    );
    assert_eq!(
        walk(b"/a/b/c", Some(b"/a")),
        (0, v(&[b"/a/b/c", b"/a/b/", b"/a/"]))
    );
    assert_eq!(walk(b"/a/b/c", Some(b"/x")), (0, v(&[b"/a/b/c"])));
    assert_eq!(walk(b"", None), (0, v(&[b""])));
    let mut n = 0;
    assert_eq!(
        walk_up(b"a/b/c", None, |_| {
            n += 1;
            if n == 2 { 42 } else { 0 }
        }),
        42
    );
}

#[test]
fn set_error_messages() {
    let _g = test_support::init();
    errors::set_errno(libc::ENOENT);
    assert_eq!(set_error(libc::ENOENT, b"p", "stat"), ErrorCode::NOT_FOUND);
    // (Miri's `strerror` adds the number.)
    let (class, msg) = errors::last();
    assert_eq!(class, ErrorClass::OS);
    assert!(msg.starts_with(b"could not find 'p' to stat: No such file or directory"));
    assert_eq!(errors::errno(), 0);
    assert_eq!(set_error(libc::EEXIST, b"p", "x"), ErrorCode::EXISTS);
    assert_eq!(
        errors::last_message().unwrap(),
        b"failed x - 'p' already exists: "
    );
    assert_eq!(set_error(libc::EACCES, b"p", "x"), ErrorCode::LOCKED);
    assert_eq!(
        set_error(libc::ENAMETOOLONG, b"p", "x"),
        ErrorCode::INVALID_SPEC
    );
    assert_eq!(set_error(libc::EPERM, b"p", "x"), ErrorCode::GENERIC);
    assert_eq!(errors::last_message().unwrap(), b"could not x 'p': ");
}

#[test]
fn owner_mock() {
    let _g = test_support::init();
    set_owner(GIT_FS_PATH_OWNER_ADMINISTRATOR);
    assert_eq!(owner_is_system(c"/nonexistent"), Ok(true));
    assert_eq!(owner_is_current_user(c"/nonexistent"), Ok(false));
    set_owner(GIT_FS_PATH_OWNER_NONE);
}

// ----- the exports, called as C does -----

mod exports {
    use super::super::ffi::*;
    use super::*;
    use core::ffi::c_void;

    #[test]
    fn dirname_r_of_its_own_buffer() {
        let _g = test_support::init();
        let mut b = s(b"/usr/lib//");
        let p = b.as_ptr();
        // SAFETY: a valid git_str, and a C string within it (the C idiom).
        let r = unsafe { git_fs_path_dirname_r(&raw mut b, p) };
        assert_eq!((r, b.as_bytes()), (4, &b"/usr"[..]));
        let mut b = s(b"a/bcd");
        let p = b.as_ptr();
        // SAFETY: as above.
        let r = unsafe { git_fs_path_basename_r(&raw mut b, p.wrapping_add(2)) };
        assert_eq!((r, b.as_bytes()), (3, &b"bcd"[..]));
        assert_eq!(
            // SAFETY: NULL buffer and path are allowed.
            unsafe { git_fs_path_dirname_r(core::ptr::null_mut(), core::ptr::null()) },
            1
        );
    }

    #[test]
    fn join_unrooted_with_aliased_base() {
        let _g = test_support::init();
        let mut b = s(b"/base");
        let base = b.as_ptr();
        let mut root = -7isize;
        // SAFETY: valid arguments; `base` lies in `b`.
        let r =
            unsafe { git_fs_path_join_unrooted(&raw mut b, c"x/y".as_ptr(), base, &raw mut root) };
        // Not the C's result (C measures `strlen(base)` after writing the
        // join, root 9): the base is copied first (rust/DIVERGENCES.md).
        assert_eq!((r, b.as_bytes(), root), (0, &b"/base/x/y"[..], 5));
        // SAFETY: NULL path: the assertion.
        let r = unsafe {
            git_fs_path_join_unrooted(&raw mut b, core::ptr::null(), base, &raw mut root)
        };
        assert_eq!(r, -1);
        assert_eq!(errors::last_message().unwrap(), b"invalid argument: 'path'");
    }

    #[test]
    fn is_valid_with_size_max() {
        let _g = test_support::init();
        #[repr(C)]
        struct Raw {
            ptr: *const c_char,
            asize: usize,
            size: usize,
        }
        let raw = Raw {
            ptr: c"a/../b".as_ptr(),
            asize: 0,
            size: usize::MAX,
        };
        let p = (&raw const raw).cast::<GitStr>();
        // SAFETY: a `git_str`-shaped value with SIZE_MAX and a C string, as
        // the inline `git_fs_path_is_valid` makes.
        let ok = unsafe {
            git_fs_path_str_is_valid_ext(
                p,
                GIT_FS_PATH_REJECT_TRAVERSAL,
                None,
                None,
                None,
                core::ptr::null_mut(),
            )
        };
        assert!(!ok);
        unsafe extern "C" fn no_x(c: c_char, payload: *mut c_void) -> bool {
            // SAFETY: the test passes a `usize` counter.
            unsafe { *payload.cast::<usize>() += 1 };
            c != b'x' as c_char
        }
        let raw = Raw {
            ptr: c"abxy".as_ptr(),
            asize: 0,
            size: 4,
        };
        let mut n = 0usize;
        let p = (&raw const raw).cast::<GitStr>();
        // SAFETY: a borrowed-shaped `git_str` of 4 readable bytes; the
        // callback reads its counter.
        let ok = unsafe {
            git_fs_path_str_is_valid_ext(p, 1, Some(no_x), None, None, (&raw mut n).cast())
        };
        assert!(!ok);
        assert_eq!(n, 3);
    }

    #[test]
    fn walk_up_writes_and_restores_nuls() {
        let _g = test_support::init();
        unsafe extern "C" fn cb(payload: *mut c_void, path: *const c_char) -> c_int {
            // SAFETY: the test passes a Vec and walk_up a C string.
            let v = unsafe { &mut *payload.cast::<Vec<Vec<u8>>>() };
            // SAFETY: as above.
            v.push(unsafe { CStr::from_ptr(path) }.to_bytes().to_vec());
            0
        }
        let mut b = s(b"a/b/c");
        let mut seen: Vec<Vec<u8>> = Vec::new();
        // SAFETY: a valid git_str, no ceiling, and a callback that reads.
        let r = unsafe {
            git_fs_path_walk_up(
                &raw mut b,
                core::ptr::null(),
                Some(cb),
                (&raw mut seen).cast(),
            )
        };
        assert_eq!(r, 0);
        assert_eq!(
            seen,
            [
                b"a/b/c".to_vec(),
                b"a/b/".to_vec(),
                b"a/".to_vec(),
                b"".to_vec()
            ]
        );
        assert_eq!(b.as_bytes(), b"a/b/c");
        assert_eq!(b.as_c_str().unwrap().to_bytes(), b"a/b/c");
    }

    #[test]
    fn apply_relative_returns_one_on_failure() {
        let _g = test_support::init();
        let mut b = s(b"/");
        assert_eq!(
            // SAFETY: valid arguments.
            unsafe { git_fs_path_apply_relative(&raw mut b, c"..".as_ptr()) },
            1
        );
    }

    #[test]
    fn string_to_dir_export() {
        let _g = test_support::init();
        let mut buf = *b".\0\0";
        // SAFETY: a C string with two writable bytes from its NUL.
        unsafe { git_fs_path_string_to_dir(buf.as_mut_ptr().cast(), 2) };
        assert_eq!(&buf, b"./\0");
    }

    #[test]
    fn diriter_without_init() {
        let _g = test_support::init();
        let mut it = GitFsPathDiriter::new();
        // SAFETY: an initialized (GIT_FS_PATH_DIRITER_INIT) iterator.
        assert_eq!(unsafe { git_fs_path_diriter_next(&raw mut it) }, -1);
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: 'diriter->dir'"
        );
        let (mut out, mut len) = (core::ptr::null(), 0usize);
        assert_eq!(
            // SAFETY: as above, with writable outputs.
            unsafe { git_fs_path_diriter_filename(&raw mut out, &raw mut len, &raw mut it) },
            -1
        );
        // SAFETY: as above.
        unsafe { git_fs_path_diriter_free(&raw mut it) };
        // An empty path fails cleanly (C dereferences NULL).
        assert_eq!(it.init(b"", 0), Err(ErrorCode::GENERIC));
        assert_eq!(
            errors::last_message().unwrap(),
            b"could not open directory ''"
        );
    }

    #[test]
    fn raw_c_strings_are_read_up_to_their_nul() {
        let _g = test_support::init();
        let b = *b"a/b\0zz";
        // SAFETY: a NUL-terminated string, unwritten for the test.
        let r = unsafe { RawCStr::new(core::ptr::NonNull::from(&b).cast()) };
        assert_eq!(common_dirlen_with(r, &b"a/c"[..]), 2);
        assert!(!str_is_valid_with(
            r,
            usize::MAX,
            GIT_FS_PATH_REJECT_SLASH,
            ValidateCallbacks {
                char_cb: None,
                component_cb: None
            }
        ));
        let mut comps = Vec::new();
        assert!(str_is_valid_with(
            r,
            usize::MAX,
            GIT_FS_PATH_REJECT_TRAVERSAL,
            ValidateCallbacks {
                char_cb: None,
                component_cb: Some(&mut |c: &[u8]| {
                    comps.push(c.to_vec());
                    true
                }),
            }
        ));
        assert_eq!(comps, [b"a".to_vec(), b"b".to_vec()]);
    }

    #[test]
    fn aliased_decoders_read_their_input_first() {
        let _g = test_support::init();
        // C clears the buffer before reading it: an empty result
        // (rust/DIVERGENCES.md); the Rust decodes the input.
        let mut b = s(b"a%41b");
        let p = b.as_ptr();
        // SAFETY: a valid git_str and a C string in it.
        assert_eq!(unsafe { git__percent_decode(&raw mut b, p) }, 0);
        assert_eq!(b.as_bytes(), b"aAb");
        let mut b = s(b"file:///x%20y");
        let p = b.as_ptr();
        // SAFETY: as above.
        assert_eq!(unsafe { git_fs_path_fromurl(&raw mut b, p) }, 0);
        assert_eq!(b.as_bytes(), b"/x y");
        let mut b = s(b"a\\b");
        let p = b.as_ptr();
        // SAFETY: as above.
        assert_eq!(unsafe { git_fs_path_normalize_slashes(&raw mut b, p) }, 0);
        // (The whole result is normalized, as in C.)
        assert_eq!(b.as_bytes(), b"a/ba/b");
    }

    #[test]
    fn borrowed_strings_are_not_written() {
        let _g = test_support::init();
        let data = *b"a//b/c";
        let mut b = GitStr::new();
        // SAFETY: `data` outlives `b`'s use and is not written by it.
        unsafe { b.attach_notowned(data.as_ptr().cast(), data.len()) };
        squash_slashes(&mut b);
        assert_eq!(&data, b"a//b/c");
        unsafe extern "C" fn cb(payload: *mut c_void, path: *const c_char) -> c_int {
            // SAFETY: the test's Vec, and a C string from walk_up.
            let v = unsafe { &mut *payload.cast::<Vec<Vec<u8>>>() };
            // SAFETY: as above.
            v.push(unsafe { CStr::from_ptr(path) }.to_bytes().to_vec());
            0
        }
        let mut seen: Vec<Vec<u8>> = Vec::new();
        // SAFETY: a borrowed git_str; the callback only reads.
        let r = unsafe {
            git_fs_path_walk_up(
                &raw mut b,
                core::ptr::null(),
                Some(cb),
                (&raw mut seen).cast(),
            )
        };
        assert_eq!(r, 0);
        assert_eq!(
            seen,
            [
                b"a//b/c".to_vec(),
                b"a//b/".to_vec(),
                b"a//".to_vec(),
                b"".to_vec()
            ]
        );
        assert_eq!(&data, b"a//b/c");
        // Resolving copies it into an owned buffer first.
        resolve_relative(&mut b, 0).unwrap();
        assert_eq!(b.as_bytes(), b"a/b/c");
        assert_eq!(&data, b"a//b/c");
    }
}
