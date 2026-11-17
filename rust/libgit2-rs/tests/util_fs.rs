//! Tests of `futils`, `fs_path`'s directory functions, `posix` and
//! `filebuf` on the filesystem, through the Rust functions and through the
//! exports as C calls them. Each test works in its own scratch directory.
//! They reach the system (and, for hashing, the C), so not under Miri.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
#![cfg(not(miri))]

use core::ffi::{CStr, c_char, c_int, c_void};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use libgit2_rs::__test::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::__test::util::filebuf::{self, GitFilebuf};
use libgit2_rs::__test::util::fs_path::{self, GitFsPathDiriter};
use libgit2_rs::__test::util::futils::{self, Filestamp, MkdirOptions, MkdirPerfdata};
use libgit2_rs::__test::util::posix::{self, GitMap, mode_t};
use libgit2_rs::__test::util::str::GitStr;
use libgit2_rs::__test::util::test_support;
use libgit2_rs::__test::util::vector::GitVector;

/// A scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("u3-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn path(&self, rel: &str) -> Vec<u8> {
        let mut v = self.0.as_os_str().as_bytes().to_vec();
        if !rel.is_empty() {
            v.push(b'/');
            v.extend_from_slice(rel.as_bytes());
        }
        v
    }
    fn c(&self, rel: &str) -> CString {
        CString::new(self.path(rel)).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `GitFilebuf::open`.
fn open_fb(f: &mut GitFilebuf, path: &[u8], flags: c_int, mode: u32) -> errors::Result<()> {
    f.open(path, flags, mode)
}

fn last() -> (ErrorClass, String) {
    let (c, m) = errors::last();
    (c, String::from_utf8_lossy(&m).into_owned())
}

#[test]
fn mkdir_variants() {
    let _g = test_support::init();
    let t = Scratch::new("mkdir");
    futils::mkdir(&t.path("a/b/c"), 0o755, futils::GIT_MKDIR_PATH).unwrap();
    assert!(fs_path::isdir(&t.c("a/b/c")));
    assert_eq!(
        futils::mkdir(&t.path("a/b/c"), 0o755, futils::GIT_MKDIR_EXCL),
        Err(ErrorCode::EXISTS)
    );
    assert!(last().1.ends_with("/a/b/c': directory exists"));
    assert!(futils::mkdir(&t.path("x/y"), 0o755, 0).is_err());
    assert_eq!(last().0, ErrorClass::OS);
    assert!(
        last().1.ends_with("/x/y': No such file or directory"),
        "{}",
        last().1
    );
    futils::mkpath2file(&t.path("m/n/file"), 0o755).unwrap();
    assert!(fs_path::isdir(&t.c("m/n")) && !fs_path::exists(&t.c("m/n/file")));
    assert_eq!(
        futils::mkdir(b"", 0o755, futils::GIT_MKDIR_PATH),
        Err(ErrorCode::GENERIC)
    );
    assert!(last().1.starts_with("attempt to create empty path"));

    // The perf counters and the cache, through the export.
    let mut opts = MkdirOptions {
        cache_pool: core::ptr::null_mut(),
        cache_pathset: core::ptr::null_mut(),
        perfdata: MkdirPerfdata::default(),
    };
    let base = t.c("");
    // SAFETY: valid strings and options without a cache.
    let r = unsafe {
        git_futils_mkdir_relative(
            c"p/q/r".as_ptr(),
            base.as_ptr(),
            0o755,
            futils::GIT_MKDIR_PATH,
            &mut opts,
        )
    };
    assert_eq!(r, 0);
    assert_eq!(
        opts.perfdata,
        MkdirPerfdata {
            stat_calls: 3,
            mkdir_calls: 3,
            chmod_calls: 0
        }
    );

    // A Rust cache.
    struct Cache(Vec<Vec<u8>>);
    impl futils::MkdirCache for Cache {
        fn contains(&mut self, p: &CStr) -> bool {
            self.0.iter().any(|x| x.as_slice() == p.to_bytes())
        }
        fn add(&mut self, buf: &[u8]) -> errors::Result<()> {
            self.0.push(fs_path::c_str(buf).to_vec());
            Ok(())
        }
    }
    let mut cache = Cache(Vec::new());
    let mut o = futils::MkdirOpts {
        cache: Some(&mut cache),
        perfdata: MkdirPerfdata::default(),
    };
    futils::mkdir_relative(
        b"c1/c2",
        Some(&t.path("")),
        0o755,
        futils::GIT_MKDIR_PATH,
        &mut o,
    )
    .unwrap();
    futils::mkdir_relative(
        b"c1/c2/c3",
        Some(&t.path("")),
        0o755,
        futils::GIT_MKDIR_PATH,
        &mut o,
    )
    .unwrap();
    assert_eq!(o.perfdata.stat_calls, 3);
    assert_eq!(cache.0.len(), 3);
}

#[test]
fn rmdir_variants() {
    let _g = test_support::init();
    let t = Scratch::new("rmdir");
    futils::mkdir(&t.path("a/b/c"), 0o755, futils::GIT_MKDIR_PATH).unwrap();
    futils::writebuffer(b"x", &t.c("a/b/file"), 0, 0).unwrap();
    assert!(futils::rmdir_r(&t.path("a"), None, 0).is_err());
    assert!(last().1.contains("still present"));
    futils::rmdir_r(&t.path("a"), None, futils::GIT_RMDIR_SKIP_NONEMPTY).unwrap();
    assert!(fs_path::isdir(&t.c("a/b")) && !fs_path::isdir(&t.c("a/b/c")));
    futils::rmdir_r(
        &t.path("a"),
        None,
        futils::GIT_RMDIR_REMOVE_FILES | futils::GIT_RMDIR_SKIP_ROOT,
    )
    .unwrap();
    assert!(fs_path::is_empty_dir(&t.c("a")));
    futils::mkdir(&t.path("a/p/q"), 0o755, futils::GIT_MKDIR_PATH).unwrap();
    // (Without it, the scratch directory itself would go: it is below the
    // base's length only without its slash.)
    futils::writebuffer(b"", &t.c("keep"), 0, 0).unwrap();
    let r = futils::rmdir_r(b"a/p/q", Some(&t.path("")), futils::GIT_RMDIR_EMPTY_PARENTS);
    assert_eq!(r, Ok(()), "{:?}", last());
    assert!(!fs_path::exists(&t.c("a")));
    futils::writebuffer(b"x", &t.c("blocker"), 0, 0).unwrap();
    futils::rmdir_r(
        &t.path("blocker/x/y"),
        None,
        futils::GIT_RMDIR_REMOVE_BLOCKERS,
    )
    .unwrap();
    assert!(!fs_path::exists(&t.c("blocker")));
    futils::rmdir_r(&t.path("missing"), None, 0).unwrap();
}

#[test]
fn readbuffer_and_writebuffer() {
    let _g = test_support::init();
    let t = Scratch::new("readbuf");
    let data: Vec<u8> = (0..30000u32).map(|i| b'a' + (i % 23) as u8).collect();
    futils::writebuffer(
        &data,
        &t.c("big"),
        libc::O_CREAT | libc::O_WRONLY | futils::O_FSYNC,
        0o600,
    )
    .unwrap();
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &t.c("big")).unwrap();
    assert_eq!(b.as_bytes(), &data[..]);
    assert_eq!(
        futils::readbuffer(&mut b, &t.c("")),
        Err(ErrorCode::NOT_FOUND)
    );
    assert_eq!(
        last(),
        (ErrorClass::INVALID, "requested file is a directory".into())
    );

    // More than one block: C kept only the last one (rust/DEFECTS.md).
    let fd = futils::open_ro(&t.c("big")).unwrap();
    futils::readbuffer_fd_full(&mut b, fd).unwrap();
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    assert_eq!(b.as_bytes(), &data[..]);

    // A failing read (a directory: EISDIR) is C's error, and the string is
    // left empty (it was a length of `usize::MAX` before the check).
    let fd = futils::open_ro(&t.c("")).unwrap();
    assert_eq!(
        futils::readbuffer_fd_full(&mut b, fd),
        Err(ErrorCode::GENERIC)
    );
    assert_eq!(
        last().1.split(':').next().unwrap(),
        "failed to read descriptor"
    );
    assert_eq!((b.len(), b.capacity()), (0, 0));
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();

    let fd = futils::open_ro(&t.c("big")).unwrap();
    assert!(futils::readbuffer_fd(&mut b, fd, 40000).is_err());
    assert_eq!(
        last(),
        (
            ErrorClass::FILESYSTEM,
            "could not read (expected 40000 bytes, read 30000)".into()
        )
    );
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();

    // SHA-256 checksums.
    let mut sum = [0u8; 32];
    let mut updated = -1;
    futils::readbuffer_updated(&mut b, &t.c("big"), Some(&mut sum), Some(&mut updated)).unwrap();
    assert_eq!(updated, 1);
    futils::readbuffer_updated(&mut b, &t.c("big"), Some(&mut sum), Some(&mut updated)).unwrap();
    assert_eq!(updated, 0);

    // The export's argument checks.
    assert_eq!(
        // SAFETY: a valid git_str, and an empty path.
        unsafe { git_futils_readbuffer(&mut b, c"".as_ptr()) },
        -1
    );
    assert_eq!(last().1, "invalid argument: 'path && *path'");

    // The path in the buffer the call replaces (and frees): read first.
    b.sets(&t.path("big")).unwrap();
    let in_b = b.as_ptr();
    // SAFETY: a valid git_str, and a C string within its buffer.
    assert_eq!(unsafe { git_futils_readbuffer(&mut b, in_b) }, 0);
    assert_eq!(b.as_bytes(), &data[..]);
}

