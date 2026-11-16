//! The safe Rust API of hash, zstream, regexp, wildmatch, date and rand,
//! used as a Rust consumer would: no `unsafe`.
#![forbid(unsafe_code)]

use libgit2_rs::util::date::{self, Clock, SystemClock, Tm, UtcClock};
use libgit2_rs::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::util::hash::{self, HashAlgorithm, Hasher};
use libgit2_rs::util::regexp::{GIT_REGEXP_ICASE, RegMatch, Regexp};
use libgit2_rs::util::str::GitStr;
use libgit2_rs::util::wildmatch::{
    self, WM_ABORT_ALL, WM_CASEFOLD, WM_MATCH, WM_NOMATCH, WM_PATHNAME,
};
use libgit2_rs::util::zstream::{self, ZStream, ZstreamKind};
use libgit2_rs::util::{rand, test_support};

/// `zlib.compress(b"a" * 16)`.
const A16: [u8; 11] = [
    0x78, 0x9c, 0x4b, 0x4c, 0x44, 0x05, 0x00, 0x33, 0x98, 0x06, 0x11,
];
/// `zlib.compress(b"hello world")`.
const HELLO: [u8; 19] = [
    0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x28, 0xcf, 0x2f, 0xca, 0x49, 0x01, 0x00, 0x1a,
    0x0b, 0x04, 0x5d,
];

fn resource(path: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/resources")
        .join(path);
    std::fs::read(p).expect("test resource")
}

