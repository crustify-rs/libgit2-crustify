//! Equivalence of `src/util/utf8.c`'s exported functions with their ports: `git_utf8_iterate`
//! with `decode_char`, `git_utf8_char_length` with `char_count` and
//! `git_utf8_valid_buf_length` with `valid_prefix_len`, each C call reading its own copy of
//! the bytes the Rust reads.
//!
//! `decode_char`'s result is compared through the mapping its safe API applies to C's: a code
//! point and its length are what `git_utf8_iterate` writes through `out` and returns, and a
//! refusal is its -1 with 0 written. Empty bytes are not compared: `git_utf8_iterate` reads
//! past a size of 0, where `decode_char` refuses them, a unit test's case.
//!
//! The static `utf8_charlen` and `utf8proc_utf8class` are compared through the three: every
//! lead byte, followed by every kind of byte after it, reaches each class of the table and
//! each of `utf8_charlen`'s outcomes.

use std::ffi::c_int;

use libgit2_crustify::libgit2::src::util::utf8::{char_count, decode_char, valid_prefix_len};
use libgit2_crustify::sys::libgit2 as sys;

/// `git_utf8_iterate` of a copy of `bytes`, which are not empty: what it writes through `out`
/// and what it returns.
fn c_iterate(bytes: &[u8]) -> (u32, c_int) {
    assert!(!bytes.is_empty(), "git_utf8_iterate reads past a size of 0");
    let copy = bytes.to_vec();
    let mut out = u32::MAX;
    // SAFETY: `copy` holds `copy.len()` bytes, which the call reads only within that size
    // when it is not 0, and `out` is a live `u32` it writes.
    let len = unsafe { sys::git_utf8_iterate(&raw mut out, copy.as_ptr().cast(), copy.len()) };
    (out, len)
}

/// `decode_char` of `bytes`, as `git_utf8_iterate` reports it.
fn rust_iterate(bytes: &[u8]) -> (u32, c_int) {
    decode_char(bytes).map_or((0, -1), |(c, len)| {
        (u32::from(c), c_int::try_from(len).expect("at most 4"))
    })
}

/// `git_utf8_char_length` of a copy of `bytes`.
fn c_char_length(bytes: &[u8]) -> usize {
    let copy = bytes.to_vec();
    // SAFETY: `copy` holds `copy.len()` bytes, which the call reads only within that size.
    unsafe { sys::git_utf8_char_length(copy.as_ptr().cast(), copy.len()) }
}

/// `git_utf8_valid_buf_length` of a copy of `bytes`.
fn c_valid_buf_length(bytes: &[u8]) -> usize {
    let copy = bytes.to_vec();
    // SAFETY: `copy` holds `copy.len()` bytes, which the call reads only within that size.
    unsafe { sys::git_utf8_valid_buf_length(copy.as_ptr().cast(), copy.len()) }
}

/// The three functions on `bytes`, C's and Rust's alike.
fn assert_same(bytes: &[u8]) {
    if !bytes.is_empty() {
        assert_eq!(
            rust_iterate(bytes),
            c_iterate(bytes),
            "iterate {bytes:02X?}"
        );
    }
    assert_eq!(
        char_count(bytes),
        c_char_length(bytes),
        "char_length {bytes:02X?}"
    );
    assert_eq!(
        valid_prefix_len(bytes),
        c_valid_buf_length(bytes),
        "valid_buf_length {bytes:02X?}"
    );
}

