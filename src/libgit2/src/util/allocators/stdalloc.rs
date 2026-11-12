//! `src/util/allocators/stdalloc.c`: the standard allocator, over the C library's `malloc`,
//! `realloc` and `free`, which libgit2 runs on by default. A foundational module: its
//! routines are bound as strategies and stay C.
//!
//! Its three routines are `static`: C reaches them only through the `git_allocator` that
//! `git_stdalloc_init_allocator` fills, and so does Rust, through [`allocator`]. `malloc`
//! and `realloc` ignore the call site libgit2 passes them and forward to the C library's
//! routines of the same name, and `free` to `free`, so the family's storage is the C
//! library's heap. libgit2 installs it as its default allocator at initialisation, and
//! again when [`set_allocator`] is passed `None`, unless built with `DEBUG_STRICT_ALLOC`,
//! which installs `debugalloc` instead.
//!
//! The value is the starting point of an allocator that changes one member, as libgit2's
//! own out-of-memory tests build theirs (`tests/util/str/oom.c`): copy it, and replace
//! the member with a function that refuses some requests and forwards the rest to the
//! standard one.
//!
//! [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator

pub use ffi_import::allocator;

/// What the module reaches in C: `git_stdalloc_init_allocator`.
mod ffi_import {
    use crate::ffi;
    use crate::libgit2::src::util::alloc::Allocator;

    /// libgit2's standard allocator: its forwarders to the C library's `malloc`, `realloc`
    /// and `free`, as C fills a `git_allocator` with them.
    ///
    /// Every member is set, and two calls return the same three functions. Calling it
    /// touches no state of libgit2's, so it needs no initialisation and may run on any
    /// thread. The family meets [`Alloc`]'s contract as the C library's `malloc` does, so
    /// the value may be installed with [`set_allocator`] under that function's other
    /// obligations: before libgit2 holds any allocation, or over a family sharing the C
    /// library's heap, such as libgit2's default allocator in a build without
    /// `DEBUG_STRICT_ALLOC`.
    ///
    /// An allocator that refuses requests over 100 bytes, as libgit2's out-of-memory
    /// tests build one, starts from the standard one and replaces its `malloc`:
    ///
    /// ```
    /// use std::ffi::{c_char, c_int, c_void};
    /// use std::ptr;
    /// use std::sync::OnceLock;
    ///
    /// use libgit2::libgit2::src::util::alloc::Allocator;
    /// use libgit2::libgit2::src::util::allocators::stdalloc;
    ///
    /// static STANDARD: OnceLock<Allocator> = OnceLock::new();
    ///
    /// /// Refuses requests over 100 bytes, and forwards the rest to the standard `malloc`.
    /// unsafe extern "C" fn small_malloc(n: usize, file: *const c_char, line: c_int) -> *mut c_void {
    ///     if n > 100 {
    ///         return ptr::null_mut();
    ///     }
    ///     let malloc = STANDARD.get().and_then(|a| a.as_ref().malloc()).unwrap();
    ///     // SAFETY: forwarded under the caller's own `MallocFn` contract.
    ///     unsafe { malloc(n, file, line) }
    /// }
    ///
    /// let mut small = *STANDARD.get_or_init(stdalloc::allocator);
    /// small.as_mut().set_malloc(small_malloc);
    ///
    /// let (malloc, free) = (
    ///     small.as_ref().malloc().unwrap(),
    ///     small.as_ref().free().unwrap(),
    /// );
    /// // SAFETY: a NULL `file` is allowed; the block is released below, by its family.
    /// assert!(unsafe { malloc(101, ptr::null(), 0) }.is_null());
    /// // SAFETY: as above.
    /// let block = unsafe { malloc(100, ptr::null(), 0) };
    /// assert!(!block.is_null());
    /// // SAFETY: `block` is a live allocation of the standard family, released once.
    /// unsafe { free(block) };
    /// ```
    ///
    /// [`Alloc`]: crate::libgit2::src::util::alloc::Alloc
    /// [`set_allocator`]: crate::libgit2::src::libgit2::settings::set_allocator
    ///
    /// Wraps `fn::git_stdalloc_init_allocator`
    #[doc(alias = "git_stdalloc_init_allocator")]
    #[must_use]
    pub fn allocator() -> Allocator {
        let mut raw = ffi::git_allocator::default();
        // SAFETY: `raw` is a live, writable `git_allocator` on this frame for the whole
        // call; C writes its three members, reads nothing and keeps no pointer to it.
        let rc = unsafe { ffi::git_stdalloc_init_allocator(&raw mut raw) };
        debug_assert_eq!(rc, 0, "git_stdalloc_init_allocator returns 0 on every path");
        let ffi::git_allocator {
            gmalloc: Some(malloc),
            grealloc: Some(realloc),
            gfree: Some(free),
        } = raw
        else {
            unreachable!("git_stdalloc_init_allocator sets every member");
        };
        Allocator::from_fns(malloc, realloc, free)
    }

    #[cfg(test)]
    mod unit_tests {
        use core::ffi::{c_char, c_int, c_void};
        use core::{ptr, slice};

