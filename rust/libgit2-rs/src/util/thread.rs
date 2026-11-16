//! Port of `src/util/thread.c` (thread-local data) and Rust equivalents of
//! `src/util/thread.h` and `unix/pthread.h`: locks and atomics with C's
//! layout, for structs that C and Rust share.
//!
//! | C | Rust | Layout |
//! |---|---|---|
//! | `git_mutex` (`pthread_mutex_t`) | [`GitMutex`] | the same |
//! | `git_rwlock` (`pthread_rwlock_t`) | [`GitRwLock`] | the same |
//! | `git_cond` (`pthread_cond_t`) | [`GitCond`] | the same |
//! | `git_atomic32` | [`Atomic32`] | `{ volatile int val; }` |
//! | `git_atomic64` / `git_atomic_ssize` | [`Atomic64`] | `{ volatile int64_t val; }` |
//! | `git_refcount` (`util.h`) | [`GitRefcount`] | `{ git_atomic32 refcount; void *owner; }` |
//! | `void *` set with `git_atomic_compare_and_swap` | `AtomicPtr<T>` | a pointer |
//!
//! The header's inline functions and macros stay for the C. A Rust type
//! that C never sees uses `std::sync` instead.
//!
//! The locks are `Sync` and are used through `&self`: C locks them through
//! the same memory, so they must stay at one address while in use (they
//! live in C-shared structs on the heap). They are not recursive, and they
//! have no `Drop`: `git_mutex_free` (`pthread_mutex_destroy`) is called by
//! whoever disposes the struct, as in C.
//!
//! ```
//! use libgit2_rs::util::thread::{GitMutex, Atomic32};
//! static M: GitMutex = GitMutex::new();
//! let count = Atomic32::new(0);
//! {
//!     let _g = M.lock();
//!     assert_eq!(count.inc(), 1);
//! }
//! ```

use core::cell::UnsafeCell;
use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicI32, AtomicI64, AtomicPtr, Ordering};

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

/// `git_mutex`: a `pthread_mutex_t`, mirrored.
#[repr(transparent)]
pub struct GitMutex(UnsafeCell<libc::pthread_mutex_t>);

// SAFETY: a pthread mutex is made to be shared and locked from any thread;
// every access to the inner value goes through the pthread functions.
unsafe impl Sync for GitMutex {}
// SAFETY: an unlocked pthread mutex is not tied to a thread; a locked one is
// only reachable through `&self` (a guard borrows it), so it cannot move
// while locked.
unsafe impl Send for GitMutex {}

impl GitMutex {
    /// An unlocked mutex (`PTHREAD_MUTEX_INITIALIZER`, what
    /// `git_mutex_init` makes on Linux).
    pub const fn new() -> GitMutex {
        GitMutex(UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER))
    }

    /// Lock, blocking (`git_mutex_lock`). Panics if the lock fails (which,
    /// for a default mutex, it does not).
    pub fn lock(&self) -> GitMutexGuard<'_> {
        // SAFETY: the mutex is initialized (constructors) and at a stable
        // address for the borrow.
        let r = unsafe { libc::pthread_mutex_lock(self.0.get()) };
        assert_eq!(r, 0, "pthread_mutex_lock failed");
        GitMutexGuard { m: self }
    }

    /// Lock as C's `git_mutex_lock` does: 0, or the pthread error. Pair it
    /// with [`raw_unlock`](Self::raw_unlock).
    pub fn raw_lock(&self) -> c_int {
        // SAFETY: as in `lock`.
        unsafe { libc::pthread_mutex_lock(self.0.get()) }
    }

    /// `git_mutex_unlock`.
    ///
    /// # Safety
    ///
    /// The calling thread holds the lock, taken with
    /// [`raw_lock`](Self::raw_lock) (or by C), and no guard covers it.
    pub unsafe fn raw_unlock(&self) -> c_int {
        // SAFETY: the caller holds the lock.
        unsafe { libc::pthread_mutex_unlock(self.0.get()) }
    }

    /// The `pthread_mutex_t *` for C.
    pub fn as_ptr(&self) -> *mut libc::pthread_mutex_t {
        self.0.get()
    }

    /// `git_mutex_free`: destroy the mutex.
    ///
    /// # Safety
    ///
    /// The mutex is unlocked, and is not used again unless re-initialized.
    pub unsafe fn destroy(&self) {
        // SAFETY: the caller guarantees it is unlocked and unused after.
        unsafe { libc::pthread_mutex_destroy(self.0.get()) };
    }
}

impl Default for GitMutex {
    fn default() -> Self {
        Self::new()
    }
}

/// A held [`GitMutex`]; unlocks on drop.
#[must_use]
pub struct GitMutexGuard<'a> {
    m: &'a GitMutex,
}

