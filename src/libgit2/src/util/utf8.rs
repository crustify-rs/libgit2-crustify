//! `src/util/utf8.c`: UTF-8 character lengths, iteration and validation, ported to Rust.
//!
//! libgit2 reads UTF-8 in two strengths, both from utf8proc's class table
//! (`UTF8_CLASS`):
//!
//! - **Structure.** [`char_count`] and [`valid_prefix_len`] split bytes into sequences by their
//!   lead byte's class and its continuation bytes alone, as `utf8_charlen` does: an overlong form,
//!   a surrogate or a code point past U+10FFFF whose bytes are well formed counts as one sequence.
//!   These are not [`str::from_utf8`]'s rules, which reject those.
//! - **Code points.** [`decode_char`] also refuses what `git_utf8_iterate` refuses: overlong forms,
//!   surrogates, code points past U+10FFFF and the noncharacters U+FDD0 to U+FDEF and U+*n*FFFE,
//!   U+*n*FFFF of every plane. What it accepts is therefore always a [`char`].
//!
//! Every function reads its argument alone and holds no state, so the module is native Rust.
//! The C that remains calls the three `git_utf8_*` functions (`src/libgit2/path.c` and
//! `src/libgit2/diff_xdiff.c`, and on Windows `src/util/fs_path.c`), which reach this module
//! through its exports: the module has switched from C, and `src/util/utf8.c` has left the
//! build.

use core::error::Error;
use core::fmt;

/// The length of the UTF-8 sequence each lead byte starts, by its value: 1 for ASCII, 2, 3
/// or 4 for a multi-byte lead (`0xC0` to `0xDF`, `0xE0` to `0xEF`, `0xF0` to `0xF7`), and 0
/// for a byte no sequence starts with, a continuation byte (`0x80` to `0xBF`) or `0xF8` to
/// `0xFF`.
///
/// This is utf8proc's table, which libgit2 copies: its class of `0xC0` and `0xC1`, which
/// start only overlong forms, is 2, and of `0xF5` to `0xF7`, which start only code points past
/// U+10FFFF, is 4, so that [`decode_char`] refuses those by their value, as C does.
///
/// Replaces `glb::utf8proc_utf8class`
#[doc(alias = "utf8proc_utf8class")]
static UTF8_CLASS: [u8; 256] = class_table();

/// [`UTF8_CLASS`], from the ranges of lead bytes it classifies.
const fn class_table() -> [u8; 256] {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < table.len() {
        table[byte] = match byte {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 0,
        };
        byte += 1;
    }
    table
}

/// The length of the UTF-8 sequence at the start of `bytes`, by structure alone: its lead
/// byte's class, when `bytes` hold that many bytes and every byte after the lead is a
/// continuation byte (`0b10xx_xxxx`); `None` when `bytes` are empty, the lead starts no
/// sequence or the sequence is cut short or broken.
///
/// `utf8_charlen(str, str_len)` reads past `str_len` when it is 0, from a caller that never
/// passes it: here no length is unbounded, and empty bytes hold none.
///
/// Replaces `fn::utf8_charlen`
#[doc(alias = "utf8_charlen")]
fn sequence_len(bytes: &[u8]) -> Option<usize> {
    let (&lead, rest) = bytes.split_first()?;
    let len = usize::from(UTF8_CLASS[usize::from(lead)]);
    let continuation = rest.get(..len.checked_sub(1)?)?;
    continuation
        .iter()
        .all(|&b| b & 0xC0 == 0x80)
        .then_some(len)
}

/// The error of [`decode_char`]: the bytes it was given start with no valid code point, or
/// are empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InvalidCodePoint;

impl fmt::Display for InvalidCodePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid UTF-8 code point")
    }
}

impl Error for InvalidCodePoint {}

