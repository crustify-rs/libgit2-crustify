//! Equivalence of `src/util/allocators/failalloc.c` with the Rust failing family.
//!
//! The strategies call the three C routines through their raw bindings, so the comparisons
//! check the mapping they apply: NULL as `None`, a pointer as `Option<NonNull<u8>>`, a
//! release that leaves its buffer alone. They run on independently owned buffers, filled
//! alike, and compare what each call left of its buffer: a buffer the family did not
//! allocate, which `FailAlloc` documents its release and resize accept and leave alone,
//! as the C's `GIT_UNUSED` shows them, since the family makes none of its own. NULL is
//! compared too.
//!
//! The routines called directly set no error and read no state of libgit2, so they are
//! compared in the test process. Through `git__allocator`, whose initial value they are,
//! only a release is safe before `git_libgit2_init`: `git_odb_backend_data_free` is
//! `git__free`, which sets no error, where `git__malloc` and `git__realloc` report their
//! failure through `git_error_set_oom`, whose thread-state lookup reads a TLS key not yet
//! created (the hazard `git_odb_backend_data_alloc`'s finding records). Those steps, and
//! the initialisation that must replace the Rust family's `git_allocator` as it replaces
//! C's initial value, since both hold C's own routines, run alone in child processes that
//! start with libgit2 uninitialised, as its allocator is process-global.

use std::ffi::{CStr, c_char, c_void};
use std::process::Command;
use std::ptr::{self, NonNull};

use ffibox::{CAlloc, CDrop, CLenAlloc, CLenDrop};
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

/// The sizes every allocation is asked for: empty, small, page-sized and unmeetable.
const SIZES: [usize; 5] = [0, 1, 24, 4096, usize::MAX];

/// The call sites a request may name: none, and one as libgit2 names them.
const SITES: [(Option<&CStr>, i32); 2] = [(None, 0), (Some(c"src/util/alloc.h"), 21)];

/// A layout type for the singleton policy to allocate.
#[repr(C)]
#[expect(
    unreachable_pub,
    reason = "`define_ctype!` makes its newtype `pub`, which names this layout"
)]
pub struct probe_st {
    pub value: u64,
}
ffibox::define_ctype!(Probe, ProbeRef, ProbeMut, probe_st);

fn site(file: Option<&CStr>) -> *const c_char {
    file.map_or(ptr::null(), CStr::as_ptr)
}

/// A buffer of `len` bytes, its bytes a function of `seed`, which a call must leave as it
/// found it.
fn buffer(len: usize, seed: u8) -> Box<[u8]> {
    (0..len)
        .map(|i| seed.wrapping_add(i.to_le_bytes()[0]))
        .collect()
}

/// What a call left of a buffer: its bytes, read back once it returned.
type Left = Vec<u8>;

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn allocations_fail_as_c_does() {
    let malloc = FAIL.as_ref().malloc().unwrap();
    let realloc = FAIL.as_ref().realloc().unwrap();
    for len in SIZES {
        for (file, line) in SITES {
            // SAFETY: `file` is NULL or a C string live for the call.
            let c = NonNull::new(unsafe { ffi::git_failalloc_malloc(len, site(file), line) });
            assert_eq!(c, None, "C malloc({len})");
            assert_eq!(FailAlloc::malloc(len, file, line), c.map(NonNull::cast));
            // SAFETY: as above; the member is C's routine itself.
            assert_eq!(NonNull::new(unsafe { malloc(len, site(file), line) }), c);
            if len != 0 {
                assert_eq!(FailArrayAlloc.c_alloc_len(len), c.map(NonNull::cast));
            }

            // SAFETY: a NULL `ptr` asks for a fresh allocation, of any size.
            let c = NonNull::new(unsafe {
                ffi::git_failalloc_realloc(ptr::null_mut(), len, site(file), line)
            });
            assert_eq!(c, None, "C realloc(NULL, {len})");
            // SAFETY: `None` asks for a fresh allocation, of any size.
            let rust = unsafe { FailAlloc::realloc(None, len, file, line) };
            assert_eq!(rust, c.map(NonNull::cast));
            // SAFETY: as above, through the member.
            let member = unsafe { realloc(ptr::null_mut(), len, site(file), line) };
            assert_eq!(NonNull::new(member), c);
        }
    }
    assert!(<FailAlloc as CAlloc<Probe>>::c_alloc(&FailAlloc).is_none());
}

