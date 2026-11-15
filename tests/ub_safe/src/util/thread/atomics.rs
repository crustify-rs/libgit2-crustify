//! `thread.h`'s atomic operations on the safe API, from many threads at once, as libgit2's
//! threads use them: a reference count raised and dropped, a total added to, a slot installed
//! once by racing initialisers, a slot whose holders swap values, and a fence between a write
//! and a flag. Under `TSan` they race nothing, under Miri they reach no undefined behaviour, and
//! no update is lost.

use core::ptr;
use core::sync::atomic::Ordering::{Relaxed, SeqCst};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicUsize};
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::{
    Atomic32, AtomicSsize, atomic_compare_and_swap, atomic_load, atomic_swap, memory_barrier,
};

/// Threads per test, and steps per thread: enough interleavings for `TSan` and Miri to see,
/// few enough for Miri to finish.
const THREADS: usize = 4;
const STEPS: i32 = if cfg!(miri) { 20 } else { 1000 };

/// Threads that each take and drop references to one count, through shared handles of their
/// own, leave it where it started, and none sees it fall to 0 while it holds a reference.
#[test]
fn a_refcount_rises_and_falls_from_many_threads() {
    let refcount = Atomic32::new(1);
    thread::scope(|scope| {
        for _ in 0..THREADS {
            let handle = refcount.as_ref();
            scope.spawn(move || {
                for _ in 0..STEPS {
                    assert!(handle.inc() >= 2);
                    assert!(handle.dec() >= 1);
                }
            });
        }
    });
    assert_eq!(refcount.as_ref().get(), 1);
}

/// Threads that add to and subtract from one `static` total, as the object cache's storage
/// count is, lose no update.
#[test]
fn a_static_total_adds_from_many_threads() {
    static TOTAL: AtomicSsize = AtomicSsize::new(0);
    thread::scope(|scope| {
        for i in 0..THREADS {
            scope.spawn(move || {
                let sign = if i % 2 == 0 { 1 } else { -1 };
                for _ in 0..STEPS {
                    TOTAL.as_ref().add(sign * 4096);
                    TOTAL.as_ref().add(sign);
                }
            });
        }
    });
    assert_eq!(TOTAL.as_ref().get(), 0);
    TOTAL.as_ref().set(0);
}

/// Racing initialisers each try to install their own value in an empty slot, as libgit2's
/// lazily loaded caches do (`git_atomic_compare_and_swap(&repo->attrcache, NULL, cache)`):
/// exactly one wins, and every thread reads back the winner's.
#[test]
fn racing_initialisers_install_one_value() {
    let candidates: [AtomicI32; THREADS] = Default::default();
    let slot: AtomicPtr<AtomicI32> = AtomicPtr::new(ptr::null_mut());
    let wins = AtomicUsize::new(0);
    thread::scope(|scope| {
        for mine in &candidates {
            let (slot, wins) = (&slot, &wins);
            scope.spawn(move || {
                let mine = ptr::from_ref(mine).cast_mut();
                let found = atomic_compare_and_swap(slot, ptr::null_mut(), mine);
                if found.is_null() {
                    wins.fetch_add(1, SeqCst);
                } else {
                    assert_ne!(found, mine);
                }
                assert!(!atomic_load(slot).is_null());
            });
        }
    });
    assert_eq!(atomic_load(&wins), 1);
    let winner = atomic_load(&slot);
    assert!(
        candidates
            .iter()
            .any(|c| ptr::from_ref(c).cast_mut() == winner)
    );
}

/// Threads that each swap their own values into one slot get back every value once: the
/// values swapped out, with what the slot holds at the end, are the values swapped in and the
/// one it started with, none lost and none repeated.
#[test]
fn swaps_from_many_threads_lose_no_value() {
    let slot = AtomicUsize::new(0);
    let returned: Vec<Vec<usize>> = thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|t| {
                let slot = &slot;
                scope.spawn(move || {
                    (0..STEPS)
                        .map(|step| {
                            let step = usize::try_from(step).unwrap();
                            atomic_swap(slot, 1 + t * 1_000_000 + step)
                        })
                        .collect()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });
    let mut seen: Vec<usize> = returned.into_iter().flatten().collect();
    seen.push(atomic_load(&slot));
    seen.sort_unstable();
    let steps = usize::try_from(STEPS).unwrap();
    let mut expected: Vec<usize> = (0..THREADS)
        .flat_map(|t| (0..steps).map(move |step| 1 + t * 1_000_000 + step))
        .chain([0])
        .collect();
    expected.sort_unstable();
    assert_eq!(seen, expected);
}

/// A writer stores a value, fences and raises a flag; a reader that sees the flag fences and
/// reads the value, as `git_runtime_init_count`'s callers order their reads.
#[test]
fn a_fence_orders_a_value_before_its_flag() {
    let value = Atomic32::new(0);
    let ready = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| {
            value.as_ref().set(42);
            memory_barrier();
            ready.store(true, Relaxed);
        });
        scope.spawn(|| {
            while !ready.load(Relaxed) {
                thread::yield_now();
            }
            memory_barrier();
            assert_eq!(value.as_ref().get(), 42);
        });
    });
}
