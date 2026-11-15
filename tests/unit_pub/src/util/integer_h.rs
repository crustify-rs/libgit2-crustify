//! `src/util/integer.h`: the range checks and the checked arithmetic that port its
//! `git__is_*` functions and `git__*_overflow` macros, checked against the contracts their
//! C documents and the exact arithmetic of the compilers' intrinsics, which this build's
//! macros expand to.
//!
//! `tests/util/integer.c`'s three cases are carried over: their bodies compile empty on this
//! build, which defines `git__multiply_int64_overflow` as `__builtin_mul_overflow`, and test
//! the header's fallback, whose four edge cases this build's intrinsic answers otherwise, as
//! the module's docs record. The inputs `tests/util/errors.c`'s
//! `test_errors__integer_overflow_sets_oom` passes `git__add_sizet_overflow` are too; the
//! error it checks is `GIT_ADD_SIZET_OVERFLOW`'s (`git2_util.h`). None reaches C, so Miri
//! runs them all.

use core::ffi::{c_int, c_ulong};

use libgit2_crustify::libgit2::src::util::integer_h::{
    checked_add_c_int, checked_add_i64, checked_add_usize, checked_mul_i64, checked_mul_usize,
    checked_sub_c_int, fits_c_int, fits_c_ulong, fits_isize, fits_u16, fits_u32, fits_usize,
};

/// The `i64` values the range checks and the `i64` operations are probed at: each bound of
/// the types they convert to, its neighbours, and a few values between.
const I64_EDGES: [i64; 23] = [
    i64::MIN,
    i64::MIN + 1,
    -0x1_0000_0000,
    -0x8000_0001,
    -0x8000_0000,
    -0x7FFF_FFFF,
    -0x1_0000,
    -2,
    -1,
    0,
    1,
    2,
    0xFFFF,
    0x1_0000,
    0x7FFF_FFFE,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    0x1_0000_0000,
    0x4000_0000_0000_0000,
    i64::MAX - 1,
    i64::MAX,
    0x1234_5678_9ABC,
];

/// The `usize` values the range checks and the `usize` operations are probed at.
const USIZE_EDGES: [usize; 16] = [
    0,
    1,
    2,
    0xFFFF,
    0x1_0000,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    0x1_0000_0000,
    0x1_0000_0001,
    usize::MAX / 2,
    usize::MAX / 2 + 1,
    usize::MAX - 2,
    usize::MAX - 1,
    usize::MAX,
    0x1234_5678,
];

/// The `int` values the `c_int` operations are probed at.
const INT_EDGES: [c_int; 12] = [
    c_int::MIN,
    c_int::MIN + 1,
    -0x4000_0000,
    -2,
    -1,
    0,
    1,
    2,
    0x4000_0000,
    c_int::MAX - 1,
    c_int::MAX,
    0x1234,
];

/// Each range check accepts exactly the values in its type's range, at their bounds.
#[test]
fn range_checks_accept_exactly_the_range() {
    for p in I64_EDGES {
        let wide = i128::from(p);
        assert_eq!(
            fits_usize(p),
            p >= 0 && wide <= i128::try_from(usize::MAX).unwrap(),
            "{p}"
        );
        assert_eq!(
            fits_c_ulong(p),
            p >= 0 && wide <= i128::from(c_ulong::MAX),
            "{p}"
        );
        assert_eq!(
            fits_c_int(p),
            (i128::from(c_int::MIN)..=i128::from(c_int::MAX)).contains(&wide),
            "{p}"
        );
    }
    for p in USIZE_EDGES {
        let wide = u128::try_from(p).unwrap();
        assert_eq!(
            fits_isize(p),
            wide <= u128::try_from(isize::MAX).unwrap(),
            "{p}"
        );
        assert_eq!(fits_u16(p), wide <= u128::from(u16::MAX), "{p}");
        assert_eq!(fits_u32(p), wide <= u128::from(u32::MAX), "{p}");
    }
}

