//! Equivalence of `src/util/integer.h`'s overflow checks with their ports.
//!
//! The header's macros stay for the C that still expands them, and are compared where they
//! expand, through C the raw bindings reach: `git__multiply_sizet_overflow` through
//! `GIT_MULTIPLY_SIZET_OVERFLOW` in `git__calloc`, `git__mallocarray` and `git__reallocarray`
//! (`src/util/alloc.c`), `git__add_sizet_overflow` through `GIT_ERROR_CHECK_ALLOC_ADD` in
//! `git_strlist_copy_with_null` (`src/util/strlist.c`), and `git__multiply_int64_overflow` and
//! `git__add_int64_overflow` through `git_config_parse_int64` (`src/libgit2/config.c`), whose
//! `git__strntol64` (`src/util/util.c`) accumulates each digit with both and whose unit
//! suffixes multiply by 1024 with the first. `git__add_int_overflow` and `git__sub_int_overflow`
//! expand only in `patch_parse.c`'s `parse_hunk_body`, which `git_diff_from_buffer`
//! (`diff_parse.c`) reaches through `git_patch_parse`: they are compared through which hunks it
//! refuses, with `git__is_int` at `parse_int`, which reads each of the hunk header's numbers
//! and calls the module's export.
//!
//! The range checks' equivalence tests were retired at the module's switch, when the header's
//! `static inline` definitions became prototypes of the module's exports: their raw bindings
//! reach the Rust since, and the unit tests (`tests/unit_pub/src/util/integer_h.rs`) keep what
//! the comparisons established.

use core::ffi::{CStr, c_int};
use std::ffi::CString;

use libgit2_crustify::libgit2::src::libgit2::diff::Diff;
use libgit2_crustify::libgit2::src::util::errors::{self, ErrorClass};
use libgit2_crustify::libgit2::src::util::integer_h::{
    checked_add_c_int, checked_add_i64, checked_add_usize, checked_mul_i64, checked_mul_usize,
    checked_sub_c_int, fits_c_int,
};
use libgit2_crustify::sys::libgit2 as sys;

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    let _ = core::mem::ManuallyDrop::new(
        libgit2_crustify::libgit2::src::libgit2::libgit2::init().unwrap(),
    );
}

/// The largest product the allocation probes ask libgit2's allocator for.
const SMALL: usize = 1 << 16;

/// The `(nelem, elsize)` pairs whose product overflows or is at most [`SMALL`] bytes: every
/// other one is a request the allocator may refuse for its size alone.
fn array_sizes() -> Vec<(usize, usize)> {
    let edges = [
        0,
        1,
        2,
        3,
        0x100,
        SMALL,
        0xFFFF_FFFF,
        0x1_0000_0000,
        0x1_0000_0001,
        usize::MAX / 2,
        usize::MAX / 2 + 1,
        usize::MAX / 3 + 1,
        usize::MAX - 1,
        usize::MAX,
    ];
    let mut pairs = Vec::new();
    for nelem in edges {
        for elsize in edges {
            if checked_mul_usize(nelem, elsize).is_none_or(|bytes| bytes <= SMALL) {
                pairs.push((nelem, elsize));
            }
        }
    }
    pairs
}

/// Whether the C refused a request as out of memory: a NULL result, with
/// `GIT_ERROR_NOMEMORY` left as the thread's last error. A block it returns is released.
fn refused(block: *mut core::ffi::c_void) -> bool {
    if block.is_null() {
        assert_eq!(errors::last().class(), Some(ErrorClass::NoMemory));
        return true;
    }
    // SAFETY: `block` is a live allocation of libgit2's allocator, given up here.
    unsafe { sys::git__free(block) };
    false
}

/// `git__calloc`, `git__mallocarray` and `git__reallocarray(NULL, …)` refuse exactly the
/// products `checked_mul_usize` reports as overflowing.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn array_allocations_refuse_the_overflowing_products() {
    init();
    for (nelem, elsize) in array_sizes() {
        let overflows = checked_mul_usize(nelem, elsize).is_none();
        errors::clear();
        // SAFETY: any sizes are accepted; an overflowing product is refused before any
        // allocation, and any other is at most `SMALL` bytes.
        let block = unsafe { sys::git__calloc(nelem, elsize) };
        assert_eq!(refused(block), overflows, "git__calloc({nelem}, {elsize})");
        errors::clear();
        // SAFETY: as above.
        let block = unsafe { sys::git__mallocarray(nelem, elsize) };
        assert_eq!(
            refused(block),
            overflows,
            "git__mallocarray({nelem}, {elsize})"
        );
        errors::clear();
        // SAFETY: as above; a NULL block resizes no live allocation.
        let block = unsafe { sys::git__reallocarray(core::ptr::null_mut(), nelem, elsize) };
        assert_eq!(
            refused(block),
            overflows,
            "git__reallocarray(NULL, {nelem}, {elsize})"
        );
    }
}

