//! `src/util/thread.c`'s thread-local storage on its public API: [`TlsKey`] (`git_tlsdata_key`)
//! and its routines, `git_tlsdata_init`, `git_tlsdata_set`, `git_tlsdata_get` and
//! `git_tlsdata_dispose`, with their destructor ([`TlsdataInitDestroyFn`]), and
//! `git_threads_global_init` ([`global_init`]). The first two tests carry over
//! `tests/libgit2/threads/tlsdata.c`'s two cases; the others hold what POSIX's keys do that C's
//! callers rely on, and where the port departs from it. None reaches C, so Miri runs them all.
//!
//! A destructor carries no context, so each test's values carry the log their destructor
//! writes to, and tests that run at once do not share one.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::{
    TlsError, TlsKey, TlsdataInitDestroyFn, global_init,
};

/// A value that names the log its destructor writes its tag to.
#[derive(Clone, Debug)]
struct Logged {
    tag: usize,
    log: Arc<Mutex<Vec<usize>>>,
}

/// The destructor of [`Logged`] values: records the value's tag.
fn record(value: Logged) {
    let Logged { tag, log } = value;
    log.lock().unwrap_or_else(PoisonError::into_inner).push(tag);
}

/// A key whose destructor is [`record`].
fn logged_key() -> TlsKey<Logged> {
    TlsKey::new(Some(TlsdataInitDestroyFn::new(record))).expect("a key")
}

/// A new log.
fn log() -> Arc<Mutex<Vec<usize>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// The tags `log` holds, sorted.
fn tags(log: &Mutex<Vec<usize>>) -> Vec<usize> {
    let mut tags = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    tags.sort_unstable();
    tags
}

/// `test_threads_tlsdata__can_set_and_get`: three keys, each set to its own value and read
/// back, then disposed of.
#[test]
fn can_set_and_get() {
    let key_one = TlsKey::<usize>::new(None).expect("key one");
    let key_two = TlsKey::<usize>::new(None).expect("key two");
    let key_three = TlsKey::<usize>::new(None).expect("key three");

    assert_eq!(key_one.set(Some(42_424_242)), Ok(None));
    assert_eq!(key_two.set(Some(0xDEAD_BEEF)), Ok(None));
    assert_eq!(key_three.set(Some(98_761_234)), Ok(None));

    assert_eq!(key_one.get(), Some(42_424_242));
    assert_eq!(key_two.get(), Some(0xDEAD_BEEF));
    assert_eq!(key_three.get(), Some(98_761_234));

    assert_eq!(key_one.dispose(), Ok(()));
    assert_eq!(key_two.dispose(), Ok(()));
    assert_eq!(key_three.dispose(), Ok(()));
}

/// `test_threads_tlsdata__threads`: ten threads share one key, each storing a value of its own
/// (C's, the address of a local) and reading it back.
#[test]
fn threads() {
    const THREAD_COUNT: usize = 10;
    let key = TlsKey::<Arc<usize>>::new(None).expect("a key");
    let threads: Vec<_> = (0..THREAD_COUNT)
        .map(|i| {
            thread::spawn(move || {
                let val = Arc::new(i);
                key.set(Some(Arc::clone(&val))).is_ok()
                    && key.get().is_some_and(|got| Arc::ptr_eq(&got, &val))
            })
        })
        .collect();
    for thread in threads {
        assert!(thread.join().expect("the thread ends"));
    }
    assert_eq!(key.dispose(), Ok(()));
}

/// A thread holds no value of a key until it stores one, whatever another thread stores, and
/// storing `None` removes its value, as C's `NULL` does.
#[test]
fn a_value_is_absent_until_stored_and_after_none() {
    let key = TlsKey::<u32>::new(None).expect("a key");
    assert_eq!(key.get(), None);
    assert_eq!(key.set(Some(7)), Ok(None));
    thread::spawn(move || assert_eq!(key.get(), None))
        .join()
        .expect("the thread ends");
    assert_eq!(key.get(), Some(7));
    assert_eq!(key.set(None), Ok(Some(7)));
    assert_eq!(key.get(), None);
    assert_eq!(key.set(None), Ok(None));
    assert_eq!(key.dispose(), Ok(()));
}

