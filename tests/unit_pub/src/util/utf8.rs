//! `src/util/utf8.c`: `decode_char`, `char_count` and `valid_prefix_len`, the ports of
//! `git_utf8_iterate`, `git_utf8_char_length` and `git_utf8_valid_buf_length`, checked
//! against their contracts.
//!
//! `char_count_carries_over_the_c_test` carries over the module's one C test
//! case, `test_utf8__char_length` (`tests/util/utf8.c`), which left the C with
//! `git_utf8_char_length`'s export; the others cover the boundaries of each sequence length
//! and what `decode_char` refuses. None reaches C, so Miri runs them all.

use libgit2_crustify::libgit2::src::util::utf8::{
    InvalidCodePoint, char_count, decode_char, valid_prefix_len,
};

/// Carries over `test_utf8__char_length` (`tests/util/utf8.c`): its ten inputs and counts,
/// unchanged, each C string and size as the bytes of one byte string.
#[test]
fn char_count_carries_over_the_c_test() {
    assert_eq!(char_count(b""), 0);
    assert_eq!(char_count(b"$"), 1);
    assert_eq!(char_count(b"abcde"), 5);
    assert_eq!(char_count(b"\xc2\xa2"), 1);
    assert_eq!(char_count(b"\x24\xc2\xa2"), 2);
    assert_eq!(char_count(b"\xf0\x90\x8d\x88"), 1);

    // uncontinued character counted as single characters
    assert_eq!(char_count(b"\x24\xc2"), 2);
    assert_eq!(char_count(b"\x24\xc2\xc2\xa2"), 3);

    // invalid characters are counted as single characters
    assert_eq!(char_count(b"\x24\xc0\xc0\x34"), 4);
    assert_eq!(char_count(b"\x24\xf5\xfd\xc2"), 4);
}

/// A sequence is split by structure alone: an overlong form, a surrogate and a code point
/// past U+10FFFF, well formed, are one character each; a sequence cut short or broken counts
/// its lead alone, and its continuation bytes, which start none, one each.
#[test]
fn char_count_splits_by_structure() {
    assert_eq!(char_count(b"\xC0\x80"), 1);
    assert_eq!(char_count(b"\xED\xA0\x80"), 1);
    assert_eq!(char_count(b"\xF4\x90\x80\x80"), 1);
    assert_eq!(char_count(b"\xF0\x90\x8D"), 3);
    assert_eq!(char_count(b"\xF0\x41\x8D\x88"), 4);
    assert_eq!(char_count(&[0x80; 5]), 5);
    assert_eq!(char_count(&[0xFF; 3]), 3);
    assert_eq!(char_count("aé€𐍈".as_bytes()), 4);
}

/// Every valid string counts as many characters as the standard library's `chars`.
#[test]
fn char_count_matches_chars_on_valid_strings() {
    for s in [
        "",
        "a",
        "héllo",
        "€uro",
        "𐍈𐍈",
        "\u{FFFE}\u{FDD0}",
        "\u{10FFFF}a\u{7F}",
    ] {
        assert_eq!(char_count(s.as_bytes()), s.chars().count(), "{s:?}");
        assert_eq!(valid_prefix_len(s.as_bytes()), s.len(), "{s:?}");
    }
}

/// The prefix ends at the first byte that starts no sequence, or at the first sequence broken
/// or cut short, and is every byte otherwise.
#[test]
fn valid_prefix_len_stops_at_the_first_broken_sequence() {
    assert_eq!(valid_prefix_len(b""), 0);
    assert_eq!(valid_prefix_len(b"abc"), 3);
    assert_eq!(valid_prefix_len(b"\x80abc"), 0);
    assert_eq!(valid_prefix_len(b"ab\xFFcd"), 2);
    assert_eq!(valid_prefix_len(b"ab\xE2\x82"), 2);
    assert_eq!(valid_prefix_len(b"ab\xE2\x82\xAC"), 5);
    assert_eq!(valid_prefix_len(b"ab\xE2\x41\xAC"), 2);
    assert_eq!(valid_prefix_len(b"\xC2\xA2\xC2"), 2);
    // Structure alone: an overlong form and a surrogate are kept.
    assert_eq!(valid_prefix_len(b"\xC0\x80\xED\xA0\x80"), 5);
    // A hunk header cut inside its last character, as `git_xdiff_cb` sanitizes it.
    let header = "@@ -1 +1 @@ fn café()".as_bytes();
    let cut = &header[..header.len() - 3];
    assert_eq!(valid_prefix_len(cut), cut.len() - 1);
}

