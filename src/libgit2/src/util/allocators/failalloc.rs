//! `src/util/allocators/failalloc.c`: the allocator whose every call fails, which
//! libgit2 installs before initialisation. A foundational module: its routines are
//! bound as strategies and stay C.
//!
//! `git__allocator` starts out as `{git_failalloc_malloc, git_failalloc_realloc,
//! git_failalloc_free}`, so that every allocation made before `git_libgit2_init` fails,
//! until `git_allocator_global_init` replaces it with the standard allocator. The three
//! routines ignore their arguments: `malloc` and `realloc` return NULL, whatever the
//! size, and `free` releases nothing.
//!
//! The family is a strategy here, as libgit2's other allocator families are, each of its
//! routines called through its raw binding:
//!
//! - [`FailAlloc`], the policy of singletons and strings: [`CBox::new_uninit`] under it returns
//!   `None`, and its release is `git_failalloc_free`; it is also the [`Alloc`] implementation of
//!   the family, from which [`Allocator::new`] builds the `git_allocator` C installs; and
//! - [`FailArrayAlloc`], the policy of counted arrays: [`CVec::new_uninit`] and [`CVec::from_fn`]
//!   under it return `None` for every non-empty request.
//!
//! ```
//! use ffibox::{CBox, CVec};
//! use libgit2::ffi;
//! use libgit2::libgit2::src::util::alloc::{Allocator, MallocFn};
//! use libgit2::libgit2::src::util::allocators::failalloc::{FailAlloc, FailArrayAlloc};
//!
//! # ffibox::define_ctype!(Point, PointRef, PointMut, point_st);
//! # #[repr(C)] pub struct point_st { x: i32 }
//! assert!(CBox::<Point, FailAlloc>::new_uninit().is_none());
//! assert!(CVec::<u64, FailArrayAlloc>::from_fn(3, |i| i as u64).is_none());
//! // An empty array needs no storage, so nothing is asked of the allocator.
//! assert!(CVec::<u64, FailArrayAlloc>::from_fn(0, |i| i as u64).is_some());
//!
//! // The `git_allocator` over the family holds C's own routines, as `git__allocator`'s
//! // initial value does.
//! static FAIL: Allocator = Allocator::new::<FailAlloc>();
//! let malloc: MallocFn = ffi::git_failalloc_malloc;
//! assert!(core::ptr::fn_addr_eq(
//!     FAIL.as_ref().malloc().unwrap(),
//!     malloc
//! ));
//! ```
//!
//! # C's own addresses
//!
//! libgit2 tells this family apart from every other by address:
//! `git_allocator_global_init` keeps whatever allocator is installed unless its `gmalloc`
//! is `git_failalloc_malloc`. [`Allocator::new`] over [`FailAlloc`] therefore stores the
//! three C routines themselves, through [`Alloc::C_ALLOCATOR`], rather than trampolines
//! into the Rust methods, so that the value behaves as C's initial one does. Installed
//! before `git_libgit2_init`, initialisation replaces it with the standard allocator;
//! installed after it, every later allocation fails until another allocator is
//! installed.
//!
//! Before initialisation, a failed allocation made through libgit2 (`git__malloc`,
//! `git__realloc`) reports itself through libgit2's error state, which reads thread state
//! libgit2 has not set up yet: only a release is safe to make through it then. The
//! routines called directly, as these strategies call them, touch no state of libgit2.
//!
//! [`CBox::new_uninit`]: ffibox::CBox::new_uninit
//! [`CVec::new_uninit`]: ffibox::CVec::new_uninit
//! [`CVec::from_fn`]: ffibox::CVec::from_fn
//! [`Alloc`]: crate::libgit2::src::util::alloc::Alloc
//! [`Alloc::C_ALLOCATOR`]: crate::libgit2::src::util::alloc::Alloc::C_ALLOCATOR
//! [`Allocator::new`]: crate::libgit2::src::util::alloc::Allocator::new

pub use ffi_import::{FailAlloc, FailArrayAlloc};

#[expect(
    unsafe_code,
    reason = "the policies call the family's C routines, and implement ffibox's and `Alloc`'s \
              unsafe lifecycle traits"
)]
mod ffi_import {
    use core::ffi::CStr;
    use core::mem::MaybeUninit;
    use core::ptr::{self, NonNull};

    use ffibox::{CAlloc, CCell, CDrop, CLenAlloc, CLenDrop};

