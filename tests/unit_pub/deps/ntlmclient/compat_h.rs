//! `deps/ntlmclient/compat.h`: `min`, `MIN`'s port, checked against the macro's body,
//! `(((x) < (y)) ? (x) : (y))`, and against the standard library's `min` where the type is
//! `Ord`. The module has no C tests to carry over; these cover `ntlm.c`'s uses of the macro,
//! its ties and the pairs `<` cannot order. None reaches C, so Miri runs them all.

use core::cmp::Ordering;

use libgit2_crustify::libgit2::deps::ntlmclient::compat_h::min;

/// `ntlm.c`'s three uses: `des_key_from_password`'s `MIN(plaintext_len, 7)` and
/// `generate_lm_hash`'s `MIN(7, password_len)` and `MIN(14, password_len) - 7`, which split a
/// password into two 7-byte DES keys, for every password length up to 20.
#[test]
fn ntlm_key_lengths() {
    for password_len in 0_usize..=20 {
        let keystr1_len = min(7, password_len);
        let keystr2_len = if password_len > 7 {
            min(14, password_len) - 7
        } else {
            0
        };
        assert_eq!(keystr1_len, password_len.clamp(0, 7), "{password_len}");
        assert_eq!(keystr2_len, password_len.clamp(7, 14) - 7, "{password_len}");
        assert_eq!(min(password_len, 7), keystr1_len, "{password_len}");
    }
}

/// Integers of every width and signedness, at their bounds, agree with `Ord::min`.
#[test]
fn integers_agree_with_ord_min() {
    let sizes = [0, 1, 6, 7, 8, 14, usize::MAX - 1, usize::MAX];
    for &x in &sizes {
        for &y in &sizes {
            assert_eq!(min(x, y), x.min(y), "{x} {y}");
            assert_eq!(min(x, y), min(y, x), "{x} {y}");
        }
    }
    let signed = [i64::MIN, -7, -1, 0, 1, 7, i64::MAX];
    for &x in &signed {
        for &y in &signed {
            assert_eq!(min(x, y), x.min(y), "{x} {y}");
        }
    }
    assert_eq!(min(u8::MAX, 0), 0);
    assert_eq!(min(i8::MIN, i8::MAX), i8::MIN);
    assert_eq!(min('a', 'Z'), 'Z');
}

/// A value whose ordering ignores its tag, to tell which operand a call returns.
#[derive(Debug, Clone, Copy)]
struct Tagged {
    key: i32,
    tag: char,
}

impl PartialEq for Tagged {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl PartialOrd for Tagged {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.key.partial_cmp(&other.key)
    }
}

/// The C returns `x` only when `x < y`: a tie returns `y`, as the ternary's false branch.
#[test]
fn a_tie_returns_the_second() {
    let x = Tagged { key: 3, tag: 'x' };
    let y = Tagged { key: 3, tag: 'y' };
    assert_eq!(min(x, y).tag, 'y');
    assert_eq!(min(y, x).tag, 'x');
    assert_eq!(min(Tagged { key: 2, tag: 'x' }, y).tag, 'x');
    assert_eq!(min(Tagged { key: 4, tag: 'x' }, y).tag, 'y');
}

/// Floats follow the C's `<`: a NaN on either side, and `-0.0` against `0.0`, return `y`.
#[test]
fn unordered_pairs_return_the_second() {
    assert_eq!(min(f64::NAN, 1.0).to_bits(), 1.0_f64.to_bits());
    assert!(min(1.0, f64::NAN).is_nan());
    assert!(min(f64::NAN, f64::NAN).is_nan());
    assert_eq!(min(-0.0_f64, 0.0).to_bits(), 0.0_f64.to_bits());
    assert_eq!(min(0.0_f64, -0.0).to_bits(), (-0.0_f64).to_bits());
    assert_eq!(
        min(f64::NEG_INFINITY, f64::INFINITY).to_bits(),
        f64::NEG_INFINITY.to_bits()
    );
    assert_eq!(min(1.5_f32, -2.5).to_bits(), (-2.5_f32).to_bits());
}

/// Each operand is evaluated once, before the call, whichever is returned: the macro
/// evaluated the one it returned twice.
#[test]
fn each_operand_is_evaluated_once() {
    let mut calls = 0;
    let mut next = |value: usize| {
        calls += 1;
        value
    };
    assert_eq!(min(next(3), next(7)), 3);
    assert_eq!(min(next(7), next(3)), 3);
    assert_eq!(calls, 4);
}

/// Operands are moved, not copied: owned values that are not `Copy` are compared and the
/// lesser returned.
#[test]
fn owned_operands_are_moved() {
    assert_eq!(
        min(String::from("refs/heads"), String::from("refs/tags")),
        "refs/heads"
    );
    assert_eq!(min(vec![2, 1], vec![1, 9, 9]), vec![1, 9, 9]);
    assert_eq!(min(Some(3), None), None);
}