        use super::allocator;
        use crate::libgit2::src::util::alloc::{FreeFn, MallocFn};

        /// The value is plain data: replacing the members of a copy leaves the value it was
        /// copied from, and every later one, the standard functions.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_changed_copy_leaves_the_standard_value() {
            unsafe extern "C" fn refuse(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
                ptr::null_mut()
            }
            unsafe extern "C" fn keep(_: *mut c_void) {}
            let (refuse, keep): (MallocFn, FreeFn) = (refuse, keep);

            let standard = allocator();
            let mut changed = standard;
            let mut members = changed.as_mut();
            members.set_malloc(refuse);
            members.set_free(keep);

            let (now, before) = (changed.as_ref(), standard.as_ref());
            assert!(ptr::fn_addr_eq(now.malloc().unwrap(), refuse));
            assert!(ptr::fn_addr_eq(now.free().unwrap(), keep));
            assert!(ptr::fn_addr_eq(
                now.realloc().unwrap(),
                before.realloc().unwrap()
            ));
            assert!(!ptr::fn_addr_eq(
                before.malloc().unwrap(),
                now.malloc().unwrap()
            ));
            assert!(!ptr::fn_addr_eq(
                before.free().unwrap(),
                now.free().unwrap()
            ));
            assert_eq!(format!("{standard:?}"), format!("{:?}", allocator()));
            assert_ne!(format!("{changed:?}"), format!("{standard:?}"));
        }

        /// The members reach the C library's heap: a block of at least the size asked for,
        /// aligned as `malloc` aligns, resized with its contents kept, and released once;
        /// `realloc(NULL, ..)` allocates, and `free(NULL)` does nothing.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn members_reach_the_c_library_heap() {
            let standard = allocator();
            let members = standard.as_ref();
            let (malloc, realloc, free) = (
                members.malloc().unwrap(),
                members.realloc().unwrap(),
                members.free().unwrap(),
            );

            // SAFETY: `file` is a live C string for the call; the block is released below.
            let block = unsafe { malloc(5, c"stdalloc.c".as_ptr(), 7) };
            assert!(!block.is_null());
            assert_eq!(block as usize % 16, 0, "malloc aligns to max_align_t");
            // SAFETY: `block` holds 5 writable bytes.
            unsafe {
                block
                    .cast::<u8>()
                    .copy_from_nonoverlapping(b"hello".as_ptr(), 5);
            };
            // SAFETY: `block` is a live block of this family, given up on success; NULL `file`.
            let grown = unsafe { realloc(block, 4096, ptr::null(), -1) };
            assert!(!grown.is_null());
            // SAFETY: `grown` holds 4096 bytes, the first 5 carried over from `block`.
            let kept = unsafe { slice::from_raw_parts(grown.cast::<u8>(), 5) };
            assert_eq!(kept, b"hello");
            // SAFETY: `grown` holds 4096 writable bytes.
            unsafe { grown.cast::<u8>().write_bytes(0xA5, 4096) };
            // SAFETY: as above, shrinking `grown`, whose first byte survives.
            let shrunk = unsafe { realloc(grown, 1, c"x".as_ptr(), 1) };
            assert!(!shrunk.is_null());
            // SAFETY: `shrunk` holds one byte, carried over.
            assert_eq!(unsafe { shrunk.cast::<u8>().read() }, 0xA5);

            // SAFETY: a NULL `ptr` allocates afresh; the block is released below.
            let fresh = unsafe { realloc(ptr::null_mut(), 3, ptr::null(), 0) };
            assert!(!fresh.is_null() && fresh != shrunk);
            // SAFETY: a zero-byte request is glibc's unique, freeable block; released below.
            let empty = unsafe { malloc(0, ptr::null(), 0) };
            assert!(!empty.is_null() && empty != fresh && empty != shrunk);

            // SAFETY: NULL is a no-op.
            unsafe { free(ptr::null_mut()) };
            // SAFETY: `shrunk`, `fresh` and `empty` are live blocks of this family, each given up
            // once.
            unsafe { free(shrunk) };
            // SAFETY: as above.
            unsafe { free(fresh) };
            // SAFETY: as above.
            unsafe { free(empty) };
        }

        /// The family's storage is the C library's heap, so a block of either is released
        /// by the other's `free`.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn storage_is_the_c_library_heap() {
            unsafe extern "C" {
                fn malloc(n: usize) -> *mut c_void;
                fn free(p: *mut c_void);
            }
            let standard = allocator();
            let r = standard.as_ref();
            let (std_malloc, std_free) = (r.malloc().unwrap(), r.free().unwrap());
            // SAFETY: NULL `file`; the block is the C library's, released below by `free`.
            let ours = unsafe { std_malloc(24, ptr::null(), 0) };
            assert!(!ours.is_null());
            // SAFETY: a C library allocation, released below by the family's `free`.
            let theirs = unsafe { malloc(24) };
            assert!(!theirs.is_null());
            // SAFETY: each block is live and released once, by the other family's `free`,
            // which is the same heap's.
            unsafe { free(ours) };
            // SAFETY: as above.
            unsafe { std_free(theirs) };
        }
    }
}