/// The values whose C round trip keeps them though they are out of range are refused:
/// `git__is_ssizet` of a count past `SSIZE_MAX`, and `git__is_sizet` and `git__is_ulong` of
/// a negative `int64_t`, which C accepts on this build. This is the correction the module's
/// docs record.
#[test]
fn out_of_range_values_c_keeps_are_refused() {
    assert!(!fits_isize(usize::MAX));
    assert!(!fits_isize(isize::MAX.unsigned_abs() + 1));
    assert!(fits_isize(isize::MAX.unsigned_abs()));
    for p in [-1_i64, -2, i64::MIN] {
        assert!(!fits_usize(p), "{p}");
        assert!(!fits_c_ulong(p), "{p}");
    }
}

/// A value of an unsigned type, which C's callers pass through the `int64_t` parameter, is
/// judged by its own value: every `u64` fits a 64-bit `usize` and `unsigned long`, as C's
/// round trip through a negative `int64_t` answers, and only those up to `INT_MAX` fit an
/// `int`.
#[test]
fn unsigned_values_are_judged_at_their_own_type() {
    for p in [0_u64, 1, 0x7FFF_FFFF, 0x8000_0000, 1 << 63, u64::MAX] {
        assert_eq!(fits_usize(p), usize::try_from(p).is_ok(), "{p}");
        assert_eq!(fits_c_ulong(p), c_ulong::try_from(p).is_ok(), "{p}");
        assert_eq!(fits_c_int(p), p <= 0x7FFF_FFFF, "{p}");
    }
    assert!(fits_usize(u64::MAX));
    assert!(fits_usize(usize::MAX));
    assert!(fits_u16(u16::MAX));
    assert!(fits_u32(u32::MAX));
    assert!(!fits_u16(-1_i32));
    assert!(!fits_u32(-1_i64));
}

/// Each `usize` operation gives the exact result where it fits and `None` where it does not,
/// as `__builtin_uaddl_overflow` and `__builtin_umull_overflow` compute it.
#[test]
fn usize_operations_match_the_exact_arithmetic() {
    let max = u128::try_from(usize::MAX).unwrap();
    for one in USIZE_EDGES {
        for two in USIZE_EDGES {
            let (a, b) = (u128::try_from(one).unwrap(), u128::try_from(two).unwrap());
            let sum = (a + b <= max).then(|| usize::try_from(a + b).unwrap());
            assert_eq!(checked_add_usize(one, two), sum, "{one} + {two}");
            let product = a
                .checked_mul(b)
                .filter(|&p| p <= max)
                .map(|p| usize::try_from(p).unwrap());
            assert_eq!(checked_mul_usize(one, two), product, "{one} * {two}");
        }
    }
}

/// Each `int` operation gives the exact result where it fits and `None` where it does not,
/// as `__builtin_sadd_overflow` and `__builtin_ssub_overflow` compute it.
#[test]
fn int_operations_match_the_exact_arithmetic() {
    for one in INT_EDGES {
        for two in INT_EDGES {
            let (a, b) = (i64::from(one), i64::from(two));
            assert_eq!(
                checked_add_c_int(one, two),
                c_int::try_from(a + b).ok(),
                "{one} + {two}"
            );
            assert_eq!(
                checked_sub_c_int(one, two),
                c_int::try_from(a - b).ok(),
                "{one} - {two}"
            );
        }
    }
}

/// Each `int64_t` operation gives the exact result where it fits and `None` where it does
/// not, as `__builtin_add_overflow` and `__builtin_mul_overflow` compute it.
#[test]
fn int64_operations_match_the_exact_arithmetic() {
    for one in I64_EDGES {
        for two in I64_EDGES {
            let (a, b) = (i128::from(one), i128::from(two));
            assert_eq!(
                checked_add_i64(one, two),
                i64::try_from(a + b).ok(),
                "{one} + {two}"
            );
            assert_eq!(
                checked_mul_i64(one, two),
                a.checked_mul(b).and_then(|p| i64::try_from(p).ok()),
                "{one} * {two}"
            );
        }
    }
}

