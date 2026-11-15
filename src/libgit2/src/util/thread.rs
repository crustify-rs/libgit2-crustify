//! `src/util/thread.c`: the threads, locks, atomics and thread-local storage libgit2 builds
//! on, ported to Rust.
//!
//! # Atomics
//!
//! An [`Atomic32`] is a `git_atomic32` and an [`Atomic64`] a `git_atomic64` (`thread.h`): one
//! integer, a `volatile int` and a `volatile int64_t`, that libgit2's threads share and change
//! only through `thread.h`'s atomic operations, `git_atomic32_inc` and its siblings, each a
//! sequentially consistent `__atomic` builtin in this build (`GIT_THREADS`, `GIT_BUILTIN_ATOMIC`).
//! C embeds them by value: the reference count of every counted object (`git_refcount`, and
//! `git_cached_obj`'s), a pack's and a pack file's counts, an index's readers, a transport's
//! cancellation flag and the repository's attribute session key, and keeps a few as globals,
//! the runtime's `init_count` and the object cache's `git_cache__current_storage`.
//! [`AtomicSsize`] is `git_atomic_ssize`, an [`Atomic64`] where pointers are 64 bits wide.
//!
//! The value is the whole object, and its holders change it through shared access, so the
//! handles lend it as the standard atomic it is, [`AtomicI32`](core::sync::atomic::AtomicI32) or
//! [`AtomicI64`](core::sync::atomic::AtomicI64), through
//! [`val`](Atomic32Ref::val): every Rust access is atomic, as every write C makes to one it
//! shares is (C also zeroes them, with the storage that holds them, before sharing it). C also
//! reads some of them plainly (`t->cancelled.val` in the smart transport,
//! `git_cache__current_storage.val` in `git_libgit2_opts`), which races with an atomic write made
//! on another thread as it races with C's own; nothing in Rust can make those reads atomic.
//!
//! A shared handle lends the atomic for writing, as `&AtomicI32` does, so a binding of a type
//! that embeds one whose value C relies on, such as a reference count, lends its holders no
//! handle to it, only what they may do with the value. A value Rust holds, as [`Atomic32::new`]
//! builds it, lives in an [`UnsafeCell`](core::cell::UnsafeCell), so that it may be written
//! through a shared borrow and a `static` of it is writable.
//!
//! `thread.h`'s operations over them are methods of the shared handles, each the one
//! sequentially consistent operation C's is: [`Atomic32Ref::get`], [`set`](Atomic32Ref::set),
//! [`inc`](Atomic32Ref::inc), [`dec`](Atomic32Ref::dec) and [`add`](Atomic32Ref::add), which
//! return the value they leave, as C's do, and [`Atomic64Ref`]'s `get`, `set` and `add`, which
//! are also `git_atomic_ssize`'s. Its operations over a pointer-sized slot,
//! `git_atomic_compare_and_swap`, `git_atomic_swap` and `git_atomic_load`, are
//! [`atomic_compare_and_swap`], [`atomic_swap`] and [`atomic_load`], over a standard
//! pointer-sized atomic ([`AtomicWord`]), and its fence, `GIT_MEMORY_BARRIER`, is
//! [`memory_barrier`]. C defines the eleven functions `static inline` in `thread.h`, which its
//! translation units expand, so until the module's switch the two operate on the same objects
//! with the same instructions: C11's sequentially consistent atomics, which Rust's
//! [`SeqCst`](core::sync::atomic::Ordering::SeqCst) are. The switch replaces each definition by
//! its prototype, and the C that remains, its macros' expansions and `util.h`'s
//! `GIT_REFCOUNT_INC` and `GIT_REFCOUNT_DEC` included, then calls the module's exports under
//! the functions' names, which run these operations over the object C passes: a call where C
//! inlined one instruction, until cross-language LTO inlines it back. The macros stay in the
//! header, unexported. The build is threaded (`GIT_THREADS`) with the `__atomic` builtins
//! (`GIT_BUILTIN_ATOMIC`); its other branches, Windows' `Interlocked` calls, the older `__sync`
//! builtins and the plain reads and writes of a build without threads, compute the same values
//! for every input, with orderings no stronger than these.
//!
//! # Threads
//!
//! A [`Thread`] is a `git_thread` (`unix/pthread.h`): the `pthread_t` of a thread that
//! `git_thread_create` starts into it and `git_thread_join` joins, C's only routines over it,
//! both macros. C holds them by value, in the storage of what it hands each thread (the pack
//! builder's `thread_params`, which it zeroes). Its one field is read by
//! [`thread`](ThreadRef::thread) and written by [`set_thread`](ThreadMut::set_thread), which is
//! `unsafe`: a join made through the object follows the id it stores. A zeroed value, as
//! [`Thread::default`] gives, names no thread, as C's storage does before `git_thread_create`.
//! A `pthread_t` joins once, so a [`Thread`] is neither `Clone` nor `Copy`.
//!
//! A thread Rust starts is native: [`spawn`] starts it, as `git_thread_create` does, and
//! the [`JoinHandle`] it returns owns it, joined once by [`JoinHandle::join`], as
//! `git_thread_join` joins, whose result is the closure's, as `pthread_join`'s `status` is the
//! start routine's. A [`Thread`] stays the C form of a thread C starts into storage of its own,
//! and a [`JoinHandle`] is never turned into one: the standard library cannot join a thread by
//! its raw id, so only C could join it. The [locks](#locks) describe why these are native.
//!
//! # Locks
//!
//! [`Mutex`], [`Condvar`] and [`RwLock`] are `unix/pthread.h`'s `git_mutex`, `git_cond` and
//! `git_rwlock`, which name `pthread_mutex_t`, `pthread_cond_t` and `pthread_rwlock_t`, their
//! operations each the POSIX function of its name with default attributes. The header keeps
//! them, and `git_thread_create` and `git_thread_join`, for the C that remains, which embeds its
//! locks in its own structs and expands the macros where it locks them. The Rust forms are not
//! linked to that C, and no lock or thread crosses between the two, so they are the standard
//! library's: [`std::sync::Mutex`], [`std::sync::Condvar`], [`std::sync::RwLock`] and
//! [`std::thread::Builder::spawn`], each behind a type of this module that carries its C names.
//!
//! Where C locks a lock and unlocks it later by name, Rust locks it into a guard, which its
//! drop or its `unlock` releases: C's rule that a read lock is released with
//! `git_rwlock_rdunlock` and a write lock with `git_rwlock_wrunlock` is the guard's type. A
//! guard is not [`Send`], so a lock is released on the thread that took it, as POSIX requires
//! of a default mutex and read-write lock. A lock guards the data it holds, a `T`, where C's
//! guard state kept beside them: a lock of C's kind is a `Mutex<()>`.
//!
//! The standard locks differ from C's in three ways, none of which libgit2's C relies on:
//!
//! - They cannot fail to initialise, where `pthread_mutex_init` and its siblings may report
//!   `EAGAIN` or `ENOMEM`, which libgit2 reports as an error: `new` is a `const fn`, and a lock may
//!   be a `static`, as `GIT_RWLOCK_STATIC_INIT` initialises one in C.
//! - A lock is poisoned when a thread panics while holding it, a state C has no form of: locking it
//!   then fails with a [`PoisonError`], which hands back the guard, so that a caller that can vouch
//!   for the data may go on with it. C's lock fails only with the errors POSIX lists, positive
//!   numbers, which libgit2's callers test with `< 0` and so never see.
//! - Taking a lock the calling thread already holds, a read lock included, deadlocks or panics,
//!   where it deadlocks or is undefined in C: `unix/pthread.h` forbids recursive read locks for the
//!   Windows implementation's sake.
//!
//! The macros are `unix/pthread.h`'s because this build defines `GIT_THREADS` and not
//! `GIT_WIN32`. The branches it does not take compute the same: on Windows, `win32/thread.h`
//! declares each as a function over a critical section, an event and a slim reader-writer
//! lock, which exclude and wake as these do; without `GIT_THREADS`, `thread.h` makes each lock
//! an `unsigned int` and each operation `git__noop_args`, returning 0, which excludes nothing
//! because no second thread exists, and its `git_thread_create` and `git_thread_join` are only
//! expanded under `#ifdef GIT_THREADS`.
//!
//! # Thread-local storage
//!
//! A [`TlsKey`] is a `git_tlsdata_key` (`thread.h`): a key made at run time, under which each
//! thread keeps a value of its own, with a destructor that runs on each thread's value at that
//! thread's exit. C keeps two, made at `git_libgit2_init`: `errors.c`'s, for each thread's
//! error state, and `oid.c`'s, for `git_oid_tostr_s`'s per-thread string. C's `thread.c` builds
//! them on POSIX keys (`pthread_key_create` and its siblings) where libgit2 is built with
//! threads, as here; this module reimplements them over the standard library alone: a
//! registry of the live keys, behind a lock, and one table per thread, in a `thread_local!`
//! whose destructor runs the keys' destructors when the thread exits, a thread C started
//! included. Each value holds its key's state beside it, so that reading or replacing a value
//! and a thread's exit take no lock: the lock serves making and disposing of keys and a
//! thread's first value of a key. A child forked from a threaded process may so run its
//! thread-local destructors at `exit`, as libgit2's `git_process_start` does when its
//! `execve` fails, without waiting on a lock that another thread of its parent held. The
//! values never leave the thread that stored them, so a key is `Send` and `Sync` whatever its
//! values are.
//!
//! A key's value is typed, [`TlsKey<T>`](TlsKey), where C's is a `void *`, and its destructor,
//! a [`TlsdataInitDestroyFn`], is a plain Rust function of the value. The C that remains
//! reaches the same keys through the module's exports, `git_tlsdata_init`, `git_tlsdata_set`,
//! `git_tlsdata_get` and `git_tlsdata_dispose`, once the module has switched; until then they
//! compile only under the `ffi-export-src-util-thread-c` feature. A key C makes holds C's
//! pointers and C's destructor, and no safe Rust reaches it: a [`TlsKey`] is made only by
//! [`TlsKey::new`]. They keep POSIX's behaviour where C's callers can see it: a value is
//! absent until a thread stores one, storing one destroys nothing, the destructor runs at a
//! thread's exit on the values it still holds of the keys still live, and disposing of a key
//! runs no destructor, on any thread. They depart from it where nothing in libgit2 relies on
//! it:
//!
//! - no key is 0, and a key is never reused, so a key's copy that outlives its disposal, or the
//!   zeroed `static` a C caller has not initialised yet, names no key: reading it gives nothing and
//!   storing under it fails. C's `threadstate_get` reads its key before `git_error_global_init` has
//!   made it, which on POSIX reads whatever another component keeps under key 0
//!   (`crustify/LESSONS.md`); here it reads nothing;
//! - a destructor that stores a value again, while its thread exits, fails to store it, where POSIX
//!   would run the destructors again, up to `PTHREAD_DESTRUCTOR_ITERATIONS` times; the destructors
//!   C passes, `errors.c`'s `threadstate_free` and `oid.c`'s `thread_str_free`, store nothing;
//! - the number of live keys is bounded by the ids alone, 2³² − 1 over the process's life, where
//!   glibc bounds the live keys by `PTHREAD_KEYS_MAX`, 1024; and
//! - the main thread's values are destroyed at `exit` too, as the standard library's thread-locals
//!   are on glibc, where POSIX destroys none of the main thread's.
//!
//! The configurations this build does not take differ in what disposing of a key does: without
//! threads (`!GIT_THREADS`) C keeps one value per key, in a table of 16, and runs the
//! destructor on it when the key is disposed of; on Windows (`FlsAlloc`) disposing of a key
//! runs its destructor on every fiber's value.

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

pub use self::ffi_import::{Atomic32, Atomic32Mut, Atomic32Ref, Thread, ThreadMut, ThreadRef};
#[cfg(target_pointer_width = "64")]
pub use self::ffi_import::{Atomic64, Atomic64Mut, Atomic64Ref};

/// The atomic signed size of `thread.h` (`git_atomic_ssize`), an [`Atomic64`] where pointers
/// are 64 bits wide (`GIT_ARCH_64`), as on every target this crate builds for; C keeps the
/// object cache's total size in one.
///
/// Wraps `tdf::git_atomic_ssize`
#[doc(alias = "git_atomic_ssize")]
#[cfg(target_pointer_width = "64")]
pub type AtomicSsize = Atomic64;

/// The atomic signed size of `thread.h` (`git_atomic_ssize`), an [`Atomic32`] where pointers
/// are narrower than 64 bits.
///
/// Wraps `tdf::git_atomic_ssize`
#[doc(alias = "git_atomic_ssize")]
#[cfg(not(target_pointer_width = "64"))]
pub type AtomicSsize = Atomic32;

// ---------------------------------------------------------------------------------------------
// Atomic operations (`thread.h`): `git_atomic32_*`, `git_atomic64_*`, `git_atomic_ssize_*`,
// the pointer-sized `git_atomic_*` and `GIT_MEMORY_BARRIER`.
// ---------------------------------------------------------------------------------------------

use core::sync::atomic::Ordering::SeqCst;
use core::sync::atomic::{AtomicIsize, AtomicPtr, AtomicUsize, fence};

/// A full memory fence, ordering every load and store before it with every one after it, as
/// `GIT_MEMORY_BARRIER` does: `__atomic_thread_fence(__ATOMIC_SEQ_CST)` in this build,
/// `MemoryBarrier()` on Windows, `__sync_synchronize()` with the older builtins, and nothing in
/// a build without threads, where no other thread observes the order.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::memory_barrier;
///
/// memory_barrier();
/// ```
///
/// Replaces `mac::GIT_MEMORY_BARRIER`
#[doc(alias = "GIT_MEMORY_BARRIER")]
#[inline]
pub fn memory_barrier() {
    fence(SeqCst);
}

impl Atomic32Ref<'_> {
    /// The value, read atomically.
    ///
    /// Replaces `fn::git_atomic32_get`
    #[doc(alias = "git_atomic32_get")]
    #[inline]
    #[must_use]
    pub fn get(&self) -> i32 {
        self.val().load(SeqCst)
    }

    /// Stores `val`, atomically.
    ///
    /// Replaces `fn::git_atomic32_set`
    #[doc(alias = "git_atomic32_set")]
    #[inline]
    pub fn set(&self, val: i32) {
        self.val().store(val, SeqCst);
    }

    /// Adds 1, atomically, and returns the value it leaves, which wraps from `i32::MAX` to
    /// `i32::MIN`, as the C builtin does.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::src::util::thread::Atomic32;
    ///
    /// let refcount = Atomic32::new(1);
    /// assert_eq!(refcount.as_ref().inc(), 2);
    /// assert_eq!(refcount.as_ref().dec(), 1);
    /// ```
    ///
    /// Replaces `fn::git_atomic32_inc`
    #[doc(alias = "git_atomic32_inc")]
    #[inline]
    #[expect(
        clippy::must_use_candidate,
        reason = "C callers discard what it returns"
    )]
    pub fn inc(&self) -> i32 {
        self.val().fetch_add(1, SeqCst).wrapping_add(1)
    }

    /// Subtracts 1, atomically, and returns the value it leaves, which wraps from `i32::MIN` to
    /// `i32::MAX`, as the C builtin does.
    ///
    /// Replaces `fn::git_atomic32_dec`
    #[doc(alias = "git_atomic32_dec")]
    #[inline]
    #[expect(
        clippy::must_use_candidate,
        reason = "C callers discard what it returns"
    )]
    pub fn dec(&self) -> i32 {
        self.val().fetch_sub(1, SeqCst).wrapping_sub(1)
    }

    /// Adds `addend`, atomically, and returns the value it leaves, wrapping on overflow, as the
    /// C builtin does.
    ///
    /// Replaces `fn::git_atomic32_add`
    #[doc(alias = "git_atomic32_add")]
    #[inline]
    #[expect(
        clippy::must_use_candidate,
        reason = "C callers discard what it returns"
    )]
    pub fn add(&self, addend: i32) -> i32 {
        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)
    }
}

