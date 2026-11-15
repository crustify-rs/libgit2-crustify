//! `ThreadMut::set_thread`, `src/util/thread.c`'s one `unsafe` function, called within its
//! contract: the ids of live threads that their `JoinHandle`s still own and join, since
//! nothing joins through the object, and ids that name no thread, 0 among them, which nothing
//! joins either. The id is read back unchanged, a later id replaces an earlier one without
//! joining it, and the objects are dropped holding ids, which releases nothing.

use std::os::unix::thread::{JoinHandleExt, RawPthread};
use std::sync::mpsc;
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::Thread;

/// The id of a live thread, stored while its `JoinHandle` keeps it, reads back unchanged;
/// the handle then joins it, as nothing joined it through the object.
#[test]
fn stores_the_id_of_a_live_thread() {
    let (release, wait) = mpsc::channel::<()>();
    let worker = thread::spawn(move || wait.recv().unwrap());
    let id = worker.as_pthread_t();
    let mut thread = Thread::default();
    // SAFETY: nothing joins or detaches through `thread`; `worker` joins its thread below.
    unsafe { thread.as_mut().set_thread(id) };
    assert_eq!(thread.as_ref().thread(), id);
    release.send(()).unwrap();
    worker.join().unwrap();
    // The object still holds the id of the joined thread, which nothing joins through it.
    assert_eq!(thread.as_ref().thread(), id);
}

/// A later id replaces an earlier one without joining or detaching it: both threads are still
/// joined by their handles.
#[test]
fn a_later_id_replaces_an_earlier_one() {
    let (first_release, first_wait) = mpsc::channel::<()>();
    let (second_release, second_wait) = mpsc::channel::<()>();
    let first = thread::spawn(move || first_wait.recv().unwrap());
    let second = thread::spawn(move || second_wait.recv().unwrap());
    let mut thread = Thread::default();
    let mut exclusive = thread.as_mut();
    // SAFETY: nothing joins or detaches through `thread`; the handles join both threads.
    unsafe { exclusive.as_mut().set_thread(first.as_pthread_t()) };
    assert_eq!(exclusive.as_ref().thread(), first.as_pthread_t());
    // SAFETY: as above.
    unsafe { exclusive.set_thread(second.as_pthread_t()) };
    assert_eq!(thread.as_ref().thread(), second.as_pthread_t());
    first_release.send(()).unwrap();
    second_release.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();
}

/// Ids that name no thread, zero and the integer's bounds, are stored and read back as any
/// other: nothing joins through the object, so the contract asks nothing of them.
#[test]
fn stores_ids_that_name_no_thread() {
    for id in [0, 1, RawPthread::MAX] {
        let mut thread = Thread::default();
        // SAFETY: nothing joins or detaches through `thread`.
        unsafe { thread.as_mut().set_thread(id) };
        assert_eq!(thread.as_ref().thread(), id);
    }
}

/// Objects stored in a vector, as C's per-thread storage is an array, each hold their own id,
/// and moving the vector keeps them.
#[test]
fn each_object_of_an_array_holds_its_own_id() {
    let (release, wait) = mpsc::channel::<()>();
    let wait = std::sync::Arc::new(std::sync::Mutex::new(wait));
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let wait = std::sync::Arc::clone(&wait);
            thread::spawn(move || {
                let _ = wait.lock().unwrap().recv();
            })
        })
        .collect();
    let mut threads: Vec<Thread> = (0..3).map(|_| Thread::default()).collect();
    for (thread, worker) in threads.iter_mut().zip(&workers) {
        // SAFETY: nothing joins or detaches through the objects; `workers` join the threads.
        unsafe { thread.as_mut().set_thread(worker.as_pthread_t()) };
    }
    let moved: Vec<Thread> = threads.into_iter().collect();
    for (thread, worker) in moved.iter().zip(&workers) {
        assert_eq!(thread.as_ref().thread(), worker.as_pthread_t());
    }
    drop(release);
    for worker in workers {
        worker.join().unwrap();
    }
}
