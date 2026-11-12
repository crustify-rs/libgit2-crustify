//! `src/util/alloc.c`, a foundational module: the memory allocator libgit2 runs on
//! (`include/git2/sys/alloc.h`), and the lifecycle strategies through which Rust
//! allocates, copies and releases memory in libgit2's allocator family
//! (`src/util/alloc.h`).
//!
//! # The allocator
//!
//! An [`Allocator`] is a `git_allocator` value: three C function pointers forming one
//! `malloc` / `realloc` / `free` family. `GIT_OPT_SET_ALLOCATOR` copies it into
//! libgit2's hidden global `git__allocator` (`git_allocator_setup` is a `memcpy`), so
//! the value is only borrowed for that call and owns nothing: it is held by value,
//! copied freely, and never released.
//!
//! The header's init routines are not this module's: `git_stdalloc_init_allocator` is
//! `src/util/allocators/stdalloc.c`'s, whose [`stdalloc::allocator`] returns the standard
//! allocator's value, and `git_win32_crtdbg_init_allocator` is never defined on this
//! platform. A value is built here from an [`Alloc`] implementation ([`Allocator::new`]),
//! whose `extern "C"` trampolines adapt libgit2's calls to it, or from three C functions
//! ([`Allocator::from_fns`]).
//!
//! The members are function pointers, so reading or replacing one is safe and calling
//! one is `unsafe`, under the contract its type alias states. Which members make a
//! coherent family is a property of the installed value as a whole, so it is the
//! installing call that relies on it, not these accessors.
//!
//! # The strategies
//!
//! Everything libgit2 allocates, it allocates through `git__malloc` and its siblings
//! over `git__allocator`, and releases with `git__free`. The owners that hold such
//! memory on the Rust side allocate, copy and release it through three policies over
//! that family, so that libgit2 can free what Rust built and Rust what libgit2 built:
//!
//! | policy | owner | allocates | copies | releases |
//! |---|---|---|---|---|
//! | [`Free`] | [`AllocBox<T>`], one `T` | `git__malloc`, `git__calloc` | — | `git__free` |
//! | [`ArrayFree`] | [`AllocVec<T>`], a counted run of `T` | `git__mallocarray`, `git__calloc`, `git__malloc` | `git__malloc` and a copy | `git__free` |
//! | [`StrFree`] | [`AllocStr`], a NUL-terminated string | — | `git__strdup`, `git__strndup`, `git__substrdup` | `git__free` |
//!
//! The primitives are not functions of their own: an allocation is only ever made
//! through an owner that releases it with the routine paired with its allocator.
//!
//! Each strategy calls its primitive through its raw binding: the five functions
//! `alloc.c` defines link from the variant's static archive, although libgit2.so hides
//! them, and `git__malloc` and `git__free`, `static inline` in `alloc.h`, through the
//! wrappers the raw bindings compile for them. The size checks, out-of-memory errors
//! and copies are therefore C's own. `git__mallocarray` is `git__realloc(NULL, n)`, so
//! it reaches the allocator's `grealloc` where the others reach `gmalloc`, and an empty
//! run built by [`ArrayFree::new_uninit`] or [`ArrayFree::new_zeroed`] asks the
//! allocator for zero bytes, as `git__mallocarray(0, ..)` and `git__calloc(0, ..)` do.
//!
//! Every allocation first pins libgit2's runtime, initialising libgit2 once for the
//! rest of the process: before initialisation libgit2's allocator fails every request
//! and reports the failure through thread-local state that does not exist yet. A
//! refused allocation leaves `GIT_ERROR_NOMEMORY` as the thread's last error, as C
//! leaves it, and so does a run whose size overflows, which C refuses before asking
//! the allocator. The strategies add one check of their own: a run whose size passes
//! `isize::MAX` bytes, which no Rust slice may span, is refused before C is asked, with
//! the same error, whichever constructor builds the run: ffibox's own
//! ([`AllocVec::new_uninit`], [`AllocVec::from_fn`]) included, which refuse a size that
//! overflows `usize` without reporting it.
//!
//! Storage is vouched for at the alignment of a `usize` or a `u64`, whichever is wider
//! (8 bytes on 64-bit targets), not `malloc`'s: libgit2 configured with
//! `DEBUG_STRICT_ALLOC` runs on its `debugalloc`, which returns `malloc`'s block offset
//! by a `size_t`. A type aligned beyond it is refused at compile time.
//!
//! An allocation is released through whichever allocator libgit2 holds when it is
//! released, so replacing the allocator while an owner lives is
//! [`set_allocator`]'s caller's obligation.
//!
//! A run is resized through the family's `git__reallocarray` ([`ArrayFree::resize`]) and
//! `git__realloc` ([`ArrayFree::resize_bytes`]), which keep its prefix and leave it as it
//! was when the allocator refuses. xdiff's allocation macros (`xdl_malloc`, `xdl_calloc`,
//! `xdl_realloc`, `xdl_free`, `deps/xdiff/git-xdiff.h`) are aliases of the family's
//! primitives, and stand in Rust as the strategies over those primitives.
//!
//! # The installed allocator
//!
//! Every allocation reads libgit2's allocator from a hidden global, `git__allocator`, with
//! no synchronisation. It starts as the failing family
//! ([`failalloc`](crate::libgit2::src::util::allocators::failalloc)), which
//! `git_libgit2_init` replaces with the standard one ([`global_init`]) unless another was
//! installed first; [`set_allocator`] (`GIT_OPT_SET_ALLOCATOR`, `git_allocator_setup`)
//! copies a value over it. Each write races every read on another thread, so no safe path
//! writes it but libgit2's first initialisation, which runs under libgit2's own lock:
//! [`installed`], the borrow of it, is safe because it pins libgit2's runtime first, after
//! which only the `unsafe` installers write the global, and their callers keep every other
//! thread from reading it meanwhile.
//!
//! [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator
//! [`stdalloc::allocator`]: crate::libgit2::src::util::allocators::stdalloc::allocator

pub use ffi_import::{
    Alloc, AllocBox, AllocStr, AllocVec, Allocator, AllocatorMut, AllocatorRef, ArrayFree, Free,
    FreeFn, MallocFn, ReallocFn, StrFree, global_init, installed,
};

/// What the module reaches in C: `git_allocator`'s layout and trampolines, and the
/// strategies over libgit2's allocator family.
mod ffi_import {
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::fmt;
    use core::marker::PhantomData;
    use core::mem::{self, MaybeUninit, align_of, size_of};
    use core::ptr::{self, NonNull};

    use ffibox::callback::barrier;
    use ffibox::{
        CAlloc, CAllocZeroed, CBox, CCell, CDrop, CDupClone, CLenAlloc, CLenClone, CLenDrop,
        CMutSeam, CPlainElem, CRefSeam, CStrBox, CVec, CZeroable, define_ctype,
    };