/// `patch_parse.c`'s `parse_hunk_body`, the only site that expands `git__add_int_overflow`
/// and `git__sub_int_overflow`, computes `start + lines - remaining` for each side of a hunk,
/// from the header's non-negative `int` start and count, and refuses the hunk where either
/// step overflows; its `remaining` runs from the count down to zero. No raw binding reaches
/// it, so this checks the chain at its arguments' bounds against the exact arithmetic.
#[test]
fn hunk_line_numbers_match_the_exact_arithmetic() {
    let counts = [0, 1, 2, 0x4000_0000, c_int::MAX - 1, c_int::MAX];
    for start in counts {
        for lines in counts {
            for remaining in counts.into_iter().filter(|&r| r <= lines) {
                let exact = i64::from(start) + i64::from(lines) - i64::from(remaining);
                let sum = c_int::try_from(i64::from(start) + i64::from(lines)).ok();
                let line = checked_add_c_int(start, lines)
                    .and_then(|sum| checked_sub_c_int(sum, remaining));
                assert_eq!(
                    line,
                    sum.and_then(|_| c_int::try_from(exact).ok()),
                    "{start} + {lines} - {remaining}"
                );
            }
        }
    }
}

/// `test_errors__integer_overflow_sets_oom`'s inputs: `SIZE_MAX - 1 + 1` and `42 + 69` fit,
/// `SIZE_MAX + SIZE_MAX` overflows.
#[test]
fn errors_integer_overflow_sets_oom_inputs() {
    assert_eq!(checked_add_usize(usize::MAX - 1, 1), Some(usize::MAX));
    assert_eq!(checked_add_usize(42, 69), Some(111));
    assert_eq!(checked_add_usize(usize::MAX, usize::MAX), None);
}

