//! Equivalence of `src/util/allocators/failalloc.c` with the Rust failing family.
//!
//! libgit2.so does not export the three C routines (hidden visibility), so they are
//! reached through what calls them: `git__allocator`, whose initial value they are, until
//! `git_libgit2_init` replaces it. Before initialisation, `git_odb_backend_data_free` is
//! `git__free`, which is `git_failalloc_free`; it sets no error, so it is safe to call
//! then. Its siblings are not: `git__malloc` and `git__realloc` report their failure
//! through `git_error_set_oom`, whose thread-state lookup reads a TLS key not created
//! before initialisation and may write through another component's slot (the hazard
//! `git_odb_backend_data_alloc`'s finding records), so C's `malloc` and `realloc` are
//! compared through nothing here; the Rust suites cover the Rust side's.
//!
//! Every step runs on independently owned buffers, filled alike, and compares what each
//! release left of its buffer: the C routine and every Rust release of the family leave it
//! untouched and live. Handing them a buffer they did not allocate probes the bodies, as
//! the C's `GIT_UNUSED` shows them, beyond the family's contract, which only an
//! allocation of the family (there are none) or NULL meets; NULL is compared too.
//!
//! libgit2's allocator is process-global, so the test reruns alone in a child process
//! that never initialises libgit2.

use std::ffi::c_void;
use std::process::Command;
use std::ptr::{self, NonNull};

use ffibox::{CDrop, CLenDrop};
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
use libgit2::libgit2::src::util::allocators::failalloc::{FailAlloc, FailArrayAlloc};

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals and
/// nothing initialises libgit2 before it.
fn isolated(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
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

/// The `git_allocator` over the Rust family.
static FAIL: Allocator = Allocator::new::<FailAlloc>();

/// A buffer of `len` bytes, its bytes a function of `seed`, which a release must leave
/// as it found it.
fn buffer(len: usize, seed: u8) -> Box<[u8]> {
    (0..len)
        .map(|i| seed.wrapping_add(i.to_le_bytes()[0]))
        .collect()
}

/// What a release left of a buffer: its bytes, read back once it returned.
type Left = Vec<u8>;

/// The release C's initial allocator makes: `git_odb_backend_data_free`, which is
/// `git__free`, which is `git_failalloc_free` before initialisation, or whichever
/// family `set_allocator` installed.
fn c_free(buf: Option<&mut [u8]>) -> Option<Left> {
    let ptr = buf
        .as_ref()
        .map_or(ptr::null_mut(), |b| b.as_ptr().cast_mut());
    // SAFETY: the backend is never read (`GIT_UNUSED`); `ptr` is NULL or a live buffer
    // this test owns, which the installed family's free ignores (see the module docs).
    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), ptr.cast::<c_void>()) };
    buf.map(|b| b.to_vec())
}

/// Each Rust release of the family, on one buffer each.
fn rust_frees(len: usize, seed: u8) -> [Left; 4] {
    let mut a = buffer(len, seed);
    let mut b = buffer(len, seed);
    let mut c = buffer(len, seed);
    let mut d = buffer(len, seed);
    let pa = NonNull::new(a.as_mut_ptr()).unwrap();
    // SAFETY: a live buffer the test owns, handed for the call; the family keeps nothing.
    unsafe { FailAlloc.c_drop(pa) };
    // SAFETY: as above; `len` bytes of it are in use.
    unsafe { FailArrayAlloc.c_drop_len(b.as_mut_ptr(), len) };
    let pc = NonNull::new(c.as_mut_ptr()).unwrap();
    // SAFETY: as above.
    unsafe { <FailAlloc as Alloc>::free(pc) };
    let free = FAIL.as_ref().free().unwrap();
    // SAFETY: as above, through the member's trampoline.
    unsafe { free(d.as_mut_ptr().cast::<c_void>()) };
    [a.to_vec(), b.to_vec(), c.to_vec(), d.to_vec()]
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process, and calls C")]
fn free_releases_nothing_as_c_does() {
    isolated(
        concat!(module_path!(), "::free_releases_nothing_as_c_does"),
        run,
    );
}

fn run() {
    // Step 1: libgit2 is not initialised, so its allocator is C's failing one.
    for (step, len) in [1usize, 16, 4096].into_iter().enumerate() {
        let seed = u8::try_from(step).unwrap();
        let mut theirs = buffer(len, seed);
        let c = c_free(Some(&mut theirs[..])).unwrap();
        assert_eq!(
            c,
            buffer(len, seed).to_vec(),
            "C released into buffer {step}"
        );
        for (i, rust) in rust_frees(len, seed).into_iter().enumerate() {
            assert_eq!(rust, c, "Rust release {i} differs from C at step {step}");
        }
        assert_eq!(c_free(None), None);
    }

    // Step 2: the Rust family installed in C's place, still before initialisation, so
    // libgit2 holds no allocation the family would have to release: libgit2's release
    // now runs the Rust body, and leaves the buffer as C's did.
    // SAFETY: an `Allocator::new` value; libgit2 is not initialised and holds nothing;
    // this child runs one test on one thread.
    unsafe { set_allocator(Some(FAIL.as_ref())) };
    for (step, len) in [1usize, 16, 4096].into_iter().enumerate() {
        let seed = u8::try_from(step).unwrap().wrapping_mul(7);
        let mut theirs = buffer(len, seed);
        let installed = c_free(Some(&mut theirs[..])).unwrap();
        assert_eq!(
            installed,
            buffer(len, seed).to_vec(),
            "installed release, step {step}"
        );
        assert_eq!(c_free(None), None);
    }

    // Step 3: the system allocator back, as `git_allocator_setup(NULL)` restores it.
    // SAFETY: as above; nothing was allocated under the failing family.
    unsafe { set_allocator(None) };
}