#[test]
fn copies_and_moves() {
    let _g = test_support::init();
    let t = Scratch::new("copy");
    futils::mkdir(&t.path("src/d/e"), 0o755, futils::GIT_MKDIR_PATH).unwrap();
    futils::writebuffer(b"hello", &t.c("src/d/f"), 0, 0o644).unwrap();
    futils::writebuffer(b"dot", &t.c("src/.dot"), 0, 0o644).unwrap();
    posix::symlink(c"f", &t.c("src/d/l")).unwrap();
    futils::cp(&t.c("src/d/f"), &t.c("copy"), 0o644).unwrap();
    assert_eq!(
        futils::cp(&t.c("src/d/f"), &t.c("copy"), 0o644),
        Err(ErrorCode::EXISTS)
    );
    futils::cp_r(
        &t.path("src"),
        &t.path("dst"),
        futils::GIT_CPDIR_COPY_SYMLINKS,
        0o755,
    )
    .unwrap();
    assert!(fs_path::isfile(&t.c("dst/d/f")) && fs_path::islink(&t.c("dst/d/l")));
    assert!(!fs_path::exists(&t.c("dst/.dot")) && !fs_path::exists(&t.c("dst/d/e")));
    futils::cp_r(
        &t.path("src"),
        &t.path("dst2"),
        futils::GIT_CPDIR_COPY_DOTFILES | futils::GIT_CPDIR_CREATE_EMPTY_DIRS,
        0o755,
    )
    .unwrap();
    assert!(fs_path::exists(&t.c("dst2/.dot")) && fs_path::isdir(&t.c("dst2/d/e")));
    futils::mv_withpath(&t.c("copy"), &t.c("moved/deep/x"), 0o755).unwrap();
    assert!(fs_path::isfile(&t.c("moved/deep/x")));
    futils::fake_symlink(b"target", &t.c("fake/l")).unwrap();
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &t.c("fake/l")).unwrap();
    assert_eq!(b.as_bytes(), b"target");
    futils::touch(&t.c("fake/l"), Some(1234567890)).unwrap();
    assert_eq!(posix::stat(&t.c("fake/l")).unwrap().st_mtime, 1234567890);
}