/// The first and last code point of each sequence length decode, with their lengths; only
/// the first sequence is read.
#[test]
fn decode_char_reads_each_length() {
    assert_eq!(decode_char(b"\0"), Ok(('\0', 1)));
    assert_eq!(decode_char(b"\x7F\xFF"), Ok(('\u{7F}', 1)));
    assert_eq!(decode_char(b"\xC2\x80"), Ok(('\u{80}', 2)));
    assert_eq!(decode_char(b"\xDF\xBF\xFF"), Ok(('\u{7FF}', 2)));
    assert_eq!(decode_char(b"\xE0\xA0\x80"), Ok(('\u{800}', 3)));
    assert_eq!(decode_char(b"\xEF\xBF\xBD"), Ok(('\u{FFFD}', 3)));
    assert_eq!(decode_char(b"\xF0\x90\x80\x80"), Ok(('\u{10000}', 4)));
    assert_eq!(decode_char(b"\xF4\x8F\xBF\xBD\x80"), Ok(('\u{10FFFD}', 4)));
    assert_eq!(decode_char("¢$".as_bytes()), Ok(('¢', 2)));
}

/// Broken, cut short and overlong sequences, surrogates and code points past U+10FFFF are
/// refused.
#[test]
fn decode_char_refuses_malformed_sequences() {
    let refused: [&[u8]; 16] = [
        b"\x80",
        b"\xBF",
        b"\xF8\x80\x80\x80",
        b"\xFF",
        b"\xC2",
        b"\xE2\x82",
        b"\xF0\x90\x8D",
        b"\xC2\x41",
        b"\xE2\x82\x41",
        b"\xC0\x80",
        b"\xC1\xBF",
        b"\xE0\x9F\xBF",
        b"\xF0\x8F\xBF\xBF",
        b"\xED\xA0\x80",
        b"\xF4\x90\x80\x80",
        b"\xF7\xBF\xBF\xBF",
    ];
    for bytes in refused {
        assert_eq!(decode_char(bytes), Err(InvalidCodePoint), "{bytes:02X?}");
    }
    assert_eq!(decode_char(b"\xED\x9F\xBF"), Ok(('\u{D7FF}', 3)));
    assert_eq!(decode_char(b"\xEE\x80\x80"), Ok(('\u{E000}', 3)));
}

/// The noncharacters are refused: U+FDD0 to U+FDEF, and the last two code points of every
/// plane. Their neighbours decode.
#[test]
fn decode_char_refuses_noncharacters() {
    let mut buf = [0; 4];
    for c in ('\u{FDD0}'..='\u{FDEF}').chain(
        (0..=0x10)
            .flat_map(|plane| [plane << 16 | 0xFFFE, plane << 16 | 0xFFFF])
            .filter_map(char::from_u32),
    ) {
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        assert_eq!(decode_char(bytes), Err(InvalidCodePoint), "{c:?}");
    }
    for c in [
        '\u{FDCF}',
        '\u{FDF0}',
        '\u{FFFD}',
        '\u{1FFFD}',
        '\u{10FFFD}',
    ] {
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        assert_eq!(decode_char(bytes), Ok((c, c.len_utf8())), "{c:?}");
    }
}

/// Empty bytes hold no code point. `git_utf8_iterate(&out, str, 0)` reads `str` as if it were
/// unbounded, past the size its caller gives; the port refuses instead.
#[test]
fn decode_char_refuses_empty_bytes() {
    assert_eq!(decode_char(b""), Err(InvalidCodePoint));
}

/// Every `char`'s encoding decodes to it, with its length, but for the noncharacters. Miri
/// samples them.
#[test]
fn decode_char_reads_every_char() {
    let step = if cfg!(miri) { 4099 } else { 1 };
    let mut buf = [0; 4];
    for c in (0..=0x10_FFFF).step_by(step).filter_map(char::from_u32) {
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        let noncharacter =
            ('\u{FDD0}'..='\u{FDEF}').contains(&c) || u32::from(c) & 0xFFFF >= 0xFFFE;
        let expected = if noncharacter {
            Err(InvalidCodePoint)
        } else {
            Ok((c, c.len_utf8()))
        };
        assert_eq!(decode_char(bytes), expected, "{c:?}");
    }
}

/// Decoding a string one code point at a time, as `next_hfs_char` does, reads every
/// character and ends at its end.
#[test]
fn decode_char_walks_a_string() {
    let s = "a\u{200C}é€𐍈.git";
    let mut rest = s.as_bytes();
    let mut chars = Vec::new();
    while !rest.is_empty() {
        let (c, len) = decode_char(rest).expect("valid");
        chars.push(c);
        rest = &rest[len..];
    }
    assert_eq!(chars, s.chars().collect::<Vec<_>>());
}

/// The error is a `std::error::Error` with a message.
#[test]
fn invalid_code_point_is_an_error() {
    let error: Box<dyn std::error::Error> = Box::new(InvalidCodePoint);
    assert_eq!(error.to_string(), "invalid UTF-8 code point");
}
