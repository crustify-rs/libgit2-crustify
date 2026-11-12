//! Equivalence of `git2/sys/alloc.h`'s `Allocator` with a raw `git_allocator`, and of the
//! lifecycle strategies with `src/util/alloc.h`'s primitives.
//!
//! - Values: the same member writes on an `Allocator` and on a raw `git_allocator` leave the same
//!   members, step by step.
//! - Behaviour: libgit2 runs one workload on a raw allocator of C-shaped functions and on
//!   `Allocator::new` over an `Alloc` implementation that does the same; every call either receives
//!   is traced, with pointers replaced by allocation numbers, and the traces, return values and
//!   error states are compared after each step.
//!
//! libgit2's allocator is process-global, so the behavioural test reruns alone in a
//! child process. Both allocators forward to libc, so switching between them while
//! blocks are live is sound; each side runs on a fresh thread, so each starts without
//! thread-local error state and releases it on exit, through the allocator under test.
//!
//! Documented divergence, outside the comparison: `free(NULL)` stops at the trampoline
//! and never reaches `Alloc::free`, so the raw side does not trace it either.
//!
//! The strategies, `AllocBox`, `AllocVec` and `AllocStr`, are compared with the primitives
//! of `src/util/alloc.h` they call, as the section below describes; the resizes,
//! `ArrayFree::resize` and `ArrayFree::resize_bytes`, with `git__reallocarray` and
//! `git__realloc`; and the global's borrow and writers, `installed`, `set_allocator` and
//! `global_init`, with reads of `git__allocator`, `git_allocator_setup` and
//! `git_allocator_global_init`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::MaybeUninit;
use std::process::Command;
use std::ptr::{self, NonNull};
use std::sync::Mutex;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::OdbDataStrFree;
use libgit2::libgit2::src::util::alloc::{
    Alloc, AllocBox, AllocStr, AllocVec, Allocator, ArrayFree, FreeFn, MallocFn, ReallocFn, StrFree,
};

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals.
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

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

// --- values -------------------------------------------------------------------------