#[test]
fn filestamps_and_maps() {
    let _g = test_support::init();
    let t = Scratch::new("stamp");
    futils::writebuffer(b"abcdef", &t.c("f"), 0, 0).unwrap();
    let mut st = Filestamp::default();
    assert_eq!(futils::filestamp_check(Some(&mut st), &t.c("f")), 1);
    assert_eq!(futils::filestamp_check(Some(&mut st), &t.c("f")), 0);
    assert_eq!(futils::filestamp_check(None, &t.c("f")), 1);
    assert_eq!(futils::filestamp_check(Some(&mut st), &t.c("none")), -3);
    let mut copy = Filestamp::default();
    // SAFETY: valid stamps; source and target may alias.
    unsafe {
        git_futils_filestamp_set(&mut copy, &st);
        git_futils_filestamp_set(&mut copy, &copy);
    }
    assert_eq!(copy, st);
    futils::filestamp_set(&mut copy, None);
    assert_eq!(copy, Filestamp::default());

    let mut map = futils::mmap_ro_file(&t.c("f")).unwrap();
    assert_eq!(
        // SAFETY: a fresh 6-byte read-only mapping.
        unsafe { core::slice::from_raw_parts(map.data.cast::<u8>(), map.len) },
        b"abcdef"
    );
    // SAFETY: the mapping is unmapped once, with no reference into it.
    unsafe { posix::munmap(&mut map) };
    futils::writebuffer(b"", &t.c("empty"), 0, 0).unwrap();
    assert!(futils::mmap_ro_file(&t.c("empty")).is_err());
    assert_eq!(
        last().1,
        "unrecoverable internal error: 'out != ((void*)0) && len > 0'"
    );
    let mut m = GitMap::default();
    // SAFETY: a writable map; a NULL map to unmap.
    unsafe {
        assert_eq!(p_mmap(&mut m, 0, 1, 2, 0, 0), -1);
        assert_eq!(p_munmap(core::ptr::null_mut()), -1);
    }
    assert_eq!(last().1, "invalid argument: 'map'");
    let mut page = 0usize;
    // SAFETY: a writable size_t.
    assert_eq!(unsafe { git__mmap_alignment(&mut page) }, 0);
    assert!(page.is_power_of_two());
}

