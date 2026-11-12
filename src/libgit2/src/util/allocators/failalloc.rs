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
//! The family is a strategy here, as libgit2's other allocator families are:
//!
//! - [`FailAlloc`], the policy of singletons and strings: [`CBox::new_uninit`] under it returns
//!   `None`, and its release is a no-op; it is also the [`Alloc`] implementation from which
//!   [`Allocator::new`] builds a `git_allocator` value over the family; and
//! - [`FailArrayAlloc`], the policy of counted arrays: [`CVec::new_uninit`] and [`CVec::from_fn`]
//!   under it return `None` for every non-empty request.
//!
//! ```
//! use ffibox::{CBox, CVec};
//! use libgit2::libgit2::src::util::alloc::Allocator;
//! use libgit2::libgit2::src::util::allocators::failalloc::{FailAlloc, FailArrayAlloc};
//!
//! # ffibox::define_ctype!(Point, PointRef, PointMut, point_st);
//! # #[repr(C)] pub struct point_st { x: i32 }
//! assert!(CBox::<Point, FailAlloc>::new_uninit().is_none());
//! assert!(CVec::<u64, FailArrayAlloc>::from_fn(3, |i| i as u64).is_none());
//! // An empty array needs no storage, so nothing is asked of the allocator.
//! assert!(CVec::<u64, FailArrayAlloc>::from_fn(0, |i| i as u64).is_some());
//!
//! // The `git_allocator` over the family, to install where every allocation must fail.
//! static FAIL: Allocator = Allocator::new::<FailAlloc>();
//! assert!(FAIL.as_ref().malloc().is_some());
//! ```
//!
//! # The C routines are reproduced, not called
//!
//! libgit2 declares the three routines `extern`, not `GIT_EXTERN`, and compiles under
//! `-fvisibility=hidden`, so `libgit2.so` exports none of them and no raw binding can
//! link them. Each body ignores every argument and returns a constant, so the strategy
//! reproduces them in Rust, and its methods carry `Replaces` anchors; the C copies stay,
//! `git__allocator`'s initial value.
//!
//! One thing tells the two apart: their addresses. `git_allocator_global_init` keeps
//! whatever allocator is installed unless its `gmalloc` is C's own
//! `git_failalloc_malloc`, so an [`Allocator`] built over [`FailAlloc`] and installed
//! before `git_libgit2_init` survives it, where C's initial value is replaced: libgit2
//! then runs on the failing family until another allocator is installed. Either family,
//! before initialisation, also fails into libgit2's error reporting, which reads
//! thread state libgit2 has not set up yet: only a release is safe to make then.
//!
//! [`CBox::new_uninit`]: ffibox::CBox::new_uninit
//! [`CVec::new_uninit`]: ffibox::CVec::new_uninit
//! [`CVec::from_fn`]: ffibox::CVec::from_fn
//! [`Alloc`]: crate::libgit2::src::util::alloc::Alloc
//! [`Allocator`]: crate::libgit2::src::util::alloc::Allocator
//! [`Allocator::new`]: crate::libgit2::src::util::alloc::Allocator::new

pub use ffi_import::{FailAlloc, FailArrayAlloc};

#[expect(
    unsafe_code,
    reason = "the policies implement ffibox's and `Alloc`'s unsafe lifecycle traits"
)]
mod ffi_import {
    use core::ffi::CStr;
    use core::mem::MaybeUninit;
    use core::ptr::NonNull;

    use ffibox::{CAlloc, CCell, CDrop, CLenAlloc, CLenDrop};

    use crate::libgit2::src::util::alloc::Alloc;

    /// The failing allocator family as the policy of singletons and strings, and as the
    /// [`Alloc`] implementation of a `git_allocator` over it.
    ///
    /// It never allocates: [`CAlloc::c_alloc`] and every [`Alloc`] allocation return
    /// `None`. Its release is a no-op, which is the release of everything it allocated:
    /// nothing. An owner under it therefore never holds storage: a safe constructor
    /// fails, and only an `unsafe` adoption, whose contract asks for storage this family
    /// made, could build one.
    ///
    /// Stateless, so it is `Send` and `Sync`: the family has no thread to free on.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FailAlloc;