/// `test_integer__multiply_int64_no_overflow`: every product it checks fits, with the value
/// it asserts.
#[test]
fn integer_multiply_int64_no_overflow() {
    let cases: [(i64, i64, Option<i64>); 81] = [
        (0x0, 0x0, Some(0x0)),
        (0x0, 0x1, Some(0x0)),
        (0x0, -0x1, Some(0x0)),
        (0x0, 0x2, Some(0x0)),
        (0x0, -0x2, Some(0x0)),
        (0x0, 0x7FF_FFFF_FFFF_FFFF, Some(0x0)),
        (0x0, -0x7FF_FFFF_FFFF_FFFF, Some(0x0)),
        (0x0, 0x800_0000_0000_0000, Some(0x0)),
        (0x0, -0x800_0000_0000_0000, Some(0x0)),
        (0x0, 0x7FFF_FFFF_FFFF_FFFF, Some(0x0)),
        (0x0, -0x7FFF_FFFF_FFFF_FFFF, Some(0x0)),
        (0x0, i64::MIN, Some(0x0)),
        (0x1, 0x0, Some(0x0)),
        (0x1, 0x1, Some(0x1)),
        (0x1, -0x1, Some(-0x1)),
        (0x1, 0x2, Some(0x2)),
        (0x1, -0x2, Some(-0x2)),
        (0x1, 0x7FF_FFFF_FFFF_FFFF, Some(0x7FF_FFFF_FFFF_FFFF)),
        (0x1, -0x7FF_FFFF_FFFF_FFFF, Some(-0x7FF_FFFF_FFFF_FFFF)),
        (0x1, 0x800_0000_0000_0000, Some(0x800_0000_0000_0000)),
        (0x1, -0x800_0000_0000_0000, Some(-0x800_0000_0000_0000)),
        (0x1, 0x7FFF_FFFF_FFFF_FFFF, Some(0x7FFF_FFFF_FFFF_FFFF)),
        (0x1, -0x7FFF_FFFF_FFFF_FFFF, Some(-0x7FFF_FFFF_FFFF_FFFF)),
        (-0x1, 0x0, Some(0x0)),
        (-0x1, 0x1, Some(-0x1)),
        (-0x1, -0x1, Some(0x1)),
        (-0x1, 0x2, Some(-0x2)),
        (-0x1, -0x2, Some(0x2)),
        (-0x1, 0x7FF_FFFF_FFFF_FFFF, Some(-0x7FF_FFFF_FFFF_FFFF)),
        (-0x1, -0x7FF_FFFF_FFFF_FFFF, Some(0x7FF_FFFF_FFFF_FFFF)),
        (-0x1, 0x800_0000_0000_0000, Some(-0x800_0000_0000_0000)),
        (-0x1, -0x800_0000_0000_0000, Some(0x800_0000_0000_0000)),
        (-0x1, 0x7FFF_FFFF_FFFF_FFFF, Some(-0x7FFF_FFFF_FFFF_FFFF)),
        (-0x1, -0x7FFF_FFFF_FFFF_FFFF, Some(0x7FFF_FFFF_FFFF_FFFF)),
        (0x2, 0x0, Some(0x0)),
        (0x2, 0x1, Some(0x2)),
        (0x2, -0x1, Some(-0x2)),
        (0x2, 0x2, Some(0x4)),
        (0x2, -0x2, Some(-0x4)),
        (0x2, 0x7FF_FFFF_FFFF_FFFF, Some(0xFFF_FFFF_FFFF_FFFE)),
        (0x2, -0x7FF_FFFF_FFFF_FFFF, Some(-0xFFF_FFFF_FFFF_FFFE)),
        (0x2, 0x800_0000_0000_0000, Some(0x1000_0000_0000_0000)),
        (0x2, -0x800_0000_0000_0000, Some(-0x1000_0000_0000_0000)),
        (-0x2, 0x0, Some(0x0)),
        (-0x2, 0x1, Some(-0x2)),
        (-0x2, -0x1, Some(0x2)),
        (-0x2, 0x2, Some(-0x4)),
        (-0x2, -0x2, Some(0x4)),
        (-0x2, 0x7FF_FFFF_FFFF_FFFF, Some(-0xFFF_FFFF_FFFF_FFFE)),
        (-0x2, -0x7FF_FFFF_FFFF_FFFF, Some(0xFFF_FFFF_FFFF_FFFE)),
        (-0x2, 0x800_0000_0000_0000, Some(-0x1000_0000_0000_0000)),
        (-0x2, -0x800_0000_0000_0000, Some(0x1000_0000_0000_0000)),
        (0x2, -0x4000_0000_0000_0000, Some(i64::MIN)),
        (0x7FF_FFFF_FFFF_FFFF, 0x0, Some(0x0)),
        (0x7FF_FFFF_FFFF_FFFF, 0x1, Some(0x7FF_FFFF_FFFF_FFFF)),
        (0x7FF_FFFF_FFFF_FFFF, -0x1, Some(-0x7FF_FFFF_FFFF_FFFF)),
        (0x7FF_FFFF_FFFF_FFFF, 0x2, Some(0xFFF_FFFF_FFFF_FFFE)),
        (0x7FF_FFFF_FFFF_FFFF, -0x2, Some(-0xFFF_FFFF_FFFF_FFFE)),
        (-0x7FF_FFFF_FFFF_FFFF, 0x0, Some(0x0)),
        (-0x7FF_FFFF_FFFF_FFFF, 0x1, Some(-0x7FF_FFFF_FFFF_FFFF)),
        (-0x7FF_FFFF_FFFF_FFFF, -0x1, Some(0x7FF_FFFF_FFFF_FFFF)),
        (-0x7FF_FFFF_FFFF_FFFF, 0x2, Some(-0xFFF_FFFF_FFFF_FFFE)),
        (-0x7FF_FFFF_FFFF_FFFF, -0x2, Some(0xFFF_FFFF_FFFF_FFFE)),
        (0x800_0000_0000_0000, 0x0, Some(0x0)),
        (0x800_0000_0000_0000, 0x1, Some(0x800_0000_0000_0000)),
        (0x800_0000_0000_0000, -0x1, Some(-0x800_0000_0000_0000)),
        (0x800_0000_0000_0000, 0x2, Some(0x1000_0000_0000_0000)),
        (0x800_0000_0000_0000, -0x2, Some(-0x1000_0000_0000_0000)),
        (-0x800_0000_0000_0000, 0x0, Some(0x0)),
        (-0x800_0000_0000_0000, 0x1, Some(-0x800_0000_0000_0000)),
        (-0x800_0000_0000_0000, -0x1, Some(0x800_0000_0000_0000)),
        (-0x800_0000_0000_0000, 0x2, Some(-0x1000_0000_0000_0000)),
        (-0x800_0000_0000_0000, -0x2, Some(0x1000_0000_0000_0000)),
        (0x7FFF_FFFF_FFFF_FFFF, 0x0, Some(0x0)),
        (0x7FFF_FFFF_FFFF_FFFF, 0x1, Some(0x7FFF_FFFF_FFFF_FFFF)),
        (0x7FFF_FFFF_FFFF_FFFF, -0x1, Some(-0x7FFF_FFFF_FFFF_FFFF)),
        (-0x4000_0000_0000_0000, 0x2, Some(i64::MIN)),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x0, Some(0x0)),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x1, Some(-0x7FFF_FFFF_FFFF_FFFF)),
        (-0x7FFF_FFFF_FFFF_FFFF, -0x1, Some(0x7FFF_FFFF_FFFF_FFFF)),
        (i64::MIN, 0x0, Some(0x0)),
    ];
    for (one, two, product) in cases {
        assert_eq!(checked_mul_i64(one, two), product, "{one} * {two}");
    }
}