/// C's resize of a live buffer, and each Rust one, on one buffer each: every one fails and
/// leaves its buffer as it found it.
fn resizes(len: usize, seed: u8) -> [(bool, Left); 3] {
    let mut c = buffer(len, seed);
    let mut rust = buffer(len, seed);
    let mut member = buffer(len, seed);
    let realloc = FAIL.as_ref().realloc().unwrap();
    // SAFETY: a live buffer this test owns, borrowed for the call: the routine reads
    // nothing and fails, so it keeps nothing and the buffer stays the test's.
    let from_c =
        unsafe { ffi::git_failalloc_realloc(c.as_mut_ptr().cast(), 2 * len, ptr::null(), 0) };
    // SAFETY: as above, through the strategy.
    let from_rust =
        unsafe { FailAlloc::realloc(NonNull::new(rust.as_mut_ptr()), 2 * len, None, 0) };
    // SAFETY: as above, through the member.
    let from_member = unsafe { realloc(member.as_mut_ptr().cast(), 2 * len, ptr::null(), 0) };
    [
        (from_c.is_null(), c.to_vec()),
        (from_rust.is_none(), rust.to_vec()),
        (from_member.is_null(), member.to_vec()),
    ]
}

/// C's release, and each Rust release of the family, on one buffer each.
fn releases(len: usize, seed: u8) -> [Left; 5] {
    let mut by_c = buffer(len, seed);
    let mut by_drop = buffer(len, seed);
    let mut by_drop_len = buffer(len, seed);
    let mut by_free = buffer(len, seed);
    let mut by_member = buffer(len, seed);
    // SAFETY: a live buffer the test owns, handed for the call; the family keeps nothing.
    unsafe { ffi::git_failalloc_free(by_c.as_mut_ptr().cast()) };
    // SAFETY: as above.
    unsafe { FailAlloc.c_drop(NonNull::new(by_drop.as_mut_ptr()).unwrap()) };
    // SAFETY: as above; `len` bytes of it are in use.
    unsafe { FailArrayAlloc.c_drop_len(by_drop_len.as_mut_ptr(), len) };
    // SAFETY: as above.
    unsafe { <FailAlloc as Alloc>::free(NonNull::new(by_free.as_mut_ptr()).unwrap()) };
    let free = FAIL.as_ref().free().unwrap();
    // SAFETY: as above, through the member.
    unsafe { free(by_member.as_mut_ptr().cast::<c_void>()) };
    [by_c, by_drop, by_drop_len, by_free, by_member].map(|b| b.to_vec())
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn resizes_and_releases_leave_buffers_as_c_does() {
    for (step, len) in [1usize, 16, 4096].into_iter().enumerate() {
        let seed = u8::try_from(step).unwrap();
        let expected = buffer(len, seed).to_vec();
        let [c, rest @ ..] = resizes(len, seed);
        assert_eq!(c, (true, expected.clone()), "C resize, step {step}");
        for (i, rust) in rest.into_iter().enumerate() {
            assert_eq!(rust, c, "Rust resize {i} differs from C at step {step}");
        }
        let [c, rest @ ..] = releases(len, seed);
        assert_eq!(c, expected, "C release, step {step}");
        for (i, rust) in rest.into_iter().enumerate() {
            assert_eq!(rust, c, "Rust release {i} differs from C at step {step}");
        }
    }
    // NULL, which every release accepts.
    // SAFETY: NULL is always accepted.
    unsafe { ffi::git_failalloc_free(ptr::null_mut()) };
    let free = FAIL.as_ref().free().unwrap();
    // SAFETY: as above, through the member.
    unsafe { free(ptr::null_mut()) };
}

/// The release `git__allocator` makes: `git_odb_backend_data_free`, which is `git__free`,
/// which is `git_failalloc_free` before initialisation, or whichever family
/// `set_allocator` installed.
fn installed_free(buf: Option<&mut [u8]>) -> Option<Left> {
    let ptr = buf
        .as_ref()
        .map_or(ptr::null_mut(), |b| b.as_ptr().cast_mut());
    // SAFETY: the backend is never read (`GIT_UNUSED`); `ptr` is NULL or a live buffer
    // this test owns, which the installed family's free ignores (see the module docs).
    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), ptr.cast::<c_void>()) };
    buf.map(|b| b.to_vec())
}