/// `git_strlist_copy_with_null(&out, in, len)` sizes its array as `len + 1` pointers, and
/// refuses a `len` whose sum overflows, before it reads `in`, as `checked_add_usize` reports
/// it; any other `len` it copies.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn strlist_copy_refuses_the_overflowing_length() {
    init();
    let strings = [c"a".as_ptr(), c"bc".as_ptr(), c"".as_ptr()];
    for len in [usize::MAX, 0, 1, 2, 3] {
        let overflows = checked_add_usize(len, 1).is_none();
        let mut out = core::ptr::null_mut();
        errors::clear();
        // SAFETY: `out` is a live out-pointer; `in` holds `len` live strings where
        // `len <= 3`, and the C refuses `usize::MAX` before it reads `in`, as the comparison
        // checks.
        let rc = unsafe {
            sys::git_strlist_copy_with_null(&raw mut out, strings.as_ptr().cast_mut(), len)
        };
        if overflows {
            assert_eq!(rc, -1, "len {len}");
            assert!(out.is_null());
            assert_eq!(errors::last().class(), Some(ErrorClass::NoMemory));
        } else {
            assert_eq!(rc, 0, "len {len}");
            // SAFETY: `out` is the NULL-ended array the call made, given up here.
            unsafe { sys::git_strlist_free_with_null(out) };
        }
    }
}

/// `git_config_parse_int64(value)`: the integer the C parses, or `None` where it fails.
fn c_parse_int64(value: &CStr) -> Option<i64> {
    let mut out = 0_i64;
    // SAFETY: `value` is a live NUL-terminated string and `out` a live `int64_t`, both
    // borrowed for the call.
    let rc = unsafe { sys::git_config_parse_int64(&raw mut out, value.as_ptr()) };
    (rc == 0).then_some(out)
}

/// `git_config_parse_int64`'s arithmetic over a string of a sign, digits and a unit suffix,
/// as the C does it, each step through a port: `git__strntol64` takes the base from the
/// digits' prefix, and accumulates each digit as `n * base + digit`, the digit negated
/// under a minus sign, failing where either step overflows; `k`, `m` and `g` then multiply
/// by 1024 once, twice or three times, failing where a product overflows.
fn rust_parse_int64(value: &str) -> Option<i64> {
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let (base, digits) = match unsigned.strip_prefix("0x") {
        Some(hex) if !hex.is_empty() => (16, hex),
        _ if unsigned.starts_with('0') => (8, unsigned),
        _ => (10, unsigned),
    };
    let end = digits
        .find(|c: char| !c.is_digit(base))
        .unwrap_or(digits.len());
    let (digits, suffix) = digits.split_at(end);
    if digits.is_empty() {
        return None;
    }
    let mut n = Some(0_i64);
    for digit in digits.chars() {
        let digit = i64::from(digit.to_digit(base).unwrap());
        let digit = if negative { -digit } else { digit };
        n = n
            .and_then(|n| checked_mul_i64(n, i64::from(base)))
            .and_then(|n| checked_add_i64(n, digit));
    }
    let scale = match suffix {
        "" => 0,
        "k" | "K" => 1,
        "m" | "M" => 2,
        "g" | "G" => 3,
        _ => return None,
    };
    (0..scale).try_fold(n?, |n, _| checked_mul_i64(n, 1024))
}

/// The magnitudes probed: each bound a step of the parse can overflow at, with its
/// neighbours, for no suffix and for each of the three.
fn magnitudes() -> Vec<i128> {
    let mut magnitudes = vec![0, 1, 7, 10, 1024, 10_i128.pow(19), 10_i128.pow(20), 1 << 64];
    for scale in 0..=3 {
        let bound = (1_i128 << 63) >> (10 * scale);
        for delta in -2..=2 {
            magnitudes.push(bound + delta);
        }
    }
    magnitudes
}

/// `git_config_parse_int64` accepts exactly the values whose digits and suffix
/// `checked_mul_i64` and `checked_add_i64` carry without overflow, with the same result, in
/// each base and for each suffix.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn config_parse_int64_overflows_as_the_ports_do() {
    init();
    for magnitude in magnitudes() {
        for sign in ["", "-", "+"] {
            for digits in [
                format!("{magnitude}"),
                format!("0x{magnitude:x}"),
                format!("0{magnitude:o}"),
            ] {
                for suffix in ["", "k", "M", "g"] {
                    let value = format!("{sign}{digits}{suffix}");
                    let c = c_parse_int64(&CString::new(value.as_str()).unwrap());
                    assert_eq!(
                        rust_parse_int64(&value),
                        c,
                        "git_config_parse_int64({value:?})"
                    );
                }
            }
        }
    }
}