/// `git_atomic64`'s operations, defined where pointers are 64 bits wide (`GIT_ARCH_64`), and
/// `git_atomic_ssize`'s there, its `git_atomic_ssize_get` and `git_atomic_ssize_add` naming
/// `git_atomic64_get` and `git_atomic64_add`. Where pointers are narrower, [`AtomicSsize`] is
/// [`Atomic32`] and the two macros name [`Atomic32Ref`]'s `get` and `add`.
#[cfg(target_pointer_width = "64")]
impl Atomic64Ref<'_> {
    /// The value, read atomically.
    ///
    /// Replaces `fn::git_atomic64_get`
    #[doc(alias = "git_atomic64_get")]
    /// Replaces `mac::git_atomic_ssize_get`
    #[doc(alias = "git_atomic_ssize_get")]
    #[inline]
    #[must_use]
    pub fn get(&self) -> i64 {
        self.val().load(SeqCst)
    }

    /// Stores `val`, atomically.
    ///
    /// Replaces `fn::git_atomic64_set`
    #[doc(alias = "git_atomic64_set")]
    #[inline]
    pub fn set(&self, val: i64) {
        self.val().store(val, SeqCst);
    }

    /// Adds `addend`, atomically, and returns the value it leaves, wrapping on overflow, as the
    /// C builtin does.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::src::util::thread::AtomicSsize;
    ///
    /// let storage = AtomicSsize::new(0);
    /// assert_eq!(storage.as_ref().add(4096), 4096);
    /// assert_eq!(storage.as_ref().add(-1024), 3072);
    /// assert_eq!(storage.as_ref().get(), 3072);
    /// ```
    ///
    /// Replaces `fn::git_atomic64_add`
    #[doc(alias = "git_atomic64_add")]
    /// Replaces `mac::git_atomic_ssize_add`
    #[doc(alias = "git_atomic_ssize_add")]
    #[inline]
    #[expect(
        clippy::must_use_candidate,
        reason = "C callers discard what it returns"
    )]
    pub fn add(&self, addend: i64) -> i64 {
        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)
    }
}

mod word {
    /// The operations [`AtomicWord`](super::AtomicWord) is implemented by, out of reach of
    /// other crates, so that no other type implements it.
    pub trait Sealed {
        /// The value the slot holds: a pointer, or an integer of a pointer's size.
        type Value: Copy;

        fn compare_and_swap(&self, old: Self::Value, new: Self::Value) -> Self::Value;

        fn swap(&self, new: Self::Value) -> Self::Value;

        fn load(&self) -> Self::Value;
    }
}

/// A standard atomic of a pointer's size, the slot `thread.h`'s pointer operations act on:
/// `git_atomic_compare_and_swap`, `git_atomic_swap` and `git_atomic_load` take "a pointer or a
/// value that is the same size as a pointer", and C passes them pointer fields and `intptr_t`
/// ones. An [`AtomicPtr<T>`] holds a pointer, an [`AtomicIsize`] or [`AtomicUsize`] an integer;
/// a slot C shares is reached as one of these through the accessor of what holds it. Sealed:
/// no other type implements it.
pub trait AtomicWord: word::Sealed + Sync {}

macro_rules! atomic_word {
    ($(#[$attr:meta])* [$($generic:tt)*] $atomic:ty, $value:ty) => {
        impl<$($generic)*> word::Sealed for $atomic {
            type Value = $value;

            #[inline]
            fn compare_and_swap(&self, old: $value, new: $value) -> $value {
                // Strong, as `__atomic_compare_exchange(…, false, …)` is: it fails only when
                // the slot holds another value, which it returns.
                match self.compare_exchange(old, new, SeqCst, SeqCst) {
                    Ok(found) | Err(found) => found,
                }
            }

            #[inline]
            fn swap(&self, new: $value) -> $value {
                <$atomic>::swap(self, new, SeqCst)
            }

            #[inline]
            fn load(&self) -> $value {
                <$atomic>::load(self, SeqCst)
            }
        }

        $(#[$attr])*
        impl<$($generic)*> AtomicWord for $atomic {}
    };
}

atomic_word!(
    /// A pointer slot, as C's `void *` fields are.
    [T] AtomicPtr<T>, *mut T
);
atomic_word!(
    /// An `intptr_t` slot, as `git_repository`'s configuration cache is.
    [] AtomicIsize, isize
);
atomic_word!(
    /// A `uintptr_t` slot.
    [] AtomicUsize, usize
);

/// Stores `new` in `slot` if it holds `old`, atomically, and returns what it held, which is
/// `old` exactly when it stored `new`, as `git_atomic_compare_and_swap` does: a strong,
/// sequentially consistent compare-and-swap, which fails only when the slot holds another
/// value.
///
/// ```
/// use core::ptr;
/// use core::sync::atomic::AtomicPtr;
///
/// use libgit2_crustify::libgit2::src::util::thread::atomic_compare_and_swap;
///
/// let mut cache = 7;
/// let slot = AtomicPtr::new(ptr::null_mut());
/// // The first to install a value wins; a later one finds it.
/// assert!(atomic_compare_and_swap(&slot, ptr::null_mut(), &raw mut cache).is_null());
/// assert_eq!(
///     atomic_compare_and_swap(&slot, ptr::null_mut(), ptr::null_mut()),
///     &raw mut cache
/// );
/// ```
///
/// Replaces `fn::git_atomic__compare_and_swap`
#[doc(alias = "git_atomic__compare_and_swap")]
/// Replaces `mac::git_atomic_compare_and_swap`
#[doc(alias = "git_atomic_compare_and_swap")]
#[inline]
pub fn atomic_compare_and_swap<A: AtomicWord>(slot: &A, old: A::Value, new: A::Value) -> A::Value {
    slot.compare_and_swap(old, new)
}

/// Stores `new` in `slot`, atomically, and returns what it held, as `git_atomic_swap` does.
///
/// Replaces `fn::git_atomic__swap`
#[doc(alias = "git_atomic__swap")]
/// Replaces `mac::git_atomic_swap`
#[doc(alias = "git_atomic_swap")]
#[inline]
pub fn atomic_swap<A: AtomicWord>(slot: &A, new: A::Value) -> A::Value {
    slot.swap(new)
}

/// What `slot` holds, read atomically, as `git_atomic_load` reads it.
///
/// Replaces `fn::git_atomic__load`
#[doc(alias = "git_atomic__load")]
/// Replaces `mac::git_atomic_load`
#[doc(alias = "git_atomic_load")]
#[inline]
#[must_use]
pub fn atomic_load<A: AtomicWord>(slot: &A) -> A::Value {
    slot.load()
}

// ---------------------------------------------------------------------------------------------
// End of the atomic operations.
// ---------------------------------------------------------------------------------------------

// -----------------------------------------------------------------------------------------
// Locks, condition variables and threads: `unix/pthread.h`'s `git_mutex`, `git_cond`,
// `git_rwlock` and `git_thread_*` macros, native over the standard library.
// -----------------------------------------------------------------------------------------

use core::fmt;
use core::ops::{Deref, DerefMut};
use std::io;
use std::sync::{LockResult, PoisonError};

/// A mutual-exclusion lock over a `T`: libgit2's `git_mutex`, `pthread_mutex_t` with
/// default attributes, as a [`std::sync::Mutex`].
///
/// [`new`](Self::new) initialises it, as `git_mutex_init` does, [`lock`](Self::lock)
/// takes it into a [`MutexGuard`], as `git_mutex_lock` does, the guard's drop or
/// [`MutexGuard::unlock`] releases it, as `git_mutex_unlock` does, and dropping it
/// destroys it, as `git_mutex_free` does: the borrow checker sees that no guard holds it,
/// which POSIX asks of `pthread_mutex_destroy`'s caller. The [module documentation](self#locks)
/// gives how it differs from C's.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::Mutex;
///
/// let count = Mutex::new(0);
/// *count.lock().unwrap() += 1;
/// assert_eq!(*count.lock().unwrap(), 1);
/// assert_eq!(count.into_inner().unwrap(), 1);
/// ```
///
/// Replaces `mac::git_mutex`
#[doc(alias = "git_mutex")]
/// Replaces `mac::git_mutex_free`
#[doc(alias = "git_mutex_free")]
pub struct Mutex<T: ?Sized> {
    inner: std::sync::Mutex<T>,
}

/// The lock of a [`Mutex`], held until it is dropped or passed to
/// [`unlock`](Self::unlock); it dereferences to the data the lock guards. It is not
/// [`Send`]: the thread that took the lock releases it.
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    inner: std::sync::MutexGuard<'a, T>,
}

impl<T> Mutex<T> {
    /// A new, unlocked lock over `value`, as `git_mutex_init` initialises one with
    /// default attributes; it cannot fail, where `pthread_mutex_init` may.
    ///
    /// Replaces `mac::git_mutex_init`
    #[doc(alias = "git_mutex_init")]
    #[must_use]
    pub const fn new(value: T) -> Mutex<T> {
        Mutex {
            inner: std::sync::Mutex::new(value),
        }
    }

    /// The data, taking the lock apart, which needs no locking.
    ///
    /// # Errors
    ///
    /// When the lock is poisoned, the [`PoisonError`] holds the data.
    pub fn into_inner(self) -> LockResult<T> {
        self.inner.into_inner()
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Takes the lock, blocking until no other thread holds it, as `git_mutex_lock`
    /// does.
    ///
    /// Taking it again on a thread that holds it deadlocks or panics, as it deadlocks
    /// in C.
    ///
    /// # Errors
    ///
    /// When a thread panicked while holding the lock, the lock is poisoned: it is still
    /// taken, and the [`PoisonError`] holds the guard
    /// ([`into_inner`](PoisonError::into_inner)).
    ///
    /// Replaces `mac::git_mutex_lock`
    #[doc(alias = "git_mutex_lock")]
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        match self.inner.lock() {
            Ok(inner) => Ok(MutexGuard { inner }),
            Err(poisoned) => Err(PoisonError::new(MutexGuard {
                inner: poisoned.into_inner(),
            })),
        }
    }

    /// Whether a thread panicked while holding the lock.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// The data, through the exclusive borrow of the lock, which needs no locking.
    ///
    /// # Errors
    ///
    /// When the lock is poisoned, the [`PoisonError`] holds the data.
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        self.inner.get_mut()
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Mutex<T> {
        Mutex::new(T::default())
    }
}

impl<T> From<T> for Mutex<T> {
    fn from(value: T) -> Mutex<T> {
        Mutex::new(value)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl<T: ?Sized> MutexGuard<'_, T> {
    /// Releases the lock, as `git_mutex_unlock` does; dropping the guard does the same.
    /// An associated function, `MutexGuard::unlock(guard)`, so that it hides no method
    /// of the data.
    ///
    /// Replaces `mac::git_mutex_unlock`
    #[doc(alias = "git_mutex_unlock")]
    pub fn unlock(guard: Self) {
        drop(guard);
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.inner, f)
    }
}

/// A condition variable: libgit2's `git_cond`, `pthread_cond_t` with default
/// attributes, as a [`std::sync::Condvar`].
///
/// [`new`](Self::new) initialises it, as `git_cond_init` does,
/// [`wait`](Self::wait) releases a [`Mutex`]'s lock and waits on it, as `git_cond_wait`
/// does, [`notify_one`](Self::notify_one) wakes one waiter, as `git_cond_signal` does,
/// and dropping it destroys it, as `git_cond_free` does: a waiter borrows it, so none is
/// left, which POSIX asks of `pthread_cond_destroy`'s caller. A wait may also end with no
/// notification, as `pthread_cond_wait`'s may, so a waiter tests its condition in a
/// loop, as libgit2's pack builder does.
///
/// ```
/// use std::sync::Arc;
///
/// use libgit2_crustify::libgit2::src::util::thread::{Condvar, Mutex, spawn};
///
/// let shared = Arc::new((Mutex::new(false), Condvar::new()));
/// let theirs = Arc::clone(&shared);
/// let thread = spawn(move || {
///     *theirs.0.lock().unwrap() = true;
///     theirs.1.notify_one();
/// })
/// .unwrap();
/// let mut ready = shared.0.lock().unwrap();
/// while !*ready {
///     ready = shared.1.wait(ready).unwrap();
/// }
/// drop(ready);
/// thread.join().unwrap();
/// ```
///
/// Replaces `mac::git_cond`
#[doc(alias = "git_cond")]
/// Replaces `mac::git_cond_free`
#[doc(alias = "git_cond_free")]
pub struct Condvar {
    inner: std::sync::Condvar,
}

impl Condvar {
    /// A new condition variable, which no thread waits on, as `git_cond_init`
    /// initialises one with default attributes; it cannot fail, where
    /// `pthread_cond_init` may.
    ///
    /// Replaces `mac::git_cond_init`
    #[doc(alias = "git_cond_init")]
    #[must_use]
    pub const fn new() -> Condvar {
        Condvar {
            inner: std::sync::Condvar::new(),
        }
    }

    /// Releases `guard`'s lock and blocks until this condition variable is notified,
    /// then takes the lock again and returns its guard, as `git_cond_wait` does. The
    /// wait may end with no notification, so the caller tests its condition again.
    ///
    /// # Errors
    ///
    /// When the lock is poisoned once taken again, the [`PoisonError`] holds its guard.
    ///
    /// Replaces `mac::git_cond_wait`
    #[doc(alias = "git_cond_wait")]
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        match self.inner.wait(guard.inner) {
            Ok(inner) => Ok(MutexGuard { inner }),
            Err(poisoned) => Err(PoisonError::new(MutexGuard {
                inner: poisoned.into_inner(),
            })),
        }
    }

    /// Wakes one thread waiting on this condition variable, if any waits, as
    /// `git_cond_signal` does; a notification no thread waits for is lost, as C's is.
    ///
    /// Replaces `mac::git_cond_signal`
    #[doc(alias = "git_cond_signal")]
    pub fn notify_one(&self) {
        self.inner.notify_one();
    }
}

impl Default for Condvar {
    fn default() -> Condvar {
        Condvar::new()
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

/// A read-write lock over a `T`: libgit2's `git_rwlock`, `pthread_rwlock_t` with
/// default attributes, as a [`std::sync::RwLock`].
///
/// [`new`](Self::new) initialises it, as `git_rwlock_init` does, [`read`](Self::read)
/// and [`write`](Self::write) take it shared and exclusive, as `git_rwlock_rdlock` and
/// `git_rwlock_wrlock` do, into a [`RwLockReadGuard`] or a [`RwLockWriteGuard`], whose
/// drop or `unlock` releases it, as `git_rwlock_rdunlock` and `git_rwlock_wrunlock` do,
/// and dropping it destroys it, as `git_rwlock_free` does: the borrow checker sees that no
/// guard holds it, which POSIX asks of `pthread_rwlock_destroy`'s caller. A read lock is
/// not taken twice by one thread, as `unix/pthread.h` asks.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::RwLock;
///
/// let cache = RwLock::new(Vec::new());
/// cache.write().unwrap().push(1);
/// assert_eq!(*cache.read().unwrap(), [1]);
/// assert_eq!(cache.into_inner().unwrap(), [1]);
/// ```
///
/// Replaces `mac::git_rwlock`
#[doc(alias = "git_rwlock")]
/// Replaces `mac::git_rwlock_free`
#[doc(alias = "git_rwlock_free")]
pub struct RwLock<T: ?Sized> {
    inner: std::sync::RwLock<T>,
}

/// The shared lock of an [`RwLock`], held until it is dropped or passed to
/// [`unlock`](Self::unlock); it dereferences to the data. It is not [`Send`].
pub struct RwLockReadGuard<'a, T: ?Sized + 'a> {
    inner: std::sync::RwLockReadGuard<'a, T>,
}

/// The exclusive lock of an [`RwLock`], held until it is dropped or passed to
/// [`unlock`](Self::unlock); it dereferences to the data, mutably. It is not [`Send`].
pub struct RwLockWriteGuard<'a, T: ?Sized + 'a> {
    inner: std::sync::RwLockWriteGuard<'a, T>,
}

impl<T> RwLock<T> {
    /// A new, unlocked lock over `value`, as `git_rwlock_init` initialises one with
    /// default attributes; it cannot fail, where `pthread_rwlock_init` may.
    ///
    /// Replaces `mac::git_rwlock_init`
    #[doc(alias = "git_rwlock_init")]
    #[must_use]
    pub const fn new(value: T) -> RwLock<T> {
        RwLock {
            inner: std::sync::RwLock::new(value),
        }
    }

    /// The data, taking the lock apart, which needs no locking.
    ///
    /// # Errors
    ///
    /// When the lock is poisoned, the [`PoisonError`] holds the data.
    pub fn into_inner(self) -> LockResult<T> {
        self.inner.into_inner()
    }
}

impl<T: ?Sized> RwLock<T> {
    /// Takes the lock shared, blocking while a thread holds it exclusive, as
    /// `git_rwlock_rdlock` does; any number of threads hold it shared at once.
    ///
    /// Taking it again on a thread that holds it, shared or exclusive, may deadlock or
    /// panic, as `unix/pthread.h` forbids.
    ///
    /// # Errors
    ///
    /// When a thread panicked while holding the lock exclusive, the lock is poisoned:
    /// it is still taken, and the [`PoisonError`] holds the guard.
    ///
    /// Replaces `mac::git_rwlock_rdlock`
    #[doc(alias = "git_rwlock_rdlock")]
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        match self.inner.read() {
            Ok(inner) => Ok(RwLockReadGuard { inner }),
            Err(poisoned) => Err(PoisonError::new(RwLockReadGuard {
                inner: poisoned.into_inner(),
            })),
        }
    }

    /// Takes the lock exclusive, blocking while any thread holds it, as
    /// `git_rwlock_wrlock` does.
    ///
    /// Taking it again on a thread that holds it may deadlock or panic.
    ///
    /// # Errors
    ///
    /// When a thread panicked while holding the lock exclusive, the lock is poisoned:
    /// it is still taken, and the [`PoisonError`] holds the guard.
    ///
    /// Replaces `mac::git_rwlock_wrlock`
    #[doc(alias = "git_rwlock_wrlock")]
    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, T>> {
        match self.inner.write() {
            Ok(inner) => Ok(RwLockWriteGuard { inner }),
            Err(poisoned) => Err(PoisonError::new(RwLockWriteGuard {
                inner: poisoned.into_inner(),
            })),
        }
    }

    /// Whether a thread panicked while holding the lock exclusive.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    /// The data, through the exclusive borrow of the lock, which needs no locking.
    ///
    /// # Errors
    ///
    /// When the lock is poisoned, the [`PoisonError`] holds the data.
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        self.inner.get_mut()
    }
}

