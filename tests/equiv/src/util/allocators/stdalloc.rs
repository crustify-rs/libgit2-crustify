//! Equivalence of `stdalloc::allocator` with `git_stdalloc_init_allocator` on a raw
//! `git_allocator`.
//!
//! - Values: the value Rust returns holds the members C writes into a raw `git_allocator`, whatever
//!   the raw value held before, and the two change alike, step by step.
//! - Behaviour: libgit2's out-of-memory allocator (`tests/util/str/oom.c`), the standard one with
//!   `malloc` and `realloc` replaced by functions that refuse more than 100 bytes and forward the
//!   rest, is built once over the Rust value and once over the raw value C fills, and installed in
//!   a child process of its own each (libgit2's allocator is process-global), before libgit2 is
//!   initialised. Both run one workload through the allocator strategies, and its outcomes and
//!   error states are compared. Each side then restores libgit2's default allocator, as
//!   `test_str_oom__cleanup` does (`git_allocator_setup(NULL)`): `set_allocator(None)` and
//!   `GIT_OPT_SET_ALLOCATOR` with NULL, after which the refusing functions, still armed, no longer
//!   serve, and the blocks they made are released by the standard family they came from.

use std::ffi::{CString, c_char, c_int, c_void};
use std::process::Command;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{
    AllocVec, Allocator, ArrayFree, FreeFn, MallocFn, ReallocFn, StrFree,
};
use libgit2::libgit2::src::util::allocators::stdalloc;
use libgit2::libgit2::src::util::errors;

// --- values -------------------------------------------------------------------------

