//! The exports of `repository.c`, `libgit2.c` and `settings.c` (WP l-repo),
//! called as C calls them. They reach the C that stays (config, refs, odb),
//! so they run with `--features link-c` and not under Miri.
#![cfg(all(feature = "link-c", not(miri)))]
// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};

use core::ptr;
use libgit2_rs::config::ffi::{git_config_free, git_config_set_int32, git_config_set_string};
use std::ffi::CString;
use std::path::{Path, PathBuf};

use libgit2_rs::buf::GitBuf;
use libgit2_rs::repository::GitRepository;
use libgit2_rs::repository::ffi as r;
use libgit2_rs::strarray::GitStrArray;
use libgit2_rs::util::errors;
use libgit2_rs::util::test_support;

unsafe extern "C" {
    fn git_libgit2_opts(key: c_int, ...) -> c_int;
    fn git_buf_dispose(buf: *mut GitBuf);
    fn git_strarray_dispose(a: *mut GitStrArray);
    fn git_fs_path__set_owner(owner: c_uint);
    fn git_reference_free(r: *mut c_void);
    fn git_reference_lookup(out: *mut *mut c_void, repo: *mut c_void, name: *const c_char)
    -> c_int;
    fn git_reference_symbolic_target(r: *const c_void) -> *const c_char;
}

const GIT_OPT_GET_USER_AGENT: c_int = 17;
const GIT_OPT_SET_USER_AGENT: c_int = 13;
const GIT_OPT_GET_EXTENSIONS: c_int = 33;
const GIT_OPT_SET_EXTENSIONS: c_int = 34;
const GIT_OPT_GET_OWNER_VALIDATION: c_int = 35;
const GIT_OPT_SET_SEARCH_PATH: c_int = 5;
const GIT_OPT_GET_MWINDOW_SIZE: c_int = 0;
const GIT_OPT_SET_MWINDOW_SIZE: c_int = 1;

fn msg() -> String {
    String::from_utf8_lossy(&errors::last_message().unwrap_or_default()).into_owned()
}

/// A fresh directory under the system's temporary directory.
fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("l-repo-ffi-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn c(p: &Path) -> CString {
    CString::new(p.as_os_str().as_encoded_bytes()).unwrap()
}

fn cs<'a>(p: *const c_char) -> Option<&'a str> {
    // SAFETY: NULL or a string the repository owns, read at once.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_str().unwrap())
}

fn init(path: &Path, bare: bool) -> *mut GitRepository {
    let mut repo = ptr::null_mut();
    // SAFETY: a writable out and a path.
    let e = unsafe { r::git_repository_init(&mut repo, c(path).as_ptr(), c_uint::from(bare)) };
    assert_eq!(e, 0, "{}", msg());
    repo
}

