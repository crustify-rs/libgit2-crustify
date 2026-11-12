//! `src/util/alloc.c`: the contract of the resizes, `ArrayFree::resize` and
//! `ArrayFree::resize_bytes`, where it is Rust's own rather than C's: a resize to zero
//! releases the run, a size no Rust slice may span is refused before C is asked, and a
//! failure leaves the run as it was and reports the thread's last error.

use std::mem::MaybeUninit;

use libgit2::libgit2::src::util::alloc::{AllocVec, ArrayFree};
use libgit2::libgit2::src::util::errors::{self, ErrorClass};

/// Where C would ask its allocator for a zero-byte resize, which C leaves
/// implementation-defined, the run is released and held without storage.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_resize_to_zero_releases_the_run() {
    let mut run = ArrayFree::new_uninit::<u64>(8).unwrap();
    ArrayFree::resize(&mut run, 0).unwrap();
    assert_eq!(run.len(), 0);
    assert!(run.as_ptr().is_null());
    let mut bytes = ArrayFree::new_uninit::<u8>(3).unwrap();
    ArrayFree::resize_bytes(&mut bytes, 0).unwrap();
    assert!(bytes.is_empty() && bytes.as_ptr().is_null());
}

/// A run without storage resized to zero asks nothing and stays so; Miri runs it, since
/// libgit2 is never reached.
#[test]
fn an_empty_run_resized_to_zero_stays_empty() {
    let mut run = AllocVec::<MaybeUninit<u16>>::empty();
    ArrayFree::resize(&mut run, 0).unwrap();
    ArrayFree::resize_bytes(&mut AllocVec::empty(), 0).unwrap();
    assert!(run.is_empty() && run.as_ptr().is_null());
}

/// A size past `isize::MAX` bytes that does not overflow is refused before C is asked,
/// with libgit2's out-of-memory error, which is also the thread's last error; the run is
/// left as it was, and nothing panics.
#[test]
#[cfg_attr(miri, ignore = "reports through libgit2's error state")]
fn sizes_past_isize_max_are_refused_with_out_of_memory() {
    let mut run = ArrayFree::new_uninit::<u32>(2).unwrap();
    let before = run.as_ptr();
    errors::clear();
    let e = ArrayFree::resize(&mut run, isize::MAX as usize / 4 + 1).unwrap_err();
    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
    assert_eq!(errors::last(), e);
    assert_eq!((run.as_ptr(), run.len()), (before, 2));

    let mut bytes = AllocVec::<MaybeUninit<u8>>::empty();
    errors::clear();
    let e = ArrayFree::resize_bytes(&mut bytes, isize::MAX as usize + 1).unwrap_err();
    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
    assert_eq!(errors::last(), e);
    assert!(bytes.as_ptr().is_null());
}

/// A resize keeps the run's length in step with the request, growing and shrinking, and a
/// run without storage grows like any other.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_resize_sets_the_length_it_was_asked_for() {
    let mut run = AllocVec::<MaybeUninit<u32>>::empty();
    for len in [1, 7, 7, 4096, 2, 9] {
        ArrayFree::resize(&mut run, len).unwrap();
        assert_eq!(run.len(), len);
        assert!(!run.as_ptr().is_null());
        run.as_mut_slice()[len - 1].write(1);
    }
}
