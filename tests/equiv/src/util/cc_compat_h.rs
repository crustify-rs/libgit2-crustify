//! Equivalence of `src/util/cc-compat.h`'s `PRIdZ`, `PRIuZ`, `PRIxZ` and `va_copy` with their
//! ports, `PRI_DZ`, `PRI_UZ`, `PRI_XZ` and `va_copy`.
//!
//! The formats are compared with the header's expansion on this build, which the raw
//! bindings hold as constants, and through C's printf: a format built from each constant
//! formats a value, in `git_error_set`, as Rust's formatting does.
//!
//! The raw bindings do not reach `va_copy`, a macro, so it is compared through the C that
//! expands it: `git_error_set` formats its arguments through `git_error_vset` into the
//! thread's message with `git_str_vprintf` (`src/util/str.c`), which formats a fresh
//! `va_copy` of its list on each pass and grows the buffer between passes, until the message
//! fits. A fresh thread starts with an empty message buffer, so a long message takes more
//! than one pass, each from its own copy. Each run sets the same message on both sides, the
//! C formatting its own arguments and the Rust a copy of its list, compared after each step.
//!
//! `GIT_UNUSED` is compared nowhere: its expansion only evaluates its argument, and computes
//! nothing that a function holding it could show.

use std::ffi::{CStr, CString, c_char, c_int};

use libgit2_crustify::libgit2::src::util::cc_compat_h::{PRI_DZ, PRI_UZ, PRI_XZ, va_copy};
use libgit2_crustify::libgit2::src::util::errors::last;
use libgit2_crustify::sys::libgit2 as raw;

/// A class `git_error_vset` appends nothing to, as it appends `strerror` to `GIT_ERROR_OS`.
const CLASS: c_int = raw::GIT_ERROR_INVALID.cast_signed();

fn init() {
    // SAFETY: no arguments; refcounted, never released in this process. `git_error_set`
    // needs the thread-state key it creates.
    assert!(unsafe { raw::git_libgit2_init() } > 0);
}

/// The message `git_error_set` stored on this thread.
fn message() -> Vec<u8> {
    last().message().to_bytes().to_vec()
}

/// The header's expansion of a format macro on this build, without its NUL.
fn expansion(constant: &'static [u8]) -> &'static str {
    CStr::from_bytes_with_nul(constant)
        .expect("one NUL, at the end")
        .to_str()
        .expect("ASCII")
}

/// Each constant is the header's expansion on this build, as bindgen evaluates it.
#[test]
fn the_formats_are_the_headers() {
    assert_eq!(PRI_DZ, expansion(raw::PRIdZ));
    assert_eq!(PRI_UZ, expansion(raw::PRIuZ));
    assert_eq!(PRI_XZ, expansion(raw::PRIxZ));
}

/// `%` and a format, as C writes `"%" PRIuZ`.
fn spec(format: &str) -> CString {
    CString::new(format!("%{format}")).expect("no NUL in the format")
}

/// A `size_t` formatted by C's printf with `PRIuZ` and `PRIxZ`, and an `ssize_t` with `PRIdZ`,
/// gives what Rust's `{}` and `{:x}` give: the extremes, the powers of two around the widths
/// of narrower types, and the values between.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn the_formats_format_as_rust_does() {
    init();
    let unsigned = [
        0,
        1,
        9,
        10,
        0xFF,
        0x100,
        0xFFFF_FFFF,
        0x1_0000_0000,
        usize::MAX / 2,
        usize::MAX,
    ];
    let (decimal, hex) = (spec(PRI_UZ), spec(PRI_XZ));
    for value in unsigned {
        // SAFETY: the format is NUL-terminated and converts one `size_t`, which `usize` is.
        unsafe { raw::git_error_set(CLASS, decimal.as_ptr(), value) };
        assert_eq!(message(), value.to_string().into_bytes(), "{value}");
        // SAFETY: as above.
        unsafe { raw::git_error_set(CLASS, hex.as_ptr(), value) };
        assert_eq!(message(), format!("{value:x}").into_bytes(), "{value:#x}");
    }
    let signed = [
        isize::MIN,
        isize::MIN + 1,
        -0x1_0000_0000,
        -1,
        0,
        1,
        i32::MAX as isize,
        isize::MAX,
    ];
    let decimal = spec(PRI_DZ);
    for value in signed {
        // SAFETY: the format is NUL-terminated and converts one `ssize_t`, which `isize` is.
        unsafe { raw::git_error_set(CLASS, decimal.as_ptr(), value) };
        assert_eq!(message(), value.to_string().into_bytes(), "{value}");
    }
}