#[test]
fn posix_wrappers() {
    let _g = test_support::init();
    let t = Scratch::new("posix");
    let p = t.c("f");
    // SAFETY: a valid path; `p_open`'s mode is read with O_CREAT only.
    let fd = unsafe { git__p_open(p.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o600) };
    assert!(fd >= 0);
    // SAFETY: 3 readable bytes.
    assert_eq!(unsafe { p_write(fd, b"abc".as_ptr().cast(), 3) }, 0);
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    let fd = futils::open_ro(&p).unwrap();
    let mut buf = [0u8; 8];
    // SAFETY: 8 writable bytes.
    assert_eq!(unsafe { p_read(fd, buf.as_mut_ptr().cast(), 8) }, 3);
    // An oversized count goes to the kernel, as in C (`git__is_ssizet` is
    // always true on LP64), which refuses it.
    assert_eq!(
        // SAFETY: the kernel checks the buffer before writing past it.
        unsafe { p_read(fd, buf.as_mut_ptr().cast(), usize::MAX) },
        -1
    );
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    // p_rename does not replace an existing target with link + unlink...
    futils::writebuffer(b"x", &t.c("g"), 0, 0).unwrap();
    assert_eq!(
        // SAFETY: valid paths.
        unsafe { p_rename(p.as_ptr(), t.c("g").as_ptr()) },
        0
    );
    assert!(!fs_path::exists(&p));
    // getcwd with a trailing slash, and never past the buffer.
    let mut cwd = [0u8; 4096];
    assert_eq!(
        // SAFETY: 4096 writable bytes.
        unsafe { p_getcwd(cwd.as_mut_ptr().cast(), cwd.len()) },
        0
    );
    let s = fs_path::c_str(&cwd);
    assert_eq!(s.last(), Some(&b'/'));
    let n = s.len();
    let mut exact = vec![0xffu8; n];
    // SAFETY: `n` writable bytes: room for the path and its NUL only.
    assert_eq!(unsafe { p_getcwd(exact.as_mut_ptr().cast(), n) }, 0);
    assert_eq!(fs_path::c_str(&exact).len(), n - 1);
    // SAFETY: realpath into a git__allocator string, freed once.
    unsafe {
        let r = p_realpath(t.c("").as_ptr(), core::ptr::null_mut());
        assert!(!r.is_null());
        libgit2_rs::__test::util::alloc::free(r.cast());
    }
    let before = posix::p_fsync__cnt.load(core::sync::atomic::Ordering::Relaxed);
    futils::fsync_dir(&t.c("")).unwrap();
    assert!(posix::p_fsync__cnt.load(core::sync::atomic::Ordering::Relaxed) > before);
    assert!(futils::fsync_dir(&t.c("none")).is_err());
    assert!(last().1.contains("failed to open directory"));
}

