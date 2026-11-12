//! `src/util/allocators/failalloc.c`: the owners a failing allocator family can type,
//! built through their safe constructors, which must fail cleanly, never hand out
//! storage, and drop what they do build (the empty array) without touching an
//! allocator.
//!
//! The strategy is Rust alone, so every test runs under Miri too.

use std::cell::Cell;
use std::mem::MaybeUninit;
use std::thread;

use ffibox::{CBox, CStrBox, CVec};
use libgit2::libgit2::src::util::allocators::failalloc::{FailAlloc, FailArrayAlloc};

use crate::fixture::Pair;

/// An element aligned to the 16 bytes the family vouches for, the most any libgit2
/// allocator family promises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(align(16))]
struct Wide([u8; 16]);

#[test]
fn boxes_never_hold_storage() {
    assert!(CBox::<Pair, FailAlloc>::new_uninit().is_none());
    assert!(CBox::<Pair, FailAlloc>::new_uninit_with(FailAlloc).is_none());
    // Repeated requests keep failing: the family keeps no state between them.
    for _ in 0..64 {
        assert!(CBox::<Pair, FailAlloc>::new_uninit().is_none());
    }
}

#[test]
fn arrays_never_hold_storage() {
    for len in [1, 2, 4096, 1 << 30] {
        assert!(CVec::<u8, FailArrayAlloc>::new_uninit(len).is_none());
        assert!(CVec::<u64, FailArrayAlloc>::new_uninit(len).is_none());
        assert!(CVec::<Wide, FailArrayAlloc>::new_uninit_with(len, FailArrayAlloc).is_none());
    }
    // A size that overflows is refused before the allocator is asked.
    assert!(CVec::<u64, FailArrayAlloc>::new_uninit(usize::MAX).is_none());
}

#[test]
fn a_failed_fill_never_runs_its_closure() {
    let calls = Cell::new(0);
    let count = |i: usize| {
        calls.set(calls.get() + 1);
        i
    };
    assert!(CVec::<usize, FailArrayAlloc>::from_fn(3, count).is_none());
    assert!(CVec::<usize, FailArrayAlloc>::from_fn_with(1, FailArrayAlloc, count).is_none());
    assert_eq!(calls.get(), 0, "no storage, so no element is made");

    // Elements that own strings of the family: none is built, so none leaks.
    let made = CVec::<Option<CStrBox<FailAlloc>>, FailArrayAlloc>::from_fn(2, |_| {
        calls.set(calls.get() + 1);
        None
    });
    assert!(made.is_none());
    assert_eq!(calls.get(), 0);
}

#[test]
fn empty_arrays_ask_for_nothing() {
    let calls = Cell::new(0);
    let v = CVec::<u64, FailArrayAlloc>::from_fn(0, |i| {
        calls.set(calls.get() + 1);
        i as u64
    })
    .unwrap();
    assert_eq!(calls.get(), 0);
    assert!(v.is_empty() && v.as_ptr().is_null());
    assert_eq!(v.as_slice().len(), 0);

    let w: CVec<MaybeUninit<Wide>, FailArrayAlloc> = CVec::new_uninit(0).unwrap();
    assert!(w.is_empty() && w.as_ptr().is_null());

    let strings = CVec::<Option<CStrBox<FailAlloc>>, FailArrayAlloc>::from_fn(0, |_| None).unwrap();
    assert!(strings.is_empty());
    let none = CVec::<u8, FailArrayAlloc>::default();
    assert!(none.is_empty() && none.as_ptr().is_null());
    // Dropping them releases nothing, as no allocator was asked for anything.
    drop((v, w, strings, none));
}

#[test]
fn owners_cross_threads() {
    // The policies are stateless, so the owners they type move between threads; each
    // thread's requests fail on their own, and an empty run made on one drops on another.
    let empty = CVec::<u64, FailArrayAlloc>::from_fn(0, |i| i as u64).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            thread::spawn(move || {
                assert!(CBox::<Pair, FailAlloc>::new_uninit().is_none());
                assert!(CVec::<u64, FailArrayAlloc>::new_uninit(i + 1).is_none());
                CVec::<u8, FailArrayAlloc>::from_fn(0, |_| 0).unwrap()
            })
        })
        .collect();
    let made: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(made.iter().all(CVec::is_empty));
    thread::spawn(move || drop(empty)).join().unwrap();
}
