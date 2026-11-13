//! Equivalence of `src/util/strnlen.h`'s `p_strnlen` with its port, `strnlen`.
//!
//! The raw bindings do not reach `p_strnlen`, a macro, so it is compared through the C that
//! expands it: `git__strndup(s, n)` (`src/util/alloc.c`) copies `p_strnlen(s, n)` bytes of
//! `s`, which `StrFree::dup_prefix` reaches through its raw binding. On this build the
//! expansion is libc's `strnlen`. Each run is the same bytes for both sides, the C reading its
//! own copy, compared by the length and the bytes the C copied.

use libgit2_crustify::libgit2::src::util::alloc::StrFree;
use libgit2_crustify::libgit2::src::util::strnlen_h::strnlen;

/// `p_strnlen(run, run.len())` as `git__strndup` measures it: the bytes it copies.
fn c_strnlen(run: &[u8]) -> usize {
    let copy = StrFree::dup_prefix(run.to_vec().as_slice()).expect("git__strndup");
    assert_eq!(copy.as_bytes(), &run[..copy.as_bytes().len()]);
    copy.as_bytes().len()
}

/// The edge cases of the bound and the NUL: none, first, inner, last and several; bytes past
/// ASCII; and a bound that ends before a NUL.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn edge_cases_match_the_c() {
    let runs: [&[u8]; 12] = [
        b"",
        b"\0",
        b"\0abc",
        b"ab\0cd",
        b"abc\0",
        b"a\0\0b\0",
        b"abcdef",
        &b"abc\0"[..3],
        &[0x80, 0xFF, 0x7F, 0x01],
        &[0xFF, 0xFF, 0x00, 0xFF],
        &[0; 16],
        &[b'x'; 4096],
    ];
    for run in runs {
        assert_eq!(strnlen(run), c_strnlen(run), "{run:?}");
    }
}

/// The longest bound the sweep below reaches. Under `BorrowSanitizer`, whose debug runs cost tens
/// of milliseconds each, it stops at 24, past the 16 bytes from which core's `memchr` scans word
/// by word, and leaves glibc's longer vector cases to the other builds, which sweep to 70.
const LONGEST: usize = if cfg!(bsan) { 24 } else { 70 };

/// Every NUL position in every bound up to [`LONGEST`] bytes, from every start within a word,
/// so that glibc's vectorized `strnlen` crosses each of its alignment cases. Each start fills a
/// buffer of its own: `BorrowSanitizer`'s cost per access to one buffer grows with the borrows of
/// it already made.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_nul_position_matches_the_c() {
    for start in 0..8 {
        let mut buf = [b'x'; 80];
        for len in 0..=LONGEST {
            for nul in (0..len).map(Some).chain([None]) {
                buf.fill(b'x');
                if let Some(at) = nul {
                    buf[start + at] = 0;
                }
                let run = &buf[start..start + len];
                assert_eq!(strnlen(run), c_strnlen(run), "{run:?}");
            }
        }
    }
}