/// The code point at the start of `bytes`, with the length of its sequence in bytes, which
/// is how far its caller advances to read the next one.
///
/// A sequence is refused, as [`InvalidCodePoint`], when it is broken or cut short, when it
/// is overlong (`C0 80` for U+0000), when it encodes a surrogate (U+D800 to U+DFFF) or a
/// code point past U+10FFFF, and when it encodes a noncharacter: U+FDD0 to U+FDEF, or the
/// last two code points of any plane, U+FFFE and U+FFFF to U+10FFFE and U+10FFFF. Only the
/// bytes of the first sequence are read.
///
/// `git_utf8_iterate(&out, str, 0)` reads `str` as if it were unbounded; empty `bytes`
/// here hold no code point, and are refused.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::utf8::{InvalidCodePoint, decode_char};
///
/// assert_eq!(decode_char(b"$x"), Ok(('$', 1)));
/// assert_eq!(decode_char("¢".as_bytes()), Ok(('¢', 2)));
/// assert_eq!(decode_char("𐍈".as_bytes()), Ok(('𐍈', 4)));
/// // Overlong, cut short, and a noncharacter.
/// assert_eq!(decode_char(b"\xC0\x80"), Err(InvalidCodePoint));
/// assert_eq!(decode_char(b"\xE2\x82"), Err(InvalidCodePoint));
/// assert_eq!(decode_char("\u{FFFE}".as_bytes()), Err(InvalidCodePoint));
/// ```
///
/// # Errors
///
/// [`InvalidCodePoint`] when `bytes` start with no valid code point, as above.
///
/// Replaces `fn::git_utf8_iterate`
#[doc(alias = "git_utf8_iterate")]
pub fn decode_char(bytes: &[u8]) -> Result<(char, usize), InvalidCodePoint> {
    let len = sequence_len(bytes).ok_or(InvalidCodePoint)?;
    let tail = |b: u8| u32::from(b & 0x3F);
    let code_point = match bytes[..len] {
        [a] => Some(u32::from(a)),
        [a, b] => Some((u32::from(a & 0x1F) << 6) + tail(b)).filter(|&c| c >= 0x80),
        [a, b, c] => Some((u32::from(a & 0x0F) << 12) + (tail(b) << 6) + tail(c))
            .filter(|&c| c >= 0x800 && !(0xD800..0xE000).contains(&c))
            .filter(|c| !(0xFDD0..0xFDF0).contains(c)),
        [a, b, c, d] => {
            Some((u32::from(a & 0x07) << 18) + (tail(b) << 12) + (tail(c) << 6) + tail(d))
                .filter(|c| (0x1_0000..0x11_0000).contains(c))
        }
        _ => None,
    };
    code_point
        .filter(|&c| c & 0xFFFF < 0xFFFE)
        .and_then(char::from_u32)
        .map(|c| (c, len))
        .ok_or(InvalidCodePoint)
}

/// The number of characters in `bytes`: their well-formed sequences, by structure alone, and
/// each byte that starts none, which counts as a character of its own.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::utf8::char_count;
///
/// assert_eq!(char_count(b"$"), 1);
/// assert_eq!(char_count("$¢".as_bytes()), 2);
/// assert_eq!(char_count("𐍈".as_bytes()), 1);
/// // `$`, two bytes that start no sequence, and `4`.
/// assert_eq!(char_count(b"\x24\xC0\xC1\x34"), 4);
/// ```
///
/// Replaces `fn::git_utf8_char_length`
#[doc(alias = "git_utf8_char_length")]
#[must_use]
pub fn char_count(bytes: &[u8]) -> usize {
    let mut rest = bytes;
    let mut count = 0;
    while !rest.is_empty() {
        let len = sequence_len(rest).unwrap_or(1);
        rest = &rest[len..];
        count += 1;
    }
    count
}

/// The length in bytes of the longest prefix of `bytes` made of well-formed sequences, by
/// structure alone: the offset of the first byte that starts no sequence, or of the first
/// sequence broken or cut short, or `bytes.len()`.
///
/// ```
/// use libgit2_crustify::libgit2::src::util::utf8::valid_prefix_len;
///
/// assert_eq!(valid_prefix_len("$¢".as_bytes()), 3);
/// assert_eq!(valid_prefix_len(b"ab\xFFcd"), 2);
/// // A sequence cut short by the end of the bytes.
/// assert_eq!(valid_prefix_len(b"ab\xE2\x82"), 2);
/// ```
///
/// Replaces `fn::git_utf8_valid_buf_length`
#[doc(alias = "git_utf8_valid_buf_length")]
#[must_use]
pub fn valid_prefix_len(bytes: &[u8]) -> usize {
    let mut offset = 0;
    while let Some(len) = sequence_len(&bytes[offset..]) {
        offset += len;
    }
    offset
}

