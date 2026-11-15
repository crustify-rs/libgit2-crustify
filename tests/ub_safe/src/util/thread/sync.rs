//! `src/util/thread.c`'s locks, condition variables and threads on the safe API: data that
//! is not atomic, shared between threads that change it at once under a [`Mutex`] or an
//! [`RwLock`], handed between them through a [`Condvar`], and moved into and out of threads
//! that [`spawn`] starts and their handles join, from other threads too. Under `TSan` they
//! race nothing, and under Miri they reach no undefined behaviour. The compile-fail cases
//! under `compile_fail/` hold what the types refuse: a guard outliving its lock or sent to
//! another thread, a lock dropped while a guard holds it, and a thread borrowing its caller's
//! data.

use std::sync::Arc;

use libgit2_crustify::libgit2::src::util::thread::{
    Condvar, JoinHandle, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, spawn,
};

/// Threads per test, and steps per thread: enough interleavings for `TSan` and Miri to see,
/// few enough for Miri to finish.
const THREADS: usize = 4;
const STEPS: usize = if cfg!(miri) { 20 } else { 1000 };

/// Joins every handle, propagating a thread's panic.
fn join_all<T>(threads: Vec<JoinHandle<T>>) -> Vec<T> {
    threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect()
}

/// Threads that share a vector under one lock push to it and read it at once, through the
/// guard's mutable and shared dereferences, and lose no push.
#[test]
fn a_mutex_guards_plain_data_across_threads() {
    let data = Arc::new(Mutex::new(Vec::new()));
    let threads = (0..THREADS)
        .map(|id| {
            let data = Arc::clone(&data);
            spawn(move || {
                for step in 0..STEPS {
                    let mut guard = data.lock().unwrap();
                    guard.push((id, step));
                    let len = guard.len();
                    MutexGuard::unlock(guard);
                    assert!(len <= THREADS * STEPS);
                }
            })
            .unwrap()
        })
        .collect();
    join_all(threads);
    let data = data.lock().unwrap();
    assert_eq!(data.len(), THREADS * STEPS);
}

/// A `static` lock guards a `String` that threads append to at once.
#[test]
fn a_static_mutex_guards_a_string() {
    static TEXT: Mutex<String> = Mutex::new(String::new());
    let threads = (0..THREADS)
        .map(|_| {
            spawn(|| {
                for _ in 0..STEPS {
                    TEXT.lock().unwrap().push('x');
                }
            })
            .unwrap()
        })
        .collect();
    join_all(threads);
    assert_eq!(TEXT.lock().unwrap().len(), THREADS * STEPS);
}

/// Readers read a vector while writers replace it, each through its own guard, and every
/// read sees a whole vector.
#[test]
fn an_rwlock_guards_plain_data_across_threads() {
    let data = Arc::new(RwLock::new(vec![0_usize; 8]));
    let mut threads = Vec::new();
    for id in 0..THREADS {
        let data = Arc::clone(&data);
        threads.push(
            spawn(move || {
                for step in 0..STEPS {
                    if id % 2 == 0 {
                        let mut guard = data.write().unwrap();
                        *guard = vec![step; 8];
                        RwLockWriteGuard::unlock(guard);
                    } else {
                        let guard = data.read().unwrap();
                        assert!(guard.iter().all(|&v| v == guard[0]));
                        RwLockReadGuard::unlock(guard);
                    }
                }
            })
            .unwrap(),
        );
    }
    join_all(threads);
    assert_eq!(data.read().unwrap().len(), 8);
}

/// A producer hands boxed values to a consumer through a slot a lock guards and a
/// condition variable signals, each waiting in a loop, and the consumer takes each value
/// once.
#[test]
fn a_condvar_hands_values_between_threads() {
    let shared = Arc::new((Mutex::new(None::<Box<usize>>), Condvar::new()));
    let theirs = Arc::clone(&shared);
    let consumer = spawn(move || {
        let (slot, cond) = &*theirs;
        let mut sum = 0;
        for _ in 0..STEPS {
            let mut guard = slot.lock().unwrap();
            let value = loop {
                if let Some(value) = guard.take() {
                    break value;
                }
                guard = cond.wait(guard).unwrap();
            };
            cond.notify_one();
            sum += *value;
        }
        sum
    })
    .unwrap();

    let (slot, cond) = &*shared;
    for value in 0..STEPS {
        let mut guard = slot.lock().unwrap();
        while guard.is_some() {
            guard = cond.wait(guard).unwrap();
        }
        *guard = Some(Box::new(value));
        cond.notify_one();
    }
    assert_eq!(consumer.join().unwrap(), (0..STEPS).sum::<usize>());
}

/// A thread's result, an owned allocation, moves out through its join, and handles move to
/// other threads that join them there.
#[test]
fn handles_move_results_and_join_elsewhere() {
    let threads: Vec<JoinHandle<Vec<usize>>> = (0..THREADS)
        .map(|id| spawn(move || vec![id; STEPS]).unwrap())
        .collect();
    let joiners = threads
        .into_iter()
        .map(|thread| spawn(move || thread.join().unwrap().into_iter().sum::<usize>()).unwrap())
        .collect();
    let sums = join_all(joiners);
    assert_eq!(sums, (0..THREADS).map(|id| id * STEPS).collect::<Vec<_>>());
}

/// A handle dropped unjoined leaves its thread running detached, which still reaches the
/// data it owns.
#[test]
fn an_unjoined_thread_runs_on() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let data = Arc::new(Mutex::new(0));
    let theirs = Arc::clone(&data);
    drop(
        spawn(move || {
            *theirs.lock().unwrap() += 1;
            sender.send(()).unwrap();
        })
        .unwrap(),
    );
    receiver.recv().unwrap();
    assert_eq!(*data.lock().unwrap(), 1);
}
