//! `src/util/thread.c`'s thread-local storage on the safe API: [`TlsKey`]s made, used and
//! disposed of from many threads at once, values whose drops and clones reach the storage
//! while it is borrowed or destroyed, and values that outlive their key's disposal. Under
//! `TSan` they race nothing, and under Miri they reach no undefined behaviour: no value is
//! used after it is dropped, none is dropped twice, and none is leaked but by design.

use std::rc::Rc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Barrier, OnceLock};
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::{TlsError, TlsKey, TlsdataInitDestroyFn};

/// Threads per test, and rounds per thread: enough interleavings for `TSan` and Miri to see,
/// few enough for Miri to finish.
const THREADS: usize = 4;
const ROUNDS: usize = if cfg!(miri) { 4 } else { 200 };

/// A value that counts its drops.
struct Counted(Arc<AtomicUsize>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_add(1, SeqCst);
    }
}

/// Threads make, use and dispose of keys of their own while they share a `static` one, so that
/// the registry and the disposals counter change under every thread's table: each sees its own
/// values alone, and every value is dropped exactly once.
#[test]
fn keys_made_and_disposed_while_threads_share_one() {
    static SHARED: OnceLock<TlsKey<Rc<usize>>> = OnceLock::new();
    let shared = *SHARED.get_or_init(|| TlsKey::new(None).expect("a key"));
    let drops = Arc::new(AtomicUsize::new(0));
    // Joined, rather than scoped, so that each thread's exit has run when its join returns.
    let threads: Vec<_> = (0..THREADS)
        .map(|i| {
            let drops = Arc::clone(&drops);
            thread::spawn(move || {
                for round in 0..ROUNDS {
                    let own = TlsKey::<Counted>::new(None).expect("a key");
                    assert!(
                        own.set(Some(Counted(Arc::clone(&drops))))
                            .expect("stored")
                            .is_none()
                    );
                    shared
                        .set(Some(Rc::new(i * ROUNDS + round)))
                        .expect("stored");
                    assert_eq!(shared.get().as_deref(), Some(&(i * ROUNDS + round)));
                    let replaced = own.set(Some(Counted(Arc::clone(&drops)))).expect("stored");
                    drop(replaced);
                    // Half the keys keep their value past their disposal.
                    if round % 2 == 0 {
                        drop(own.set(None).expect("removed"));
                    }
                    own.dispose().expect("disposed");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("the thread ends");
    }
    assert_eq!(drops.load(SeqCst), 2 * THREADS * ROUNDS);
}

/// A value its thread still holds when another thread disposes of its key is dropped once, by
/// its own thread, and never given to the key's destructor.
#[test]
fn values_of_a_key_disposed_elsewhere_are_dropped_once() {
    fn destroy(value: Counted) {
        drop(value);
        unreachable!("a disposed key's destructor runs");
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let key = TlsKey::new(Some(TlsdataInitDestroyFn::new(destroy))).expect("a key");
    let stored = Arc::new(Barrier::new(THREADS + 1));
    let disposed = Arc::new(Barrier::new(THREADS + 1));
    let threads: Vec<_> = (0..THREADS)
        .map(|i| {
            let (drops, stored, disposed) = (
                Arc::clone(&drops),
                Arc::clone(&stored),
                Arc::clone(&disposed),
            );
            thread::spawn(move || {
                key.set(Some(Counted(Arc::clone(&drops)))).expect("stored");
                stored.wait();
                disposed.wait();
                // Half the threads reach their storage again, the others only exit.
                if i % 2 == 0 {
                    let taken = key.set(None).map(|value| value.is_some());
                    assert_eq!(taken, Err(TlsError::Disposed));
                    assert!(drops.load(SeqCst) >= 1);
                }
            })
        })
        .collect();
    stored.wait();
    key.dispose().expect("disposed");
    disposed.wait();
    for thread in threads {
        thread.join().expect("the thread ends");
    }
    assert_eq!(drops.load(SeqCst), THREADS);
}

/// Values that reach the storage from their drops: a replaced value, a refused one, a value of
/// a disposed key and one destroyed at its thread's exit each store a value of another key,
/// which succeeds once the storage is released, and fails while it is being destroyed.
#[test]
fn drops_that_reach_the_storage() {
    /// Stores a value under `other` when dropped, and counts the stores that succeed.
    struct Reaching {
        other: TlsKey<u8>,
        stores: Arc<AtomicUsize>,
    }
    impl Drop for Reaching {
        fn drop(&mut self) {
            if self.other.set(Some(1)).is_ok() {
                self.stores.fetch_add(1, SeqCst);
            }
        }
    }
    let stores = Arc::new(AtomicUsize::new(0));
    let own = TlsKey::<Reaching>::new(None).expect("a key");
    let other = TlsKey::<u8>::new(None).expect("a key");
    let reaching = {
        let stores = Arc::clone(&stores);
        move || Reaching {
            other,
            stores: Arc::clone(&stores),
        }
    };
    thread::spawn(move || {
        // Replaced: dropped by the caller, with the storage free.
        own.set(Some(reaching())).expect("stored");
        drop(own.set(Some(reaching())).expect("stored"));
        // Destroyed at the thread's exit: its store fails.
    })
    .join()
    .expect("the thread ends");
    assert_eq!(stores.load(SeqCst), 1);
    let disposed = TlsKey::<Reaching>::new(None).expect("a key");
    let thread_stores = Arc::clone(&stores);
    thread::spawn(move || {
        // Its key disposed of: dropped as the thread next reaches its storage, after the read.
        let value = Reaching {
            other,
            stores: Arc::clone(&thread_stores),
        };
        disposed.set(Some(value)).expect("stored");
        disposed.dispose().expect("disposed");
        assert_eq!(other.get(), None);
        assert_eq!(other.get(), Some(1));
        // Refused, for the disposed key: dropped once the storage is released.
        let refused = Reaching {
            other,
            stores: thread_stores,
        };
        let refusal = disposed.set(Some(refused)).map(|value| value.is_some());
        assert_eq!(refusal, Err(TlsError::Disposed));
    })
    .join()
    .expect("the thread ends");
    assert_eq!(stores.load(SeqCst), 3);
    own.dispose().expect("disposed");
    other.dispose().expect("disposed");
}

/// A destructor that reads and stores values of its own key and of another as its thread
/// exits: the storage is out of reach, and nothing is used after it is dropped.
#[test]
fn destructors_that_reach_the_storage_at_exit() {
    /// A value naming the keys its destructor reaches.
    struct Exiting {
        own: TlsKey<Exiting>,
        other: TlsKey<Rc<u8>>,
        reached: Arc<AtomicUsize>,
    }
    fn destroy(value: Exiting) {
        let Exiting {
            own,
            other,
            reached,
        } = value;
        if other.get().is_some() || other.set(Some(Rc::new(1))).is_ok() {
            reached.fetch_add(1, SeqCst);
        }
        drop(own.set(None));
        let _ = own.dispose();
    }
    let reached = Arc::new(AtomicUsize::new(0));
    let own = TlsKey::new(Some(TlsdataInitDestroyFn::new(destroy))).expect("a key");
    let other = TlsKey::<Rc<u8>>::new(None).expect("a key");
    // Every thread stores before any exits, and so before the first destructor disposes of
    // the key.
    let stored = Arc::new(Barrier::new(THREADS));
    let threads: Vec<_> = (0..THREADS)
        .map(|_| {
            let (reached, stored) = (Arc::clone(&reached), Arc::clone(&stored));
            thread::spawn(move || {
                other.set(Some(Rc::new(0))).expect("stored");
                own.set(Some(Exiting {
                    own,
                    other,
                    reached,
                }))
                .expect("stored");
                stored.wait();
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("the thread ends");
    }
    assert_eq!(reached.load(SeqCst), 0);
    // The first destructor disposed of its own key; the others found it disposed.
    assert_eq!(own.dispose(), Err(TlsError::Disposed));
    other.dispose().expect("disposed");
}

/// A value's `Clone`, run by `get` while the thread's table is borrowed, that disposes of its
/// own key, reads it and stores under it: the value being cloned stays in place until the
/// clone is made.
#[test]
fn a_clone_that_disposes_of_its_key() {
    /// Disposes of `key`, and stores under it, when cloned.
    struct Disposing {
        key: OnceLock<TlsKey<Disposing>>,
        payload: Rc<String>,
    }
    impl Clone for Disposing {
        fn clone(&self) -> Disposing {
            let key = *self.key.get().expect("the key");
            let _ = key.dispose();
            assert!(key.get().is_none());
            let copy = Disposing {
                key: OnceLock::new(),
                payload: Rc::new(String::new()),
            };
            assert_eq!(
                key.set(Some(copy)).map(|value| value.is_some()),
                Err(TlsError::Unavailable)
            );
            Disposing {
                key: self.key.clone(),
                payload: Rc::clone(&self.payload),
            }
        }
    }
    let key = TlsKey::<Disposing>::new(None).expect("a key");
    let cell = OnceLock::new();
    cell.set(key).expect("set");
    key.set(Some(Disposing {
        key: cell,
        payload: Rc::new("payload".to_owned()),
    }))
    .expect("stored");
    let copy = key.get().expect("cloned");
    assert_eq!(copy.payload.as_str(), "payload");
    assert!(key.get().is_none());
    assert_eq!(Rc::strong_count(&copy.payload), 1);
}