/// The gateway through which the C that remains calls this module:
/// `git_utf8_iterate`, `git_utf8_char_length` and `git_utf8_valid_buf_length`, each
/// checked against its raw binding's signature, so that a C declaration that drifts fails
/// to build.
mod ffi_export {
    use core::ffi::{c_char, c_int};
    use core::slice;

    use ffibox::callback::barrier;

    use super::{char_count, decode_char, valid_prefix_len};
    use crate::sys::libgit2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::git_utf8_iterate, git_utf8_iterate];
        let _ = [sys::git_utf8_char_length, git_utf8_char_length];
        let _ = [sys::git_utf8_valid_buf_length, git_utf8_valid_buf_length];
    };

    /// The `str_len` bytes at `str`, as C passes a string and its size; no bytes when
    /// `str_len` is 0, whatever `str` is, NULL included.
    ///
    /// # Safety
    ///
    /// When `str_len` is not 0, `str` is valid for reads of `str_len` bytes, which nothing
    /// writes while the returned slice lives.
    unsafe fn bytes<'a>(str: *const c_char, str_len: usize) -> &'a [u8] {
        if str_len == 0 {
            return &[];
        }
        // SAFETY: `str_len` is not 0, so the caller guarantees that `str` is valid for reads
        // of `str_len` bytes nothing writes while the slice lives, which is the call that
        // receives it; `u8` and `c_char` have one size and alignment.
        unsafe { slice::from_raw_parts(str.cast::<u8>(), str_len) }
    }

    /// `git_utf8_iterate`: [`decode_char`] of the `str_len` bytes at `str`, its code point
    /// written through `out` and its length returned; on a refusal, 0 is written and -1
    /// returned, as C does. A panic is reported as a refusal.
    ///
    /// With `str_len` 0, C reads `str` as if it were unbounded; here it holds no code point,
    /// and the call is refused without reading `str`.
    ///
    /// # Safety
    ///
    /// `out` is valid and aligned for a write of a `u32`, as C's `uint32_t *` is, and `str`
    /// valid for reads of `str_len` bytes.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_utf8_iterate(
        out: *mut u32,
        str: *const c_char,
        str_len: usize,
    ) -> c_int {
        // SAFETY: the caller guarantees that `str` is valid for reads of `str_len` bytes,
        // which C does not change during the call.
        let bytes = unsafe { bytes(str, str_len) };
        let (code_point, len) = barrier(
            || match decode_char(bytes) {
                Ok((c, len)) => (u32::from(c), c_int::try_from(len).unwrap_or(-1)),
                Err(_) => (0, -1),
            },
            || (0, -1),
        );
        // SAFETY: the caller guarantees that `out` is valid for a write of a `u32`.
        unsafe { out.write(code_point) };
        len
    }

    /// `git_utf8_char_length`: [`char_count`] of the `str_len` bytes at `str`. It cannot
    /// fail, so a panic aborts.
    ///
    /// # Safety
    ///
    /// `str` is valid for reads of `str_len` bytes.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_utf8_char_length(str: *const c_char, str_len: usize) -> usize {
        // SAFETY: the caller guarantees that `str` is valid for reads of `str_len` bytes,
        // which C does not change during the call.
        let bytes = unsafe { bytes(str, str_len) };
        barrier(|| char_count(bytes), || std::process::abort())
    }

    /// `git_utf8_valid_buf_length`: [`valid_prefix_len`] of the `str_len` bytes at `str`. It
    /// cannot fail, so a panic aborts.
    ///
    /// # Safety
    ///
    /// `str` is valid for reads of `str_len` bytes.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn git_utf8_valid_buf_length(str: *const c_char, str_len: usize) -> usize {
        // SAFETY: the caller guarantees that `str` is valid for reads of `str_len` bytes,
        // which C does not change during the call.
        let bytes = unsafe { bytes(str, str_len) };
        barrier(|| valid_prefix_len(bytes), || std::process::abort())
    }

    /// The exports as their C callers call them: `next_hfs_char`'s walk by `git_utf8_iterate`,
    /// `git_xdiff_cb`'s prefix and `fs_path.c`'s count, each reading only the `str_len` bytes
    /// it is given, from an allocation of exactly that size, so that Miri reports a read past
    /// them. They compile with the gateway.
    #[cfg(test)]
    mod unit_priv {
        use core::ffi::{c_char, c_int};
        use core::ptr;

        use super::{git_utf8_char_length, git_utf8_iterate, git_utf8_valid_buf_length};
        use crate::libgit2::src::util::utf8::{char_count, decode_char, valid_prefix_len};

        /// Inputs of every outcome: each length, cut short, broken, overlong, a surrogate, a
        /// noncharacter, a code point past U+10FFFF and bytes that start no sequence.
        const CASES: [&[u8]; 16] = [
            b"a",
            b"\0",
            b"\xC2\xA2$",
            b"\xE2\x82\xAC",
            b"\xF0\x90\x8D\x88",
            b"\xE2\x82",
            b"\xE2\x41\xAC",
            b"\xC0\x80",
            b"\xED\xA0\x80",
            b"\xEF\xBF\xBE",
            b"\xF4\x90\x80\x80",
            b"\x80",
            b"\xFF",
            b"\x24\xC0\xC0\x34",
            b"\x24\xF5\xFD\xC2",
            b"ab\xE2\x82\xACcd\xFF",
        ];

        /// `git_utf8_iterate` of `bytes`, held in an allocation of exactly their size: what
        /// it writes through `out`, which starts as `u32::MAX`, and what it returns.
        fn iterate(bytes: &[u8]) -> (u32, c_int) {
            let exact: Box<[u8]> = bytes.into();
            let mut out = u32::MAX;
            // SAFETY: `out` is a live, aligned `u32`, and `exact` holds `exact.len()` bytes,
            // which nothing writes during the call.
            let len = unsafe {
                git_utf8_iterate(&raw mut out, exact.as_ptr().cast::<c_char>(), exact.len())
            };
            (out, len)
        }

        /// `git_utf8_char_length` and `git_utf8_valid_buf_length` of `bytes`, held as above.
        fn lengths(bytes: &[u8]) -> (usize, usize) {
            let exact: Box<[u8]> = bytes.into();
            // SAFETY: `exact` holds `exact.len()` bytes, which nothing writes during the call.
            let count = unsafe { git_utf8_char_length(exact.as_ptr().cast(), exact.len()) };
            // SAFETY: as above.
            let prefix = unsafe { git_utf8_valid_buf_length(exact.as_ptr().cast(), exact.len()) };
            (count, prefix)
        }

        /// `git_utf8_iterate` writes the code point `decode_char` reads and returns its
        /// length, and on a refusal writes 0 over what `out` held and returns -1, as C does.
        #[test]
        fn iterate_reports_decode_char_as_c_does() {
            for bytes in CASES {
                let expected = decode_char(bytes).map_or((0, -1), |(c, len)| {
                    (u32::from(c), c_int::try_from(len).expect("at most 4"))
                });
                assert_eq!(iterate(bytes), expected, "{bytes:02X?}");
            }
            assert_eq!(iterate(b"\xF0\x90\x8D\x88"), (0x1_0348, 4));
            assert_eq!(iterate(b"\xC2\xA2$"), (0xA2, 2));
            assert_eq!(iterate(b"\xE2\x82"), (0, -1));
        }

        /// The walk `next_hfs_char` makes: each length is within the bytes left, and the code
        /// points are the string's characters.
        #[test]
        fn iterate_walks_a_path_within_its_size() {
            let path = "a\u{200C}.g\u{FEFF}it/é€𐍈".as_bytes();
            let mut rest = path;
            let mut chars = Vec::new();
            while !rest.is_empty() {
                let (code_point, len) = iterate(rest);
                let len = usize::try_from(len).expect("a valid path");
                assert!((1..=rest.len()).contains(&len), "{rest:02X?}");
                chars.push(char::from_u32(code_point).expect("a scalar value"));
                rest = &rest[len..];
            }
            let expected: Vec<char> = "a\u{200C}.g\u{FEFF}it/é€𐍈".chars().collect();
            assert_eq!(chars, expected);
        }

        /// `git_utf8_char_length` and `git_utf8_valid_buf_length` are `char_count` and
        /// `valid_prefix_len`; a prefix stops before a sequence its size cuts short, as
        /// `git_xdiff_cb` relies on when it truncates a hunk header.
        #[test]
        fn lengths_are_the_native_counts() {
            for bytes in CASES {
                assert_eq!(
                    lengths(bytes),
                    (char_count(bytes), valid_prefix_len(bytes)),
                    "{bytes:02X?}"
                );
            }
            assert_eq!(lengths(b"\x24\xC2\xC2\xA2"), (3, 1));
            assert_eq!(lengths(b"ab\xE2\x82"), (4, 2));
            assert_eq!(lengths(b""), (0, 0));
        }

        /// With `str_len` 0, `str` is not read, whether it is NULL or dangling: the count and
        /// the prefix are 0, and `git_utf8_iterate` refuses, writing 0, where C would read
        /// past the size.
        #[test]
        fn empty_input_is_not_read() {
            for str in [ptr::null(), ptr::dangling::<c_char>()] {
                let mut out = u32::MAX;
                // SAFETY: `out` is a live, aligned `u32`, and `str_len` is 0, for which the
                // export reads nothing at `str`.
                let len = unsafe { git_utf8_iterate(&raw mut out, str, 0) };
                assert_eq!((out, len), (0, -1));
                // SAFETY: `str_len` is 0, for which the export reads nothing at `str`.
                assert_eq!(unsafe { git_utf8_char_length(str, 0) }, 0);
                // SAFETY: as above.
                assert_eq!(unsafe { git_utf8_valid_buf_length(str, 0) }, 0);
            }
        }
    }
}

