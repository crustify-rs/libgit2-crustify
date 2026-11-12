//! UB tests for `deps/reftable/basics.c`'s types: the enum `HashId` and the buffer `Buf`.
//!
//! `HashId` is a fieldless enum over C's `enum reftable_hash`. The only safe way in from a
//! raw value is `from_raw` / `try_from`, which must reject every value without a variant
//! rather than produce an invalid discriminant, which Miri and `UBSan` would flag on the
//! next read; an `as` cast to it does not compile (a `compile_fail` doctest on `HashId`).
//!
//! A `Buf` value comes from `Buf::zeroed()`, the unallocated state, which its getters read
//! without following `buf`. Its handles reborrow without aliasing, and what they read is
//! tied to the value's borrow (`compile_fail` doctests on `Buf` and its getters). None of these
//! calls C, so they also run under Miri.

use std::collections::HashSet;
use std::ffi::CStr;
use std::hint::black_box;

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{Buf, BufMut, BufRef, HashId};

/// Every value around the declared ones, then a coarse stride across the whole type and
/// its top; `BorrowSanitizer` instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::reftable_hash> {
    (0..=300)
        .chain((0..=ffi::reftable_hash::MAX).step_by(1 << 26))
        .chain([ffi::reftable_hash::MAX - 1, ffi::reftable_hash::MAX])
}

#[test]
fn hash_id_from_raw_yields_only_valid_ids() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        let Some(id) = HashId::from_raw(black_box(raw)) else {
            assert!(HashId::try_from(raw).is_err());
            continue;
        };
        // Reading the discriminant back is where an invalid value would be caught.
        let back = black_box(id).raw();
        assert_eq!(back, raw);
        assert!(matches!(id, HashId::Sha1 | HashId::Sha256));
        assert_eq!(HashId::try_from(raw), Ok(id));
        seen.insert(id);
    }
    assert_eq!(seen.len(), 2, "both ids are reached");
}

#[test]
fn hash_id_round_trips_through_its_c_value() {
    for id in [HashId::Sha1, HashId::Sha256] {
        let raw = ffi::reftable_hash::from(black_box(id));
        assert_eq!(HashId::from_raw(raw), Some(id));
    }
}

/// Reads every getter, so that each one runs over the handle it is given.
fn read_all(b: BufRef<'_>) -> (usize, bool, usize, usize, bool) {
    (
        b.len(),
        b.is_empty(),
        b.capacity(),
        b.as_bytes().len(),
        b.as_c_str().is_some_and(CStr::is_empty),
    )
}

const EMPTY: (usize, bool, usize, usize, bool) = (0, true, 0, 0, true);

#[test]
fn zeroed_buf_reads_as_empty_through_every_handle() {
    let mut value = Buf::zeroed();
    assert_eq!(read_all(value.as_ref()), EMPTY);
    let mut m = value.as_mut();
    assert_eq!(read_all(m.as_ref()), EMPTY);
    assert_eq!(read_all(m.as_mut().as_ref()), EMPTY);
    assert_eq!(read_all(value.as_ref()), EMPTY);
    assert_eq!(format!("{value:?}"), format!("{:?}", value.as_ref()));
}

/// Takes an exclusive handle by value and hands a reborrow on, as a routine that fills a
/// buffer would.
fn pass_on(mut m: BufMut<'_>) -> usize {
    let inner = read_all(m.as_mut().as_ref()).0;
    inner + m.as_ref().len()
}

#[test]
fn buf_handles_reborrow_without_aliasing() {
    let mut value = Buf::zeroed();
    let mut m = value.as_mut();
    assert_eq!(pass_on(m.as_mut()), 0);
    assert_eq!(pass_on(m), 0);
    let shared = value.as_ref();
    let copy = shared;
    assert_eq!(read_all(shared), read_all(copy));
}

#[test]
fn buf_values_move_and_stay_empty() {
    let boxed = Box::new(Buf::zeroed());
    let mut values: Vec<Buf> = (0..64).map(|_| Buf::zeroed()).collect();
    values.push(*boxed);
    for value in &mut values {
        assert_eq!(read_all(value.as_mut().as_ref()), EMPTY);
    }
    let moved: Vec<Buf> = values.into_iter().rev().collect();
    assert!(moved.iter().all(|v| read_all(v.as_ref()) == EMPTY));
}

#[test]
fn bytes_outlive_the_handle_they_were_read_through() {
    let value = Buf::zeroed();
    // Tied to the value's borrow, not to the handle's own scope.
    let (bytes, c_str) = {
        let shared = value.as_ref();
        (shared.as_bytes(), shared.as_c_str())
    };
    assert_eq!(bytes, b"");
    assert_eq!(c_str, Some(c""));
    assert_eq!(read_all(value.as_ref()), EMPTY);
}
