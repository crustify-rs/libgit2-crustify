//! Port of `src/util/runtime.c`: the library's init count, and the registry
//! of shutdown callbacks that subsystems register from their global init.
//!
//! `git_libgit2_init` (in `src/libgit2/libgit2.c`) calls `git_runtime_init`
//! with its list of global init functions; the first call runs them, the
//! last `git_runtime_shutdown` runs the registered shutdown callbacks in the
//! reverse order of registration.
//!
//! The logic lives in [`Runtime`], so that tests can exercise it on a value
//! of their own; the C ABI and [`init`] / [`shutdown`] use the process-wide
//! one.

use core::ffi::c_int;
use std::sync::{Mutex, MutexGuard};

use super::errors::{ErrorClass, Result};

/// A global init function (`git_runtime_init_fn`): 0 on success.
pub type InitFn = unsafe extern "C" fn() -> c_int;

/// A shutdown callback (`git_runtime_shutdown_fn`).
pub type ShutdownFn = unsafe extern "C" fn();

/// How many shutdown callbacks may be registered (C's array size).
pub const MAX_SHUTDOWN_CALLBACKS: usize = 32;

/// An init count and a shutdown registry.
pub struct Runtime {
    /// The init lock and the count it protects (C's `init_mutex` and
    /// `init_count`).
    count: Mutex<c_int>,
    /// The registered callbacks. Its own lock: callbacks are registered
    /// from the init functions, which run under `count`'s lock.
    shutdown: Mutex<Vec<ShutdownFn>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding these locks leaves consistent data.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Runtime {
    /// A runtime with a count of 0 and no callbacks.
    pub const fn new() -> Runtime {
        Runtime {
            count: Mutex::new(0),
            shutdown: Mutex::new(Vec::new()),
        }
    }

    /// `git_runtime_init`: increment the count; on the 0 → 1 transition run
    /// `init_fns` in order, stopping at the first that fails. Returns the
    /// new count, or -1 if an init function failed.
    ///
    /// # Safety
    ///
    /// Each of `init_fns` must be sound to call here, with the runtime's
    /// lock held (as C's init functions are).
    pub unsafe fn init(&self, init_fns: &[InitFn]) -> c_int {
        let mut count = lock(&self.count);
        *count += 1;
        let ret = *count;
        if ret == 1 {
            for f in init_fns {
                // SAFETY: the caller vouches for each init function.
                if unsafe { f() } != 0 {
                    return -1;
                }
            }
        }
        ret
    }

    /// `git_runtime_init_count`.
    pub fn init_count(&self) -> c_int {
        *lock(&self.count)
    }

    /// `git_runtime_shutdown`: decrement the count; on the 1 → 0 transition
    /// run the registered callbacks, last registered first, and forget
    /// them. Returns the new count.
    ///
    /// Calling it more often than `init` makes the count negative, as in C.
    pub fn shutdown(&self) -> c_int {
        let mut count = lock(&self.count);
        *count -= 1;
        if *count == 0 {
            loop {
                let Some(cb) = lock(&self.shutdown).pop() else {
                    break;
                };
                // SAFETY: whoever registered `cb` vouched that it may run at
                // shutdown (`shutdown_register`'s contract).
                unsafe { cb() };
            }
        }
        *count
    }

    /// `git_runtime_shutdown_register`.
    ///
    /// # Safety
    ///
    /// `callback` must be sound to call at the shutdown that runs it, with
    /// the runtime's lock held.
    pub unsafe fn shutdown_register(&self, callback: ShutdownFn) -> Result<()> {
        let mut cbs = lock(&self.shutdown);
        if cbs.len() >= MAX_SHUTDOWN_CALLBACKS {
            drop(cbs);
            return Err(crate::git_error_set!(
                ErrorClass::INVALID,
                "too many shutdown callbacks registered"
            ));
        }
        if cbs.try_reserve(1).is_err() {
            return Err(super::errors::oom());
        }
        cbs.push(callback);
        Ok(())
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

static RUNTIME: Runtime = Runtime::new();

/// Initialize the library's runtime with `init_fns` (see [`Runtime::init`]).
///
/// # Safety
///
/// As [`Runtime::init`].
pub unsafe fn init(init_fns: &[InitFn]) -> c_int {
    // SAFETY: forwarded.
    unsafe { RUNTIME.init(init_fns) }
}

/// The library's init count (`git_runtime_init_count`).
pub fn init_count() -> c_int {
    RUNTIME.init_count()
}

/// Shut the library's runtime down (see [`Runtime::shutdown`]).
pub fn shutdown() -> c_int {
    RUNTIME.shutdown()
}

/// Register a Rust shutdown callback with the library's runtime. A safe
/// `extern "C" fn` is sound to call at any time, so this is safe.
pub fn shutdown_register(callback: extern "C" fn()) -> Result<()> {
    // SAFETY: `callback` is a safe function: calling it is always sound.
    unsafe { RUNTIME.shutdown_register(callback) }
}

/// The exports of `runtime.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, status};

    /// `git_runtime_init`.
    ///
    /// # Safety
    ///
    /// `init_fns` points to `cnt` function pointers (or `cnt` is 0), each
    /// sound to call during initialization.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_runtime_init(
        init_fns: *const Option<InitFn>,
        cnt: usize,
    ) -> c_int {
        barrier(|| {
            let fns: &[Option<InitFn>] = if cnt == 0 || init_fns.is_null() {
                &[]
            } else {
                // SAFETY: the caller passes `cnt` readable entries.
                unsafe { core::slice::from_raw_parts(init_fns, cnt) }
            };
            // A NULL entry would crash the C; treat the list as ending there.
            let fns: Vec<InitFn> = fns.iter().map_while(|f| *f).collect();
            // SAFETY: the caller vouches for each init function.
            unsafe { init(&fns) }
        })
    }

    /// `git_runtime_init_count`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_runtime_init_count() -> c_int {
        barrier(init_count)
    }