/// The parse's model holds the overflows each probe meets: the bounds of `int64_t`, and of
/// each suffix's product, on both sides.
#[test]
fn the_parse_model_overflows_at_each_bound() {
    assert_eq!(rust_parse_int64("9223372036854775807"), Some(i64::MAX));
    assert_eq!(rust_parse_int64("9223372036854775808"), None);
    assert_eq!(rust_parse_int64("-9223372036854775808"), Some(i64::MIN));
    assert_eq!(rust_parse_int64("-9223372036854775809"), None);
    assert_eq!(rust_parse_int64("9007199254740991k"), Some(i64::MAX - 1023));
    assert_eq!(rust_parse_int64("9007199254740992k"), None);
    assert_eq!(rust_parse_int64("-9007199254740992k"), Some(i64::MIN));
    assert_eq!(rust_parse_int64("8796093022208M"), None);
    assert_eq!(rust_parse_int64("-8589934592g"), Some(i64::MIN));
    assert_eq!(rust_parse_int64("8589934592g"), None);
    assert_eq!(rust_parse_int64("0x7fffffffffffffff"), Some(i64::MAX));
    assert_eq!(rust_parse_int64("0777"), Some(0o777));
    assert_eq!(rust_parse_int64("12q"), None);
}

/// One hunk's header and body: its old and new start and counts, and its body's context,
/// deleted and added lines, in that order, as many as the counts ask.
#[derive(Clone, Copy, Debug)]
struct Hunk {
    old_start: i64,
    new_start: i64,
    context: i64,
    deleted: i64,
    added: i64,
}

impl Hunk {
    /// A patch of one file holding this hunk alone.
    fn patch(self) -> Vec<u8> {
        let mut patch = format!(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -{},{} +{},{} @@\n",
            self.old_start,
            self.context + self.deleted,
            self.new_start,
            self.context + self.added,
        )
        .into_bytes();
        for (count, line) in [
            (self.context, &b" c\n"[..]),
            (self.deleted, b"-d\n"),
            (self.added, b"+a\n"),
        ] {
            for _ in 0..count {
                patch.extend_from_slice(line);
            }
        }
        patch
    }

    /// Whether `parse_hunk_header` and `parse_hunk_body` accept the hunk, each step through a
    /// port: `parse_int` refuses a number past `int`'s range, and each line's old and new
    /// numbers are `start + count - remaining`, the hunk refused where either step
    /// overflows; `remaining` runs down from the count as the lines consume it.
    fn accepted(self) -> bool {
        let numbers = [
            self.old_start,
            self.context + self.deleted,
            self.new_start,
            self.context + self.added,
        ];
        if !numbers.into_iter().all(fits_c_int) {
            return false;
        }
        let [old_start, old_lines, new_start, new_lines] =
            numbers.map(|n| c_int::try_from(n).unwrap());
        let (mut old_left, mut new_left) = (old_lines, new_lines);
        let lines = [
            (1, 1, self.context),
            (1, 0, self.deleted),
            (0, 1, self.added),
        ];
        for (old_step, new_step, count) in lines {
            for _ in 0..count {
                let old = checked_add_c_int(old_start, old_lines)
                    .and_then(|sum| checked_sub_c_int(sum, old_left));
                let new = checked_add_c_int(new_start, new_lines)
                    .and_then(|sum| checked_sub_c_int(sum, new_left));
                if old.is_none() || new.is_none() {
                    return false;
                }
                old_left -= old_step;
                new_left -= new_step;
            }
        }
        true
    }
}

/// `git_diff_from_buffer` accepts exactly the hunks whose header numbers `fits_c_int`
/// accepts and whose line numbers `checked_add_c_int` and `checked_sub_c_int` carry, on
/// either side, at `int`'s bound; a hunk it refuses for them fails as a patch error.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn diff_from_buffer_refuses_the_hunks_the_ports_overflow() {
    init();
    let max = i64::from(c_int::MAX);
    let mut probes = 0;
    for (context, deleted, added) in [(1, 0, 0), (2, 0, 0), (0, 1, 1), (1, 1, 0), (0, 0, 2)] {
        let lines = context + deleted.max(added);
        for start in [
            1,
            max - lines - 1,
            max - lines,
            max - lines + 1,
            max - 1,
            max,
            max + 1,
        ] {
            for (old_start, new_start) in [(start, 1), (1, start), (start, start)] {
                let hunk = Hunk {
                    old_start,
                    new_start,
                    context,
                    deleted,
                    added,
                };
                let parsed = Diff::from_buffer(&hunk.patch());
                assert_eq!(parsed.is_ok(), hunk.accepted(), "{hunk:?}");
                if let Err(error) = parsed {
                    assert_eq!(error.class(), Some(ErrorClass::Patch), "{hunk:?}");
                }
                probes += 1;
            }
        }
    }
    assert_eq!(probes, 105);
}