/// Storing a value hands back the one it replaces, which the destructor does not run on, as
/// C's `git_tlsdata_set` does not destroy it.
#[test]
fn set_returns_the_value_it_replaces_without_destroying_it() {
    let log = log();
    let key = logged_key();
    let first = Logged {
        tag: 1,
        log: Arc::clone(&log),
    };
    let second = Logged {
        tag: 2,
        log: Arc::clone(&log),
    };
    assert!(key.set(Some(first)).expect("stored").is_none());
    let replaced = key.set(Some(second)).expect("stored").expect("the first");
    assert_eq!(replaced.tag, 1);
    assert_eq!(tags(&log), Vec::<usize>::new());
    assert_eq!(
        key.set(None).expect("removed").map(|value| value.tag),
        Some(2)
    );
    assert_eq!(tags(&log), Vec::<usize>::new());
    assert_eq!(key.dispose(), Ok(()));
}

/// At a thread's exit, the destructor runs once on each value the thread still holds of a live
/// key, on no value it removed or replaced, and not at all on a thread that holds none.
#[test]
fn the_destructor_runs_at_thread_exit_on_the_values_held() {
    let log = log();
    let key = logged_key();
    let other = logged_key();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let log = Arc::clone(&log);
            thread::spawn(move || match i {
                // Holds a value of each key.
                0 => {
                    key.set(Some(Logged {
                        tag: 10,
                        log: Arc::clone(&log),
                    }))
                    .expect("stored");
                    other.set(Some(Logged { tag: 11, log })).expect("stored");
                }
                // Replaces its value: only the last one is destroyed.
                1 => {
                    key.set(Some(Logged {
                        tag: 20,
                        log: Arc::clone(&log),
                    }))
                    .expect("stored");
                    let replaced = key.set(Some(Logged { tag: 21, log })).expect("stored");
                    drop(replaced);
                }
                // Removes its value: nothing is destroyed.
                2 => {
                    key.set(Some(Logged { tag: 30, log })).expect("stored");
                    let removed = key.set(None).expect("removed");
                    drop(removed);
                }
                // Holds none.
                _ => drop(log),
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("the thread ends");
    }
    assert_eq!(tags(&log), [10, 11, 21]);
    assert_eq!(key.dispose(), Ok(()));
    assert_eq!(other.dispose(), Ok(()));
}

/// A key without a destructor drops the values its threads still hold at their exit.
#[test]
fn a_key_without_a_destructor_drops_its_values_at_thread_exit() {
    let value = Arc::new(());
    let key = TlsKey::<Arc<()>>::new(None).expect("a key");
    let held = Arc::clone(&value);
    thread::spawn(move || key.set(Some(held)).map(drop))
        .join()
        .expect("the thread ends")
        .expect("stored");
    assert_eq!(Arc::strong_count(&value), 1);
    assert_eq!(key.dispose(), Ok(()));
}

/// Once a key is disposed of, no copy of it names a key: reading gives nothing, storing and
/// disposing again fail, and its destructor runs on no thread's value, while those values are
/// still dropped.
#[test]
fn a_disposed_key_names_no_key_and_destroys_nothing() {
    let log = log();
    let key = logged_key();
    let copy = key;
    key.set(Some(Logged {
        tag: 1,
        log: Arc::clone(&log),
    }))
    .expect("stored");

    // A thread that holds a value when the key is disposed of, then exits.
    let (stored_tx, stored_rx) = std::sync::mpsc::channel();
    let (disposed_tx, disposed_rx) = std::sync::mpsc::channel::<()>();
    let thread_log = Arc::clone(&log);
    let holder = thread::spawn(move || {
        key.set(Some(Logged {
            tag: 2,
            log: thread_log,
        }))
        .expect("stored");
        stored_tx.send(()).expect("sent");
        disposed_rx.recv().expect("disposed");
        key.get().map(|value| value.tag)
    });
    stored_rx.recv().expect("stored");

    assert_eq!(copy.dispose(), Ok(()));
    disposed_tx.send(()).expect("sent");
    assert_eq!(holder.join().expect("the thread ends"), None);

    assert!(key.get().is_none());
    assert_eq!(
        key.set(None).map(|value| value.is_some()),
        Err(TlsError::Disposed)
    );
    let refused = Logged {
        tag: 3,
        log: Arc::clone(&log),
    };
    assert_eq!(
        key.set(Some(refused)).map(|value| value.is_some()),
        Err(TlsError::Disposed)
    );
    assert_eq!(key.dispose(), Err(TlsError::Disposed));
    assert_eq!(tags(&log), Vec::<usize>::new());
    // Both threads' values, and the refused one, are dropped: only `log` holds it now.
    assert_eq!(Arc::strong_count(&log), 1);
}

/// No key is made with a disposed key's id, so an old copy never reaches a new key's values.
#[test]
fn ids_are_never_reused() {
    let old = TlsKey::<u8>::new(None).expect("a key");
    old.dispose().expect("disposed");
    let new = TlsKey::<u8>::new(None).expect("a key");
    assert_ne!(old, new);
    new.set(Some(1)).expect("stored");
    assert_eq!(old.get(), None);
    assert_eq!(new.get(), Some(1));
    assert_eq!(new.dispose(), Ok(()));
}

/// Keys of different value types live side by side, each with its own values.
#[test]
fn keys_of_different_types_are_independent() {
    let number = TlsKey::<u64>::new(None).expect("a key");
    let text = TlsKey::<String>::new(None).expect("a key");
    number.set(Some(u64::MAX)).expect("stored");
    text.set(Some("text".to_owned())).expect("stored");
    assert_eq!(number.get(), Some(u64::MAX));
    assert_eq!(text.get().as_deref(), Some("text"));
    number.dispose().expect("disposed");
    assert_eq!(text.get().as_deref(), Some("text"));
    text.dispose().expect("disposed");
}

/// A key in a `static` serves every thread, each with its own value.
#[test]
fn a_static_key_serves_every_thread() {
    static KEY: std::sync::OnceLock<TlsKey<usize>> = std::sync::OnceLock::new();
    let key = || *KEY.get_or_init(|| TlsKey::new(None).expect("a key"));
    key().set(Some(usize::MAX)).expect("stored");
    let threads: Vec<_> = (0..4)
        .map(|i| {
            thread::spawn(move || {
                assert_eq!(key().get(), None);
                key().set(Some(i)).expect("stored");
                key().get()
            })
        })
        .collect();
    for (i, thread) in threads.into_iter().enumerate() {
        assert_eq!(thread.join().expect("the thread ends"), Some(i));
    }
    assert_eq!(key().get(), Some(usize::MAX));
}

/// What a value's code saw of a key: its read, and what storing a value under it returned.
type Seen = (Option<u8>, Result<Option<u8>, TlsError>);

/// A value whose destructor stores a value of a key, as the thread exits, fails to store it,
/// and reads nothing: the departure from POSIX, which runs the destructors again on what they
/// stored.
#[test]
fn a_destructor_cannot_store_while_its_thread_exits() {
    /// What a destructor saw of the storage as its thread exited.
    #[derive(Clone)]
    struct Probe {
        other: TlsKey<u8>,
        seen: Arc<Mutex<Option<Seen>>>,
    }
    fn probe(value: Probe) {
        let Probe { other, seen } = value;
        let read = other.get();
        let stored = other.set(Some(2));
        *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some((read, stored));
    }
    let other = TlsKey::<u8>::new(None).expect("a key");
    let key = TlsKey::new(Some(TlsdataInitDestroyFn::new(probe))).expect("a key");
    let seen = Arc::new(Mutex::new(None));
    let probe_value = Probe {
        other,
        seen: Arc::clone(&seen),
    };
    thread::spawn(move || {
        other.set(Some(1)).expect("stored");
        key.set(Some(probe_value)).map(drop)
    })
    .join()
    .expect("the thread ends")
    .expect("stored");
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner).take();
    assert_eq!(seen, Some((None, Err(TlsError::Unavailable))));
    key.dispose().expect("disposed");
    other.dispose().expect("disposed");
}