impl<T: Default> Default for RwLock<T> {
    fn default() -> RwLock<T> {
        RwLock::new(T::default())
    }
}

impl<T> From<T> for RwLock<T> {
    fn from(value: T) -> RwLock<T> {
        RwLock::new(value)
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl<T: ?Sized> RwLockReadGuard<'_, T> {
    /// Releases the shared lock, as `git_rwlock_rdunlock` does; dropping the guard does
    /// the same. An associated function, so that it hides no method of the data.
    ///
    /// Replaces `mac::git_rwlock_rdunlock`
    #[doc(alias = "git_rwlock_rdunlock")]
    pub fn unlock(guard: Self) {
        drop(guard);
    }
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.inner, f)
    }
}

impl<T: ?Sized> RwLockWriteGuard<'_, T> {
    /// Releases the exclusive lock, as `git_rwlock_wrunlock` does; dropping the guard
    /// does the same. An associated function, so that it hides no method of the data.
    ///
    /// Replaces `mac::git_rwlock_wrunlock`
    #[doc(alias = "git_rwlock_wrunlock")]
    pub fn unlock(guard: Self) {
        drop(guard);
    }
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwLockWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.inner, f)
    }
}

/// Starts a thread that runs `f`, as `git_thread_create` starts one that runs its start
/// routine over its argument, and returns the [`JoinHandle`] that owns it.
///
/// The closure owns what it captures (`'static`), where C hands its thread a pointer
/// that the caller keeps alive until the join; a thread that borrows its caller's data
/// is started in a [`std::thread::scope`]. A thread nothing joins runs on after its
/// handle is dropped, detached, where C's runs on unjoined.
///
/// # Errors
///
/// The system's error when it cannot start a thread, as `pthread_create` reports it
/// (`EAGAIN` when a resource limit is reached), which libgit2 reports as
/// `GIT_ERROR_THREAD`.
///
/// Replaces `mac::git_thread_create`
#[doc(alias = "git_thread_create")]
pub fn spawn<F, T>(f: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .spawn(f)
        .map(|inner| JoinHandle { inner })
}

/// A thread [`spawn`] started, which [`join`](Self::join) waits for and whose result
/// it returns.
pub struct JoinHandle<T> {
    inner: std::thread::JoinHandle<T>,
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to end and returns what its closure returned, as
    /// `git_thread_join` waits and reads the start routine's result through `status`.
    /// The handle is consumed, so the thread is joined once.
    ///
    /// # Errors
    ///
    /// When the closure panicked, the panic's payload, which
    /// [`std::panic::resume_unwind`] can propagate; C's thread has no such ending.
    ///
    /// # Panics
    ///
    /// When the thread joins itself, through a handle it was given: `pthread_join`
    /// refuses that with `EDEADLK`.
    ///
    /// Replaces `mac::git_thread_join`
    #[doc(alias = "git_thread_join")]
    pub fn join(self) -> std::thread::Result<T> {
        self.inner.join()
    }

    /// The thread, as the standard library names it.
    #[must_use]
    pub fn thread(&self) -> &std::thread::Thread {
        self.inner.thread()
    }

    /// Whether the thread's closure has returned or panicked, so that
    /// [`join`](Self::join) would not block.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle").finish_non_exhaustive()
    }
}

#[expect(
    unsafe_code,
    reason = "lays out git_atomic32 and git_atomic64 by hand, with their handles, lends their \
              values as standard atomics, and reads and writes git_thread's field through its \
              handles' pointers; keeps each thread's table of thread-local values \
              (`thread_local!`) and calls the destructors C gives its keys"
)]
mod ffi_import {
    use core::cell::{RefCell, UnsafeCell};
    use core::ffi::c_void;
    use core::fmt;
    use core::marker::PhantomData;
    use core::ptr::NonNull;
    use core::sync::atomic::AtomicI32;
    #[cfg(target_pointer_width = "64")]
    use core::sync::atomic::AtomicI64;
    use std::os::unix::thread::RawPthread;
    use std::thread::LocalKey;

    use ffibox::{CBorrowedPtr, CCell, CMutSeam, CRefSeam, CZeroable, define_ctype};

    use crate::sys::libgit2 as sys;

