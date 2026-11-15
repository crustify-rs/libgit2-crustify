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
/// destroys it, as `git_mutex_free` does. The [module documentation](self#locks) gives how it
/// differs from C's.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::Mutex;
///
/// let count = Mutex::new(0);
/// *count.lock().unwrap() += 1;
/// assert_eq!(*count.lock().unwrap(), 1);
/// ```
///
/// Replaces `mac::git_mutex`
#[doc(alias = "git_mutex")]
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

impl<T: ?Sized> Drop for Mutex<T> {
    /// Destroys the lock, as `git_mutex_free` does: the borrow checker sees that no
    /// guard holds it, which POSIX asks of `pthread_mutex_destroy`'s caller.
    ///
    /// Replaces `mac::git_mutex_free`
    #[doc(alias = "git_mutex_free")]
    fn drop(&mut self) {}
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
/// and dropping it destroys it, as `git_cond_free` does. A wait may also end with no
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

impl Drop for Condvar {
    /// Destroys the condition variable, as `git_cond_free` does: a waiter borrows it,
    /// so none is left, which POSIX asks of `pthread_cond_destroy`'s caller.
    ///
    /// Replaces `mac::git_cond_free`
    #[doc(alias = "git_cond_free")]
    fn drop(&mut self) {}
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
/// and dropping it destroys it, as `git_rwlock_free` does. A read lock is not taken
/// twice by one thread, as `unix/pthread.h` asks.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::thread::RwLock;
///
/// let cache = RwLock::new(Vec::new());
/// cache.write().unwrap().push(1);
/// assert_eq!(*cache.read().unwrap(), [1]);
/// ```
///
/// Replaces `mac::git_rwlock`
#[doc(alias = "git_rwlock")]
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

impl<T: ?Sized> Drop for RwLock<T> {
    /// Destroys the lock, as `git_rwlock_free` does: the borrow checker sees that no
    /// guard holds it, which POSIX asks of `pthread_rwlock_destroy`'s caller.
    ///
    /// Replaces `mac::git_rwlock_free`
    #[doc(alias = "git_rwlock_free")]
    fn drop(&mut self) {}
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
              handles' pointers"
)]
mod ffi_import {
    use core::cell::UnsafeCell;
    use core::fmt;
    use core::marker::PhantomData;
    use core::ptr::NonNull;
    use core::sync::atomic::AtomicI32;
    #[cfg(target_pointer_width = "64")]
    use core::sync::atomic::AtomicI64;
    use std::os::unix::thread::RawPthread;

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
}
