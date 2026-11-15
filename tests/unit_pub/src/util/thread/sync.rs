//! `src/util/thread.c`'s locks, condition variables and threads (`unix/pthread.h`'s
//! `git_mutex`, `git_cond`, `git_rwlock`, `git_thread_create` and `git_thread_join`), which
//! are native: what each lock excludes and shares, what a guard releases, what a condition
//! variable wakes, what a join returns, and what a panic leaves. The tests that carry over a C
//! case name it. Only those that call libgit2 stop under Miri.

use std::sync::{Arc, Barrier, mpsc};
use std::{panic, thread};

use libgit2_crustify::libgit2::src::libgit2::libgit2;
use libgit2_crustify::libgit2::src::util::errors::{self, ErrorClass};
use libgit2_crustify::libgit2::src::util::thread::{
    Condvar, JoinHandle, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, spawn,
};

/// Threads per test, and steps per thread: enough interleavings to lose an update the lock
/// failed to exclude, few enough for Miri to finish.
const THREADS: usize = 4;
const STEPS: usize = if cfg!(miri) { 20 } else { 500 };

/// Adds one to `*count` in two steps, a read and a write with a yield between, so that an
/// update a lock fails to exclude is lost.
fn slow_increment(count: &mut usize) {
    let seen = *count;
    thread::yield_now();
    *count = seen + 1;
}