#[test]
fn directory_iteration() {
    let _g = test_support::init();
    let t = Scratch::new("dirs");
    for f in ["a", "b", ".h"] {
        futils::writebuffer(b"", &t.c(f), 0, 0).unwrap();
    }
    futils::mkdir(&t.path("d"), 0o755, 0).unwrap();

    let mut names = Vec::new();
    let mut p = GitStr::from_bytes(&t.path("")).unwrap();
    fs_path::direach(&mut p, 0, &mut |s: &mut GitStr| {
        names.push(s.as_bytes().to_vec());
        0
    })
    .unwrap();
    names.sort();
    let want: Vec<Vec<u8>> = ["/.h", "/a", "/b", "/d"]
        .iter()
        .map(|n| [t.path(""), n.as_bytes().to_vec()].concat())
        .collect();
    assert_eq!(names, want);
    // A callback's non-zero return stops the walk and is returned.
    assert_eq!(
        fs_path::direach(&mut p, 0, &mut |_| 7),
        Err(ErrorCode::from_raw(7).unwrap())
    );
    let mut missing = GitStr::from_bytes(&t.path("none")).unwrap();
    // C's ENOENT test runs after errno was reset: -1, never ENOTFOUND.
    assert_eq!(
        fs_path::direach(&mut missing, 0, &mut |_| 0),
        Err(ErrorCode::GENERIC)
    );

    let mut it = GitFsPathDiriter::new();
    let base = t.path("");
    it.init(&[base.clone(), b"///".to_vec()].concat(), 0)
        .unwrap();
    let mut n = Vec::new();
    loop {
        match it.next() {
            Ok(()) => {
                n.push(it.filename().unwrap().to_vec());
                assert!(it.fullpath().starts_with(&base));
                it.stat().unwrap();
            }
            Err(ErrorCode::ITER_OVER) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    n.sort();
    assert_eq!(
        n,
        [b".h".to_vec(), b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]
    );
    it.free();

    // dirload, through the export, with `.` and `..`.
    let mut v = GitVector::new();
    let c = t.c("");
    // SAFETY: a valid vector and path; the vector's strings are freed below.
    let r = unsafe {
        git_fs_path_dirload(
            &mut v,
            c.as_ptr(),
            base.len() + 1,
            fs_path::GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT,
        )
    };
    assert_eq!(r, 0);
    let mut got: Vec<Vec<u8>> = v
        .iter()
        .map(|p| {
            // SAFETY: each element is a C string from git__strndup.
            unsafe { CStr::from_ptr(p.cast::<c_char>()) }
                .to_bytes()
                .to_vec()
        })
        .collect();
    got.sort();
    assert_eq!(got.len(), 6);
    assert_eq!(&got[..2], &[b".".to_vec(), b"..".to_vec()]);
    for p in v.iter() {
        // SAFETY: freed once.
        unsafe { libgit2_rs::__test::util::alloc::free(p) };
    }
    v.clear();
    assert!(fs_path::dirload(&mut v, &base, base.len() + 10, 0).is_err());
    assert_eq!(
        last().1,
        "unrecoverable internal error: 'name_len > prefix_len'"
    );

    assert!(fs_path::is_empty_dir(&t.c("d")) && !fs_path::is_empty_dir(&t.c("")));
    let mut d = GitStr::from_bytes(&base).unwrap();
    assert!(fs_path::contains_file(&mut d, b"a") && fs_path::contains_dir(&mut d, b"d"));
    assert_eq!(d.as_bytes(), &base[..]);
}

#[test]
fn prettify_and_owner() {
    let _g = test_support::init();
    let t = Scratch::new("pretty");
    futils::mkdir(&t.path("x/y"), 0o755, futils::GIT_MKDIR_PATH).unwrap();
    let real = posix::realpath(&t.c("x/y")).unwrap();
    let mut b = GitStr::new();
    fs_path::prettify(&mut b, c"y/../y", Some(&t.path("x"))).unwrap();
    assert_eq!(b.as_bytes(), &real[..]);
    fs_path::prettify_dir(&mut b, &t.c("x/y"), None).unwrap();
    assert_eq!(b.as_bytes(), &[real.clone(), b"/".to_vec()].concat()[..]);
    assert_eq!(
        fs_path::prettify(&mut b, &t.c("none/x"), None),
        Err(ErrorCode::NOT_FOUND)
    );
    assert!(b.is_empty());
    // The C idiom: the path is the output buffer's own contents.
    let mut b = GitStr::from_bytes(&t.path("x/./y")).unwrap();
    let p = b.as_ptr();
    assert_eq!(
        // SAFETY: a valid git_str and a C string within it.
        unsafe { git_fs_path_prettify(&mut b, p, core::ptr::null()) },
        0
    );
    assert_eq!(b.as_bytes(), &real[..]);
    assert_eq!(fs_path::owner_is_current_user(&t.c("x")), Ok(true));
    assert_eq!(
        fs_path::owner_is_current_user(&t.c("none")),
        Err(ErrorCode::NOT_FOUND)
    );
    let mut e = GitStr::new();
    fs_path::find_executable(&mut e, b"sh").unwrap();
    assert!(e.as_bytes().ends_with(b"/sh"));
    assert_eq!(
        fs_path::find_executable(&mut e, b"no-such-exe-u3"),
        Err(ErrorCode::NOT_FOUND)
    );
    assert!(fs_path::supports_symlinks(&t.path("")));
}

#[test]
fn filebuf_lifecycle() {
    let _g = test_support::init();
    let t = Scratch::new("filebuf");
    let target = t.path("sub/target");
    let mut f = GitFilebuf::new();
    open_fb(
        &mut f,
        &target,
        filebuf::GIT_FILEBUF_HASH_SHA1 | filebuf::GIT_FILEBUF_CREATE_LEADING_DIRS,
        0o644,
    )
    .unwrap();
    assert_eq!(
        f.lock_path().unwrap().to_bytes(),
        &[target.clone(), b".lock".to_vec()].concat()[..]
    );
    // A second writer finds the lock.
    let mut g = GitFilebuf::new();
    assert_eq!(open_fb(&mut g, &target, 0, 0o644), Err(ErrorCode::LOCKED));
    assert!(last().1.starts_with("failed to lock file '"));
    assert_eq!(g.fd(), -1);
    for i in 0..3000 {
        f.printf_formatted(Some(format!("line {i}\n").as_bytes()))
            .unwrap();
    }
    f.write(b"tail").unwrap();
    f.reserve(3).unwrap().copy_from_slice(b"xyz");
    let mut h = [0u8; 32];
    f.hash(&mut h).unwrap();
    f.commit().unwrap();
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &CString::new(target.clone()).unwrap()).unwrap();
    let mut want: Vec<u8> = (0..3000)
        .flat_map(|i| format!("line {i}\n").into_bytes())
        .collect();
    want.extend_from_slice(b"tailxyz");
    assert_eq!(b.as_bytes(), &want[..]);
    assert!(!fs_path::exists(
        &CString::new([target.clone(), b".lock".to_vec()].concat()).unwrap()
    ));

    // A failed reservation poisons the filebuf.
    open_fb(&mut f, &target, 0, 0o644).unwrap();
    assert!(f.reserve(1 << 20).is_err());
    assert!(f.write(b"x").is_err());
    assert_eq!(f.commit(), Err(ErrorCode::GENERIC));
    assert_eq!(last(), (ErrorClass::NOMEMORY, "Out of memory".into()));

    // Temporary files cannot be committed; cleanup removes them.
    open_fb(
        &mut f,
        &t.path("tmp"),
        filebuf::GIT_FILEBUF_TEMPORARY,
        0o644,
    )
    .unwrap();
    let lock = f.lock_path().unwrap().to_owned();
    assert!(lock.to_bytes().starts_with(&t.path("tmp_git2_")));
    assert_eq!(f.commit(), Err(ErrorCode::GENERIC));
    assert_eq!(
        last().1,
        "unrecoverable internal error: 'file->path_original'"
    );
    f.cleanup();
    assert!(!fs_path::exists(&lock));

    assert_eq!(
        open_fb(&mut f, &t.path("sub"), 0, 0o644),
        Err(ErrorCode::DIRECTORY)
    );
    assert!(
        open_fb(
            &mut f,
            &t.path("x"),
            99 << filebuf::GIT_FILEBUF_DEFLATE_SHIFT,
            0o644
        )
        .is_err()
    );
    assert_eq!(
        last(),
        (ErrorClass::ZLIB, "failed to initialize zlib".into())
    );
}

#[test]
fn filebuf_exports() {
    let _g = test_support::init();
    let t = Scratch::new("filebuf-ffi");
    let target = t.c("out");
    let mut f = GitFilebuf::new();
    // SAFETY: a zeroed filebuf that stays in place; valid strings and
    // buffers; the filebuf is cleaned up by commit.
    unsafe {
        assert_eq!(
            git_filebuf_open(
                &mut f,
                target.as_ptr(),
                filebuf::GIT_FILEBUF_APPEND | (6 << filebuf::GIT_FILEBUF_DEFLATE_SHIFT),
                0o644
            ),
            0
        );
        for _ in 0..2000 {
            assert_eq!(
                git_filebuf_write(&mut f, b"compress me ".as_ptr().cast(), 12),
                0
            );
        }
        let mut res: *mut c_void = core::ptr::null_mut();
        assert_eq!(git_filebuf_reserve(&mut f, &mut res, 2), 0);
        assert!(!res.is_null());
        res.cast::<u8>().copy_from_nonoverlapping(b"!!".as_ptr(), 2);
        let (mut mtime, mut size) = (0, 0usize);
        assert_eq!(git_filebuf_stats(&mut mtime, &mut size, &mut f), 0);
        assert_eq!(git_filebuf_commit_at(&mut f, t.c("final").as_ptr()), 0);
        assert!(!f.has_buffer() && f.fd() == -1);
        // Without a hash, the assertion.
        let mut out = [0u8; 32];
        assert_eq!(git_filebuf_hash(out.as_mut_ptr(), &mut f), -1);
        assert_eq!(last().1, "invalid argument: 'file->compute_digest'");
        // Formatting failures poison the filebuf.
        assert_eq!(git_filebuf_open(&mut f, target.as_ptr(), 0, 0o644), 0);
        assert_eq!(
            git_filebuf__printf_formatted(&mut f, core::ptr::null(), -1),
            -1
        );
        assert_eq!(git_filebuf_write(&mut f, b"x".as_ptr().cast(), 1), -1);
        git_filebuf_cleanup(&mut f);
        assert_eq!(f.fd(), -1);
    }
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &t.c("final")).unwrap();
    // zlib output: a deflate stream header.
    assert_eq!(b.as_bytes()[0], 0x78);
    assert!(b.len() < 2000 * 12 / 10);
    assert!(!fs_path::exists(&t.c("out")) && !fs_path::exists(&t.c("out.lock")));
}