/// The three formats in one string, beside a string argument, as `tree-cache.c` and
/// `odb.c` write them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn the_formats_mix_with_other_conversions() {
    init();
    let format = CString::new(format!("%s: %{PRI_UZ} of %{PRI_XZ} (%{PRI_DZ})")).unwrap();
    let (name, count, mask, delta) = (c"tree", 42_usize, 0xBEEF_usize, -7_isize);
    // SAFETY: the format converts a string, two `size_t` and an `ssize_t`, given in order.
    unsafe { raw::git_error_set(CLASS, format.as_ptr(), name.as_ptr(), count, mask, delta) };
    assert_eq!(message(), b"tree: 42 of beef (-7)");
}

/// One step: a message of `len` bytes, then a `size_t`.
fn text(len: usize, seed: u8) -> CString {
    CString::new(
        (0..len)
            .map(|i| b'a' + i.to_le_bytes()[0].wrapping_add(seed) % 26)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// What `git_error_set(CLASS, "%s-%zu-%s", head, n, tail)` stores, the C side.
fn c_step(head: &CStr, n: usize, tail: &CStr) -> Vec<u8> {
    let format: &CStr = c"%s-%zu-%s";
    // SAFETY: the format converts a string, a `size_t` and a string, given in order.
    unsafe {
        raw::git_error_set(
            CLASS,
            format.as_ptr(),
            head.as_ptr().cast::<c_char>(),
            n,
            tail.as_ptr().cast::<c_char>(),
        );
    }
    message()
}

/// The same message, formatted by Rust from a copy of its list, the original formatted
/// again after it to show the copy left it whole.
fn rust_step(head: &CStr, n: usize, tail: &CStr) -> Vec<u8> {
    let (head, tail) = (head.to_str().unwrap(), tail.to_str().unwrap());
    let args = format_args!("{head}-{n}-{tail}");
    let copied = va_copy(&args).to_string();
    assert_eq!(args.to_string(), copied);
    copied.into_bytes()
}

/// Message lengths that start below `git_str_vprintf`'s first estimate, twice the format's
/// length, and grow past it, then past the buffer each earlier step left, so that a step
/// takes a second pass from a fresh copy, then a shorter one that fits at once.
const LENGTHS: [usize; 9] = [0, 1, 7, 64, 300, 4096, 10, 70_000, 3];

/// Each step's message is the same on both sides, each run on a fresh thread whose message
/// buffer starts empty.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn va_copy_formats_as_git_str_vprintf() {
    init();
    let steps: Vec<_> = LENGTHS
        .iter()
        .enumerate()
        .map(|(i, &len)| {
            (
                text(len, u8::try_from(i).unwrap()),
                len.wrapping_mul(31),
                text(len / 3, 7),
            )
        })
        .collect();
    let run = |side: fn(&CStr, usize, &CStr) -> Vec<u8>| {
        let steps = steps.clone();
        std::thread::spawn(move || {
            steps
                .iter()
                .map(|(h, n, t)| side(h, *n, t))
                .collect::<Vec<_>>()
        })
        .join()
        .unwrap()
    };
    let (c, rust) = (run(c_step), run(rust_step));
    for (i, (c, rust)) in c.iter().zip(&rust).enumerate() {
        assert_eq!(c.len(), rust.len(), "step {i}");
        assert!(c == rust, "step {i}: the messages differ");
    }
    assert_eq!(c.len(), LENGTHS.len());
}
