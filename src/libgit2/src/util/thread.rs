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

#[expect(
    unsafe_code,
    reason = "lays out git_atomic32 and git_atomic64 by hand, with their handles, lends their \
              values as standard atomics, and reads and writes git_thread's field through its \
              handles' pointers"
)]
mod ffi_import {
    use core::cell::UnsafeCell;
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
                    // `$atomic` (asserted above, the field at offset 0). Every access it may
                    // race with is atomic: the handle's other holders reach the value only
                    // through this, and C writes it only through `thread.h`'s atomic operations
                    // (the module documentation), of the same size. The pointer may be written
                    // through: it comes from C, or from a borrow of a Rust value's `UnsafeCell`.
                    unsafe { <$atomic>::from_ptr(val) }
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
        /// reads and writes it with sequentially consistent operations
        /// ([`SeqCst`](core::sync::atomic::Ordering::SeqCst)) alone.
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
        /// reads and writes it with sequentially consistent operations
        /// ([`SeqCst`](core::sync::atomic::Ordering::SeqCst)) alone.
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