#[cfg(test)]
mod unit_priv {
    use super::{UTF8_CLASS, class_table, sequence_len};

    /// utf8proc's table, as `src/util/utf8.c` writes it.
    #[rustfmt::skip]
    const C_TABLE: [u8; 256] = [
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
        2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
        3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
        4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    /// The table built from its ranges is C's, entry by entry: the static, which the compiler
    /// evaluates, and the builder run again at run time, which coverage then counts.
    #[test]
    fn the_class_table_is_utf8procs() {
        assert_eq!(UTF8_CLASS, C_TABLE);
        assert_eq!(std::hint::black_box(class_table)(), C_TABLE);
    }

    /// No bytes hold no sequence, where `utf8_charlen(str, 0)` would read past them.
    #[test]
    fn empty_bytes_hold_no_sequence() {
        assert_eq!(sequence_len(b""), None);
    }

    /// Every lead byte, alone and followed by enough continuation bytes: a sequence of its
    /// class's length when it starts one, and none when it is cut short or starts none.
    #[test]
    fn every_lead_byte_starts_its_class() {
        for lead in 0..=u8::MAX {
            let class = usize::from(C_TABLE[usize::from(lead)]);
            let mut seq = vec![lead];
            seq.extend([0x80; 3]);
            let expected = (class != 0).then_some(class);
            assert_eq!(sequence_len(&seq), expected, "{lead:#04X}");
            for cut in 1..class {
                assert_eq!(sequence_len(&seq[..cut]), None, "{lead:#04X} cut to {cut}");
            }
        }
    }

    /// A byte after the lead that is no continuation byte breaks the sequence, wherever it
    /// is; bytes past the sequence are not read.
    #[test]
    fn a_broken_continuation_breaks_the_sequence() {
        assert_eq!(sequence_len(b"\xF0\x90\x8D\x88"), Some(4));
        assert_eq!(sequence_len(b"\xF0\x41\x8D\x88"), None);
        assert_eq!(sequence_len(b"\xF0\x90\xC0\x88"), None);
        assert_eq!(sequence_len(b"\xF0\x90\x8D\xFF"), None);
        assert_eq!(sequence_len(b"\xC2\xA2\xFF"), Some(2));
        // Structure alone: an overlong form and a surrogate are well formed.
        assert_eq!(sequence_len(b"\xC0\x80"), Some(2));
        assert_eq!(sequence_len(b"\xED\xA0\x80"), Some(3));
    }
}
