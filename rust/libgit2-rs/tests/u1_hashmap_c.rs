//! `GitHashMap` against the C khash of `hashmap.h`: the same sequence of
//! operations as `rust/difftest/hashmap_order.c` must print the same states
//! and iteration orders.
//!
//! The C program is built against the C reference (`C_BUILD`, default
//! `/work/build-c`) with `rust/difftest/link.sh`.

#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use libgit2_rs::__test::util::hashmap::{CStrKey, GitHashMap, GitHashSet};
use libgit2_rs::__test::util::test_support;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}

fn rust_output() -> String {
    let mut rng = Rng(99);
    let keys: Vec<std::ffi::CString> = (0..4096)
        .map(|i| {
            let mut v = format!("k{}", rng.next() % 100000).into_bytes();
            v.push(0xe9);
            v.extend_from_slice(i.to_string().as_bytes());
            v.truncate(15);
            std::ffi::CString::new(v).unwrap()
        })
        .collect();
    let mut out = String::new();
    let mut m: GitHashMap<i64, i64> = GitHashMap::new();
    let mut s: GitHashMap<CStrKey<'_>, i64> = GitHashMap::new();
    let mut set: GitHashSet<CStrKey<'_>> = GitHashSet::new();
    let order = |out: &mut String, m: &GitHashMap<i64, i64>| {
        out.push_str("order");
        for (k, v) in m {
            let _ = write!(out, " {k}={v}");
        }
        out.push('\n');
    };
    for i in 0..30000i64 {
        let op = rng.next() % 10;
        let mut k = (rng.next() % 3000) as i64;
        if rng.next().is_multiple_of(2) {
            k = k.wrapping_mul(0x1_0000_0001);
        }
        let r: i64 = if op < 5 {
            m.put(k, i).map_or(-1, |_| 0)
        } else if op < 8 {
            m.remove(&k).map_or(-3, |_| 0)
        } else if op < 9 {
            match m.get(&k) {
                Some(v) => v % 1000,
                None => -3000 - 1,
            }
        } else if i % 997 == 0 {
            m.clear();
            7
        } else {
            m.contains(&k) as i64
        };
        if i % 50 == 0 {
            let _ = writeln!(
                out,
                "off {i} op={op} r={r} size={} buckets={} occ={}",
                m.len(),
                m.buckets(),
                m.occupied()
            );
        }
        if i % 1000 == 999 {
            order(&mut out, &m);
        }

        let sk = CStrKey::new(&keys[(rng.next() % 4096) as usize]);
        let op = rng.next() % 10;
        let r: i64 = if op < 6 {
            s.put(sk, i).map_or(-1, |_| 0)
        } else if op < 9 {
            s.remove(&sk).map_or(-3, |_| 0)
        } else {
            s.contains(&sk) as i64
        };
        if i % 50 == 0 {
            let _ = writeln!(
                out,
                "str {i} op={op} r={r} size={} buckets={} occ={}",
                s.len(),
                s.buckets(),
                s.occupied()
            );
        }
        if i % 3 == 0 {
            set.add(sk).unwrap();
            if i % 999 == 0 {
                let _ = writeln!(out, "set size={} buckets={}", set.len(), set.buckets());
            }
        }
    }
    let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    out.push_str("strorder");
    for (k, v) in &s {
        let _ = write!(out, " {}={v}", lossy(k.as_c_str().to_bytes()));
    }
    out.push_str("\nsetorder");
    let mut it = 0;
    while let Some(k) = set.iterate_keys(&mut it) {
        let _ = write!(out, " {}", lossy(k.as_c_str().to_bytes()));
    }
    out.push('\n');
    let r = m.resize(5).map_or(-1, |()| 0);
    let _ = writeln!(out, "resize5 r={r} buckets={}", m.buckets());
    let r = m.resize(100000).map_or(-1, |()| 0);
    let _ = writeln!(
        out,
        "resize100000 r={r} buckets={} size={}",
        m.buckets(),
        m.len()
    );
    order(&mut out, &m);
    out
}

#[test]
#[cfg_attr(
    any(miri, not(feature = "link-c")),
    ignore = "builds and runs a C program against the C reference"
)]
fn same_as_c_khash() {
    let _g = test_support::init();
    let difftest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../difftest");
    let out = std::env::temp_dir().join(format!("u1-hashmap-{}", std::process::id()));
    let st = Command::new(difftest.join("link.sh"))
        .arg(difftest.join("hashmap_order.c"))
        .arg(&out)
        .env("RUST_BUILD", "/nonexistent")
        .status()
        .expect("link.sh");
    assert!(st.success());
    let c = Command::new(out.join("hashmap_order_c"))
        .output()
        .expect("run");
    assert!(c.status.success());
    let c = String::from_utf8_lossy(&c.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&out);
    let r = rust_output();
    if c != r {
        let (cl, rl): (Vec<_>, Vec<_>) = (c.lines().collect(), r.lines().collect());
        let i = cl
            .iter()
            .zip(&rl)
            .position(|(a, b)| a != b)
            .unwrap_or(cl.len().min(rl.len()));
        panic!(
            "differs at line {}:\nC:    {}\nRust: {}",
            i + 1,
            cl.get(i).map_or("", |l| &l[..l.len().min(300)]),
            rl.get(i).map_or("", |l| &l[..l.len().min(300)])
        );
    }
    assert!(c.lines().count() > 1000);
}
