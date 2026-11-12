//! `src/util/allocators/debugalloc.c`: the debug allocator libgit2 runs on when built with
//! `DEBUG_STRICT_ALLOC`, which records each block's size before it. A foundational module:
//! its routines are bound as strategies and stay C.
//!
//! The module defines one family of `malloc`, `realloc` and `free`, private to its
//! translation unit, and one routine that hands it out, `git_debugalloc_init_allocator`,
//! which writes the three routines into a `git_allocator`. That routine is bound as the
//! constructor of the [`Allocator`] value over the family, [`Allocator::debug`]: the value
//! holds C's routines, behind guards that refuse the sizes C mishandles, and
//! [`set_allocator`] installs it as libgit2's allocator.
//!
//! ```
//! use libgit2::libgit2::src::util::alloc::Allocator;
//!
//! let debug = Allocator::debug();
//! let free = debug.as_ref().free().unwrap();
//! // The same routines every time: the family has no state.
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
//! # Sizes past `isize::MAX`
//!
//! C adds the word to the size asked for unchecked, so a request within a word of
//! `usize::MAX` wraps the block's size: `malloc` is asked for fewer than 8 bytes, the length
//! is written past their end, and the allocation returned spans none of the bytes asked for.
//! libgit2 makes such requests from the bytes it reads: a loose object whose header names a
//! size near `usize::MAX` has `git_str_init` ask for it. So the value does not hold C's
//! `malloc` and `realloc` themselves: its members are two guards that refuse a request past
//! `isize::MAX` bytes with NULL, as a failed allocation, and pass every other request to the
//! routine C wrote, unchanged; its `free` is C's own. Nothing changes for the sizes C does not
//! wrap, since `malloc` refuses `len + 8` bytes past `isize::MAX` too, and no Rust allocation
//! spans more than `isize::MAX` bytes.
//!
//! # `Alloc`'s contract
//!
//! [`set_allocator`]'s contract asks for a family meeting [`Alloc`]'s, which this value
//! does:
//!
//! - **Alignment.** An allocation starts a word past `malloc`'s block, so it is aligned to `usize`
//!   (8 bytes here), not to `max_align_t` (16). That is what [`Alloc`] asks for: libgit2's own C is
//!   written for it, since its CI runs on this family, and so is every owner of this crate over
//!   libgit2's allocator, whose strategies refuse wider types at compile time
//!   (`crustify/LESSONS.md`).
//! - **Zero bytes.** A zero-byte request fails, which [`Alloc`] allows `malloc(0)` to do.
//! - **Sizes.** An allocation spans at least the bytes asked for: the guards above refuse the sizes
//!   C would wrap.
//!
//! Blocks are not interchangeable with another family's: releasing a block of the standard
//! allocator through this family, or one of this family's through the standard allocator,
//! hands `free` a pointer `malloc` never returned. So the family is installed before
//! libgit2 initialises, and is then kept: initialisation keeps any allocator installed
//! before it (`git_allocator_global_init`), and from then on libgit2 holds this family's
//! blocks. libgit2 configured with `DEBUG_STRICT_ALLOC` installs it itself, as its default,
//! without the guards.
//!
//! [`Allocator`]: crate::libgit2::src::util::alloc::Allocator
//! [`Allocator::debug`]: crate::libgit2::src::util::alloc::Allocator::debug
//! [`Alloc`]: crate::libgit2::src::util::alloc::Alloc
//! [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator

#[expect(unsafe_code, reason = "calls the family's C initialiser and routines")]
mod ffi_import {
    use core::ffi::{c_char, c_int, c_void};
    use core::ptr;

    use crate::ffi;
    use crate::libgit2::src::util::alloc::Allocator;

    /// The largest request the guards pass to C: no Rust allocation spans more, and C's
    /// `len + size_of::<usize>()` does not wrap up to it.
    const MAX_REQUEST: usize = isize::MAX.unsigned_abs();

    /// The family's routines, as `git_debugalloc_init_allocator` writes them, and its
    /// status.
    fn family() -> (c_int, ffi::git_allocator) {
        let mut raw = ffi::git_allocator::default();
        // SAFETY: `raw` is a live, writable `git_allocator`, which the routine only
        // writes: it stores its three routines in `gmalloc`, `grealloc` and `gfree`,
        // keeps no pointer to it, touches no other state, allocates nothing, and returns 0.
        let rc = unsafe { ffi::git_debugalloc_init_allocator(&raw mut raw) };
        (rc, raw)
    }

    /// `gmalloc`: `debugalloc__malloc`, a request past [`MAX_REQUEST`] refused with NULL
    /// first.
    ///
    /// # Safety
    ///
    /// As [`MallocFn`](crate::libgit2::src::util::alloc::MallocFn).
    unsafe extern "C" fn guarded_malloc(
        len: usize,
        file: *const c_char,
        line: c_int,
    ) -> *mut c_void {
        if len > MAX_REQUEST {
            return ptr::null_mut();
        }
        let (_, raw) = family();
        let Some(malloc) = raw.gmalloc else {
            return ptr::null_mut();
        };
        // SAFETY: `MallocFn`'s contract, which the caller keeps, is the routine's: `file`
        // is NULL or a C string live for the call, which it ignores; `len` is at most
        // `isize::MAX`, so the block size C adds up does not wrap.
        unsafe { malloc(len, file, line) }
    }

