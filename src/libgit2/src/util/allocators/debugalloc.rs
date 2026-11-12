//! `src/util/allocators/debugalloc.c`: the debug allocator libgit2 runs on when built with
//! `DEBUG_STRICT_ALLOC`, which records each block's size before it. A foundational module:
//! its routines are bound as strategies and stay C.
//!
//! The module defines one family of `malloc`, `realloc` and `free`, private to its
//! translation unit, and one routine that hands it out, `git_debugalloc_init_allocator`,
//! which writes the three routines into a `git_allocator`. That routine is bound as the
//! constructor of the [`Allocator`] value over the family, [`Allocator::debug`]: the value
//! holds C's own routines, which [`set_allocator`] installs as libgit2's allocator.
//!
//! ```
//! use libgit2::libgit2::src::util::alloc::Allocator;
//!
//! let debug = Allocator::debug();
//! let free = debug.as_ref().free().unwrap();
//! // The same three routines every time: the family has no state.
//! assert!(core::ptr::fn_addr_eq(
//!     Allocator::debug().as_ref().free().unwrap(),
//!     free
//! ));
//! ```
//!
//! The value is held by the caller, and its handles borrow it:
//!
//! ```compile_fail,E0716
//! use libgit2::libgit2::src::util::alloc::Allocator;
//!
//! let handle = Allocator::debug().as_ref();
//! let _ = handle.malloc();
//! ```
//!
//! # The family
//!
//! Each block is `malloc`'s, `size_of::<usize>()` bytes longer than asked for: its first
//! word holds the size asked for, and the allocation handed out starts past it.
//!
//! - `malloc(len)` returns NULL for `len == 0`, as for a failed `malloc`, and otherwise the
//!   allocation past the word. libgit2 reads NULL as out of memory, so under this family it fails
//!   every zero-byte request (`git__calloc(0, ..)`, `git__mallocarray(0, ..)`), which the standard
//!   allocator serves.
//! - `realloc(ptr, len)` never resizes in place: it allocates a new block, copies the first
//!   `min(old, len)` bytes, fills the old block, word included, with `0xfd`, and releases it. A
//!   failed allocation returns NULL and leaves `ptr` live. `realloc(NULL, 0)` returns NULL;
//!   `realloc(ptr, 0)` releases `ptr` and returns NULL, as glibc's does.
//! - `free(ptr)` releases the block `ptr` starts past; `free(NULL)` does nothing.
//!
//! The routines read and write no state of libgit2 and keep none of their own, so they run
//! on any thread, before `git_libgit2_init` too, as `malloc` and `free` do.
//!
//! # Where it departs from `Alloc`
//!
//! [`set_allocator`]'s contract asks for a family meeting [`Alloc`]'s, and this one departs
//! from it twice:
//!
//! - **Alignment.** An allocation starts a word past `malloc`'s block, so it is aligned to `usize`
//!   (8 bytes here), not to `max_align_t` (16). libgit2's own C is written for it, since its CI
//!   runs on this family, and so is every owner of this crate over libgit2's allocator: their
//!   strategies vouch for the alignment of a `usize` or a `u64` and refuse wider types at compile
//!   time (`crustify/LESSONS.md`).
//! - **Sizes near `usize::MAX`.** The block's size is `len + size_of::<usize>()` unchecked, so a
//!   request within a word of `usize::MAX` wraps it: `malloc` is asked for fewer than 8 bytes, the
//!   length is written past their end and the allocation returned spans none of the `len` bytes
//!   asked for; `realloc` then copies the old allocation past the new block's end. This is a defect
//!   of the C, which no equivalent of the Rust here copies. This crate's strategies never ask for
//!   more than `isize::MAX` bytes; installing the family vouches that nothing else asks it for such
//!   a size either.
//!
//! Blocks are not interchangeable with another family's: releasing a block of the standard
//! allocator through this family, or one of this family's through the standard allocator,
//! hands `free` a pointer `malloc` never returned. So the family is installed before
//! libgit2 initialises, and is then kept: initialisation keeps any allocator installed
//! before it (`git_allocator_global_init`), and from then on libgit2 holds this family's
//! blocks. libgit2 configured with `DEBUG_STRICT_ALLOC` installs it itself, as its default.
//!
//! [`Allocator`]: crate::libgit2::src::util::alloc::Allocator
//! [`Allocator::debug`]: crate::libgit2::src::util::alloc::Allocator::debug
//! [`Alloc`]: crate::libgit2::src::util::alloc::Alloc
//! [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator

#[expect(unsafe_code, reason = "calls the family's C initialiser")]
mod ffi_import {
    use crate::ffi;
    use crate::libgit2::src::util::alloc::Allocator;

    impl Allocator {
        /// libgit2's debug allocator family (`git_debugalloc_init_allocator`): `malloc`,
        /// `realloc` and `free` over C's, each block prefixed with its size.
        ///
        /// The value holds the family's three C routines themselves, the same ones on
        /// every call, as libgit2 configured with `DEBUG_STRICT_ALLOC` installs them by
        /// default. Building it touches no state of libgit2, so it needs no
        /// initialisation, on any thread. What the routines do, and where they depart
        /// from [`Alloc`](crate::libgit2::src::util::alloc::Alloc)'s contract, which
        /// installing them with
        /// [`set_allocator`](crate::libgit2::src::libgit2::settings::set_allocator)
        /// relies on, is in [the module's
        /// documentation](crate::libgit2::src::util::allocators::debugalloc): its storage
        /// is aligned to `usize` only, and it fails every zero-byte request.
        ///
        /// Its members add one obligation to those of
        /// [`MallocFn`](crate::libgit2::src::util::alloc::MallocFn) and
        /// [`ReallocFn`](crate::libgit2::src::util::alloc::ReallocFn): the size asked for
        /// is at most `usize::MAX - size_of::<usize>()`, which C adds to it unchecked.
        ///
        /// Wraps `fn::git_debugalloc_init_allocator`
        #[doc(alias = "git_debugalloc_init_allocator")]
        #[must_use]
        pub fn debug() -> Self {
            let mut raw = ffi::git_allocator::default();
            // SAFETY: `raw` is a live, writable `git_allocator`, which the routine only
            // writes: it stores its three routines in `gmalloc`, `grealloc` and `gfree`,
            // keeps no pointer to it, touches no other state, and returns 0.
            let rc = unsafe { ffi::git_debugalloc_init_allocator(&raw mut raw) };
            debug_assert_eq!(rc, 0);
            let ffi::git_allocator {
                gmalloc: Some(malloc),
                grealloc: Some(realloc),
                gfree: Some(free),
            } = raw
            else {
                unreachable!("git_debugalloc_init_allocator sets every member");
            };
            Self::from_fns(malloc, realloc, free)
        }
    }
}