/// A value's `Clone`, which `get` runs while it reads the thread's values, may read them, but
/// storing a value from it fails rather than replacing what is being read.
#[test]
fn a_clone_run_by_get_reads_but_cannot_store() {
    /// Reads and stores a value of `other` when cloned.
    struct Reentrant {
        other: TlsKey<u8>,
        seen: Arc<Mutex<Vec<Seen>>>,
    }
    impl Clone for Reentrant {
        fn clone(&self) -> Reentrant {
            let read = self.other.get();
            let stored = self.other.set(Some(9));
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((read, stored));
            Reentrant {
                other: self.other,
                seen: Arc::clone(&self.seen),
            }
        }
    }
    let other = TlsKey::<u8>::new(None).expect("a key");
    let key = TlsKey::<Reentrant>::new(None).expect("a key");
    let seen = Arc::new(Mutex::new(Vec::new()));
    other.set(Some(1)).expect("stored");
    key.set(Some(Reentrant {
        other,
        seen: Arc::clone(&seen),
    }))
    .expect("stored");
    assert!(key.get().is_some());
    assert_eq!(
        *seen.lock().unwrap_or_else(PoisonError::into_inner),
        [(Some(1), Err(TlsError::Unavailable))]
    );
    assert_eq!(other.get(), Some(1));
    key.dispose().expect("disposed");
    other.dispose().expect("disposed");
}

