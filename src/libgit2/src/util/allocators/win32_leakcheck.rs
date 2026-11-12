//! `src/util/allocators/win32_leakcheck.c`: the allocator that records allocations for the
//! Windows CRT's leak checker; off Windows, its initialiser only reports that it is not
//! built. A foundational module: its routines are bound as strategies and stay C.
//!
//! Its one routine, `git_win32_leakcheck_init_allocator`, is an `init` constructor of a
//! `git_allocator`: it fills caller-provided storage, which it never reads, and retains
//! nothing. An [`Allocator`] is a plain value of three function pointers that owns nothing
//! and is never released, so the constructor needs no owner or policy: [`allocator`] has
//! C fill a local and returns it by value, as `alloc.rs` describes for every
//! `git_allocator`.
//!
//! What it builds depends on how libgit2 was configured:
//!
//! - with `GIT_DEBUG_LEAKCHECK_WIN32`, on Windows debug builds, it sets the three members to
//!   wrappers over the Microsoft CRT's debug heap (`_malloc_dbg` and `_realloc_dbg`, which tag each
//!   block with its call site's file and a stack trace for the leak report, and `free`), and
//!   succeeds. No variant of this campaign compiles that branch;
//! - otherwise, as every variant here is built, it writes nothing, sets the thread's last error and
//!   fails: the leak-checking allocator is not built.
//!
//! [`Allocator`]: crate::libgit2::src::util::alloc::Allocator

pub use ffi_import::allocator;

/// What the module reaches in C: `git_win32_leakcheck_init_allocator`.
#[expect(
    unsafe_code,
    reason = "calls git_win32_leakcheck_init_allocator through its raw binding"
)]
mod ffi_import {
    use crate::ffi;
    use crate::libgit2::src::util::alloc::Allocator;
    use crate::libgit2::src::util::errors::{ErrorBuf, ErrorClass, last, runtime_pinned, set};