    use crate::ffi;
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};

    /// The failing allocator family as the policy of singletons and strings, and as the
    /// [`Alloc`] implementation of a `git_allocator` over it.
    ///
    /// It never allocates: [`CAlloc::c_alloc`] and every [`Alloc`] allocation return
    /// `None`, as `git_failalloc_malloc` and `git_failalloc_realloc` do. Its release is
    /// `git_failalloc_free`, which releases what the family allocated: nothing. An owner
    /// under it therefore never holds storage: a safe constructor fails, and only an
    /// `unsafe` adoption, whose contract asks for storage this family made, could build
    /// one.
    ///
    /// Stateless, so it is `Send` and `Sync`: the family keeps no state, in Rust or in
    /// C, and has no thread to free on.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FailAlloc;

    /// The failing allocator family as the policy of counted arrays.
    ///
    /// [`CLenAlloc::c_alloc_len`] is `git_failalloc_malloc`, which returns `None` for every
    /// request, and its release `git_failalloc_free`. An empty array asks the allocator
    /// for nothing, so it builds as under any policy, as NULL.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FailArrayAlloc;

    /// `git_failalloc_malloc` for `len` bytes, the call site left unnamed, which the
    /// routine ignores.
    fn malloc(len: usize) -> Option<NonNull<u8>> {
        // SAFETY: a NULL `file` is one `git_failalloc_malloc` accepts (it reads none of
        // its arguments); it touches no state, so it is callable on any thread, before
        // `git_libgit2_init` too.
        let p = unsafe { ffi::git_failalloc_malloc(len, ptr::null(), 0) };
        NonNull::new(p.cast())
    }

    /// `git_failalloc_free` on `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live allocation of this family that the caller owns and does
    /// not use again.
    unsafe fn free(ptr: *mut u8) {
        // SAFETY: per this function's contract, `ptr` is NULL or one of the family's
        // allocations, handed over: what `git_failalloc_free`, the family's `gfree`,
        // releases.
        unsafe { ffi::git_failalloc_free(ptr.cast()) }
    }

    // SAFETY: `c_drop` hands `git_failalloc_free`, the family's release, a `T` this family
    // allocated, which it releases exactly once without reading it (it reads nothing);
    // `self` carries no state.
    unsafe impl<T> CDrop<T> for FailAlloc {
        /// Releases `ptr` through `git_failalloc_free`, which releases nothing: the family
        /// never allocated it.
        ///
        /// Wraps `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<T>) {
            // SAFETY: per `c_drop`'s contract, `ptr` is a live `T` of this family whose
            // claim the caller hands over.
            unsafe { free(ptr.as_ptr().cast()) }
        }
    }

    // SAFETY: `c_alloc` returns what `git_failalloc_malloc` returns, and its body returns
    // NULL for every size: `c_alloc` never returns `Some`, so the obligations on the
    // storage it hands out (size, alignment for `T`, uniqueness) hold vacuously, and
    // `CDrop<MaybeUninit<T>>` above reads nothing.
    unsafe impl<T: CCell> CAlloc<T> for FailAlloc {
        /// Always `None`: `git_failalloc_malloc` fails every allocation.
        ///
        /// Wraps `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
            malloc(size_of::<T>()).map(NonNull::cast)
        }
    }

    // SAFETY: the three methods are the family's C routines, which meet `Alloc`'s contract
    // as `git__allocator`'s initial value: `malloc` and `realloc` always return NULL, which
    // `Alloc` allows (for `realloc` it leaves `ptr` live and unchanged, since the routine
    // reads nothing), and `free` releases nothing. They keep no state, so they run on any
    // thread at once, and none allocates through libgit2 or touches its state, the libgit2
    // calls `Alloc` admits. `C_ALLOCATOR` holds those same three routines, one family over
    // the methods' storage, of which there is none.
    unsafe impl Alloc for FailAlloc {
        /// The family's C routines, so that a value [`Allocator::new`] builds over the
        /// family holds C's own addresses, which `git_allocator_global_init` compares.
        const C_ALLOCATOR: Option<Allocator> = Some(Allocator::from_fns(
            ffi::git_failalloc_malloc,
            ffi::git_failalloc_realloc,
            ffi::git_failalloc_free,
        ));

        /// Always `None`, whatever `len`: `git_failalloc_malloc` fails every allocation.
        ///
        /// Wraps `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
            let file = file.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: `file` is NULL or a C string borrowed for the call; the routine reads
            // none of its arguments and touches no state.
            let p = unsafe { ffi::git_failalloc_malloc(len, file, line) };
            NonNull::new(p.cast())
        }

        /// Always `None`, leaving `ptr` live and the caller's: `git_failalloc_realloc`
        /// fails every allocation.
        ///
        /// Wraps `fn::git_failalloc_realloc`
        #[doc(alias = "git_failalloc_realloc")]
        #[inline]
        unsafe fn realloc(
            ptr: Option<NonNull<u8>>,
            len: usize,
            file: Option<&CStr>,
            line: i32,
        ) -> Option<NonNull<u8>> {
            let ptr = ptr.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            let file = file.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: per `realloc`'s contract, `ptr` is NULL or a live allocation of this
            // family, which `git_failalloc_realloc` borrows without reading and leaves
            // live on failure, its only outcome; `file` as in `malloc`.
            let p = unsafe { ffi::git_failalloc_realloc(ptr, len, file, line) };
            NonNull::new(p.cast())
        }

        /// Releases `ptr` through `git_failalloc_free`, which releases nothing: the family
        /// never allocated it.
        ///
        /// Wraps `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn free(ptr: NonNull<u8>) {
            // SAFETY: per `free`'s contract, `ptr` is a live allocation of this family,
            // handed over.
            unsafe { free(ptr.as_ptr()) }
        }
    }

    // SAFETY: as `CDrop` above: `git_failalloc_free` releases a buffer of this family
    // exactly once and reads no byte of it.
    unsafe impl CLenDrop for FailArrayAlloc {
        /// Releases `ptr` through `git_failalloc_free`, which releases nothing: the family
        /// never allocated it.
        ///
        /// Wraps `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, byte_len: usize) {
            let _ = byte_len;
            // SAFETY: per `c_drop_len`'s contract, `ptr` is a buffer of this family whose
            // claim the caller hands over.
            unsafe { free(ptr) }
        }
    }

    // SAFETY: as `CAlloc` above: `c_alloc_len` returns what `git_failalloc_malloc`
    // returns, NULL for every size, so the storage obligations and `ALIGN` hold vacuously,
    // and its `None` always means the allocation failed.
    unsafe impl CLenAlloc for FailArrayAlloc {
        /// The alignment libgit2's allocator families vouch for, glibc `malloc`'s
        /// `max_align_t`, so the element types every other family takes are taken here
        /// too. Vacuous: no buffer is ever returned.
        const ALIGN: usize = 16;

        /// Always `None`, whatever `byte_len`: `git_failalloc_malloc` fails every
        /// allocation.
        ///
        /// Wraps `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn c_alloc_len(&self, byte_len: usize) -> Option<NonNull<u8>> {
            malloc(byte_len)
        }
    }
}
