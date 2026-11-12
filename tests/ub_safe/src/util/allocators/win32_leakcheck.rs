//! `src/util/allocators/win32_leakcheck.c`: [`allocator`], through the safe API alone.
//!
//! Every variant builds the C stub, which fails and sets the thread's error. The tests
//! race first calls to initialise libgit2 from a fresh process, call it from many threads
//! at once, hold its errors across later libgit2 calls that overwrite the thread's state,
//! and keep allocating through libgit2 after it, under the sanitizers: the errors are owned
//! copies that outlive the thread state they were read from, the first calls race to pin
//! the runtime, and the allocations show that libgit2's installed allocator is left as it
//! was.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::util::alloc::{AllocVec, StrFree};
use libgit2::libgit2::src::util::allocators::win32_leakcheck::allocator;
use libgit2::libgit2::src::util::errors::{ErrorClass, last};

use crate::fixture::isolated;

const MESSAGE: &[u8] = b"leakcheck memory allocator not available";

/// Calls it, checks the error it reports, and allocates through libgit2 after it.
fn fails_and_allocates(seed: u8) {
    let error = allocator().unwrap_err();
    assert_eq!(error.class(), Some(ErrorClass::Invalid));
    assert_eq!(error.message().to_bytes(), MESSAGE);
    let v = AllocVec::<u8>::from_slice(&[seed; 48]).unwrap();
    let s = StrFree::dup_bytes(&[b'a' + seed % 26; 5]).unwrap();
    assert_eq!(v.as_slice(), &[seed; 48]);
    assert_eq!(s.as_bytes(), &[b'a' + seed % 26; 5]);
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process")]
fn first_calls_race_to_pin_the_runtime() {
    isolated(
        concat!(module_path!(), "::first_calls_race_to_pin_the_runtime"),
        || {
            let barrier = Barrier::new(8);
            thread::scope(|s| {
                for i in 0..8u8 {
                    let barrier = &barrier;
                    s.spawn(move || {
                        barrier.wait();
                        fails_and_allocates(i);
                    });
                }
            });
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn many_threads_fail_alike() {
    thread::scope(|s| {
        for i in 0..16u8 {
            s.spawn(move || {
                for _ in 0..32 {
                    fails_and_allocates(i);
                }
            });
        }
    });
}

/// The error is an owned copy: libgit2 rewrites the thread's error buffer on the next
/// failure, which the copy must survive, here and on another thread.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn errors_outlive_later_failures() {
    let held: Vec<_> = (0..8).map(|_| allocator().unwrap_err()).collect();
    let other = Oid::from_string(c"not hex", OidType::Sha1).unwrap_err();
    assert_ne!(last().message(), held[0].message());
    assert_ne!(other.message(), held[0].message());
    let moved = thread::spawn(move || held).join().unwrap();
    for error in &moved {
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(error.message().to_bytes(), MESSAGE);
    }
}