impl GitMutexGuard<'_> {
    /// The mutex this guard holds.
    pub fn mutex(&self) -> &GitMutex {
        self.m
    }
}

impl Drop for GitMutexGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard holds the lock, taken by this thread.
        unsafe { libc::pthread_mutex_unlock(self.m.0.get()) };
    }
}

/// `git_rwlock`: a `pthread_rwlock_t`, mirrored. Do not take a read lock
/// recursively (libgit2's Windows implementation does not support it).
#[repr(transparent)]
pub struct GitRwLock(UnsafeCell<libc::pthread_rwlock_t>);

// SAFETY: as for `GitMutex`.
unsafe impl Sync for GitRwLock {}
// SAFETY: as for `GitMutex`.
unsafe impl Send for GitRwLock {}

impl GitRwLock {
    /// An unlocked lock (`GIT_RWLOCK_STATIC_INIT`).
    pub const fn new() -> GitRwLock {
        GitRwLock(UnsafeCell::new(libc::PTHREAD_RWLOCK_INITIALIZER))
    }

    /// Take a read lock (`git_rwlock_rdlock`).
    pub fn read(&self) -> GitReadGuard<'_> {
        // SAFETY: initialized, at a stable address for the borrow.
        let r = unsafe { libc::pthread_rwlock_rdlock(self.0.get()) };
        assert_eq!(r, 0, "pthread_rwlock_rdlock failed");
        GitReadGuard { l: self }
    }

    /// Take the write lock (`git_rwlock_wrlock`).
    pub fn write(&self) -> GitWriteGuard<'_> {
        // SAFETY: as in `read`.
        let r = unsafe { libc::pthread_rwlock_wrlock(self.0.get()) };
        assert_eq!(r, 0, "pthread_rwlock_wrlock failed");
        GitWriteGuard { l: self }
    }

    /// The `pthread_rwlock_t *` for C.
    pub fn as_ptr(&self) -> *mut libc::pthread_rwlock_t {
        self.0.get()
    }

    /// `git_rwlock_free`.
    ///
    /// # Safety
    ///
    /// The lock is not held, and is not used again unless re-initialized.
    pub unsafe fn destroy(&self) {
        // SAFETY: the caller guarantees it is unlocked and unused after.
        unsafe { libc::pthread_rwlock_destroy(self.0.get()) };
    }
}

impl Default for GitRwLock {
    fn default() -> Self {
        Self::new()
    }
}

/// A held read lock of a [`GitRwLock`].
#[must_use]
pub struct GitReadGuard<'a> {
    l: &'a GitRwLock,
}

impl Drop for GitReadGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard holds a read lock taken by this thread.
        unsafe { libc::pthread_rwlock_unlock(self.l.0.get()) };
    }
}

/// The held write lock of a [`GitRwLock`].
#[must_use]
pub struct GitWriteGuard<'a> {
    l: &'a GitRwLock,
}

impl Drop for GitWriteGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard holds the write lock taken by this thread.
        unsafe { libc::pthread_rwlock_unlock(self.l.0.get()) };
    }
}

/// `git_cond`: a `pthread_cond_t`, mirrored.
#[repr(transparent)]
pub struct GitCond(UnsafeCell<libc::pthread_cond_t>);

// SAFETY: as for `GitMutex`.
unsafe impl Sync for GitCond {}
// SAFETY: as for `GitMutex`.
unsafe impl Send for GitCond {}

impl GitCond {
    /// A condition variable (`git_cond_init`).
    pub const fn new() -> GitCond {
        GitCond(UnsafeCell::new(libc::PTHREAD_COND_INITIALIZER))
    }

    /// Wait (`git_cond_wait`), releasing and retaking `guard`'s mutex.
    /// Wake-ups may be spurious, as with pthreads.
    pub fn wait(&self, guard: &mut GitMutexGuard<'_>) {
        // SAFETY: `guard` proves this thread holds the mutex; both objects
        // are initialized and at stable addresses.
        let r = unsafe { libc::pthread_cond_wait(self.0.get(), guard.m.0.get()) };
        assert_eq!(r, 0, "pthread_cond_wait failed");
    }

    /// `git_cond_signal`.
    pub fn signal(&self) {
        // SAFETY: initialized, at a stable address.
        unsafe { libc::pthread_cond_signal(self.0.get()) };
    }

    /// `git_cond_broadcast`.
    pub fn broadcast(&self) {
        // SAFETY: initialized, at a stable address.
        unsafe { libc::pthread_cond_broadcast(self.0.get()) };
    }

    /// `git_cond_free`.
    ///
    /// # Safety
    ///
    /// No thread waits on it, and it is not used again.
    pub unsafe fn destroy(&self) {
        // SAFETY: the caller guarantees no waiter and no later use.
        unsafe { libc::pthread_cond_destroy(self.0.get()) };
    }
}

