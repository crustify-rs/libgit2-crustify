//! `deps/reftable/basics.c`: `set_alloc` and `set_default_alloc`, which replace reftable's
//! allocator hooks, called within their `# Safety` contract: in a child process of their
//! own, with libgit2's runtime pinned first and kept, nothing else allocating through
//! reftable, and no allocation crossing from one family to another. One suite per
//! function.
//!
//! Each family here is libc's allocator, counted, so that `ASan` flags a hook that frees
//! through the wrong routine, `LSan` one that loses a block, and the counts show that every
//! owner allocated and released through the hooks installed. The hooks are process-wide,
//! so each test reruns alone in a child process.

use core::ffi::c_void;
use core::num::NonZeroUsize;
use core::ptr::{self, NonNull};
use std::process::Command;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;

use libgit2::libgit2::deps::reftable::basics::{
    Alloc, Buf, Error, ReftableArrayFree, ReftableBox, ReftableStr, ReftableVec, parse_names,
    set_alloc, set_default_alloc,
};
use libgit2::libgit2::src::libgit2::libgit2::{Runtime, init};

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
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

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static RESIZED: AtomicUsize = AtomicUsize::new(0);
static RELEASED: AtomicUsize = AtomicUsize::new(0);

/// A request no family grants, which libc would answer by aborting under `ASan`.
const REFUSED: usize = 1 << 40;

/// libc's allocator, counted.
struct Counted;

// SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract,
// refusing `REFUSED` bytes as a failure.
unsafe impl Alloc for Counted {
    fn malloc(len: NonZeroUsize) -> Option<NonNull<u8>> {
        if len.get() == REFUSED {
            return None;
        }
        ALLOCATED.fetch_add(1, SeqCst);
        // SAFETY: a plain libc allocation.
        NonNull::new(unsafe { malloc(len.get()) }.cast())
    }

    unsafe fn realloc(p: Option<NonNull<u8>>, len: NonZeroUsize) -> Option<NonNull<u8>> {
        if len.get() == REFUSED {
            return None;
        }
        RESIZED.fetch_add(1, SeqCst);
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len.get()) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        RELEASED.fetch_add(1, SeqCst);
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

/// Pins libgit2's runtime, whose first initialisation installs libgit2's hooks; kept for
/// the rest of the child.
fn runtime() -> Runtime {
    init().expect("libgit2 initialises")
}

/// Allocates through every owner and routine, then releases it all.
fn exercise() {
    let mut value = Buf::new();
    value.as_mut().add(b"refs/heads/main").unwrap();
    value.as_mut().add(&[b'x'; 300]).unwrap();
    value.as_mut().add_str(c"!").unwrap();
    let detached = value.as_mut().detach().unwrap();
    assert_eq!(detached.to_bytes().len(), 316);
    value.as_mut().add_str(c"again").unwrap();
    let names = parse_names(b"a\nb\nc\n").unwrap();
    assert_eq!(names.len(), 3);
    let s = ReftableStr::from_c_str(c"name").unwrap();
    assert_eq!(s.clone(), s);
    let mut run = ReftableVec::<u32>::new_uninit(0).unwrap();
    ReftableArrayFree::grow(&mut run, 5).unwrap();
    ReftableArrayFree::resize(&mut run, 2).unwrap();
    let zeroed = ReftableArrayFree::new_zeroed::<u16>(3).unwrap();
    assert_eq!(&*zeroed, [0; 3]);
    let boxed = ReftableBox::<Buf>::new_zeroed().unwrap();
    assert!(boxed.as_ref().is_empty());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_alloc_routes_every_owner_through_the_hooks() {
    isolated(
        concat!(
            module_path!(),
            "::set_alloc_routes_every_owner_through_the_hooks"
        ),
        || {
            let _runtime = runtime();
            // SAFETY: the child runs alone and nothing has allocated through reftable;
            // libgit2's runtime stays initialised while `Counted` is installed.
            unsafe { set_alloc::<Counted>() };
            exercise();
            let (allocated, resized, released) = (
                ALLOCATED.load(SeqCst),
                RESIZED.load(SeqCst),
                RELEASED.load(SeqCst),
            );
            assert!(allocated >= 7, "{allocated}");
            assert!(resized >= 4, "{resized}");
            // Every block one hook made, another released: `realloc` replaces its own.
            assert!(released >= allocated, "{released} < {allocated}");

            // A request the family refuses fails cleanly, through each path.
            let mut value = Buf::new();
            let mut run = ReftableVec::<u8>::new_uninit(0).unwrap();
            assert_eq!(
                ReftableArrayFree::resize(&mut run, REFUSED),
                Err(Error::OutOfMemory)
            );
            assert!(ReftableVec::<u8>::new_uninit(REFUSED).is_none());
            value.as_mut().add(b"kept").unwrap();
            assert_eq!(value.as_ref().as_bytes(), b"kept");
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_default_alloc_hands_reftable_back_to_libc() {
    isolated(
        concat!(
            module_path!(),
            "::set_default_alloc_hands_reftable_back_to_libc"
        ),
        || {
            let _runtime = runtime();
            // SAFETY: as above; `Counted` is libc's family, so what it allocated libc
            // releases, and the reverse.
            unsafe { set_alloc::<Counted>() };
            let mut kept = Buf::new();
            kept.as_mut().add(b"made by Counted").unwrap();
            let before = ALLOCATED.load(SeqCst) + RESIZED.load(SeqCst);
            // SAFETY: the child runs alone; `kept` was made by libc's family, which the
            // new hooks (libc's own) release.
            unsafe { set_default_alloc() };
            exercise();
            assert_eq!(
                ALLOCATED.load(SeqCst) + RESIZED.load(SeqCst),
                before,
                "libc's own routines, not the hooks"
            );
            let released = RELEASED.load(SeqCst);
            drop(kept);
            assert_eq!(RELEASED.load(SeqCst), released, "released by libc directly");
            // And back again.
            // SAFETY: as above.
            unsafe { set_alloc::<Counted>() };
            exercise();
            assert!(ALLOCATED.load(SeqCst) + RESIZED.load(SeqCst) > before);
        },
    );
}