/// `with` lends this thread's value, of a type that need not be `Clone`, without moving it,
/// and lends `None` where `get` would give it: before a value is stored, once `None` is, and
/// once the key is disposed of. Inside it, another key's value is read, and a store fails.
#[test]
fn with_lends_the_value_where_get_would_clone_it() {
    /// A value that is not `Clone`.
    struct Buffer(Vec<u8>);
    let key = TlsKey::<Buffer>::new(None).expect("a key");
    let other = TlsKey::<u8>::new(None).expect("a key");
    assert!(key.with(|buffer| buffer.is_none()));
    key.set(Some(Buffer(vec![1, 2]))).expect("stored");
    other.set(Some(5)).expect("stored");
    let lent = key.with(|buffer| {
        let buffer = buffer.expect("a value");
        (buffer.0.clone(), other.get(), other.set(Some(6)))
    });
    assert_eq!(lent, (vec![1, 2], Some(5), Err(TlsError::Unavailable)));
    assert_eq!(other.get(), Some(5));
    // The value stays where `with` found it.
    assert_eq!(
        key.with(|buffer| buffer.map(|buffer| buffer.0.len())),
        Some(2)
    );
    thread::spawn(move || key.with(|buffer| assert!(buffer.is_none())))
        .join()
        .expect("the thread ends");
    assert!(key.set(None).expect("removed").is_some());
    assert!(key.with(|buffer| buffer.is_none()));
    key.set(Some(Buffer(Vec::new()))).expect("stored");
    key.dispose().expect("disposed");
    assert!(key.with(|buffer| buffer.is_none()));
    other.dispose().expect("disposed");
}

/// The failures say what failed, as errors.
#[test]
fn errors_describe_the_failure() {
    let cases = [
        (TlsError::KeysExhausted, "no thread-local key is left"),
        (
            TlsError::Disposed,
            "the thread-local key has been disposed of",
        ),
        (
            TlsError::Unavailable,
            "the thread's local storage is unavailable",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert!(boxed.source().is_none());
    }
}

/// A key prints as its id, and a destructor as itself; copies of either are the same.
#[test]
fn keys_and_destructors_print_and_copy() {
    let key = TlsKey::<()>::new(None).expect("a key");
    let copy = key;
    assert_eq!(copy, key);
    assert_eq!(format!("{key:?}"), format!("{copy:?}"));
    assert!(format!("{key:?}").starts_with("TlsKey("));
    let destructor = TlsdataInitDestroyFn::new(drop::<()>);
    let copied = destructor;
    assert!(format!("{copied:?}").starts_with("TlsdataInitDestroyFn"));
    assert!(format!("{destructor:?}").starts_with("TlsdataInitDestroyFn"));
    let mut seen = std::collections::HashSet::new();
    assert!(seen.insert(key));
    assert!(!seen.insert(copy));
    key.dispose().expect("disposed");
}

/// `Clone`, as generic code reaches it, gives the same key and a destructor that runs the
/// same function.
#[test]
fn clones_are_copies() {
    fn cloned<T: Clone>(value: &T) -> T {
        value.clone()
    }
    let log = log();
    let key = logged_key();
    assert_eq!(cloned(&key), key);
    let destructor = cloned(&TlsdataInitDestroyFn::new(record));
    let other = TlsKey::new(Some(destructor)).expect("a key");
    let thread_log = Arc::clone(&log);
    thread::spawn(move || {
        other
            .set(Some(Logged {
                tag: 5,
                log: thread_log,
            }))
            .map(drop)
    })
    .join()
    .expect("the thread ends")
    .expect("stored");
    assert_eq!(tags(&log), [5]);
    key.dispose().expect("disposed");
    other.dispose().expect("disposed");
}

/// `git_threads_global_init` sets nothing up on POSIX: it may run any number of times.
#[test]
fn global_init_sets_nothing_up() {
    global_init();
    global_init();
    let key = TlsKey::<u8>::new(None).expect("a key");
    key.dispose().expect("disposed");
}
