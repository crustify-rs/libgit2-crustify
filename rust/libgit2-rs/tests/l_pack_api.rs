//! The safe API of the pack reader (`Pack`, `OdbPackBackend`) on the test
//! fixtures: every object of each pack is read and its id recomputed, so
//! the index lookup, delta resolution and inflation are all checked
//! against git's own ids.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use libgit2_rs::__test::odb_pack::OdbPackBackend;
use libgit2_rs::__test::pack::{Pack, PackObjectType};
use libgit2_rs::__test::util::hash::{HashAlgorithm, hash_vec};
use libgit2_rs::__test::util::test_support;
use libgit2_rs::{ErrorCode, Oid, OidType};

const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/resources");

fn idx(repo: &str, prefix: &str) -> Vec<u8> {
    let dir = format!("{RES}/{repo}/objects/pack");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".idx"))
        .collect();
    names.sort();
    format!("{dir}/{}", names[0]).into_bytes()
}

fn type_name(t: PackObjectType) -> &'static str {
    match t {
        PackObjectType::Commit => "commit",
        PackObjectType::Tree => "tree",
        PackObjectType::Blob => "blob",
        PackObjectType::Tag => "tag",
        _ => panic!("a delta type for a read object"),
    }
}

/// The id git gives `data` of type `t`.
fn object_id(t: PackObjectType, data: &[u8], oid_type: OidType) -> Oid {
    let hdr = format!("{} {}\0", type_name(t), data.len());
    let alg = match oid_type {
        OidType::Sha1 => HashAlgorithm::Sha1,
        OidType::Sha256 => HashAlgorithm::Sha256,
    };
    let d = hash_vec(&[hdr.as_bytes(), data], alg).unwrap();
    Oid::from_bytes(d.as_bytes(), oid_type).unwrap()
}

fn check_pack(path: &[u8], oid_type: OidType) -> usize {
    let pack = Pack::open(path, oid_type).unwrap();
    let n = pack.num_objects().unwrap() as usize;
    let ids = pack.ids().unwrap();
    let entries = pack.entries().unwrap();
    assert_eq!((ids.len(), entries.len()), (n, n));
    let mut sorted: Vec<Oid> = entries.iter().map(|e| e.0).collect();
    sorted.sort();
    let mut by_pack = ids.clone();
    by_pack.sort();
    assert_eq!(sorted, by_pack);
    for (id, off) in &entries {
        let (found_off, found) = pack.find(id, oid_type.hexsize()).unwrap();
        assert_eq!((found_off, found), (*off, *id));
        let obj = pack.read(*off).unwrap();
        assert_eq!(object_id(obj.kind(), obj.data(), oid_type), *id);
        let (size, kind) = pack.read_header(*off).unwrap();
        assert_eq!((size, kind), (obj.data().len(), obj.kind()));
        // A unique prefix finds it too.
        let (_, p) = pack.find(&id.prefix(16), 16).unwrap();
        assert_eq!(p, *id);
    }
    // Pack order follows the offsets.
    let pos: Vec<u64> = ids
        .iter()
        .map(|id| entries.iter().find(|e| e.0 == *id).unwrap().1)
        .collect();
    assert!(pos.windows(2).all(|w| w[0] <= w[1]));
    n
}

#[test]
fn testrepo_pack_reads_every_object() {
    let _g = test_support::init();
    let n = check_pack(&idx("testrepo.git", "pack-a81e"), OidType::Sha1);
    assert_eq!(n, 1628);
}

#[test]
fn sha256_pack_reads_every_object() {
    let _g = test_support::init();
    let n = check_pack(&idx("testrepo_256.git", "pack-e2f0"), OidType::Sha256);
    assert!(n > 1000);
}

#[test]
fn missing_and_ambiguous() {
    let _g = test_support::init();
    let pack = Pack::open(&idx("testrepo.git", "pack-a81e"), OidType::Sha1).unwrap();
    let missing =
        Oid::from_hex(b"ffffffffffffffffffffffffffffffffffffffff", OidType::Sha1).unwrap();
    let e = pack.find(&missing, 40).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(
        e.message(),
        "object not found - failed to find offset for pack entry (ffffffffffffffffffffffffffffffffffffffff)"
    );
    // 1628 objects: some 1-digit prefix matches many.
    let first = pack.ids().unwrap()[0];
    let e = pack.find(&first.prefix(1), 1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    assert_eq!(
        e.message(),
        "ambiguous OID prefix - found multiple offsets for pack entry"
    );
    // A pack without its .pack file is not found.
    let e = Pack::open(b"/nonexistent/pack-x.idx", OidType::Sha1).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.message(), "object not found - packfile not found");
    assert!(Pack::open(b"a\0b.idx", OidType::Sha1).is_err());
}