impl Default for GitCond {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Atomics
// ---------------------------------------------------------------------------

/// `git_atomic32`: an `int` that C and Rust update atomically
/// (sequentially consistent, as the C's `__atomic` builtins).
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Atomic32(AtomicI32);

impl Atomic32 {
    /// A counter with value `v`.
    pub const fn new(v: i32) -> Atomic32 {
        Atomic32(AtomicI32::new(v))
    }
    /// `git_atomic32_set`.
    #[inline]
    pub fn set(&self, v: i32) {
        self.0.store(v, Ordering::SeqCst)
    }
    /// `git_atomic32_get`.
    #[inline]
    pub fn get(&self) -> i32 {
        self.0.load(Ordering::SeqCst)
    }
    /// `git_atomic32_inc`: the new value.
    #[inline]
    pub fn inc(&self) -> i32 {
        self.0.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }
    /// `git_atomic32_dec`: the new value.
    #[inline]
    pub fn dec(&self) -> i32 {
        self.0.fetch_sub(1, Ordering::SeqCst).wrapping_sub(1)
    }
    /// `git_atomic32_add`: the new value.
    #[inline]
    pub fn add(&self, addend: i32) -> i32 {
        self.0.fetch_add(addend, Ordering::SeqCst).wrapping_add(addend)
    }
}

/// `git_atomic64` (and `git_atomic_ssize` on 64-bit platforms).
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Atomic64(AtomicI64);

impl Atomic64 {
    /// A counter with value `v`.
    pub const fn new(v: i64) -> Atomic64 {
        Atomic64(AtomicI64::new(v))
    }
    /// `git_atomic64_set`.
    #[inline]
    pub fn set(&self, v: i64) {
        self.0.store(v, Ordering::SeqCst)
    }
    /// `git_atomic64_get`.
    #[inline]
    pub fn get(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
    /// `git_atomic64_add`: the new value.
    #[inline]
    pub fn add(&self, addend: i64) -> i64 {
        self.0.fetch_add(addend, Ordering::SeqCst).wrapping_add(addend)
    }
}

/// `git_atomic_ssize`.
pub type AtomicSsize = Atomic64;

/// `git_atomic_compare_and_swap`: replace `*p` with `new` if it is `old`;
/// the previous value either way.
#[inline]
pub fn compare_and_swap<T>(p: &AtomicPtr<T>, old: *mut T, new: *mut T) -> *mut T {
    match p.compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(v) | Err(v) => v,
    }
}

/// `git_atomic_swap`: the previous value.
#[inline]
pub fn swap<T>(p: &AtomicPtr<T>, new: *mut T) -> *mut T {
    p.swap(new, Ordering::SeqCst)
}

/// `git_atomic_load`.
#[inline]
pub fn load<T>(p: &AtomicPtr<T>) -> *mut T {
    p.load(Ordering::SeqCst)
}

/// `git_refcount` (`src/util/util.h`): the count and owner at the start of
/// libgit2's refcounted objects, mirrored.
#[repr(C)]
#[derive(Debug, Default)]
pub struct GitRefcount {
    /// The count.
    pub refcount: Atomic32,
    /// The owning object, or NULL.
    pub owner: AtomicPtr<c_void>,
}

impl GitRefcount {
    /// A count of `n` with no owner.
    pub const fn new(n: i32) -> GitRefcount {
        GitRefcount { refcount: Atomic32::new(n), owner: AtomicPtr::new(core::ptr::null_mut()) }
    }
    /// `GIT_REFCOUNT_INC`.
    #[inline]
    pub fn inc(&self) {
        self.refcount.inc();
    }
    /// `GIT_REFCOUNT_DEC` without the free: decrement, and whether the
    /// object must now be freed (the count reached 0 or less and it has no
    /// owner).
    #[inline]
    #[must_use]
    pub fn dec(&self) -> bool {
        let v = self.refcount.dec();
        v <= 0 && self.owner.load(Ordering::SeqCst).is_null()
    }
    /// `GIT_REFCOUNT_OWN`.
    #[inline]
    pub fn own(&self, owner: *mut c_void) {
        self.owner.swap(owner, Ordering::SeqCst);
    }
    /// `GIT_REFCOUNT_OWNER`.
    #[inline]
    pub fn owner(&self) -> *mut c_void {
        self.owner.load(Ordering::SeqCst)
    }
    /// `GIT_REFCOUNT_VAL`.
    #[inline]
    pub fn val(&self) -> i32 {
        self.refcount.get()
    }
}

// ---------------------------------------------------------------------------
// Thread-local data (thread.c)
// ---------------------------------------------------------------------------

/// `git_tlsdata_key` (`pthread_key_t`).
pub type TlsdataKey = libc::pthread_key_t;

/// The destructor of a thread-local value.
pub type TlsdataDestroyFn = unsafe extern "C" fn(*mut c_void);

/// The exports of `thread.c`: thread-local data over pthread keys. They set
/// no error message on failure (the error state is itself thread-local).
pub mod ffi {
    use super::*;