#[test]
fn filebuf_symlinks_are_followed() {
    let _g = test_support::init();
    let t = Scratch::new("filebuf-sl");
    futils::writebuffer(b"old", &t.c("real"), 0, 0).unwrap();
    posix::symlink(c"real", &t.c("l1")).unwrap();
    posix::symlink(c"l1", &t.c("l2")).unwrap();
    let mut f = GitFilebuf::new();
    open_fb(&mut f, &t.path("l2"), 0, 0o644).unwrap();
    assert_eq!(f.original_path().unwrap().to_bytes(), &t.path("real")[..]);
    f.write(b"new").unwrap();
    f.commit().unwrap();
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &t.c("l2")).unwrap();
    assert_eq!(b.as_bytes(), b"new");
    // A loop of links.
    posix::symlink(c"loop2", &t.c("loop1")).unwrap();
    posix::symlink(c"loop1", &t.c("loop2")).unwrap();
    assert!(open_fb(&mut f, &t.path("loop1"), 0, 0o644).is_err());
    assert_eq!(
        last(),
        (ErrorClass::INVALID, "maximum symlink depth reached".into())
    );
}

#[test]
fn mktmp_and_locks() {
    let _g = test_support::init();
    let t = Scratch::new("mktmp");
    let mut p = GitStr::new();
    let fd = futils::mktmp(&mut p, &t.path("f"), 0o600).unwrap();
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    assert_eq!(p.len(), t.path("f").len() + 6 + 16);
    assert_eq!(
        futils::creat_locked(&fs_path::str_cpath(&p), 0o644),
        Err(ErrorCode::LOCKED)
    );
    assert_eq!(
        futils::creat_locked(&t.c("no/x"), 0o644),
        Err(ErrorCode::NOT_FOUND)
    );
    let fd = futils::creat_locked_withpath(&t.c("no/x"), 0o755, 0o644).unwrap();
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    futils::truncate(&t.c("no/x"), 0o644).unwrap();
    let fd = futils::open_ro(&t.c("no/x")).unwrap();
    assert_eq!(futils::filesize(fd), Ok(0));
    // SAFETY: a descriptor this code opened, closed once.
    unsafe { posix::close(fd) }.unwrap();
    assert_eq!(futils::open_ro(&t.c("none")), Err(ErrorCode::NOT_FOUND));
    let _ = c_int::default();
}