    /// Declares the layout type of one of `thread.h`'s atomic integers, with its handles and
    /// the accessor that lends its value as `$atomic`.
    ///
    /// `define_ctype!` lays a type out as its C struct alone, so that a handle built from a
    /// value Rust holds may only be read through; the value here is written through shared
    /// access, so the layout type is written by hand against [`CCell`]'s contract, over an
    /// `UnsafeCell` of the C struct.
    macro_rules! atomic_type {
        (
            $(#[$attr:meta])*
            $name:ident, $rf:ident, $mt:ident, $c_name:ident, $atomic:ty, $int:ty,
            val: $(#[$val_attr:meta])*
        ) => {
            $(#[$attr])*
            #[repr(transparent)]
            pub struct $name(UnsafeCell<sys::$c_name>);

            #[doc = concat!(
                "Shared borrow of an [`", stringify!($name), "`]. ",
                "`Copy`, like `&T`; lends the value."
            )]
            #[repr(transparent)]
            #[derive(Clone, Copy)]
            pub struct $rf<'a>(CBorrowedPtr<'a, $name>);

            #[doc = concat!(
                "Exclusive borrow of an [`", stringify!($name), "`]. ",
                "Reaches the value through [`as_ref`](Self::as_ref)."
            )]
            #[repr(transparent)]
            pub struct $mt<'a>(
                $rf<'a>,
                // Invariant in the pointee, as `&'a mut` is; see `CCell`'s contract.
                PhantomData<&'a mut $name>,
            );

            // The layout the handles' pointer casts and the lent atomic rely on.
            const _: () = {
                assert!(size_of::<$name>() == size_of::<sys::$c_name>());
                assert!(align_of::<$name>() == align_of::<sys::$c_name>());
                assert!(size_of::<$atomic>() == size_of::<$int>());
                assert!(align_of::<sys::$c_name>() >= align_of::<$atomic>());
            };

            // SAFETY: `$name` is `#[repr(transparent)]` over `UnsafeCell<sys::$c_name>`, which
            // has `sys::$c_name`'s layout; the handles are `#[repr(transparent)]` over
            // `CBorrowedPtr<'a, $name>` (`$mt` through `$rf`) with zero-sized markers and no
            // `Drop`, form no reference to `$name`, and `$mt` is invariant in `$name` through
            // its marker. `$rf` writes only through the atomic it lends, which is what
            // `$name`'s handles admit beside them (the module documentation).
            unsafe impl CCell for $name {
                type C = sys::$c_name;
                type Ref<'a>
                    = $rf<'a>
                where
                    Self: 'a;
                type Mut<'a>
                    = $mt<'a>
                where
                    Self: 'a;
            }

            impl<'a> CRefSeam<'a> for $rf<'a> {
                type Cell = $name;
            }

            impl<'a> CMutSeam<'a> for $mt<'a> {
                type Cell = $name;
            }

            // SAFETY: a value is one integer behind an `UnsafeCell`: no pointer, no thread
            // affinity, nothing to release, so moving one to another thread moves its bytes.
            unsafe impl Send for $name {}

            // SAFETY: a shared `$name` is reached only through `$rf`, whose one access is the
            // atomic `val` lends, so threads that share it race nothing, as `$atomic` is
            // `Sync`.
            unsafe impl Sync for $name {}

            // SAFETY: as `$name`'s `Sync`: `$rf` is a shared borrow, as `&$atomic` is, which is
            // `Send` and `Sync`.
            unsafe impl Send for $rf<'_> {}

            // SAFETY: as above.
            unsafe impl Sync for $rf<'_> {}

            // SAFETY: `$mt` is an exclusive borrow, as `&mut $atomic` is, which is `Send`.
            unsafe impl Send for $mt<'_> {}

            // SAFETY: `&$mt` reaches only `as_ref`, the shared handle, which is `Sync`.
            unsafe impl Sync for $mt<'_> {}

            impl $name {
                /// A value holding `val`, as C initialises one (`{0}`, or zeroed storage).
                #[must_use]
                pub const fn new(val: $int) -> $name {
                    $name(UnsafeCell::new(sys::$c_name { val }))
                }

                /// Shared handle to this value.
                ///
                /// A reference covers this value's bytes, which is sound because a value is
                /// Rust-owned storage, behind an `UnsafeCell`: nothing hands out a reference
                /// over a C object.
                #[must_use]
                pub fn as_ref(&self) -> $rf<'_> {
                    // SAFETY: `self` is a live, initialised value borrowed for the handle's
                    // lifetime; the pointer comes from a shared borrow of an `UnsafeCell`, so
                    // the atomic writes `val` lends may go through it.
                    $rf(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
                }

                /// Exclusive handle to this value.
                #[must_use]
                pub fn as_mut(&mut self) -> $mt<'_> {
                    // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries
                    // write provenance and no other handle exists.
                    $mt($rf(unsafe { CBorrowedPtr::new(NonNull::from(self)) }), PhantomData)
                }
            }

            /// The value 0, as C's zeroed storage holds it.
            impl Default for $name {
                fn default() -> $name {
                    $name::new(0)
                }
            }

            impl<'a> $rf<'a> {
                $(#[$val_attr])*
                #[must_use]
                pub fn val(&self) -> &'a $atomic {
                    let cell = self.0.as_non_null().as_ptr();
                    // SAFETY: the handle addresses a live `$name` for `'a` (its invariant), and
                    // `$name` is transparent over `UnsafeCell<sys::$c_name>`, so the cast
                    // addresses its `sys::$c_name`; the projection forms no reference and reads
                    // nothing.
                    let val = unsafe { &raw mut (*cell.cast::<sys::$c_name>()).val };
                    // SAFETY: `val` is live for `'a`, as the object is, and aligned for
                    // `$atomic` (asserted above, the field at offset 0). Every write it may
                    // race with is atomic, of the same size: the handle's other holders reach
                    // the value only through this, and C writes it only through `thread.h`'s
                    // atomic operations. Every Rust read is atomic too. C's few plain reads of
                    // a shared value (the module documentation) race with any write made on
                    // another thread, C's own atomic ones as much as one made through this: a
                    // race of the C's, which lending the atomic neither adds nor can remove.
                    // The pointer may be written through: it comes from C, or from a borrow of
                    // a Rust value's `UnsafeCell`.
                    unsafe { <$atomic>::from_ptr(val) }
                }
            }

            /// The integer, as the standard atomic's `Debug` prints it.
            impl fmt::Debug for $rf<'_> {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Debug::fmt(self.val(), f)
                }
            }

            /// The integer, as the shared handle prints it.
            impl fmt::Debug for $mt<'_> {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Debug::fmt(&self.as_ref(), f)
                }
            }

            /// The integer, as the shared handle prints it.
            impl fmt::Debug for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Debug::fmt(&self.as_ref(), f)
                }
            }

            /// A value holding `val`, as [`new`](Self::new) builds it.
            impl From<$int> for $name {
                fn from(val: $int) -> $name {
                    $name::new(val)
                }
            }

            impl $mt<'_> {
                /// Reborrow shared, for passing where only the value is wanted.
                #[must_use]
                pub fn as_ref(&self) -> $rf<'_> {
                    self.0
                }

                /// Reborrow exclusively, for passing to a function that takes the handle by
                /// value while keeping this one. This handle is frozen while the result lives.
                #[must_use]
                pub fn as_mut(&mut self) -> $mt<'_> {
                    $mt(self.0, PhantomData)
                }
            }
        };
    }

    atomic_type!(
        /// A 32-bit integer libgit2's threads share and change atomically (`git_atomic32`),
        /// as reference counts, flags and counters: the module documentation describes where C
        /// keeps one.
        ///
        /// A value Rust holds, built with [`Atomic32::new`], is reached through
        /// [`as_ref`](Self::as_ref) and [`as_mut`](Self::as_mut), and may be a `static`. The
        /// handles lend the integer as an [`AtomicI32`] through
        /// [`val`](Atomic32Ref::val).
        ///
        /// A handle cannot outlive its value:
        ///
        /// ```compile_fail,E0597
        /// use libgit2_crustify::libgit2::src::util::thread::Atomic32;
        ///
        /// let handle = {
        ///     let count = Atomic32::new(1);
        ///     count.as_ref()
        /// };
        /// let _ = handle.val();
        /// ```
        ///
        /// Wraps `st::git_atomic32`
        #[doc(alias = "git_atomic32")]
        Atomic32,
        Atomic32Ref,
        Atomic32Mut,
        git_atomic32,
        AtomicI32,
        i32,
        val:
        /// The integer, as the atomic it is, borrowed for as long as the handle's object: C
        /// writes it with sequentially consistent operations
        /// ([`SeqCst`](core::sync::atomic::Ordering::SeqCst)) alone, and reads it with them
        /// but for the plain reads the module documentation names.
        ///
        /// ```
        /// use core::sync::atomic::Ordering::SeqCst;
        ///
        /// use libgit2_crustify::libgit2::src::util::thread::Atomic32;
        ///
        /// let count = Atomic32::new(1);
        /// assert_eq!(count.as_ref().val().fetch_add(1, SeqCst), 1);
        /// assert_eq!(count.as_ref().val().load(SeqCst), 2);
        /// ```
        ///
        /// Wraps `fld::git_atomic32.val`
        #[doc(alias = "git_atomic32.val")]
    );

    #[cfg(target_pointer_width = "64")]
    atomic_type!(
        /// A 64-bit integer libgit2's threads share and change atomically (`git_atomic64`),
        /// defined where pointers are 64 bits wide (`GIT_ARCH_64`): as
        /// [`AtomicSsize`](super::AtomicSsize), the object cache's total size.
        ///
        /// A value Rust holds, built with [`Atomic64::new`], is reached through
        /// [`as_ref`](Self::as_ref) and [`as_mut`](Self::as_mut), and may be a `static`. The
        /// handles lend the integer as an [`AtomicI64`] through
        /// [`val`](Atomic64Ref::val).
        ///
        /// Wraps `st::git_atomic64`
        #[doc(alias = "git_atomic64")]
        Atomic64,
        Atomic64Ref,
        Atomic64Mut,
        git_atomic64,
        AtomicI64,
        i64,
        val:
        /// The integer, as the atomic it is, borrowed for as long as the handle's object: C
        /// writes it with sequentially consistent operations
        /// ([`SeqCst`](core::sync::atomic::Ordering::SeqCst)) alone, and reads it with them
        /// but for the plain reads the module documentation names.
        ///
        /// ```
        /// use core::sync::atomic::Ordering::SeqCst;
        ///
        /// use libgit2_crustify::libgit2::src::util::thread::Atomic64;
        ///
        /// let size = Atomic64::new(i64::MAX - 1);
        /// size.as_ref().val().fetch_add(1, SeqCst);
        /// assert_eq!(size.as_ref().val().load(SeqCst), i64::MAX);
        /// ```
        ///
        /// Wraps `fld::git_atomic64.val`
        #[doc(alias = "git_atomic64.val")]
    );

    // `git_atomic_ssize` is `git_atomic64` in the bindings, as `AtomicSsize` is `Atomic64`.
    #[cfg(target_pointer_width = "64")]
    const _: fn(sys::git_atomic_ssize) -> sys::git_atomic64 = |ssize| ssize;

    define_ctype!(
        /// A thread libgit2 starts and joins (`git_thread`): the `pthread_t` that
        /// `git_thread_create` stores and `git_thread_join` joins.
        ///
        /// It owns nothing it releases, and C keeps no pointer to it, so it is held by value,
        /// as C holds it in the storage it hands each thread; [`Thread::default`] and
        /// [`Thread::zeroed`](ffibox::CZeroable::zeroed) give the zeroed value C starts from,
        /// which names no thread. It is neither `Clone` nor `Copy`, since the id it stores
        /// joins once, nor `Send` or `Sync`.
        ///
        /// Two exclusive handles to one value cannot coexist:
        ///
        /// ```compile_fail,E0499
        /// use libgit2_crustify::libgit2::src::util::thread::Thread;
        ///
        /// let mut thread = Thread::default();
        /// let a = thread.as_mut();
        /// let b = thread.as_mut();
        /// let _ = (a, b);
        /// ```
        ///
        /// Wraps `st::git_thread`
        #[doc(alias = "git_thread")]
        Thread,
        ThreadRef,
        ThreadMut,
        sys::git_thread
    );

    // The bindings' `pthread_t` is the standard library's: the accessors convert nothing.
    const _: fn(sys::pthread_t) -> RawPthread = |thread| thread;

    // SAFETY: `git_thread` is one integer, the `pthread_t`, so all-zero is the id 0, which the
    // getter reads as any other value and which names no thread, as C's zeroed storage holds it
    // before `git_thread_create` (`ll_find_deltas`'s `git__calloc`). No teardown is paired with
    // the value, and C keeps no pointer to it (`pthread_create` writes the id through it during
    // the call alone), so a value moves freely.
    unsafe impl CZeroable for Thread {}

    /// The zeroed value, which names no thread, as [`zeroed`](ffibox::CZeroable::zeroed) gives
    /// it.
    impl Default for Thread {
        fn default() -> Thread {
            Thread::zeroed()
        }
    }

    /// The stored id, as `Thread { thread: <id> }`.
    impl fmt::Debug for ThreadRef<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Thread")
                .field("thread", &self.thread())
                .finish()
        }
    }

    /// As the shared handle prints it.
    impl fmt::Debug for ThreadMut<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(&self.as_ref(), f)
        }
    }

    /// As the shared handle prints it.
    impl fmt::Debug for Thread {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(&self.as_ref(), f)
        }
    }

    impl ThreadRef<'_> {
        /// The id of the thread stored in the object: the `pthread_t` `git_thread_create`
        /// stored, or whatever the storage held, 0 when zeroed, which names no thread. Reading
        /// it joins and detaches nothing.
        ///
        /// Wraps `fld::git_thread.thread`
        #[doc(alias = "git_thread.thread")]
        #[must_use]
        pub fn thread(&self) -> RawPthread {
            // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
            // projection forms no reference to it.
            let thread = unsafe { &raw const (*self.as_ptr()).thread };
            // SAFETY: a read through the shared handle's pointer to a live `git_thread`; any
            // value is a valid `pthread_t`, an integer.
            unsafe { thread.read() }
        }
    }

    impl ThreadMut<'_> {
        /// Stores `thread` as the id of the object's thread, as `git_thread_create` stores
        /// the one `pthread_create` starts, without joining or detaching the one it held.
        ///
        /// # Safety
        ///
        /// While the object holds `thread`, until a later `set_thread` replaces it or a join
        /// through the object ends it, any join or detach made through the object, by C's
        /// `git_thread_join` or by Rust, acts on `thread`. If one may be made, `thread` must
        /// name a joinable thread, neither detached nor joined, that nothing but the object
        /// joins or detaches for that time. A value nothing will join through needs nothing.
        ///
        /// Wraps `fld::git_thread.thread`
        #[doc(alias = "git_thread.thread")]
        pub unsafe fn set_thread(&mut self, thread: RawPthread) {
            // SAFETY: `self.as_mut_ptr()` addresses the live object this exclusive handle
            // borrows, with write provenance; the projection forms no reference to it.
            let slot = unsafe { &raw mut (*self.as_mut_ptr()).thread };
            // SAFETY: a write through the exclusive handle's pointer to a live `git_thread`,
            // which no other handle reaches while this one lives; the caller vouches for what
            // a join through the object makes of the id.
            unsafe { slot.write(thread) }
        }
    }

    // Thread-local storage (wave-226/batch-3): the C forms of the keys C makes, a destructor
    // and the values it destroys, which the module's native code holds and pairs by key.

    /// A destructor C gave `git_tlsdata_init` (`void (*)(void *)`), which Rust calls on each
    /// value C stored under its key, at the exit of the thread that stored it.
    ///
    /// It is called only through [`destroy`](Self::destroy), with a [`CWord`] C stored under
    /// the key it was registered for: the module's native code pairs the two by the key's id,
    /// and never hands either to safe code outside the module.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct CDestructor(unsafe extern "C" fn(*mut c_void));

    impl CDestructor {
        /// The destructor `f`, for the key `git_tlsdata_init` registers it with.
        ///
        /// # Safety
        ///
        /// `f` may be called once with each [`CWord`] built from a value C stores under that
        /// key, on the thread that stored it, at that thread's exit, while the key lives: what
        /// `thread.h` has `git_tlsdata_init`'s caller provide.
        #[cfg_attr(
            not(any(test, feature = "ffi-export-src-util-thread-c")),
            expect(
                dead_code,
                reason = "the export gateway alone calls it, under its feature"
            )
        )]
        pub(super) unsafe fn new(f: unsafe extern "C" fn(*mut c_void)) -> CDestructor {
            CDestructor(f)
        }

        /// Runs the destructor on `word`, a value C stored under the destructor's key, on the
        /// thread that stored it, at that thread's exit.
        pub(super) fn destroy(self, word: CWord) {
            // SAFETY: the native code calls this only from a thread's table as the thread
            // exits (`Table`'s `Drop`), once per entry, with the word the entry holds, which C
            // stored under the key this destructor was registered for (`CWord::new`), while
            // that key is live: what `CDestructor::new`'s caller vouched `f` accepts.
            unsafe { (self.0)(word.0.as_ptr()) }
        }
    }

    /// A non-null pointer C stored under one of its keys with `git_tlsdata_set`. Rust never
    /// reads through it: it hands it back to `git_tlsdata_get` and to the key's destructor.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct CWord(NonNull<c_void>);

    impl CWord {
        /// The value `ptr`, for the key C stores it under.
        ///
        /// # Safety
        ///
        /// `ptr` is stored under the key C passed `git_tlsdata_set` with it, and that key's
        /// destructor, if it has one, may be called with it at the storing thread's exit, as
        /// `git_tlsdata_set`'s caller provides.
        #[cfg_attr(
            not(any(test, feature = "ffi-export-src-util-thread-c")),
            expect(
                dead_code,
                reason = "the export gateway alone calls it, under its feature"
            )
        )]
        pub(super) unsafe fn new(ptr: NonNull<c_void>) -> CWord {
            CWord(ptr)
        }

        /// The pointer, as C stored it.
        #[cfg_attr(
            not(any(test, feature = "ffi-export-src-util-thread-c")),
            expect(
                dead_code,
                reason = "the export gateway alone calls it, under its feature"
            )
        )]
        pub(super) fn as_ptr(self) -> *mut c_void {
            self.0.as_ptr()
        }
    }

    std::thread_local! {
        /// Each thread's values of the thread-local keys, which [`Table`](super::tls::Table)'s
        /// drop destroys as the thread exits.
        static TLS_TABLE: RefCell<super::tls::Table> = const {
            RefCell::new(super::tls::Table::new())
        };
    }

    /// This thread's table of thread-local values, for the module's native code.
    pub(super) fn tls_table() -> &'static LocalKey<RefCell<super::tls::Table>> {
        &TLS_TABLE
    }

    #[cfg(test)]
    mod unit_priv {
        use core::ffi::c_void;
        use core::num::NonZeroUsize;
        use core::ptr::NonNull;
        use core::sync::atomic::{AtomicI32, Ordering};
        use std::sync::{Mutex, PoisonError};
        use std::thread;

        use nix::unistd::getpid;

        use super::super::{TlsError, TlsKey, c_keys};
        use super::{CDestructor, CWord};

        /// The addresses [`record`] was called with.
        static LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());

        /// A C destructor that records the address it is given, and reads nothing through it.
        unsafe extern "C" fn record(value: *mut c_void) {
            LOG.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(value.addr());
        }

        /// The test process's id, set before a test forks, by which a destructor tells a forked
        /// child apart.
        static PARENT: AtomicI32 = AtomicI32::new(0);

        /// Ends a forked child at once, with status 0, from a destructor it runs at `exit`, so
        /// that no atexit handler runs after the destructors: a sanitizer's leak check would
        /// find the memory of the parent's other threads, which the child lacks, unreachable.
        /// In the test process, it does nothing.
        fn end_if_child() {
            if getpid().as_raw() != PARENT.load(Ordering::Relaxed) {
                // SAFETY: `_exit` ends the process, running nothing more.
                unsafe { nix::libc::_exit(0) }
            }
        }

        /// A C destructor that ends a forked child, and takes no lock that a test beside its
        /// own may hold when the child is forked.
        unsafe extern "C" fn end_child(_: *mut c_void) {
            end_if_child();
        }

        /// The addresses recorded, sorted.
        fn recorded() -> Vec<usize> {
            let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner).clone();
            log.sort_unstable();
            log
        }

        /// A word C would store, at `addr`, which nothing reads through.
        fn word(addr: usize) -> CWord {
            let addr = NonZeroUsize::new(addr).expect("not NULL");
            // SAFETY: `record`, the only destructor these words reach, accepts any pointer.
            unsafe { CWord::new(NonNull::without_provenance(addr)) }
        }

        /// The address of the word stored under `id` on this thread.
        fn stored(id: u32) -> Option<usize> {
            c_keys::get(id).map(|word| word.as_ptr().addr())
        }

        /// C's keys store its words, give them back on the thread that stored them, and run
        /// C's destructor on each word a thread still holds of a live key at its exit: not on
        /// one it removed, nor once the key is disposed of.
        #[test]
        fn c_keys_store_words_and_destroy_them_at_thread_exit() {
            // SAFETY: `record` may be called with any pointer, on any thread.
            let destructor = unsafe { CDestructor::new(record) };
            let id = c_keys::init(Some(destructor)).expect("a key");
            assert_ne!(id, 0);
            assert_eq!(stored(id), None);
            let run = |f: fn(u32)| {
                thread::spawn(move || f(id))
                    .join()
                    .expect("the thread ends");
            };

            // Held at exit: destroyed.
            run(|id| {
                c_keys::set(id, Some(word(0x10))).expect("stored");
                assert_eq!(stored(id), Some(0x10));
            });
            assert_eq!(recorded(), [0x10]);

            // Replaced, then removed: nothing destroyed.
            run(|id| {
                c_keys::set(id, Some(word(0x20))).expect("stored");
                c_keys::set(id, Some(word(0x21))).expect("stored");
                assert_eq!(stored(id), Some(0x21));
                c_keys::set(id, None).expect("removed");
                assert_eq!(stored(id), None);
            });
            assert_eq!(recorded(), [0x10]);

            // Held when the key is disposed of: left to C, as POSIX leaves it.
            c_keys::set(id, Some(word(0x30))).expect("stored");
            run(|id| {
                c_keys::set(id, Some(word(0x31))).expect("stored");
                c_keys::dispose(id).expect("disposed");
                assert_eq!(stored(id), None);
            });
            assert_eq!(recorded(), [0x10]);
            assert_eq!(stored(id), None);
            assert_eq!(c_keys::set(id, Some(word(0x32))), Err(TlsError::Disposed));
            assert_eq!(c_keys::set(id, None), Err(TlsError::Disposed));
            assert_eq!(c_keys::dispose(id), Err(TlsError::Disposed));
        }

        /// A key without a destructor keeps C's words and destroys none.
        #[test]
        fn a_c_key_without_a_destructor_destroys_nothing() {
            let id = c_keys::init(None).expect("a key");
            thread::spawn(move || c_keys::set(id, Some(word(0x40))))
                .join()
                .expect("the thread ends")
                .expect("stored");
            assert!(!recorded().contains(&0x40));
            c_keys::dispose(id).expect("disposed");
        }

        /// C reaches none of Rust's keys, nor an id no key has, 0 included, which names no key:
        /// reading gives `NULL`, and storing or disposing fails, leaving Rust's values as they
        /// are.
        #[test]
        fn c_reaches_no_rust_key_and_no_unknown_id() {
            let rust = TlsKey::<u8>::new(None).expect("a key");
            rust.set(Some(1)).expect("stored");
            let rust_id = rust.id.get();
            for id in [0, rust_id, u32::MAX] {
                assert_eq!(stored(id), None);
                assert_eq!(c_keys::set(id, Some(word(0x50))), Err(TlsError::Disposed));
                assert_eq!(c_keys::set(id, None), Err(TlsError::Disposed));
                assert_eq!(c_keys::dispose(id), Err(TlsError::Disposed));
            }
            assert_eq!(rust.get(), Some(1));
            rust.dispose().expect("disposed");
        }

        /// C reaches none of Rust's keys on a thread that holds no value of it either, where a
        /// store would make the thread's first entry for the key: it fails, and leaves the key
        /// with no value on that thread.
        #[test]
        fn c_reaches_no_rust_key_from_a_thread_that_holds_none() {
            let rust = TlsKey::<u8>::new(None).expect("a key");
            let rust_id = rust.id.get();
            thread::spawn(move || {
                assert_eq!(
                    c_keys::set(rust_id, Some(word(0x58))),
                    Err(TlsError::Disposed)
                );
                assert_eq!(stored(rust_id), None);
                assert_eq!(rust.get(), None);
                rust.set(Some(2)).expect("stored");
                assert_eq!(rust.get(), Some(2));
            })
            .join()
            .expect("the thread ends");
            rust.dispose().expect("disposed");
        }

        /// A thread that calls `exit` destroys the values it holds of live keys, as the main
        /// thread does, where POSIX destroys none of them: a forked child, whose one thread
        /// holds a value of C's key it inherited, runs C's destructor at `exit`, which writes to
        /// a pipe whose descriptor is the value.
        #[test]
        #[cfg_attr(miri, ignore = "forks")]
        fn a_thread_that_calls_exit_destroys_its_values() {
            use std::os::fd::AsRawFd;

            use nix::sys::wait::{WaitStatus, waitpid};
            use nix::unistd::{ForkResult, fork, pipe, read};

            /// Writes one byte to the descriptor the value is, then ends a forked child.
            unsafe extern "C" fn write_byte(fd: *mut c_void) {
                let fd = i32::try_from(fd.addr()).expect("a descriptor");
                // SAFETY: a one-byte write from a static byte, to a descriptor the test made.
                let _ = unsafe { nix::libc::write(fd, b"d".as_ptr().cast(), 1) };
                end_if_child();
            }

            let (reader, writer) = pipe().expect("a pipe");
            let fd = usize::try_from(writer.as_raw_fd()).expect("a descriptor");
            // SAFETY: `write_byte` writes to the descriptor it is given, which the words this
            // key holds are.
            let c = c_keys::init(Some(unsafe { CDestructor::new(write_byte) })).expect("a key");
            c_keys::set(c, Some(word(fd))).expect("stored");
            PARENT.store(getpid().as_raw(), Ordering::Relaxed);
            // SAFETY: the child calls only `exit`, which runs its thread-local destructors.
            match unsafe { fork() }.expect("forked") {
                // SAFETY: as above.
                ForkResult::Child => unsafe { nix::libc::exit(0) },
                ForkResult::Parent { child } => {
                    // The parent's thread holds the value no more, so that its own exit
                    // writes nothing.
                    c_keys::set(c, None).expect("removed");
                    drop(writer);
                    assert_eq!(waitpid(child, None), Ok(WaitStatus::Exited(child, 0)));
                    let mut written = [0; 2];
                    assert_eq!(read(&reader, &mut written), Ok(1));
                    assert_eq!(&written[..1], b"d");
                }
            }
            c_keys::dispose(c).expect("disposed");
        }

        /// A child forked while another thread makes and disposes of keys, from a thread that
        /// holds a value of C's key and one of Rust's, runs its thread-local destructors at
        /// `exit`, as `git_process_start`'s child does when its `execve` fails, and ends: the
        /// thread's exit takes no lock another thread of the parent may have held at the fork.
        /// Each child is given two seconds; with the registry's lock taken at exit, nine
        /// children in ten hung.
        #[test]
        #[cfg_attr(miri, ignore = "forks")]
        fn a_forked_child_exits_while_other_threads_make_keys() {
            use core::sync::atomic::AtomicBool;
            use std::sync::Arc;
            use std::time::{Duration, Instant};

            use nix::sys::signal::{Signal, kill};
            use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
            use nix::unistd::{ForkResult, fork};

            let stop = Arc::new(AtomicBool::new(false));
            let churn = thread::spawn({
                let stop = Arc::clone(&stop);
                move || {
                    while !stop.load(Ordering::Relaxed) {
                        let id = c_keys::init(None).expect("a key");
                        c_keys::dispose(id).expect("disposed");
                        TlsKey::<u8>::new(None)
                            .expect("a key")
                            .dispose()
                            .expect("disposed");
                    }
                }
            });
            // SAFETY: `end_child` accepts any pointer, on any thread.
            let c = c_keys::init(Some(unsafe { CDestructor::new(end_child) })).expect("a key");
            c_keys::set(c, Some(word(0x80))).expect("stored");
            let rust = TlsKey::<u8>::new(None).expect("a key");
            rust.set(Some(1)).expect("stored");

            PARENT.store(getpid().as_raw(), Ordering::Relaxed);
            let mut hung = 0;
            for _ in 0..40 {
                // SAFETY: the child calls only `exit`, which runs its thread-local destructors.
                match unsafe { fork() }.expect("forked") {
                    // SAFETY: as above.
                    ForkResult::Child => unsafe { nix::libc::exit(0) },
                    ForkResult::Parent { child } => {
                        let start = Instant::now();
                        while waitpid(child, Some(WaitPidFlag::WNOHANG)).expect("waited")
                            == WaitStatus::StillAlive
                        {
                            if start.elapsed() > Duration::from_secs(2) {
                                hung += 1;
                                kill(child, Signal::SIGKILL).expect("killed");
                                waitpid(child, None).expect("reaped");
                                break;
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
            }
            stop.store(true, Ordering::Relaxed);
            churn.join().expect("the thread ends");
            assert_eq!(hung, 0, "forked children hung in exit");
            c_keys::set(c, None).expect("removed");
            c_keys::dispose(c).expect("disposed");
            rust.dispose().expect("disposed");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Thread-local storage and `git_threads_global_init` (wave-226/batch-3)
// ---------------------------------------------------------------------------------------------

use core::any::{Any, TypeId};
use core::marker::PhantomData;
use core::num::NonZeroU32;
use std::sync::Arc;

use self::tls::{Destructor, Kind, Value, dispose_key, get_value, registry, set_value};

/// Sets up what libgit2's threads need before its runtime starts (`git_threads_global_init`,
/// the third routine `git_libgit2_init` runs): nothing, where libgit2 is built with POSIX
/// threads, as here. It cannot fail, so it returns nothing; its export, which C's table of
/// initialisers holds once the module has switched, returns 0, the success C's returns.
///
/// The configurations this build does not take: on Windows it finds the slim reader/writer
/// lock routines in `kernel32` and makes the fiber-local slot that names a thread, failing
/// when none is left; without threads it is `git__noop`, which does nothing too.
///
/// ```
/// libgit2_crustify::libgit2::src::util::thread::global_init();
/// ```
///
/// Replaces `fn::git_threads_global_init`
#[doc(alias = "git_threads_global_init")]
#[inline]
pub fn global_init() {}

/// Why a call on a [`TlsKey`] failed. C's routines report a failure as a non-zero return
/// alone and set no error message, since libgit2 keeps its error state in thread-local storage
/// itself (`thread.h`); its exports report each of these so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TlsError {
    /// Every key id has been handed out, 2³² − 1 of them over the process's life:
    /// `pthread_key_create`'s `EAGAIN`.
    KeysExhausted,
    /// The key has been disposed of: `pthread_setspecific`'s and `pthread_key_delete`'s
    /// `EINVAL`. Through the exports, also a key id C passes that names none of C's keys.
    Disposed,
    /// The thread cannot store a value now: its storage is being destroyed, as the thread
    /// exits, or a value's `Clone`, which [`TlsKey::get`] runs, is storing one.
    Unavailable,
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TlsError::KeysExhausted => "no thread-local key is left",
            TlsError::Disposed => "the thread-local key has been disposed of",
            TlsError::Unavailable => "the thread's local storage is unavailable",
        })
    }
}

impl core::error::Error for TlsError {}

/// A thread-local key (`git_tlsdata_key`): a key made at run time, under which each thread
/// stores a value of its own, of type `T`, and whose destructor, if it has one, runs on each
/// thread's value at that thread's exit.
///
/// A key is an id, `Copy` as C's `pthread_key_t` is, and every copy names the same key; once
/// it is [disposed](Self::dispose) of, no copy names one, and no other key takes its id. The
/// values never leave the thread that stored them, so a key is `Send` and `Sync` whatever `T`
/// is: a key in a `static` serves every thread, each with its own value.
///
/// ```
/// use std::rc::Rc;
/// use std::thread;
///
/// use libgit2_crustify::libgit2::src::util::thread::TlsKey;
///
/// let key = TlsKey::<Rc<String>>::new(None)?;
/// assert_eq!(key.get(), None);
/// key.set(Some(Rc::new("main".to_owned())))?;
/// thread::spawn(move || {
///     assert_eq!(key.get(), None);
///     key.set(Some(Rc::new("other".to_owned()))).unwrap();
///     assert_eq!(key.get().as_deref().map(String::as_str), Some("other"));
/// })
/// .join()
/// .unwrap();
/// assert_eq!(key.get().as_deref().map(String::as_str), Some("main"));
/// key.dispose()?;
/// assert_eq!(key.get(), None);
/// # Ok::<(), libgit2_crustify::libgit2::src::util::thread::TlsError>(())
/// ```
///
/// A value outlives the thread that stores it only as what it holds, so it is `'static`:
///
/// ```compile_fail,E0597
/// use libgit2_crustify::libgit2::src::util::thread::TlsKey;
///
/// let text = String::from("borrowed");
/// let key = TlsKey::<&str>::new(None).unwrap();
/// key.set(Some(&text)).unwrap();
/// ```
///
/// Replaces `mac::git_tlsdata_key`
#[doc(alias = "git_tlsdata_key")]
pub struct TlsKey<T: 'static> {
    /// The key's id, as C's `pthread_key_t` would hold it; never 0.
    id: NonZeroU32,
    /// Which values the key holds; `fn() -> T` keeps the key `Send` and `Sync`, since no value
    /// moves between threads.
    value: PhantomData<fn() -> T>,
}