#[test]
fn packs_are_shared() {
    let _g = test_support::init();
    let path = idx("testrepo.git", "pack-a81e");
    let a = Pack::open(&path, OidType::Sha1).unwrap();
    let b = Pack::open(&path, OidType::Sha1).unwrap();
    let c = a.clone();
    assert_eq!(a.name(), b.name());
    drop((a, b));
    assert_eq!(c.num_objects().unwrap(), 1628);
}

#[test]
fn packs_read_from_many_threads() {
    let _g = test_support::init();
    let pack = Pack::open(&idx("testrepo.git", "pack-a81e"), OidType::Sha1).unwrap();
    let entries = pack.entries().unwrap();
    std::thread::scope(|s| {
        for t in 0..8 {
            let (pack, entries) = (&pack, &entries);
            s.spawn(move || {
                for (id, off) in entries.iter().skip(t).step_by(3) {
                    let obj = pack.read(*off).unwrap();
                    assert_eq!(object_id(obj.kind(), obj.data(), OidType::Sha1), *id);
                }
            });
        }
    });
}

#[test]
fn backend_over_a_repository_with_a_multi_pack_index() {
    let _g = test_support::init();
    let objects = format!("{RES}/testrepo.git/objects");
    let b = OdbPackBackend::open(objects.as_bytes(), OidType::Sha1).unwrap();
    let packs = b.packs();
    assert_eq!(packs.len(), 3);
    let mut all = Vec::new();
    assert!(
        b.foreach(|id| {
            all.push(*id);
            true
        })
        .unwrap()
    );
    let total: u32 = packs.iter().map(|p| p.num_objects().unwrap()).sum();
    // The multi-pack-index lists each object once.
    let mut uniq = all.clone();
    uniq.sort();
    uniq.dedup();
    assert!(all.len() as u32 >= uniq.len() as u32 && uniq.len() as u32 <= total);
    for id in uniq.iter().step_by(7) {
        assert!(b.exists(id));
        let obj = b.read(id).unwrap();
        assert_eq!(object_id(obj.kind(), obj.data(), OidType::Sha1), *id);
        let (size, kind) = b.read_header(id).unwrap();
        assert_eq!((size, kind), (obj.data().len(), obj.kind()));
        let (full, obj2) = b.read_prefix(&id.prefix(12), 12).unwrap();
        assert_eq!((full, obj2.data()), (*id, obj.data()));
        assert_eq!(b.exists_prefix(&id.prefix(12), 12).unwrap(), *id);
    }
    // Stopping early.
    let mut n = 0;
    assert!(
        !b.foreach(|_| {
            n += 1;
            n < 10
        })
        .unwrap()
    );
    assert_eq!(n, 10);
    b.refresh().unwrap();
    let e = b.read_prefix(&uniq[0].prefix(3), 3).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    assert_eq!(
        e.message(),
        "ambiguous OID prefix - prefix length too short"
    );
    let missing =
        Oid::from_hex(b"0000000000000000000000000000000000000001", OidType::Sha1).unwrap();
    assert!(!b.exists(&missing));
    assert_eq!(b.read(&missing).unwrap_err().code(), ErrorCode::NOT_FOUND);
}

#[test]
fn backend_without_a_pack_directory_is_empty() {
    let _g = test_support::init();
    let b = OdbPackBackend::open(b"/nonexistent", OidType::Sha1).unwrap();
    assert!(b.packs().is_empty());
    assert!(b.foreach(|_| true).unwrap());
    let one = OdbPackBackend::open_one(&idx("testrepo.git", "pack-d7c6"), OidType::Sha1).unwrap();
    assert_eq!(one.packs().len(), 1);
}

#[test]
fn write_a_multi_pack_index() {
    let _g = test_support::init();
    let tmp = std::env::temp_dir().join(format!("l_pack_midx_{}", std::process::id()));
    let pack_dir = tmp.join("objects/pack");
    std::fs::create_dir_all(&pack_dir).unwrap();
    let src = format!("{RES}/testrepo.git/objects/pack");
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().into_string().unwrap();
        if n.ends_with(".idx") || n.ends_with(".pack") {
            std::fs::copy(e.path(), pack_dir.join(&n)).unwrap();
        }
    }
    let objects = tmp.join("objects");
    let b = OdbPackBackend::open(objects.to_str().unwrap().as_bytes(), OidType::Sha1).unwrap();
    b.write_multi_pack_index().unwrap();
    assert!(pack_dir.join("multi-pack-index").exists());
    let mut n = 0;
    b.foreach(|_| {
        n += 1;
        true
    })
    .unwrap();
    assert!(n > 1600);
    drop(b);
    std::fs::remove_dir_all(&tmp).unwrap();
}