    /// `grealloc`: `debugalloc__realloc`, a request past [`MAX_REQUEST`] refused with NULL
    /// first, which leaves `ptr` live, as a failed resize does.
    ///
    /// # Safety
    ///
    /// As [`ReallocFn`](crate::libgit2::src::util::alloc::ReallocFn): `ptr` is NULL or a
    /// live block of this family.
    unsafe extern "C" fn guarded_realloc(
        ptr: *mut c_void,
        len: usize,
        file: *const c_char,
        line: c_int,
    ) -> *mut c_void {
        if len > MAX_REQUEST {
            return ptr::null_mut();
        }
        let (_, raw) = family();
        let Some(realloc) = raw.grealloc else {
            return ptr::null_mut();
        };
        // SAFETY: `ReallocFn`'s contract, which the caller keeps: `ptr` is NULL or a live
        // block of this family, whose blocks C's `malloc` made through the guard above, given
        // up on success, and `len` is non-zero when `ptr` is not NULL; `len` is at most
        // `isize::MAX`, so the block size C adds up does not wrap.
        unsafe { realloc(ptr, len, file, line) }
    }

    impl Allocator {
        /// libgit2's debug allocator family (`git_debugalloc_init_allocator`): `malloc`,
        /// `realloc` and `free` over C's, each block prefixed with its size.
        ///
        /// The value holds C's `free` and two guards over C's `malloc` and `realloc`, which
        /// refuse a request past `isize::MAX` bytes, whose block size C would wrap, and
        /// otherwise call C's routine unchanged: the same three functions on every call.
        /// Building it touches no state of libgit2, so it needs no initialisation, on any
        /// thread. What the routines do, and how they meet
        /// [`Alloc`](crate::libgit2::src::util::alloc::Alloc)'s contract, which installing
        /// them with
        /// [`set_allocator`](crate::libgit2::src::libgit2::settings::set_allocator)
        /// relies on, is in [the module's
        /// documentation](crate::libgit2::src::util::allocators::debugalloc): its storage
        /// is aligned to `usize`, and it fails every zero-byte request.
        ///
        /// Wraps `fn::git_debugalloc_init_allocator`
        #[doc(alias = "git_debugalloc_init_allocator")]
        #[must_use]
        pub fn debug() -> Self {
            let (rc, raw) = family();
            debug_assert_eq!(rc, 0);
            let Some(free) = raw.gfree else {
                unreachable!("git_debugalloc_init_allocator sets every member");
            };
            Self::from_fns(guarded_malloc, guarded_realloc, free)
        }
    }

    #[cfg(test)]
    mod unit_tests {
        use core::ffi::c_void;
        use core::ptr;

        use super::{MAX_REQUEST, guarded_malloc, guarded_realloc};

        /// The sizes C would wrap, and the first past `isize::MAX`, which `malloc` refuses.
        const REFUSED: [usize; 4] = [
            usize::MAX,
            usize::MAX - 7,
            usize::MAX - size_of::<usize>() + 1,
            MAX_REQUEST + 1,
        ];

        #[test]
        fn guards_refuse_sizes_past_isize_max_before_c() {
            for len in REFUSED {
                // SAFETY: a NULL `file`; the guard returns before reaching C.
                let block = unsafe { guarded_malloc(len, ptr::null(), 0) };
                assert!(block.is_null(), "{len}");
                // SAFETY: a NULL `ptr` allocates afresh; refused as above.
                let fresh = unsafe { guarded_realloc(ptr::null_mut(), len, ptr::null(), 0) };
                assert!(fresh.is_null(), "{len}");
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_refused_resize_keeps_the_block_and_its_length_word() {
            // SAFETY: a NULL `file`; 16 bytes.
            let block = unsafe { guarded_malloc(16, ptr::null(), 0) }.cast::<u8>();
            assert!(!block.is_null());
            // SAFETY: a fresh block of 16 bytes, this test's.
            unsafe { block.write_bytes(0x5A, 16) };
            for len in REFUSED {
                // SAFETY: `block` is a live block of the family, kept on failure.
                let resized = unsafe { guarded_realloc(block.cast(), len, ptr::null(), 0) };
                assert!(resized.is_null(), "{len}");
            }
            // SAFETY: the word before the block is the start of `malloc`'s block, in the same
            // allocation.
            let word = unsafe { block.sub(size_of::<usize>()) };
            // SAFETY: as above; C wrote the word with `memcpy`, and bytes need no alignment.
            let word = unsafe { word.cast::<[u8; size_of::<usize>()]>().read() };
            assert_eq!(usize::from_ne_bytes(word), 16);
            // SAFETY: the block's 16 bytes, written above and still live.
            let bytes = unsafe { core::slice::from_raw_parts(block, 16) };
            assert_eq!(bytes, &[0x5A; 16]);
            // Grown within range, through the guard, the bytes are kept.
            // SAFETY: `block` is live and given up on success; 32 is non-zero.
            let grown = unsafe { guarded_realloc(block.cast(), 32, ptr::null(), 0) }.cast::<u8>();
            assert!(!grown.is_null());
            // SAFETY: the grown block's first 16 bytes, which C copied.
            let kept = unsafe { core::slice::from_raw_parts(grown, 16) };
            assert_eq!(kept, &[0x5A; 16]);
            let (_, raw) = super::family();
            // SAFETY: `grown` is a live block of the family, given up.
            unsafe { raw.gfree.unwrap()(grown.cast::<c_void>()) };
        }
    }
}
