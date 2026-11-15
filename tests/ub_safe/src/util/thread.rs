//! `src/util/thread.c`'s types on the safe API: the atomics' handles and values, which are
//! `Send` and `Sync`, shared and sent between threads that change the value at once, as
//! libgit2's threads change a reference count. Under `TSan` they race nothing, and under
//! Miri they reach no undefined behaviour. The compile-fail cases under `compile_fail/` hold what
//! the types refuse: a lent atomic outliving its value, a shared handle beside an exclusive
//! one, a `Thread` copied, sent to a thread or written without `unsafe`.

use core::sync::atomic::Ordering::SeqCst;
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::{Atomic32, Atomic64, AtomicSsize};

/// Threads per test, and increments per thread: enough interleavings for `TSan` and Miri
/// to see, few enough for Miri to finish.
const THREADS: i32 = 4;
const STEPS: i32 = if cfg!(miri) { 20 } else { 1000 };

/// Threads that share a value Rust holds, each through a shared handle of its own, add to it
/// at once and lose no increment.
#[test]
fn shared_handles_add_from_many_threads() {
    let count = Atomic32::new(0);
    thread::scope(|scope| {
        for _ in 0..THREADS {
            let handle = count.as_ref();
            scope.spawn(move || {
                for _ in 0..STEPS {
                    handle.val().fetch_add(1, SeqCst);
                }
            });
        }
    });
    assert_eq!(count.as_ref().val().load(SeqCst), THREADS * STEPS);
}

/// Threads that share the value itself (`&Atomic64`) and the atomic lent from it add and
/// subtract at once, and the value ends where it began.
#[test]
fn a_shared_value_and_its_lent_atomic_add_and_subtract_from_many_threads() {
    let size = Atomic64::new(7);
    let lent = size.as_ref().val();
    thread::scope(|scope| {
        for i in 0..THREADS {
            let size = &size;
            scope.spawn(move || {
                for _ in 0..STEPS {
                    if i % 2 == 0 {
                        size.as_ref().val().fetch_add(3, SeqCst);
                    } else {
                        lent.fetch_sub(3, SeqCst);
                    }
                }
            });
        }
    });
    assert_eq!(size.as_ref().val().load(SeqCst), 7);
}

/// An exclusive handle sent to another thread writes the value there, and the write is seen
/// once that thread has been joined.
#[test]
fn an_exclusive_handle_moves_to_another_thread() {
    let mut count = Atomic32::new(1);
    thread::scope(|scope| {
        let mut exclusive = count.as_mut();
        scope.spawn(move || {
            exclusive.as_mut().as_ref().val().store(2, SeqCst);
        });
    });
    assert_eq!(count.as_ref().val().load(SeqCst), 2);
}

/// A value moved to another thread keeps its integer there, and a `static` is shared by every
/// thread, as C's globals are.
#[test]
fn values_move_between_threads_and_statics_are_shared() {
    static TOTAL: AtomicSsize = AtomicSsize::new(0);
    let count = Atomic32::new(5);
    let moved = thread::spawn(move || count.as_ref().val().load(SeqCst))
        .join()
        .unwrap();
    assert_eq!(moved, 5);
    thread::scope(|scope| {
        for _ in 0..THREADS {
            scope.spawn(|| {
                for _ in 0..STEPS {
                    TOTAL.as_ref().val().fetch_add(1, SeqCst);
                }
            });
        }
    });
    assert_eq!(
        TOTAL.as_ref().val().load(SeqCst),
        i64::from(THREADS * STEPS)
    );
}

/// A compare-and-swap loop, as a reference count's increment guarded against zero would be,
/// from many threads at once, through shared handles: every success is counted once.
#[test]
fn compare_and_swap_from_many_threads() {
    let count = Atomic32::new(1);
    thread::scope(|scope| {
        for _ in 0..THREADS {
            let handle = count.as_ref();
            scope.spawn(move || {
                for _ in 0..STEPS {
                    let val = handle.val();
                    let mut seen = val.load(SeqCst);
                    while let Err(now) = val.compare_exchange(seen, seen + 1, SeqCst, SeqCst) {
                        seen = now;
                    }
                }
            });
        }
    });
    assert_eq!(count.as_ref().val().load(SeqCst), 1 + THREADS * STEPS);
}