/// What libgit2's allocator does once `git_libgit2_init` has run: the count it returns,
/// and whether `git__malloc` then allocates.
fn after_init() -> (i32, bool) {
    // SAFETY: this child runs one test on one thread, and initialises libgit2 once.
    let count = unsafe { ffi::git_libgit2_init() };
    // SAFETY: the backend is never read; any size may be asked for.
    let data = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 64) };
    let allocated = !data.is_null();
    if allocated {
        // SAFETY: `data` addresses 64 fresh bytes of the installed allocator.
        unsafe { data.cast::<[u8; 64]>().write([7; 64]) };
        // SAFETY: `data` is the installed allocator's, released once.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data) };
    }
    (count, allocated)
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process, and calls C")]
fn cs_initial_family_releases_nothing_and_gives_way_to_init() {
    isolated(
        concat!(
            module_path!(),
            "::cs_initial_family_releases_nothing_and_gives_way_to_init"
        ),
        || {
            // libgit2 is not initialised, so its allocator is C's initial value.
            for (step, len) in [1usize, 16, 4096].into_iter().enumerate() {
                let seed = u8::try_from(step).unwrap();
                let mut theirs = buffer(len, seed);
                let left = installed_free(Some(&mut theirs[..])).unwrap();
                assert_eq!(left, releases(len, seed)[1], "initial release, step {step}");
                assert_eq!(installed_free(None), None);
            }
            assert_eq!(after_init(), (1, true));
            // SAFETY: balances the one `git_libgit2_init`.
            assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process, and calls C")]
fn rust_family_installed_before_init_behaves_as_cs_initial_one() {
    isolated(
        concat!(
            module_path!(),
            "::rust_family_installed_before_init_behaves_as_cs_initial_one"
        ),
        || {
            // The Rust family installed in C's place before initialisation, while libgit2
            // holds no allocation: its release leaves buffers as C's initial value did.
            // SAFETY: an `Allocator::new` value; libgit2 is not initialised and holds
            // nothing; this child runs one test on one thread.
            unsafe { set_allocator(Some(FAIL.as_ref())) };
            for (step, len) in [1usize, 16, 4096].into_iter().enumerate() {
                let seed = u8::try_from(step).unwrap().wrapping_mul(7);
                let mut theirs = buffer(len, seed);
                let left = installed_free(Some(&mut theirs[..])).unwrap();
                assert_eq!(
                    left,
                    releases(len, seed)[0],
                    "installed release, step {step}"
                );
                assert_eq!(installed_free(None), None);
            }
            // Its `gmalloc` is `git_failalloc_malloc` itself, so initialisation replaces
            // it with the standard allocator, as it replaces C's initial value: a family
            // of trampolines would survive it, and libgit2 would fail every allocation.
            assert_eq!(after_init(), (1, true));

            // Installed once libgit2 runs, every later allocation fails, until another
            // allocator is installed. No allocation is released in between.
            // SAFETY: `FailAlloc` documents that it resizes (failing) and releases (as a
            // no-op) storage of any family without touching it, so every allocation
            // libgit2 holds is one it can resize and release; one thread.
            unsafe { set_allocator(Some(FAIL.as_ref())) };
            // SAFETY: the backend is never read; any size may be asked for.
            let data = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 64) };
            assert!(data.is_null());
            // SAFETY: the standard allocator back, which made every allocation libgit2
            // holds; one thread.
            unsafe { set_allocator(None) };
            // SAFETY: the backend is never read; any size may be asked for.
            let data = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 64) };
            assert!(!data.is_null());
            // SAFETY: the standard allocator's, released once.
            unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data) };
            // SAFETY: balances the one `git_libgit2_init`.
            assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
        },
    );
}
