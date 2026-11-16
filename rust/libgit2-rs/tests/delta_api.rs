//! The safe Rust API of `delta.c` (no `unsafe`).
#![forbid(unsafe_code)]

use libgit2_rs::__test::delta::{self, DeltaIndex};
use libgit2_rs::__test::util::errors::{self, ErrorCode};
use libgit2_rs::__test::util::test_support;

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

/// A xorshift generator (deterministic).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn mutate(rng: &mut Rng, src: &[u8], edits: usize) -> Vec<u8> {
    let mut v = src.to_vec();
    for _ in 0..edits {
        let at = if v.is_empty() { 0 } else { rng.below(v.len()) };
        match rng.below(3) {
            0 if !v.is_empty() => v[at] = rng.next() as u8,
            1 => {
                let n = rng.below(40);
                for _ in 0..n {
                    v.insert(at, rng.next() as u8);
                }
            }
            _ => {
                let n = rng.below(40).min(v.len() - at.min(v.len()));
                v.drain(at..at + n);
            }
        }
    }
    v
}

#[test]
fn roundtrip_random() {
    let _g = test_support::init();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let rounds = if cfg!(miri) { 6 } else { 300 };
    for r in 0..rounds {
        let len = if cfg!(miri) {
            rng.below(600)
        } else {
            rng.below(20000)
        };
        let alphabet = 1 + rng.below(255);
        let src: Vec<u8> = (0..len)
            .map(|_| (rng.next() % alphabet as u64) as u8)
            .collect();
        let trg = mutate(&mut rng, &src, 1 + r % 20);
        match delta::diff(&src, &trg, 0).unwrap() {
            Some(d) => {
                assert!(d.len() >= delta::GIT_DELTA_SIZE_MIN);
                assert_eq!(delta::read_header(&d).unwrap(), (src.len(), trg.len()));
                assert_eq!(delta::apply(&src, &d).unwrap(), trg);
            }
            None => assert!(src.is_empty() || trg.is_empty()),
        }
    }
}

#[test]
fn repetitive_source_and_long_copies() {
    let _g = test_support::init();
    // One repeated window: every bucket entry collides (HASH_LIMIT culling),
    // and copies exceed 64 KiB.
    let n = if cfg!(miri) { 3000 } else { 300_000 };
    let src = vec![b'a'; n];
    let mut trg = src.clone();
    trg.extend_from_slice(b"tail");
    let d = delta::diff(&src, &trg, 0).unwrap().unwrap();
    assert_eq!(delta::apply(&src, &d).unwrap(), trg);
    let src: Vec<u8> = (0..n).map(|i| (i % 16) as u8).collect();
    let trg: Vec<u8> = src.iter().rev().copied().collect();
    let d = delta::diff(&src, &trg, 0).unwrap().unwrap();
    assert_eq!(delta::apply(&src, &d).unwrap(), trg);
}

#[test]
fn empty_and_tiny() {
    let _g = test_support::init();
    assert!(DeltaIndex::new(b"").unwrap().is_none());
    let idx = DeltaIndex::new(b"x").unwrap().unwrap();
    // 32 + 0 entries + 16 buckets.
    assert_eq!(idx.memsize(), 32 + 16 * 8);
    assert!(idx.create(b"", 0).unwrap().is_none());
    let d = idx.create(b"y", 0).unwrap().unwrap();
    assert_eq!(d, [1, 1, 1, b'y']);
    assert_eq!(delta::apply(b"x", &d).unwrap(), b"y");
}

#[test]
fn max_size() {
    let _g = test_support::init();
    let src = vec![7u8; 100];
    let trg: Vec<u8> = (0..5000u32).map(|i| (i * 7919 % 251) as u8).collect();
    assert_eq!(delta::diff(&src, &trg, 10), Err(ErrorCode::BUFS));
    assert_eq!(msg(), "delta would be larger than maximum size");
    let full = delta::diff(&src, &trg, 0).unwrap().unwrap();
    assert_eq!(delta::diff(&src, &trg, full.len()).unwrap().unwrap(), full);
    assert_eq!(
        delta::diff(&src, &trg, full.len() - 1),
        Err(ErrorCode::BUFS)
    );
}

#[test]
fn malformed() {
    let _g = test_support::init();
    let base = [0u8; 16];
    let cases: &[(&[u8], &str)] = &[
        (
            &[0x10, 0x10, 0xff, 0xff, 0xff, 0xff, 0xff, 0x10, 0x00, 0x00],
            "failed to apply delta",
        ),
        (&[0x10, 0x70, 0xff], "failed to apply delta"),
        (
            &[
                0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01,
            ],
            "failed to apply delta: base size does not match given data",
        ),
        (
            &[],
            "failed to apply delta: base size does not match given data",
        ),
        (
            &[0x10],
            "failed to apply delta: base size does not match given data",
        ),
        (
            &[0x11, 0x00],
            "failed to apply delta: base size does not match given data",
        ),
        (&[0x10, 0x01, 0x00], "failed to apply delta"),
        (&[0x10, 0x02, 0x01, b'a'], "failed to apply delta"),
        (&[0x10, 0x01, 0x02, b'a', b'b'], "failed to apply delta"),
        (&[0x10, 0x01, 0x91, 0x10, 0x01], "failed to apply delta"),
        (&[0x10, 0x10, 0x90], "failed to apply delta"),
        (
            &[0x10, 0xff, 0xff, 0xff, 0xff, 0x0f],
            "failed to apply delta: overly large object",
        ),
    ];
    for (d, m) in cases {
        assert_eq!(delta::apply(&base, d), Err(ErrorCode::GENERIC), "{d:?}");
        assert_eq!(msg(), *m, "{d:?}");
    }
    // A copy of the whole base, then an insert.
    assert_eq!(
        delta::apply(&base, &[0x10, 0x11, 0x90, 0x10, 0x01, b'z']).unwrap(),
        [&base[..], b"z"].concat()
    );
    assert_eq!(delta::read_header(&[0x80]), Err(ErrorCode::GENERIC));
    assert_eq!(msg(), "truncated delta");
    assert_eq!(delta::read_header(&[0xff; 11]), Err(ErrorCode::GENERIC));
    assert_eq!(msg(), "delta header overflow");
    assert_eq!(delta::read_header(&[0x80, 0x01, 0x05]).unwrap(), (128, 5));
}

#[test]
fn fuzz_mutated_deltas() {
    let _g = test_support::init();
    let mut rng = Rng(42);
    let rounds = if cfg!(miri) { 40 } else { 20000 };
    let src: Vec<u8> = (0..3000).map(|_| (rng.next() % 7) as u8).collect();
    let trg = mutate(&mut rng, &src, 30);
    let good = delta::diff(&src, &trg, 0).unwrap().unwrap();
    for _ in 0..rounds {
        let mut d = good.clone();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(d.len());
            match rng.below(3) {
                0 => d[at] = rng.next() as u8,
                1 => d.truncate(at),
                _ => d.insert(at, rng.next() as u8),
            }
            if d.is_empty() {
                break;
            }
        }
        let _ = delta::apply(&src, &d);
        let _ = delta::read_header(&d);
    }
}
