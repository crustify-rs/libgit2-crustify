//! The safe Rust API of `net`, `sortedcache` and `process`, used without
//! `unsafe` (PORTING.md §5).

#![forbid(unsafe_code)]

use libgit2_rs::__test::util::test_support;
use libgit2_rs::__test::util::{
    net::Url,
    process::{Process, ProcessOptions, ProcessStatus},
    sortedcache::SortedCache,
};
use libgit2_rs::{ErrorClass, ErrorCode};

#[test]
fn url() {
    let _g = test_support::init();
    let u = Url::parse(b"http://user:p%40ss@[::1]:8080/a/b?x=1#frag").unwrap();
    assert_eq!(u.scheme(), Some(&b"http"[..]));
    assert_eq!(u.host(), Some(&b"::1"[..]));
    assert_eq!(u.port(), Some(&b"8080"[..]));
    assert_eq!(u.password(), Some(&b"p@ss"[..]));
    assert_eq!(u.fragment(), Some(&b"frag"[..]));
    assert!(u.port_specified() && u.is_ipv6() && u.is_valid() && !u.is_default_port());
    assert_eq!(u.path_and_query().unwrap(), b"/a/b?x=1");
    let j = u.join_path(b"/info/refs?service=git-upload-pack").unwrap();
    assert_eq!(
        j.to_bytes().unwrap(),
        b"http://user:p@ss@::1:8080/a/b/info/refs?service=git-upload-pack"
    );

    let mut r = Url::parse_standard_or_scp(b"git@example.com:org/repo.git").unwrap();
    assert_eq!(r.scheme(), Some(&b"ssh"[..]));
    assert_eq!(r.username(), Some(&b"git"[..]));
    assert!(r.matches_pattern_list(b"other.org,.example.com"));
    r.apply_redirect(b"/elsewhere", false, None).unwrap();
    assert_eq!(r.path(), Some(&b"/elsewhere"[..]));
    let c = r.try_clone().unwrap();
    assert_eq!(c.host(), r.host());

    let h = Url::parse_http(b"localhost:3128").unwrap();
    assert_eq!(h.to_bytes().unwrap(), b"http://localhost:3128/");
    assert!(libgit2_rs::__test::util::net::is_url(b"https://x"));
    assert!(libgit2_rs::__test::util::net::hostname_matches_cert(
        b"a.example.com",
        b"*.example.com"
    ));

    let e = Url::parse_scp(b"no-path").unwrap_err();
    assert_eq!(
        (e.code(), e.class()),
        (ErrorCode::INVALID_SPEC, ErrorClass::NET)
    );
    assert_eq!(e.message(), "invalid scp-style path: path is required");
    let e = Url::parse(b"relative").unwrap().to_bytes().unwrap_err();
    assert_eq!(e.message(), "invalid argument: 'url->scheme'");
}

#[test]
fn sorted_cache() {
    let _g = test_support::init();
    let cache: SortedCache<(u32, String)> = SortedCache::new(None).unwrap();
    assert!(cache.is_empty() && cache.path().is_empty());
    {
        let mut w = cache.write();
        for (i, k) in ["b", "c", "a"].into_iter().enumerate() {
            w.upsert_with(k.as_bytes(), || (i as u32, k.to_uppercase()))
                .unwrap();
        }
        assert_eq!(w.entry(0).unwrap().0, b"a");
    }
    let r = cache.read();
    assert_eq!(r.get(b"c").unwrap().1, "C");
    assert_eq!(r.iter().map(|(_, v)| v.0).collect::<Vec<_>>(), [2, 0, 1]);
    assert_eq!(r.index_of(b"b"), Some(1));
    drop(r);
    let other = cache.try_clone().unwrap();
    cache.write().clear();
    assert_eq!(cache.len(), 0);
    assert_eq!(other.len(), 3);
}

#[test]
#[cfg_attr(miri, ignore = "fork, exec and the C fs_path")]
fn process() {
    if !cfg!(feature = "link-c") {
        return;
    }
    let _g = test_support::init();
    let opts = ProcessOptions {
        capture_in: true,
        capture_out: true,
        ..ProcessOptions::default()
    };
    let mut p = Process::new(&[b"tr", b"a-z", b"A-Z"], &[], &opts).unwrap();
    p.start().unwrap();
    assert!(p.id().unwrap() > 0);
    assert_eq!(p.write(b"hello").unwrap(), 5);
    p.close_in().unwrap();
    let mut buf = [0u8; 16];
    let mut got = Vec::new();
    loop {
        let n = p.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"HELLO");
    assert_eq!(p.wait().unwrap(), ProcessStatus::Exited(0));
    assert_eq!(p.wait().unwrap_err().message(), "process is stopped");

    let mut p = Process::from_cmdline(b"exit 7", &[], &ProcessOptions::default()).unwrap();
    p.start().unwrap();
    assert_eq!(p.wait().unwrap(), ProcessStatus::Exited(7));
    assert_eq!(p.read(&mut buf).unwrap_err().class(), ErrorClass::INTERNAL);

    let mut p = Process::new(&[b"/nonexistent/x"], &[], &ProcessOptions::default()).unwrap();
    let e = p.start().unwrap_err();
    assert_eq!(
        e.message(),
        "cannot run /nonexistent/x: No such file or directory"
    );
}
