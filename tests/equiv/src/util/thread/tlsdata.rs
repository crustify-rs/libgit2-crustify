//! `src/util/thread.c`'s thread-local storage against its C: the POSIX keys of
//! `git_tlsdata_init`, `git_tlsdata_set`, `git_tlsdata_get` and `git_tlsdata_dispose`, reached
//! through their raw bindings, and [`TlsKey`], each run through the same calls, compared after
//! each step: their status as `Ok` or `Err`, their values with `NULL` as `None`, and which
//! values their destructors run on at each thread's exit. C's values are the integers C's own
//! test stores, as pointers nothing reads through; Rust's the same integers.
//!
//! What they leave unspecified is not compared: the keys' ids, and every call on a disposed key,
//! which POSIX leaves undefined. glibc gives a disposed key's index to the next key made, in
//! this process by any test that runs beside these, so that C's call would reach another
//! test's key: no test makes one. Where the port departs from POSIX, the module's documentation
//! says so, and its unit tests hold the port's behaviour: its refusals on a disposed key, key 0,
//! which POSIX may give, and a destructor that stores again at its thread's exit.
//! `git_tlsdata_*` need no `git_libgit2_init`, so the tests do not run it.

use core::ffi::c_void;
use core::ptr;
use std::sync::{Barrier, Mutex, PoisonError};
use std::thread;

use libgit2_crustify::libgit2::src::util::thread::{TlsError, TlsKey, TlsdataInitDestroyFn};
use libgit2_crustify::sys::libgit2 as sys;

/// A POSIX key C makes, and the Rust key made beside it.
struct Pair {
    c: sys::pthread_key_t,
    rust: TlsKey<usize>,
}

impl Pair {
    /// A pair of keys, with `c_destroy` and `rust_destroy` as their destructors.
    fn new(
        c_destroy: Option<unsafe extern "C" fn(*mut c_void)>,
        rust_destroy: Option<TlsdataInitDestroyFn<usize>>,
    ) -> Pair {
        let mut c = 0;
        // SAFETY: `c` is a local; the destructors these tests pass record the address alone,
        // on any thread.
        assert_eq!(unsafe { sys::git_tlsdata_init(&raw mut c, c_destroy) }, 0);
        let rust = TlsKey::new(rust_destroy).expect("a key");
        Pair { c, rust }
    }

    /// Stores `value` on both sides, 0 as `NULL` and `None`, and returns the outcomes.
    fn set(&self, value: usize) -> (bool, bool) {
        let c_value = ptr::without_provenance_mut::<c_void>(value);
        // SAFETY: the value is an integer nothing reads through, which the destructors record.
        let c = unsafe { sys::git_tlsdata_set(self.c, c_value) } == 0;
        let rust = self.rust.set((value != 0).then_some(value)).is_ok();
        (c, rust)
    }

    /// Both sides' values on this thread, `NULL` and `None` as 0.
    fn get(&self) -> (usize, usize) {
        // SAFETY: a read of this thread's value, which C hands back unread.
        let c = unsafe { sys::git_tlsdata_get(self.c) }.addr();
        (c, self.rust.get().unwrap_or(0))
    }

    /// Disposes of both keys, and returns the outcomes.
    fn dispose(&self) -> (bool, bool) {
        // SAFETY: disposing of a key runs no destructor and reads no value.
        let c = unsafe { sys::git_tlsdata_dispose(self.c) } == 0;
        (c, self.rust.dispose().is_ok())
    }
}

/// `test_threads_tlsdata__can_set_and_get`'s calls, and more: three keys set, read, cleared with
/// `NULL`, set again and disposed of.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_get_and_dispose_match_c() {
    let keys = [
        Pair::new(None, None),
        Pair::new(None, None),
        Pair::new(None, None),
    ];
    let values = [42_424_242, 0xDEAD_BEEF, 98_761_234];
    for key in &keys {
        assert_eq!(key.get(), (0, 0));
    }
    for (key, value) in keys.iter().zip(values) {
        assert_eq!(key.set(value), (true, true));
    }
    for (key, value) in keys.iter().zip(values) {
        assert_eq!(key.get(), (value, value));
    }
    assert_eq!(keys[1].set(0), (true, true));
    assert_eq!(keys[1].get(), (0, 0));
    assert_eq!(keys[1].set(7), (true, true));
    assert_eq!(keys[1].get(), (7, 7));
    for key in &keys {
        assert_eq!(key.dispose(), (true, true));
    }
}

