//! `src/util/ctype_compat.h`: the eight character classes and case maps, checked for every
//! byte against the C locale's classes as the C standard defines them (C17 7.4.1), which the
//! header's `GIT_WIN32` branch writes out as ASCII ranges. The module has no C tests to carry
//! over. None reaches C, so Miri runs them all.

use libgit2_crustify::libgit2::src::util::ctype_compat_h::{
    is_alnum, is_alpha, is_digit, is_print, is_space, is_xdigit, to_lower, to_upper,
};

/// Every byte.
fn bytes() -> impl Iterator<Item = u8> {
    u8::MIN..=u8::MAX
}

/// The bytes in `set`, for every byte: the class `set` defines.
fn class_of(set: &[u8]) -> impl Fn(u8) -> bool + '_ {
    move |c| set.contains(&c)
}

const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS: &[u8] = b"0123456789";

/// The letters are the 52 of the Latin alphabet, in both cases, and nothing else.
#[test]
fn is_alpha_is_the_latin_letters() {
    let letters = [UPPER, LOWER].concat();
    let expected = class_of(&letters);
    for c in bytes() {
        assert_eq!(is_alpha(c), expected(c), "{c:#04x}");
    }
    assert_eq!(bytes().filter(|&c| is_alpha(c)).count(), 52);
}

/// The digits are `0` to `9`.
#[test]
fn is_digit_is_the_ten_decimal_digits() {
    let expected = class_of(DIGITS);
    for c in bytes() {
        assert_eq!(is_digit(c), expected(c), "{c:#04x}");
    }
}

/// `isalnum` is `isalpha` or `isdigit`.
#[test]
fn is_alnum_is_a_letter_or_a_digit() {
    for c in bytes() {
        assert_eq!(is_alnum(c), is_alpha(c) || is_digit(c), "{c:#04x}");
    }
    assert_eq!(bytes().filter(|&c| is_alnum(c)).count(), 62);
}

/// The hexadecimal digits are the decimal ones and `a` to `f` in both cases.
#[test]
fn is_xdigit_is_the_hexadecimal_digits() {
    let expected = class_of(b"0123456789abcdefABCDEF");
    for c in bytes() {
        assert_eq!(is_xdigit(c), expected(c), "{c:#04x}");
    }
}

/// White space is the standard's six: the space, `\t`, `\n`, `\v`, `\f` and `\r`. The vertical
/// tab is the one `u8::is_ascii_whitespace` leaves out.
#[test]
fn is_space_is_the_six_standard_white_space_characters() {
    let expected = class_of(b" \t\n\x0b\x0c\r");
    for c in bytes() {
        assert_eq!(is_space(c), expected(c), "{c:#04x}");
    }
    assert!(is_space(0x0B) && !0x0B_u8.is_ascii_whitespace());
}

/// The printable characters are `0x20` to `0x7E`: the graphic ones and the space, not the
/// controls, DEL or any byte past ASCII.
#[test]
fn is_print_is_the_space_and_the_graphic_characters() {
    for c in bytes() {
        assert_eq!(is_print(c), (0x20..=0x7E).contains(&c), "{c:#04x}");
        assert_eq!(is_print(c), c == b' ' || c.is_ascii_graphic(), "{c:#04x}");
    }
    assert!(!is_print(b'\t') && !is_print(0x7F));
}

/// No byte past ASCII is in any class, whatever it would be in a single-byte locale: `0xE9`
/// is `é` in ISO-8859-1, `0xA0` its no-break space and `0x85` the next line in C1.
#[test]
fn no_byte_past_ascii_is_in_a_class() {
    for c in 0x80..=u8::MAX {
        let classes = [is_alnum, is_alpha, is_digit, is_print, is_space, is_xdigit];
        assert!(classes.iter().all(|class| !class(c)), "{c:#04x}");
    }
}

/// The case maps move each Latin letter to its other case and leave every other byte as it
/// is, bytes past ASCII included.
#[test]
fn the_case_maps_move_only_the_latin_letters() {
    for (&upper, &lower) in UPPER.iter().zip(LOWER) {
        assert_eq!(to_lower(upper), lower);
        assert_eq!(to_lower(lower), lower);
        assert_eq!(to_upper(lower), upper);
        assert_eq!(to_upper(upper), upper);
    }
    for c in bytes().filter(|&c| !is_alpha(c)) {
        assert_eq!(to_lower(c), c, "{c:#04x}");
        assert_eq!(to_upper(c), c, "{c:#04x}");
    }
    assert_eq!((to_lower(0xC9), to_upper(0xE9)), (0xC9, 0xE9));
}

/// Each map is idempotent, and either one undoes the other on a letter of its result's case.
#[test]
fn the_case_maps_compose() {
    for c in bytes() {
        assert_eq!(to_lower(to_lower(c)), to_lower(c));
        assert_eq!(to_upper(to_upper(c)), to_upper(c));
        assert_eq!(to_lower(to_upper(c)), to_lower(c));
        assert_eq!(to_upper(to_lower(c)), to_upper(c));
        assert_eq!(is_alpha(to_lower(c)), is_alpha(c));
    }
}

/// A C caller's argument is an `int` the macro casts to `unsigned char`; for a `char` or
/// `EOF`, the caller's cast to `u8` is that cast, so `EOF` and a negative `char` give their
/// byte's result, as on every build but `GIT_WIN32`. A value wider than a byte, such as a code
/// point, is no byte of these classes, as the module's documentation describes.
#[test]
fn a_c_callers_int_is_its_byte() {
    let eof: i32 = -1;
    let e_acute = -23_i8; // `é`'s ISO-8859-1 byte in a signed `char`
    assert_eq!(eof.to_le_bytes()[0], 0xFF);
    assert!(!is_space(eof.to_le_bytes()[0]) && !is_print(eof.to_le_bytes()[0]));
    assert_eq!(to_lower(eof.to_le_bytes()[0]), 0xFF);
    assert_eq!(e_acute.cast_unsigned(), 0xE9);
    assert!(!is_alpha(e_acute.cast_unsigned()));
}

/// They are `const fn`s: a class is computed where a constant is needed, as at run time.
#[test]
fn they_evaluate_in_constants() {
    const CLASSES: [bool; 6] = [
        is_alnum(b'q'),
        is_alpha(b'1'),
        is_digit(b'1'),
        is_print(b' '),
        is_space(0x0B),
        is_xdigit(b'G'),
    ];
    const CASES: [u8; 2] = [to_lower(b'Q'), to_upper(b'q')];
    assert_eq!(CLASSES, [true, false, true, true, true, false]);
    assert_eq!(CASES, *b"qQ");
}