/// A lock excludes every other thread while one holds it: increments made in two steps
/// under it lose none.
#[test]
fn a_mutex_excludes_other_threads() {
    let count = Arc::new(Mutex::new(0));
    let threads: Vec<JoinHandle<()>> = (0..THREADS)
        .map(|_| {
            let count = Arc::clone(&count);
            spawn(move || {
                for _ in 0..STEPS {
                    slow_increment(&mut count.lock().unwrap());
                }
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(*count.lock().unwrap(), THREADS * STEPS);
}

/// `MutexGuard::unlock` releases the lock, as dropping the guard does: another thread then
/// takes it, and so does this one again.
#[test]
fn unlock_releases_a_mutex() {
    let lock = Arc::new(Mutex::new(Vec::new()));
    let mut guard = lock.lock().unwrap();
    guard.push(1);
    MutexGuard::unlock(guard);

    let theirs = Arc::clone(&lock);
    spawn(move || theirs.lock().unwrap().push(2))
        .unwrap()
        .join()
        .unwrap();

    let guard = lock.lock().unwrap();
    assert_eq!(*guard, [1, 2]);
    drop(guard);
    assert_eq!(*lock.lock().unwrap(), [1, 2]);
}

/// A lock is a `static` built at compile time, as `GIT_RWLOCK_STATIC_INIT` builds one, and
/// a lock of C's kind, over no data, serialises the threads that take it.
#[test]
fn a_static_mutex_serialises_threads() {
    static LOCK: Mutex<()> = Mutex::new(());
    static LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let threads: Vec<JoinHandle<()>> = (0..THREADS)
        .map(|id| {
            spawn(move || {
                let _held = LOCK.lock().unwrap();
                // Two pushes under the one lock are never interleaved with another thread's.
                LOG.lock().unwrap().push(id);
                thread::yield_now();
                LOG.lock().unwrap().push(id);
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let log = LOG.lock().unwrap();
    assert_eq!(log.len(), 2 * THREADS);
    assert!(log.chunks(2).all(|pair| pair[0] == pair[1]), "{log:?}");
}

/// A thread that panics while holding the lock poisons it: locking it still takes it, and
/// the error hands back the guard over the data the thread left; `get_mut` reports the same.
#[test]
fn a_panic_under_a_mutex_poisons_it() {
    let lock = Arc::new(Mutex::new(1));
    assert!(!lock.is_poisoned());
    let theirs = Arc::clone(&lock);
    let payload = spawn(move || {
        let mut guard = theirs.lock().unwrap();
        *guard = 2;
        panic::panic_any("left half done");
    })
    .unwrap()
    .join()
    .unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"left half done"));

    assert!(lock.is_poisoned());
    let guard = lock.lock().unwrap_err().into_inner();
    assert_eq!(*guard, 2);
    drop(guard);

    let mut lock = Arc::into_inner(lock).unwrap();
    assert_eq!(*lock.get_mut().unwrap_err().into_inner(), 2);
}

/// An unpoisoned lock lends its data through an exclusive borrow without locking, and builds
/// from a value, its default and its `Debug` form as the standard lock's.
#[test]
fn a_mutex_builds_and_lends_its_data() {
    let mut lock = Mutex::from(3);
    *lock.get_mut().unwrap() += 1;
    assert_eq!(*lock.lock().unwrap(), 4);
    assert_eq!(*Mutex::<u8>::default().lock().unwrap(), 0);
    assert_eq!(
        format!("{lock:?}"),
        format!("{:?}", std::sync::Mutex::new(4))
    );
    assert_eq!(format!("{:?}", lock.lock().unwrap()), "4");
}

/// A lock hands back its data when taken apart, as the standard locks do, and reports a
/// poisoned lock's data as poisoned.
#[test]
fn locks_hand_back_their_data() {
    let lock = Mutex::new(vec![1]);
    lock.lock().unwrap().push(2);
    assert_eq!(lock.into_inner().unwrap(), [1, 2]);
    let lock = RwLock::new(String::from("a"));
    lock.write().unwrap().push('b');
    assert_eq!(lock.into_inner().unwrap(), "ab");

    let lock = Arc::new(Mutex::new(5));
    let theirs = Arc::clone(&lock);
    thread::spawn(move || {
        let _guard = theirs.lock().unwrap();
        panic::resume_unwind(Box::new("poisons the lock"));
    })
    .join()
    .unwrap_err();
    let lock = Arc::into_inner(lock).unwrap();
    assert_eq!(lock.into_inner().unwrap_err().into_inner(), 5);
}

/// A lock over a borrow may be dropped after what it borrows, in the same scope, as the
/// standard locks may: dropping one runs no code that could use the borrow.
#[test]
#[expect(
    clippy::needless_late_init,
    reason = "the locks are declared before what they borrow, so that they drop after it"
)]
fn a_lock_over_a_borrow_drops_after_what_it_borrows() {
    let mutex;
    let rwlock;
    let value = 7;
    mutex = Mutex::new(&value);
    rwlock = RwLock::new(&value);
    assert_eq!(**mutex.lock().unwrap(), 7);
    assert_eq!(**rwlock.read().unwrap(), 7);
}

/// A notification wakes the thread that waits for it, which finds the condition its
/// notifier set under the lock, and the waiter's reply wakes the notifier in turn: the pack
/// builder's hand-off of work between its threads (`threaded_find_deltas`), each side waiting
/// in a loop.
#[test]
fn notify_one_wakes_a_waiter() {
    // (work handed over, work done)
    let shared = Arc::new((Mutex::new((0, 0)), Condvar::new()));
    let theirs = Arc::clone(&shared);
    let worker = spawn(move || {
        let (lock, cond) = &*theirs;
        for round in 1..=STEPS {
            let mut state = lock.lock().unwrap();
            while state.0 < round {
                state = cond.wait(state).unwrap();
            }
            state.1 = round;
            cond.notify_one();
        }
    })
    .unwrap();

    let (lock, cond) = &*shared;
    for round in 1..=STEPS {
        let mut state = lock.lock().unwrap();
        state.0 = round;
        cond.notify_one();
        while state.1 < round {
            state = cond.wait(state).unwrap();
        }
    }
    worker.join().unwrap();
    assert_eq!(*lock.lock().unwrap(), (STEPS, STEPS));
}

/// A notification no thread waits for wakes nothing and is not kept, and a condition
/// variable builds as its default and formats as the standard one.
#[test]
fn notify_one_without_a_waiter_does_nothing() {
    let cond = Condvar::default();
    cond.notify_one();
    cond.notify_one();
    assert_eq!(
        format!("{cond:?}"),
        format!("{:?}", std::sync::Condvar::new())
    );
}

/// A wait that takes back a lock a thread poisoned while it waited fails with the guard,
/// over what that thread left.
#[test]
fn a_wait_reports_a_lock_poisoned_meanwhile() {
    let shared = Arc::new((Mutex::new(false), Condvar::new()));
    let (waiting, waits) = mpsc::channel();
    let theirs = Arc::clone(&shared);
    let waiter = spawn(move || {
        let (lock, cond) = &*theirs;
        let mut ready = lock.lock().unwrap();
        // Sent under the lock, which the poisoner takes only once this thread waits.
        waiting.send(()).unwrap();
        loop {
            match cond.wait(ready) {
                Ok(guard) if !*guard => ready = guard,
                Ok(_) => return Ok(()),
                Err(poisoned) => return Err(*poisoned.into_inner()),
            }
        }
    })
    .unwrap();

    waits.recv().unwrap();
    let poisoner = spawn(move || {
        let (lock, cond) = &*shared;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cond.notify_one();
        panic::panic_any("poisoned while notifying");
    })
    .unwrap();
    assert!(poisoner.join().is_err());
    assert_eq!(waiter.join().unwrap(), Err(true));
}

/// Readers hold the lock at once: each waits, under its read lock, for all the others to
/// hold theirs, which would never happen were the lock exclusive.
#[test]
fn readers_share_an_rwlock() {
    let lock = Arc::new(RwLock::new(7));
    let barrier = Arc::new(Barrier::new(THREADS));
    let threads: Vec<JoinHandle<i32>> = (0..THREADS)
        .map(|_| {
            let (lock, barrier) = (Arc::clone(&lock), Arc::clone(&barrier));
            spawn(move || {
                let guard = lock.read().unwrap();
                barrier.wait();
                let seen = *guard;
                RwLockReadGuard::unlock(guard);
                seen
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 7);
    }
    assert_eq!(*lock.write().unwrap(), 7);
}

/// A writer excludes every reader and writer: increments made in two steps under the write
/// lock lose none, and readers between them see a count no writer is changing.
#[test]
fn a_writer_excludes_an_rwlock() {
    let count = Arc::new(RwLock::new(0));
    let threads: Vec<JoinHandle<()>> = (0..THREADS)
        .map(|_| {
            let count = Arc::clone(&count);
            spawn(move || {
                for _ in 0..STEPS {
                    let mut guard = count.write().unwrap();
                    slow_increment(&mut guard);
                    RwLockWriteGuard::unlock(guard);
                    let seen = *count.read().unwrap();
                    assert!(seen <= THREADS * STEPS);
                }
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(*count.read().unwrap(), THREADS * STEPS);
}

/// Each unlock releases what its guard holds: a read lock, then a write lock, then a read
/// lock again, on one thread, none of which would be taken were the one before still held.
#[test]
fn rwlock_unlocks_release_their_lock() {
    let lock = RwLock::new(String::from("a"));
    RwLockReadGuard::unlock(lock.read().unwrap());
    let mut writer = lock.write().unwrap();
    writer.push('b');
    RwLockWriteGuard::unlock(writer);
    let reader = lock.read().unwrap();
    assert_eq!(*reader, "ab");
    assert_eq!(format!("{reader:?}"), "\"ab\"");
    RwLockReadGuard::unlock(reader);
    let writer = lock.write().unwrap();
    assert_eq!(format!("{writer:?}"), "\"ab\"");
}

/// A writer that panics poisons the lock, for readers and writers alike, each handed the
/// guard; a reader that panics does not.
#[test]
fn a_panicking_writer_poisons_an_rwlock() {
    let lock = Arc::new(RwLock::new(1));
    let theirs = Arc::clone(&lock);
    assert!(
        spawn(move || {
            let _reading = theirs.read().unwrap();
            panic::panic_any("reader");
        })
        .unwrap()
        .join()
        .is_err()
    );
    assert!(!lock.is_poisoned());

    let theirs = Arc::clone(&lock);
    assert!(
        spawn(move || {
            let mut writing = theirs.write().unwrap();
            *writing = 2;
            panic::panic_any("writer");
        })
        .unwrap()
        .join()
        .is_err()
    );
    assert!(lock.is_poisoned());
    assert_eq!(*lock.read().unwrap_err().into_inner(), 2);
    assert_eq!(*lock.write().unwrap_err().into_inner(), 2);

    let mut lock = Arc::into_inner(lock).unwrap();
    assert_eq!(*lock.get_mut().unwrap_err().into_inner(), 2);
    assert_eq!(*RwLock::<u8>::default().read().unwrap(), 0);
    assert_eq!(*RwLock::from(5).read().unwrap(), 5);
    assert_eq!(
        format!("{:?}", RwLock::from(5)),
        format!("{:?}", std::sync::RwLock::new(5))
    );
}

/// A join returns what the thread's closure returned, as `git_thread_join` reads the start
/// routine's result through `status`.
///
/// Carries over `test_threads_basic__exit` (`tests/libgit2/threads/basic.c`), its first half:
/// the start routine's return value, 424242, is what the join returns. Its second half ends
/// the thread through `git_thread_exit`, which has no Rust form: a closure ends by returning.
#[test]
fn join_returns_the_closures_result() {
    let thread = spawn(|| 424_242_usize).unwrap();
    assert_eq!(thread.join().unwrap(), 424_242);

    let input = vec![1, 2, 3];
    let thread = spawn(move || input.into_iter().sum::<i32>()).unwrap();
    assert_eq!(thread.join().unwrap(), 6);
}

/// A join returns a panicking closure's payload, which C's thread has no form of.
#[test]
fn join_returns_a_panics_payload() {
    let thread = spawn(|| -> u8 { panic::panic_any(232_323_usize) }).unwrap();
    let payload = thread.join().unwrap_err();
    assert_eq!(payload.downcast_ref::<usize>(), Some(&232_323));
}

/// A handle names its thread, which reports itself finished once its closure returns, and a
/// handle moves to another thread, which joins it there.
#[test]
fn a_handle_names_its_thread_and_moves() {
    let (sender, receiver) = mpsc::channel();
    let thread = spawn(move || {
        sender.send(thread::current().id()).unwrap();
        "done"
    })
    .unwrap();
    assert_eq!(thread.thread().id(), receiver.recv().unwrap());
    assert!(format!("{thread:?}").starts_with("JoinHandle"));

    let joiner = spawn(move || {
        while !thread.is_finished() {
            thread::yield_now();
        }
        thread.join().unwrap()
    })
    .unwrap();
    assert_eq!(joiner.join().unwrap(), "done");
}

/// Threads started at once each run their closure and are each joined, as the C tests'
/// `run_in_parallel` starts and joins them, repeated.
#[test]
fn threads_run_in_parallel_and_join() {
    for repeat in 0..3 {
        let threads: Vec<JoinHandle<usize>> = (0..THREADS)
            .map(|id| spawn(move || id * repeat).unwrap())
            .collect();
        let results: Vec<usize> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(
            results,
            (0..THREADS).map(|id| id * repeat).collect::<Vec<_>>()
        );
    }
}

/// Each thread's last error is its own: four threads set one at once, each reads it back.
///
/// Carries over `test_threads_basic__set_error` (`tests/libgit2/threads/basic.c`), whose
/// four threads each call `git_error_set` with the same message through `run_in_parallel`;
/// each also reads its error back here, which the C case leaves to its sanitizers.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn each_thread_sets_its_own_error() {
    let threads: Vec<JoinHandle<()>> = (0..THREADS)
        .map(|_| {
            spawn(|| {
                errors::set(ErrorClass::Invalid, c"oh no, something happened!\n");
                let last = errors::last();
                assert_eq!(last.class(), Some(ErrorClass::Invalid));
                assert_eq!(last.message(), c"oh no, something happened!\n");
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

/// Threads take and release libgit2's runtime at once while another reference keeps it
/// up, and every take and release succeeds.
///
/// Carries over `test_init__concurrent_init_succeeds` (`tests/util/init.c`): ten threads,
/// each initialising and shutting libgit2 down twenty times, while the test holds a reference
/// of its own. The C case asserts the count its own initialisation returns, 2 under the clar
/// runner's; here other tests hold references too, so the count is only at least 1.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn concurrent_init_succeeds() {
    let held = libgit2::init().unwrap();
    let threads: Vec<JoinHandle<()>> = (0..10)
        .map(|_| {
            spawn(|| {
                for _ in 0..20 {
                    let runtime = libgit2::init().unwrap();
                    assert!(runtime.count().get() > 1);
                    runtime.shutdown().unwrap();
                }
            })
            .unwrap()
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(held.shutdown().is_ok());
}
