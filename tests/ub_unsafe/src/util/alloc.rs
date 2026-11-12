//! `git2/sys/alloc.h`: the `unsafe extern "C"` members an `Allocator` built by
//! `Allocator::new` holds, called within the contracts `MallocFn`, `ReallocFn` and
//! `FreeFn` state, edge cases included. One suite per member.
//!
//! The implementation behind them is libc's allocator, which Miri models, so these run
//! there too; ASan/LSan flag a trampoline that loses, leaks or double-frees a block.
//!
//! `src/util/alloc.c`'s `global_init`, which writes libgit2's allocator global, gets a suite
//! of its own too, and `set_allocator` (`git_allocator_setup`) one seen through `installed`,
//! the global's safe borrow. Each test runs alone in a child process, where it can keep the
//! contract that no other thread reads or writes the global while it is written, and where
//! libgit2 starts uninitialised, with the failing family installed.

use core::ffi::{CStr, c_void};
use core::ptr::{self, NonNull};
use core::slice;

use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{
    Alloc, Allocator, AllocatorRef, FreeFn, MallocFn, ReallocFn, StrFree, global_init, installed,
};
use libgit2::libgit2::src::util::allocators::failalloc::FailAlloc;

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator behind `Alloc`, refusing [`REFUSED`] bytes: a request libc cannot
/// meet would abort under `ASan` (`allocator_may_return_null=0`) instead of failing.
struct Libc;

const REFUSED: usize = 1 << 40;

// SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract.
unsafe impl Alloc for Libc {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if len == REFUSED {
            return None;
        }
        // SAFETY: plain libc allocation.
        NonNull::new(unsafe { malloc(len) }.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        if len == REFUSED {
            return None;
        }
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

static LIBC: Allocator = Allocator::new::<Libc>();

fn members() -> (MallocFn, ReallocFn, FreeFn) {
    let r = LIBC.as_ref();
    (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap())
}

/// Fills `len` bytes at `p` with a pattern and checks them back.
///
/// # Safety
///
/// `p` is valid for writes of `len` bytes.
unsafe fn scribble(p: *mut c_void, len: usize, seed: u8) {
    let p = p.cast::<u8>();
    for i in 0..len {
        // SAFETY: `i < len`.
        let add = unsafe { p.add(i) };
        // SAFETY: as above.
        unsafe { add.write(seed.wrapping_add(i.to_le_bytes()[0])) };
    }
    // SAFETY: the `len` bytes were just written.
    let bytes = unsafe { slice::from_raw_parts(p, len) };
    assert!(
        bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| b == seed.wrapping_add(i.to_le_bytes()[0]))
    );
}

#[test]
fn malloc_member_returns_writable_aligned_blocks() {
    let (malloc, _, free) = members();
    for (len, file) in [
        (1, ptr::null()),
        (24, c"alloc.c".as_ptr()),
        (4096, c"".as_ptr()),
    ] {
        // SAFETY: `file` is NULL or a live C string; the block is released below.
        let p = unsafe { malloc(len, file, 42) };
        assert!(!p.is_null());
        let align = (1usize << len.ilog2()).min(16);
        assert_eq!(p as usize % align, 0, "aligned as malloc aligns");
        // SAFETY: `p` holds `len` writable bytes.
        unsafe { scribble(p, len, len.to_le_bytes()[0]) };
        // SAFETY: `p` is a live block of this family, given up.
        unsafe { free(p) };
    }
}

#[test]
fn malloc_member_accepts_zero_bytes() {
    let (malloc, _, free) = members();
    // SAFETY: NULL `file`; a non-NULL result is a block released below.
    let p = unsafe { malloc(0, ptr::null(), 0) };
    // SAFETY: NULL or a live block of this family, given up.
    unsafe { free(p) };
}

/// Zero-byte requests, through either allocating member, are allocations of their own:
/// resized and released like any other. A live block resized to zero bytes is the next
/// test's.
#[test]
fn zero_byte_requests_are_allocations() {
    let (malloc, realloc, free) = members();
    // SAFETY: NULL `file`; each non-NULL result is a block released below.
    let malloc2 = unsafe { malloc(0, ptr::null(), 0) };
    // SAFETY: as above.
    let (a, b) = unsafe { (malloc2, realloc(ptr::null_mut(), 0, c"".as_ptr(), 0)) };
    // SAFETY: `a` is NULL or a live block of this family, given up on success.
    let grown = unsafe { realloc(a, 8, ptr::null(), 0) };
    assert!(!grown.is_null());
    // SAFETY: `grown` holds 8 writable bytes.
    unsafe { scribble(grown, 8, 3) };
    // SAFETY: `grown` is a live block; `b` is NULL or one; each given up once.
    unsafe { free(grown) };
    // SAFETY: as above.
    unsafe { free(b) };
}

/// A zero-byte resize of a live block, which C leaves implementation-defined, is refused
/// before it reaches the implementation: NULL, the block left live. Were it passed on,
/// glibc would release the block and return NULL, which reads as a failure that left it
/// live: the release below would then be a double free, which `ASan` reports.
#[test]
fn realloc_member_refuses_a_zero_byte_resize_of_a_live_block() {
    let (malloc, realloc, free) = members();
    // SAFETY: NULL `file`; the block is released below.
    let p = unsafe { malloc(16, ptr::null(), 0) };
    assert!(!p.is_null());
    // SAFETY: `p` holds 16 writable bytes.
    unsafe { scribble(p, 16, 9) };
    // SAFETY: `p` is a live block of this family; the members `Allocator::new` builds accept a
    // zero-byte resize of one, and refuse it.
    assert!(unsafe { realloc(p, 0, c"alloc.c".as_ptr(), 4) }.is_null());
    // SAFETY: `p` is still live and unchanged.
    unsafe { scribble(p, 16, 9) };
    // SAFETY: `p` is a live block of this family, given up.
    unsafe { free(p) };
}

#[test]
fn malloc_member_reports_failure_as_null() {
    let (malloc, _, _) = members();
    // SAFETY: NULL `file`; the implementation refuses the request.
    assert!(unsafe { malloc(REFUSED, ptr::null(), 0) }.is_null());
}

#[test]
fn realloc_member_resizes_and_keeps_the_prefix() {
    let (_, realloc, free) = members();
    // SAFETY: NULL allocates afresh, as `malloc`; `file` is a live C string.
    let p = unsafe { realloc(ptr::null_mut(), 8, c"str.c".as_ptr(), 1) };
    assert!(!p.is_null());
    // SAFETY: `p` holds 8 writable bytes.
    unsafe { scribble(p, 8, 7) };
    // SAFETY: `p` is a live block of this family, given up on success.
    let grown = unsafe { realloc(p, 1 << 16, ptr::null(), 2) };
    assert!(!grown.is_null());
    // SAFETY: the first 8 bytes were carried over.
    let kept = unsafe { slice::from_raw_parts(grown.cast::<u8>(), 8) };
    assert_eq!(kept, [7, 8, 9, 10, 11, 12, 13, 14]);
    // SAFETY: as above, shrinking.
    let shrunk = unsafe { realloc(grown, 3, ptr::null(), 3) };
    assert!(!shrunk.is_null());
    // SAFETY: the first 3 bytes were carried over.
    let kept = unsafe { slice::from_raw_parts(shrunk.cast::<u8>(), 3) };
    assert_eq!(kept, [7, 8, 9]);
    // SAFETY: `shrunk` is a live block of this family, given up.
    unsafe { free(shrunk) };
}

#[test]
fn realloc_member_failure_leaves_the_block_live() {
    let (_, realloc, free) = members();
    // SAFETY: NULL allocates afresh.
    let p = unsafe { realloc(ptr::null_mut(), 16, ptr::null(), 0) };
    assert!(!p.is_null());
    // SAFETY: `p` holds 16 writable bytes.
    unsafe { scribble(p, 16, 1) };
    // SAFETY: `p` is a live block; the implementation refuses the request.
    assert!(unsafe { realloc(p, REFUSED, ptr::null(), 0) }.is_null());
    // SAFETY: `p` is still live and unchanged.
    unsafe { scribble(p, 16, 1) };
    // SAFETY: `p` is a live block of this family, given up.
    unsafe { free(p) };
}

#[test]
fn free_member_ignores_null() {
    let (_, _, free) = members();
    // SAFETY: NULL is a no-op.
    unsafe { free(ptr::null_mut()) };
}

#[test]
fn free_member_releases_blocks_from_either_allocating_member() {
    let (malloc, realloc, free) = members();
    // SAFETY: NULL `file`; both blocks are released below.
    let malloc2 = unsafe { malloc(32, ptr::null(), 0) };
    // SAFETY: as above.
    let (a, b) = unsafe { (malloc2, realloc(ptr::null_mut(), 32, ptr::null(), 0)) };
    assert!(!a.is_null() && !b.is_null());
    // SAFETY: two distinct live blocks of this family, each given up once.
    unsafe { free(a) };
    // SAFETY: as above.
    unsafe { free(b) };
}

// --- set_allocator, global_init ---------------------------------------------------------

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2 and no
/// other thread runs.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
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

/// The family's three members, as their addresses: what libgit2 compares them by.
fn addrs(a: AllocatorRef<'_>) -> [usize; 3] {
    [
        a.malloc().unwrap() as usize,
        a.realloc().unwrap() as usize,
        a.free().unwrap() as usize,
    ]
}

/// The failing family's members, as libgit2's global starts with them.
fn failing() -> [usize; 3] {
    addrs(FAIL.as_ref())
}

static FAIL: Allocator = Allocator::new::<FailAlloc>();

/// The global's members as C reads them, without pinning libgit2's runtime as `installed`
/// does: what the global holds before initialisation.
fn unpinned() -> [usize; 3] {
    // SAFETY: a read of the C global, live for the whole process, which no other thread
    // writes in the child processes these tests run in.
    let g = unsafe { (&raw const libgit2::ffi::git__allocator).read() };
    [
        g.gmalloc.map_or(0, |f| f as usize),
        g.grealloc.map_or(0, |f| f as usize),
        g.gfree.map_or(0, |f| f as usize),
    ]
}

/// `git_allocator_setup`, which `set_allocator` stands in for, seen through the global's
/// borrow, `installed`, which reads the live global.
mod set_allocator {
    use super::*;

