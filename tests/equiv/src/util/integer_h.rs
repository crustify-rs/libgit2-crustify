//! Equivalence of `src/util/integer.h`'s range checks and overflow checks with their ports.
//!
//! - The range checks, `static inline`, are reached through their raw bindings, the wrappers
//!   `static_fns.c` compiles over the header: each is compared with its port over its parameter's
//!   bounds. Where the C's round trip keeps a value its range does not hold (`git__is_ssizet` past
//!   `SSIZE_MAX`, `git__is_sizet` and `git__is_ulong` of a negative value, on this 64-bit build),
//!   the C's answer is asserted beside the port's refusal: the correction the module's docs record.
//!   A `uint64_t` a C caller passes through the `int64_t` parameter is compared at its own type.
//! - The macros are compared where they expand, through C the raw bindings reach:
//!   `git__multiply_sizet_overflow` through `GIT_MULTIPLY_SIZET_OVERFLOW` in `git__calloc`,
//!   `git__mallocarray` and `git__reallocarray` (`src/util/alloc.c`), `git__add_sizet_overflow`
//!   through `GIT_ERROR_CHECK_ALLOC_ADD` in `git_strlist_copy_with_null` (`src/util/strlist.c`),
//!   and `git__multiply_int64_overflow` and `git__add_int64_overflow` through
//!   `git_config_parse_int64` (`src/libgit2/config.c`), whose `git__strntol64` (`src/util/util.c`)
//!   accumulates each digit with both and whose unit suffixes multiply by 1024 with the first.
//!   `git__add_int_overflow` and `git__sub_int_overflow` expand only in `patch_parse.c`'s
//!   `parse_hunk_body`, which no raw binding reaches: the unit tests check them at its arguments.

use core::ffi::{CStr, c_int, c_ulong};
use std::ffi::CString;

use libgit2_crustify::libgit2::src::util::errors::{self, ErrorClass};
use libgit2_crustify::libgit2::src::util::integer_h::{
    checked_add_i64, checked_add_usize, checked_mul_i64, checked_mul_usize, fits_c_int,
    fits_c_ulong, fits_isize, fits_u16, fits_u32, fits_usize,
};
use libgit2_crustify::sys::libgit2 as sys;

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    let _ = core::mem::ManuallyDrop::new(
        libgit2_crustify::libgit2::src::libgit2::libgit2::init().unwrap(),
    );
}

/// The `int64_t` values probed: each bound of the types the checks convert to, with its
/// neighbours, and values between.
fn i64_probes() -> Vec<i64> {
    let mut probes = vec![0, 0x1234_5678_9ABC, -0x1234_5678_9ABC];
    for bound in [
        i64::MIN,
        i64::MAX,
        i64::from(c_int::MIN),
        i64::from(c_int::MAX),
        i64::from(u16::MAX),
        i64::from(u32::MAX),
        0,
    ] {
        for delta in -2..=2 {
            probes.extend(bound.checked_add(delta));
        }
    }
    probes
}

/// The `size_t` values probed, as [`i64_probes`].
fn usize_probes() -> Vec<usize> {
    let mut probes = vec![0x1234_5678];
    for bound in [
        0,
        usize::from(u16::MAX),
        usize::try_from(u32::MAX).unwrap(),
        isize::MAX.unsigned_abs(),
        usize::MAX,
    ] {
        for delta in -2_isize..=2 {
            probes.extend(bound.checked_add_signed(delta));
        }
    }
    probes
}

/// `git__is_int`, `git__is_uint16` and `git__is_uint32` narrow, so their round trips keep
/// their ranges: their ports agree with them on every probe.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn narrowing_checks_match_the_c() {
    for p in i64_probes() {
        // SAFETY: a pure function of its argument.
        let c = unsafe { sys::git__is_int(p) } != 0;
        assert_eq!(fits_c_int(p), c, "git__is_int({p})");
    }
    for p in usize_probes() {
        // SAFETY: a pure function of its argument.
        let c = unsafe { sys::git__is_uint16(p) } != 0;
        assert_eq!(fits_u16(p), c, "git__is_uint16({p})");
        // SAFETY: a pure function of its argument.
        let c = unsafe { sys::git__is_uint32(p) } != 0;
        assert_eq!(fits_u32(p), c, "git__is_uint32({p})");
    }
}

/// `git__is_sizet` and `git__is_ulong` agree with their ports on every non-negative
/// `int64_t`, and accept every negative one, which the ports refuse; for each `uint64_t` a C
/// caller passes through the parameter, the ports at its own type give the C's answer.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn widening_checks_match_the_c_but_for_negative_values() {
    for p in i64_probes() {
        // SAFETY: a pure function of its argument.
        let sizet = unsafe { sys::git__is_sizet(p) } != 0;
        // SAFETY: a pure function of its argument.
        let ulong = unsafe { sys::git__is_ulong(p) } != 0;
        if p >= 0 {
            assert_eq!(fits_usize(p), sizet, "git__is_sizet({p})");
            assert_eq!(fits_c_ulong(p), ulong, "git__is_ulong({p})");
        } else {
            assert!(sizet && !fits_usize(p), "git__is_sizet({p})");
            assert!(ulong && !fits_c_ulong(p), "git__is_ulong({p})");
        }
    }
    for p in usize_probes()
        .into_iter()
        .map(|p| u64::try_from(p).unwrap())
    {
        // SAFETY: a pure function of its argument.
        let sizet = unsafe { sys::git__is_sizet(p.cast_signed()) } != 0;
        // SAFETY: a pure function of its argument.
        let ulong = unsafe { sys::git__is_ulong(p.cast_signed()) } != 0;
        assert_eq!(fits_usize(p), sizet, "git__is_sizet({p}_u64)");
        assert_eq!(fits_c_ulong(p), ulong, "git__is_ulong({p}_u64)");
        assert_eq!(c_ulong::try_from(p).is_ok(), ulong);
    }
}

/// `git__is_ssizet` accepts every `size_t`; its port agrees up to `SSIZE_MAX` and refuses
/// the rest.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn ssizet_check_matches_the_c_up_to_ssize_max() {
    for p in usize_probes() {
        // SAFETY: a pure function of its argument.
        let c = unsafe { sys::git__is_ssizet(p) } != 0;
        assert!(c, "git__is_ssizet({p})");
        assert_eq!(
            fits_isize(p),
            p <= isize::MAX.unsigned_abs(),
            "git__is_ssizet({p})"
        );
    }
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