unsafe extern "C" fn malloc_a(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn malloc_b(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn realloc_a(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn realloc_b(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn free_a(_: *mut c_void) {}
unsafe extern "C" fn free_b(_: *mut c_void) {}

fn same<F: Copy>(a: Option<F>, b: Option<F>, addr: fn(F) -> usize) -> bool {
    a.map(addr) == b.map(addr)
}

fn assert_same_members(rust: &Allocator, c: &ffi::git_allocator) {
    let r = rust.as_ref();
    assert!(same(r.malloc(), c.gmalloc, |f: MallocFn| f as usize));
    assert!(same(r.realloc(), c.grealloc, |f: ReallocFn| f as usize));
    assert!(same(r.free(), c.gfree, |f: FreeFn| f as usize));
}

#[test]
fn member_writes_match_a_raw_allocator() {
    // One stored pointer value per function: each coercion may yield a new address.
    let (ma, mb): (MallocFn, MallocFn) = (malloc_a, malloc_b);
    let (ra, rb): (ReallocFn, ReallocFn) = (realloc_a, realloc_b);
    let (fa, fb): (FreeFn, FreeFn) = (free_a, free_b);

    let mut rust = Allocator::from_fns(ma, ra, fa);
    let mut c = ffi::git_allocator {
        gmalloc: Some(ma),
        grealloc: Some(ra),
        gfree: Some(fa),
    };
    assert_same_members(&rust, &c);

    rust.as_mut().set_malloc(mb);
    c.gmalloc = Some(mb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_free(fb);
    c.gfree = Some(fb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_realloc(rb);
    c.grealloc = Some(rb);
    assert_same_members(&rust, &c);

    rust.as_mut().set_malloc(ma);
    c.gmalloc = Some(ma);
    assert_same_members(&rust, &c);
}

// --- behaviour ------------------------------------------------------------------------

/// The one request size both allocators refuse.
const REFUSED: usize = 1 << 40;

/// One call an allocator received, pointers as they were.
#[derive(Debug)]
enum Raw {
    Malloc {
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: usize,
    },
    Realloc {
        from: usize,
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: usize,
    },
    Free {
        ptr: usize,
    },
}

static TRACING: AtomicBool = AtomicBool::new(false);
static TRACE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());

fn trace(call: Raw) {
    if TRACING.load(SeqCst) {
        TRACE.lock().unwrap().push(call);
    }
}

fn libc_malloc(len: usize) -> *mut c_void {
    if len == REFUSED || (len != 0 && len == REFUSE.load(SeqCst)) {
        return ptr::null_mut();
    }
    // SAFETY: plain libc allocation.
    unsafe { malloc(len) }
}

/// # Safety
///
/// `p` is NULL or a live libc block the caller gives up on success.
unsafe fn libc_realloc(p: *mut c_void, len: usize) -> *mut c_void {
    if len == REFUSED || (len != 0 && len == REFUSE.load(SeqCst)) {
        return ptr::null_mut();
    }
    // SAFETY: per this function's contract.
    unsafe { realloc(p, len) }
}

/// # Safety
///
/// `file` is NULL or a live C string.
unsafe fn file_of(file: *const c_char) -> Option<CString> {
    // SAFETY: per this function's contract.
    (!file.is_null()).then(|| unsafe { CStr::from_ptr(file) }.to_owned())
}

// The C side: functions in `git_allocator`'s own shape.

unsafe extern "C" fn c_traced_malloc(len: usize, file: *const c_char, line: c_int) -> *mut c_void {
    let got = libc_malloc(len);
    // SAFETY: libgit2 passes `__FILE__`.
    let file = unsafe { file_of(file) };
    trace(Raw::Malloc {
        len,
        file,
        line,
        got: got as usize,
    });
    got
}

unsafe extern "C" fn c_traced_realloc(
    p: *mut c_void,
    len: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    // SAFETY: `p` is NULL or a live block of this libc-backed family.
    let got = unsafe { libc_realloc(p, len) };
    // SAFETY: libgit2 passes `__FILE__`.
    let file = unsafe { file_of(file) };
    trace(Raw::Realloc {
        from: p as usize,
        len,
        file,
        line,
        got: got as usize,
    });
    got
}

unsafe extern "C" fn c_traced_free(p: *mut c_void) {
    if !p.is_null() {
        trace(Raw::Free { ptr: p as usize });
    }
    // SAFETY: `p` is NULL or a live libc block, given up.
    unsafe { free(p) }
}

// The Rust side: the same logic behind `Alloc`.

struct Traced;

// SAFETY: forwards to libc's `malloc` family (refusing one size), which meets `Alloc`'s
// contract; the trace sits behind a lock.
unsafe impl Alloc for Traced {
    fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
        let got = libc_malloc(len);
        trace(Raw::Malloc {
            len,
            file: file.map(CStr::to_owned),
            line,
            got: got as usize,
        });
        NonNull::new(got.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        file: Option<&CStr>,
        line: i32,
    ) -> Option<NonNull<u8>> {
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live block of this libc-backed family.
        let got = unsafe { libc_realloc(p, len) };
        trace(Raw::Realloc {
            from: p as usize,
            len,
            file: file.map(CStr::to_owned),
            line,
            got: got as usize,
        });
        NonNull::new(got.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        trace(Raw::Free {
            ptr: p.as_ptr() as usize,
        });
        // SAFETY: `p` is a live libc block, given up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

/// One call with its pointers numbered by the allocation that made them (`None` for
/// NULL, or for a block from before the side began).
#[derive(Debug, PartialEq, Eq)]
enum Event {
    Malloc {
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: Option<usize>,
    },
    Realloc {
        from: Option<usize>,
        len: usize,
        file: Option<CString>,
        line: c_int,
        got: Option<usize>,
    },
    Free {
        block: Option<usize>,
    },
}

/// What one step left behind: its calls, its result and the thread's error.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    events: Vec<Event>,
    result: c_int,
    error: (c_int, CString),
}

#[derive(Default)]
struct Numbering {
    blocks: std::collections::HashMap<usize, usize>,
    next: usize,
}

impl Numbering {
    fn made(&mut self, p: usize) -> Option<usize> {
        (p != 0).then(|| {
            self.blocks.insert(p, self.next);
            self.next += 1;
            self.next - 1
        })
    }

    fn gone(&mut self, p: usize) -> Option<usize> {
        self.blocks.remove(&p)
    }

    fn normalise(&mut self, raw: Vec<Raw>) -> Vec<Event> {
        raw.into_iter()
            .map(|r| match r {
                Raw::Malloc {
                    len,
                    file,
                    line,
                    got,
                } => Event::Malloc {
                    len,
                    file,
                    line,
                    got: self.made(got),
                },
                Raw::Realloc {
                    from,
                    len,
                    file,
                    line,
                    got,
                } => {
                    // A failed resize leaves `from` live under its number.
                    let from_no = if got == 0 {
                        self.blocks.get(&from).copied()
                    } else {
                        self.gone(from)
                    };
                    Event::Realloc {
                        from: from_no,
                        len,
                        file,
                        line,
                        got: self.made(got),
                    }
                }
                Raw::Free { ptr } => Event::Free {
                    block: self.gone(ptr),
                },
            })
            .collect()
    }
}

fn last_error() -> (c_int, CString) {
    {
        // SAFETY: `git_error_last` never returns NULL; the error is copied out before the
        // next libgit2 call on this thread.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: as above.
        let klass = unsafe { (*e).klass };
        // SAFETY: as above.
        let message2 = unsafe { (*e).message };
        // SAFETY: as above.
        unsafe { (klass, CStr::from_ptr(message2).to_owned()) }
    }
}

fn set_str(class: ffi::git_error_t, message: &CStr) -> c_int {
    // SAFETY: `message` is a live C string, which libgit2 copies.
    unsafe { ffi::git_error_set_str(class.cast_signed(), message.as_ptr()) }
}

fn install(allocator: *const ffi::git_allocator) {
    // SAFETY: `allocator` is NULL (the default allocator) or addresses a
    // `git_allocator` with every member set, which libgit2 copies; every family
    // installed here is libc-backed, so live blocks stay valid across the switch.
    let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), allocator) };
    assert_eq!(rc, 0);
}

/// Runs the workload on a fresh thread under the installed allocator; one `Step` per
/// stage, the last being the thread's exit, which releases its error state.
fn workload() -> Vec<Step> {
    let (mut steps, mut numbering) = std::thread::spawn(|| {
        let mut numbering = Numbering::default();
        let mut steps = Vec::new();
        let mut step = |f: &mut dyn FnMut() -> c_int| {
            TRACE.lock().unwrap().clear();
            TRACING.store(true, SeqCst);
            let result = f();
            TRACING.store(false, SeqCst);
            let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
            steps.push(Step {
                events,
                result,
                error: last_error(),
            });
        };
        let long = CString::new("libgit2 grows its message buffer ".repeat(64)).unwrap();
        // The thread's error state is allocated, then its message buffer grows.
        step(&mut || set_str(ffi::GIT_ERROR_CONFIG, c"short"));
        step(&mut || set_str(ffi::GIT_ERROR_ODB, &long));
        step(&mut || set_str(ffi::GIT_ERROR_INDEX, c"shrinks back"));
        // An allocation libgit2 makes and frees for the caller.
        step(&mut || {
            OdbDataStrFree::dup(c"a string").map_or(-1, |s| i32::try_from(s.len()).unwrap())
        });
        // A zero-byte request, which libgit2 makes and releases like any other.
        step(&mut || {
            {
                // SAFETY: the routines ignore `backend`; a NULL result owns nothing, and a
                // block is released once.
                let p = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 0) };
                let got = !p.is_null();
                // SAFETY: as above.
                unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), p) };
                c_int::from(got)
            }
        });
        // A refused allocation, reported as out of memory.
        step(&mut || {
            // SAFETY: the routine ignores `backend`; a NULL result owns nothing.
            let p = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), REFUSED) };
            c_int::from(p.is_null())
        });
        step(&mut || {
            // SAFETY: no arguments.
            unsafe { ffi::git_error_clear() };
            0
        });
        // Trace the thread's exit, which frees its error state after this returns.
        TRACE.lock().unwrap().clear();
        TRACING.store(true, SeqCst);
        (steps, numbering)
    })
    .join()
    .unwrap();
    TRACING.store(false, SeqCst);
    let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
    steps.push(Step {
        events,
        result: 0,
        error: (0, CString::default()),
    });
    steps
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn libgit2_drives_both_allocators_alike() {
    isolated(
        concat!(module_path!(), "::libgit2_drives_both_allocators_alike"),
        || {
            let c_side = ffi::git_allocator {
                gmalloc: Some(c_traced_malloc),
                grealloc: Some(c_traced_realloc),
                gfree: Some(c_traced_free),
            };
            let rust_side = Allocator::new::<Traced>();
            install(&raw const c_side);
            // SAFETY: no arguments; refcounted, never released in this process.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);

            let run = |allocator: *const ffi::git_allocator| {
                install(allocator);
                workload()
            };
            let c_steps = run(&raw const c_side);
            let rust_steps = run((&raw const rust_side).cast());
            // NULL restores the default allocator, libc's as well.
            install(ptr::null());

            assert_eq!(c_steps.len(), rust_steps.len());
            for (i, (c, rust)) in c_steps.iter().zip(&rust_steps).enumerate() {
                assert_eq!(c, rust, "step {i}");
            }
            // The comparison saw real traffic of every kind.
            let all: Vec<_> = c_steps.iter().flat_map(|s| &s.events).collect();
            assert!(
                all.iter()
                    .any(|e| matches!(e, Event::Malloc { got: Some(_), .. }))
            );
            assert!(all.iter().any(|e| matches!(
                e,
                Event::Malloc {
                    got: None,
                    len: REFUSED,
                    ..
                }
            )));
            assert!(all.iter().any(|e| matches!(
                e,
                Event::Realloc {
                    from: Some(_),
                    got: Some(_),
                    ..
                }
            )));
            assert!(
                all.iter()
                    .any(|e| matches!(e, Event::Free { block: Some(_) }))
            );
            // The zero-byte request was an allocation of its own, released once.
            assert_eq!(c_steps[4].result, 1);
            assert!(
                matches!(
                    c_steps[4].events[..],
                    [
                        Event::Malloc { len: 0, got: Some(n), .. },
                        Event::Free { block: Some(m) }
                    ] if n == m
                ),
                "{:?}",
                c_steps[4].events
            );
            assert_eq!(c_steps[5].error.0, ffi::GIT_ERROR_NOMEMORY.cast_signed());
        },
    );
}

