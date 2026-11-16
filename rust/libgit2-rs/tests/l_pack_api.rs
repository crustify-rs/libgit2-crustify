//! The pack backends through the safe API (root items only, no `unsafe`,
//! no init): every object of the fixtures' packs is read and its id
//! recomputed, so the index lookup, delta resolution and inflation are
//! checked against git's own ids, from one thread and from many. The
//! pack reader's internals are tested in `l_pack_internals.rs`.

#![forbid(unsafe_code)]
#![cfg(not(miri))]

use libgit2_rs::{Backend, ErrorCode, ObjectType, Odb, Oid, OidType};

const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/resources");

fn idx(repo: &str, prefix: &str) -> String {
    let dir = format!("{RES}/{repo}/objects/pack");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".idx"))
        .collect();
    names.sort();
    format!("{dir}/{}", names[0])
}

fn ids(odb: &Odb<'_>) -> Vec<Oid> {
    let mut v = Vec::new();
    odb.foreach(|id| {
        v.push(*id);
        true
    })
    .unwrap();
    v.sort();
    v.dedup();
    v
}

fn check(odb: &Odb<'_>, id: &Oid, oid_type: OidType) {
    let o = odb.read(id).unwrap();
    assert_eq!(Odb::hash(o.data(), o.kind(), oid_type).unwrap(), *id);
    assert_eq!(odb.read_header(id).unwrap(), (o.len(), o.kind()));
    // A unique prefix finds it too.
    let p = odb.read_prefix(&id.prefix(16), 16).unwrap();
    assert_eq!(p.id(), id);
    assert_eq!(odb.exists_prefix(&id.prefix(16), 16).unwrap(), *id);
}

fn one_pack(repo: &str, prefix: &str, oid_type: OidType) -> Odb<'static> {
    let odb = Odb::with_oid_type(oid_type).unwrap();
    odb.add_backend(Backend::one_pack(idx(repo, prefix), oid_type).unwrap(), 1)
        .unwrap();
    odb
}

#[test]
fn testrepo_pack_reads_every_object() {
    let odb = one_pack("testrepo.git", "pack-a81e", OidType::Sha1);
    let all = ids(&odb);
    assert_eq!(all.len(), 1628);
    for id in &all {
        check(&odb, id, OidType::Sha1);
    }
}

#[test]
fn sha256_pack_reads_every_object() {
    let odb = one_pack("testrepo_256.git", "pack-e2f0", OidType::Sha256);
    let all = ids(&odb);
    assert!(all.len() > 1000);
    for id in &all {
        check(&odb, id, OidType::Sha256);
    }
    assert!(
        odb.read(&Oid::zero(OidType::Sha256)).is_err(),
        "the zero id is never found"
    );
}

#[test]
fn missing_and_ambiguous() {
    let odb = one_pack("testrepo.git", "pack-a81e", OidType::Sha1);
    let missing = Oid::from_hex("ffffffffffffffffffffffffffffffffffffffff", OidType::Sha1).unwrap();
    let e = odb.read(&missing).unwrap_err();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert!(!odb.exists(&missing));
    let all = ids(&odb);
    // Too short for the odb.
    let e = odb.read_prefix(&all[0].prefix(3), 3).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    assert_eq!(
        e.message(),
        "ambiguous OID prefix - prefix length too short"
    );
    // Two ids with the same first 4 digits (1628 ids: there are some).
    let pair = all
        .windows(2)
        .find(|w| w[0].ncmp(&w[1], 4).is_eq())
        .expect("a shared 4-digit prefix");
    let e = odb.read_prefix(&pair[0].prefix(4), 4).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
    let e = odb.exists_prefix(&pair[0].prefix(4), 4).unwrap_err();
    assert_eq!(e.code(), ErrorCode::AMBIGUOUS);
}

#[test]
fn packs_read_from_many_threads() {
    let odb = Odb::new().unwrap();
    odb.add_backend(
        Backend::pack(format!("{RES}/testrepo.git/objects"), OidType::Sha1).unwrap(),
        1,
    )
    .unwrap();
    let all = ids(&odb);
    assert!(all.len() >= 1628);
    std::thread::scope(|s| {
        for t in 0..8 {
            let (odb, all) = (odb.clone(), &all);
            s.spawn(move || {
                for id in all.iter().skip(t).step_by(3) {
                    let o = odb.read(id).unwrap();
                    assert_eq!(Odb::hash(o.data(), o.kind(), OidType::Sha1).unwrap(), *id);
                }
            });
        }
    });
    // Commits, trees and blobs are all there.
    let mut kinds: Vec<ObjectType> = all
        .iter()
        .map(|id| odb.read_header(id).unwrap().1)
        .collect();
    kinds.sort();
    kinds.dedup();
    for k in [ObjectType::Commit, ObjectType::Tree, ObjectType::Blob] {
        assert!(kinds.contains(&k), "{kinds:?}");
    }
}
