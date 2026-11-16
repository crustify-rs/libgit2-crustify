//! The safe Rust API of hash, zstream, regexp, wildmatch, date and rand,
//! used as a Rust consumer would: no `unsafe`.
#![forbid(unsafe_code)]

use core::mem::MaybeUninit;

use libgit2_rs::util::date::{self, Clock, SystemClock, Tm, UtcClock};
use libgit2_rs::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::util::hash::{self, HashAlgorithm, Hasher};
use libgit2_rs::util::regexp::{Regexp, RegexpFlags};
use libgit2_rs::util::str::GitStr;
use libgit2_rs::util::wildmatch::{self, WildMatch, WildmatchFlags};
use libgit2_rs::util::zstream::{self, ZStream, ZstreamKind};
use libgit2_rs::util::{rand, test_support};

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

fn streaming_and_one_shot_agree(alg: HashAlgorithm) {
    let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 + i / 13) as u8).collect();
    let whole = hash::hash_buf(&data, alg).unwrap();
    assert_eq!(whole.as_bytes().len(), alg.size());
    let mut h = Hasher::new(alg).unwrap();
    for piece in data.chunks(333) {
        h.update(piece).unwrap();
    }
    assert_eq!(h.finalize().unwrap(), whole);
    // A restarted computation is a new one.
    h.reset().unwrap();
    h.update(b"abc").unwrap();
    let abc = h.finalize().unwrap();
    assert_eq!(abc, hash::hash_vec(&[b"a", b"", b"bc"], alg).unwrap());
    assert_eq!(h.algorithm(), alg);
}

#[test]
fn sha1_streaming_and_one_shot_agree() {
    let _g = test_support::init();
    streaming_and_one_shot_agree(HashAlgorithm::Sha1);
    assert_eq!(
        hash::hash_buf(b"abc", HashAlgorithm::Sha1)
            .unwrap()
            .to_hex(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

#[test]
#[cfg_attr(miri, ignore = "SHA-256 is OpenSSL's (C)")]
fn sha256_streaming_and_one_shot_agree() {
    let _g = test_support::init();
    streaming_and_one_shot_agree(HashAlgorithm::Sha256);
    assert_eq!(
        hash::hash_buf(b"abc", HashAlgorithm::Sha256)
            .unwrap()
            .to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Long input, against a known digest (1 MiB of b'a').
    let a = vec![b'a'; 1 << 20];
    assert_eq!(
        hash::hash_buf(&a, HashAlgorithm::Sha256).unwrap().to_hex(),
        "9bc1b2a288b26af7257a36277ae3816a7d4f16e89c1e7e77d0a5c48bad62b360"
    );
}

#[test]
#[cfg_attr(miri, ignore = "hashes 400 KiB: too slow under Miri; SHA-256 is C")]
fn hash_detects_the_shattered_collision() {
    let _g = test_support::init();
    let pdf = resource("sha1/shattered-1.pdf");
    let e = hash::hash_buf(&pdf, HashAlgorithm::Sha1).unwrap_err();
    assert_eq!(e, ErrorCode::GENERIC);
    assert_eq!(
        errors::last(),
        (ErrorClass::SHA, b"SHA1 collision attack detected".to_vec())
    );
    let mut h = Hasher::new(HashAlgorithm::Sha1).unwrap();
    h.update(&pdf).unwrap();
    let (d, collision) = h.finalize_report().unwrap();
    assert!(collision);
    // sha1dc's "safe" digest, as the C's.
    assert_eq!(d.to_hex(), "16e96b70000dd1e7c85b8368ee197754400e58ec");
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
    assert_eq!(zstream::inflate(&HELLO).unwrap(), b"hello world");

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

    // Into uninitialized memory.
    let mut zs = ZStream::new(ZstreamKind::Inflate).unwrap();
    zs.set_input(&HELLO);
    let mut out = [MaybeUninit::<u8>::uninit(); 64];
    assert_eq!(zs.get_output_uninit(&mut out).unwrap(), 11);
}

#[test]
fn zstream_inflate_errors() {
    let _g = test_support::init();
    assert!(zstream::inflate(&HELLO[..2]).is_err());
    assert_eq!(
        errors::last(),
        (ErrorClass::ZLIB, b"premature end of zlib stream".to_vec())
    );
    let mut bad = HELLO;
    bad[0] = 0x79;
    assert!(zstream::inflate(&bad).is_err());
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
    assert!(zstream::inflate(&trailing).is_err());
}

#[test]
#[cfg_attr(miri, ignore = "deflates with the system zlib")]
fn zstream_deflate_round_trip() {
    let _g = test_support::init();
    let data: Vec<u8> = (0..300_000u64)
        .map(|i| b"abcdefgh\n"[(i * i % 9) as usize])
        .collect();
    let z = zstream::deflate(&data).unwrap();
    assert!(z.len() < data.len() / 2);
    assert_eq!(zstream::inflate(&z).unwrap(), data);
    assert_eq!(zstream::deflate(b"").unwrap().len(), 8);
    // Streaming deflate gives the same bytes.
    let mut zs = ZStream::new(ZstreamKind::Deflate).unwrap();
    zs.set_input(&data);
    let mut out = vec![0u8; zs.suggest_output_len()];
    let mut total = Vec::new();
    while !zs.done() {
        let n = zs.get_output(&mut out).unwrap();
        total.extend_from_slice(&out[..n]);
    }
    assert_eq!(total, z);
    // Moved to another thread.
    let mut zs = ZStream::new(ZstreamKind::Deflate).unwrap();
    static INPUT: [u8; 1000] = [b'q'; 1000];
    zs.set_input(&INPUT);
    let n = std::thread::spawn(move || {
        let mut out = [0u8; 4096];
        zs.get_output(&mut out).unwrap()
    })
    .join()
    .unwrap();
    assert!(n > 0);
}

#[test]
#[cfg_attr(miri, ignore = "PCRE2 is C")]
fn regexp() {
    let _g = test_support::init();
    let re = Regexp::new(b"^refs/(heads|tags)/(.*)$", RegexpFlags::default()).unwrap();
    assert!(re.is_match(b"refs/heads/main").unwrap());
    assert!(!re.is_match(b"refs/remotes/x").unwrap());
    assert_eq!(re.matches(b"nope"), Err(ErrorCode::NOT_FOUND));
    assert_eq!(
        re.search(b"refs/tags/v1.0", 4).unwrap(),
        [Some(0..14), Some(5..9), Some(10..14), None]
    );
    assert_eq!(re.search(b"nope", 1), Err(ErrorCode::NOT_FOUND));
    let icase = RegexpFlags { ignore_case: true };
    let re = Regexp::new(b"ABC", icase).unwrap();
    assert!(re.is_match(b"xabcx").unwrap());
    // Shared between threads.
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| assert!(re.is_match(b"aBc").unwrap()));
        }
    });
    assert_eq!(
        Regexp::new(b"(", RegexpFlags::default()).unwrap_err(),
        ErrorCode::INVALID_SPEC
    );
    assert_eq!(
        errors::last(),
        (ErrorClass::REGEX, b"missing closing parenthesis".to_vec())
    );
    // A NUL ends the pattern, as C's string does.
    assert!(Regexp::new(b"a\0(", RegexpFlags::default()).is_ok());
}