// --- the strategies -------------------------------------------------------------------
//
// Each strategy is compared with the primitive it stands in for, called through its raw
// binding (`strategies_match_their_primitives`): `git__malloc` and `git__calloc` with
// `AllocBox`, `git__calloc` and `git__mallocarray` with `ArrayFree`'s runs, empty ones
// included, `git__strdup`, `git__strndup` and `git__substrdup` with `StrFree`, and
// `git__free` with each owner's drop. Each step's allocator calls, file and line
// included, its result and the thread's error are compared, side by side, and so is what
// each allocation holds; the mixed sides release what the other side made.
//
// Then through the exported routines that call the primitives
// (`strategies_allocate_and_release_as_libgit2_does`): `git_signature_new`
// (`git__calloc(1, sizeof)` and two `git__substrdup`s of the trimmed name and email),
// `git_signature_dup` (`git__calloc` and two `git__strdup`s) and `git_signature_free`
// (three `git__free`s), which the Rust side builds and releases through `AllocBox`,
// `StrFree` and `AllocStr`. Not compared there: refusals after the first allocation,
// where `git_signature_new` and `git_signature_dup` leak what they built (C defects,
// LSan).

/// A request size the C-shaped tracer refuses besides [`REFUSED`]; 0 refuses none.
static REFUSE: AtomicUsize = AtomicUsize::new(0);

