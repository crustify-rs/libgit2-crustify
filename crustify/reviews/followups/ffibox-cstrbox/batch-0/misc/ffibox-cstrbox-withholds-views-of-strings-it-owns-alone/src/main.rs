//! What reading an owned C string costs through ffibox's safe API, against an owned C
//! byte buffer under the same ownership.
//!
//! Both values below are sole owners of libgit2-allocator storage that no C code
//! points at: an `OdbDataStr` (`CStrBox`, whose `from_raw` requires a uniquely-owned
//! string) and an `OdbDataVec<u8>` (`CVec`). `CVec` lends its bytes as a `&[u8]`
//! because it owns them alone; `CStrBox` lends nothing, on the premise that "C keeps
//! its pointer to this string". Decoding the string, or making a path of it, then
//! takes a heap copy. The counting allocator sees only Rust's allocations: libgit2's
//! own go through its allocator, outside it. The only unsafe code is that allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataVec, data_str_bytes};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to `System`, counting.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn counted<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCS.load(Ordering::Relaxed);
    let r = f();
    (r, ALLOCS.load(Ordering::Relaxed) - before)
}

fn main() {
    let name = c"refs/heads/main";
    let s = OdbDataStrFree::dup(name).unwrap();
    let v = OdbDataVec::<u8>::from_slice(name.to_bytes()).unwrap();

    let (vec_ok, vec_allocs) = counted(|| {
        let text = std::str::from_utf8(v.as_slice()).is_ok();
        let dir = Path::new(OsStr::from_bytes(v.as_slice())).parent().is_some();
        text && dir
    });
    // The route `CStrBox::as_handles`'s docs prescribe: copy the bytes into a buffer
    // of one's own, and decode there.
    let (str_ok, str_allocs) = counted(|| {
        let bytes = data_str_bytes(&s);
        let text = std::str::from_utf8(&bytes).is_ok();
        let dir = Path::new(OsStr::from_bytes(&bytes)).parent().is_some();
        text && dir
    });
    assert!(vec_ok && str_ok);
    println!("decode + path, CVec<u8> (borrowed &[u8]):      {vec_allocs} Rust allocations");
    println!("decode + path, CStrBox (copied out of a CSlice): {str_allocs} Rust allocations");
    assert_eq!(
        str_allocs, vec_allocs,
        "a string the CStrBox owns alone is copied to be read, where a CVec under the \
         same ownership lends its bytes"
    );
}