/// `test_integer__multiply_int64_overflow`: every product it checks overflows.
#[test]
fn integer_multiply_int64_overflow() {
    let cases: [(i64, i64); 63] = [
        (0x2, 0x4000_0000_0000_0000),
        (0x2, 0x7FFF_FFFF_FFFF_FFFF),
        (0x2, -0x7FFF_FFFF_FFFF_FFFF),
        (0x2, i64::MIN),
        (-0x2, 0x7FFF_FFFF_FFFF_FFFF),
        (-0x2, -0x7FFF_FFFF_FFFF_FFFF),
        (-0x2, i64::MIN),
        (0x7FF_FFFF_FFFF_FFFF, 0x7FF_FFFF_FFFF_FFFF),
        (0x7FF_FFFF_FFFF_FFFF, -0x7FF_FFFF_FFFF_FFFF),
        (0x7FF_FFFF_FFFF_FFFF, 0x800_0000_0000_0000),
        (0x7FF_FFFF_FFFF_FFFF, -0x800_0000_0000_0000),
        (0x7FF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF),
        (0x7FF_FFFF_FFFF_FFFF, -0x7FFF_FFFF_FFFF_FFFF),
        (0x7FF_FFFF_FFFF_FFFF, i64::MIN),
        (-0x7FF_FFFF_FFFF_FFFF, 0x7FF_FFFF_FFFF_FFFF),
        (-0x7FF_FFFF_FFFF_FFFF, -0x7FF_FFFF_FFFF_FFFF),
        (-0x7FF_FFFF_FFFF_FFFF, 0x800_0000_0000_0000),
        (-0x7FF_FFFF_FFFF_FFFF, -0x800_0000_0000_0000),
        (-0x7FF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF),
        (-0x7FF_FFFF_FFFF_FFFF, -0x7FFF_FFFF_FFFF_FFFF),
        (-0x7FF_FFFF_FFFF_FFFF, i64::MIN),
        (0x800_0000_0000_0000, 0x7FF_FFFF_FFFF_FFFF),
        (0x800_0000_0000_0000, -0x7FF_FFFF_FFFF_FFFF),
        (0x800_0000_0000_0000, 0x800_0000_0000_0000),
        (0x800_0000_0000_0000, -0x800_0000_0000_0000),
        (0x800_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF),
        (0x800_0000_0000_0000, -0x7FFF_FFFF_FFFF_FFFF),
        (0x800_0000_0000_0000, i64::MIN),
        (-0x800_0000_0000_0000, 0x7FF_FFFF_FFFF_FFFF),
        (-0x800_0000_0000_0000, -0x7FF_FFFF_FFFF_FFFF),
        (-0x800_0000_0000_0000, 0x800_0000_0000_0000),
        (-0x800_0000_0000_0000, -0x800_0000_0000_0000),
        (-0x800_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF),
        (-0x800_0000_0000_0000, -0x7FFF_FFFF_FFFF_FFFF),
        (-0x800_0000_0000_0000, i64::MIN),
        (0x4000_0000_0000_0000, 0x2),
        (0x7FFF_FFFF_FFFF_FFFF, 0x2),
        (0x7FFF_FFFF_FFFF_FFFF, -0x2),
        (0x7FFF_FFFF_FFFF_FFFF, 0x7FF_FFFF_FFFF_FFFF),
        (0x7FFF_FFFF_FFFF_FFFF, -0x7FF_FFFF_FFFF_FFFF),
        (0x7FFF_FFFF_FFFF_FFFF, 0x800_0000_0000_0000),
        (0x7FFF_FFFF_FFFF_FFFF, -0x800_0000_0000_0000),
        (0x7FFF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF),
        (0x7FFF_FFFF_FFFF_FFFF, -0x7FFF_FFFF_FFFF_FFFF),
        (0x7FFF_FFFF_FFFF_FFFF, i64::MIN),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x2),
        (-0x7FFF_FFFF_FFFF_FFFF, -0x2),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x7FF_FFFF_FFFF_FFFF),
        (-0x7FFF_FFFF_FFFF_FFFF, -0x7FF_FFFF_FFFF_FFFF),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x800_0000_0000_0000),
        (-0x7FFF_FFFF_FFFF_FFFF, -0x800_0000_0000_0000),
        (-0x7FFF_FFFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF_FFFF),
        (-0x7FFF_FFFF_FFFF_FFFF, -0x7FFF_FFFF_FFFF_FFFF),
        (-0x7FFF_FFFF_FFFF_FFFF, i64::MIN),
        (i64::MIN, 0x2),
        (i64::MIN, -0x2),
        (i64::MIN, 0x7FF_FFFF_FFFF_FFFF),
        (i64::MIN, -0x7FF_FFFF_FFFF_FFFF),
        (i64::MIN, 0x800_0000_0000_0000),
        (i64::MIN, -0x800_0000_0000_0000),
        (i64::MIN, 0x7FFF_FFFF_FFFF_FFFF),
        (i64::MIN, -0x7FFF_FFFF_FFFF_FFFF),
        (i64::MIN, i64::MIN),
    ];
    for (one, two) in cases {
        assert_eq!(checked_mul_i64(one, two), None, "{one} * {two}");
    }
}