/// `test_threads_tlsdata__threads`'s calls on both sides: ten threads share a key, each storing
/// its own value and reading it back.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn threads_match_c() {
    const THREAD_COUNT: usize = 10;
    let key = Pair::new(None, None);
    let (c, rust) = (key.c, key.rust);
    let threads: Vec<_> = (1..=THREAD_COUNT)
        .map(|i| {
            thread::spawn(move || {
                let pair = Pair { c, rust };
                let stored = pair.set(i);
                (stored, pair.get())
            })
        })
        .collect();
    for (i, thread) in (1..=THREAD_COUNT).zip(threads) {
        assert_eq!(
            thread.join().expect("the thread ends"),
            ((true, true), (i, i))
        );
    }
    assert_eq!(key.get(), (0, 0));
    assert_eq!(key.dispose(), (true, true));
}

/// The values C's destructor ran on, at its threads' exits.
static C_LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// The values Rust's destructor ran on.
static RUST_LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// C's destructor: records the value it is given, and reads nothing through it.
unsafe extern "C" fn c_record(value: *mut c_void) {
    C_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(value.addr());
}

/// Rust's destructor: records the value it is given.
fn rust_record(value: usize) {
    RUST_LOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(value);
}

/// What both destructors recorded, sorted.
fn recorded() -> (Vec<usize>, Vec<usize>) {
    let sorted = |log: &Mutex<Vec<usize>>| {
        let mut values = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
        values.sort_unstable();
        values
    };
    (sorted(&C_LOG), sorted(&RUST_LOG))
}

/// At each thread's exit, both destructors run on the same values: those the thread still
/// holds of a live key, after storing, replacing and removing values, and none once the key is
/// disposed of while a thread holds a value, nor on the values of a key without a destructor.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn destructors_run_on_the_values_c_destroys() {
    let key = Pair::new(Some(c_record), Some(TlsdataInitDestroyFn::new(rust_record)));
    let bare = Pair::new(None, None);
    let (c, rust, bare_c, bare_rust) = (key.c, key.rust, bare.c, bare.rust);
    let pair = move || Pair { c, rust };
    let bare_pair = move || Pair {
        c: bare_c,
        rust: bare_rust,
    };

    // Each thread's script, run on both sides at once, each step compared.
    let scripts: [&[usize]; 5] = [&[10], &[20, 21], &[30, 0], &[], &[40, 0, 41]];
    let threads: Vec<_> = scripts
        .into_iter()
        .map(|script| {
            thread::spawn(move || {
                let key = pair();
                let bare = bare_pair();
                for &value in script {
                    assert_eq!(key.set(value), (true, true));
                    assert_eq!(key.get(), (value, value));
                    assert_eq!(bare.set(value + 1), (true, true));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("the thread ends");
    }
    assert_eq!(recorded(), (vec![10, 21, 41], vec![10, 21, 41]));

    // A thread that holds a value when the key is disposed of, then exits.
    let stored = std::sync::Arc::new(Barrier::new(2));
    let disposed = std::sync::Arc::new(Barrier::new(2));
    let (stored_there, disposed_there) = (stored.clone(), disposed.clone());
    let holder = thread::spawn(move || {
        let key = pair();
        assert_eq!(key.set(50), (true, true));
        stored_there.wait();
        disposed_there.wait();
    });
    stored.wait();
    assert_eq!(key.dispose(), (true, true));
    disposed.wait();
    holder.join().expect("the thread ends");
    assert_eq!(recorded(), (vec![10, 21, 41], vec![10, 21, 41]));
    assert_eq!(bare.dispose(), (true, true));
}

/// A key that names none is refused on both sides: C's -1 and `NULL` for an id glibc never
/// gives, past `PTHREAD_KEYS_MAX`, so that no test's key can take it, and Rust's
/// [`TlsError::Disposed`] and `None` for a disposed key, the one way a [`TlsKey`] names none.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn refusals_of_a_key_naming_none_match_c() {
    let none = sys::pthread_key_t::MAX;
    let rust = TlsKey::<usize>::new(None).expect("a key");
    rust.dispose().expect("disposed");
    // SAFETY: glibc refuses the id before it reaches any key's storage.
    let c = unsafe { sys::git_tlsdata_set(none, ptr::without_provenance_mut(1)) };
    assert_eq!((c, rust.set(Some(1))), (-1, Err(TlsError::Disposed)));
    // SAFETY: as above.
    let c = unsafe { sys::git_tlsdata_get(none) };
    assert_eq!((c.addr(), rust.get()), (0, None));
    // SAFETY: as above.
    let c = unsafe { sys::git_tlsdata_dispose(none) };
    assert_eq!((c, rust.dispose()), (-1, Err(TlsError::Disposed)));
}