unsafe extern "C" fn other_malloc(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn other_realloc(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn other_free(_: *mut c_void) {}

/// The members' addresses, `0` for an unset one.
fn raw_members(raw: &ffi::git_allocator) -> [usize; 3] {
    [
        raw.gmalloc.map_or(0, |f| f as usize),
        raw.grealloc.map_or(0, |f| f as usize),
        raw.gfree.map_or(0, |f| f as usize),
    ]
}

fn members(a: Allocator) -> [usize; 3] {
    let r = a.as_ref();
    [
        r.malloc().map_or(0, |f| f as usize),
        r.realloc().map_or(0, |f| f as usize),
        r.free().map_or(0, |f| f as usize),
    ]
}

/// `git_stdalloc_init_allocator` over `raw`, which must return 0.
fn c_init(raw: &mut ffi::git_allocator) {
    // SAFETY: `raw` is a live, writable `git_allocator` for the call, which C only writes.
    let rc = unsafe { ffi::git_stdalloc_init_allocator(raw) };
    assert_eq!(rc, 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn values_match_c_init_step_by_step() {
    // Every member C writes, over an unset raw value and over one holding other functions.
    let (malloc, realloc, free): (MallocFn, ReallocFn, FreeFn) =
        (other_malloc, other_realloc, other_free);
    let mut unset = ffi::git_allocator::default();
    c_init(&mut unset);
    let mut raw = ffi::git_allocator {
        gmalloc: Some(malloc),
        grealloc: Some(realloc),
        gfree: Some(free),
    };
    c_init(&mut raw);
    let mut rust = stdalloc::allocator();
    assert_eq!(members(rust), raw_members(&unset));
    assert_eq!(members(rust), raw_members(&raw));
    assert!(members(rust).iter().all(|&m| m != 0));

    // One member replaced on each side.
    raw.gmalloc = Some(malloc);
    rust.as_mut().set_malloc(malloc);
    assert_eq!(members(rust), raw_members(&raw));
    raw.gfree = Some(free);
    rust.as_mut().set_free(free);
    assert_eq!(members(rust), raw_members(&raw));

    // Initialised again: C overwrites every member, and Rust returns a fresh value.
    c_init(&mut raw);
    let rust = stdalloc::allocator();
    assert_eq!(members(rust), raw_members(&raw));
    assert_eq!(members(rust), raw_members(&unset));
}

// --- behaviour ------------------------------------------------------------------------

const CHILD: &str = "CRUSTIFY_EQUIV_STDALLOC_SIDE";

/// The standard family's `malloc` and `realloc` the refusing ones forward to, as the side
/// under test built them.
static STANDARD: OnceLock<(MallocFn, ReallocFn)> = OnceLock::new();

/// Whether the refusing functions refuse: off while libgit2 initialises, so that its
/// own start-up allocations are not refused.
static ARMED: AtomicBool = AtomicBool::new(false);

const LIMIT: usize = 100;

/// `tests/util/str/oom.c`'s `oom_malloc`: refuses more than [`LIMIT`] bytes once armed.
unsafe extern "C" fn oom_malloc(n: usize, file: *const c_char, line: c_int) -> *mut c_void {
    if ARMED.load(SeqCst) && n > LIMIT {
        return ptr::null_mut();
    }
    let (malloc, _) = STANDARD.get().unwrap();
    // SAFETY: forwarded under the caller's own `MallocFn` contract.
    unsafe { malloc(n, file, line) }
}

/// `tests/util/str/oom.c`'s `oom_realloc`: refuses more than [`LIMIT`] bytes once armed.
unsafe extern "C" fn oom_realloc(
    p: *mut c_void,
    n: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    if ARMED.load(SeqCst) && n > LIMIT {
        return ptr::null_mut();
    }
    let (_, realloc) = STANDARD.get().unwrap();
    // SAFETY: forwarded under the caller's own `ReallocFn` contract: `p` is NULL or a
    // block of the standard family, which every block this allocator makes is.
    unsafe { realloc(p, n, file, line) }
}

/// Installs the out-of-memory allocator over the standard one `side` builds, before
/// libgit2 is initialised.
fn install(side: &str) {
    match side {
        "rust" => {
            let mut oom = stdalloc::allocator();
            let r = oom.as_ref();
            STANDARD
                .set((r.malloc().unwrap(), r.realloc().unwrap()))
                .unwrap();
            let mut m = oom.as_mut();
            m.set_malloc(oom_malloc);
            m.set_realloc(oom_realloc);
            // SAFETY: the family forwards to the standard one, which meets `Alloc`'s
            // contract, and so do the refusals; nothing has initialised libgit2 in this
            // process, which runs this test alone, on its only thread calling libgit2.
            unsafe { set_allocator(Some(oom.as_ref())) };
        }
        "c" => {
            let mut raw = ffi::git_allocator::default();
            c_init(&mut raw);
            STANDARD
                .set((raw.gmalloc.unwrap(), raw.grealloc.unwrap()))
                .unwrap();
            raw.gmalloc = Some(oom_malloc);
            raw.grealloc = Some(oom_realloc);
            let opt = c_int::try_from(ffi::GIT_OPT_SET_ALLOCATOR).unwrap();
            // SAFETY: as above; `GIT_OPT_SET_ALLOCATOR` copies the live `raw` it is passed.
            let rc = unsafe { ffi::git_libgit2_opts(opt, &raw const raw) };
            assert_eq!(rc, 0);
        }
        _ => unreachable!("unknown side {side}"),
    }
}

/// Restores libgit2's default allocator on `side`, as `test_str_oom__cleanup` does.
fn restore_default(side: &str) {
    match side {
        // SAFETY: every block libgit2 holds was made by the refusing functions, which forward
        // to the standard family the default allocator is (this build has no
        // `DEBUG_STRICT_ALLOC`), so it resizes and releases them; this process runs this test
        // alone, on its only thread calling libgit2.
        "rust" => unsafe { set_allocator(None) },
        "c" => {
            let opt = c_int::try_from(ffi::GIT_OPT_SET_ALLOCATOR).unwrap();
            // SAFETY: as above; `GIT_OPT_SET_ALLOCATOR` reads NULL as the default allocator.
            let rc = unsafe { ffi::git_libgit2_opts(opt, ptr::null::<ffi::git_allocator>()) };
            assert_eq!(rc, 0);
        }
        _ => unreachable!("unknown side {side}"),
    }
}

/// Whether the last error is an out-of-memory one.
fn last_is_oom() -> bool {
    errors::last().class() == Some(errors::ErrorClass::NoMemory)
}

/// The workload, through the allocator strategies, which reach the installed allocator:
/// each step's outcome, and whether a refusal left an out-of-memory error.
fn workload(side: &str) -> Vec<String> {
    let mut steps = Vec::new();
    // Initialises libgit2, through the installed allocator, before it refuses anything.
    let warm = AllocVec::<u8>::from_slice(&[1; 8]);
    steps.push(format!("warm {:?}", warm.as_deref()));
    ARMED.store(true, SeqCst);
    let mut held = Vec::new();
    for len in [LIMIT, LIMIT + 1] {
        // `git__malloc`, the family's `malloc`.
        errors::clear();
        let run = AllocVec::<u8>::from_slice(&vec![7; len]);
        steps.push(format!(
            "malloc {len}: {} oom={}",
            run.as_deref().map(<[u8]>::len).unwrap_or_default(),
            run.is_none() && last_is_oom()
        ));
        // `git__mallocarray`, `git__realloc(NULL, ..)`: the family's `realloc`.
        errors::clear();
        let array = ArrayFree::new_uninit::<u8>(len);
        steps.push(format!(
            "realloc {len}: {} oom={}",
            array.is_some(),
            array.is_none() && last_is_oom()
        ));
        // `git__strdup` copies the text and its terminator.
        errors::clear();
        let text = CString::new(vec![b'a'; len - 1]).unwrap();
        let copy = StrFree::dup(&text);
        steps.push(format!(
            "strdup {len}: {:?} oom={}",
            copy.as_ref().map(|s| s.to_bytes().len()),
            copy.is_none() && last_is_oom()
        ));
        held.push((run, array, copy));
    }
    // Still armed: the default allocator serves what the refusing ones refused, and releases
    // the blocks they made, held until here.
    restore_default(side);
    let len = LIMIT + 1;
    errors::clear();
    let run = AllocVec::<u8>::from_slice(&vec![7; len]);
    let array = ArrayFree::new_uninit::<u8>(len);
    let copy = StrFree::dup(&CString::new(vec![b'a'; len - 1]).unwrap());
    steps.push(format!(
        "restored {len}: malloc {:?} realloc {} strdup {:?} oom={}",
        run.as_deref().map(<[u8]>::len),
        array.is_some(),
        copy.as_ref().map(|s| s.to_bytes().len()),
        last_is_oom()
    ));
    drop(held);
    ARMED.store(false, SeqCst);
    let after = AllocVec::<u8>::from_slice(&[2; LIMIT * 2]);
    steps.push(format!("disarmed {:?}", after.as_deref().map(<[u8]>::len)));
    steps
}

/// Runs the workload in a child process on `side`'s allocator, and returns its steps.
fn run_side(side: &str) -> Vec<String> {
    let name =
        "src::util::allocators::stdalloc::oom_allocator_over_the_standard_one_as_c_builds_it";
    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, side)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "side `{side}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        // The harness prints the test's name on the line of its first output.
        .filter_map(|l| l.split_once("STEP ").map(|(_, step)| step.to_owned()))
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process")]
fn oom_allocator_over_the_standard_one_as_c_builds_it() {
    if let Ok(side) = std::env::var(CHILD) {
        install(&side);
        for step in workload(&side) {
            println!("STEP {step}");
        }
        return;
    }
    let (rust, c) = (run_side("rust"), run_side("c"));
    assert_eq!(rust, c);
    assert_eq!(
        rust,
        [
            "warm Some([1, 1, 1, 1, 1, 1, 1, 1])",
            "malloc 100: 100 oom=false",
            "realloc 100: true oom=false",
            "strdup 100: Some(99) oom=false",
            "malloc 101: 0 oom=true",
            "realloc 101: false oom=true",
            "strdup 101: None oom=true",
            "restored 101: malloc Some(101) realloc true strdup Some(100) oom=false",
            "disarmed Some(200)",
        ]
    );
}