    /// The handle reads the live global, so it sees each install this thread makes between
    /// its reads.
    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn the_borrow_follows_every_install_on_the_reading_thread() {
        isolated(
            concat!(
                module_path!(),
                "::the_borrow_follows_every_install_on_the_reading_thread"
            ),
            || {
                // Pinning initialises libgit2, whose allocations are the standard family's.
                let global = installed().unwrap();
                let standard = addrs(global);
                assert_ne!(standard, failing());
                // SAFETY: `LIBC` meets `Alloc`'s contract and shares libc's storage with the
                // standard family, which made every allocation libgit2 holds; no other thread
                // runs.
                unsafe { set_allocator(Some(LIBC.as_ref())) };
                assert_eq!(addrs(global), addrs(LIBC.as_ref()));
                assert!(StrFree::dup(c"libc").is_some());
                // SAFETY: the standard family is libc's too, so it releases what `LIBC`
                // allocated; no other thread runs.
                unsafe { set_allocator(None) };
                assert_eq!(addrs(global), standard);
                assert!(StrFree::dup(c"standard").is_some());
            },
        );
    }

    /// An allocator installed before initialisation is the one the first borrow reads:
    /// initialisation keeps it, and libgit2 allocates through it.
    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn an_allocator_installed_first_is_the_one_the_borrow_reads() {
        isolated(
            concat!(
                module_path!(),
                "::an_allocator_installed_first_is_the_one_the_borrow_reads"
            ),
            || {
                assert_eq!(unpinned(), failing());
                // SAFETY: `LIBC` meets `Alloc`'s contract and libgit2 holds no allocation yet;
                // no other thread runs.
                unsafe { set_allocator(Some(LIBC.as_ref())) };
                let global = installed().unwrap();
                assert_eq!(addrs(global), addrs(LIBC.as_ref()));
                assert!(StrFree::dup(c"libc").is_some());
            },
        );
    }
}

mod global_init {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn installs_the_standard_family_over_the_failing_one() {
        isolated(
            concat!(
                module_path!(),
                "::installs_the_standard_family_over_the_failing_one"
            ),
            || {
                assert_eq!(unpinned(), failing());
                // SAFETY: libgit2 holds no allocation before initialisation; no other thread
                // runs.
                unsafe { global_init() };
                let standard = unpinned();
                assert_ne!(standard, failing());
                // Initialisation and a second call keep it.
                let global = installed().unwrap();
                assert_eq!(addrs(global), standard);
                // SAFETY: as above; the standard family is installed, so nothing is written.
                unsafe { global_init() };
                assert_eq!(addrs(global), standard);
                assert!(StrFree::dup(c"standard").is_some());
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn keeps_any_other_family() {
        isolated(concat!(module_path!(), "::keeps_any_other_family"), || {
            // SAFETY: `LIBC` meets `Alloc`'s contract; installed before libgit2 holds any
            // allocation, with no other thread.
            unsafe { set_allocator(Some(LIBC.as_ref())) };
            // SAFETY: as above; `LIBC` is installed, so nothing is written.
            unsafe { global_init() };
            assert_eq!(unpinned(), addrs(LIBC.as_ref()));
            assert_eq!(addrs(installed().unwrap()), addrs(LIBC.as_ref()));
        });
    }

    /// The failing family installed over a running libgit2 gives way to the standard one
    /// again: the allocations libgit2 holds are the standard family's, which it releases.
    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn replaces_the_failing_family_installed_after_init() {
        isolated(
            concat!(
                module_path!(),
                "::replaces_the_failing_family_installed_after_init"
            ),
            || {
                let global = installed().unwrap();
                let standard = addrs(global);
                // SAFETY: no other thread runs; the failing family accepts and keeps every
                // allocation libgit2 holds, and nothing is allocated or released before the
                // standard family is back.
                unsafe { set_allocator(Some(FAIL.as_ref())) };
                assert_eq!(addrs(global), failing());
                // SAFETY: no other thread runs; every allocation libgit2 holds is the standard
                // family's, made by initialisation.
                unsafe { global_init() };
                assert_eq!(addrs(global), standard);
                assert!(StrFree::dup(c"back").is_some());
            },
        );
    }
}