#[test]
fn hash_streaming_and_one_shot_agree() {
    let _g = test_support::init();
    let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 + i / 13) as u8).collect();
    for alg in [HashAlgorithm::Sha1, HashAlgorithm::Sha256] {
        let whole = hash::hash_buf(&data, alg).unwrap();
        assert_eq!(whole.as_bytes().len(), alg.size());
        let mut h = Hasher::new(alg);
        for piece in data.chunks(333) {
            h.update(piece);
        }
        assert_eq!(h.finalize().unwrap(), whole);
        // A restarted computation is a new one.
        h.reset();
        h.update(b"abc");
        let abc = h.finalize().unwrap();
        assert_eq!(abc, hash::hash_vec(&[b"a", b"", b"bc"], alg).unwrap());
    }
    assert_eq!(
        hash::hash_buf(b"abc", HashAlgorithm::Sha256)
            .unwrap()
            .to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hash::hash_buf(b"abc", HashAlgorithm::Sha1)
            .unwrap()
            .to_hex(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(HashAlgorithm::from_raw(0), None);
    assert!(hash::algorithm(3).is_err());
    assert_eq!(
        errors::last(),
        (ErrorClass::INTERNAL, b"unknown hash algorithm".to_vec())
    );
}

#[test]
#[cfg_attr(miri, ignore = "hashes 400 KiB: too slow under Miri")]
fn hash_detects_the_shattered_collision() {
    let _g = test_support::init();
    let pdf = resource("sha1/shattered-1.pdf");
    let e = hash::hash_buf(&pdf, HashAlgorithm::Sha1).unwrap_err();
    assert_eq!(e, ErrorCode::GENERIC);
    assert_eq!(
        errors::last(),
        (ErrorClass::SHA, b"SHA1 collision attack detected".to_vec())
    );
    let mut h = Hasher::new(HashAlgorithm::Sha1);
    h.update(&pdf);
    let mut raw = [0u8; hash::MAX_SIZE];
    assert!(h.finalize_raw(&mut raw));
    // sha1dc's "safe" digest, as the C's.
    let mut hex = [0u8; 40];
    hash::fmt_into(&mut hex, &raw[..20]);
    assert_eq!(&hex, b"16e96b70000dd1e7c85b8368ee197754400e58ec");
    // The thread's reused computation starts clean after a collision.
    assert_eq!(
        hash::hash_buf(b"abc", HashAlgorithm::Sha1)
            .unwrap()
            .to_hex(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    // SHA-256 has no such check.
    assert!(hash::hash_buf(&pdf, HashAlgorithm::Sha256).is_ok());
    let hello = resource("sha1/hello_c");
    assert_eq!(
        hash::hash_buf(&hello, HashAlgorithm::Sha1)
            .unwrap()
            .to_hex(),
        "4e72679e3ea4d04e0c642f029e61eb8056c7ed94"
    );
}

#[test]
fn zstream_inflates() {
    let _g = test_support::init();
    let mut out = GitStr::new();
    zstream::inflate_buf(&mut out, &HELLO).unwrap();
    assert_eq!(out.as_bytes(), b"hello world");
    assert_eq!(out.as_c_str().unwrap().to_bytes(), b"hello world");

    // Output that fills the buffer exactly is still NUL-terminated
    // (rust/DEFECTS.md).
    let mut out = GitStr::new();
    zstream::inflate_buf(&mut out, &A16).unwrap();
    assert_eq!(out.as_bytes(), [b'a'; 16]);
    assert!(out.capacity() > out.len());
    assert_eq!(out.as_c_str().unwrap().to_bytes(), [b'a'; 16]);

    // Streaming into a small buffer.
    let mut zs = ZStream::new(ZstreamKind::Inflate).unwrap();
    zs.set_input(&HELLO);
    let mut got = Vec::new();
    let mut buf = [0u8; 3];
    while !zs.done() {
        let n = zs.get_output_chunk(&mut buf).unwrap();
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"hello world");
    assert!(zs.eos());
    assert_eq!(zs.in_len(), 0);
    zs.reset();
    assert!(zs.done());
}

#[test]
fn zstream_inflate_errors() {
    let _g = test_support::init();
    let mut out = GitStr::new();
    assert!(zstream::inflate_buf(&mut out, &HELLO[..2]).is_err());
    assert_eq!(
        errors::last(),
        (ErrorClass::ZLIB, b"premature end of zlib stream".to_vec())
    );
    let mut bad = HELLO;
    bad[0] = 0x79;
    assert!(zstream::inflate_buf(&mut GitStr::new(), &bad).is_err());
    assert_eq!(
        errors::last(),
        (ErrorClass::ZLIB, b"incorrect header check".to_vec())
    );
    let mut trailing = HELLO.to_vec();
    trailing.extend_from_slice(b"xyz");
    let mut zs = ZStream::new(ZstreamKind::Inflate).unwrap();
    zs.set_input(&trailing);
    let mut buf = [0u8; 64];
    assert_eq!(zs.get_output(&mut buf).unwrap(), 11);
    assert_eq!(zs.in_len(), 3);
    assert!(zs.get_output(&mut buf).is_err());
    assert_eq!(
        errors::last(),
        (
            ErrorClass::ZLIB,
            b"zlib input had trailing garbage".to_vec()
        )
    );
}

#[test]
#[cfg_attr(miri, ignore = "deflates with the system zlib")]
fn zstream_deflate_round_trip() {
    let _g = test_support::init();
    let data: Vec<u8> = (0..300_000u64)
        .map(|i| b"abcdefgh\n"[(i * i % 9) as usize])
        .collect();
    let mut z = GitStr::new();
    zstream::deflate_buf(&mut z, &data).unwrap();
    assert!(z.len() < data.len() / 2);
    let mut back = GitStr::new();
    zstream::inflate_buf(&mut back, z.as_bytes()).unwrap();
    assert_eq!(back.as_bytes(), &data[..]);
    // The empty input deflates to 8 bytes, which filled the C's 8-byte
    // buffer and left it without a NUL.
    let mut e = GitStr::new();
    zstream::deflate_buf(&mut e, b"").unwrap();
    assert_eq!(e.len(), 8);
    assert!(e.as_c_str().is_some());
    // Streaming deflate.
    let mut zs = ZStream::new(ZstreamKind::Deflate).unwrap();
    zs.set_input(&data);
    let mut out = vec![0u8; zs.suggest_output_len()];
    let mut total = Vec::new();
    while !zs.done() {
        let n = zs.get_output(&mut out).unwrap();
        total.extend_from_slice(&out[..n]);
    }
    assert_eq!(total, z.as_bytes());
}

#[test]
#[cfg_attr(miri, ignore = "PCRE2 is C")]
fn regexp() {
    let _g = test_support::init();
    let re = Regexp::new(b"^refs/(heads|tags)/(.*)$", 0).unwrap();
    assert!(re.is_match(b"refs/heads/main").unwrap());
    assert!(!re.is_match(b"refs/remotes/x").unwrap());
    assert_eq!(re.matches(b"nope"), Err(ErrorCode::NOT_FOUND));
    let m = re.search(b"refs/tags/v1.0", 4).unwrap();
    assert_eq!(
        m,
        [
            RegMatch { start: 0, end: 14 },
            RegMatch { start: 5, end: 9 },
            RegMatch { start: 10, end: 14 },
            RegMatch { start: -1, end: -1 }
        ]
    );
    let re = Regexp::new(b"ABC", GIT_REGEXP_ICASE).unwrap();
    assert!(re.is_match(b"xabcx").unwrap());
    // Shared between threads.
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| assert!(re.is_match(b"aBc").unwrap()));
        }
    });
    assert_eq!(Regexp::new(b"(", 0).unwrap_err(), ErrorCode::INVALID_SPEC);
    assert_eq!(
        errors::last(),
        (ErrorClass::REGEX, b"missing closing parenthesis".to_vec())
    );
    // A NUL ends the pattern, as C's string does.
    assert!(Regexp::new(b"a\0(", 0).is_ok());
}

