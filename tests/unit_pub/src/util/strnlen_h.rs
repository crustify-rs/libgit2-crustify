//! `src/util/strnlen.h`: `strnlen`, `p_strnlen`'s port, checked against the length its
//! contract gives, the offset of the first NUL within the bound or the bound itself. The
//! module has no C tests to carry over; these cover the edge cases of the bound and the NUL.
//! None reaches C, so Miri runs them all.

use libgit2_crustify::libgit2::src::util::strnlen_h::strnlen;

/// The length its contract gives, by the standard library's scan.
fn expected(bytes: &[u8]) -> usize {
    bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())
}

/// An empty bound reads nothing and gives zero.
#[test]
fn an_empty_bound_gives_zero() {
    assert_eq!(strnlen(b""), 0);
    assert_eq!(strnlen(&b"abc"[..0]), 0);
}

/// The first NUL ends the string, wherever it is, and later NULs do not count.
#[test]
fn the_first_nul_ends_the_string() {
    assert_eq!(strnlen(b"\0"), 0);
    assert_eq!(strnlen(b"\0abc"), 0);
    assert_eq!(strnlen(b"ab\0cd"), 2);
    assert_eq!(strnlen(b"abc\0"), 3);
    assert_eq!(strnlen(b"a\0\0b\0"), 1);
    assert_eq!(strnlen(&[0; 16]), 0);
}

/// Without a NUL within the bound, the bound is the length, even when one follows it.
#[test]
fn without_a_nul_the_bound_is_the_length() {
    assert_eq!(strnlen(b"a"), 1);
    assert_eq!(strnlen(b"abcdef"), 6);
    assert_eq!(strnlen(&b"abc\0"[..3]), 3);
    assert_eq!(strnlen(&b"abcdef\0"[..4]), 4);
}

/// Bytes past ASCII are not terminators, whatever the signedness of C's `char`.
#[test]
fn high_bytes_are_not_terminators() {
    assert_eq!(strnlen(&[0x80, 0xFF, 0x7F, 0x01]), 4);
    assert_eq!(strnlen(&[0xFF, 0xFF, 0x00, 0xFF]), 2);
    assert_eq!(strnlen("é\0".as_bytes()), 2);
}

/// The longest bound the sweep below reaches. Under `BorrowSanitizer`, whose debug runs cost tens
/// of milliseconds each, it stops at 24, past the 16 bytes from which core's `memchr` scans word
/// by word; every other build sweeps to 70.
const LONGEST: usize = if cfg!(bsan) { 24 } else { 70 };

/// Every NUL position in every bound up to [`LONGEST`] bytes, from every start within a word,
/// so that a scan done word by word would cross each boundary, against the standard library's.
/// Each start fills a buffer of its own: `BorrowSanitizer`'s cost per access to one buffer grows
/// with the borrows of it already made.
#[test]
fn every_nul_position_in_every_bound() {
    for start in 0..8 {
        let mut buf = [b'x'; 80];
        for len in 0..=LONGEST {
            for nul in (0..len).map(Some).chain([None]) {
                buf.fill(b'x');
                if let Some(at) = nul {
                    buf[start + at] = 0;
                }
                let run = &buf[start..start + len];
                assert_eq!(strnlen(run), nul.unwrap_or(len), "{run:?}");
                assert_eq!(strnlen(run), expected(run));
            }
        }
    }
}

/// It is a `const fn`: a length is computed where a constant is needed, as at runtime.
#[test]
fn it_evaluates_in_constants() {
    const LEN: usize = strnlen(b"refs/heads\0main");
    const WHOLE: usize = strnlen(b"refs");
    const ARRAY: [u8; strnlen(b"ab\0")] = [0; 2];
    assert_eq!((LEN, WHOLE, ARRAY.len()), (10, 4, 2));
}