impl<T: 'static> TlsKey<T> {
    /// Makes a key (`git_tlsdata_init`), under which no thread holds a value yet. At each
    /// thread's exit, `destructor`, if given, runs on the value the thread still holds, and the
    /// value is dropped otherwise, while the key is live.
    ///
    /// # Errors
    ///
    /// [`TlsError::KeysExhausted`] when every id has been handed out.
    ///
    /// Replaces `fn::git_tlsdata_init`
    #[doc(alias = "git_tlsdata_init")]
    pub fn new(destructor: Option<TlsdataInitDestroyFn<T>>) -> Result<TlsKey<T>, TlsError> {
        let run = destructor.map(|TlsdataInitDestroyFn(f)| -> tls::RustDestructor {
            Arc::new(move |value: Box<dyn Any>| {
                if let Ok(value) = value.downcast::<T>() {
                    f(*value);
                }
            })
        });
        let id = registry().create(Destructor::Rust {
            type_id: TypeId::of::<T>(),
            run,
        })?;
        Ok(TlsKey {
            id,
            value: PhantomData,
        })
    }

    /// Stores `value` as this thread's value of the key (`git_tlsdata_set`), `None` as C's
    /// `NULL`, and returns the value it replaces, which the destructor does not run on, as C's
    /// does not.
    ///
    /// # Errors
    ///
    /// [`TlsError::Disposed`] when the key has been disposed of, and
    /// [`TlsError::Unavailable`] while the thread exits, from a destructor or a value's drop,
    /// or from a value's `Clone` that [`get`](Self::get) runs. `value` is dropped.
    ///
    /// Replaces `fn::git_tlsdata_set`
    #[doc(alias = "git_tlsdata_set")]
    pub fn set(self, value: Option<T>) -> Result<Option<T>, TlsError> {
        let value = value.map(|value| Value::Rust(Box::new(value)));
        let previous = set_value(self.id.get(), Self::kind(), value)?;
        Ok(previous.and_then(|previous| match previous {
            Value::Rust(previous) => previous.downcast::<T>().ok().map(|previous| *previous),
            Value::C(_) => None,
        }))
    }

    /// A clone of this thread's value of the key (`git_tlsdata_get`), `None` where the thread
    /// holds none, as C's `NULL`: before the thread stores one, once it stores `None`, once the
    /// key is disposed of, and while the thread exits.
    ///
    /// The value is cloned while the thread's table is borrowed: a `Clone` that stores a
    /// value of any key fails to, with [`TlsError::Unavailable`].
    ///
    /// Replaces `fn::git_tlsdata_get`
    #[doc(alias = "git_tlsdata_get")]
    #[must_use]
    pub fn get(self) -> Option<T>
    where
        T: Clone,
    {
        get_value(self.id.get(), Self::kind(), |value| match value {
            Value::Rust(value) => value.downcast_ref::<T>().cloned(),
            Value::C(_) => None,
        })
    }

    /// Runs `f` on this thread's value of the key, lent, or on `None` where [`get`](Self::get)
    /// gives `None`, and returns what it returns, as the standard library's
    /// [`LocalKey::with`](std::thread::LocalKey::with) lends a thread-local: the value is
    /// neither cloned nor moved.
    ///
    /// `f` runs while the thread's table is borrowed: it reads any key's value, and storing a
    /// value of any key fails, with [`TlsError::Unavailable`].
    ///
    /// ```
    /// use libgit2_crustify::libgit2::src::util::thread::TlsKey;
    ///
    /// let key = TlsKey::<Vec<u8>>::new(None)?;
    /// key.set(Some(vec![1, 2, 3]))?;
    /// assert_eq!(key.with(|bytes| bytes.map(Vec::len)), Some(3));
    /// key.dispose()?;
    /// assert!(key.with(|bytes| bytes.is_none()));
    /// # Ok::<(), libgit2_crustify::libgit2::src::util::thread::TlsError>(())
    /// ```
    pub fn with<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        let mut f = Some(f);
        let found = get_value(self.id.get(), Self::kind(), |value| match value {
            Value::Rust(value) => value
                .downcast_ref::<T>()
                .and_then(|value| f.take().map(|f| f(Some(value)))),
            Value::C(_) => None,
        });
        match (found, f) {
            (Some(found), _) => found,
            (None, Some(f)) => f(None),
            (None, None) => unreachable!("`f` ran and returned a value"),
        }
    }

    /// Disposes of the key (`git_tlsdata_dispose`): no copy of it names a key from then on.
    /// No destructor runs, on this thread or any other: the values the threads still hold are
    /// dropped as each thread next reaches its storage, or exits, as POSIX leaves them to
    /// their holders.
    ///
    /// # Errors
    ///
    /// [`TlsError::Disposed`] when the key has already been disposed of.
    ///
    /// Replaces `fn::git_tlsdata_dispose`
    #[doc(alias = "git_tlsdata_dispose")]
    pub fn dispose(self) -> Result<(), TlsError> {
        dispose_key(self.id.get(), Self::kind())
    }

    /// The kind of value the key holds, which its registry entry and its values carry.
    fn kind() -> Kind {
        Kind::Rust(TypeId::of::<T>())
    }
}

impl<T: 'static> Clone for TlsKey<T> {
    fn clone(&self) -> TlsKey<T> {
        *self
    }
}

// A copy names the same key; `#[derive]` would bound `T`.
impl<T: 'static> Copy for TlsKey<T> {}

impl<T: 'static> PartialEq for TlsKey<T> {
    fn eq(&self, other: &TlsKey<T>) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for TlsKey<T> {}

impl<T: 'static> core::hash::Hash for TlsKey<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> fmt::Debug for TlsKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TlsKey").field(&self.id).finish()
    }
}

/// The destructor of a [`TlsKey`]'s values (`git_tlsdata_init`'s `destroy_fn`), which runs on
/// the value a thread still holds of a live key at that thread's exit, on that thread. A plain
/// function, or a closure without captures: like C's, it carries no context of its own.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::{TlsKey, TlsdataInitDestroyFn};
///
/// fn flush(lines: Vec<String>) {
///     for line in lines {
///         println!("{line}");
///     }
/// }
///
/// let key = TlsKey::new(Some(TlsdataInitDestroyFn::new(flush)))?;
/// std::thread::spawn(move || key.set(Some(vec!["at exit".to_owned()])).map(drop))
///     .join()
///     .unwrap()?;
/// # Ok::<(), libgit2_crustify::libgit2::src::util::thread::TlsError>(())
/// ```
///
/// The C functions C passes `git_tlsdata_init` free the value they are given
/// (`threadstate_free`, `thread_str_free`), so they are sound only with the values C stores
/// under their own keys: they reach Rust through the export alone, which keeps them with their
/// key, never as this type, as a C destructor sound with any value would be. No C calls a
/// destructor, so no trampoline reaches one.
///
/// Replaces `fnp::git_tlsdata_init.destroy_fn`
#[doc(alias = "git_tlsdata_init.destroy_fn")]
pub struct TlsdataInitDestroyFn<T>(fn(T));

impl<T> TlsdataInitDestroyFn<T> {
    /// The destructor that calls `f` with the value.
    #[must_use]
    pub const fn new(f: fn(T)) -> TlsdataInitDestroyFn<T> {
        TlsdataInitDestroyFn(f)
    }
}

impl<T> Clone for TlsdataInitDestroyFn<T> {
    fn clone(&self) -> TlsdataInitDestroyFn<T> {
        *self
    }
}

// A copy calls the same function; `#[derive]` would bound `T`.
impl<T> Copy for TlsdataInitDestroyFn<T> {}

impl<T> fmt::Debug for TlsdataInitDestroyFn<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TlsdataInitDestroyFn")
            .finish_non_exhaustive()
    }
}

/// The keys C makes, uses and disposes of through the exports, over the same registry and
/// tables as [`TlsKey`]'s: their values are [`CWord`](ffi_import::CWord)s, and their
/// destructors C's.
#[cfg_attr(
    not(any(test, feature = "ffi-export-src-util-thread-c")),
    expect(
        dead_code,
        reason = "the export gateway alone calls them, under its feature"
    )
)]
mod c_keys {
    use super::TlsError;
    use super::ffi_import::{CDestructor, CWord};
    use super::tls::{Destructor, Kind, Value, dispose_key, get_value, registry, set_value};

    /// `git_tlsdata_init`: a key whose values C stores, with C's `destructor`.
    pub(super) fn init(destructor: Option<CDestructor>) -> Result<u32, TlsError> {
        registry()
            .create(Destructor::C(destructor))
            .map(core::num::NonZeroU32::get)
    }

    /// `git_tlsdata_set`: stores `word`, `None` for C's `NULL`, under C's key `id`.
    pub(super) fn set(id: u32, word: Option<CWord>) -> Result<(), TlsError> {
        set_value(id, Kind::C, word.map(Value::C)).map(drop)
    }

    /// `git_tlsdata_get`: this thread's value of C's key `id`, `None` for C's `NULL`.
    pub(super) fn get(id: u32) -> Option<CWord> {
        get_value(id, Kind::C, |value| match value {
            Value::C(word) => Some(*word),
            Value::Rust(_) => None,
        })
    }

    /// `git_tlsdata_dispose`: disposes of C's key `id`.
    pub(super) fn dispose(id: u32) -> Result<(), TlsError> {
        dispose_key(id, Kind::C)
    }
}

/// The keys' registry and the threads' tables, which [`TlsKey`] and the exports share.
mod tls {
    use core::any::{Any, TypeId};
    use core::cell::RefCell;
    use core::mem;
    use core::num::NonZeroU32;
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::thread::LocalKey;

    use super::TlsError;
    use super::ffi_import::{CDestructor, CWord};

    /// A Rust destructor, the typed function of a
    /// [`TlsdataInitDestroyFn`](super::TlsdataInitDestroyFn) over the boxed value, which it
    /// unboxes; shared between the registry and the values stored under its key.
    pub(super) type RustDestructor = Arc<dyn Fn(Box<dyn Any>) + Send + Sync>;