/// `test_integer__multiply_int64_edge_cases`, at this build's answers: the fallback the C
/// case tests reports `INT64_MIN * -1` and `-1 * INT64_MIN` as fitting and `INT64_MIN * 1`
/// and `1 * INT64_MIN` as overflowing; `__builtin_mul_overflow`, and the port, answer each the
/// other way, as the exact products have it.
#[test]
fn integer_multiply_int64_edge_cases() {
    assert_eq!(checked_mul_i64(i64::MIN, -1), None);
    assert_eq!(checked_mul_i64(-1, i64::MIN), None);
    assert_eq!(checked_mul_i64(1, i64::MIN), Some(i64::MIN));
    assert_eq!(checked_mul_i64(i64::MIN, 1), Some(i64::MIN));
}

/// The six operations evaluate in a constant.
#[test]
fn operations_are_const() {
    const RESULTS: [bool; 6] = [
        checked_add_usize(usize::MAX, 1).is_none(),
        matches!(checked_mul_usize(2, 3), Some(6)),
        checked_add_c_int(c_int::MAX, 1).is_none(),
        checked_sub_c_int(c_int::MIN, 1).is_none(),
        matches!(checked_add_i64(i64::MAX, 0), Some(i64::MAX)),
        checked_mul_i64(i64::MIN, -1).is_none(),
    ];
    assert_eq!(RESULTS, [true; 6]);
}
