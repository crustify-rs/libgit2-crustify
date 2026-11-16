//! The safe API of work package u1 (oid, oid arrays, the shortener, maps,
//! pool, pqueue, parse context, utf8, varint), used as a Rust consumer
//! would: no `unsafe`.

#![forbid(unsafe_code)]

use libgit2_rs::__test::hashmap_oid::OidMap;
use libgit2_rs::__test::parse::GitParseCtx;
use libgit2_rs::__test::util::array::GitArray;
use libgit2_rs::__test::util::hashmap::{CStrKey, GitHashMap, GitHashSet};
use libgit2_rs::__test::util::pool::GitPool;
use libgit2_rs::__test::util::test_support;
use libgit2_rs::__test::util::{utf8, varint};
use libgit2_rs::{
    __test::oidarray, __test::oidarray::OidArray, Error, ErrorClass, Oid, OidShortener, OidType,
};

const A: &str = "8b137891791fe96927ad78e64b0aad7bded08bdc";
const B: &str = "8b137891791fe96927ad78e64b0aad7bded08bdd";

#[test]
fn oid_round_trips_and_errors() -> Result<(), Error> {
    let _g = test_support::init();
    let a = Oid::from_str_hex(A, OidType::Sha1)?;
    let b = Oid::from_str_hex(B, OidType::Sha1)?;
    assert!(a < b);
    assert_eq!(a.kind(), Some(OidType::Sha1));
    assert_eq!(a.to_string(), A);
    assert_eq!(a.as_bytes().len(), 20);
    assert_eq!(Oid::from_bytes(a.as_bytes(), OidType::Sha1), Some(a));
    assert_eq!(a.ncmp(&b, 39), 0);
    assert_ne!(a.ncmp(&b, 40), 0);
    assert!(a.streq(A.as_bytes()));
    assert_eq!(a.hex_prefix(8).as_str(), "8b13789");
    assert_eq!(a.hex_path()?.as_str().as_bytes()[2], b'/');
    assert_eq!(Oid::from_prefix(b"8b13", OidType::Sha1)?, a.prefix(4));

    let e: Error = Oid::from_str_hex("xyz", OidType::Sha1).unwrap_err().into();
    assert_eq!(e.class(), ErrorClass::INVALID);
    assert_eq!(
        e.message(),
        "unable to parse OID - contains invalid characters"
    );
    let e: Error = Oid::from_prefix(b"", OidType::Sha256).unwrap_err().into();
    assert_eq!(e.message(), "unable to parse OID - too short");

    let z = Oid::zero(OidType::Sha256);
    assert!(z.is_zero());
    assert_eq!(z.hex().as_str().len(), 64);
    assert_eq!(
        Oid::EMPTY_TREE_SHA1.to_string(),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
    );
    assert_eq!(format!("{a:?}"), format!("Oid(sha1:{A})"));
    Ok(())
}

#[test]
fn shortener_and_arrays() -> Result<(), Error> {
    let _g = test_support::init();
    let mut s = OidShortener::new(4);
    assert_eq!(s.add(A.as_bytes())?, 4);
    assert_eq!(s.add(B.as_bytes())?, 40);
    assert_eq!(s.min_length(), 40);

    let a = Oid::from_str_hex(A, OidType::Sha1)?;
    let b = Oid::from_str_hex(B, OidType::Sha1)?;
    let mut arr = GitArray::new();
    oidarray::add(&mut arr, &a)?;
    oidarray::add(&mut arr, &b)?;
    oidarray::add(&mut arr, &a)?;
    let mut ids = OidArray::from_array(arr);
    assert_eq!(&ids[..], &[a, b]);
    ids.reverse();
    assert_eq!(ids.first(), Some(&b));
    let mut back = ids.into_array();
    assert!(oidarray::remove(&mut back, &b));
    assert_eq!(back.as_slice(), &[a]);
    Ok(())
}

#[test]
fn maps() -> Result<(), Error> {
    let _g = test_support::init();
    let ids: Vec<Oid> = (0..100u8)
        .map(|i| Oid::from_prefix(format!("{i:02x}").as_bytes(), OidType::Sha1))
        .collect::<Result<_, _>>()?;
    let mut m: OidMap<'_, usize> = OidMap::new();
    for (i, id) in ids.iter().enumerate() {
        m.put(id, i)?;
    }
    assert_eq!(m.get(&&ids[42]), Some(&42));
    m.retain(|_, v| *v % 2 == 0);
    assert_eq!(m.len(), 50);
    let mut owned: GitHashMap<Oid, String> = GitHashMap::new();
    owned.put(ids[1], "one".into())?;
    assert_eq!(
        owned.remove(&ids[1]).map(|(_, v)| v),
        Some("one".to_string())
    );

    let mut set: GitHashSet<CStrKey<'_>> = GitHashSet::new();
    set.add(CStrKey::new(c"a"))?;
    set.add(CStrKey::new(c"b"))?;
    assert!(set.contains(&CStrKey::new(c"a")));
    assert_eq!(set.keys().count(), 2);
    Ok(())
}

#[test]
fn pool_and_parse() -> Result<(), Error> {
    let _g = test_support::init();
    let mut pool = GitPool::with_item_size(1)?;
    let p = pool.strndup(b"hello")?;
    assert!(pool.ptr_in_pool(p.as_ptr().cast()));
    assert_eq!(pool.open_pages(), 1);
    pool.clear();
    assert_eq!(pool.open_pages(), 0);

    let text = format!("{A} refs/heads/main\n[core]\n");
    let mut ctx = GitParseCtx::new(text.as_bytes());
    assert_eq!(ctx.advance_oid(OidType::Sha1)?.to_string(), A);
    ctx.advance_ws()?;
    assert!(ctx.contains(b"refs/"));
    ctx.advance_line();
    assert_eq!(ctx.peek(true), Some(b'['));
    assert_eq!(ctx.line_num(), 2);
    Ok(())
}

#[test]
fn utf8_and_varint() {
    assert_eq!(utf8::char_length("héllo".as_bytes()), 5);
    assert_eq!(utf8::iterate("é".as_bytes()), Some((0xe9, 2)));
    let mut buf = [0u8; 10];
    let n = varint::encode(Some(&mut buf), 300) as usize;
    assert_eq!(varint::decode(&buf[..n]), Some((300, n)));
}

/// Review U1-1: an add that fails after reaching a leaf leaves a usable set.
#[test]
fn shortener_failed_add_mid_walk() {
    let _g = test_support::init();
    let a: &[u8] = b"abcdef0123456789abcdef0123456789abcdef01";
    let mut s = OidShortener::new(0);
    assert_eq!(s.add(a).ok(), Some(1));
    assert!(s.add(b"ax").is_err());
    assert_eq!(
        s.add(b"abffffffffffffffffffffffffffffffffffffff").ok(),
        Some(2)
    );
    assert_eq!(
        s.add(b"a4ffffffffffffffffffffffffffffffffffffff").ok(),
        Some(2)
    );
}