// The handles are Rust types C sees as opaque: it only passes pointers to them.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn git_filebuf_printf(file: *mut c_void, format: *const c_char, ...) -> c_int;
    fn git__mmap_alignment(alignment: *mut usize) -> c_int;
    fn git__p_open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int;
    fn git_filebuf__printf_formatted(
        file: *mut GitFilebuf,
        data: *const c_char,
        len: c_int,
    ) -> c_int;
    fn git_filebuf_cleanup(file: *mut GitFilebuf);
    fn git_filebuf_commit(file: *mut GitFilebuf) -> c_int;
    fn git_filebuf_commit_at(file: *mut GitFilebuf, path: *const c_char) -> c_int;
    fn git_filebuf_hash(out: *mut u8, file: *mut GitFilebuf) -> c_int;
    fn git_filebuf_open(
        file: *mut GitFilebuf,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
    ) -> c_int;
    fn git_filebuf_open_withsize(
        file: *mut GitFilebuf,
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
        size: usize,
    ) -> c_int;
    fn git_filebuf_reserve(file: *mut GitFilebuf, buff: *mut *mut c_void, len: usize) -> c_int;
    fn git_filebuf_stats(
        mtime: *mut libc::time_t,
        size: *mut usize,
        file: *mut GitFilebuf,
    ) -> c_int;
    fn git_filebuf_write(file: *mut GitFilebuf, buff: *const c_void, len: usize) -> c_int;
    fn git_fs_path_dirload(
        contents: *mut GitVector,
        path: *const c_char,
        prefix_len: usize,
        flags: u32,
    ) -> c_int;
    fn git_fs_path_prettify(
        path_out: *mut GitStr,
        path: *const c_char,
        base: *const c_char,
    ) -> c_int;
    fn git_futils_filestamp_set(target: *mut Filestamp, source: *const Filestamp);
    fn git_futils_mkdir_relative(
        relative_path: *const c_char,
        base: *const c_char,
        mode: mode_t,
        flags: u32,
        opts: *mut MkdirOptions,
    ) -> c_int;
    fn git_futils_readbuffer(buf: *mut GitStr, path: *const c_char) -> c_int;
    fn p_getcwd(buffer_out: *mut c_char, size: usize) -> c_int;
    fn p_mmap(
        out: *mut GitMap,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> c_int;
    fn p_munmap(map: *mut GitMap) -> c_int;
    fn p_read(fd: c_int, buf: *mut c_void, cnt: usize) -> isize;
    fn p_realpath(pathname: *const c_char, resolved: *mut c_char) -> *mut c_char;
    fn p_rename(from: *const c_char, to: *const c_char) -> c_int;
    fn p_write(fd: c_int, buf: *const c_void, cnt: usize) -> c_int;
}