/// The cases of `test_utf8__char_length` and of the unit tests: each length's boundaries,
/// broken, cut short and overlong sequences, surrogates, noncharacters and code points past
/// U+10FFFF.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn edge_cases_match_the_c() {
    let cases: [&[u8]; 38] = [
        b"",
        b"$",
        b"abcde",
        b"\xc2\xa2",
        b"\x24\xc2\xa2",
        b"\xf0\x90\x8d\x88",
        b"\x24\xc2",
        b"\x24\xc2\xc2\xa2",
        b"\x24\xc0\xc0\x34",
        b"\x24\xf5\xfd\xc2",
        b"\0",
        b"\x7F",
        b"\xC2\x80",
        b"\xDF\xBF",
        b"\xE0\xA0\x80",
        b"\xEF\xBF\xBD",
        b"\xEF\xBF\xBE",
        b"\xEF\xBF\xBF",
        b"\xEF\xB7\x90",
        b"\xEF\xB7\xAF",
        b"\xEF\xB7\xB0",
        b"\xF0\x90\x80\x80",
        b"\xF4\x8F\xBF\xBD",
        b"\xF4\x8F\xBF\xBF",
        b"\xF4\x90\x80\x80",
        b"\xF7\xBF\xBF\xBF",
        b"\xF8\x80\x80\x80",
        b"\xFF",
        b"\x80",
        b"\xC0\x80",
        b"\xC1\xBF",
        b"\xE0\x9F\xBF",
        b"\xF0\x8F\xBF\xBF",
        b"\xED\xA0\x80",
        b"\xED\xBF\xBF",
        b"\xE2\x82",
        b"\xE2\x41\xAC",
        b"ab\xE2\x82\xACcd\xFF",
    ];
    for bytes in cases {
        assert_same(bytes);
    }
}

/// Every lead byte, alone, followed by each kind of byte (ASCII, the first and last
/// continuation byte, a lead) in every count up to three, so that each class of the table is
/// reached cut short, broken and whole.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_lead_byte_matches_the_c() {
    let kinds = [0x41, 0x80, 0xBF, 0xC2];
    for lead in 0..=u8::MAX {
        assert_same(&[lead]);
        for &a in &kinds {
            assert_same(&[lead, a]);
            for &b in &kinds {
                assert_same(&[lead, a, b]);
                for &c in &kinds {
                    assert_same(&[lead, a, b, c]);
                }
            }
        }
    }
}

/// Every pair of bytes, which reaches every two-byte sequence, overlong ones included.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_byte_pair_matches_the_c() {
    for a in 0..=u8::MAX {
        for b in 0..=u8::MAX {
            assert_same(&[a, b]);
        }
    }
}

/// Every three-byte sequence, a lead of the class followed by every pair of continuation
/// bytes, which reaches the overlong ones, the surrogates and the noncharacters of the basic
/// plane.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_three_byte_sequence_matches_the_c() {
    for a in 0xE0..=0xEF {
        for b in 0x80..=0xBF {
            for c in 0x80..=0xBF {
                assert_same(&[a, b, c]);
            }
        }
    }
}

/// Every `char`'s encoding, and the four-byte forms past U+10FFFF, by the lead byte and the
/// second byte that decide them.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_char_matches_the_c() {
    let mut buf = [0; 4];
    for c in (0..=0x10_FFFF).filter_map(char::from_u32) {
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        assert_eq!(rust_iterate(bytes), c_iterate(bytes), "{c:?}");
    }
    for a in 0xF0..=0xF7 {
        for b in 0x80..=0xBF {
            assert_same(&[a, b, 0xBF, 0xBF]);
            assert_same(&[a, b, 0x80, 0x80]);
        }
    }
}

/// A buffer walked one code point at a time, as `next_hfs_char` walks a path, and each of its
/// prefixes counted and sanitized, as `git_xdiff_cb` sanitizes a hunk header: compared after
/// each step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn walks_match_the_c() {
    let mut text = "a\u{200C}.g\u{200D}it/é€𐍈\u{FEFF}x".as_bytes().to_vec();
    text.extend_from_slice(b"\xE2\x82\xC0\x80\xED\xA0\x80\xFFz");
    let mut rest: &[u8] = &text;
    while !rest.is_empty() {
        let (ours, theirs) = (rust_iterate(rest), c_iterate(rest));
        assert_eq!(ours, theirs, "at {rest:02X?}");
        rest = &rest[usize::try_from(ours.1).unwrap_or(1)..];
    }
    for end in 0..=text.len() {
        assert_same(&text[..end]);
        assert_same(&text[end..]);
    }
}