    /// What a key holds: Rust values of one type, or C's pointers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Kind {
        /// The values of a [`TlsKey`](super::TlsKey) of the type.
        Rust(TypeId),
        /// The pointers of a key C made through `git_tlsdata_init`.
        C,
    }

    /// What runs on a live key's value at its thread's exit.
    #[derive(Clone)]
    pub(super) enum Destructor {
        /// A [`TlsKey`](super::TlsKey)'s: its values' type, and the destructor that runs on
        /// one, which is dropped otherwise.
        Rust {
            type_id: TypeId,
            run: Option<RustDestructor>,
        },
        /// A key's C made, with C's destructor, if it gave one.
        C(Option<CDestructor>),
    }

    impl Destructor {
        /// The kind of value the key holds.
        fn kind(&self) -> Kind {
            match self {
                Destructor::Rust { type_id, .. } => Kind::Rust(*type_id),
                Destructor::C(_) => Kind::C,
            }
        }
    }

    /// A value a thread stores under a key.
    pub(super) enum Value {
        /// A [`TlsKey`](super::TlsKey)'s, of its type.
        Rust(Box<dyn Any>),
        /// A pointer C stored under one of its keys.
        C(CWord),
    }

    /// A key's state, which the registry holds while the key is live, and each value a thread
    /// stores under it holds beside it, so that a thread reads it without the registry's lock.
    pub(super) struct Key {
        /// The key's id.
        id: u32,
        /// Which values the key holds.
        kind: Kind,
        /// Whether the key is live: cleared, once, when it is disposed of.
        live: AtomicBool,
        /// What runs on a value at its thread's exit, while the key is live.
        destructor: Destructor,
    }

    impl Key {
        /// Whether the key is live and holds values of `kind`.
        fn holds(&self, kind: Kind) -> bool {
            self.kind == kind && self.live.load(Ordering::Acquire)
        }
    }

    /// The live keys, by id, and the next id to hand out.
    pub(super) struct Registry {
        /// The next id; 0 once every id has been handed out.
        next: u32,
        /// The live keys, by id.
        live: BTreeMap<u32, Arc<Key>>,
    }

    impl Registry {
        /// A registry with no key, whose first id is 1.
        const fn new() -> Registry {
            Registry {
                next: 1,
                live: BTreeMap::new(),
            }
        }

        /// Registers a key with `destructor`, under an id no key has had.
        pub(super) fn create(&mut self, destructor: Destructor) -> Result<NonZeroU32, TlsError> {
            let id = NonZeroU32::new(self.next).ok_or(TlsError::KeysExhausted)?;
            // `u32::MAX` wraps to 0, which hands out nothing from then on.
            self.next = self.next.wrapping_add(1);
            let key = Key {
                id: id.get(),
                kind: destructor.kind(),
                live: AtomicBool::new(true),
                destructor,
            };
            self.live.insert(id.get(), Arc::new(key));
            Ok(id)
        }

        /// The live key `id`, if it holds values of `kind`.
        fn key(&self, id: u32, kind: Kind) -> Option<&Arc<Key>> {
            self.live.get(&id).filter(|key| key.holds(kind))
        }

        /// Whether `id` is a live key holding values of `kind`.
        fn holds(&self, id: u32, kind: Kind) -> bool {
            self.key(id, kind).is_some()
        }
    }

    /// The registry, shared by every thread. A thread takes its lock to make or dispose of a
    /// key, and to store a first value under one; reading a value, replacing it, and a thread's
    /// exit take none.
    static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

    /// The registry, locked. Nothing panics while holding it, so a poisoned lock still holds
    /// a consistent registry.
    pub(super) fn registry() -> MutexGuard<'static, Registry> {
        REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One thread's values, one entry per key it holds a value of.
    pub(super) struct Table {
        /// The values, each with its key, live and of the value's kind when the entry was
        /// made.
        entries: Vec<(Arc<Key>, Value)>,
    }

    impl Table {
        /// A table with no value, as a thread starts with.
        pub(super) const fn new() -> Table {
            Table {
                entries: Vec::new(),
            }
        }

        /// Takes out the entries of the keys disposed of since they were made, for the caller
        /// to drop once it has released the table, since a Rust value's drop may reach it.
        fn prune(&mut self) -> Vec<(Arc<Key>, Value)> {
            if self
                .entries
                .iter()
                .all(|(key, _)| key.live.load(Ordering::Acquire))
            {
                return Vec::new();
            }
            let (live, dead) = mem::take(&mut self.entries)
                .into_iter()
                .partition(|(key, _)| key.live.load(Ordering::Acquire));
            self.entries = live;
            dead
        }

        /// The index of `id`'s entry.
        fn position(&self, id: u32) -> Option<usize> {
            self.entries.iter().position(|(key, _)| key.id == id)
        }
    }

    /// At the thread's exit, each value of a live key goes to its key's destructor, and every
    /// other value is dropped; a C value without a destructor, or of a disposed key, is left as
    /// POSIX leaves it, to C. It takes no lock, so that a child forked from a threaded process,
    /// which runs its thread-local destructors at `exit` (libgit2's `git_process_start` does,
    /// when its `execve` fails), cannot wait on a lock that another thread of its parent held.
    /// The destructors run with the table out of reach, so that a destructor that stores a
    /// value fails to.
    impl Drop for Table {
        fn drop(&mut self) {
            for (key, value) in mem::take(&mut self.entries) {
                if !key.live.load(Ordering::Acquire) {
                    drop(value);
                    continue;
                }
                match (value, &key.destructor) {
                    (Value::Rust(value), Destructor::Rust { run: Some(run), .. }) => run(value),
                    (Value::C(word), Destructor::C(Some(destructor))) => destructor.destroy(word),
                    (value, _) => drop(value),
                }
            }
        }
    }

    /// This thread's values, which `ffi_import` keeps, since `thread_local!` expands to unsafe
    /// code.
    fn table() -> &'static LocalKey<RefCell<Table>> {
        super::ffi_import::tls_table()
    }

    /// Stores `value` as this thread's value of the key `id`, of `kind`, `None` removing it,
    /// and returns the value it replaces.
    pub(super) fn set_value(
        id: u32,
        kind: Kind,
        value: Option<Value>,
    ) -> Result<Option<Value>, TlsError> {
        let stored = table().try_with(|table| {
            let Ok(mut table) = table.try_borrow_mut() else {
                return (Err(TlsError::Unavailable), Vec::new(), value);
            };
            let dead = table.prune();
            let result = match (table.position(id), value) {
                // An entry's key is live, since the table has just been pruned.
                (Some(index), value) if table.entries[index].0.kind == kind => match value {
                    Some(value) => Ok(Some(mem::replace(&mut table.entries[index].1, value))),
                    None => Ok(Some(table.entries.swap_remove(index).1)),
                },
                (Some(_), value) => return (Err(TlsError::Disposed), dead, value),
                (None, value) => {
                    let Some(key) = registry().key(id, kind).cloned() else {
                        return (Err(TlsError::Disposed), dead, value);
                    };
                    if let Some(value) = value {
                        table.entries.push((key, value));
                    }
                    Ok(None)
                }
            };
            (result, dead, None)
        });
        // The values the call leaves behind, dropped with the table released.
        match stored {
            Ok((result, dead, refused)) => {
                drop(dead);
                drop(refused);
                result
            }
            Err(_) => Err(TlsError::Unavailable),
        }
    }

    /// `read` of this thread's value of the key `id`, of `kind`, with the table borrowed;
    /// `None` where the thread holds none, or cannot reach its table.
    pub(super) fn get_value<R>(
        id: u32,
        kind: Kind,
        read: impl FnOnce(&Value) -> Option<R>,
    ) -> Option<R> {
        table()
            .try_with(|table| {
                // The entries of disposed keys go first, when the table is free; a `Clone`
                // that reads it while it is borrowed finds them by their key's state.
                let dead = table.try_borrow_mut().map(|mut table| table.prune());
                let found = table.try_borrow().ok().and_then(|table| {
                    let index = table.position(id)?;
                    let (key, value) = &table.entries[index];
                    if key.holds(kind) { read(value) } else { None }
                });
                drop(dead);
                found
            })
            .ok()
            .flatten()
    }

    /// Disposes of the key `id`, of `kind`.
    pub(super) fn dispose_key(id: u32, kind: Kind) -> Result<(), TlsError> {
        let mut registry = registry();
        if !registry.holds(id, kind) {
            return Err(TlsError::Disposed);
        }
        if let Some(key) = registry.live.remove(&id) {
            key.live.store(false, Ordering::Release);
        }
        Ok(())
    }

    #[cfg(test)]
    mod unit_priv {
        use core::any::TypeId;
        use core::num::NonZeroU32;

        use super::{Destructor, Kind, Registry};
        use crate::libgit2::src::util::thread::TlsError;

        /// A destructor-less Rust key's registry entry.
        fn plain() -> Destructor {
            Destructor::Rust {
                type_id: TypeId::of::<()>(),
                run: None,
            }
        }

        /// Ids start at 1, so that none is 0, the zeroed `static` C has not initialised yet;
        /// each is handed out once, and once `u32::MAX` has been, none is left.
        #[test]
        fn ids_start_at_one_and_run_out_after_the_last() {
            let mut registry = Registry::new();
            assert_eq!(registry.create(plain()).map(NonZeroU32::get), Ok(1));
            assert_eq!(registry.create(plain()).map(NonZeroU32::get), Ok(2));
            registry.next = u32::MAX;
            assert_eq!(registry.create(plain()).map(NonZeroU32::get), Ok(u32::MAX));
            assert_eq!(
                registry.create(plain()).map(NonZeroU32::get),
                Err(TlsError::KeysExhausted)
            );
            assert_eq!(
                registry.create(Destructor::C(None)).map(NonZeroU32::get),
                Err(TlsError::KeysExhausted)
            );
            assert_eq!(registry.live.len(), 3);
        }

        /// A key holds the kind it was made for alone: C's pointers, or one Rust type.
        #[test]
        fn a_key_holds_one_kind() {
            let mut registry = Registry::new();
            let rust = registry.create(plain()).expect("an id").get();
            let c = registry.create(Destructor::C(None)).expect("an id").get();
            assert!(registry.holds(rust, Kind::Rust(TypeId::of::<()>())));
            assert!(!registry.holds(rust, Kind::Rust(TypeId::of::<u8>())));
            assert!(!registry.holds(rust, Kind::C));
            assert!(registry.holds(c, Kind::C));
            assert!(!registry.holds(c, Kind::Rust(TypeId::of::<()>())));
            assert!(!registry.holds(0, Kind::C));
            assert!(!registry.holds(c + 1, Kind::C));
        }
    }
}

/// The gateway through which the C that remains reaches this module once it has switched:
///
/// - `thread.h`'s eleven atomic operations, `git_atomic32_get`, `_set`, `_inc`, `_dec` and `_add`,
///   `git_atomic64_get`, `_set` and `_add`, and `git_atomic__compare_and_swap`, `__swap` and
///   `__load`, which its macros `git_atomic_compare_and_swap`, `git_atomic_swap`, `git_atomic_load`
///   and `git_atomic_ssize_*`, and `util.h`'s `GIT_REFCOUNT_*`, expand to, for every translation
///   unit of libgit2 and its C tests;
/// - `unix/pthread.h`'s `git_threads_global_init`, whose address `git_libgit2_init` takes; and
/// - `git_tlsdata_init`, `git_tlsdata_set`, `git_tlsdata_get` and `git_tlsdata_dispose`, for
///   `errors.c`, `oid.c` and the C tests of `threads/tlsdata.c`.
///
/// The first twelve are `static inline` in their headers, so each C caller compiles its own
/// copy of their bodies, which no export reaches: the module's switch replaces each
/// definition by its prototype, as porting's Static inline functions section has it, and the
/// macros, which stay, then expand to these exports. Each export is checked against its raw
/// binding's signature, so that a C declaration that drifts fails to build. Each atomic export
/// is the one sequentially consistent operation the builtin it replaces is, with the same
/// wrapping, over the caller's `git_atomic32`, `git_atomic64` or pointer-sized slot, which no
/// export keeps past the call: they cannot panic, and need no barrier.
#[cfg(feature = "ffi-export-src-util-thread-c")]
#[expect(
    unsafe_code,
    reason = "the bare-named exports through which the remaining C reaches the atomics and the \
              thread-local keys, the objects C passes them, and the out-parameter \
              git_tlsdata_init writes"
)]
mod ffi_export {
    use core::ffi::{c_int, c_void};
    use core::ptr::{self, NonNull};
    use core::sync::atomic::AtomicPtr;
    use std::process;

    use ffibox::CRefSeam;
    use ffibox::callback::barrier;

