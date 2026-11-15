//! Equivalence of `src/util/ctype_compat.h`'s eight macros with their ports.
//!
//! On this build each macro's replacement list is `<ctype.h>`'s function over its argument
//! cast to `unsigned char`, as `!!isalpha((unsigned char)(a))`. The raw bindings reach no
//! macro, so each is compared twice:
//!
//! - with its replacement list, glibc's function called on the byte the cast gives, for every `int`
//!   from below `CHAR_MIN` to past `UCHAR_MAX`: `EOF`, the negative `char`s a signed `char` holds
//!   and the `int`s the cast reduces. Neither the raw bindings nor `glibc-bindings` bind
//!   `<ctype.h>`, so its eight functions are declared here, as glibc exports them; and
//! - where the C expands it, through the functions whose result a single byte's class decides:
//!   `git__isspace` through `git_blob_data_is_binary` (`git_str_is_binary`, for the control bytes
//!   it does not count as printable) and `git_config_parse_int64` (`git__strntol64`'s leading white
//!   space), and `git__isalnum` through `git_message_trailers` (a trailer key's characters).
//!
//! The other sites are not reached through a function the bindings give a single byte to:
//! `git__isalpha`, `git__isdigit` and `git__toupper` expand in `date.c`'s parser, which
//! libgit2 runs on a commit's date from the environment, `git__isprint` and `git__isxdigit`
//! in `smart_pkt.c`'s `parse_len` and `str.c`'s `git_str_decode_percent`, which no bound
//! function calls, and `git__tolower` in case-insensitive comparisons of whole names. Their
//! comparison with the replacement list covers every argument those sites can pass.
//!
//! The test process never calls `setlocale`, so `<ctype.h>` runs in the C locale, as in
//! libgit2.

use std::ffi::{CString, c_int};

use libgit2_crustify::libgit2::src::libgit2::blob::data_is_binary;
use libgit2_crustify::libgit2::src::libgit2::config::parse_i64;
use libgit2_crustify::libgit2::src::libgit2::trailer::MessageTrailerArray;
use libgit2_crustify::libgit2::src::util::ctype_compat_h::{
    is_alnum, is_alpha, is_digit, is_print, is_space, is_xdigit, to_lower, to_upper,
};

unsafe extern "C" {
    fn isalnum(c: c_int) -> c_int;
    fn isalpha(c: c_int) -> c_int;
    fn isdigit(c: c_int) -> c_int;
    fn isprint(c: c_int) -> c_int;
    fn isspace(c: c_int) -> c_int;
    fn isxdigit(c: c_int) -> c_int;
    fn tolower(c: c_int) -> c_int;
    fn toupper(c: c_int) -> c_int;
}

/// `(unsigned char)(a)`: the low byte of `a`'s two's complement.
fn as_unsigned_char(a: c_int) -> u8 {
    a.to_le_bytes()[0]
}

/// `f((unsigned char)(a))`, as the macro's replacement list calls it.
fn c_call(f: unsafe extern "C" fn(c_int) -> c_int, a: c_int) -> c_int {
    // SAFETY: `<ctype.h>`'s functions are defined for every value of `unsigned char` and for
    // `EOF`, read only the calling thread's locale tables, and keep nothing; the cast makes the
    // argument an `unsigned char`'s value.
    unsafe { f(c_int::from(as_unsigned_char(a))) }
}

/// Every `int` the comparison passes: below `CHAR_MIN`, through every `char` and byte value,
/// and past `UCHAR_MAX`.
fn arguments() -> impl Iterator<Item = c_int> {
    -300..=600
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_predicates_match_the_replacement_lists() {
    type Pair = (
        &'static str,
        fn(u8) -> bool,
        unsafe extern "C" fn(c_int) -> c_int,
    );
    let pairs: [Pair; 6] = [
        ("git__isalnum", is_alnum, isalnum),
        ("git__isalpha", is_alpha, isalpha),
        ("git__isdigit", is_digit, isdigit),
        ("git__isprint", is_print, isprint),
        ("git__isspace", is_space, isspace),
        ("git__isxdigit", is_xdigit, isxdigit),
    ];
    for (name, rust, c) in pairs {
        for a in arguments() {
            // `!!f(...)`: the C normalises the class to 0 or 1.
            let c_class = c_call(c, a) != 0;
            assert_eq!(rust(as_unsigned_char(a)), c_class, "{name}({a})");
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_case_maps_match_the_replacement_lists() {
    type Pair = (
        &'static str,
        fn(u8) -> u8,
        unsafe extern "C" fn(c_int) -> c_int,
    );
    let pairs: [Pair; 2] = [
        ("git__tolower", to_lower, tolower),
        ("git__toupper", to_upper, toupper),
    ];
    for (name, rust, c) in pairs {
        for a in arguments() {
            let mapped = rust(as_unsigned_char(a));
            assert_eq!(c_int::from(mapped), c_call(c, a), "{name}({a})");
        }
    }
}

/// `git_str_is_binary` counts a byte as printable above `0x1F` but for DEL, and for `\b`,
/// ESC and the form feed; of the others, a NUL makes the data binary, and any byte
/// `git__isspace` rejects counts against it. A single byte of those is binary exactly when it
/// is not white space.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_space_matches_git_str_is_binary() {
    for c in u8::MIN..=u8::MAX {
        let printable = (c > 0x1F && c != 0x7F) || matches!(c, 0x08 | 0x1B | 0x0C);
        let expected = c == 0 || (!printable && !is_space(c));
        assert_eq!(data_is_binary(&[c]), expected, "{c:#04x}");
    }
}

/// `git__strntol64` skips the white space `git__isspace` finds before the number:
/// `<c><c>7` reads as 7 exactly when `c` is white space, or the octal prefix `0`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_space_matches_git_config_parse_int64() {
    for c in 1..=u8::MAX {
        let value = CString::new([c, c, b'7']).expect("no NUL");
        let seven = parse_i64(Some(&value)).ok() == Some(7);
        assert_eq!(seven, is_space(c) || c == b'0', "{c:#04x}");
    }
}

/// A trailer key is read while `git__isalnum` or `-` accepts its characters: `K<c>y: v` is
/// the trailer `K<c>y` exactly when `c` is a letter, a digit or `-`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn is_alnum_matches_git_message_trailers() {
    for c in 1..=u8::MAX {
        let mut message = b"Subject\n\nK".to_vec();
        message.extend_from_slice(&[c, b'y', b':', b' ', b'v', b'\n']);
        let message = CString::new(message).expect("no NUL");
        let array = MessageTrailerArray::parse(&message).expect("git_message_trailers");
        let trailers: Vec<_> = array
            .as_ref()
            .trailers()
            .iter()
            .map(|t| (t.key().to_bytes().to_vec(), t.value().to_bytes().to_vec()))
            .collect();
        let is_key = trailers == [(vec![b'K', c, b'y'], b"v".to_vec())];
        assert_eq!(is_key, is_alnum(c) || c == b'-', "{c:#04x}");
    }
}