/// One request of the primitives' workload.
#[derive(Clone, Copy, Debug)]
enum Req {
    /// `git__malloc(sizeof(git_signature))`, `AllocBox::new_uninit`.
    Malloc,
    /// `git__calloc(1, sizeof(git_signature))`, `AllocBox::new_zeroed`.
    CallocOne,
    /// `git__calloc(n, 8)`, `ArrayFree::new_zeroed::<u64>(n)`.
    CallocRun(usize),
    /// `git__mallocarray(n, 4)`, `ArrayFree::new_uninit::<u32>(n)`.
    MallocArray(usize),
    /// `git__strdup(s)`, `StrFree::dup(s)`.
    Strdup(&'static CStr),
    /// `git__strdup(s)` again, through `AllocStr`'s `Clone` (`CDupClone::c_dup`).
    StrClone(&'static CStr),
    /// `git__strndup(run, run.len())`, `StrFree::dup_prefix(run)`.
    Strndup(&'static [u8]),
    /// `git__substrdup(run, run.len())`, `StrFree::dup_bytes(run)`.
    Substrdup(&'static [u8]),
}

/// What a request made, and how to release it.
#[derive(Clone, Copy, Debug)]
enum Made {
    Box(*mut c_void),
    Run(*mut c_void, usize),
    Str(*mut c_char),
}

/// A pointer C may read `run.len()` bytes through, as the strategies pass it.
fn run_ptr(run: &[u8]) -> *const c_char {
    if run.is_empty() {
        c"".as_ptr()
    } else {
        run.as_ptr().cast()
    }
}

/// The bytes at `p`, `len` of them; none at NULL.
///
/// # Safety
///
/// `p` is NULL or valid for reads of `len` initialised bytes.
unsafe fn bytes_at(p: *const c_void, len: usize) -> Vec<u8> {
    if len == 0 || p.is_null() {
        return Vec::new();
    }
    // SAFETY: per this function's contract.
    unsafe { std::slice::from_raw_parts(p.cast::<u8>(), len) }.to_vec()
}

/// Makes `req` on `side`: 1 and what it holds when it made something, -1 when it was
/// refused.
fn request(side: Side, req: Req) -> (c_int, Vec<u8>, Option<Made>) {
    let sig = size_of::<ffi::git_signature>();
    let (made, held) = match (side, req) {
        // Each side fills its uninitialised signature, untraced, so that every box
        // released holds one.
        (Side::C, Req::Malloc) => {
            // SAFETY: any size.
            let p = unsafe { ffi::git__malloc(sig) };
            if !p.is_null() {
                // SAFETY: a fresh allocation of `sig` bytes, aligned for the struct.
                unsafe {
                    p.cast::<ffi::git_signature>()
                        .write(ffi::git_signature::default());
                }
            }
            (NonNull::new(p).map(|p| Made::Box(p.as_ptr())), Vec::new())
        }
        (Side::Rust, Req::Malloc) => (
            AllocBox::<Sig>::new_uninit().map(|mut b| {
                // SAFETY: the box's fresh storage for one `git_signature`.
                unsafe { b.as_c_ptr().write(ffi::git_signature::default()) };
                // SAFETY: just filled.
                Made::Box(unsafe { b.assume_init() }.into_raw().cast())
            }),
            Vec::new(),
        ),
        (Side::C, Req::CallocOne) => {
            // SAFETY: any sizes.
            let p = unsafe { ffi::git__calloc(1, sig) };
            // SAFETY: a non-NULL `p` holds `sig` zeroed bytes.
            let held = unsafe { bytes_at(p, sig) };
            (NonNull::new(p).map(|p| Made::Box(p.as_ptr())), held)
        }
        (Side::Rust, Req::CallocOne) => match AllocBox::<Sig>::new_zeroed() {
            Some(b) => {
                let p = b.into_raw().cast::<c_void>();
                // SAFETY: `p` holds `sig` zeroed bytes.
                (Some(Made::Box(p)), unsafe { bytes_at(p, sig) })
            }
            None => (None, Vec::new()),
        },
        (Side::C, Req::CallocRun(n)) => {
            // SAFETY: any sizes, an overflowing product refused.
            let p = unsafe { ffi::git__calloc(n, 8) };
            // SAFETY: a non-NULL `p` holds `8 * n` zeroed bytes, a product that does not
            // overflow then; a NULL one is never read.
            let held = unsafe { bytes_at(p, n.wrapping_mul(8)) };
            (NonNull::new(p).map(|p| Made::Run(p.as_ptr(), n)), held)
        }
        (Side::Rust, Req::CallocRun(n)) => match ArrayFree::new_zeroed::<u64>(n) {
            Some(v) => {
                let held = v.as_slice().iter().flat_map(|w| w.to_ne_bytes()).collect();
                let (p, n) = v.into_raw_parts();
                (Some(Made::Run(p.cast(), n)), held)
            }
            None => (None, Vec::new()),
        },
        (Side::C, Req::MallocArray(n)) => {
            // SAFETY: any sizes, an overflowing product refused.
            let p = unsafe { ffi::git__mallocarray(n, 4) };
            (
                NonNull::new(p).map(|p| Made::Run(p.as_ptr(), n)),
                Vec::new(),
            )
        }
        (Side::Rust, Req::MallocArray(n)) => (
            ArrayFree::new_uninit::<u32>(n).map(|v| {
                let (p, n) = v.into_raw_parts();
                Made::Run(p.cast(), n)
            }),
            Vec::new(),
        ),
        (Side::C, Req::Strdup(s) | Req::StrClone(s)) => {
            // SAFETY: a live C string, read for the call.
            str_made(unsafe { ffi::git__strdup(s.as_ptr()) }, None)
        }
        (Side::Rust, Req::Strdup(s)) => str_made(
            StrFree::dup(s).map_or(ptr::null_mut(), AllocStr::into_raw),
            None,
        ),
        (Side::Rust, Req::StrClone(s)) => {
            // The source is made untraced and unrefused: only the copy is compared.
            TRACING.store(false, SeqCst);
            let refuse = REFUSE.swap(0, SeqCst);
            let src = StrFree::dup(s).unwrap();
            REFUSE.store(refuse, SeqCst);
            TRACING.store(true, SeqCst);
            let copy = src.try_clone();
            TRACING.store(false, SeqCst);
            drop(src);
            TRACING.store(true, SeqCst);
            str_made(copy.map_or(ptr::null_mut(), AllocStr::into_raw), None)
        }
        (Side::C, Req::Strndup(run)) => {
            // SAFETY: C reads at most `run.len()` bytes, all in `run`.
            str_made(unsafe { ffi::git__strndup(run_ptr(run), run.len()) }, None)
        }
        (Side::Rust, Req::Strndup(run)) => str_made(
            StrFree::dup_prefix(run).map_or(ptr::null_mut(), AllocStr::into_raw),
            None,
        ),
        (Side::C, Req::Substrdup(run)) => str_made(
            // SAFETY: C copies exactly `run.len()` bytes, all in `run`.
            unsafe { ffi::git__substrdup(run_ptr(run), run.len()) },
            Some(run.len() + 1),
        ),
        (Side::Rust, Req::Substrdup(run)) => str_made(
            StrFree::dup_bytes(run).map_or(ptr::null_mut(), AllocStr::into_raw),
            Some(run.len() + 1),
        ),
    };
    (if made.is_some() { 1 } else { -1 }, held, made)
}

/// A string request's outcome: what the copy holds, through its terminator, or all `len`
/// bytes of its allocation.
fn str_made(p: *mut c_char, len: Option<usize>) -> (Option<Made>, Vec<u8>) {
    if p.is_null() {
        return (None, Vec::new());
    }
    let held = match len {
        // SAFETY: a copy of `n - 1` bytes and its terminator spans `n` bytes.
        Some(n) => unsafe { bytes_at(p.cast(), n) },
        // SAFETY: a live NUL-terminated copy.
        None => unsafe { CStr::from_ptr(p) }.to_bytes_with_nul().to_vec(),
    };
    (Some(Made::Str(p)), held)
}

/// Releases what a request made, as `side` releases it.
fn release(side: Side, made: Made) -> c_int {
    match (side, made) {
        (Side::C, Made::Box(p) | Made::Run(p, _)) => {
            // SAFETY: a live allocation of libgit2's allocator, given up.
            unsafe { ffi::git__free(p) }
        }
        (Side::C, Made::Str(p)) => {
            // SAFETY: as above.
            unsafe { ffi::git__free(p.cast()) }
        }
        (Side::Rust, Made::Box(p)) => {
            // SAFETY: a live `git_signature` in libgit2's allocator, given up to the box,
            // which releases it without reading it.
            drop(unsafe { AllocBox::<Sig>::from_c(p.cast()) });
        }
        (Side::Rust, Made::Run(p, n)) => {
            // SAFETY: a live allocation of libgit2's allocator for `n` elements, given up
            // to the run, which reads none of them.
            drop(unsafe { AllocVec::<MaybeUninit<u64>>::from_raw_parts(p.cast(), n) });
        }
        (Side::Rust, Made::Str(p)) => {
            // SAFETY: a live NUL-terminated string of libgit2's allocator, given up.
            drop(unsafe { AllocStr::from_raw(p) });
        }
    }
    0
}

/// The requests the primitives' workload makes, and the size whose refusal fails each.
const REQUESTS: [(Req, usize); 14] = [
    (Req::Malloc, size_of::<ffi::git_signature>()),
    (Req::CallocOne, size_of::<ffi::git_signature>()),
    (Req::CallocRun(5), 40),
    (Req::CallocRun(0), 0),
    (Req::MallocArray(3), 12),
    (Req::MallocArray(0), 0),
    (Req::Strdup(c"refs/heads/main"), 16),
    (Req::Strdup(c""), 1),
    (Req::StrClone(c"origin"), 7),
    (Req::Strndup(b"ab\0cd"), 3),
    (Req::Strndup(b"abc"), 4),
    (Req::Strndup(b""), 1),
    (Req::Substrdup(b"ab\0cd"), 6),
    (Req::Substrdup(b""), 1),
];

/// Makes every request on the `make` side, refuses each in turn, makes two whose size
/// overflows, and releases what was made on the `free` side; one `Step` per call, and
/// what each request made holds.
fn primitive_workload(make: Side, free: Side) -> (Vec<Step>, Vec<Vec<u8>>) {
    let mut numbering = Numbering::default();
    let mut steps = Vec::new();
    let mut step = |f: &mut dyn FnMut() -> c_int| {
        set_str(ffi::GIT_ERROR_INVALID, c"marker");
        TRACE.lock().unwrap().clear();
        TRACING.store(true, SeqCst);
        let result = f();
        TRACING.store(false, SeqCst);
        let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
        steps.push(Step {
            events,
            result,
            error: last_error(),
        });
    };
    let mut blocks = Vec::new();
    let mut held = Vec::new();
    for (req, _) in REQUESTS {
        step(&mut || {
            let (rc, bytes, m) = request(make, req);
            blocks.extend(m);
            held.push(bytes);
            rc
        });
    }
    for (req, size) in REQUESTS {
        if size == 0 {
            continue;
        }
        REFUSE.store(size, SeqCst);
        step(&mut || {
            let (rc, _, m) = request(make, req);
            assert!(m.is_none(), "{req:?}");
            rc
        });
        REFUSE.store(0, SeqCst);
    }
    for req in [
        Req::CallocRun(usize::MAX / 4),
        Req::MallocArray(usize::MAX / 2),
    ] {
        step(&mut || request(make, req).0);
    }
    for m in blocks {
        step(&mut || release(free, m));
    }
    (steps, held)
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn strategies_match_their_primitives() {
    isolated(
        concat!(module_path!(), "::strategies_match_their_primitives"),
        || {
            let c_side = ffi::git_allocator {
                gmalloc: Some(c_traced_malloc),
                grealloc: Some(c_traced_realloc),
                gfree: Some(c_traced_free),
            };
            install(&raw const c_side);
            // SAFETY: no arguments; refcounted, never released in this process.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);
            // The thread's error state is allocated before any step.
            set_str(ffi::GIT_ERROR_INVALID, c"marker");

            let (c_steps, c_held) = primitive_workload(Side::C, Side::C);
            for (make, free) in [
                (Side::Rust, Side::Rust),
                (Side::C, Side::Rust),
                (Side::Rust, Side::C),
            ] {
                let (steps, held) = primitive_workload(make, free);
                assert_eq!(held, c_held, "{make:?}/{free:?}");
                assert_eq!(steps.len(), c_steps.len());
                for (i, (c, rust)) in c_steps.iter().zip(&steps).enumerate() {
                    assert_eq!(c, rust, "{make:?}/{free:?}, step {i}");
                }
            }
            install(ptr::null());

            // The comparison saw what it is about: requests through `gmalloc` and, for
            // `git__mallocarray`, `grealloc`; zero-byte blocks; refusals and overflows
            // reported as out of memory; and one release per block made.
            let sig = size_of::<ffi::git_signature>();
            let malloc = |s: &Step| match s.events[..] {
                [Event::Malloc { len, got, .. }] => Some((len, got.is_some())),
                _ => None,
            };
            assert_eq!(malloc(&c_steps[0]), Some((sig, true)));
            assert_eq!(malloc(&c_steps[3]), Some((0, true)));
            assert!(matches!(
                c_steps[4].events[..],
                [Event::Realloc {
                    from: None,
                    len: 12,
                    got: Some(_),
                    ..
                }]
            ));
            assert!(matches!(
                c_steps[5].events[..],
                [Event::Realloc {
                    from: None,
                    len: 0,
                    got: Some(_),
                    ..
                }]
            ));
            assert_eq!(c_held[1], vec![0; sig]);
            assert_eq!(c_held[2], vec![0; 40]);
            assert_eq!(c_held[9], b"ab\0");
            assert_eq!(c_held[12], b"ab\0cd\0");
            let refused = REQUESTS.iter().filter(|(_, size)| *size != 0).count();
            let failures = &c_steps[REQUESTS.len()..REQUESTS.len() + refused + 2];
            assert!(
                failures
                    .iter()
                    .all(|s| s.result == -1 && s.error.0 == ffi::GIT_ERROR_NOMEMORY.cast_signed())
            );
            // The overflows asked the allocator nothing.
            assert!(failures[refused..].iter().all(|s| s.events.is_empty()));
            let releases = &c_steps[REQUESTS.len() + refused + 2..];
            assert_eq!(releases.len(), REQUESTS.len());
            assert!(
                releases
                    .iter()
                    .all(|s| matches!(s.events[..], [Event::Free { block: Some(_) }]))
            );
        },
    );
}

ffibox::define_ctype!(Sig, SigRef, SigMut, ffi::git_signature);

// SAFETY: all zero is two NULL strings and a zero time, which `git_signature_free` and
// `rust_free` accept, and which nothing reads before it is filled.
unsafe impl CZeroable for Sig {}

/// `signature.c`'s `is_crud`.
fn is_crud(c: u8) -> bool {
    c <= 32 || b",:;<>\"\\'".contains(&c)
}

/// `signature.c`'s `extract_trimmed`, before its copy.
fn trimmed(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&c| !is_crud(c)).unwrap_or(s.len());
    let s = &s[start..];
    let end = s.iter().rposition(|&c| !is_crud(c)).map_or(0, |i| i + 1);
    &s[..end]
}

/// One side's routines.
#[derive(Clone, Copy, Debug)]
enum Side {
    C,
    Rust,
}

const NAME: &CStr = c"  Ann Example ";
const EMAIL: &CStr = c" ann@example.org; ";

/// `git_signature_new(NAME, EMAIL, 1234, -90)`, as the side builds it.
fn sig_new(side: Side, out: &mut *mut ffi::git_signature) -> c_int {
    match side {
        Side::C => {
            // SAFETY: `out` is writable; both strings are live C strings.
            unsafe { ffi::git_signature_new(out, NAME.as_ptr(), EMAIL.as_ptr(), 1234, -90) }
        }
        Side::Rust => {
            let Some(sig) = AllocBox::<Sig>::new_zeroed() else {
                return -1;
            };
            let name = StrFree::dup_bytes(trimmed(NAME.to_bytes())).unwrap();
            let email = StrFree::dup_bytes(trimmed(EMAIL.to_bytes())).unwrap();
            let when = ffi::git_time {
                time: 1234,
                offset: -90,
                sign: b'-'.cast_signed(),
            };
            *out = fill(sig, name, email, when);
            0
        }
    }
}

/// Moves the strings into the boxed signature and hands it over.
fn fill(
    sig: AllocBox<Sig>,
    name: AllocStr,
    email: AllocStr,
    when: ffi::git_time,
) -> *mut ffi::git_signature {
    let p = sig.into_c();
    let filled = ffi::git_signature {
        name: name.into_raw(),
        email: email.into_raw(),
        when,
    };
    // SAFETY: `p` is the live, zeroed `git_signature` the box gave up; the strings move
    // into it, and `git_signature_free` or `sig_free` releases all three.
    unsafe { p.write(filled) };
    p
}

/// `git_signature_dup(src)`, as the side copies it.
fn sig_dup(side: Side, out: &mut *mut ffi::git_signature, src: *const ffi::git_signature) -> c_int {
    match side {
        // SAFETY: `out` is writable; `src` is a live signature.
        Side::C => unsafe { ffi::git_signature_dup(out, src) },
        Side::Rust => {
            let Some(sig) = AllocBox::<Sig>::new_zeroed() else {
                return -1;
            };
            // SAFETY: `src` is a live signature.
            let src = unsafe { src.read() };
            // SAFETY: its strings are live and NUL-terminated.
            let name = StrFree::dup(unsafe { CStr::from_ptr(src.name) }).unwrap();
            // SAFETY: as above.
            let email = StrFree::dup(unsafe { CStr::from_ptr(src.email) }).unwrap();
            *out = fill(sig, name, email, src.when);
            0
        }
    }
}

/// `git_signature_free(sig)`, as the side releases it.
fn sig_free(side: Side, sig: *mut ffi::git_signature) -> c_int {
    match side {
        // SAFETY: `sig` is a live signature of libgit2's allocator, given up.
        Side::C => unsafe { ffi::git_signature_free(sig) },
        Side::Rust => {
            // SAFETY: `sig` is a live signature.
            let raw = unsafe { sig.read() };
            // The struct and its strings are allocations of libgit2's allocator, all
            // given up; each is adopted once, and released in `git_signature_free`'s
            // order: name, email, struct.
            // SAFETY: as above.
            drop(unsafe { AllocStr::from_raw(raw.name) });
            // SAFETY: as above.
            drop(unsafe { AllocStr::from_raw(raw.email) });
            // SAFETY: as above.
            drop(unsafe { AllocBox::<Sig>::from_c(sig) });
        }
    }
    0
}

/// What a signature holds.
fn contents(sig: *const ffi::git_signature) -> Contents {
    // SAFETY: `sig` is a live signature.
    let sig = unsafe { sig.read() };
    // SAFETY: its strings are live and NUL-terminated.
    let name = unsafe { CStr::from_ptr(sig.name) }.to_owned();
    // SAFETY: as above.
    let email = unsafe { CStr::from_ptr(sig.email) }.to_owned();
    (name, email, sig.when.time, sig.when.offset, sig.when.sign)
}

type Contents = (CString, CString, i64, c_int, c_char);

/// Builds, copies, fails to build and releases signatures with the given sides'
/// constructor, copier and releaser; one `Step` per call.
fn signature_workload(new: Side, dup: Side, free: Side) -> (Vec<Step>, Vec<Contents>) {
    let mut numbering = Numbering::default();
    let mut steps = Vec::new();
    let mut step = |f: &mut dyn FnMut() -> c_int| {
        // A marker error, which a step that reports nothing leaves in place.
        set_str(ffi::GIT_ERROR_INVALID, c"marker");
        TRACE.lock().unwrap().clear();
        TRACING.store(true, SeqCst);
        let result = f();
        TRACING.store(false, SeqCst);
        let events = numbering.normalise(std::mem::take(&mut *TRACE.lock().unwrap()));
        steps.push(Step {
            events,
            result,
            error: last_error(),
        });
    };
    let (mut p, mut q, mut r) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    step(&mut || sig_new(new, &mut p));
    step(&mut || sig_dup(dup, &mut q, p));
    let built = vec![contents(p), contents(q)];
    // The first allocation refused: nothing is built, out of memory is reported.
    REFUSE.store(size_of::<ffi::git_signature>(), SeqCst);
    step(&mut || sig_new(new, &mut r));
    step(&mut || sig_dup(dup, &mut r, p));
    REFUSE.store(0, SeqCst);
    assert!(r.is_null());
    step(&mut || sig_free(free, q));
    step(&mut || sig_free(free, p));
    (steps, built)
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn strategies_allocate_and_release_as_libgit2_does() {
    isolated(
        concat!(
            module_path!(),
            "::strategies_allocate_and_release_as_libgit2_does"
        ),
        || {
            let c_side = ffi::git_allocator {
                gmalloc: Some(c_traced_malloc),
                grealloc: Some(c_traced_realloc),
                gfree: Some(c_traced_free),
            };
            install(&raw const c_side);
            // SAFETY: no arguments; refcounted, never released in this process.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);
            // The thread's error state is allocated before any step.
            set_str(ffi::GIT_ERROR_INVALID, c"marker");

            let (c_steps, c_built) = signature_workload(Side::C, Side::C, Side::C);
            for (new, dup, free) in [
                (Side::Rust, Side::Rust, Side::Rust),
                // Rust copies and releases what C built.
                (Side::C, Side::Rust, Side::Rust),
                // C copies and releases what Rust built.
                (Side::Rust, Side::C, Side::C),
            ] {
                let (steps, built) = signature_workload(new, dup, free);
                assert_eq!(built, c_built, "{new:?}/{dup:?}/{free:?}");
                assert_eq!(steps.len(), c_steps.len());
                for (i, (c, rust)) in c_steps.iter().zip(&steps).enumerate() {
                    assert_eq!(c, rust, "{new:?}/{dup:?}/{free:?}, step {i}");
                }
            }
            install(ptr::null());

            // The comparison saw what it is about.
            let sig = size_of::<ffi::git_signature>();
            let lens = |s: &Step| {
                s.events
                    .iter()
                    .map(|e| match e {
                        Event::Malloc { len, got, .. } => (*len, got.is_some()),
                        Event::Free { block } => (0, block.is_some()),
                        Event::Realloc { .. } => unreachable!("no resize"),
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(lens(&c_steps[0]), [(sig, true), (12, true), (16, true)]);
            assert_eq!(lens(&c_steps[1]), [(sig, true), (12, true), (16, true)]);
            assert_eq!(lens(&c_steps[2]), [(sig, false)]);
            assert_eq!(lens(&c_steps[4]), [(0, true); 3]);
            assert_eq!(c_steps[0].error.1.as_c_str(), c"marker");
            assert_eq!(c_steps[2].result, -1);
            assert_eq!(c_steps[2].error.0, ffi::GIT_ERROR_NOMEMORY.cast_signed());
            assert_eq!(
                c_built[0],
                (
                    c"Ann Example".to_owned(),
                    c"ann@example.org".to_owned(),
                    1234,
                    -90,
                    b'-'.cast_signed()
                )
            );
        },
    );
}

// --- the resizes ------------------------------------------------------------------------
//
// One run on each side, filled alike, goes through the same lengths: C's a raw buffer
// resized by `git__reallocarray` or `git__realloc`, Rust's an `AllocVec` resized by
// `ArrayFree::resize` or `ArrayFree::resize_bytes`. After each step the outcome, the error
// class of a failure and the kept prefix are compared, and both new tails are written
// alike. Documented divergences, outside the comparison: a resize to zero, which Rust
// answers by releasing the run where C would ask the allocator for an
// implementation-defined zero-byte resize no libgit2 path makes; and a size past
// `isize::MAX` bytes that does not overflow, which Rust refuses before the allocator is
// asked.

mod resizes {
    use std::ffi::{CStr, c_int, c_void};
    use std::mem::MaybeUninit;
    use std::ptr::{self, NonNull};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::settings::set_allocator;
    use libgit2::libgit2::src::util::alloc::{Alloc, AllocVec, Allocator, ArrayFree};
    use libgit2::libgit2::src::util::errors;

    use super::isolated;

    /// The value element `i` of either run holds after step `step`.
    fn value(step: usize, i: usize) -> u32 {
        u32::try_from(step * 1000 + i).unwrap()
    }

    /// C's run: its storage and length.
    struct CRun {
        data: *mut c_void,
        len: usize,
    }

    impl CRun {
        fn elements(&self, n: usize) -> Vec<u32> {
            if n == 0 {
                return Vec::new();
            }
            // SAFETY: the first `n` elements of the live storage were written.
            unsafe { std::slice::from_raw_parts(self.data.cast::<u32>(), n) }.to_vec()
        }

        fn fill(&mut self, from: usize, step: usize) {
            for i in from..self.len {
                // SAFETY: `i < len`, an element of the live storage.
                let slot = unsafe { self.data.cast::<u32>().add(i) };
                // SAFETY: as above; the storage is aligned for `u32`.
                unsafe { slot.write(value(step, i)) };
            }
        }
    }

    fn rust_elements(run: &AllocVec<MaybeUninit<u32>>, n: usize) -> Vec<u32> {
        run.as_slice()[..n]
            .iter()
            // SAFETY: the first `n` elements were written.
            .map(|e| unsafe { e.assume_init() })
            .collect()
    }

    fn rust_fill(run: &mut AllocVec<MaybeUninit<u32>>, from: usize, step: usize) {
        for (i, slot) in run.as_mut_slice().iter_mut().enumerate().skip(from) {
            slot.write(value(step, i));
        }
    }

    /// The class of the thread's last libgit2 error, read after a failed C call.
    fn c_error_class() -> c_int {
        // SAFETY: no arguments.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: `git_error_last` never returns NULL; the class is read at once.
        unsafe { (*e).klass }
    }

    /// Resizes both runs to each length of `lens` in turn, `bytes` choosing the resizes
    /// over bytes (`git__realloc`, `resize_bytes`, with 4-byte elements counted as 4 bytes)
    /// rather than elements.
    fn compare(start: usize, lens: &[usize], bytes: bool) {
        errors::clear();
        let data = if start == 0 {
            ptr::null_mut()
        } else {
            // SAFETY: any sizes; a fresh run of `start` elements.
            unsafe { ffi::git__mallocarray(start, 4) }
        };
        let mut c = CRun { data, len: start };
        let mut rust = if start == 0 {
            AllocVec::empty()
        } else {
            ArrayFree::new_uninit::<u32>(start).unwrap()
        };
        c.fill(0, 0);
        rust_fill(&mut rust, 0, 0);
        for (step, &len) in lens.iter().enumerate().map(|(i, l)| (i + 1, l)) {
            let kept = c.len.min(len);
            let resized = if bytes {
                // SAFETY: `c.data` is NULL or the live run, given up on success; the size
                // is non-zero.
                unsafe { ffi::git__realloc(c.data, len * 4) }
            } else {
                // SAFETY: as above.
                unsafe { ffi::git__reallocarray(c.data, len, 4) }
            };
            let c_class = resized.is_null().then(c_error_class);
            let rust_out = if bytes {
                // The byte resize sees the run as bytes: a run of `u32` reinterpreted is
                // what C's `git__realloc` callers resize.
                let (p, n) = std::mem::take(&mut rust).into_raw_parts();
                // SAFETY: the run's storage, `n * 4` bytes of libgit2's allocator, adopted
                // as bytes by a run of the same policy.
                let mut as_bytes = unsafe {
                    AllocVec::<MaybeUninit<u8>>::from_raw_parts_or_empty(p.cast(), n * 4)
                }
                .unwrap();
                let out = ArrayFree::resize_bytes(&mut as_bytes, len * 4);
                let (p, n) = as_bytes.into_raw_parts();
                // SAFETY: the storage, resized or not, back as the elements it holds; its
                // byte length is a multiple of 4.
                rust = unsafe { AllocVec::from_raw_parts_or_empty(p.cast(), n / 4) }.unwrap();
                out
            } else {
                ArrayFree::resize(&mut rust, len)
            };
            assert_eq!(
                c_class,
                rust_out.as_ref().err().map(errors::ErrorBuf::raw_class),
                "step {step}: resize to {len}"
            );
            if let Some(resized) = NonNull::new(resized) {
                c = CRun {
                    data: resized.as_ptr(),
                    len,
                };
                c.fill(kept, step);
                rust_fill(&mut rust, kept, step);
            }
            assert_eq!(c.len, rust.len(), "step {step}");
            assert_eq!(
                c.elements(c.len),
                rust_elements(&rust, rust.len()),
                "step {step}"
            );
        }
        // SAFETY: the live run, given up once.
        unsafe { ffi::git__free(c.data) };
        drop(rust);
    }

    /// The lengths each comparison walks: growth, shrinkage, the same length, past a page,
    /// and back.
    const LENS: [usize; 7] = [5, 2, 9, 9, 1, 4096, 3];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn element_resizes_match_reallocarray_step_by_step() {
        compare(3, &LENS, false);
        // A run without storage is allocated afresh by both.
        compare(0, &LENS, false);
        // An overflowing product is refused by both, the run left as it was.
        compare(2, &[usize::MAX / 2 + 1, 4, usize::MAX / 4 + 1, 1], false);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn byte_resizes_match_realloc_step_by_step() {
        compare(3, &LENS, true);
        compare(0, &LENS, true);
    }

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// The one resize size, in bytes, [`Refusing`] refuses.
    const REFUSED: usize = 4 * 4242;

    /// libc's allocator, refusing every request of [`REFUSED`] bytes.
    pub(super) struct Refusing;

    // SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract,
    // but for refusals, which leave every block as it was.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            // SAFETY: plain libc allocation.
            (len != REFUSED).then(|| NonNull::new(unsafe { malloc(len) }.cast()))?
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

    static REFUSING: Allocator = Allocator::new::<Refusing>();

    /// A resize the allocator refuses fails alike on both sides, each run kept whole, and
    /// the next resize succeeds on both.
    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn refused_resizes_match() {
        isolated(concat!(module_path!(), "::refused_resizes_match"), || {
            // SAFETY: `Refusing` meets `Alloc`'s contract; installed before libgit2 holds
            // any allocation, in a process of its own.
            unsafe { set_allocator(Some(REFUSING.as_ref())) };
            compare(3, &[REFUSED / 4, 7, REFUSED / 4, 2], false);
            compare(3, &[REFUSED / 4, 7, REFUSED / 4, 2], true);
            compare(0, &[REFUSED / 4, 1], true);
        });
    }
}

// --- the installed allocator ------------------------------------------------------------
//
// In a child process, before libgit2 is initialised, where it holds no allocation: the
// same installs made through Rust and through C's routines leave the same global, which
// `installed` reads as C does.

mod installed {
    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::settings::set_allocator;
    use libgit2::libgit2::src::util::alloc::{Allocator, global_init, installed};
    use libgit2::libgit2::src::util::allocators::failalloc::FailAlloc;

    use super::isolated;

    /// The global's members as C reads them, by address.
    fn c_reads() -> [usize; 3] {
        // SAFETY: a read of the global, which no other thread writes in this process.
        let g = unsafe { (&raw const ffi::git__allocator).read() };
        [
            g.gmalloc.map_or(0, |f| f as usize),
            g.grealloc.map_or(0, |f| f as usize),
            g.gfree.map_or(0, |f| f as usize),
        ]
    }

    /// The global's members as `installed` reads them, by address.
    fn rust_reads() -> [usize; 3] {
        // SAFETY: no other thread runs in this process, so none writes the global.
        let g = unsafe { installed() };
        [
            g.malloc().map_or(0, |f| f as usize),
            g.realloc().map_or(0, |f| f as usize),
            g.free().map_or(0, |f| f as usize),
        ]
    }

    /// The raw value of `a`, for C's routines to copy.
    fn raw(a: &Allocator) -> ffi::git_allocator {
        let r = a.as_ref();
        ffi::git_allocator {
            gmalloc: r.malloc(),
            grealloc: r.realloc(),
            gfree: r.free(),
        }
    }

    static FAIL: Allocator = Allocator::new::<FailAlloc>();

    /// A family over libc's allocator.
    static LIBC: Allocator = Allocator::new::<super::resizes::Refusing>();

    /// `a`'s members, by address.
    fn addrs_of(a: &Allocator) -> [usize; 3] {
        let r = a.as_ref();
        [
            r.malloc().map_or(0, |f| f as usize),
            r.realloc().map_or(0, |f| f as usize),
            r.free().map_or(0, |f| f as usize),
        ]
    }

    /// `allocator`'s install through `set_allocator` and through `git_allocator_setup`,
    /// each from the failing family, and the global each leaves.
    fn both_setups(allocator: Option<&Allocator>) -> ([usize; 3], [usize; 3]) {
        let mut failing = raw(&FAIL);
        // SAFETY: no other thread runs, and libgit2 holds no allocation before
        // initialisation, so any family may replace any other; C copies the live value.
        unsafe { ffi::git_allocator_setup(&raw mut failing) };
        // SAFETY: as above.
        unsafe { set_allocator(allocator.map(Allocator::as_ref)) };
        let rust = rust_reads();
        assert_eq!(rust, c_reads());
        // SAFETY: as above.
        unsafe { ffi::git_allocator_setup(&raw mut failing) };
        let mut copy = allocator.map(raw);
        let arg = copy.as_mut().map_or(std::ptr::null_mut(), |c| &raw mut *c);
        // SAFETY: as above; `arg` is NULL or a live value C copies.
        let rc = unsafe { ffi::git_allocator_setup(arg) };
        assert_eq!(rc, 0);
        (rust, c_reads())
    }

    /// `global_init` and `git_allocator_global_init` from the same global, and the
    /// global each leaves.
    fn both_global_inits(from: &Allocator) -> ([usize; 3], [usize; 3]) {
        let mut start = raw(from);
        // SAFETY: as in `both_setups`.
        unsafe { ffi::git_allocator_setup(&raw mut start) };
        // SAFETY: as above.
        unsafe { global_init() };
        let rust = rust_reads();
        // SAFETY: as above.
        unsafe { ffi::git_allocator_setup(&raw mut start) };
        // SAFETY: as above.
        let rc = unsafe { ffi::git_allocator_global_init() };
        assert_eq!(rc, 0);
        (rust, c_reads())
    }

    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    fn installs_through_rust_and_c_leave_the_same_global() {
        isolated(
            concat!(
                module_path!(),
                "::installs_through_rust_and_c_leave_the_same_global"
            ),
            || {
                // Before anything: the failing family, read alike.
                assert_eq!(rust_reads(), c_reads());
                assert_eq!(rust_reads(), addrs_of(&FAIL));
                // A value, and `None` (the standard family).
                let (rust, c) = both_setups(Some(&LIBC));
                assert_eq!((rust, c), (addrs_of(&LIBC), rust));
                let (rust, c) = both_setups(None);
                assert_eq!(rust, c);
                assert_ne!(rust, addrs_of(&FAIL));
                let standard = rust;
                // `global_init` over the failing family installs the standard one, over any
                // other keeps it.
                assert_eq!(both_global_inits(&FAIL), (standard, standard));
                let libc = addrs_of(&LIBC);
                assert_eq!(both_global_inits(&LIBC), (libc, libc));
            },
        );
    }
}