    #[cfg(target_pointer_width = "64")]
    use super::Atomic64Ref;
    use super::ffi_import::{CDestructor, CWord};
    use super::{
        Atomic32Ref, atomic_compare_and_swap, atomic_load, atomic_swap, c_keys, global_init,
    };
    use crate::sys::libgit2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::git_atomic32_get, git_atomic32_get];
        let _ = [sys::git_atomic32_set, git_atomic32_set];
        let _ = [sys::git_atomic32_inc, git_atomic32_inc];
        let _ = [sys::git_atomic32_dec, git_atomic32_dec];
        let _ = [sys::git_atomic32_add, git_atomic32_add];
        #[cfg(target_pointer_width = "64")]
        let _ = [sys::git_atomic64_get, git_atomic64_get];
        #[cfg(target_pointer_width = "64")]
        let _ = [sys::git_atomic64_set, git_atomic64_set];
        #[cfg(target_pointer_width = "64")]
        let _ = [sys::git_atomic64_add, git_atomic64_add];
        let _ = [
            sys::git_atomic__compare_and_swap,
            git_atomic__compare_and_swap,
        ];
        let _ = [sys::git_atomic__swap, git_atomic__swap];
        let _ = [sys::git_atomic__load, git_atomic__load];
        let _ = [sys::git_threads_global_init, git_threads_global_init];
        let _ = [sys::git_tlsdata_init, git_tlsdata_init];
        let _ = [sys::git_tlsdata_set, git_tlsdata_set];
        let _ = [sys::git_tlsdata_get, git_tlsdata_get];
        let _ = [sys::git_tlsdata_dispose, git_tlsdata_dispose];
    };

    /// The shared handle of the `git_atomic32` C passes, for the call. C's builtin faults on a
    /// null `a`, which no caller passes; this aborts on one.
    ///
    /// # Safety
    ///
    /// `a`, if not null, addresses a live, aligned `git_atomic32` for `'a`, whose value every
    /// thread writes only atomically while `'a` lasts: through `thread.h`'s operations, which
    /// are these exports, or through the atomic its handles lend.
    unsafe fn atomic32<'a>(a: *mut sys::git_atomic32) -> Atomic32Ref<'a> {
        // SAFETY: the caller guarantees the object for `'a`. `ref_from_ptr` lets nothing but a
        // `Mut` of the same borrow write it while the handle lives; an `Atomic32Ref` admits
        // beside it the atomic writes its own `val` makes, which is every write the caller
        // lets happen, the one exception `atomic_type!`'s `CCell` impl takes
        // (`crustify/evolution/ffibox-atomic-layout-types`).
        match unsafe { Atomic32Ref::from_ptr(a) } {
            Some(handle) => handle,
            None => process::abort(),
        }
    }

    /// The shared handle of the `git_atomic64` C passes, for the call, as [`atomic32`] gives a
    /// `git_atomic32`'s.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], of a `git_atomic64`.
    #[cfg(target_pointer_width = "64")]
    unsafe fn atomic64<'a>(a: *mut sys::git_atomic64) -> Atomic64Ref<'a> {
        // SAFETY: as `atomic32`'s, the caller's guarantee for `'a`.
        match unsafe { Atomic64Ref::from_ptr(a) } {
            Some(handle) => handle,
            None => process::abort(),
        }
    }

    /// The pointer-sized slot C passes (`void * volatile *`), as the standard atomic it is
    /// for the call: a pointer field, or an `intptr_t` one, which `thread.h`'s macros cast. C's
    /// builtin faults on a null `ptr`, which no caller passes; this aborts on one.
    ///
    /// # Safety
    ///
    /// `ptr`, if not null, addresses a live, pointer-aligned slot for `'a`, which every thread
    /// accesses only atomically while `'a` lasts.
    unsafe fn slot<'a>(ptr: *mut *mut c_void) -> &'a AtomicPtr<c_void> {
        if ptr.is_null() {
            process::abort();
        }
        // SAFETY: `ptr` is not null, and the caller guarantees it live, aligned for a pointer,
        // which `AtomicPtr` is, and accessed only atomically for `'a`, as `from_ptr` asks. C
        // also reads a few such slots plainly, a race of the C's own, as the module
        // documentation describes of the atomic integers.
        unsafe { AtomicPtr::from_ptr(ptr) }
    }

    /// `git_atomic32_get`: [`Atomic32Ref::get`], the sequentially consistent load
    /// `__atomic_load_n(&a->val, __ATOMIC_SEQ_CST)` is.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic32_get(a: *mut sys::git_atomic32) -> c_int {
        // SAFETY: the caller's guarantee is `atomic32`'s, for the call.
        unsafe { atomic32(a) }.get()
    }

    /// `git_atomic32_set`: [`Atomic32Ref::set`], the sequentially consistent store
    /// `__atomic_store_n(&a->val, val, __ATOMIC_SEQ_CST)` is.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic32_set(a: *mut sys::git_atomic32, val: c_int) {
        // SAFETY: as `git_atomic32_get`.
        unsafe { atomic32(a) }.set(val);
    }

    /// `git_atomic32_inc`: [`Atomic32Ref::inc`], `__atomic_add_fetch(&a->val, 1,
    /// __ATOMIC_SEQ_CST)`, returning the value it leaves, wrapped from `INT_MAX` to `INT_MIN`.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic32_inc(a: *mut sys::git_atomic32) -> c_int {
        // SAFETY: as `git_atomic32_get`.
        unsafe { atomic32(a) }.inc()
    }

    /// `git_atomic32_dec`: [`Atomic32Ref::dec`], `__atomic_sub_fetch(&a->val, 1,
    /// __ATOMIC_SEQ_CST)`, returning the value it leaves, wrapped from `INT_MIN` to `INT_MAX`.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic32_dec(a: *mut sys::git_atomic32) -> c_int {
        // SAFETY: as `git_atomic32_get`.
        unsafe { atomic32(a) }.dec()
    }

    /// `git_atomic32_add`: [`Atomic32Ref::add`], `__atomic_add_fetch(&a->val, addend,
    /// __ATOMIC_SEQ_CST)`, returning the value it leaves, wrapping on overflow.
    ///
    /// # Safety
    ///
    /// As [`atomic32`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic32_add(a: *mut sys::git_atomic32, addend: i32) -> c_int {
        // SAFETY: as `git_atomic32_get`.
        unsafe { atomic32(a) }.add(addend)
    }

    /// `git_atomic64_get`: [`Atomic64Ref::get`], the sequentially consistent load
    /// `__atomic_load_n(&a->val, __ATOMIC_SEQ_CST)` is; `git_atomic_ssize_get` names it.
    ///
    /// # Safety
    ///
    /// As [`atomic64`], for the call.
    #[cfg(target_pointer_width = "64")]
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic64_get(a: *mut sys::git_atomic64) -> i64 {
        // SAFETY: the caller's guarantee is `atomic64`'s, for the call.
        unsafe { atomic64(a) }.get()
    }

    /// `git_atomic64_set`: [`Atomic64Ref::set`], the sequentially consistent store
    /// `__atomic_store_n(&a->val, val, __ATOMIC_SEQ_CST)` is; `git_atomic_ssize_set` names it.
    ///
    /// # Safety
    ///
    /// As [`atomic64`], for the call.
    #[cfg(target_pointer_width = "64")]
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic64_set(a: *mut sys::git_atomic64, val: i64) {
        // SAFETY: as `git_atomic64_get`.
        unsafe { atomic64(a) }.set(val);
    }

    /// `git_atomic64_add`: [`Atomic64Ref::add`], `__atomic_add_fetch(&a->val, addend,
    /// __ATOMIC_SEQ_CST)`, returning the value it leaves, wrapping on overflow;
    /// `git_atomic_ssize_add` names it.
    ///
    /// # Safety
    ///
    /// As [`atomic64`], for the call.
    #[cfg(target_pointer_width = "64")]
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic64_add(a: *mut sys::git_atomic64, addend: i64) -> i64 {
        // SAFETY: as `git_atomic64_get`.
        unsafe { atomic64(a) }.add(addend)
    }

    /// `git_atomic__compare_and_swap`: [`atomic_compare_and_swap`], the strong, sequentially
    /// consistent `__atomic_compare_exchange(ptr, &foundval, &newval, false, __ATOMIC_SEQ_CST,
    /// __ATOMIC_SEQ_CST)`, returning what the slot held, which is `oldval` exactly when it
    /// stored `newval`; `git_atomic_compare_and_swap` expands to it.
    ///
    /// # Safety
    ///
    /// As [`slot`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic__compare_and_swap(
        ptr: *mut *mut c_void,
        oldval: *mut c_void,
        newval: *mut c_void,
    ) -> *mut c_void {
        // SAFETY: the caller's guarantee is `slot`'s, for the call.
        atomic_compare_and_swap(unsafe { slot(ptr) }, oldval, newval)
    }

    /// `git_atomic__swap`: [`atomic_swap`], the sequentially consistent `__atomic_exchange`,
    /// returning what the slot held; `git_atomic_swap` expands to it.
    ///
    /// # Safety
    ///
    /// As [`slot`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic__swap(
        ptr: *mut *mut c_void,
        newval: *mut c_void,
    ) -> *mut c_void {
        // SAFETY: as `git_atomic__compare_and_swap`.
        atomic_swap(unsafe { slot(ptr) }, newval)
    }

    /// `git_atomic__load`: [`atomic_load`], the sequentially consistent `__atomic_load_n(ptr,
    /// __ATOMIC_SEQ_CST)`; `git_atomic_load` expands to it.
    ///
    /// # Safety
    ///
    /// As [`slot`], for the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_atomic__load(ptr: *mut *mut c_void) -> *mut c_void {
        // SAFETY: as `git_atomic__compare_and_swap`.
        atomic_load(unsafe { slot(ptr) })
    }

    /// `git_threads_global_init`: [`global_init`], which does nothing in this build, then 0,
    /// the success C's returns, so that `git_libgit2_init`, which calls it through its table of
    /// initialisers (`init_fns`), goes on.
    #[unsafe(no_mangle)]
    extern "C" fn git_threads_global_init() -> c_int {
        global_init();
        0
    }

    /// `git_tlsdata_init`: makes a key whose values C stores, with `destroy_fn` as its
    /// destructor, and writes its id through `key`, returning 0; returns -1, writing nothing,
    /// when no id is left. A panic is reported as a failure.
    ///
    /// # Safety
    ///
    /// `key` is valid and aligned for a write of a `pthread_key_t`. `destroy_fn`, if not
    /// `NULL`, may be called once with each value C stores under the key, on the thread that
    /// stored it, at that thread's exit, while the key lives.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_tlsdata_init(
        key: *mut sys::pthread_key_t,
        destroy_fn: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int {
        // SAFETY: the caller vouches for `destroy_fn` with the key it makes, as
        // `CDestructor::new` asks.
        let destructor = destroy_fn.map(|f| unsafe { CDestructor::new(f) });
        match barrier(|| c_keys::init(destructor).ok(), || None) {
            Some(id) => {
                // SAFETY: the caller guarantees that `key` is valid for the write.
                unsafe { key.write(id) };
                0
            }
            None => -1,
        }
    }

    /// `git_tlsdata_set`: stores `value` as this thread's value of C's key `key`, `NULL`
    /// removing it, and returns 0; returns -1 when `key` names none of C's live keys, or while
    /// the thread exits. The value replaced is left to C, as POSIX leaves it. A panic is
    /// reported as a failure.
    ///
    /// # Safety
    ///
    /// `value`, if not `NULL`, may be passed to `key`'s destructor at this thread's exit.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_tlsdata_set(key: sys::pthread_key_t, value: *mut c_void) -> c_int {
        // SAFETY: the caller vouches for `value` with `key`'s destructor, as `CWord::new` asks;
        // the word is stored under `key` alone.
        let word = NonNull::new(value).map(|value| unsafe { CWord::new(value) });
        if barrier(|| c_keys::set(key, word).is_ok(), || false) {
            0
        } else {
            -1
        }
    }

    /// `git_tlsdata_get`: this thread's value of C's key `key`, `NULL` where it holds none, or
    /// where `key` names none of C's live keys. A panic is reported as `NULL`.
    #[unsafe(no_mangle)]
    extern "C" fn git_tlsdata_get(key: sys::pthread_key_t) -> *mut c_void {
        barrier(
            || c_keys::get(key).map_or(ptr::null_mut(), CWord::as_ptr),
            ptr::null_mut,
        )
    }

    /// `git_tlsdata_dispose`: disposes of C's key `key`, running no destructor, and returns 0;
    /// returns -1 when `key` names none of C's live keys. A panic is reported as a failure.
    #[unsafe(no_mangle)]
    extern "C" fn git_tlsdata_dispose(key: sys::pthread_key_t) -> c_int {
        if barrier(|| c_keys::dispose(key).is_ok(), || false) {
            0
        } else {
            -1
        }
    }

    #[cfg(test)]
    mod unit_priv {
        use core::ffi::{c_int, c_void};
        use core::marker::PhantomData;
        use core::ptr;
        use core::sync::atomic::{AtomicIsize, AtomicPtr};
        use std::sync::{Mutex, PoisonError};
        use std::thread;

        use ffibox::CRefSeam;

        #[cfg(target_pointer_width = "64")]
        use super::super::Atomic64;
        use super::super::{Atomic32, atomic_load};
        use super::{
            git_atomic__compare_and_swap, git_atomic__load, git_atomic__swap, git_atomic32_add,
            git_atomic32_dec, git_atomic32_get, git_atomic32_inc, git_atomic32_set,
            git_threads_global_init, git_tlsdata_dispose, git_tlsdata_get, git_tlsdata_init,
            git_tlsdata_set,
        };
        #[cfg(target_pointer_width = "64")]
        use super::{git_atomic64_add, git_atomic64_get, git_atomic64_set};
        use crate::sys::libgit2 as sys;

        /// The addresses [`record`] was called with.
        static LOG: Mutex<Vec<usize>> = Mutex::new(Vec::new());

        /// A destructor that records the address it is given, and reads nothing through it.
        unsafe extern "C" fn record(value: *mut c_void) {
            LOG.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(value.addr());
        }

        /// A pointer C would store, at `addr`, which nothing reads through.
        fn at(addr: usize) -> *mut c_void {
            ptr::without_provenance_mut(addr)
        }

        /// A key `git_tlsdata_init` makes, as `errors.c` makes its own.
        fn init(destroy_fn: Option<unsafe extern "C" fn(*mut c_void)>) -> sys::pthread_key_t {
            let mut key: sys::pthread_key_t = 0;
            // SAFETY: `key` is a local, and `record` accepts any pointer on any thread.
            assert_eq!(unsafe { git_tlsdata_init(&raw mut key, destroy_fn) }, 0);
            assert_ne!(key, 0);
            key
        }

        /// The calls `threads/tlsdata.c` and `errors.c` make, through the exports: a value is
        /// `NULL` until stored, read back on its thread alone, removed by `NULL`, destroyed
        /// at its thread's exit while the key is live, and the key disposed of once.
        #[test]
        fn the_exports_keep_cs_contract() {
            let key = init(Some(record));
            assert!(git_tlsdata_get(key).is_null());
            // SAFETY: `record` accepts any pointer.
            assert_eq!(unsafe { git_tlsdata_set(key, at(0x60)) }, 0);
            assert_eq!(git_tlsdata_get(key), at(0x60));
            thread::spawn(move || {
                assert!(git_tlsdata_get(key).is_null());
                // SAFETY: as above.
                assert_eq!(unsafe { git_tlsdata_set(key, at(0x61)) }, 0);
            })
            .join()
            .expect("the thread ends");
            assert_eq!(
                LOG.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_slice(),
                [0x61]
            );
            // SAFETY: `NULL` reaches no destructor.
            assert_eq!(unsafe { git_tlsdata_set(key, ptr::null_mut()) }, 0);
            assert!(git_tlsdata_get(key).is_null());
            assert_eq!(git_tlsdata_dispose(key), 0);
            assert_eq!(git_tlsdata_dispose(key), -1);
            assert!(git_tlsdata_get(key).is_null());
            // SAFETY: the key is gone, so nothing keeps the pointer.
            assert_eq!(unsafe { git_tlsdata_set(key, at(0x62)) }, -1);
        }

        /// The zeroed key a C caller has not initialised names no key: `threadstate_get`'s
        /// read before `git_error_global_init` gives `NULL`, and its store fails.
        #[test]
        fn key_zero_names_no_key() {
            assert!(git_tlsdata_get(0).is_null());
            // SAFETY: no key keeps the pointer.
            assert_eq!(unsafe { git_tlsdata_set(0, at(0x70)) }, -1);
            assert_eq!(git_tlsdata_dispose(0), -1);
            let key = init(None);
            assert!(git_tlsdata_get(0).is_null());
            assert_eq!(git_tlsdata_dispose(key), 0);
        }

        /// A `git_atomic32` Rust holds, reached as C reaches one: through the exports, over
        /// the pointer its handle lends, which addresses its `UnsafeCell`.
        struct C32<'a>(*mut sys::git_atomic32, PhantomData<&'a Atomic32>);

        impl C32<'_> {
            fn new(count: &Atomic32) -> C32<'_> {
                C32(count.as_ref().as_ptr().cast_mut(), PhantomData)
            }

            fn get(&self) -> c_int {
                // SAFETY: `self.0` addresses the count `self` borrows, live and aligned, which
                // every thread writes only atomically.
                unsafe { git_atomic32_get(self.0) }
            }

            fn set(&self, val: c_int) {
                // SAFETY: as `get`.
                unsafe { git_atomic32_set(self.0, val) };
            }

            fn inc(&self) -> c_int {
                // SAFETY: as `get`.
                unsafe { git_atomic32_inc(self.0) }
            }

            fn dec(&self) -> c_int {
                // SAFETY: as `get`.
                unsafe { git_atomic32_dec(self.0) }
            }

            fn add(&self, addend: i32) -> c_int {
                // SAFETY: as `get`.
                unsafe { git_atomic32_add(self.0, addend) }
            }
        }

        // SAFETY: `C32` is a shared borrow of an `Atomic32`, which is `Sync`; every access
        // through it is atomic.
        unsafe impl Send for C32<'_> {}

        // SAFETY: as above.
        unsafe impl Sync for C32<'_> {}

        /// A pointer-sized slot Rust holds, reached through the exports, as C's macros reach a
        /// pointer field: `git_atomic_compare_and_swap(&field, …)` casts it to
        /// `void * volatile *`.
        struct Slot<'a>(*mut *mut c_void, PhantomData<&'a AtomicPtr<c_void>>);

        impl Slot<'_> {
            fn new(slot: &AtomicPtr<c_void>) -> Slot<'_> {
                Slot(slot.as_ptr(), PhantomData)
            }

            fn compare_and_swap(&self, old: *mut c_void, new: *mut c_void) -> *mut c_void {
                // SAFETY: `self.0` addresses the slot `self` borrows, live and aligned, which
                // every thread accesses only atomically.
                unsafe { git_atomic__compare_and_swap(self.0, old, new) }
            }

            fn swap(&self, new: *mut c_void) -> *mut c_void {
                // SAFETY: as `compare_and_swap`.
                unsafe { git_atomic__swap(self.0, new) }
            }

            fn load(&self) -> *mut c_void {
                // SAFETY: as `compare_and_swap`.
                unsafe { git_atomic__load(self.0) }
            }
        }

        // SAFETY: `Slot` is a shared borrow of an `AtomicPtr`, which is `Sync`; every access
        // through it is atomic.
        unsafe impl Send for Slot<'_> {}

        // SAFETY: as above.
        unsafe impl Sync for Slot<'_> {}

        /// Threads per test that shares one object, and steps per thread.
        const THREADS: usize = 4;
        const STEPS: usize = 500;

        /// The calls `GIT_REFCOUNT_INC`, `GIT_REFCOUNT_DEC` and the object cache make: each
        /// returns the value it leaves, wrapping at `int`'s bounds as the builtins do, and the
        /// native API reads and writes the same value.
        #[test]
        fn atomic32_exports_return_what_they_leave() {
            let count = Atomic32::new(0);
            let c = C32::new(&count);
            c.set(1);
            assert_eq!(c.get(), 1);
            assert_eq!(c.inc(), 2);
            assert_eq!(c.dec(), 1);
            assert_eq!(c.dec(), 0);
            assert_eq!(c.add(-5), -5);
            assert_eq!(c.add(7), 2);
            assert_eq!(c.add(0), 2);
            c.set(i32::MAX);
            assert_eq!(c.inc(), i32::MIN);
            assert_eq!(c.dec(), i32::MAX);
            assert_eq!(c.add(2), i32::MIN + 1);
            assert_eq!(c.add(i32::MIN), 1);
            c.set(i32::MIN);
            assert_eq!(c.dec(), i32::MAX);
            assert_eq!(count.as_ref().get(), i32::MAX);
            count.as_ref().set(41);
            assert_eq!(c.inc(), 42);
            assert_eq!(count.as_ref().get(), 42);
        }

        /// `git_atomic_ssize_*`'s calls, the object cache's total size: each returns the value
        /// it leaves, wrapping at `int64_t`'s bounds.
        #[cfg(target_pointer_width = "64")]
        #[test]
        fn atomic64_exports_return_what_they_leave() {
            let size = Atomic64::new(0);
            let a = size.as_ref().as_ptr().cast_mut();
            // SAFETY: `a` addresses `size`, live for the test and written only atomically.
            unsafe { git_atomic64_set(a, 4096) };
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_get(a) }, 4096);
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_add(a, -1024) }, 3072);
            // SAFETY: as above.
            unsafe { git_atomic64_set(a, i64::MAX) };
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_add(a, 1) }, i64::MIN);
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_add(a, -1) }, i64::MAX);
            assert_eq!(size.as_ref().get(), i64::MAX);
            size.as_ref().set(-7);
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_get(a) }, -7);
        }

        /// The calls `git_atomic_compare_and_swap`, `git_atomic_swap` and `git_atomic_load`
        /// expand to: the compare-and-swap is strong, storing only over the value it expects
        /// and returning what the slot held either way, as a lazily filled cache relies on.
        #[test]
        fn slot_exports_return_what_the_slot_held() {
            let (one, two) = (
                ptr::without_provenance_mut(0x10),
                ptr::without_provenance_mut(0x20),
            );
            let cache = AtomicPtr::new(ptr::null_mut());
            let c = Slot::new(&cache);
            assert!(c.load().is_null());
            assert!(c.compare_and_swap(ptr::null_mut(), one).is_null());
            assert_eq!(
                c.compare_and_swap(ptr::null_mut(), two),
                one,
                "it holds another"
            );
            assert_eq!(c.load(), one);
            assert_eq!(c.compare_and_swap(one, two), one);
            assert_eq!(atomic_load(&cache), two);
            assert_eq!(c.swap(ptr::null_mut()), two);
            assert!(c.swap(one).is_null());
            assert_eq!(cache.into_inner(), one);
        }

        /// An `intptr_t` slot, as `git_atomic_compare_and_swap` is given the address of one: its
        /// value goes through the pointer exports as the integer it is.
        #[test]
        fn slot_exports_carry_an_integer() {
            let word = AtomicIsize::new(-1);
            let c = Slot(word.as_ptr().cast::<*mut c_void>(), PhantomData);
            let minus_one = ptr::without_provenance_mut(usize::MAX);
            assert_eq!(c.load(), minus_one);
            assert_eq!(
                c.compare_and_swap(minus_one, ptr::without_provenance_mut(5)),
                minus_one
            );
            assert_eq!(word.into_inner(), 5);
        }

        /// Threads raise and drop one count, half through the exports and half through the
        /// native API, and lose no update; threads swap their own pointers into one slot, and
        /// every pointer stored comes out of it once, by a swap or the final load.
        #[test]
        fn exports_lose_no_update_across_threads() {
            let count = Atomic32::new(1);
            let c = C32::new(&count);
            thread::scope(|scope| {
                for t in 0..THREADS {
                    let (c, count) = (&c, &count);
                    scope.spawn(move || {
                        for _ in 0..STEPS {
                            if t % 2 == 0 {
                                assert!(c.inc() > 1);
                                assert!(c.dec() >= 1);
                            } else {
                                assert!(count.as_ref().inc() > 1);
                                assert!(count.as_ref().dec() >= 1);
                            }
                        }
                    });
                }
            });
            assert_eq!(c.get(), 1);

            let slot = AtomicPtr::new(ptr::null_mut());
            let s = Slot::new(&slot);
            let mut seen: Vec<usize> = thread::scope(|scope| {
                let handles: Vec<_> = (0..THREADS)
                    .map(|t| {
                        let s = &s;
                        scope.spawn(move || {
                            (0..STEPS)
                                .map(|i| {
                                    let mine = 1 + t * 10_000 + i;
                                    s.swap(ptr::without_provenance_mut(mine)).addr()
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("the thread ends"))
                    .collect()
            });
            seen.push(s.load().addr());
            seen.sort_unstable();
            let mut stored: Vec<usize> = (0..THREADS)
                .flat_map(|t| (0..STEPS).map(move |i| 1 + t * 10_000 + i))
                .collect();
            stored.push(0);
            stored.sort_unstable();
            assert_eq!(seen, stored);
        }

        /// `git_libgit2_init` calls `git_threads_global_init` through its table of initialisers,
        /// and goes on only where it returns 0.
        #[test]
        fn global_init_reports_success() {
            let init_fns: [extern "C" fn() -> c_int; 1] = [git_threads_global_init];
            for init in init_fns {
                assert_eq!(init(), 0);
                assert_eq!(
                    init(),
                    0,
                    "it may run again, as each git_libgit2_init runs it"
                );
            }
        }

        /// C's builtins fault on a null object, which no caller passes; each atomic export
        /// aborts on one instead, before it reaches anything through it. Each call runs in a
        /// forked child, which the abort ends, its core dump turned off.
        #[test]
        #[cfg_attr(miri, ignore = "forks")]
        fn the_atomic_exports_abort_on_a_null_object() {
            use nix::sys::signal::Signal;
            use nix::sys::wait::{WaitStatus, waitpid};
            use nix::unistd::{ForkResult, fork};

            fn get32() {
                // SAFETY: a null object, which the export refuses before any access.
                let _ = unsafe { git_atomic32_get(ptr::null_mut()) };
            }
            fn set32() {
                // SAFETY: as `get32`.
                unsafe { git_atomic32_set(ptr::null_mut(), 1) };
            }
            fn inc32() {
                // SAFETY: as `get32`.
                let _ = unsafe { git_atomic32_inc(ptr::null_mut()) };
            }
            fn dec32() {
                // SAFETY: as `get32`.
                let _ = unsafe { git_atomic32_dec(ptr::null_mut()) };
            }
            fn add32() {
                // SAFETY: as `get32`.
                let _ = unsafe { git_atomic32_add(ptr::null_mut(), 1) };
            }
            fn compare_and_swap() {
                let null = ptr::null_mut();
                // SAFETY: a null slot, which the export refuses before any access.
                let _ = unsafe { git_atomic__compare_and_swap(null, null.cast(), null.cast()) };
            }
            fn swap() {
                // SAFETY: as `compare_and_swap`.
                let _ = unsafe { git_atomic__swap(ptr::null_mut(), ptr::null_mut()) };
            }
            fn load() {
                // SAFETY: as `compare_and_swap`.
                let _ = unsafe { git_atomic__load(ptr::null_mut()) };
            }
            #[cfg(target_pointer_width = "64")]
            fn get64() {
                // SAFETY: as `get32`.
                let _ = unsafe { git_atomic64_get(ptr::null_mut()) };
            }
            #[cfg(target_pointer_width = "64")]
            fn set64() {
                // SAFETY: as `get32`.
                unsafe { git_atomic64_set(ptr::null_mut(), 1) };
            }
            #[cfg(target_pointer_width = "64")]
            fn add64() {
                // SAFETY: as `get32`.
                let _ = unsafe { git_atomic64_add(ptr::null_mut(), 1) };
            }

            let mut calls: Vec<(&str, fn())> = vec![
                ("git_atomic32_get", get32),
                ("git_atomic32_set", set32),
                ("git_atomic32_inc", inc32),
                ("git_atomic32_dec", dec32),
                ("git_atomic32_add", add32),
                ("git_atomic__compare_and_swap", compare_and_swap),
                ("git_atomic__swap", swap),
                ("git_atomic__load", load),
            ];
            #[cfg(target_pointer_width = "64")]
            calls.extend::<[(&str, fn()); 3]>([
                ("git_atomic64_get", get64),
                ("git_atomic64_set", set64),
                ("git_atomic64_add", add64),
            ]);
            let no_core = nix::libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            for (name, call) in calls {
                // SAFETY: the child turns its core dump off and makes one call, which aborts;
                // it reaches `_exit` only if the call returns.
                match unsafe { fork() }.expect("forked") {
                    ForkResult::Child => {
                        // SAFETY: `no_core` is a valid `rlimit`, read for the call.
                        unsafe { nix::libc::setrlimit(nix::libc::RLIMIT_CORE, &raw const no_core) };
                        call();
                        // SAFETY: `_exit` ends the child, running nothing more.
                        unsafe { nix::libc::_exit(0) }
                    }
                    ForkResult::Parent { child } => {
                        let status = waitpid(child, None).expect("waited");
                        assert!(
                            matches!(status, WaitStatus::Signaled(_, Signal::SIGABRT, _)),
                            "{name}(NULL) ended the child with {status:?}"
                        );
                    }
                }
            }
        }
    }

    /// The exports beside the header's C, `thread.h`'s `static inline` bodies compiled into
    /// the raw bindings' wrappers (`static_fns.c`), on one object at once, until the switch
    /// replaces those bodies by the exports. Each test is in this gateway, which alone
    /// compiles the exports, beside the C it calls.
    #[cfg(test)]
    mod equiv_priv {
        use core::ffi::c_void;
        use core::ptr;
        use std::thread;

        use ffibox::CRefSeam;

        use super::super::Atomic32;
        #[cfg(target_pointer_width = "64")]
        use super::super::Atomic64;
        use super::{
            git_atomic__compare_and_swap, git_atomic__load, git_atomic__swap, git_atomic32_add,
            git_atomic32_dec, git_atomic32_get, git_atomic32_inc, git_atomic32_set,
        };
        #[cfg(target_pointer_width = "64")]
        use super::{git_atomic64_add, git_atomic64_get, git_atomic64_set};
        use crate::sys::libgit2 as sys;

        /// One step on a `git_atomic32`, through the export or the header's C.
        #[derive(Clone, Copy, Debug)]
        enum Step {
            Get,
            Set(i32),
            Inc,
            Dec,
            Add(i32),
        }

        const STEPS: [Step; 12] = [
            Step::Set(0),
            Step::Inc,
            Step::Inc,
            Step::Dec,
            Step::Add(-9),
            Step::Get,
            Step::Set(i32::MAX),
            Step::Inc,
            Step::Dec,
            Step::Add(i32::MIN),
            Step::Set(i32::MIN),
            Step::Dec,
        ];

        /// Runs `step` on `a`, through the exports where `rust`, else through the header's C,
        /// and returns what it returns, `None` for a store.
        ///
        /// # Safety
        ///
        /// `a` addresses a live, aligned `git_atomic32`, which every thread writes only
        /// atomically.
        unsafe fn run(rust: bool, a: *mut sys::git_atomic32, step: Step) -> Option<i32> {
            match (rust, step) {
                // SAFETY: as above.
                (true, Step::Get) => Some(unsafe { git_atomic32_get(a) }),
                // SAFETY: as above.
                (false, Step::Get) => Some(unsafe { sys::git_atomic32_get(a) }),
                (true, Step::Set(v)) => {
                    // SAFETY: as above.
                    unsafe { git_atomic32_set(a, v) };
                    None
                }
                (false, Step::Set(v)) => {
                    // SAFETY: as above.
                    unsafe { sys::git_atomic32_set(a, v) };
                    None
                }
                // SAFETY: as above.
                (true, Step::Inc) => Some(unsafe { git_atomic32_inc(a) }),
                // SAFETY: as above.
                (false, Step::Inc) => Some(unsafe { sys::git_atomic32_inc(a) }),
                // SAFETY: as above.
                (true, Step::Dec) => Some(unsafe { git_atomic32_dec(a) }),
                // SAFETY: as above.
                (false, Step::Dec) => Some(unsafe { sys::git_atomic32_dec(a) }),
                // SAFETY: as above.
                (true, Step::Add(n)) => Some(unsafe { git_atomic32_add(a, n) }),
                // SAFETY: as above.
                (false, Step::Add(n)) => Some(unsafe { sys::git_atomic32_add(a, n) }),
            }
        }

        /// Each step through the export returns what the header's C returns from the same
        /// value, and leaves the same value, which the other side then reads: two counts run
        /// the sequence, one through each side, and a third alternates between them.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn atomic32_exports_match_the_header() {
            let (rust, c, mixed) = (Atomic32::new(0), Atomic32::new(0), Atomic32::new(0));
            let ptr = |v: &Atomic32| v.as_ref().as_ptr().cast_mut();
            for (i, step) in STEPS.into_iter().enumerate() {
                // SAFETY: each pointer addresses a count of this test, live, which only these
                // atomic calls write.
                let from_rust = unsafe { run(true, ptr(&rust), step) };
                // SAFETY: as above.
                let from_c = unsafe { run(false, ptr(&c), step) };
                // SAFETY: as above.
                let from_mixed = unsafe { run(i % 2 == 0, ptr(&mixed), step) };
                assert_eq!(from_rust, from_c, "{step:?}");
                assert_eq!(from_mixed, from_c, "{step:?}");
                assert_eq!(rust.as_ref().get(), c.as_ref().get(), "{step:?}");
                assert_eq!(mixed.as_ref().get(), c.as_ref().get(), "{step:?}");
            }
        }

        /// The 64-bit exports return what the header's C returns, on one value they share.
        #[cfg(target_pointer_width = "64")]
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn atomic64_exports_match_the_header() {
            let size = Atomic64::new(0);
            let a = size.as_ref().as_ptr().cast_mut();
            // SAFETY: `a` addresses `size`, live for the test, which only these atomic calls
            // write.
            unsafe { sys::git_atomic64_set(a, i64::MAX - 1) };
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic64_add(a, 1) }, i64::MAX);
            // SAFETY: as above.
            assert_eq!(unsafe { sys::git_atomic64_add(a, 1) }, i64::MIN);
            // SAFETY: as above.
            let from_rust = unsafe { git_atomic64_get(a) };
            // SAFETY: as above.
            let from_c = unsafe { sys::git_atomic64_get(a) };
            assert_eq!(from_rust, from_c);
            // SAFETY: as above.
            unsafe { git_atomic64_set(a, 12) };
            // SAFETY: as above.
            assert_eq!(unsafe { sys::git_atomic64_get(a) }, 12);
        }

        /// The slot exports and the header's C on one slot: what one stores the other finds,
        /// a compare-and-swap of either refuses the other's stale value, and both swap out
        /// what the other stored.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn slot_exports_match_the_header() {
            let (one, two) = (
                ptr::without_provenance_mut(0x10),
                ptr::without_provenance_mut(0x20),
            );
            let mut slot: *mut c_void = ptr::null_mut();
            let p = &raw mut slot;
            // SAFETY: `p` addresses `slot`, a local live for the test, which only these atomic
            // calls access.
            assert!(unsafe { git_atomic__compare_and_swap(p, ptr::null_mut(), one) }.is_null());
            // SAFETY: as above.
            let found = unsafe { sys::git_atomic__compare_and_swap(p, ptr::null_mut(), two) };
            assert_eq!(found, one, "C refuses the export's stale value");
            // SAFETY: as above.
            let found = unsafe { sys::git_atomic__compare_and_swap(p, one, two) };
            assert_eq!(found, one);
            // SAFETY: as above.
            let found = unsafe { git_atomic__compare_and_swap(p, one, ptr::null_mut()) };
            assert_eq!(found, two, "the export refuses C's stale value");
            // SAFETY: as above.
            let from_rust = unsafe { git_atomic__load(p) };
            // SAFETY: as above.
            let from_c = unsafe { sys::git_atomic__load(p) };
            assert_eq!(from_rust, from_c);
            // SAFETY: as above.
            assert_eq!(unsafe { git_atomic__swap(p, one) }, two);
            // SAFETY: as above.
            assert_eq!(unsafe { sys::git_atomic__swap(p, ptr::null_mut()) }, one);
            // SAFETY: as above.
            assert!(unsafe { git_atomic__load(p) }.is_null());
        }

        /// C threads and export threads raise and drop one reference count at once and lose
        /// no update, as `GIT_REFCOUNT_INC` and `GIT_REFCOUNT_DEC` will once the C that
        /// remains reaches the exports while the header's bodies are still expanded elsewhere.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn exports_and_header_share_one_count() {
            let count = Atomic32::new(1);
            thread::scope(|scope| {
                for t in 0..4 {
                    let count = &count;
                    scope.spawn(move || {
                        let a = count.as_ref().as_ptr().cast_mut();
                        for _ in 0..2000 {
                            // SAFETY: `a` addresses `count`, live for the scope, which only
                            // these atomic calls write.
                            assert!(unsafe { run(t % 2 == 0, a, Step::Inc) } > Some(1));
                            // SAFETY: as above.
                            assert!(unsafe { run(t % 2 == 0, a, Step::Dec) } >= Some(1));
                        }
                    });
                }
            });
            assert_eq!(count.as_ref().get(), 1);
        }
    }
}