#[test]
fn init_open_and_paths() {
    let _g = test_support::init();
    let d = tmpdir("paths");
    let repo = init(&d.join("w"), false);
    // SAFETY: a live repository, its strings read at once.
    unsafe {
        let gitdir = format!("{}/w/.git/", d.display());
        assert_eq!(cs(r::git_repository_path(repo)), Some(gitdir.as_str()));
        assert_eq!(
            cs(r::git_repository_workdir(repo)),
            Some(format!("{}/w/", d.display()).as_str())
        );
        assert_eq!(cs(r::git_repository_commondir(repo)), Some(gitdir.as_str()));
        assert_eq!(r::git_repository_is_bare(repo), 0);
        assert_eq!(r::git_repository_is_worktree(repo), 0);
        assert_eq!(r::git_repository_head_unborn(repo), 1);
        assert_eq!(r::git_repository_is_empty(repo), 1);
        assert_eq!(r::git_repository_head_detached(repo), 0);
        assert_eq!(r::git_repository_state(repo), 0);
        assert_eq!(r::git_repository_oid_type(repo), 1);
        assert_eq!(r::git_repository_is_shallow(repo), 0);

        let mut head = ptr::null_mut();
        assert_eq!(r::git_repository_head(&mut head, repo), -9, "unborn");

        let mut buf = GitBuf::new();
        assert_eq!(r::git_repository_item_path(&mut buf, repo, 4), 0);
        assert_eq!(buf.as_bytes(), format!("{gitdir}objects/").as_bytes());
        assert_eq!(r::git_repository_item_path(&mut buf, repo, 8), 0);
        assert_eq!(buf.as_bytes(), format!("{gitdir}config").as_bytes());
        git_buf_dispose(&mut buf);

        // Discover from a subdirectory of the working directory.
        std::fs::create_dir_all(d.join("w/a/b")).unwrap();
        let mut buf = GitBuf::new();
        let e = r::git_repository_discover(&mut buf, c(&d.join("w/a/b")).as_ptr(), 0, ptr::null());
        assert_eq!(e, 0, "{}", msg());
        assert_eq!(buf.as_bytes(), gitdir.as_bytes());
        // A ceiling at `w/a` stops the search.
        let ceil = c(&d.join("w/a"));
        let e =
            r::git_repository_discover(&mut buf, c(&d.join("w/a/b")).as_ptr(), 0, ceil.as_ptr());
        assert_eq!(e, -3);
        git_buf_dispose(&mut buf);

        // Re-open by the working directory and by the gitdir.
        let mut r2 = ptr::null_mut();
        assert_eq!(r::git_repository_open(&mut r2, c(&d.join("w")).as_ptr()), 0);
        assert_eq!(cs(r::git_repository_path(r2)), Some(gitdir.as_str()));
        r::git_repository_free(r2);
        let mut r3 = ptr::null_mut();
        assert_eq!(
            r::git_repository_open_bare(&mut r3, c(&d.join("w/.git")).as_ptr()),
            0
        );
        assert_eq!(r::git_repository_is_bare(r3), 1);
        assert!(r::git_repository_workdir(r3).is_null());
        r::git_repository_free(r3);
        r::git_repository_free(repo);
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn open_errors_match_c() {
    let _g = test_support::init();
    let d = tmpdir("errors");
    // SAFETY: writable outs and paths.
    unsafe {
        let mut repo = ptr::dangling_mut();
        let p = c(&d.join("nothing"));
        std::fs::create_dir_all(d.join("nothing")).unwrap();
        assert_eq!(r::git_repository_open(&mut repo, p.as_ptr()), -3);
        assert!(repo.is_null());
        assert_eq!(
            msg(),
            format!(
                "could not find repository at '{}'",
                d.join("nothing").display()
            )
        );
        assert_eq!(errors::last_class(), Some(errors::ErrorClass::REPOSITORY));

        assert_eq!(r::git_repository_open_bare(&mut repo, p.as_ptr()), -3);
        assert_eq!(
            msg(),
            format!("path is not a repository: {}", d.join("nothing").display())
        );

        // A `.git` file that is not a gitlink.
        std::fs::write(d.join("nothing/.git"), "garbage\n").unwrap();
        assert_eq!(r::git_repository_open(&mut repo, p.as_ptr()), -1);
        assert_eq!(
            msg(),
            format!(
                "the `.git` file at '{}/.git' is malformed",
                d.join("nothing").display()
            )
        );

        // Asserted arguments.
        assert_eq!(
            r::git_repository_discover(ptr::null_mut(), ptr::null(), 0, ptr::null()),
            -1
        );
        assert_eq!(msg(), "invalid argument: 'start_path'");
        assert!(r::git_repository_path(ptr::null()).is_null());
        assert_eq!(msg(), "invalid argument: 'repo'");
        assert_eq!(r::git_repository_is_bare(ptr::null()), -1);
        assert_eq!(r::git_repository_oid_type(ptr::null_mut()), 0);
        let mut v = 7u32;
        assert_eq!(r::git_repository_init_options_init(ptr::null_mut(), 0), -1);
        assert_eq!(msg(), "invalid version 0 on git_repository_init_options");
        assert_eq!(
            r::git_repository_new_options_init((&raw mut v).cast(), 2),
            -1
        );
        assert_eq!(msg(), "invalid version 2 on git_repository_new_options");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn reinit_and_init_options() {
    let _g = test_support::init();
    let d = tmpdir("reinit");
    let repo = init(&d.join("b.git"), true);
    // SAFETY: as C would call them.
    unsafe {
        assert_eq!(r::git_repository_is_bare(repo), 1);
        r::git_repository_free(repo);
        let mut opts = core::mem::zeroed::<r::GitRepositoryInitOptions>();
        assert_eq!(r::git_repository_init_options_init(&mut opts, 1), 0);
        opts.flags = (1 << 0) | (1 << 1); // BARE | NO_REINIT
        let mut out = ptr::null_mut();
        let given = c(&d.join("b.git"));
        assert_eq!(
            r::git_repository_init_ext(&mut out, given.as_ptr(), &mut opts),
            -4
        );
        assert_eq!(
            msg(),
            format!("attempt to reinitialize '{}'", d.join("b.git").display())
        );

        // A non-bare repository with an explicit initial head and SHA-256.
        let mut opts = core::mem::zeroed::<r::GitRepositoryInitOptions>();
        r::git_repository_init_options_init(&mut opts, 1);
        opts.flags = 1 << 4; // MKPATH
        opts.initial_head = c"trunk".as_ptr();
        opts.oid_type = 2;
        opts.description = c"my repo\n".as_ptr();
        let mut out = ptr::null_mut();
        let e = r::git_repository_init_ext(&mut out, c(&d.join("s")).as_ptr(), &mut opts);
        assert_eq!(e, 0, "{}", msg());
        // C adds its internal flags to the caller's options.
        assert_ne!(opts.flags & (1 << 16), 0, "HAS_DOTGIT");
        assert_ne!(opts.flags & (1 << 17), 0, "NATURAL_WD");
        assert_eq!(r::git_repository_oid_type(out), 2);
        let mut head = ptr::null_mut();
        assert_eq!(r::git_repository_head(&mut head, out), -9);
        assert_eq!(
            std::fs::read_to_string(d.join("s/.git/HEAD")).unwrap(),
            "ref: refs/heads/trunk\n"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("s/.git/description")).unwrap(),
            "my repo\n"
        );
        let cfg = std::fs::read_to_string(d.join("s/.git/config")).unwrap();
        assert!(cfg.contains("objectformat = sha256"), "{cfg}");
        assert!(cfg.contains("repositoryformatversion = 1"), "{cfg}");
        r::git_repository_free(out);

        // Re-open: the format is read back.
        let mut o2 = ptr::null_mut();
        assert_eq!(r::git_repository_open(&mut o2, c(&d.join("s")).as_ptr()), 0);
        assert_eq!(r::git_repository_oid_type(o2), 2);
        r::git_repository_free(o2);
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn extensions_and_format_versions() {
    let _g = test_support::init();
    let d = tmpdir("ext");
    let repo = init(&d.join("e"), false);
    // SAFETY: as C would call them.
    unsafe {
        let mut cfg = ptr::null_mut();
        assert_eq!(r::git_repository_config(&mut cfg, repo), 0);
        git_config_set_int32(cfg, c"core.repositoryformatversion".as_ptr(), 1);
        git_config_set_string(cfg, c"extensions.frobnicate".as_ptr(), c"yes".as_ptr());
        git_config_free(cfg);
        r::git_repository_free(repo);

        let p = c(&d.join("e"));
        let mut o = ptr::null_mut();
        assert_eq!(r::git_repository_open(&mut o, p.as_ptr()), -1);
        assert_eq!(msg(), "unsupported extension name extensions.frobnicate");

        // Allowed once registered; rejected again with `!`.
        let exts = [c"frobnicate".as_ptr()];
        assert_eq!(
            git_libgit2_opts(GIT_OPT_SET_EXTENSIONS, exts.as_ptr(), 1usize),
            0
        );
        assert_eq!(r::git_repository_open(&mut o, p.as_ptr()), 0, "{}", msg());
        r::git_repository_free(o);
        let mut arr = GitStrArray {
            strings: ptr::null_mut(),
            count: 0,
        };
        assert_eq!(git_libgit2_opts(GIT_OPT_GET_EXTENSIONS, &mut arr), 0);
        let names: Vec<&str> = (0..arr.count)
            .map(|i| cs(*arr.strings.add(i)).unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "frobnicate",
                "noop",
                "objectformat",
                "preciousobjects",
                "refstorage",
                "relativeworktrees",
                "worktreeconfig"
            ]
        );
        git_strarray_dispose(&mut arr);
        let exts = [c"!frobnicate".as_ptr()];
        assert_eq!(
            git_libgit2_opts(GIT_OPT_SET_EXTENSIONS, exts.as_ptr(), 1usize),
            0
        );
        assert_eq!(r::git_repository_open(&mut o, p.as_ptr()), -1);
        assert_eq!(
            git_libgit2_opts(GIT_OPT_SET_EXTENSIONS, ptr::null::<*const c_char>(), 0usize),
            0
        );

        // An unsupported version.
        std::fs::write(
            d.join("e/.git/config"),
            "[core]\n\trepositoryformatversion = 2\n",
        )
        .unwrap();
        assert_eq!(r::git_repository_open(&mut o, p.as_ptr()), -1);
        assert_eq!(
            msg(),
            "unsupported repository version 2; only versions up to 1 are supported"
        );
        std::fs::write(
            d.join("e/.git/config"),
            "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha512\n",
        )
        .unwrap();
        assert_eq!(r::git_repository_open(&mut o, p.as_ptr()), -21);
        assert_eq!(msg(), "unknown object format 'sha512'");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ownership_validation() {
    let g = test_support::exclusive();
    let d = tmpdir("owner");
    let repo = init(&d.join("o"), false);
    // SAFETY: as C would call them.
    unsafe {
        r::git_repository_free(repo);
        let p = c(&d.join("o"));
        let mut o = ptr::null_mut();
        // GIT_FS_PATH_OWNER_OTHER
        git_fs_path__set_owner(1 << 4);
        let e = r::git_repository_open(&mut o, p.as_ptr());
        let m = msg();
        git_fs_path__set_owner(0);
        assert_eq!(e, -36, "{m}");
        assert_eq!(
            m,
            format!(
                "repository path '{}' is not owned by current user",
                d.join("o").display()
            )
        );
        // With validation off it opens.
        let mut v: c_int = -1;
        assert_eq!(git_libgit2_opts(GIT_OPT_GET_OWNER_VALIDATION, &mut v), 0);
        assert_eq!(v, 1);
        libgit2_rs::settings::opts::set_owner_validation(false);
        git_fs_path__set_owner(1 << 4);
        let e = r::git_repository_open(&mut o, p.as_ptr());
        git_fs_path__set_owner(0);
        libgit2_rs::settings::opts::set_owner_validation(true);
        assert_eq!(e, 0);
        r::git_repository_free(o);
    }
    drop(g);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn setters_and_state() {
    let _g = test_support::init();
    let d = tmpdir("setters");
    let repo = init(&d.join("s"), false);
    // SAFETY: as C would call them.
    unsafe {
        assert_eq!(r::git_repository_set_namespace(repo, c"ns".as_ptr()), 0);
        assert_eq!(cs(r::git_repository_get_namespace(repo)), Some("ns"));
        // Setting it to itself (C reads freed memory here).
        assert_eq!(
            r::git_repository_set_namespace(repo, r::git_repository_get_namespace(repo)),
            0
        );
        assert_eq!(cs(r::git_repository_get_namespace(repo)), Some("ns"));
        assert_eq!(r::git_repository_set_namespace(repo, ptr::null()), 0);
        assert!(r::git_repository_get_namespace(repo).is_null());

        let (mut n, mut e) = (ptr::null(), ptr::null());
        assert_eq!(
            r::git_repository_set_ident(repo, c"Me".as_ptr(), c"me@x".as_ptr()),
            0
        );
        r::git_repository_ident(&mut n, &mut e, repo);
        assert_eq!((cs(n), cs(e)), (Some("Me"), Some("me@x")));

        // State files.
        std::fs::write(d.join("s/.git/MERGE_HEAD"), "x\n").unwrap();
        assert_eq!(r::git_repository_state(repo), 1);
        std::fs::write(d.join("s/.git/MERGE_MSG"), "the message\n").unwrap();
        let mut buf = GitBuf::new();
        assert_eq!(r::git_repository_message(&mut buf, repo), 0);
        assert_eq!(buf.as_bytes(), b"the message\n");
        git_buf_dispose(&mut buf);
        assert_eq!(r::git_repository_state_cleanup(repo), 0);
        assert_eq!(r::git_repository_state(repo), 0);
        let mut buf = GitBuf::new();
        assert_eq!(r::git_repository_message(&mut buf, repo), -3);
        assert_eq!(
            msg(),
            "could not access message file: No such file or directory"
        );
        assert_eq!(r::git_repository_message_remove(repo), -1);
        std::fs::create_dir_all(d.join("s/.git/rebase-merge")).unwrap();
        assert_eq!(r::git_repository_state(repo), 9);
        std::fs::write(d.join("s/.git/rebase-merge/interactive"), "").unwrap();
        assert_eq!(r::git_repository_state(repo), 8);
        assert_eq!(r::git_repository_state_cleanup(repo), 0);

        // Bare and back.
        assert_eq!(r::git_repository_set_bare(repo), 0);
        assert_eq!(r::git_repository_is_bare(repo), 1);
        assert!(r::git_repository_workdir(repo).is_null());
        let wd = c(&d.join("s"));
        // The natural workdir needs no gitlink: C deletes `core.worktree`,
        // which is not there.
        assert_eq!(r::git_repository_set_workdir(repo, wd.as_ptr(), 1), -3);
        assert_eq!(msg(), "could not find key 'core.worktree' to delete");
        assert_eq!(
            r::git_repository_set_workdir(repo, wd.as_ptr(), 0),
            0,
            "{}",
            msg()
        );
        assert_eq!(r::git_repository_is_bare(repo), 0);
        assert_eq!(
            cs(r::git_repository_workdir(repo)),
            Some(format!("{}/s/", d.display()).as_str())
        );
        // Its own workdir as the argument.
        assert_eq!(
            r::git_repository_set_workdir(repo, r::git_repository_workdir(repo), 0),
            0
        );

        // HEAD to a new branch name (unborn), then lookup.
        assert_eq!(
            r::git_repository_set_head(repo, c"refs/heads/other".as_ptr()),
            0
        );
        let mut head = ptr::null_mut();
        assert_eq!(r::git_repository_head(&mut head, repo), -9);
        assert_eq!(
            r::git_repository_set_head(repo, c"refs/tags/none".as_ptr()),
            -3
        );
        r::git_repository_free(repo);
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn lazy_objects_are_shared_between_threads() {
    let _g = test_support::init();
    let d = tmpdir("threads");
    let repo = init(&d.join("t"), false);
    struct P(*mut GitRepository);
    // SAFETY: the repository's lazy objects are published with
    // compare-and-swap; that is what the test exercises.
    unsafe impl Send for P {}
    // SAFETY: as above.
    unsafe impl Sync for P {}
    for _round in 0..4 {
        // A fresh repository each round, so creation races.
        let mut o = ptr::null_mut();
        // SAFETY: a writable out and a path.
        let e = unsafe { r::git_repository_open(&mut o, c(&d.join("t")).as_ptr()) };
        assert_eq!(e, 0);
        let p = P(o);
        let results: Vec<[usize; 4]> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8)
                .map(|_| {
                    let p = &p;
                    s.spawn(move || {
                        let (mut a, mut b, mut c, mut d) = (
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                        );
                        // SAFETY: a live repository shared as C shares it.
                        unsafe {
                            assert_eq!(r::git_repository_odb__weakptr(&mut a, p.0), 0);
                            assert_eq!(r::git_repository_config__weakptr(&mut b, p.0), 0);
                            assert_eq!(r::git_repository_refdb__weakptr(&mut c, p.0), 0);
                            assert_eq!(r::git_repository_index__weakptr(&mut d, p.0), 0);
                        }
                        [a as usize, b as usize, c as usize, d as usize]
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(results.iter().all(|x| x == &results[0]));
        assert!(results[0].iter().all(|&x| x != 0));
        // SAFETY: ours, no longer used.
        unsafe { r::git_repository_free(o) };
    }
    // SAFETY: ours.
    unsafe { r::git_repository_free(repo) };
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn opts_through_the_c_shim() {
    let _g = test_support::init();
    // SAFETY: the documented arguments of each option.
    unsafe {
        let mut buf = GitBuf::new();
        assert_eq!(git_libgit2_opts(GIT_OPT_GET_USER_AGENT, &mut buf), 0);
        assert!(buf.as_bytes().starts_with(b"libgit2 "));
        git_buf_dispose(&mut buf);
        assert_eq!(git_libgit2_opts(9999), -1);
        assert_eq!(msg(), "invalid option key");
        // An invalid level: the path is not read (as C).
        assert_eq!(git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, 1 as c_int), -1);
        assert_eq!(msg(), "invalid config path selector 1");
        let mut sz = 0usize;
        assert_eq!(git_libgit2_opts(GIT_OPT_GET_MWINDOW_SIZE, &mut sz), 0);
        assert_eq!(git_libgit2_opts(GIT_OPT_SET_MWINDOW_SIZE, sz), 0);
        let _ = GIT_OPT_SET_USER_AGENT;
    }
}

#[test]
fn init_is_counted() {
    let _g = test_support::init();
    // SAFETY: init/shutdown pairs, with the test's count still held.
    unsafe {
        let n = libgit2_rs::libgit2::ffi::git_libgit2_init();
        assert!(n >= 2);
        assert_eq!(libgit2_rs::libgit2::ffi::git_libgit2_shutdown(), n - 1);
    }
}

#[test]
fn head_target_after_set_head() {
    let _g = test_support::init();
    let d = tmpdir("head");
    let repo = init(&d.join("h"), false);
    // SAFETY: as C would call them.
    unsafe {
        assert_eq!(
            r::git_repository_set_head(repo, c"refs/heads/main".as_ptr()),
            0
        );
        let mut cfg = ptr::null_mut();
        assert_eq!(r::git_repository_config_snapshot(&mut cfg, repo), 0);
        git_config_free(cfg);
        // HEAD is symbolic to the new branch.
        let mut out = ptr::null_mut();
        assert_eq!(
            r::git_repository_head_for_worktree(&mut out, repo, c"nope".as_ptr()),
            -3
        );
        let mut h: *mut c_void = ptr::null_mut();
        let e = git_reference_lookup(&mut h, repo.cast(), c"HEAD".as_ptr());
        assert_eq!(e, 0);
        assert_eq!(
            cs(git_reference_symbolic_target(h)),
            Some("refs/heads/main")
        );
        git_reference_free(h);
        r::git_repository_free(repo);
    }
    let _ = std::fs::remove_dir_all(&d);
}