    /// `git_runtime_shutdown`.
    ///
    /// # Safety
    ///
    /// The registered callbacks run; each registrant vouched for its own.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_runtime_shutdown() -> c_int {
        barrier(shutdown)
    }

    /// `git_runtime_shutdown_register`.
    ///
    /// # Safety
    ///
    /// `callback` is NULL or sound to call at the last shutdown.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_runtime_shutdown_register(callback: Option<ShutdownFn>) -> c_int {
        barrier(|| {
            // C stores NULL and skips it at shutdown.
            let Some(cb) = callback else { return 0 };
            // SAFETY: the caller vouches for `callback`.
            status(unsafe { RUNTIME.shutdown_register(cb) })
        })
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block)] // each block's SAFETY covers its sequence
mod tests {
    use super::*;
    use crate::util::errors::ErrorCode;
    use core::sync::atomic::{AtomicUsize, Ordering};

    static ORDER: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    static INITS: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn cb1() {
        ORDER.lock().unwrap().push(1);
    }
    extern "C" fn cb2() {
        ORDER.lock().unwrap().push(2);
    }
    unsafe extern "C" fn init_ok() -> c_int {
        INITS.fetch_add(1, Ordering::SeqCst);
        0
    }
    unsafe extern "C" fn init_fail() -> c_int {
        -1
    }

    #[test]
    fn init_runs_once_and_shutdown_runs_lifo() {
        let _g = crate::util::test_support::init();
        let rt = Runtime::new();
        // SAFETY: the test's callbacks are sound to call at any time.
        unsafe {
            assert_eq!(rt.init(&[init_ok]), 1);
            assert_eq!(rt.init(&[init_ok]), 2);
            rt.shutdown_register(cb1).unwrap();
            rt.shutdown_register(cb2).unwrap();
        }
        assert_eq!(INITS.load(Ordering::SeqCst), 1);
        assert_eq!(rt.init_count(), 2);
        assert_eq!(rt.shutdown(), 1);
        assert!(ORDER.lock().unwrap().is_empty());
        assert_eq!(rt.shutdown(), 0);
        assert_eq!(*ORDER.lock().unwrap(), [2, 1]);
        // Callbacks are forgotten after they run.
        assert_eq!(rt.init_count(), 0);
    }

    #[test]
    fn failing_init_returns_minus_one() {
        let _g = crate::util::test_support::init();
        let rt = Runtime::new();
        // SAFETY: the test's init functions are sound to call.
        assert_eq!(unsafe { rt.init(&[init_fail, init_ok]) }, -1);
        assert_eq!(rt.init_count(), 1);
    }

    #[test]
    fn too_many_callbacks() {
        let _g = crate::util::test_support::init();
        let rt = Runtime::new();
        for _ in 0..MAX_SHUTDOWN_CALLBACKS {
            // SAFETY: `cb1` is sound to call at any time.
            unsafe { rt.shutdown_register(cb1).unwrap() };
        }
        // SAFETY: as above.
        let e = unsafe { rt.shutdown_register(cb1) };
        assert_eq!(e, Err(ErrorCode::GENERIC));
        assert_eq!(
            crate::util::errors::last_message().unwrap(),
            b"too many shutdown callbacks registered"
        );
    }
}