    /// `git_tlsdata_init`.
    ///
    /// # Safety
    ///
    /// `key` is valid for a write; `destroy_fn` is NULL or sound to call at
    /// thread exit with each value set for the key.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tlsdata_init(key: *mut TlsdataKey, destroy_fn: Option<TlsdataDestroyFn>) -> c_int {
        // SAFETY: the caller passes a writable `key` and vouches for
        // `destroy_fn`.
        if unsafe { libc::pthread_key_create(key, destroy_fn) } != 0 { -1 } else { 0 }
    }

    /// `git_tlsdata_set`.
    ///
    /// # Safety
    ///
    /// `key` was created by `git_tlsdata_init` and not disposed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tlsdata_set(key: TlsdataKey, value: *mut c_void) -> c_int {
        // SAFETY: the caller passes a live key.
        if unsafe { libc::pthread_setspecific(key, value) } != 0 { -1 } else { 0 }
    }

    /// `git_tlsdata_get`.
    ///
    /// # Safety
    ///
    /// `key` was created by `git_tlsdata_init` and not disposed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tlsdata_get(key: TlsdataKey) -> *mut c_void {
        // SAFETY: the caller passes a live key.
        unsafe { libc::pthread_getspecific(key) }
    }

    /// `git_tlsdata_dispose`.
    ///
    /// # Safety
    ///
    /// `key` was created by `git_tlsdata_init` and is not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tlsdata_dispose(key: TlsdataKey) -> c_int {
        // SAFETY: the caller passes a live key, not used after.
        if unsafe { libc::pthread_key_delete(key) } != 0 { -1 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn mutex_and_cond() {
        struct Shared {
            m: GitMutex,
            c: GitCond,
            n: Atomic32,
        }
        let s = Arc::new(Shared { m: GitMutex::new(), c: GitCond::new(), n: Atomic32::new(0) });
        let t = {
            let s = Arc::clone(&s);
            std::thread::spawn(move || {
                let _g = s.m.lock();
                s.n.set(1);
                s.c.signal();
            })
        };
        {
            let mut g = s.m.lock();
            while s.n.get() == 0 {
                s.c.wait(&mut g);
            }
        }
        t.join().unwrap();
        assert_eq!(s.n.get(), 1);
    }

    #[test]
    fn rwlock_and_atomics() {
        let l = GitRwLock::new();
        {
            let _r = l.read();
        }
        {
            let _w = l.write();
        }
        let a = Atomic32::new(5);
        assert_eq!(a.inc(), 6);
        assert_eq!(a.dec(), 5);
        assert_eq!(a.add(-10), -5);
        let b = Atomic64::new(0);
        assert_eq!(b.add(1 << 40), 1 << 40);
        let r = GitRefcount::new(1);
        r.inc();
        assert!(!r.dec());
        assert!(r.dec());
        let mut x = 0u8;
        let p = AtomicPtr::new(core::ptr::null_mut::<u8>());
        assert!(compare_and_swap(&p, core::ptr::null_mut(), &mut x).is_null());
        assert_eq!(load(&p), &mut x as *mut u8);
    }

    #[test]
    fn tlsdata() {
        let mut key: TlsdataKey = 0;
        let mut v = 7u32;
        // SAFETY: a key created here, used and disposed on this thread.
        unsafe {
            assert_eq!(ffi::git_tlsdata_init(&mut key, None), 0);
            assert!(ffi::git_tlsdata_get(key).is_null());
            assert_eq!(ffi::git_tlsdata_set(key, (&raw mut v).cast()), 0);
            assert_eq!(ffi::git_tlsdata_get(key), (&raw mut v).cast());
            assert_eq!(ffi::git_tlsdata_set(key, core::ptr::null_mut()), 0);
            assert_eq!(ffi::git_tlsdata_dispose(key), 0);
        }
    }
}

crate::c_layout_test! {
    headers: ["thread.h", "util.h"],
    types: [
        GitMutex => "git_mutex" {},
        GitRwLock => "git_rwlock" {},
        GitCond => "git_cond" {},
        Atomic32 => "git_atomic32" {},
        Atomic64 => "git_atomic64" {},
        GitRefcount => "git_refcount" { refcount, owner },
        TlsdataKey => "git_tlsdata_key" {},
    ],
}