    /// The allocator that records every allocation with the Windows CRT's leak checker,
    /// or the error saying it is not built.
    ///
    /// On a libgit2 built with `GIT_DEBUG_LEAKCHECK_WIN32` (a Windows debug build), the
    /// allocator's `malloc` and `realloc` go through the Microsoft CRT's debug heap,
    /// `_malloc_dbg` and `_realloc_dbg`, tagging each block with the call site's file
    /// and a stack trace so that the CRT's leak report names where leaked memory was
    /// allocated, and set libgit2's out-of-memory error when they fail; its `free` is the
    /// CRT's `free`. Building it never fails there. Installing it, through
    /// [`set_allocator`], is the step that relies on it, under that function's contract.
    ///
    /// Every other build, every variant of this campaign included, compiles a stub that
    /// writes nothing and only reports that the allocator is not available: there this
    /// function always fails, and leaves libgit2's installed allocator as it is.
    ///
    /// Like every wrapper that can set libgit2's error state, it pins libgit2's runtime
    /// first, initialising libgit2 on the first call.
    ///
    /// ```
    /// use libgit2::libgit2::src::util::allocators::win32_leakcheck::allocator;
    /// use libgit2::libgit2::src::util::errors::ErrorClass;
    ///
    /// // This build has no leak-checking allocator.
    /// let error = allocator().unwrap_err();
    /// assert_eq!(error.class(), Some(ErrorClass::Invalid));
    /// ```
    ///
    /// Wraps `fn::git_win32_leakcheck_init_allocator`
    ///
    /// # Errors
    ///
    /// Fails, with the calling thread's last libgit2 error, on a build without
    /// `GIT_DEBUG_LEAKCHECK_WIN32`: [`ErrorClass::Invalid`], "leakcheck memory allocator
    /// not available"; and when libgit2 cannot be initialised. The C stub reports the
    /// error code `GIT_EINVALID` (-21) where the error class belongs, a class libgit2
    /// names nowhere: this function reports the class `GIT_ERROR_INVALID` instead, and
    /// sets the thread's last error to it with the same message, so that the returned
    /// error and [`last`] agree.
    ///
    /// # Panics
    ///
    /// Panics, with `git_win32_leakcheck_init_allocator sets every member when it
    /// succeeds`, only if libgit2 breaks its contract.
    ///
    /// [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator
    #[doc(alias = "git_win32_leakcheck_init_allocator")]
    pub fn allocator() -> Result<Allocator, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut raw = ffi::git_allocator {
            gmalloc: None,
            grealloc: None,
            gfree: None,
        };
        // SAFETY: `allocator` addresses `raw`, a live `git_allocator` this frame owns
        // exclusively; C only writes its three members (the Windows branch) or leaves it
        // untouched (the stub), and keeps no pointer to it. The runtime is pinned, so the
        // stub's `git_error_set` reaches libgit2's live thread state.
        let rc = unsafe { ffi::git_win32_leakcheck_init_allocator(&raw mut raw) };
        if rc < 0 {
            return Err(reported_error());
        }
        let (Some(malloc), Some(realloc), Some(free)) = (raw.gmalloc, raw.grealloc, raw.gfree)
        else {
            panic!("git_win32_leakcheck_init_allocator sets every member when it succeeds");
        };
        Ok(Allocator::from_fns(malloc, realloc, free))
    }

    /// The error a failed `git_win32_leakcheck_init_allocator` left, its class corrected:
    /// the stub passes the error code `GIT_EINVALID` as `git_error_set`'s class, which is
    /// rewritten to `GIT_ERROR_INVALID`, in the returned copy and in the thread's state.
    fn reported_error() -> ErrorBuf {
        let error = last();
        if error.raw_class() != ffi::GIT_EINVALID {
            return error;
        }
        set(ErrorClass::Invalid, error.message());
        ErrorBuf::new(ErrorClass::Invalid.raw(), error.message())
    }

    #[cfg(test)]
    mod unit_tests {
        use std::ffi::CStr;

        use super::*;
        use crate::libgit2::src::util::errors::{clear, set_str};

        const MESSAGE: &CStr = c"leakcheck memory allocator not available";

        /// The raw C call, on a sentinel value: its return code, what it left in the
        /// value, and the thread's last error.
        fn raw_call() -> (i32, bool, ErrorBuf) {
            assert!(runtime_pinned());
            let mut raw = ffi::git_allocator::default();
            // SAFETY: as in `allocator`: a live, exclusively borrowed `git_allocator`, and
            // the runtime is pinned.
            let rc = unsafe { ffi::git_win32_leakcheck_init_allocator(&raw mut raw) };
            let untouched = raw.gmalloc.is_none() && raw.grealloc.is_none() && raw.gfree.is_none();
            (rc, untouched, last())
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn this_build_has_no_leakcheck_allocator() {
            let error = allocator().unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(error.message(), MESSAGE);
        }

        /// C's stub passes `GIT_EINVALID` as the class: the correction this wrapper makes,
        /// in the returned error and in the thread's last error.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn the_stubs_error_code_class_is_corrected() {
            let (rc, untouched, c_error) = raw_call();
            assert_eq!(rc, ffi::GIT_ERROR);
            assert!(untouched, "the stub writes nothing");
            assert_eq!(c_error.raw_class(), ffi::GIT_EINVALID);
            assert_eq!(c_error.class(), None);
            assert_eq!(c_error.message(), MESSAGE);

            let error = allocator().unwrap_err();
            assert_eq!(error.raw_class(), ErrorClass::Invalid.raw());
            assert_eq!(
                last(),
                error,
                "the thread's last error is the corrected one"
            );
        }

        /// The call replaces whatever error the thread held, cleared or set, and repeats
        /// identically.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn each_call_replaces_the_threads_error() {
            clear();
            let first = allocator().unwrap_err();
            assert_eq!(last(), first);

            set_str(ErrorClass::Odb, c"an older error").unwrap();
            let second = allocator().unwrap_err();
            assert_eq!(second, first);
            assert_eq!(last(), second);
        }

        /// An error left by another call, of another class, is returned as it is.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn other_errors_pass_through_unchanged() {
            set_str(ErrorClass::Odb, c"not the stub's").unwrap();
            let error = reported_error();
            assert_eq!(error.class(), Some(ErrorClass::Odb));
            assert_eq!(error.message(), c"not the stub's");
            assert_eq!(last(), error);
        }
    }
}
