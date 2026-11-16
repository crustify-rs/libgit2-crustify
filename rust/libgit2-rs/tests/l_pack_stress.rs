//! Threads against the process-wide window limits (review L-PACK-2, -7):
//! several packs, a file limit of 1, a small window and mapped limit, and
//! threads that open, read (verifying every object) and free backends, so
//! that packs are opened, closed by the file limit, re-opened and freed
//! concurrently. Afterwards every open pack file must be registered (so
//! that the limits apply to it), and the mapped limit must hold once idle.
//! (Adapted from the reviewer's `stress_pack.c`; also run under TSan.)
//!
//! Its own test binary: it changes the process's window limits.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use std::sync::atomic::Ordering;

use libgit2_rs::mwindow::{
    self, git_mwindow__file_limit, git_mwindow__mapped_limit, git_mwindow__window_size,
};
use libgit2_rs::odb_pack::OdbPackBackend;
use libgit2_rs::pack::PackObjectType;
use libgit2_rs::util::hash::{HashAlgorithm, hash_vec};
use libgit2_rs::util::test_support;
use libgit2_rs::{Oid, OidType};

const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/resources");
const MAPPED: usize = 32 * 1024;

/// A directory of SHA-1 packs from several fixtures.
fn pack_dir() -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("l_pack_stress_{}", std::process::id()));
    let dir = tmp.join("objects/pack");
    std::fs::create_dir_all(&dir).unwrap();
    for src in [
        "testrepo.git/objects/pack",
        "revwalk.git/objects/pack",
        "redundant.git/objects/pack",
        "attr_index/.gitted/objects/pack",
        "testrepo.git",
    ] {
        let src = std::path::Path::new(RES).join(src);
        for e in std::fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_owned();
            let is_pack = p.extension().is_some_and(|x| x == "idx" || x == "pack");
            if is_pack && !dir.join(&name).exists() {
                std::fs::copy(&p, dir.join(&name)).unwrap();
            }
        }
    }
    // The big pack again under other names: more packs that hold the same
    // objects, used in turn (`last_found`), so files compete for the limit.
    let big = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("pack-a81e")
                && p.extension().is_some_and(|x| x == "pack")
        })
        .unwrap();
    for i in 0..12 {
        let stem = format!("pack-{:040x}", 0xc0ffee00u64 + i);
        std::fs::copy(&big, dir.join(format!("{stem}.pack"))).unwrap();
        std::fs::copy(big.with_extension("idx"), dir.join(format!("{stem}.idx"))).unwrap();
    }
    tmp
}

fn check(b: &OdbPackBackend, id: &Oid) {
    let obj = b.read(id).unwrap();
    let t = match obj.kind() {
        PackObjectType::Commit => "commit",
        PackObjectType::Tree => "tree",
        PackObjectType::Blob => "blob",
        PackObjectType::Tag => "tag",
        k => panic!("{k:?}"),
    };
    let hdr = format!("{t} {}\0", obj.data().len());
    let d = hash_vec(&[hdr.as_bytes(), obj.data()], HashAlgorithm::Sha1).unwrap();
    assert_eq!(Oid::from_bytes(d.as_bytes(), OidType::Sha1).unwrap(), *id);
    let (size, kind) = b.read_header(id).unwrap();
    assert_eq!((size, kind), (obj.data().len(), obj.kind()));
    assert!(b.exists(id));
}

#[test]
fn limits_hold_under_threads() {
    let _g = test_support::init();
    git_mwindow__file_limit.store(1, Ordering::Relaxed);
    git_mwindow__window_size.store(8192, Ordering::Relaxed);
    git_mwindow__mapped_limit.store(MAPPED, Ordering::Relaxed);

    let tmp = pack_dir();
    let objects = tmp.join("objects");
    let objects = objects.to_str().unwrap().as_bytes();
    // Held throughout, so that the packs (and any file the limit failed to
    // keep track of) outlive the threads' backends.
    let keep = OdbPackBackend::open(objects, OidType::Sha1).unwrap();
    let ids = {
        let b = &keep;
        assert!(b.packs().len() >= 5, "several packs");
        let mut ids = Vec::new();
        b.foreach(|id| {
            ids.push(*id);
            true
        })
        .unwrap();
        ids.sort();
        ids.dedup();
        ids
    };

    std::thread::scope(|s| {
        for t in 0..16 {
            let ids = &ids;
            s.spawn(move || {
                for round in 0..4 {
                    let b = OdbPackBackend::open(objects, OidType::Sha1).unwrap();
                    for id in ids.iter().skip((t + round) % 5).step_by(5) {
                        check(&b, id);
                    }
                }
            });
        }
    });

    // Idle: one more pass on one thread, then the accounting.
    let b = keep;
    for id in ids.iter().step_by(3) {
        check(&b, id);
    }
    let st = mwindow::stats();
    assert!(st.mapped <= MAPPED, "{st:?}");
    assert_eq!(st.open_pack_files, st.registered_pack_files, "{st:?}");
    drop(b);
    std::fs::remove_dir_all(&tmp).unwrap();
}
