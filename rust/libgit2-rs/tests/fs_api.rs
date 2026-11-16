//! The safe API of the filesystem layer (`libgit2_rs::__test::fs`), used as a Rust
//! consumer would: no `unsafe`.

#![forbid(unsafe_code)]

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use libgit2_rs::__test::fs::{self, DirIter, Filebuf, Filestamp};
use libgit2_rs::__test::util::test_support;
use libgit2_rs::{ErrorClass, ErrorCode};

#[test]
fn paths() {
    let _g = test_support::init();
    assert_eq!(fs::dirname(b"/usr/lib//").unwrap(), b"/usr");
    assert_eq!(fs::dirname(b"").unwrap(), b".");
    assert_eq!(fs::basename(b"/usr/lib//"), b"lib");
    assert_eq!(fs::basename(b"/"), b"/");
    assert_eq!(fs::root(b"/a"), Some(0));
    assert_eq!(fs::root(b"C:/a"), Some(2));
    assert_eq!(fs::root(b"a"), None);
    assert_eq!(fs::to_dir(b"a").unwrap(), b"a/");
    assert_eq!(
        fs::join_unrooted(b"x", Some(b"/base")).unwrap(),
        (b"/base/x".to_vec(), 5)
    );
    assert_eq!(
        fs::join_unrooted(b"/x", Some(b"/base")).unwrap(),
        (b"/x".to_vec(), 0)
    );
    assert_eq!(fs::squash_slashes(b"a//b///c").unwrap(), b"a/b/c");
    assert_eq!(fs::resolve_relative(b"/a/./b/../../c").unwrap(), b"/c");
    let e = fs::resolve_relative(b"/..").unwrap_err();
    assert_eq!(e.code(), ErrorCode::GENERIC);
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(e.message(), "cannot strip root component off url");
    assert_eq!(fs::apply_relative(b"a/b", b"../c").unwrap(), b"a/c");
    assert_eq!(fs::make_relative(b"/a/x", b"/a/b").unwrap(), b"../x");
    assert_eq!(
        fs::make_relative(b"a", b"b").unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );
    assert_eq!(fs::common_dirlen(b"a/b/c", b"a/b/d"), 4);
    assert_eq!(fs::percent_decode(b"a%20b").unwrap(), b"a b");
    assert!(fs::is_local_file_url(b"file:///x"));
    assert_eq!(fs::from_url(b"file:///x%41").unwrap(), b"/xA");
    assert_eq!(fs::from_url_or_path(b"rel").unwrap(), b"rel");
    assert!(fs::is_valid(b"a/b", fs::REJECT_FILESYSTEM_DEFAULTS));
    assert!(!fs::is_valid(b"a/./b", fs::REJECT_TRAVERSAL));
    assert!(!fs::is_valid(b"AUX", fs::REJECT_DOS_PATHS));
    assert!(!fs::is_valid_with(
        b"a/.git/b",
        0x1,
        |_| true,
        |c| c != b".git"
    ));
    let mut seen = Vec::new();
    fs::walk_up::<()>(b"a/b/c", None, |p| {
        seen.push(p.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        seen,
        [
            b"a/b/c".to_vec(),
            b"a/b/".to_vec(),
            b"a/".to_vec(),
            b"".to_vec()
        ]
    );
    assert_eq!(
        fs::walk_up(b"a/b/c", None, |p| if p == b"a/b/" {
            Err(3)
        } else {
            Ok(())
        }),
        Err(3)
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("u3-api-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn p(&self, rel: &str) -> Vec<u8> {
        let mut v = self.0.as_os_str().as_bytes().to_vec();
        if !rel.is_empty() {
            v.push(b'/');
            v.extend_from_slice(rel.as_bytes());
        }
        v
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn files_and_dirs() {
    let _g = test_support::init();
    let t = Scratch::new("files");
    fs::mkdir(&t.p("a/b"), 0o755, fs::MKDIR_PATH).unwrap();
    assert!(fs::is_dir(&t.p("a/b")) && fs::is_empty_dir(&t.p("a/b")));
    fs::write(&t.p("a/f"), b"data", 0o644, true).unwrap();
    assert!(fs::is_file(&t.p("a/f")) && fs::exists(&t.p("a/f")) && !fs::is_link(&t.p("a/f")));
    assert_eq!(fs::read(&t.p("a/f")).unwrap(), b"data");
    let e = fs::read(&t.p("a/none")).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.class(), ErrorClass::OS);
    assert!(
        e.message()
            .ends_with("/a/none' to stat: No such file or directory")
    );
    let mut sum = [0u8; 32];
    assert_eq!(
        fs::read_if_changed(&t.p("a/f"), &mut sum)
            .unwrap()
            .as_deref(),
        Some(&b"data"[..])
    );
    assert_eq!(fs::read_if_changed(&t.p("a/f"), &mut sum).unwrap(), None);

    let mut names: Vec<Vec<u8>> = DirIter::new(&t.p("a"), 0)
        .unwrap()
        .map(|e| e.unwrap().name().to_vec())
        .collect();
    names.sort();
    assert_eq!(names, [b"b".to_vec(), b"f".to_vec()]);
    let mut rel = fs::read_dir(&t.p("a"), t.p("").len() + 1).unwrap();
    rel.sort();
    assert_eq!(rel, [b"a/b".to_vec(), b"a/f".to_vec()]);
    assert!(DirIter::new(&t.p("none"), 0).is_err());

    let mut st = Filestamp::default();
    assert!(st.check(&t.p("a/f")).unwrap());
    assert!(!st.check(&t.p("a/f")).unwrap());
    assert_eq!(
        st.check(&t.p("none")).unwrap_err().code(),
        ErrorCode::NOT_FOUND
    );

    fs::copy_tree(&t.p("a"), &t.p("c"), fs::CPDIR_CREATE_EMPTY_DIRS, 0o755).unwrap();
    assert!(fs::is_dir(&t.p("c/b")) && fs::is_file(&t.p("c/f")));
    fs::copy(&t.p("a/f"), &t.p("g"), 0o600).unwrap();
    fs::rename_with_path(&t.p("g"), &t.p("h/i/g"), 0o755).unwrap();
    assert!(fs::is_file(&t.p("h/i/g")));
    let perf = fs::mkdir_relative(b"x/y", Some(&t.p("")), 0o755, fs::MKDIR_PATH, None).unwrap();
    assert_eq!(perf.mkdir_calls, 2);
    fs::rmdir_r(&t.p("c"), None, fs::RMDIR_REMOVE_FILES).unwrap();
    assert!(!fs::exists(&t.p("c")));
    let e = fs::rmdir_r(&t.p("a"), None, fs::RMDIR_EMPTY_HIERARCHY).unwrap_err();
    assert!(e.message().contains("still present"));
    fs::mkdir_p(&t.p("p/q"), 0o755).unwrap();

    let pretty = fs::prettify(b"a/../a", Some(&t.p(""))).unwrap();
    assert!(pretty.ends_with(b"/a"));
    assert!(fs::prettify_dir(&t.p("a"), None).unwrap().ends_with(b"/a/"));
    assert!(fs::owner_is_current_user(&t.p("a")).unwrap());
    assert!(fs::find_executable(b"sh").unwrap().ends_with(b"/sh"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn lock_files() {
    let _g = test_support::init();
    let t = Scratch::new("lock");
    let target = t.p("out");
    let mut f = Filebuf::open(&target, fs::FILEBUF_HASH_SHA256, 0o644).unwrap();
    assert!(f.lock_path().unwrap().ends_with(b"/out.lock"));
    assert_eq!(
        Filebuf::open(&target, 0, 0o644).unwrap_err().code(),
        ErrorCode::LOCKED
    );
    writeln!(f, "hello {}", 42).unwrap();
    f.write(b"more").unwrap();
    f.reserve(2).unwrap().copy_from_slice(b"!!");
    let h = f.hash().unwrap();
    assert_ne!(h, [0u8; 32]);
    f.commit().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"hello 42\nmore!!");

    // Dropped without commit: the lock file goes, the target stays.
    {
        let mut g = Filebuf::open(&target, 0, 0o644).unwrap();
        g.write(b"discarded").unwrap();
    }
    assert!(!fs::exists(&[target.clone(), b".lock".to_vec()].concat()));
    assert_eq!(fs::read(&target).unwrap(), b"hello 42\nmore!!");

    let mut z = Filebuf::open(
        &target,
        fs::FILEBUF_APPEND | (1 << fs::FILEBUF_DEFLATE_SHIFT),
        0o644,
    )
    .unwrap();
    z.write(b"!").unwrap();
    z.commit_at(&t.p("z")).unwrap();
    // APPEND copies the old contents as they are; the new bytes are
    // deflated after them.
    let z = fs::read(&t.p("z")).unwrap();
    assert!(z.starts_with(b"hello 42\nmore!!\x78"));
}

#[test]
#[cfg_attr(miri, ignore)]
fn nul_bytes_are_refused() {
    let _g = test_support::init();
    let e = fs::write(b"/tmp/a\0b", b"x", 0o644, false).unwrap_err();
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(e.message(), "path contains a NUL byte");
    assert!(fs::owner_is_current_user(b"/tmp\0/../etc").is_err());
    assert!(!fs::exists(b"/tmp\0x"));
    assert!(fs::dirname(b"a\0/b").is_err());
    let mut st = Filestamp::default();
    let e = st.check(b"/nonexistent-u3/x").unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert!(
        e.message()
            .starts_with("could not find '/nonexistent-u3/x' to stat")
    );
}