    use crate::ffi;
    use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned, set_oom};

    /// `git_allocator.gmalloc`: allocate `n` bytes for the caller at `file`:`line`.
    ///
    /// Returns a fresh allocation of at least `n` bytes, or NULL on failure. A zero-byte
    /// request is as C's `malloc(0)`: NULL, or an allocation of its own that `gfree`
    /// releases; libgit2 reads NULL as out of memory.
    ///
    /// # Safety
    ///
    /// `file` must be NULL or a NUL-terminated string live for the call (libgit2 passes
    /// `__FILE__`). A function may add obligations of its own; the trampolines
    /// [`Allocator::new`] stores add none.
    ///
    /// The C form of the member; its Rust form is [`Alloc::malloc`].
    ///
    /// Wraps `fnp::git_allocator.gmalloc`
    #[doc(alias = "git_allocator.gmalloc")]
    pub type MallocFn =
        unsafe extern "C" fn(n: usize, file: *const c_char, line: c_int) -> *mut c_void;

    /// `git_allocator.grealloc`: resize the allocation at `ptr` to `size` bytes for the
    /// caller at `file`:`line`, as C's `realloc`.
    ///
    /// NULL `ptr` allocates afresh, as [`MallocFn`] does. On success the old allocation is
    /// released and a new one holding its first `min(old, size)` bytes is returned; on
    /// failure NULL is returned and `ptr` stays live.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or a live allocation of the same allocator family, which the
    /// caller owns and, on success, gives up; `file` as for [`MallocFn`].
    ///
    /// `size` must be non-zero when `ptr` is not NULL. C leaves a zero-byte resize of a
    /// live allocation implementation-defined: glibc releases `ptr` and returns NULL, which
    /// this contract would read as a failure that left `ptr` live. No libgit2 code path
    /// makes one, so a caller must not either. A function may relax that: the trampolines
    /// [`Allocator::new`] stores accept a zero-byte resize of a live allocation and refuse
    /// it, returning NULL with `ptr` left live, so that [`Alloc::realloc`] never sees one,
    /// whatever C passes.
    ///
    /// The C form of the member; its Rust form is [`Alloc::realloc`].
    ///
    /// Wraps `fnp::git_allocator.grealloc`
    #[doc(alias = "git_allocator.grealloc")]
    pub type ReallocFn = unsafe extern "C" fn(
        ptr: *mut c_void,
        size: usize,
        file: *const c_char,
        line: c_int,
    ) -> *mut c_void;

    /// `git_allocator.gfree`: release the allocation at `ptr`; NULL is a no-op.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or a live allocation of the same allocator family, which the
    /// caller owns and does not use again.
    ///
    /// The C form of the member; its Rust form is [`Alloc::free`].
    ///
    /// Wraps `fnp::git_allocator.gfree`
    #[doc(alias = "git_allocator.gfree")]
    pub type FreeFn = unsafe extern "C" fn(ptr: *mut c_void);

    define_ctype!(
        /// A custom memory allocator for libgit2: its `malloc`, `realloc` and `free`
        /// (`git_allocator`).
        ///
        /// A plain value of three function pointers, resource-free: `Copy`, held inline,
        /// never released. Build one with [`new`](Self::new) over an [`Alloc`]
        /// implementation, or with [`from_fns`](Self::from_fns) over C functions, or take
        /// libgit2's standard one from [`stdalloc::allocator`] or its debug one from
        /// [`debug`](Self::debug); each sets every member, and the setters only ever store a
        /// function, so a value built in Rust never holds a NULL member. Read and replace
        /// members through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut).
        ///
        /// [`stdalloc::allocator`]: crate::libgit2::src::util::allocators::stdalloc::allocator
        ///
        /// Wraps `st::git_allocator`
        #[doc(alias = "git_allocator")]
        #[derive(Clone, Copy)]
        Allocator,
        AllocatorRef,
        AllocatorMut,
        ffi::git_allocator
    );

    // SAFETY: an `Allocator` is three function pointers and nothing else: no interior
    // mutability, no thread affinity, nothing to release. Moving one to another thread
    // moves three code addresses; calling them stays `unsafe`, under their own contract.
    unsafe impl Send for Allocator {}

    // SAFETY: `AllocatorMut` is an exclusive borrow of such a value, as `&mut Allocator`.
    unsafe impl Send for AllocatorMut<'_> {}

    // SAFETY: shared access only reads the three pointers (every write goes through
    // `AllocatorMut`, which excludes other handles), so concurrent readers race nothing.
    unsafe impl Sync for Allocator {}

    // SAFETY: as above; `AllocatorRef` is a shared borrow, as `&Allocator`.
    unsafe impl Send for AllocatorRef<'_> {}

    // SAFETY: as above.
    unsafe impl Sync for AllocatorRef<'_> {}

    // SAFETY: `&AllocatorMut` only reaches the getters, through `as_ref`.
    unsafe impl Sync for AllocatorMut<'_> {}

    impl Allocator {
        /// The allocator over three C functions forming one family.
        ///
        /// Building the value is safe: nothing is called until it is installed, and
        /// installing it is what relies on `malloc`, `realloc` and `free` belonging
        /// together.
        pub const fn from_fns(malloc: MallocFn, realloc: ReallocFn, free: FreeFn) -> Self {
            Self(
                ffi::git_allocator {
                    gmalloc: Some(malloc),
                    grealloc: Some(realloc),
                    gfree: Some(free),
                },
                PhantomData,
            )
        }

        /// The allocator that forwards libgit2's calls to `A`.
        ///
        /// Its members are `extern "C"` trampolines generic over `A`: they turn the call
        /// site's `file` into an `Option<&CStr>`, pointers into `Option<NonNull<u8>>`,
        /// drop `free(NULL)` before it reaches [`Alloc::free`], and refuse a zero-byte
        /// resize of a live allocation before it reaches [`Alloc::realloc`], returning NULL
        /// with the allocation left live, whatever C passes. Each is a panic
        /// barrier, since a panic cannot unwind into C: a panic in [`Alloc::malloc`] or
        /// [`Alloc::realloc`] returns NULL, C's out-of-memory convention, and one in
        /// [`Alloc::free`], which C gives no way to fail, aborts the process.
        ///
        /// ```
        /// use core::ffi::{CStr, c_void};
        /// use core::ptr::NonNull;
        ///
        /// use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
        ///
        /// unsafe extern "C" {
        ///     fn malloc(n: usize) -> *mut c_void;
        ///     fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        ///     fn free(p: *mut c_void);
        /// }
        ///
        /// /// libc's allocator, which meets `Alloc`'s contract as C's `malloc` family does
        /// /// (the trampolines never ask it to resize a live allocation to zero bytes,
        /// /// where glibc would release it and return NULL).
        /// struct Libc;
        ///
        /// // SAFETY: forwards to libc's `malloc` family, unchanged.
        /// unsafe impl Alloc for Libc {
        ///     fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        ///         NonNull::new(unsafe { malloc(len) }.cast())
        ///     }
        ///
        ///     unsafe fn realloc(
        ///         p: Option<NonNull<u8>>,
        ///         len: usize,
        ///         _: Option<&CStr>,
        ///         _: i32,
        ///     ) -> Option<NonNull<u8>> {
        ///         let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
        ///         NonNull::new(unsafe { realloc(p, len) }.cast())
        ///     }
        ///
        ///     unsafe fn free(p: NonNull<u8>) {
        ///         unsafe { free(p.as_ptr().cast()) }
        ///     }
        /// }
        ///
        /// static LIBC: Allocator = Allocator::new::<Libc>();
        /// assert!(LIBC.as_ref().malloc().is_some());
        /// ```
        ///
        /// An implementation that binds one of libgit2's own C families names its routines
        /// in [`Alloc::C_ALLOCATOR`], and the value is those routines themselves, with C's
        /// addresses, which libgit2 compares, rather than trampolines.
        pub const fn new<A: Alloc>() -> Self {
            match A::C_ALLOCATOR {
                Some(family) => family,
                None => Self::from_fns(malloc_with::<A>, realloc_with::<A>, free_with::<A>),
            }
        }
    }

    impl fmt::Debug for Allocator {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.as_ref().fmt(f)
        }
    }

    impl AllocatorRef<'_> {
        /// The `malloc` member; `None` only in a value C left unset, never in one built
        /// here.
        ///
        /// Wraps `fld::git_allocator.gmalloc`
        #[doc(alias = "git_allocator.gmalloc")]
        #[must_use]
        pub fn malloc(&self) -> Option<MallocFn> {
            // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
            // forms no reference to it.
            let gmalloc_ptr = unsafe { &raw const (*self.as_ptr()).gmalloc };
            // SAFETY: a read through the shared handle's pointer to a live `git_allocator`;
            // every bit pattern of the member, NULL included, is a valid `Option<fn>`.
            unsafe { gmalloc_ptr.read() }
        }

        /// The `realloc` member; `None` only in a value C left unset.
        ///
        /// Wraps `fld::git_allocator.grealloc`
        #[doc(alias = "git_allocator.grealloc")]
        #[must_use]
        pub fn realloc(&self) -> Option<ReallocFn> {
            // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
            // forms no reference to it.
            let grealloc_ptr = unsafe { &raw const (*self.as_ptr()).grealloc };
            // SAFETY: as in `malloc`.
            unsafe { grealloc_ptr.read() }
        }

        /// The `free` member; `None` only in a value C left unset.
        ///
        /// Wraps `fld::git_allocator.gfree`
        #[doc(alias = "git_allocator.gfree")]
        #[must_use]
        pub fn free(&self) -> Option<FreeFn> {
            // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
            // forms no reference to it.
            let gfree_ptr = unsafe { &raw const (*self.as_ptr()).gfree };
            // SAFETY: as in `malloc`.
            unsafe { gfree_ptr.read() }
        }
    }

    impl fmt::Debug for AllocatorRef<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let addr = |p: Option<*const ()>| p.unwrap_or(ptr::null());
            f.debug_struct("Allocator")
                .field("malloc", &addr(self.malloc().map(|p| p as *const ())))
                .field("realloc", &addr(self.realloc().map(|p| p as *const ())))
                .field("free", &addr(self.free().map(|p| p as *const ())))
                .finish()
        }
    }

    impl AllocatorMut<'_> {
        /// Replaces the `malloc` member.
        ///
        /// Wraps `fld::git_allocator.gmalloc`
        #[doc(alias = "git_allocator.gmalloc")]
        pub fn set_malloc(&mut self, malloc: MallocFn) {
            // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
            // projection forms no reference to it.
            let gmalloc_ptr = unsafe { &raw mut (*self.as_mut_ptr()).gmalloc };
            // SAFETY: a write through the exclusive handle's pointer to a live
            // `git_allocator`; the member is plain data with nothing to release.
            unsafe { gmalloc_ptr.write(Some(malloc)) }
        }

        /// Replaces the `realloc` member.
        ///
        /// Wraps `fld::git_allocator.grealloc`
        #[doc(alias = "git_allocator.grealloc")]
        pub fn set_realloc(&mut self, realloc: ReallocFn) {
            // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
            // projection forms no reference to it.
            let grealloc_ptr = unsafe { &raw mut (*self.as_mut_ptr()).grealloc };
            // SAFETY: as in `set_malloc`.
            unsafe { grealloc_ptr.write(Some(realloc)) }
        }

        /// Replaces the `free` member.
        ///
        /// Wraps `fld::git_allocator.gfree`
        #[doc(alias = "git_allocator.gfree")]
        pub fn set_free(&mut self, free: FreeFn) {
            // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
            // projection forms no reference to it.
            let gfree_ptr = unsafe { &raw mut (*self.as_mut_ptr()).gfree };
            // SAFETY: as in `set_malloc`.
            unsafe { gfree_ptr.write(Some(free)) }
        }
    }

    /// A memory allocator libgit2 can run on, adapted to `git_allocator`'s members by
    /// [`Allocator::new`]: libgit2's counterpart of [`GlobalAlloc`](std::alloc::GlobalAlloc),
    /// without layouts, since C passes only sizes.
    ///
    /// `git_allocator` carries no state, so an implementation is a type whose associated
    /// functions reach any state they need through statics. `file` and `line` name the
    /// libgit2 call site; `file` is borrowed for the call only.
    ///
    /// # Safety
    ///
    /// libgit2 trusts these functions as it trusts C's `malloc`, `realloc` and `free`:
    ///
    /// - `malloc(len, ..)` returns `None`, or the start of a fresh allocation of at least `len`
    ///   bytes, overlapping no other live allocation of the implementation, and aligned to the
    ///   largest power of two not above `len`, up to the alignment of a `usize` or a `u64`,
    ///   whichever is wider (8 bytes on 64-bit targets). That is what libgit2's C and this crate's
    ///   owners rely on, not `malloc`'s `max_align_t`: libgit2's own debug allocator
    ///   ([`Allocator::debug`]) keeps no more. `len` may be 0, as for C's `malloc(0)`: the result
    ///   is `None` (which libgit2 reads as out of memory) or an allocation of its own, which `free`
    ///   releases like any other;
    /// - `realloc(None, len, ..)` behaves as `malloc`. `realloc(Some(ptr), len, ..)`, with `ptr` a
    ///   live allocation of the implementation and `len` non-zero, either returns `None` and leaves
    ///   `ptr` live and unchanged, or releases `ptr` and returns an allocation as `malloc` would
    ///   whose first `min(old, len)` bytes are those `ptr` held (it may be `ptr` itself). It is
    ///   never called with `Some(ptr)` and `len == 0`, which C leaves implementation-defined (see
    ///   [`ReallocFn`]): the trampoline refuses that request before the method is reached, so an
    ///   implementation need not decide that case;
    /// - `free(ptr)` releases `ptr`, a live allocation of the implementation;
    /// - all three may run concurrently on any threads, and an allocation may be resized or
    ///   released on a thread other than the one that made it;
    /// - a panic out of `malloc` or `realloc` stands for a `None` return, which the trampoline
    ///   returns to C in its place: `realloc` leaves `ptr` live and unchanged when it panics, as
    ///   when it fails. A panic out of `free` aborts the process;
    /// - [`C_ALLOCATOR`](Self::C_ALLOCATOR), when `Some`, holds one family of C functions, each
    ///   meeting this contract as the method of its name does, over the methods' own storage: what
    ///   either allocates, the other resizes and releases. C calls them with no trampoline in
    ///   front, so its `realloc` also answers a zero-byte resize of a live allocation, by returning
    ///   NULL and leaving it live (as `git_failalloc_realloc` does), or by releasing it and
    ///   returning an allocation as `malloc(0)` would; never by releasing it and returning NULL.
    ///
    /// None of them, the methods or `C_ALLOCATOR`'s functions, may allocate through libgit2 or
    /// touch its state: libgit2 allocates through them, its per-thread error state included
    /// (`threadstate_get`), so such a call from inside one recurses without bound. A routine of
    /// libgit2's own that does neither may be called, such as its failing family's
    /// (`git_failalloc_*`), which is what `C_ALLOCATOR` holds for that family.
    pub unsafe trait Alloc {
        /// Allocates at least `len` bytes; `None` on failure.
        ///
        /// The Rust form of the member, which C reaches through a trampoline; its C form
        /// is [`MallocFn`].
        ///
        /// Wraps `fnp::git_allocator.gmalloc`
        #[doc(alias = "git_allocator.gmalloc")]
        fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>>;

        /// Resizes `ptr` to `len` bytes, or allocates afresh for `None`; `None` on
        /// failure, leaving `ptr` live.
        ///
        /// The Rust form of the member; its C form is [`ReallocFn`].
        ///
        /// Wraps `fnp::git_allocator.grealloc`
        ///
        /// # Safety
        ///
        /// `ptr` is `None` or a live allocation of this implementation that the caller
        /// owns and, on success, gives up; `len` is non-zero when `ptr` is `Some`.
        #[doc(alias = "git_allocator.grealloc")]
        unsafe fn realloc(
            ptr: Option<NonNull<u8>>,
            len: usize,
            file: Option<&CStr>,
            line: i32,
        ) -> Option<NonNull<u8>>;

        /// Releases `ptr`.
        ///
        /// The Rust form of the member; its C form is [`FreeFn`].
        ///
        /// Wraps `fnp::git_allocator.gfree`
        ///
        /// # Safety
        ///
        /// `ptr` is a live allocation of this implementation that the caller owns and does
        /// not use again.
        #[doc(alias = "git_allocator.gfree")]
        unsafe fn free(ptr: NonNull<u8>);

        /// The C functions of the family, when the implementation binds one of libgit2's own
        /// allocator families: [`Allocator::new`] stores them in place of its trampolines.
        ///
        /// libgit2 compares a family's members by address (`git_allocator_global_init` keeps
        /// whatever allocator is installed unless its `gmalloc` is `git_failalloc_malloc`),
        /// and a trampoline never has a C function's address, so a value built over the
        /// binding of a C family holds that family's own functions. `None`, the default, for
        /// every other implementation.
        const C_ALLOCATOR: Option<Allocator> = None;
    }

    /// The call site's file name, as C passes it.
    ///
    /// # Safety
    ///
    /// `file` is NULL or a NUL-terminated string live for `'a`.
    unsafe fn site_file<'a>(file: *const c_char) -> Option<&'a CStr> {
        // SAFETY: per this function's contract, a non-NULL `file` is a live C string.
        (!file.is_null()).then(|| unsafe { CStr::from_ptr(file) })
    }

    /// `gmalloc` over [`Alloc::malloc`]; a panic in it returns NULL, C's out-of-memory
    /// convention.
    ///
    /// # Safety
    ///
    /// As [`MallocFn`].
    unsafe extern "C" fn malloc_with<A: Alloc>(
        len: usize,
        file: *const c_char,
        line: c_int,
    ) -> *mut c_void {
        // SAFETY: `MallocFn`'s contract: `file` is NULL or a C string live for the call.
        let file = unsafe { site_file(file) };
        barrier(
            || A::malloc(len, file, line).map_or(ptr::null_mut(), |p| p.as_ptr().cast()),
            ptr::null_mut,
        )
    }

    /// `grealloc` over [`Alloc::realloc`]; a panic in it returns NULL, which leaves `ptr`
    /// live, as [`Alloc`]'s contract has a panicking `realloc` leave it.
    ///
    /// A zero-byte resize of a live allocation, which C leaves implementation-defined and
    /// [`Alloc::realloc`] is never asked for, is refused here, before `A` is called: NULL,
    /// `ptr` left live and unchanged, as for any failed resize. C, not a Rust caller, chooses
    /// the arguments, so the rule is kept in front of it rather than asked of it.
    ///
    /// # Safety
    ///
    /// As [`ReallocFn`], but for its zero-size rule, which this trampoline does not need:
    /// this trampoline's family is `A`'s.
    unsafe extern "C" fn realloc_with<A: Alloc>(
        ptr: *mut c_void,
        size: usize,
        file: *const c_char,
        line: c_int,
    ) -> *mut c_void {
        if size == 0 && !ptr.is_null() {
            // A zero-byte resize of a live allocation: refused, `ptr` left as it was.
            return ptr::null_mut();
        }
        // SAFETY: as in `malloc_with`.
        let file = unsafe { site_file(file) };
        let resize = || {
            // SAFETY: `ReallocFn`'s contract: `ptr` is NULL or a live allocation of this
            // family, i.e. one `A` made, which the caller gives up on success; `size` is
            // non-zero when `ptr` is not NULL, since the zero-byte resize of a live one was
            // refused above.
            let resized = unsafe { A::realloc(NonNull::new(ptr.cast()), size, file, line) };
            resized.map_or(ptr::null_mut(), |p| p.as_ptr().cast())
        };
        barrier(resize, ptr::null_mut)
    }

    /// `gfree` over [`Alloc::free`]; `free(NULL)` stops here. A panic in it aborts the
    /// process: C's `free` cannot fail, so there is no convention to map it to.
    ///
    /// # Safety
    ///
    /// As [`FreeFn`]: this trampoline's family is `A`'s.
    unsafe extern "C" fn free_with<A: Alloc>(ptr: *mut c_void) {
        if let Some(ptr) = NonNull::new(ptr.cast()) {
            // SAFETY: `FreeFn`'s contract: a non-NULL `ptr` is a live allocation `A` made,
            // which the caller gives up.
            unsafe { A::free(ptr) }
        }
    }

    // An `Allocator` is a `git_allocator`, so C reads a value Rust built in place.
    const _: () = assert!(
        size_of::<Allocator>() == size_of::<ffi::git_allocator>()
            && align_of::<Allocator>() == align_of::<ffi::git_allocator>()
    );

    // --- The strategies over libgit2's allocator family ---------------------------------

    /// The alignment every allocation this module asks for at least this many bytes is
    /// guaranteed: that of a `usize` or a `u64`, whichever is wider.
    ///
    /// Not `malloc`'s (`max_align_t`, 16 bytes on x86-64): libgit2 configured with
    /// `DEBUG_STRICT_ALLOC` installs `debugalloc` as its default allocator, which prefixes
    /// each `malloc` block with its `size_t` length and returns the address past it, so it
    /// keeps only `size_t`'s alignment. libgit2 itself places structs of pointers, `size_t`s
    /// and 64-bit integers in that storage, and its CI runs under `debugalloc`, so every
    /// allocator libgit2 runs on keeps this much, and [`Alloc`]'s contract asks this much of
    /// one installed from Rust.
    const MAX_ALIGN: usize = {
        let (word, wide) = (align_of::<usize>(), align_of::<u64>());
        if word > wide { word } else { wide }
    };

    /// A run's pointer for C to read `bytes.len()` bytes through: an empty run's is a live
    /// empty string, since C may not be handed a dangling pointer, even to read nothing.
    fn run_ptr(bytes: &[u8]) -> *const c_char {
        if bytes.is_empty() {
            c"".as_ptr()
        } else {
            bytes.as_ptr().cast()
        }
    }

    /// The allocation `alloc` makes in C once libgit2's runtime is pinned; `None` when
    /// libgit2 cannot be initialised, or when `alloc` returns NULL.
    ///
    /// Before initialisation libgit2's allocator refuses every request and reports the
    /// refusal through thread-local state that does not exist yet; once pinned, the
    /// allocator is initialised and a refusal is reported on live thread-local state.
    fn pinned<T>(alloc: impl FnOnce() -> *mut T) -> Option<NonNull<T>> {
        if !runtime_pinned() {
            return None;
        }
        NonNull::new(alloc())
    }

    /// `git__malloc(len)`: a fresh allocation of at least `len` bytes; `None` when
    /// libgit2 cannot be initialised, or when the allocator refuses, which leaves
    /// `GIT_ERROR_NOMEMORY` as the thread's last error.
    ///
    /// The allocation is aligned to the largest power of two not above `len`, up to
    /// [`MAX_ALIGN`].
    fn malloc(len: usize) -> Option<NonNull<u8>> {
        // SAFETY: any `len` is accepted; `pinned` calls it on a pinned runtime.
        pinned(|| unsafe { ffi::git__malloc(len) }).map(NonNull::cast)
    }

    /// `git__free(data)`.
    ///
    /// # Safety
    ///
    /// `data` is a live allocation of libgit2's current allocator, which the caller owns
    /// and does not use again.
    unsafe fn free(data: NonNull<u8>) {
        // SAFETY: `data` is a live allocation of the family the caller gives up, per this
        // function's contract; `git__free` releases it without reading it.
        unsafe { ffi::git__free(data.as_ptr().cast()) }
    }

    /// `git__calloc(nelem, elsize)`: zeroed storage of `nelem * elsize` bytes; `None` as
    /// for [`malloc`], or when the size overflows, which C reports as out of memory.
    fn calloc(nelem: usize, elsize: usize) -> Option<NonNull<u8>> {
        // SAFETY: any sizes are accepted, an overflowing product refused; `pinned` calls
        // it on a pinned runtime.
        pinned(|| unsafe { ffi::git__calloc(nelem, elsize) }).map(NonNull::cast)
    }

    /// `git__mallocarray(nelem, elsize)`: uninitialised storage of `nelem * elsize`
    /// bytes, through the allocator's `grealloc`; `None` as for [`calloc`].
    fn mallocarray(nelem: usize, elsize: usize) -> Option<NonNull<u8>> {
        // SAFETY: as in `calloc`; `git__realloc(NULL, n)` resizes no live allocation.
        pinned(|| unsafe { ffi::git__mallocarray(nelem, elsize) }).map(NonNull::cast)
    }

    /// `git__realloc(data, size)`: `data`'s first `min(old, size)` bytes at an address that
    /// may be new, or a fresh allocation for NULL; `None` as for [`malloc`], `data` then
    /// left live.
    ///
    /// # Safety
    ///
    /// `data` is NULL or a live allocation of libgit2's current allocator, which the caller
    /// owns and, on success, gives up; `size` is non-zero, as [`ReallocFn`] asks.
    unsafe fn realloc(data: *mut u8, size: usize) -> Option<NonNull<u8>> {
        // SAFETY: per this function's contract, on a pinned runtime.
        pinned(|| unsafe { ffi::git__realloc(data.cast(), size) }).map(NonNull::cast)
    }

    /// `git__reallocarray(data, nelem, elsize)`: [`realloc`] to `nelem * elsize` bytes;
    /// `None` as for [`calloc`], `data` then left live.
    ///
    /// # Safety
    ///
    /// As [`realloc`], with `nelem` and `elsize` non-zero: C refuses an overflowing product
    /// before it asks the allocator, and passes it any other.
    unsafe fn reallocarray(data: *mut u8, nelem: usize, elsize: usize) -> Option<NonNull<u8>> {
        // SAFETY: per this function's contract, on a pinned runtime.
        pinned(|| unsafe { ffi::git__reallocarray(data.cast(), nelem, elsize) }).map(NonNull::cast)
    }

    /// `git__strdup(s)`: a copy of `s` and its terminator; `None` as for [`malloc`].
    ///
    /// # Safety
    ///
    /// `s` is a NUL-terminated string that stays live and unchanged for the call.
    unsafe fn strdup(s: NonNull<c_char>) -> Option<NonNull<c_char>> {
        // SAFETY: per this function's contract; C reads `s` up to its terminator, on a
        // pinned runtime.
        pinned(|| unsafe { ffi::git__strdup(s.as_ptr()) })
    }

    /// `git__strndup(bytes, bytes.len())`: a copy of `bytes` up to their first NUL, and
    /// a terminator; `None` as for [`malloc`].
    fn strndup(bytes: &[u8]) -> Option<NonNull<c_char>> {
        // SAFETY: C reads at most `bytes.len()` bytes through the pointer
        // (`p_strnlen`), all of them in `bytes`, on a pinned runtime.
        pinned(|| unsafe { ffi::git__strndup(run_ptr(bytes), bytes.len()) })
    }

    /// `git__substrdup(bytes, bytes.len())`: a copy of all of `bytes` and a terminator;
    /// `None` as for [`malloc`].
    fn substrdup(bytes: &[u8]) -> Option<NonNull<c_char>> {
        // SAFETY: C copies exactly `bytes.len()` bytes, all of them in `bytes`, into
        // `git__malloc(n + 1)`, which cannot overflow since a slice spans at most
        // `isize::MAX` bytes; on a pinned runtime.
        pinned(|| unsafe { ffi::git__substrdup(run_ptr(bytes), bytes.len()) })
    }

    /// The policy of one object in libgit2's allocator: allocated by `git__malloc` or
    /// `git__calloc`, released by `git__free`.
    ///
    /// A storage policy: its free never reads the object, so it settles storage both
    /// before and after it holds a `T` ([`CBox::new_uninit`], [`CBox::new_zeroed`]), and
    /// releases nothing the object owns. A `T` owning resources is promoted to a fuller
    /// destructor with `with_policy` before it acquires them.
    ///
    /// A type aligned beyond what the allocator vouches for, 16 bytes included, is rejected:
    ///
    /// ```compile_fail,E0080
    /// use libgit2::libgit2::src::util::alloc::AllocBox;
    ///
    /// #[repr(C, align(16))]
    /// pub struct wide_st { pub x: u8 }
    /// ffibox::define_ctype!(Wide, WideRef, WideMut, wide_st);
    ///
    /// let _ = AllocBox::<Wide>::new_uninit();
    /// ```
    ///
    /// Zeroed storage needs a type that vouches all-zero is valid:
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::src::util::alloc::AllocBox;
    ///
    /// #[repr(C)]
    /// pub struct frame_st { pub format: i32 }
    /// ffibox::define_ctype!(Frame, FrameRef, FrameMut, frame_st); // not `CZeroable`
    ///
    /// let _ = AllocBox::<Frame>::new_zeroed();
    /// ```
    ///
    /// A handle borrows its box, so it cannot outlive it:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::libgit2::oid::Oid;
    /// use libgit2::libgit2::src::util::alloc::AllocBox;
    ///
    /// let id = AllocBox::<Oid>::new_zeroed().unwrap();
    /// let handle = id.as_ref();
    /// drop(id);
    /// let _ = handle.id();
    /// ```
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Free;

    /// One `T` in libgit2's allocator, released by `git__free`.
    pub type AllocBox<T> = CBox<T, Free>;

    // SAFETY: `free` releases the allocation exactly once and never reads it, so it
    // settles any `T`, initialised or not; every owner under this policy holds storage of
    // libgit2's allocator, which only `c_alloc` and `c_alloc_zeroed` below, or an `unsafe`
    // adoption vouching for the same, put in one. `self` carries no state.
    unsafe impl<T> CDrop<T> for Free {
        /// Wraps `fn::git__free`
        /// Wraps `mac::xdl_free`
        #[doc(alias = "git__free", alias = "xdl_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<T>) {
            // SAFETY: per `c_drop`'s contract, `ptr` is a live allocation of this policy's
            // family whose claim the caller gives up.
            unsafe { free(ptr.cast()) }
        }
    }

    // SAFETY: `c_alloc` returns fresh, uniquely owned storage of `size_of::<T>()` bytes,
    // which `T`'s size (a non-zero multiple of its alignment, both checked at compile time)
    // makes aligned for `T`; `c_drop` above releases it without reading it.
    unsafe impl<T: CCell> CAlloc<T> for Free {
        /// Wraps `fn::git__malloc`
        /// Wraps `mac::xdl_malloc`
        #[doc(alias = "git__malloc", alias = "xdl_malloc")]
        #[inline]
        fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
            const {
                assert!(size_of::<T>() != 0, "a zero-sized T needs no storage");
                assert!(
                    align_of::<T>() <= MAX_ALIGN,
                    "libgit2's allocator cannot align T"
                );
            };
            malloc(size_of::<T>()).map(NonNull::cast)
        }
    }

    // SAFETY: as `CAlloc`, with every byte of the `T` zeroed before it is handed out
    // (`git__calloc`'s `memset`), a state `T: CZeroable` vouches for; `c_drop` accepts any
    // `T` since it never reads it.
    unsafe impl<T: CZeroable> CAllocZeroed<T> for Free {
        /// Wraps `fn::git__calloc`
        /// Wraps `mac::xdl_calloc`
        #[doc(alias = "git__calloc", alias = "xdl_calloc")]
        #[inline]
        fn c_alloc_zeroed(&self) -> Option<NonNull<T>> {
            const {
                assert!(size_of::<T>() != 0, "a zero-sized T needs no storage");
                assert!(
                    align_of::<T>() <= MAX_ALIGN,
                    "libgit2's allocator cannot align T"
                );
            };
            calloc(1, size_of::<T>()).map(NonNull::cast)
        }
    }

    /// The policy of a counted run of elements in libgit2's allocator: allocated by
    /// `git__mallocarray`, `git__calloc` or `git__malloc`, released by `git__free`.
    ///
    /// [`new_uninit`](Self::new_uninit) and [`new_zeroed`](Self::new_zeroed) are
    /// `git__mallocarray` and `git__calloc`, an empty run included, which asks libgit2's
    /// allocator for zero bytes as C does; [`CVec::from_fn`], [`CVec::from_slice`] and
    /// `Clone` allocate a non-empty run's bytes through `git__malloc`, and an empty one
    /// as NULL, allocating nothing. Elements are dropped before the run is released, so
    /// a run of owners releases each.
    ///
    /// Element types aligned beyond what the allocator vouches for are rejected, as are
    /// zero-sized ones, which need no storage. A `u128` is 16-aligned, which `debugalloc`'s
    /// storage is not, whichever constructor would build the run:
    ///
    /// ```compile_fail,E0080
    /// use libgit2::libgit2::src::util::alloc::AllocVec;
    ///
    /// let _ = AllocVec::<u128>::from_slice(&[1, 2]);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use libgit2::libgit2::src::util::alloc::AllocVec;
    ///
    /// let _ = AllocVec::<u128>::from_fn(2, |i| i as u128);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// let _ = libgit2::libgit2::src::util::alloc::ArrayFree::new_zeroed::<u128>(2);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// let _ = libgit2::libgit2::src::util::alloc::ArrayFree::new_uninit::<()>(4);
    /// ```
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ArrayFree;

    /// A counted run of `T` in libgit2's allocator, released by `git__free`.
    pub type AllocVec<T> = CVec<T, ArrayFree>;

    // SAFETY: `free` releases the buffer exactly once and reads none of it, so any
    // `byte_len` the contract admits is sound; every buffer under this policy is storage of
    // libgit2's allocator, as `c_alloc_len`, `c_clone_len` and the constructors below make
    // it, or an `unsafe` adoption vouches.
    unsafe impl CLenDrop for ArrayFree {
        /// Wraps `fn::git__free`
        /// Wraps `mac::xdl_free`
        #[doc(alias = "git__free", alias = "xdl_free")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, _byte_len: usize) {
            // ffibox never releases a NULL run; `git__free(NULL)` would do nothing.
            if let Some(ptr) = NonNull::new(ptr) {
                // SAFETY: per `c_drop_len`'s contract, `ptr` is a live allocation of this
                // policy's family, which the caller gives up.
                unsafe { free(ptr) }
            }
        }
    }

    // SAFETY: every allocation `c_alloc_len` returns is fresh and uniquely owned, of at least
    // `byte_len` bytes, and spans at least `MAX_ALIGN` bytes, which makes it aligned to
    // `MAX_ALIGN`; `c_drop_len` above releases it without reading it.
    unsafe impl CLenAlloc for ArrayFree {
        const ALIGN: usize = MAX_ALIGN;

        /// Wraps `fn::git__malloc`
        /// Wraps `mac::xdl_malloc`
        #[doc(alias = "git__malloc", alias = "xdl_malloc")]
        #[inline]
        fn c_alloc_len(&self, byte_len: usize) -> Option<NonNull<u8>> {
            // ffibox's run constructors refuse only a size that overflows `usize`.
            if exceeds_slice_bytes(byte_len, 1) {
                set_oom();
                return None;
            }
            malloc(byte_len.max(MAX_ALIGN))
        }
    }

    // SAFETY: as `CLenAlloc`, with the `byte_len` bytes at `ptr` copied in; the source is
    // only read.
    unsafe impl CLenClone for ArrayFree {
        const ALIGN: usize = MAX_ALIGN;

        /// Wraps `fn::git__malloc`
        /// Wraps `mac::xdl_malloc`
        #[doc(alias = "git__malloc", alias = "xdl_malloc")]
        #[inline]
        unsafe fn c_clone_len(&self, ptr: *mut u8, byte_len: usize) -> Option<NonNull<u8>> {
            let copy = malloc(byte_len.max(MAX_ALIGN))?;
            // SAFETY: per `c_clone_len`'s contract, `byte_len` bytes at `ptr` are readable;
            // `copy` is a fresh allocation of at least `byte_len` bytes, so the two cannot
            // overlap.
            unsafe { ptr::copy_nonoverlapping(ptr, copy.as_ptr(), byte_len) };
            Some(copy)
        }
    }

    impl ArrayFree {
        /// `len` uninitialised elements, for C or Rust to fill before
        /// [`assume_init`](CVec::assume_init); `None` when the allocation is refused or
        /// its size overflows, which leaves `GIT_ERROR_NOMEMORY` as the thread's last
        /// error.
        ///
        /// An empty run asks libgit2's allocator for zero bytes, as C does, and holds
        /// what it returns until the run is dropped.
        ///
        /// Wraps `fn::git__mallocarray`
        #[doc(alias = "git__mallocarray")]
        #[must_use]
        pub fn new_uninit<T>(len: usize) -> Option<AllocVec<MaybeUninit<T>>> {
            let data = Self::alloc_run::<T>(len, mallocarray)?;
            // SAFETY: `data` is NULL with `len == 0`, or fresh, uniquely owned storage of
            // `len * size_of::<T>()` bytes aligned for `T`, which `c_drop_len` releases;
            // `MaybeUninit<T>` needs no initialisation.
            unsafe { CVec::from_raw_parts_or_empty(data.cast(), len) }
        }

        /// `len` zeroed elements; `None` when the allocation is refused or its size
        /// overflows, which leaves `GIT_ERROR_NOMEMORY` as the thread's last error.
        ///
        /// An empty run asks libgit2's allocator for zero bytes, as C does, and holds
        /// what it returns until the run is dropped.
        ///
        /// Wraps `fn::git__calloc`
        /// Wraps `mac::xdl_calloc`
        #[doc(alias = "git__calloc", alias = "xdl_calloc")]
        #[must_use]
        pub fn new_zeroed<T: CPlainElem>(len: usize) -> Option<AllocVec<T>> {
            let data = Self::alloc_run::<T>(len, calloc)?;
            // SAFETY: as in `new_uninit`, and every element is all zero, a valid `T` since
            // `T: CPlainElem` admits every bit pattern.
            unsafe { CVec::from_raw_parts_or_empty(data, len) }
        }

        /// Resizes `run` to `len` elements, in place or at a new address, keeping its first
        /// `min(run.len(), len)` elements' bytes; the elements past its old length are
        /// uninitialised. On failure `run` is left as it was, and the error, libgit2's
        /// out-of-memory error (`GIT_ERROR_NOMEMORY`), is also the thread's last error.
        ///
        /// This is `git__reallocarray(ptr, len, size_of::<T>())`: libgit2's allocator resizes
        /// the run's storage through its `grealloc`, and allocates storage afresh for a run
        /// that has none. A size that overflows `usize` is refused by C, and one past
        /// `isize::MAX` bytes, which no Rust slice may span, here, both before the allocator is
        /// asked. Resizing to zero elements is Rust's own: it releases the storage
        /// (`git__free`) and leaves the run empty without storage, where C would ask the
        /// allocator for a zero-byte resize of live storage, which C leaves
        /// implementation-defined and libgit2 never makes (see [`ReallocFn`]).
        ///
        /// The elements past `len` are forgotten, not dropped, as `MaybeUninit` ones always
        /// are. A run grows once its storage is resized:
        ///
        /// ```
        /// use core::mem::MaybeUninit;
        ///
        /// use libgit2::libgit2::src::util::alloc::ArrayFree;
        ///
        /// let mut run = ArrayFree::new_uninit::<u32>(2).unwrap();
        /// run.as_mut_slice()[0].write(7);
        /// ArrayFree::resize(&mut run, 3).unwrap();
        /// run.as_mut_slice()[1].write(8);
        /// run.as_mut_slice()[2].write(9);
        /// // SAFETY: every element was written above.
        /// let run = unsafe { run.assume_init() };
        /// assert_eq!(run.as_slice(), &[7, 8, 9]);
        /// ```
        ///
        /// Element types aligned beyond what the allocator vouches for are rejected, as
        /// zero-sized ones are:
        ///
        /// ```compile_fail,E0080
        /// use libgit2::libgit2::src::util::alloc::{AllocVec, ArrayFree};
        ///
        /// let _ = ArrayFree::resize(&mut AllocVec::<core::mem::MaybeUninit<u128>>::empty(), 1);
        /// ```
        ///
        /// # Errors
        ///
        /// The thread's last libgit2 error, out of memory, when the allocator refuses the
        /// resize or its size is refused before it is asked; libgit2's "library has not been
        /// initialized" when libgit2 cannot be initialised.
        ///
        /// Wraps `fn::git__reallocarray`
        #[doc(alias = "git__reallocarray")]
        pub fn resize<T>(run: &mut AllocVec<MaybeUninit<T>>, len: usize) -> Result<(), ErrorBuf> {
            const {
                assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
                assert!(
                    align_of::<T>() <= MAX_ALIGN,
                    "libgit2's allocator cannot align T"
                );
            }
            Self::resize_run(run, len, Resize::Elements)
        }

        /// Resizes a run of bytes to `len` bytes, as [`resize`](Self::resize) resizes a run
        /// of elements: in place or at a new address, its first `min(run.len(), len)` bytes
        /// kept, the run left as it was on failure; resizing to zero bytes releases the
        /// storage.
        ///
        /// This is `git__realloc(ptr, len)`, which asks the allocator for `len` bytes with no
        /// product to check, and xdiff's `xdl_realloc`, its alias. A size past `isize::MAX`
        /// is refused before the allocator is asked.
        ///
        /// # Errors
        ///
        /// As [`resize`](Self::resize)'s.
        ///
        /// Wraps `fn::git__realloc`
        /// Wraps `mac::xdl_realloc`
        #[doc(alias = "git__realloc", alias = "xdl_realloc")]
        pub fn resize_bytes(
            run: &mut AllocVec<MaybeUninit<u8>>,
            len: usize,
        ) -> Result<(), ErrorBuf> {
            Self::resize_run(run, len, Resize::Bytes)
        }

        /// The resize [`resize`](Self::resize) and [`resize_bytes`](Self::resize_bytes)
        /// share, through the primitive `how` names; `T` is aligned for by every storage
        /// of at least `size_of::<T>()` bytes the allocator returns, as `resize` checks at
        /// compile time (a byte always is).
        fn resize_run<T>(
            run: &mut AllocVec<MaybeUninit<T>>,
            len: usize,
            how: Resize,
        ) -> Result<(), ErrorBuf> {
            if len == 0 {
                // The old storage, if any, is released by the run's own drop.
                *run = CVec::empty();
                return Ok(());
            }
            if exceeds_slice_bytes(len, size_of::<T>()) {
                set_oom();
                return Err(last());
            }
            let data = run.as_ptr().cast::<u8>();
            let resized = match how {
                // SAFETY: `data` is the run's storage, a live allocation of libgit2's
                // allocator that the run owns, or NULL for a run without storage; it is given
                // up only on success, when the run is replaced below. `len` is non-zero, and
                // `resize_bytes` passes byte runs only, so `len` is the size in bytes.
                Resize::Bytes => unsafe { realloc(data, len) },
                // SAFETY: as above; `len` and `size_of::<T>()` are both non-zero.
                Resize::Elements => unsafe { reallocarray(data, len, size_of::<T>()) },
            };
            let Some(resized) = resized else {
                return Err(last());
            };
            // SAFETY: `resized` is fresh, uniquely owned storage of libgit2's allocator of
            // `len * size_of::<T>()` bytes, at least `size_of::<T>()` of them, so aligned for
            // `T` (`resize_run`'s precondition); `c_drop_len` releases it, and
            // `MaybeUninit<T>` needs no initialisation.
            let resized = unsafe { CVec::from_raw_parts(resized.as_ptr().cast(), len) };
            let old = mem::replace(run, resized.expect("a non-NULL run adopts"));
            // The resize released the old storage, or handed it on as `resized`: its owner is
            // dissolved without releasing it again.
            let (_released, _) = old.into_raw_parts();
            Ok(())
        }

        /// Storage for `len` elements of `T` from `alloc(len, size_of::<T>())`, aligned
        /// for `T`; `None` on failure, NULL only for an empty run held without storage.
        ///
        /// C checks the product for overflow; a size past `isize::MAX` bytes, which no
        /// Rust slice may span, is refused here before C is asked, with the error C
        /// leaves for an overflow. A non-empty allocation spans at least
        /// `size_of::<T>()` bytes, which makes it aligned for `T`. An empty one need not
        /// be, since no object fits in it: one that is not is released, and the run
        /// held as NULL.
        fn alloc_run<T>(
            len: usize,
            alloc: impl FnOnce(usize, usize) -> Option<NonNull<u8>>,
        ) -> Option<*mut T> {
            const {
                assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
                assert!(
                    align_of::<T>() <= MAX_ALIGN,
                    "libgit2's allocator cannot align T"
                );
            }
            if exceeds_slice_bytes(len, size_of::<T>()) {
                set_oom();
                return None;
            }
            let data = alloc(len, size_of::<T>())?.cast::<T>();
            if len == 0 && !data.is_aligned() {
                // SAFETY: `data` is the fresh allocation `alloc` just made, given up here.
                unsafe { free(data.cast()) };
                return Some(ptr::null_mut());
            }
            Some(data.as_ptr())
        }
    }

    /// Which of the family's resizes [`ArrayFree::resize_run`] calls.
    #[derive(Clone, Copy)]
    enum Resize {
        /// `git__realloc`, to a size in bytes.
        Bytes,
        /// `git__reallocarray`, to a count of elements.
        Elements,
    }

    /// Whether `nelem` elements of `elsize` bytes span more than `isize::MAX` bytes
    /// without overflowing `usize`, which C would ask its allocator for.
    fn exceeds_slice_bytes(nelem: usize, elsize: usize) -> bool {
        nelem
            .checked_mul(elsize)
            .is_some_and(|size| isize::try_from(size).is_err())
    }

    /// The policy of one NUL-terminated string in libgit2's allocator: copied by
    /// `git__strdup`, `git__strndup` or `git__substrdup`, released by `git__free`.
    ///
    /// These are the strings libgit2 releases with `git__free`: a `git_strarray`'s, a
    /// `git_signature`'s name and email, a `git_buf`'s that it owns. [`AllocStr`] hands
    /// one to C with [`CStrBox::into_raw`], and adopts one C made with the `unsafe`
    /// [`CStrBox::from_raw`].
    ///
    /// A borrow of the string ends with its owner:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::src::util::alloc::StrFree;
    ///
    /// let s = StrFree::dup(c"tied").unwrap();
    /// let view = s.as_c_str();
    /// drop(s);
    /// let _ = view.to_bytes();
    /// ```
    #[derive(Clone, Copy, Debug, Default)]
    pub struct StrFree;

    /// One NUL-terminated string in libgit2's allocator, released by `git__free`.
    pub type AllocStr = CStrBox<StrFree>;

    // SAFETY: `free` releases the string exactly once and reads none of it; every string
    // under this policy is storage of libgit2's allocator, as the copies below make it, or
    // an `unsafe` adoption vouches.
    unsafe impl CDrop<c_char> for StrFree {
        /// Wraps `fn::git__free`
        /// Wraps `mac::xdl_free`
        #[doc(alias = "git__free", alias = "xdl_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<c_char>) {
            // SAFETY: per `c_drop`'s contract, `ptr` is a live string of this policy's
            // family whose claim the caller gives up.
            unsafe { free(ptr.cast()) }
        }
    }

    // SAFETY: `c_dup` reads the live string at `ptr` up to its terminator, writes nothing
    // through it, and returns a fresh allocation holding a copy that `c_drop` releases, or
    // `None`.
    unsafe impl CDupClone<c_char> for StrFree {
        /// Wraps `fn::git__strdup`
        #[doc(alias = "git__strdup")]
        #[inline]
        unsafe fn c_dup(&self, ptr: NonNull<c_char>) -> Option<NonNull<c_char>> {
            // SAFETY: per `c_dup`'s contract, `ptr` is a live NUL-terminated string that
            // stays unchanged for the call.
            unsafe { strdup(ptr) }
        }
    }

    impl StrFree {
        /// A copy of `s` in libgit2's allocator; `None` when the allocation is refused,
        /// which leaves `GIT_ERROR_NOMEMORY` as the thread's last error.
        ///
        /// Wraps `fn::git__strdup`
        #[doc(alias = "git__strdup")]
        #[must_use]
        pub fn dup(s: &CStr) -> Option<AllocStr> {
            let s = NonNull::from(s).cast::<c_char>();
            // SAFETY: `s` is the borrowed string, live and unchanged for the call.
            Self::adopt(unsafe { strdup(s) }?)
        }

        /// A copy of `bytes` up to their first NUL, or of all of them when they hold
        /// none, in libgit2's allocator; `None` when the allocation is refused, which
        /// leaves `GIT_ERROR_NOMEMORY` as the thread's last error.
        ///
        /// `git__strndup(str, n)` reads at most `n` bytes of `str`: `bytes` is that run.
        /// Only the bytes before the NUL are allocated.
        ///
        /// Wraps `fn::git__strndup`
        #[doc(alias = "git__strndup")]
        #[must_use]
        pub fn dup_prefix(bytes: &[u8]) -> Option<AllocStr> {
            Self::adopt(strndup(bytes)?)
        }

        /// A copy of all of `bytes` and a terminator in libgit2's allocator; `None` when
        /// the allocation is refused, which leaves `GIT_ERROR_NOMEMORY` as the thread's
        /// last error.
        ///
        /// `git__substrdup(str, n)` copies exactly the `n` bytes of `str`, NULs included:
        /// `bytes` is that run. A NUL among them ends the string, as it does for C, which
        /// reads it up to its first NUL; the bytes after it are still allocated and copied.
        ///
        /// Wraps `fn::git__substrdup`
        #[doc(alias = "git__substrdup")]
        #[must_use]
        pub fn dup_bytes(bytes: &[u8]) -> Option<AllocStr> {
            Self::adopt(substrdup(bytes)?)
        }

        /// The owner of a string one of the copies above made.
        fn adopt(copy: NonNull<c_char>) -> Option<AllocStr> {
            // SAFETY: `copy` is fresh, uniquely owned storage of libgit2's allocator,
            // NUL-terminated within its allocation, which `c_drop` releases.
            unsafe { AllocStr::from_raw(copy.as_ptr()) }
        }
    }

    // --- The installed allocator -----------------------------------------------------------

    /// libgit2's allocator: the global every allocation libgit2 makes reads its
    /// members from (`git__allocator`), borrowed for the rest of the process once libgit2's
    /// runtime is pinned; `None` when libgit2 cannot be initialised.
    ///
    /// The handle reads the global itself, not a copy of it: a read after an allocator was
    /// installed reads the new one. Before `git_libgit2_init` the global holds the failing
    /// family (`git_failalloc_*`), and initialisation installs the standard allocator
    /// ([`global_init`]) unless another was installed before it; the borrow pins libgit2's
    /// runtime first, initialising libgit2 once for the rest of the process, so it reads the
    /// global as initialisation left it. [`set_allocator`] then copies a value over it.
    ///
    /// C reads and writes the global with no synchronisation, so a read through the handle
    /// races any write on another thread. Once the runtime is pinned, no safe path writes it:
    /// a later `git_libgit2_init` takes another reference to a running libgit2 and installs
    /// nothing, and the pinned reference is never released, so libgit2 is never shut down and
    /// set up again. The writers left, [`set_allocator`] and [`global_init`], are `unsafe`, and
    /// their callers keep every other thread from reading the global through this handle while
    /// they write it, as they keep it from allocating. The handle can therefore be read on any
    /// thread, and copies of it shared freely.
    ///
    /// ```
    /// use libgit2::libgit2::src::util::alloc::installed;
    ///
    /// let global = installed().expect("libgit2 initialises");
    /// // Initialisation installed the standard allocator, which allocates.
    /// assert!(global.malloc().is_some());
    /// let seen = format!("{global:?}");
    /// std::thread::spawn(move || assert_eq!(format!("{global:?}"), seen))
    ///     .join()
    ///     .unwrap();
    /// ```
    ///
    /// [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator
    ///
    /// Wraps `glb::git__allocator`
    #[doc(alias = "git__allocator")]
    #[must_use]
    pub fn installed() -> Option<AllocatorRef<'static>> {
        if !runtime_pinned() {
            return None;
        }
        let global = &raw mut ffi::git__allocator;
        // SAFETY: `global` addresses a C global, live and initialised for the whole process;
        // the handle reads it through that pointer, forming no reference. The runtime is
        // pinned, so libgit2's first initialisation, the one write a safe path makes, happened
        // before this point (`git_libgit2_init` counts under libgit2's own mutex, and the
        // pinned reference is published through a `OnceLock`), and no later one installs
        // anything, the pinned reference never being released; every other write is an
        // `unsafe` installer's, whose contract excludes reads through this handle on other
        // threads while it writes.
        unsafe { AllocatorRef::from_ptr(global) }
    }

    /// Installs libgit2's standard allocator in place of the failing family, and keeps any
    /// other allocator installed: the allocator step of `git_libgit2_init`, which runs it
    /// first, so that an allocator installed before initialisation stays.
    ///
    /// libgit2 tells the failing family by its `gmalloc`'s address
    /// (`git_failalloc_malloc`): with that one installed, this installs the standard
    /// allocator (`stdalloc`, over libc's `malloc` family, or `debugalloc` in a
    /// `DEBUG_STRICT_ALLOC` build), as [`set_allocator`]`(None)` does; with any other
    /// installed, nothing changes. Before initialisation it lets libgit2 allocate, yet its
    /// error state is not set up then, as the campaign's lessons note: an allocation that fails
    /// before initialisation still reports through it.
    ///
    /// Installing the standard allocator cannot fail on this platform, so nothing is returned.
    ///
    /// It writes a global C reads unsynchronised, so no safe call reaches it:
    ///
    /// ```compile_fail,E0133
    /// libgit2::libgit2::src::util::alloc::global_init();
    /// ```
    ///
    /// # Safety
    ///
    /// As [`set_allocator`]`(None)`'s, when the failing family is installed: no other thread
    /// may call into libgit2, through this crate or otherwise, nor read the global through
    /// [`installed`]'s handle, while this runs, since it writes the global every allocation
    /// reads without synchronisation; and every allocation
    /// libgit2 or a live owner of this crate holds must be one the standard allocator can
    /// resize and release. The failing family is installed before initialisation, when
    /// libgit2 holds no allocation, or when an earlier [`set_allocator`] installed it after;
    /// with any other family installed, the call only reads the global, which no other
    /// thread may write meanwhile.
    ///
    /// [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator
    ///
    /// Wraps `fn::git_allocator_global_init`
    #[doc(alias = "git_allocator_global_init")]
    pub unsafe fn global_init() {
        // SAFETY: no arguments; it reads the global and, with the failing family installed,
        // writes the standard family over it, which races nothing and strands no allocation
        // per this function's contract.
        let rc = unsafe { ffi::git_allocator_global_init() };
        // `git_stdalloc_init_allocator` and `git_debugalloc_init_allocator` return 0.
        debug_assert_eq!(rc, 0);
    }

    // Each owner is the pointer it stands in for: its policy is zero-sized, so C reads a
    // run of them as a `T **` or a `char **`, and `Option` of one as a nullable pointer.
    const _: () = assert!(
        size_of::<Option<AllocBox<Allocator>>>() == size_of::<*mut ffi::git_allocator>()
            && size_of::<Option<AllocStr>>() == size_of::<*mut c_char>()
    );

    #[cfg(test)]
    mod unit_tests {
        use std::alloc::{Layout, alloc, dealloc, realloc};
        use std::cell::RefCell;
        use std::ffi::CString;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering::Relaxed;

        use super::*;
        use crate::libgit2::src::libgit2::oid::Oid;
        use crate::libgit2::src::libgit2::settings::set_allocator;
        use crate::libgit2::src::util::errors::{ErrorClass, clear, last};

        /// One call that reached an `Alloc` implementation.
        #[derive(Debug, PartialEq, Eq)]
        enum Call {
            Malloc(usize, Option<CString>, i32),
            Realloc(bool, usize, Option<CString>, i32),
            Free,
        }

        thread_local! {
            static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
        }

        fn take_calls() -> Vec<Call> {
            CALLS.with(RefCell::take)
        }

        /// Applies `f` to the calling thread's record, unless the thread is exiting and
        /// the record is gone: libgit2 releases a thread's error state from a TLS
        /// destructor, which may run after it.
        fn record(f: impl FnOnce(&mut Vec<Call>)) {
            let _gone = CALLS.try_with(|c| f(&mut c.borrow_mut()));
        }

        /// The one request size [`Heap`] refuses.
        const REFUSED: usize = 4242;

        /// Whether [`Heap`] refuses zero-byte requests too, as C's `malloc(0)` may.
        static REFUSE_ZERO: AtomicBool = AtomicBool::new(false);

        /// Whether [`Heap`] answers a zero-byte request with [`odd_block`], an address
        /// aligned for no type wider than a byte, which no object of the request fits
        /// anyway.
        static ODD_ZERO: AtomicBool = AtomicBool::new(false);

        static ODD: [u64; 2] = [0; 2];

        /// The odd address [`ODD_ZERO`] hands out; never written, and released as a
        /// no-op.
        fn odd_block() -> NonNull<u8> {
            NonNull::new((&raw const ODD).cast::<u8>().wrapping_add(1).cast_mut()).unwrap()
        }

        /// A Rust-heap allocator, so the trampolines run under Miri: each block carries its
        /// size in a 16-byte header.
        struct Heap;

        const HEADER: usize = 16;

        fn layout(len: usize) -> Layout {
            Layout::from_size_align(HEADER + len, HEADER).unwrap()
        }

        // SAFETY: blocks come from the global allocator, 16-aligned past their header,
        // fresh per call; `realloc` keeps the prefix and `free` releases with the recorded
        // layout. Thread-safe as the global allocator is.
        unsafe impl Alloc for Heap {
            fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
                record(|c| c.push(Call::Malloc(len, file.map(CStr::to_owned), line)));
                if len == REFUSED || (len == 0 && REFUSE_ZERO.load(Relaxed)) {
                    return None;
                }
                if len == 0 && ODD_ZERO.load(Relaxed) {
                    return Some(odd_block());
                }
                // SAFETY: the layout is never zero-sized.
                let base = NonNull::new(unsafe { alloc(layout(len)) })?;
                // SAFETY: the header is the block's first 16 bytes, aligned for a `usize`.
                unsafe { base.cast::<usize>().write(len) };
                // SAFETY: the block spans `HEADER + len` bytes.
                Some(unsafe { base.add(HEADER) })
            }

            unsafe fn realloc(
                ptr: Option<NonNull<u8>>,
                len: usize,
                file: Option<&CStr>,
                line: i32,
            ) -> Option<NonNull<u8>> {
                record(|c| {
                    c.push(Call::Realloc(
                        ptr.is_some(),
                        len,
                        file.map(CStr::to_owned),
                        line,
                    ));
                });
                let Some(ptr) = ptr else {
                    let block = Self::malloc(len, file, line);
                    record(|c| drop(c.pop()));
                    return block;
                };
                if len == REFUSED {
                    return None;
                }
                // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
                let base = unsafe { ptr.sub(HEADER) };
                // SAFETY: as above; the header holds the block's length.
                let old = unsafe { base.cast::<usize>().read() };
                // SAFETY: `base` was allocated with `layout(old)`; the new size is non-zero.
                let base =
                    NonNull::new(unsafe { realloc(base.as_ptr(), layout(old), HEADER + len) })?;
                // SAFETY: as in `malloc`.
                unsafe { base.cast::<usize>().write(len) };
                // SAFETY: as in `malloc`.
                Some(unsafe { base.add(HEADER) })
            }

            unsafe fn free(ptr: NonNull<u8>) {
                record(|c| c.push(Call::Free));
                if ptr == odd_block() {
                    return;
                }
                // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
                let base = unsafe { ptr.sub(HEADER) };
                // SAFETY: as above; `base` was allocated with `layout(len)`.
                let read = unsafe { base.cast::<usize>().read() };
                // SAFETY: as above.
                unsafe { dealloc(base.as_ptr(), layout(read)) }
            }
        }

        unsafe extern "C" fn c_malloc(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
            ptr::null_mut()
        }

        unsafe extern "C" fn c_realloc(
            _: *mut c_void,
            _: usize,
            _: *const c_char,
            _: c_int,
        ) -> *mut c_void {
            ptr::null_mut()
        }

        unsafe extern "C" fn c_free(_: *mut c_void) {}

        unsafe extern "C" fn other_free(_: *mut c_void) {}

        // Each coercion of a function item may yield a distinct address (Miri models
        // that), so every comparison below is against one stored pointer value.

        /// The C family `CFamily` names: its functions, stored once.
        static C_FAMILY: Allocator = Allocator::from_fns(c_malloc, c_realloc, c_free);

        /// An implementation that binds a C family: its methods are never reached through an
        /// `Allocator::new` value, which holds the C functions themselves.
        struct CFamily;

        // SAFETY: every allocation fails and nothing is released, as `C_FAMILY`'s functions
        // do; none touches any state.
        unsafe impl Alloc for CFamily {
            const C_ALLOCATOR: Option<Allocator> = Some(C_FAMILY);

            fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn realloc(
                _: Option<NonNull<u8>>,
                _: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn free(_: NonNull<u8>) {}
        }

        #[test]
        fn a_c_family_keeps_its_own_functions() {
            let built = Allocator::new::<CFamily>();
            let (theirs, ours) = (C_FAMILY.as_ref(), built.as_ref());
            assert!(ptr::fn_addr_eq(
                ours.malloc().unwrap(),
                theirs.malloc().unwrap()
            ));
            assert!(ptr::fn_addr_eq(
                ours.realloc().unwrap(),
                theirs.realloc().unwrap()
            ));
            assert!(ptr::fn_addr_eq(
                ours.free().unwrap(),
                theirs.free().unwrap()
            ));
            // A family without C functions gets trampolines, never those.
            let heap = Allocator::new::<Heap>();
            assert!(!ptr::fn_addr_eq(
                heap.as_ref().malloc().unwrap(),
                theirs.malloc().unwrap()
            ));
        }

        #[test]
        #[expect(
            clippy::many_single_char_names,
            reason = "short names for the sides and values the steps compare"
        )]
        fn from_fns_sets_every_member() {
            let (m, r, f): (MallocFn, ReallocFn, FreeFn) = (c_malloc, c_realloc, c_free);
            let a = Allocator::from_fns(m, r, f);
            let h = a.as_ref();
            assert!(ptr::fn_addr_eq(h.malloc().unwrap(), m));
            assert!(ptr::fn_addr_eq(h.realloc().unwrap(), r));
            assert!(ptr::fn_addr_eq(h.free().unwrap(), f));
        }

        #[test]
        #[expect(
            clippy::many_single_char_names,
            reason = "short names for the sides and values the steps compare"
        )]
        fn setters_replace_one_member_each() {
            let (m, r, f, other): (MallocFn, ReallocFn, FreeFn, FreeFn) =
                (c_malloc, c_realloc, c_free, other_free);
            let mut a = Allocator::from_fns(m, r, f);
            let before = a;
            a.as_mut().set_free(other);
            assert!(ptr::fn_addr_eq(a.as_ref().free().unwrap(), other));
            assert!(ptr::fn_addr_eq(a.as_ref().malloc().unwrap(), m));
            let heap = Allocator::new::<Heap>();
            let (hm, hr) = (
                heap.as_ref().malloc().unwrap(),
                heap.as_ref().realloc().unwrap(),
            );
            let mut handle = a.as_mut();
            handle.as_mut().set_malloc(hm);
            handle.set_realloc(hr);
            let h = handle.as_ref();
            assert!(ptr::fn_addr_eq(h.malloc().unwrap(), hm));
            assert!(ptr::fn_addr_eq(h.realloc().unwrap(), hr));
            assert!(ptr::fn_addr_eq(h.free().unwrap(), other));
            // `before` is a copy, untouched.
            let b = before.as_ref();
            assert!(ptr::fn_addr_eq(b.malloc().unwrap(), m));
            assert!(ptr::fn_addr_eq(b.realloc().unwrap(), r));
            assert!(ptr::fn_addr_eq(b.free().unwrap(), f));
        }

        #[test]
        fn trampolines_forward_to_the_implementation() {
            static HEAP: Allocator = Allocator::new::<Heap>();
            let r = HEAP.as_ref();
            let (malloc, realloc, free) =
                (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());
            take_calls();

            // SAFETY: `file` is a live C string; the block is released below.
            let p = unsafe { malloc(5, c"alloc.c".as_ptr(), 7) };
            assert!(!p.is_null());
            assert_eq!(p as usize % 16, 0);
            // SAFETY: `p` holds 5 writable bytes.
            unsafe {
                p.cast::<u8>()
                    .copy_from_nonoverlapping(b"hello".as_ptr(), 5);
            };
            // SAFETY: `p` is a live block of this family, given up; NULL `file`.
            let q = unsafe { realloc(p, 64, ptr::null(), -1) };
            assert!(!q.is_null());
            // SAFETY: the first 5 bytes were carried over.
            let kept = unsafe { core::slice::from_raw_parts(q.cast::<u8>(), 5) };
            assert_eq!(kept, b"hello");
            // SAFETY: a refused resize leaves `q` live.
            assert!(unsafe { realloc(q, REFUSED, ptr::null(), 0) }.is_null());
            // SAFETY: NULL is a fresh allocation; the block is released below.
            let n = unsafe { realloc(ptr::null_mut(), 3, c"x".as_ptr(), 1) };
            assert!(!n.is_null());
            // SAFETY: NULL is a no-op, and `q` and `n` are live blocks given up once.
            unsafe { free(ptr::null_mut()) };
            // SAFETY: as above.
            unsafe { free(q) };
            // SAFETY: as above.
            unsafe { free(n) };
            // SAFETY: as above.
            assert!(unsafe { malloc(REFUSED, ptr::null(), 0) }.is_null());

            assert_eq!(
                take_calls(),
                [
                    Call::Malloc(5, Some(c"alloc.c".to_owned()), 7),
                    Call::Realloc(true, 64, None, -1),
                    Call::Realloc(true, REFUSED, None, 0),
                    Call::Realloc(false, 3, Some(c"x".to_owned()), 1),
                    Call::Free,
                    Call::Free,
                    Call::Malloc(REFUSED, None, 0),
                ]
            );
        }

        /// A zero-byte request is an allocation like any other: distinct, and released by
        /// `free`, through `malloc` and through `realloc(NULL, ..)` alike.
        #[test]
        fn zero_byte_requests_are_allocations() {
            static HEAP: Allocator = Allocator::new::<Heap>();
            let r = HEAP.as_ref();
            let (malloc, realloc, free) =
                (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());
            take_calls();
            // SAFETY: NULL `file`; both blocks are released below.
            let malloc2 = unsafe { malloc(0, ptr::null(), 0) };
            // SAFETY: as above.
            let (a, b) = unsafe { (malloc2, realloc(ptr::null_mut(), 0, ptr::null(), 0)) };
            assert!(!a.is_null() && !b.is_null() && a != b);
            // A zero-byte block grows like any other, keeping its (empty) prefix.
            // SAFETY: `a` is a live block of this family, given up on success.
            let grown = unsafe { realloc(a, 24, c"x".as_ptr(), 1) };
            assert!(!grown.is_null());
            // SAFETY: `grown` holds 24 writable bytes.
            unsafe { grown.cast::<u8>().write_bytes(0xAB, 24) };
            // SAFETY: two live blocks of this family, each given up once.
            unsafe { free(grown) };
            // SAFETY: as above.
            unsafe { free(b) };
            assert_eq!(
                take_calls(),
                [
                    Call::Malloc(0, None, 0),
                    Call::Realloc(false, 0, None, 0),
                    Call::Realloc(true, 24, Some(c"x".to_owned()), 1),
                    Call::Free,
                    Call::Free,
                ]
            );
        }

        /// A zero-byte resize of a live block never reaches the implementation: the trampoline
        /// refuses it, returning NULL with the block left live, whatever C passes. Pure Rust,
        /// so Miri runs it.
        #[test]
        fn zero_byte_resizes_of_live_blocks_are_refused() {
            static HEAP: Allocator = Allocator::new::<Heap>();
            let r = HEAP.as_ref();
            let (malloc, realloc, free) =
                (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());
            take_calls();
            // SAFETY: NULL `file`; the block is released below.
            let p = unsafe { malloc(4, ptr::null(), 0) };
            assert!(!p.is_null());
            // SAFETY: `p` holds 4 writable bytes.
            unsafe { p.cast::<u8>().copy_from_nonoverlapping(b"live".as_ptr(), 4) };
            // SAFETY: `p` is a live block of this family; these trampolines accept a zero-byte
            // resize of one, and refuse it.
            assert!(unsafe { realloc(p, 0, c"x".as_ptr(), 1) }.is_null());
            // SAFETY: the refusal left `p` live and its 4 bytes as they were.
            let kept = unsafe { core::slice::from_raw_parts(p.cast::<u8>(), 4) };
            assert_eq!(kept, b"live");
            // SAFETY: `p` is a live block of this family, given up once.
            unsafe { free(p) };
            assert_eq!(take_calls(), [Call::Malloc(4, None, 0), Call::Free]);
        }

        /// An implementation that panics in every member, before it allocates, resizes or
        /// releases anything, as `Alloc`'s contract asks of a panicking `realloc`.
        struct Panicking;

        // SAFETY: never returns an allocation, so nothing is ever live; each method panics
        // before touching anything, so a panicking `realloc` leaves `ptr` as it was.
        unsafe impl Alloc for Panicking {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                panic!("refusing {len} bytes");
            }

            unsafe fn realloc(
                _: Option<NonNull<u8>>,
                len: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                panic!("refusing to resize to {len} bytes");
            }

            unsafe fn free(_: NonNull<u8>) {
                panic!("refusing to release");
            }
        }

        static PANICKING: Allocator = Allocator::new::<Panicking>();

        const CHILD: &str = "CRUSTIFY_ALLOC_UNIT_CHILD";

        /// libgit2 calls the trampolines from C, which a panic must not unwind into: the
        /// barrier returns NULL in its place, C's out-of-memory convention, and leaves a
        /// resized block live. Pure Rust, so Miri runs it.
        #[test]
        fn panics_in_malloc_and_realloc_read_as_failure() {
            let r = PANICKING.as_ref();
            let (malloc, realloc) = (r.malloc().unwrap(), r.realloc().unwrap());
            // SAFETY: NULL `file`; the implementation panics before allocating.
            assert!(unsafe { malloc(1, ptr::null(), 0) }.is_null());
            // SAFETY: NULL is a fresh allocation, which the implementation never makes.
            assert!(unsafe { realloc(ptr::null_mut(), 8, c"x".as_ptr(), 1) }.is_null());
            // A live block the panicking resize must leave alone: `ReallocFn`'s contract wants
            // one of the family, and the implementation never reads it.
            let mut block = [0x5Au8; 16];
            // SAFETY: as above; the implementation panics before reading `block`.
            let resized = unsafe { realloc(block.as_mut_ptr().cast(), 32, ptr::null(), 0) };
            assert!(resized.is_null());
            assert_eq!(block, [0x5A; 16]);
        }

        /// `gfree` has no failure to report, so a panic in [`Alloc::free`] aborts rather
        /// than unwind into C.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        #[cfg_attr(bsan, ignore = "the child's abort path spins under BorrowSanitizer")]
        fn panic_in_free_aborts() {
            // The test's path within the binary: `module_path!` less the crate name.
            let path = concat!(module_path!(), "::panic_in_free_aborts");
            let name = path.split_once("::").map_or(path, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                let free = PANICKING.as_ref().free().unwrap();
                let mut block = 0u64;
                // SAFETY: the implementation panics before releasing anything.
                unsafe { free((&raw mut block).cast()) };
                unreachable!("the trampoline must not return from a panic");
            }
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{stderr}");
            assert!(stderr.contains("refusing to release"), "{stderr}");
            assert!(stderr.contains("cannot unwind"), "{stderr}");
            assert!(!stderr.contains("must not return"), "{stderr}");
        }

        #[test]
        fn debug_lists_member_addresses() {
            let f: FreeFn = c_free;
            let a = Allocator::from_fns(c_malloc, c_realloc, f);
            let shown = format!("{a:?}");
            assert!(shown.starts_with("Allocator { malloc: 0x"), "{shown}");
            assert_eq!(shown, format!("{:?}", a.as_ref()));
            assert!(
                shown.ends_with(&format!("free: {:p} }}", f as *const ())),
                "{shown}"
            );

            // A member C left NULL shows as a null address.
            let mut raw = ffi::git_allocator::default();
            // SAFETY: `raw` is a live `git_allocator` for the handle's lifetime.
            let r = unsafe { AllocatorRef::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(
                format!("{r:?}"),
                "Allocator { malloc: 0x0, realloc: 0x0, free: 0x0 }"
            );
            assert!(r.malloc().is_none() && r.realloc().is_none() && r.free().is_none());
        }

        #[test]
        #[expect(
            clippy::items_after_statements,
            reason = "each helper item sits beside the steps that use it"
        )]
        fn value_is_copy_send_and_sync() {
            fn shared<T: Copy + Send + Sync>() {}
            fn handle<T: Send + Sync>() {}
            shared::<Allocator>();
            handle::<AllocatorRef<'static>>();
            handle::<AllocatorMut<'static>>();

            static HEAP: Allocator = Allocator::new::<Heap>();
            let got = std::thread::spawn(|| HEAP.as_ref().free().map(|f| f as *const () as usize))
                .join()
                .unwrap();
            assert_eq!(got, HEAP.as_ref().free().map(|f| f as *const () as usize));
        }

        // --- the strategies ---------------------------------------------------------------

        /// Runs `body` alone in a child process of this binary, with [`Heap`] installed as
        /// libgit2's allocator before anything initialises libgit2.
        fn with_heap(path: &str, body: fn()) {
            in_child(path, || {
                // SAFETY: `Heap` meets `Alloc`'s contract; it is installed before libgit2 holds
                // any allocation, in a process of its own, before any other thread runs.
                unsafe { set_allocator(Some(HEAP.as_ref())) };
                assert!(runtime_pinned());
                take_calls();
                body();
            });
        }

        /// The [`Heap`] value [`with_heap`] installs.
        static HEAP: Allocator = Allocator::new::<Heap>();

        /// Runs `body` alone in a child process of this binary, where nothing has
        /// initialised libgit2.
        fn in_child(path: &str, body: impl FnOnce()) {
            let name = path.split_once("::").map_or(path, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// The sizes of the allocations requested since the last call, through `malloc` or
        /// `realloc(NULL, ..)`, and the number of releases.
        fn requests() -> (Vec<usize>, usize) {
            let calls = take_calls();
            let frees = calls.iter().filter(|c| **c == Call::Free).count();
            let sizes = calls
                .into_iter()
                .filter_map(|c| match c {
                    Call::Malloc(len, ..) | Call::Realloc(false, len, ..) => Some(len),
                    _ => None,
                })
                .collect();
            (sizes, frees)
        }

        fn assert_out_of_memory() {
            let e = last();
            assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{e:?}");
            clear();
        }

        /// Each strategy asks libgit2's allocator for what `alloc.c` asks it for, through
        /// `gmalloc`, and releases through `gfree` once per allocation.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn strategies_request_what_alloc_c_requests() {
            with_heap(
                concat!(module_path!(), "::strategies_request_what_alloc_c_requests"),
                || {
                    // `git__malloc(sizeof(T))` and `git__calloc(1, sizeof(T))`, each
                    // reaching `gmalloc` from `alloc.h`'s `git__malloc`.
                    let slot = AllocBox::<Allocator>::new_uninit().unwrap();
                    let id = AllocBox::<Oid>::new_zeroed().unwrap();
                    assert_eq!(id.as_ref().id(), &[0; 32][..id.as_ref().id().len()]);
                    drop((slot, id));
                    let calls = take_calls();
                    let sites: Vec<_> = calls
                        .iter()
                        .filter_map(|c| match c {
                            Call::Malloc(_, file, line) => Some((file.clone(), *line)),
                            _ => None,
                        })
                        .collect();
                    // One line of `alloc.h`, spelled from the checkout the static wrapper
                    // was compiled in, or from the one libgit2 was configured from.
                    assert_eq!(sites.len(), 2, "{calls:?}");
                    assert_eq!(sites[0].1, sites[1].1);
                    assert!(
                        sites.iter().all(|(file, _)| file
                            .as_ref()
                            .is_some_and(|f| f.to_bytes().ends_with(b"src/util/alloc.h"))),
                        "{sites:?}"
                    );
                    assert_eq!(
                        calls
                            .iter()
                            .map(|c| match c {
                                Call::Malloc(len, ..) => *len,
                                _ => 0,
                            })
                            .collect::<Vec<_>>(),
                        [size_of::<Allocator>(), size_of::<Oid>(), 0, 0]
                    );

                    // `git__mallocarray(n, size)`, which is `git__realloc(NULL, n * size)`,
                    // and `git__calloc(n, size)`.
                    let mut words = ArrayFree::new_uninit::<u32>(3).unwrap();
                    words.as_mut_slice()[2].write(7);
                    let zeros = ArrayFree::new_zeroed::<u64>(5).unwrap();
                    assert_eq!(zeros.as_slice(), &[0; 5]);
                    drop((words, zeros));
                    let calls = take_calls();
                    assert!(
                        matches!(
                            calls[..],
                            [
                                Call::Realloc(false, 12, ..),
                                Call::Malloc(40, ..),
                                Call::Free,
                                Call::Free
                            ]
                        ),
                        "{calls:?}"
                    );

                    // A run's bytes through `git__malloc`, at least `MAX_ALIGN` of them.
                    let bytes = AllocVec::<u8>::from_slice(b"ab").unwrap();
                    let copy = bytes.clone();
                    let built = AllocVec::<u64>::from_fn(3, |i| i as u64).unwrap();
                    assert_eq!(
                        (copy.as_slice(), built.as_slice()),
                        (&b"ab"[..], &[0, 1, 2][..])
                    );
                    assert_eq!(bytes.as_ptr() as usize % MAX_ALIGN, 0);
                    drop((bytes, copy, built));
                    assert_eq!(requests(), (vec![MAX_ALIGN, MAX_ALIGN, 24], 3));

                    // `git__strdup(s)`: `strlen(s) + 1`; the clone is another `git__strdup`.
                    let dup = StrFree::dup(c"abc").unwrap();
                    let cloned = dup.clone();
                    assert_eq!(cloned.to_bytes_with_nul(), b"abc\0");
                    // `git__strndup(s, n)`: `strnlen(s, n) + 1`.
                    let until_nul = StrFree::dup_prefix(b"ab\0cd").unwrap();
                    let whole = StrFree::dup_prefix(b"abc").unwrap();
                    // `git__substrdup(s, n)`: `n + 1`, NULs and all.
                    let with_nul = StrFree::dup_bytes(b"ab\0cd").unwrap();
                    let empty = StrFree::dup_bytes(b"").unwrap();
                    assert_eq!(
                        (
                            until_nul.as_bytes(),
                            whole.as_bytes(),
                            with_nul.as_bytes(),
                            empty.as_bytes()
                        ),
                        (&b"ab"[..], &b"abc"[..], &b"ab"[..], &b""[..])
                    );
                    drop((dup, cloned, until_nul, whole, with_nul, empty));
                    assert_eq!(requests(), (vec![4, 4, 3, 4, 6, 1], 6));
                },
            );
        }

        /// A request the allocator refuses fails the constructor and leaves libgit2's
        /// out-of-memory error, as `git__malloc` does; nothing is leaked.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn refused_requests_report_out_of_memory() {
            with_heap(
                concat!(module_path!(), "::refused_requests_report_out_of_memory"),
                || {
                    let long = CString::new(vec![b'x'; REFUSED - 1]).unwrap();
                    clear();
                    assert!(ArrayFree::new_zeroed::<u8>(REFUSED).is_none());
                    assert_out_of_memory();
                    assert!(ArrayFree::new_uninit::<u16>(REFUSED / 2).is_none());
                    assert_out_of_memory();
                    assert!(AllocVec::<u8>::from_slice(&[1; REFUSED]).is_none());
                    assert_out_of_memory();
                    assert!(StrFree::dup(&long).is_none());
                    assert_out_of_memory();
                    assert!(StrFree::dup_prefix(long.as_bytes_with_nul()).is_none());
                    assert_out_of_memory();
                    assert!(StrFree::dup_bytes(long.as_bytes()).is_none());
                    assert_out_of_memory();
                    // A refused clone leaves its source live.
                    let short = StrFree::dup_bytes(&long.as_bytes()[1..]).unwrap();
                    take_calls();
                    let grown = AllocVec::<u8>::from_slice(&[0; REFUSED - MAX_ALIGN]).unwrap();
                    assert!(grown.try_clone().is_some());
                    drop(grown);
                    assert_eq!(requests(), (vec![REFUSED - MAX_ALIGN; 2], 2));
                    assert_eq!(short.len(), REFUSED - 2);
                    assert!(StrFree::dup(&long).is_none());
                    assert_eq!(requests(), (vec![REFUSED], 0));
                    assert_out_of_memory();
                },
            );
        }

        /// A size that overflows, or passes `isize::MAX`, is refused before the allocator is
        /// asked, with libgit2's out-of-memory error: an overflow by `git__calloc` and
        /// `git__mallocarray` (`GIT_MULTIPLY_SIZET_OVERFLOW`), the rest by the strategy.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn overflowing_sizes_report_out_of_memory_unasked() {
            with_heap(
                concat!(
                    module_path!(),
                    "::overflowing_sizes_report_out_of_memory_unasked"
                ),
                || {
                    // The thread's error state is allocated once, before the steps.
                    clear();
                    take_calls();
                    assert!(ArrayFree::new_zeroed::<u64>(usize::MAX / 4).is_none());
                    assert_out_of_memory();
                    assert!(ArrayFree::new_uninit::<u8>(isize::MAX as usize + 1).is_none());
                    assert_out_of_memory();
                    assert!(ArrayFree::new_uninit::<[u8; 3]>(usize::MAX / 3 + 1).is_none());
                    assert_out_of_memory();
                    // ffibox's own constructors reach the policy's `c_alloc_len`, which
                    // refuses a size past `isize::MAX` as well.
                    assert!(AllocVec::<u8>::new_uninit(isize::MAX as usize + 1).is_none());
                    assert_out_of_memory();
                    assert!(AllocVec::<u16>::from_fn(isize::MAX as usize / 2 + 1, |_| 0).is_none());
                    assert_out_of_memory();
                    // An overflow is C's to refuse, a size past `isize::MAX` the strategy's.
                    assert!(!exceeds_slice_bytes(isize::MAX as usize, 1));
                    assert!(exceeds_slice_bytes(isize::MAX as usize + 1, 1));
                    assert!(!exceeds_slice_bytes(usize::MAX, 2));
                    assert_eq!(requests(), (vec![], 0));
                },
            );
        }

        /// ffibox's constructors build an empty run as NULL: nothing is asked of libgit2,
        /// so Miri runs this.
        #[test]
        fn ffibox_empty_runs_allocate_nothing() {
            let c = AllocVec::<u16>::from_slice(&[]).unwrap();
            let d = AllocVec::<u32>::from_fn(0, |_| unreachable!()).unwrap();
            assert!(c.as_ptr().is_null() && d.as_ptr().is_null());
            assert!(c.clone().is_empty() && d.as_slice().is_empty());
        }

        /// An empty run built as `git__calloc(0, ..)` or `git__mallocarray(0, ..)` asks the
        /// allocator for zero bytes and holds the block until it is dropped; a refusal is
        /// out of memory, as C reads it; and a block too misaligned for the element type
        /// is released at once, the run held as NULL.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn c_empty_runs_ask_for_zero_bytes() {
            with_heap(
                concat!(module_path!(), "::c_empty_runs_ask_for_zero_bytes"),
                || {
                    let a = ArrayFree::new_zeroed::<u64>(0).unwrap();
                    let b = ArrayFree::new_uninit::<u8>(0).unwrap();
                    assert!(!a.as_ptr().is_null() && !b.as_ptr().is_null());
                    assert!(a.as_slice().is_empty() && b.as_slice().is_empty());
                    assert_ne!(a.as_ptr().cast::<u8>(), b.as_ptr().cast::<u8>());
                    drop((a, b));
                    let calls = take_calls();
                    assert!(
                        matches!(
                            calls[..],
                            [
                                Call::Malloc(0, ..),
                                Call::Realloc(false, 0, ..),
                                Call::Free,
                                Call::Free
                            ]
                        ),
                        "{calls:?}"
                    );

                    REFUSE_ZERO.store(true, Relaxed);
                    clear();
                    assert!(ArrayFree::new_zeroed::<u32>(0).is_none());
                    assert_out_of_memory();
                    assert!(ArrayFree::new_uninit::<u32>(0).is_none());
                    assert_out_of_memory();
                    REFUSE_ZERO.store(false, Relaxed);
                    take_calls();

                    ODD_ZERO.store(true, Relaxed);
                    let wide = ArrayFree::new_uninit::<u64>(0).unwrap();
                    assert!(wide.as_ptr().is_null());
                    assert_eq!(requests(), (vec![0], 1));
                    let narrow = ArrayFree::new_zeroed::<u8>(0).unwrap();
                    assert_eq!(narrow.as_ptr(), odd_block().as_ptr());
                    drop((wide, narrow));
                    ODD_ZERO.store(false, Relaxed);
                    assert_eq!(requests(), (vec![0], 1));
                },
            );
        }

        /// libgit2 releases what the strategies made, and they release what libgit2 made.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn libgit2_and_the_strategies_free_each_others_allocations() {
            with_heap(
                concat!(
                    module_path!(),
                    "::libgit2_and_the_strategies_free_each_others_allocations"
                ),
                || {
                    let s = StrFree::dup(c"handed over").unwrap().into_raw();
                    // SAFETY: `s` is a live allocation of libgit2's allocator, given up.
                    unsafe { ffi::git__free(s.cast()) };
                    // SAFETY: any size; a fresh 8-byte allocation or NULL.
                    let raw = unsafe { ffi::git__malloc(8) };
                    // SAFETY: `raw` is fresh storage of libgit2's allocator holding 8 bytes,
                    // which the run adopts as 8 `MaybeUninit<u8>`.
                    let run = unsafe { AllocVec::<MaybeUninit<u8>>::from_raw_parts(raw.cast(), 8) };
                    drop(run.unwrap());
                    // SAFETY: a live C string, read for the call.
                    let raw = unsafe { ffi::git__strdup(c"back".as_ptr()) };
                    // SAFETY: `raw` is a fresh NUL-terminated copy in libgit2's allocator,
                    // given up to the owner.
                    let back = unsafe { AllocStr::from_raw(raw) }.unwrap();
                    assert_eq!(back.as_c_str(), c"back");
                    drop(back);
                    // SAFETY: any sizes; zeroed storage for one `git_oid`, or NULL.
                    let raw = unsafe { ffi::git__calloc(1, size_of::<ffi::git_oid>()) };
                    // SAFETY: `raw` is fresh, zeroed storage of libgit2's allocator for one
                    // `git_oid`, given up to the box.
                    let id = unsafe { AllocBox::<Oid>::from_c(raw.cast()) }.unwrap();
                    assert!(id.as_ref().id().iter().all(|&b| b == 0));
                    drop(id);
                    assert_eq!(requests(), (vec![12, 8, 5, size_of::<ffi::git_oid>()], 4));
                },
            );
        }

        // --- the resizes ----------------------------------------------------------------------

        /// A run of `len` uninitialised `T`, filled with `fill(i)`; `len` is at most 256.
        fn filled<T: Copy>(len: usize, fill: impl Fn(u8) -> T) -> AllocVec<MaybeUninit<T>> {
            let mut run = ArrayFree::new_uninit::<T>(len).unwrap();
            for (i, slot) in (0..=u8::MAX).zip(run.as_mut_slice()) {
                slot.write(fill(i));
            }
            run
        }

        /// The first `n` elements of `run`, which the test wrote.
        fn prefix<T: Copy>(run: &AllocVec<MaybeUninit<T>>, n: usize) -> Vec<T> {
            run.as_slice()[..n]
                .iter()
                // SAFETY: the callers read only the elements they wrote or kept.
                .map(|e| unsafe { e.assume_init() })
                .collect()
        }

        /// Each resize reaches the allocator's `grealloc` as `alloc.c` does: with the run's
        /// storage, or NULL for a run without, and the size in bytes; resizing to zero is
        /// a release.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn resizes_request_what_alloc_c_requests() {
            with_heap(
                concat!(module_path!(), "::resizes_request_what_alloc_c_requests"),
                || {
                    let mut run = filled::<u32>(3, |i| u32::from(i) + 1);
                    take_calls();
                    // `git__reallocarray(ptr, 5, 4)`: one resize of 20 bytes, prefix kept.
                    ArrayFree::resize(&mut run, 5).unwrap();
                    assert_eq!(run.len(), 5);
                    assert_eq!(prefix(&run, 3), [1, 2, 3]);
                    // Shrinking keeps the first elements too.
                    ArrayFree::resize(&mut run, 2).unwrap();
                    assert_eq!(prefix(&run, 2), [1, 2]);
                    assert!(matches!(
                        take_calls()[..],
                        [Call::Realloc(true, 20, ..), Call::Realloc(true, 8, ..)]
                    ));
                    // To zero: released, and held without storage.
                    ArrayFree::resize(&mut run, 0).unwrap();
                    assert!(run.is_empty() && run.as_ptr().is_null());
                    assert_eq!(take_calls(), [Call::Free]);
                    // A run without storage, and one at zero, stay so unasked.
                    ArrayFree::resize(&mut run, 0).unwrap();
                    assert_eq!(take_calls(), []);
                    // Growing a run without storage is `grealloc(NULL, ..)`.
                    ArrayFree::resize(&mut run, 4).unwrap();
                    assert_eq!(run.len(), 4);
                    assert!(matches!(take_calls()[..], [Call::Realloc(false, 16, ..)]));
                    drop(run);
                    assert_eq!(take_calls(), [Call::Free]);

                    // `git__realloc(ptr, n)` on bytes; a zero-byte block of `new_uninit(0)`
                    // grows like any other.
                    let mut bytes = ArrayFree::new_uninit::<u8>(0).unwrap();
                    assert!(!bytes.as_ptr().is_null());
                    take_calls();
                    ArrayFree::resize_bytes(&mut bytes, 5).unwrap();
                    for (slot, b) in bytes.as_mut_slice().iter_mut().zip(*b"hello") {
                        slot.write(b);
                    }
                    ArrayFree::resize_bytes(&mut bytes, 9).unwrap();
                    assert_eq!(prefix(&bytes, 5), b"hello");
                    ArrayFree::resize_bytes(&mut bytes, 0).unwrap();
                    assert!(bytes.as_ptr().is_null());
                    let calls = take_calls();
                    assert!(
                        matches!(
                            calls[..],
                            [
                                Call::Realloc(true, 5, ..),
                                Call::Realloc(true, 9, ..),
                                Call::Free
                            ]
                        ),
                        "{calls:?}"
                    );
                    // The call site is `alloc.h`'s `git__realloc`, for both resizes.
                    ArrayFree::resize_bytes(&mut bytes, 1).unwrap();
                    let calls = take_calls();
                    let Call::Realloc(false, 1, Some(file), _) = &calls[0] else {
                        panic!("{calls:?}");
                    };
                    assert!(file.to_bytes().ends_with(b"src/util/alloc.h"), "{file:?}");
                },
            );
        }

        /// A refused resize leaves the run as it was, its storage, length and elements, and
        /// libgit2's out-of-memory error.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn refused_resizes_leave_the_run_as_it_was() {
            with_heap(
                concat!(module_path!(), "::refused_resizes_leave_the_run_as_it_was"),
                || {
                    let mut run = filled::<u16>(4, |i| u16::from(i) * 3);
                    let before = run.as_ptr();
                    clear();
                    let e = ArrayFree::resize(&mut run, REFUSED / 2).unwrap_err();
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    assert_out_of_memory();
                    assert_eq!((run.as_ptr(), run.len()), (before, 4));
                    assert_eq!(prefix(&run, 4), [0, 3, 6, 9]);

                    let mut bytes = filled::<u8>(3, |i| i);
                    let e = ArrayFree::resize_bytes(&mut bytes, REFUSED).unwrap_err();
                    assert_eq!(e.class(), Some(ErrorClass::NoMemory));
                    assert_out_of_memory();
                    assert_eq!(prefix(&bytes, 3), [0, 1, 2]);
                    // A run without storage stays without on a refusal.
                    let mut empty = AllocVec::<MaybeUninit<u8>>::empty();
                    assert!(ArrayFree::resize_bytes(&mut empty, REFUSED).is_err());
                    assert!(empty.as_ptr().is_null());
                    assert_out_of_memory();
                    drop((run, bytes));
                    let calls = take_calls();
                    assert_eq!(calls.iter().filter(|c| **c == Call::Free).count(), 2);
                },
            );
        }

        /// A size past `isize::MAX` is refused by the strategy, one that overflows by
        /// `git__reallocarray`, both before the allocator is asked.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn oversized_resizes_report_out_of_memory_unasked() {
            with_heap(
                concat!(
                    module_path!(),
                    "::oversized_resizes_report_out_of_memory_unasked"
                ),
                || {
                    let mut run = filled::<u32>(2, |_| 7);
                    let mut bytes = filled::<u8>(2, |_| 7);
                    clear();
                    take_calls();
                    for len in [isize::MAX as usize / 4 + 1, usize::MAX / 4 + 1, usize::MAX] {
                        let e = ArrayFree::resize(&mut run, len).unwrap_err();
                        assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{len}");
                        assert_out_of_memory();
                    }
                    let e = ArrayFree::resize_bytes(&mut bytes, isize::MAX as usize + 1);
                    assert_eq!(e.unwrap_err().class(), Some(ErrorClass::NoMemory));
                    assert_out_of_memory();
                    assert_eq!(requests(), (vec![], 0));
                    assert_eq!(
                        (prefix(&run, 2), prefix(&bytes, 2)),
                        (vec![7; 2], vec![7; 2])
                    );
                },
            );
        }

        // --- the installed allocator ---------------------------------------------------------

        /// Whether `a` holds the failing family's `gmalloc`, as libgit2 tells it.
        fn is_failing(a: AllocatorRef<'_>) -> bool {
            let failing: MallocFn = ffi::git_failalloc_malloc;
            ptr::fn_addr_eq(a.malloc().unwrap(), failing)
        }

        /// The global, borrowed without pinning the runtime, as [`installed`] borrows it once
        /// it has: what the global holds before initialisation.
        fn unpinned_global() -> AllocatorRef<'static> {
            // SAFETY: a C global, live and initialised for the whole process; the callers run
            // alone in a child process, where no other thread writes it.
            unsafe { AllocatorRef::from_ptr(&raw mut ffi::git__allocator) }.unwrap()
        }

        /// The global holds the failing family until `global_init` installs the standard
        /// allocator, which initialisation and later `global_init`s keep; the handle reads
        /// the live global throughout, and [`installed`]'s is the same handle.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn the_failing_family_gives_way_to_the_standard_one() {
            in_child(
                concat!(
                    module_path!(),
                    "::the_failing_family_gives_way_to_the_standard_one"
                ),
                || {
                    let global = unpinned_global();
                    assert!(is_failing(global));
                    // SAFETY: a process of its own, with no other thread: nothing reads or
                    // writes the global meanwhile; libgit2 holds no allocation before
                    // initialisation.
                    unsafe { global_init() };
                    assert!(!is_failing(global));
                    let standard = global.malloc().unwrap();
                    // SAFETY: as above; the standard family is installed, so nothing changes.
                    unsafe { global_init() };
                    let again = installed().unwrap();
                    assert!(runtime_pinned());
                    assert!(ptr::fn_addr_eq(global.malloc().unwrap(), standard));
                    assert_eq!(format!("{again:?}"), format!("{global:?}"));
                    // The standard family allocates: a strategy's request succeeds.
                    assert!(StrFree::dup(c"standard").is_some());
                },
            );
        }

        /// The first borrow initialises libgit2, which installs the standard allocator over
        /// the failing family: the borrow never sees the family libgit2 starts with.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn the_first_borrow_reads_the_global_as_initialisation_left_it() {
            in_child(
                concat!(
                    module_path!(),
                    "::the_first_borrow_reads_the_global_as_initialisation_left_it"
                ),
                || {
                    assert!(is_failing(unpinned_global()));
                    let global = installed().unwrap();
                    assert!(runtime_pinned());
                    assert!(!is_failing(global));
                    assert!(StrFree::dup(c"standard").is_some());
                },
            );
        }

        /// An allocator installed before initialisation stays: `global_init` keeps any
        /// family but the failing one.
        #[test]
        #[cfg_attr(miri, ignore = "spawns a process")]
        fn global_init_keeps_an_allocator_installed_first() {
            in_child(
                concat!(
                    module_path!(),
                    "::global_init_keeps_an_allocator_installed_first"
                ),
                || {
                    // SAFETY: `Heap` meets `Alloc`'s contract; installed before libgit2 holds
                    // any allocation, in a process of its own with no other thread.
                    unsafe { set_allocator(Some(HEAP.as_ref())) };
                    let global = unpinned_global();
                    let heap = HEAP.as_ref().malloc().unwrap();
                    assert!(ptr::fn_addr_eq(global.malloc().unwrap(), heap));
                    // SAFETY: as above; the Heap family is installed, so nothing changes.
                    unsafe { global_init() };
                    assert!(ptr::fn_addr_eq(global.malloc().unwrap(), heap));
                    // Initialisation keeps it too.
                    let pinned = installed().unwrap();
                    assert!(ptr::fn_addr_eq(pinned.malloc().unwrap(), heap));
                    // libgit2 now allocates through `Heap`.
                    take_calls();
                    drop(StrFree::dup(c"heap").unwrap());
                    assert_eq!(requests(), (vec![5], 1));
                },
            );
        }
    }
}
