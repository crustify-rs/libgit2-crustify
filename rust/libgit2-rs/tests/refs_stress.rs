//! The safe API of references from several threads on one `Repository`
//! (`tests/libgit2/threads/refdb.c` through Rust), and backend
//! replacement, which needs the repository exclusively (review l-refs-r1,
//! LR-1; adapted from its reproducer).

#![forbid(unsafe_code)]
#![cfg(feature = "link-c")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use libgit2_rs::refs_api::{self, RefValue, Repository, set_refdb_backend};
use libgit2_rs::util::test_support;
use libgit2_rs::{Error, ErrorCode, Oid, OidType, RefdbBackend, Reference, References};

const MASTER: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const BR2: &str = "a4a7dce85cf63874e984719f4fdd239f5145052f";

fn oid(s: &str) -> Oid {
    Oid::from_hex(s.as_bytes(), OidType::Sha1).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

fn sandbox(tag: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources/testrepo.git");
    let dst = std::env::temp_dir().join(format!("l-refs-stress-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(&src, &dst);
    dst
}

#[test]
fn lookups_vs_writes_vs_pack() {
    let _g = test_support::init();
    let dir = sandbox("lwp");
    let repo = Repository::open(&dir).unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        for t in 0..4 {
            let (repo, stop) = (&repo, &stop);
            s.spawn(move || {
                let mut i = 0u32;
                while !stop.load(Ordering::Relaxed) {
                    let name = format!("refs/heads/thread-{t}-{}", i % 20);
                    let id = oid(if i.is_multiple_of(2) { MASTER } else { BR2 });
                    let _ = Reference::create(repo, name.as_bytes(), &id, true, None, None);
                    assert!(Reference::lookup(repo, b"refs/heads/master").is_ok());
                    let _ = Reference::lookup(repo, name.as_bytes()).map(|r| r.delete());
                    i += 1;
                }
            });
        }
        for _ in 0..2 {
            let (repo, stop) = (&repo, &stop);
            s.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(it) = References::new(repo) {
                        for r in it {
                            let _ = r;
                        }
                    }
                    assert!(Reference::lookup(repo, b"refs/heads/packed").is_ok());
                }
            });
        }
        {
            let (repo, stop) = (&repo, &stop);
            s.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = refs_api::compress(repo);
                }
            });
        }
        let secs = std::env::var("STRESS_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        std::thread::sleep(std::time::Duration::from_secs(secs));
        stop.store(true, Ordering::Relaxed);
    });
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}

struct Empty;

impl RefdbBackend for Empty {
    fn exists(&self, _: &[u8]) -> Result<bool, Error> {
        Ok(false)
    }
    fn lookup(&self, _: &[u8]) -> Result<Option<RefValue>, Error> {
        Ok(None)
    }
    fn list(&self) -> Result<Vec<(Vec<u8>, RefValue)>, Error> {
        Ok(Vec::new())
    }
    fn write(
        &self,
        _: &[u8],
        _: &RefValue,
        _: bool,
        _: Option<&Oid>,
        _: Option<&[u8]>,
        _: Option<&[u8]>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn rename(&self, _: &[u8], _: &[u8], _: bool) -> Result<RefValue, Error> {
        Ok(RefValue::Symbolic(b"x".to_vec()))
    }
    fn delete(&self, _: &[u8], _: Option<&Oid>, _: Option<&[u8]>) -> Result<(), Error> {
        Ok(())
    }
}

/// Replacing the backend takes `&mut Repository`: between replacements,
/// threads may share the repository, but not across one (a concurrent
/// replacement no longer compiles; see `set_refdb_backend`'s doctest).
#[test]
fn set_backend_between_shared_phases() {
    let _g = test_support::init();
    let dir = sandbox("sb");
    let mut repo = Repository::open(&dir).unwrap();
    set_refdb_backend(&mut repo, Empty).unwrap();
    for _ in 0..50 {
        std::thread::scope(|s| {
            for _ in 0..3 {
                let repo = &repo;
                s.spawn(move || {
                    for _ in 0..20 {
                        let e = Reference::lookup(repo, b"refs/heads/master").unwrap_err();
                        assert_eq!(e.code(), ErrorCode::NOT_FOUND);
                    }
                });
            }
        });
        set_refdb_backend(&mut repo, Empty).unwrap();
    }
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
}