#[test]
fn filebuf_printf_placement_and_failures() {
    let _g = test_support::init();
    let t = Scratch::new("filebuf-printf");
    let big = "y".repeat(9000);
    let cbig = CString::new(big.clone()).unwrap();
    for size in [16usize, 8192, 10000] {
        let target = t.c(&format!("p{size}"));
        let mut f = GitFilebuf::new();
        // SAFETY: a local filebuf that stays in place until committed; the
        // formats match their arguments.
        unsafe {
            assert_eq!(
                git_filebuf_open_withsize(&mut f, target.as_ptr(), 0, 0o644, size),
                0
            );
            // Short, exactly-fitting, and longer than both the shim's stack
            // (8 KiB) and the buffer.
            assert_eq!(
                git_filebuf_printf(
                    (&raw mut f).cast(),
                    c"%s-%d\n".as_ptr(),
                    c"abc".as_ptr(),
                    42
                ),
                0
            );
            assert_eq!(
                git_filebuf_printf((&raw mut f).cast(), c"%s".as_ptr(), cbig.as_ptr()),
                0
            );
            assert_eq!(
                git_filebuf_printf((&raw mut f).cast(), c"%015d".as_ptr(), 7),
                0
            );
            assert_eq!(git_filebuf_commit(&mut f), 0);
        }
        let mut b = GitStr::new();
        futils::readbuffer(&mut b, &target).unwrap();
        assert_eq!(
            b.as_bytes(),
            format!("abc-42\n{big}000000000000007").as_bytes()
        );
    }

    // DO_NOT_BUFFER + printf: C wrote through NULL (rust/DEFECTS.md).
    let mut f = GitFilebuf::new();
    let target = t.c("unbuffered");
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            git_filebuf_open(
                &mut f,
                target.as_ptr(),
                filebuf::GIT_FILEBUF_DO_NOT_BUFFER,
                0o644
            ),
            0
        );
        assert_eq!(
            git_filebuf_printf((&raw mut f).cast(), c"%s!".as_ptr(), c"direct".as_ptr()),
            0
        );
        assert_eq!(git_filebuf_commit(&mut f), 0);
    }
    let mut b = GitStr::new();
    futils::readbuffer(&mut b, &target).unwrap();
    assert_eq!(b.as_bytes(), b"direct!");

    // A rename that fails: the commit fails with the C's message, and the
    // lock file goes.
    let mut f = GitFilebuf::new();
    open_fb(&mut f, &t.path("r"), 0, 0o644).unwrap();
    f.write(b"data").unwrap();
    assert!(f.commit_at(&t.c("missing-dir/r")).is_err());
    assert!(
        last().1.starts_with("failed to rename lockfile to '"),
        "{}",
        last().1
    );
    assert!(!fs_path::exists(&t.c("r.lock")));
}

#[test]
fn direach_of_a_borrowed_string() {
    let _g = test_support::init();
    let t = Scratch::new("borrowed");
    futils::writebuffer(b"", &t.c("x"), 0, 0).unwrap();
    let base = t.path("");
    let mut b = GitStr::new();
    // SAFETY: `base` outlives `b`'s use and is not written through it.
    unsafe { b.attach_notowned(base.as_ptr().cast(), base.len()) };
    // C appends to the borrowed buffer; the Rust cannot grow it: an error
    // ("cannot grow a borrowed buffer"), and `base` is untouched.
    let before = base.clone();
    assert!(fs_path::direach(&mut b, 0, &mut |_| 0).is_err());
    assert_eq!(base, before);
}
