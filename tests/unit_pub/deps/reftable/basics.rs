//! Unit tests for `deps/reftable/basics.c`'s types: `HashId`'s conversions and their error,
//! and what an empty `Buf` reports, through every handle and in its `Debug` form.

use std::collections::HashSet;
use std::error::Error;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{Buf, HashId, UnnamedHashId};

#[test]
fn hash_id_converts_from_and_to_its_c_value() {
    assert_eq!(HashId::from_raw(89), Some(HashId::Sha1));
    assert_eq!(HashId::from_raw(247), Some(HashId::Sha256));
    assert_eq!(HashId::Sha1.raw(), 89);
    assert_eq!(HashId::Sha256.raw(), 247);
    assert_eq!(ffi::reftable_hash::from(HashId::Sha256), 247);
    assert_eq!(HashId::try_from(89), Ok(HashId::Sha1));
}

#[test]
fn unset_and_unknown_hash_ids_are_refused_with_their_value() {
    // `0` is reftable's unset id, which its routines read as SHA-1: no variant names it.
    for raw in [0, 1, 20, 32, 88, 90, 246, 248, u32::MAX] {
        assert_eq!(HashId::from_raw(raw), None);
        let err = HashId::try_from(raw).unwrap_err();
        assert_eq!(err.raw(), raw);
        assert_eq!(
            err.to_string(),
            format!("reftable hash id {raw} is not named by reftable-basics.h")
        );
        assert!(err.source().is_none());
    }
}

#[test]
fn hash_id_and_its_error_compare_and_hash_by_value() {
    let ids: HashSet<HashId> = [HashId::Sha1, HashId::Sha256, HashId::Sha1]
        .into_iter()
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(HashId::Sha1, HashId::Sha256);
    assert_eq!(format!("{:?}", HashId::Sha256), "Sha256");
    let errs: HashSet<UnnamedHashId> = [0, 0, 1]
        .into_iter()
        .map(|raw| HashId::try_from(raw).unwrap_err())
        .collect();
    assert_eq!(errs.len(), 2);
}

#[test]
fn empty_buf_reports_no_bytes_through_every_handle() {
    let mut value = Buf::zeroed();
    let r = value.as_ref();
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
    assert_eq!(r.capacity(), 0);
    assert_eq!(r.as_bytes(), b"");
    assert_eq!(r.as_c_str(), Some(c""));
    let mut m = value.as_mut();
    assert_eq!(m.as_mut().as_ref().len(), 0);
    assert_eq!(m.as_ref().as_bytes(), b"");
}

#[test]
fn buf_debug_shows_length_capacity_and_escaped_bytes() {
    let mut value = Buf::zeroed();
    let expected = r#"Buf { len: 0, capacity: 0, bytes: "" }"#;
    assert_eq!(format!("{value:?}"), expected);
    assert_eq!(format!("{:?}", value.as_ref()), expected);
    assert_eq!(format!("{:?}", value.as_mut()), expected);
}