#[test]
fn wildmatch_api() {
    let none = WildmatchFlags::default();
    let path = WildmatchFlags {
        pathname: true,
        ..none
    };
    let both = WildmatchFlags {
        pathname: true,
        casefold: true,
    };
    assert_eq!(
        wildmatch::wildmatch(b"*.c", b"foo.c", none),
        WildMatch::Match
    );
    assert!(wildmatch::wildmatch(b"*.c", b"foo.c", none).is_match());
    assert_eq!(
        wildmatch::wildmatch(b"*.c", b"dir/foo.c", path),
        WildMatch::NoMatch
    );
    assert_eq!(
        wildmatch::wildmatch(b"**/*.C", b"dir/foo.c", both),
        WildMatch::Match
    );
    assert_eq!(wildmatch::wildmatch(b"[", b"[", none), WildMatch::AbortAll);
    assert_eq!(
        wildmatch::wildmatch(b"*?", b"a/", path),
        WildMatch::AbortToStarstar
    );
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
    assert_eq!(
        date::rfc2822_with(&c, 1_397_031_663, 120).unwrap(),
        "Wed, 9 Apr 2014 10:21:03 +0200"
    );
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
    assert_eq!(
        date::rfc2822(1_397_031_663, -120).unwrap(),
        "Wed, 9 Apr 2014 06:21:03 -0200"
    );
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

/// U2-1: the NUL after a `git_str`'s contents is not handed out: writing
/// anything into the spare capacity leaves the string intact.
#[test]
fn git_str_spare_capacity_keeps_the_nul() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    s.puts(b"abc").unwrap();
    if let Some(b) = s.spare_capacity_mut().first_mut() {
        *b = MaybeUninit::uninit();
    }
    assert_eq!(s.as_c_str().unwrap().to_bytes(), b"abc");

    let mut s = GitStr::new();
    s.puts(b"abc").unwrap();
    for b in s.spare_capacity_mut() {
        b.write(b'x');
    }
    assert_eq!(s.as_c_str().unwrap().to_bytes(), b"abc");
    let c = s.detach().unwrap();
    assert_eq!(c.as_c_str().to_bytes(), b"abc");
    // A full buffer has no spare capacity.
    let mut s = GitStr::new();
    s.puts(b"1234567").unwrap();
    assert_eq!(s.capacity(), 8);
    assert!(s.spare_capacity_mut().is_empty());
}