#[test]
fn wildmatch_api() {
    assert_eq!(wildmatch::wildmatch(b"*.c", b"foo.c", 0), WM_MATCH);
    assert_eq!(
        wildmatch::wildmatch(b"*.c", b"dir/foo.c", WM_PATHNAME),
        WM_NOMATCH
    );
    assert_eq!(
        wildmatch::wildmatch(b"**/*.C", b"dir/foo.c", WM_PATHNAME | WM_CASEFOLD),
        WM_MATCH
    );
    assert_eq!(wildmatch::wildmatch(b"[", b"[", 0), WM_ABORT_ALL);
}

#[test]
fn date_api_on_a_fixed_clock() {
    let _g = test_support::init();
    let c = UtcClock { now: 1_234_567_890 };
    assert_eq!(
        date::offset_parse_with(&c, b"2 weeks ago").unwrap(),
        (1_234_567_890 - 14 * 86_400, None)
    );
    assert_eq!(
        date::offset_parse_with(&c, b"Fri, 13 Feb 2009 23:31:30 +0100").unwrap(),
        (1_234_564_290, Some(60))
    );
    assert!(date::offset_parse_with(&c, b"no date here").is_err());
    let mut tm = Tm::default();
    assert!(c.gmtime(c.time(), &mut tm));
    assert_eq!((tm.year + 1900, tm.mon + 1, tm.mday), (2009, 2, 13));
    let mut s = GitStr::new();
    date::rfc2822_fmt_with(&c, &mut s, 1_397_031_663, 120).unwrap();
    assert_eq!(s.as_bytes(), b"Wed, 9 Apr 2014 10:21:03 +0200");
}

#[test]
#[cfg_attr(miri, ignore = "the system clock is C (libc)")]
fn date_api_on_the_system_clock() {
    let _g = test_support::init();
    assert_eq!(date::parse(b"@1234567890 +0000").unwrap(), 1_234_567_890);
    assert_eq!(
        date::offset_parse(b"2008-12-05 10:11:12 +0200").unwrap(),
        (1_228_464_672, Some(120))
    );
    let now = SystemClock.time();
    let (t, off) = date::offset_parse(b"now").unwrap();
    assert!((t - now).abs() < 5 && off.is_none());
    let mut s = GitStr::new();
    date::rfc2822_fmt(&mut s, 1_397_031_663, -120).unwrap();
    assert_eq!(s.as_bytes(), b"Wed, 9 Apr 2014 06:21:03 -0200");
}

#[test]
fn rand_sequences_are_the_c_ones() {
    // From rust/difftest/match_diff.c, identical for the C and the Rust.
    rand::seed(0);
    let got: Vec<u64> = (0..3).map(|_| rand::next()).collect();
    assert_eq!(
        got,
        [0x99ec5f36cb75f2b4, 0xbf6e1f784956452a, 0x1a5f849d4933e6e0]
    );
    rand::seed(1);
    assert_eq!(rand::next(), 0xb3f2af6d0fc710c5);
}