    /// The failing allocator family as the policy of counted arrays.
    ///
    /// [`CLenAlloc::c_alloc_len`] returns `None` for every request, and its release is a
    /// no-op. An empty array asks the allocator for nothing, so it builds as under any
    /// policy, as NULL.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct FailArrayAlloc;

    // SAFETY: this family makes no allocation: `c_alloc`, `c_alloc_len` and every
    // `Alloc` call below return `None`. The `T` a caller may hand `c_drop` under this
    // policy is one this family allocated, of which there are none, so releasing nothing
    // releases all of it, exactly once; `self` carries no state.
    unsafe impl<T> CDrop<T> for FailAlloc {
        /// Releases nothing: the family never allocated `ptr`.
        ///
        /// Replaces `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn c_drop(&self, ptr: NonNull<T>) {
            let _ = ptr;
        }
    }

    // SAFETY: `c_alloc` never returns `Some`, so the obligations on the storage it hands
    // out hold vacuously, and `CDrop<MaybeUninit<T>>` above reads nothing.
    unsafe impl<T: CCell> CAlloc<T> for FailAlloc {
        /// Always `None`: the family fails every allocation.
        ///
        /// Replaces `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
            None
        }
    }

    // SAFETY: `malloc` and `realloc` always return `None`, which `Alloc` allows: for
    // `realloc` it leaves `ptr` live and unchanged. `free` is handed only allocations of
    // this implementation, of which there are none, and releases nothing. The three
    // carry no state, so they run on any thread at once, and none calls into libgit2.
    unsafe impl Alloc for FailAlloc {
        /// Always `None`, whatever `len`: the family fails every allocation.
        ///
        /// Replaces `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
            let _ = (len, file, line);
            None
        }

        /// Always `None`, leaving `ptr` live and the caller's: the family fails every
        /// allocation.
        ///
        /// Replaces `fn::git_failalloc_realloc`
        #[doc(alias = "git_failalloc_realloc")]
        #[inline]
        unsafe fn realloc(
            ptr: Option<NonNull<u8>>,
            len: usize,
            file: Option<&CStr>,
            line: i32,
        ) -> Option<NonNull<u8>> {
            let _ = (ptr, len, file, line);
            None
        }

        /// Releases nothing: the family never allocated `ptr`.
        ///
        /// Replaces `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn free(ptr: NonNull<u8>) {
            let _ = ptr;
        }
    }

    // SAFETY: as `CDrop` above: `c_alloc_len` makes no buffer, so the only buffers this
    // family can be handed are none, and releasing nothing touches no byte of them.
    unsafe impl CLenDrop for FailArrayAlloc {
        /// Releases nothing: the family never allocated `ptr`.
        ///
        /// Replaces `fn::git_failalloc_free`
        #[doc(alias = "git_failalloc_free")]
        #[inline]
        unsafe fn c_drop_len(&self, ptr: *mut u8, byte_len: usize) {
            let _ = (ptr, byte_len);
        }
    }

    // SAFETY: `c_alloc_len` never returns `Some`, so the storage obligations and
    // `ALIGN` hold vacuously, and its `None` always means the allocation failed.
    unsafe impl CLenAlloc for FailArrayAlloc {
        /// The alignment libgit2's allocator families vouch for, glibc `malloc`'s
        /// `max_align_t`, so the element types every other family takes are taken here
        /// too. Vacuous: no buffer is ever returned.
        const ALIGN: usize = 16;

        /// Always `None`, whatever `byte_len`: the family fails every allocation.
        ///
        /// Replaces `fn::git_failalloc_malloc`
        #[doc(alias = "git_failalloc_malloc")]
        #[inline]
        fn c_alloc_len(&self, byte_len: usize) -> Option<NonNull<u8>> {
            let _ = byte_len;
            None
        }
    }
}
