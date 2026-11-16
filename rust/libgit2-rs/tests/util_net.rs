//! Tests of `net.c`'s port: the parsers, formatting, joins, redirects and
//! patterns, through the mirrored `GitNetUrl` and the exported C ABI
//! (called with raw pointers, as C does). Expected values come from the C
//! reference (`tests/util/url/*`, `rust/difftest/net_diff.c`).

// The export tests call the exports in sequences, as C does; the SAFETY
// comment of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int};
use core::mem::MaybeUninit;

use libgit2_rs::__test::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::__test::util::net::{self, GitNetUrl};
use libgit2_rs::__test::util::str::GitStr;
use libgit2_rs::__test::util::test_support;

// The C ABI under test, declared as C declares it (the signatures are the
// `src` exports').
unsafe extern "C" {
    fn git_net_hostname_matches_cert(hostname: *const c_char, pattern: *const c_char) -> bool;
    fn git_net_str_is_url(str: *const c_char) -> bool;
    fn git_net_url_apply_redirect(
        url: *mut GitNetUrl,
        redirect_location: *const c_char,
        allow_offsite: bool,
        service_suffix: *const c_char,
    ) -> c_int;
    fn git_net_url_dispose(url: *mut GitNetUrl);
    fn git_net_url_dup(out: *mut GitNetUrl, in_: *mut GitNetUrl) -> c_int;
    fn git_net_url_fmt(buf: *mut GitStr, url: *mut GitNetUrl) -> c_int;
    fn git_net_url_is_default_port(url: *mut GitNetUrl) -> bool;
    fn git_net_url_is_ipv6(url: *mut GitNetUrl) -> bool;
    fn git_net_url_joinpath(out: *mut GitNetUrl, one: *mut GitNetUrl, path: *const c_char)
    -> c_int;
    fn git_net_url_matches_pattern_list(url: *mut GitNetUrl, pattern_list: *const c_char) -> bool;
    fn git_net_url_parse(url: *mut GitNetUrl, str: *const c_char) -> c_int;
    fn git_net_url_parse_scp(url: *mut GitNetUrl, str: *const c_char) -> c_int;
    fn git_net_url_swap(a: *mut GitNetUrl, b: *mut GitNetUrl);
    fn git_net_url_valid(url: *mut GitNetUrl) -> bool;
}

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

fn s(f: &Option<libgit2_rs::__test::util::alloc::GitCStr>) -> Option<&str> {
    f.as_ref().map(|c| c.to_str().unwrap())
}

fn parsed(f: fn(&mut GitNetUrl, &[u8]) -> errors::Result<()>, input: &str) -> GitNetUrl {
    let mut u = GitNetUrl::new();
    f(&mut u, input.as_bytes()).unwrap_or_else(|e| panic!("{input}: {e:?} {}", msg()));
    u
}

#[track_caller]
fn check(u: &GitNetUrl, fields: [Option<&str>; 8], port_specified: u32) {
    let got = [
        s(&u.scheme),
        s(&u.host),
        s(&u.port),
        s(&u.path),
        s(&u.query),
        s(&u.fragment),
        s(&u.username),
        s(&u.password),
    ];
    assert_eq!(got, fields);
    assert_eq!(u.port_specified, port_specified);
}

#[test]
fn parse_full_urls() {
    let _g = test_support::init();
    let u = parsed(
        net::parse,
        "http://user:pass@example.com:8080/resource?q=1#frag",
    );
    check(
        &u,
        [
            Some("http"),
            Some("example.com"),
            Some("8080"),
            Some("/resource"),
            Some("q=1"),
            Some("frag"),
            Some("user"),
            Some("pass"),
        ],
        1,
    );
    let u = parsed(net::parse, "HTTPS://Example.COM");
    check(
        &u,
        [
            Some("https"),
            Some("Example.COM"),
            Some("443"),
            Some("/"),
            None,
            None,
            None,
            None,
        ],
        0,
    );
    let u = parsed(
        net::parse,
        "http://us%65r:pa%73s@[fe80::dcad:beff:fe00:0001]:9090/r",
    );
    check(
        &u,
        [
            Some("http"),
            Some("fe80::dcad:beff:fe00:0001"),
            Some("9090"),
            Some("/r"),
            None,
            None,
            Some("user"),
            Some("pass"),
        ],
        1,
    );
    assert!(u.is_ipv6());
    // a relative path, and a non-hierarchical URL
    let u = parsed(net::parse, "relative/path");
    check(
        &u,
        [
            None,
            None,
            None,
            Some("relative/path"),
            None,
            None,
            None,
            None,
        ],
        0,
    );
    let u = parsed(net::parse, "mailto:user@example.com");
    check(
        &u,
        [
            Some("mailto"),
            None,
            None,
            Some("user@example.com"),
            None,
            None,
            None,
            None,
        ],
        0,
    );
    // an empty port is the default
    let u = parsed(net::parse, "http://example.com:/");
    check(
        &u,
        [
            Some("http"),
            Some("example.com"),
            Some("80"),
            Some("/"),
            None,
            None,
            None,
            None,
        ],
        0,
    );
}

#[test]
fn parse_errors_match_c() {
    let _g = test_support::init();
    for bad in [
        "http://[fe80::dcad:beff:fe00:0001",
        "http://fe80::dcad:beff:fe00:0001]/",
        "http://[fe80::dcad:beff:fe00:000x]/",
        "http://user@name@example.com/",
        "x://h:p:q/",
    ] {
        let mut u = GitNetUrl::new();
        assert_eq!(
            net::parse(&mut u, bad.as_bytes()),
            Err(ErrorCode::INVALID_SPEC),
            "{bad}"
        );
        assert_eq!(msg(), "invalid url: malformed hostname");
        assert_eq!(errors::last_class(), Some(ErrorClass::NET));
        assert!(u.scheme.is_none() && u.path.is_none());
    }
    // `@` in the user name is allowed for SSH, and for every prefix of
    // "ssh", "ssh+git" and "git+ssh" (C's `strncasecmp` with the scheme's
    // length).
    for ok in [
        "ssh://user@name@example.com/",
        "git://u@n@h/",
        "g://a@b@c/",
        "S://a@b@c/",
    ] {
        let u = parsed(net::parse, ok);
        assert!(
            u.username.as_ref().unwrap().to_bytes().contains(&b'@'),
            "{ok}"
        );
    }
    // The empty string reaches C's `GIT_ASSERT(!"unhandled state")`.
    let mut u = GitNetUrl::new();
    assert_eq!(net::parse(&mut u, b""), Err(ErrorCode::GENERIC));
    assert_eq!(
        msg(),
        "unrecoverable internal error: '!\"unhandled state\"'"
    );
    assert_eq!(errors::last_class(), Some(ErrorClass::INTERNAL));
}

#[test]
fn parse_http_and_scp() {
    let _g = test_support::init();
    let u = parsed(net::parse_http, "user:pass@localhost:8080/path?q#f");
    check(
        &u,
        [
            Some("http"),
            Some("localhost"),
            Some("8080"),
            Some("/path"),
            Some("q"),
            Some("f"),
            Some("user"),
            Some("pass"),
        ],
        1,
    );
    let u = parsed(net::parse_http, "");
    check(
        &u,
        [
            Some("http"),
            None,
            Some("80"),
            Some("/"),
            None,
            None,
            None,
            None,
        ],
        0,
    );

    let u = parsed(net::parse_scp, "git@github.com:libgit2/libgit2.git");
    check(
        &u,
        [
            Some("ssh"),
            Some("github.com"),
            Some("22"),
            Some("libgit2/libgit2.git"),
            None,
            None,
            Some("git"),
            None,
        ],
        0,
    );
    let u = parsed(net::parse_scp, "[git@github.com:42]:x");
    check(
        &u,
        [
            Some("ssh"),
            Some("github.com"),
            Some("42"),
            Some("x"),
            None,
            None,
            Some("git"),
            None,
        ],
        1,
    );
    let u = parsed(net::parse_scp, "[fe80::dcad:beff:fe00:0001]:repo");
    assert_eq!(s(&u.host), Some("[fe80::dcad:beff:fe00:0001]"));

    for (bad, m) in [
        ("@host:path", "unexpected '@'"),
        (":path", "unexpected ':'"),
        ("host", "path is required"),
        ("[[[x]]]:x", "unexpected '['"),
        ("h]:p", "unexpected ']'"),
        ("[h:p]x", "unexpected character after ipv6 address"),
        ("[::1]x:p", "unexpected character after ipv6 address"),
        ("[github.com]x:p", "unexpected character after hostname"),
    ] {
        let mut u = GitNetUrl::new();
        assert_eq!(
            net::parse_scp(&mut u, bad.as_bytes()),
            Err(ErrorCode::INVALID_SPEC),
            "{bad}"
        );
        assert_eq!(msg(), format!("invalid scp-style path: {m}"), "{bad}");
    }
    assert!(net::str_is_url(b"x+y.z-1://"));
    assert!(!net::str_is_url(b"git@host:x"));
    let u = parsed(net::parse_standard_or_scp, "host:path");
    assert_eq!(s(&u.scheme), Some("ssh"));
}

#[test]
fn format_join_redirect() {
    let _g = test_support::init();
    let mut u = parsed(net::parse, "https://u:p@example.com:443/a/b/?x=1#f");
    let mut b = GitStr::new();
    u.fmt(&mut b).unwrap();
    assert_eq!(b.as_bytes(), b"https://u:p@example.com/a/b/?x=1");
    let mut b = GitStr::new();
    u.fmt_path(&mut b).unwrap();
    assert_eq!(b.as_bytes(), b"/a/b/?x=1");

    let mut j = GitNetUrl::new();
    j.joinpath(&u, b"///c/d?foo").unwrap();
    check(
        &j,
        [
            Some("https"),
            Some("example.com"),
            Some("443"),
            Some("/a/b/c/d"),
            Some("foo"),
            None,
            Some("u"),
            Some("p"),
        ],
        0,
    );

    u.apply_redirect(
        b"/new/info/refs?service=git-upload-pack",
        false,
        Some(b"/info/refs?service=git-upload-pack"),
    )
    .unwrap();
    assert_eq!(s(&u.path), Some("/new"));
    assert!(u.query.is_none());

    // offsite and scheme downgrades
    let mut u = parsed(net::parse, "https://example.com/x");
    assert_eq!(
        u.apply_redirect(b"http://example.com/y", false, None),
        Err(ErrorCode::GENERIC)
    );
    assert_eq!(msg(), "cannot redirect from 'https' to 'http'");
    assert_eq!(
        u.apply_redirect(b"https://other.com/y", false, None),
        Err(ErrorCode::GENERIC)
    );
    assert_eq!(msg(), "cannot redirect from 'example.com' to 'other.com'");
    u.apply_redirect(b"https://other.com/y/info/refs", true, Some(b"/info/refs"))
        .unwrap();
    assert_eq!(s(&u.host), Some("other.com"));
    assert_eq!(s(&u.path), Some("/y"));
}

#[test]
fn null_fields_are_defined() {
    // The C dereferences these NULL fields (rust/DEFECTS.md).
    let _g = test_support::init();
    let empty = GitNetUrl::new();
    assert!(!empty.is_ipv6());
    assert!(!empty.is_default_port());
    assert!(!empty.matches_pattern(b"example.com"));
    assert!(empty.matches_pattern(b"*"));
    assert!(empty.matches_pattern(b"."));
    let mut u = parsed(net::parse, "http://example.com/x");
    u.port = None;
    assert!(!u.is_default_port());
    assert!(!u.matches_pattern(b"example.com:80"));
    assert!(u.matches_pattern(b"example.com"));
    // a redirect to a location without a scheme or host
    assert_eq!(
        u.apply_redirect(b"relative/x", false, None),
        Err(ErrorCode::GENERIC)
    );
    assert_eq!(msg(), "cannot redirect from 'http' to '(null)'");
    let mut u = parsed(net::parse, "http://example.com/x");
    u.scheme = None;
    assert_eq!(
        u.apply_redirect(b"relative/x", false, None),
        Err(ErrorCode::GENERIC)
    );
    assert_eq!(msg(), "cannot redirect from 'example.com' to '(null)'");
    // a service suffix with a query, and no query in the URL
    let mut u = parsed(net::parse, "http://example.com/a/info/refs");
    u.apply_redirect(b"/b/info/refs", false, Some(b"/info/refs?service=x"))
        .unwrap();
    assert_eq!(s(&u.path), Some("/b/info/refs"));
}

#[test]
fn patterns_and_certs() {
    let _g = test_support::init();
    let u = parsed(net::parse, "https://foo.example.com/");
    assert!(u.matches_pattern(b"*.example.com"));
    assert!(u.matches_pattern(b".example.com:443"));
    assert!(!u.matches_pattern(b"example.com"));
    assert!(!u.matches_pattern(b"*.example.com:80"));
    assert!(!u.matches_pattern(b"ample.com"));
    assert!(u.matches_pattern_list(b"nope,,foo.example.com,"));
    assert!(!u.matches_pattern_list(b""));

    assert!(net::hostname_matches_cert(
        b"www.example.com",
        b"*.example.com"
    ));
    assert!(!net::hostname_matches_cert(
        b"a.b.example.com",
        b"*.example.com"
    ));
    assert!(net::hostname_matches_cert(
        b"WWW.example.com",
        b"w*.EXAMPLE.com"
    ));
    assert!(net::hostname_matches_cert(b"anything", b"*"));
    assert!(!net::hostname_matches_cert(
        b"example.com",
        b"*.example.com"
    ));
    assert!(!net::hostname_matches_cert(b"", b"x"));
    // as C on Unix: a pattern byte >= 0x80 never matches; a `*` still
    // spans host bytes >= 0x80
    assert!(!net::hostname_matches_cert(b"\x80", b"\x80"));
    assert!(!net::hostname_matches_cert(b"a\xc3", b"*\xc3"));
    assert!(net::hostname_matches_cert(b"\xc3\x80b", b"*b"));
}

#[test]
fn dispose_and_dup() {
    let _g = test_support::init();
    let u = parsed(net::parse, "http://u:p@h/x#frag");
    let mut d = GitNetUrl::new();
    d.dup_from(&u).unwrap();
    // `git_net_url_dup` copies neither the fragment nor port_specified.
    check(
        &d,
        [
            Some("http"),
            Some("h"),
            Some("80"),
            Some("/x"),
            None,
            None,
            Some("u"),
            Some("p"),
        ],
        0,
    );
    d.dispose();
    check(&d, [None; 8], 0);
}

#[test]
fn dup_and_parse_under_alloc_limit() {
    let g = test_support::exclusive();
    test_support::with_alloc_limit(&g, 0, || {
        let mut u = GitNetUrl::new();
        assert_eq!(net::parse(&mut u, b"http://h/x"), Err(ErrorCode::GENERIC));
        assert!(u.scheme.is_none());
        let mut u = GitNetUrl::new();
        assert_eq!(net::parse_scp(&mut u, b"h:x"), Err(ErrorCode::GENERIC));
        assert!(u.scheme.is_none());
    });
}

// ----- the exports -----

fn c(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

fn cs(p: &Option<libgit2_rs::__test::util::alloc::GitCStr>) -> Option<&CStr> {
    p.as_deref()
}

#[test]
fn exports_parse_uninitialized_and_dispose() {
    let _g = test_support::init();
    let mut raw = MaybeUninit::<GitNetUrl>::uninit();
    // Fill with garbage: the parser must not read it.
    // SAFETY: writing bytes into the uninitialized buffer.
    unsafe {
        raw.as_mut_ptr()
            .cast::<u8>()
            .write_bytes(0xab, size_of::<GitNetUrl>())
    };
    let url = c("ssh://git@example.com/repo");
    // SAFETY: a writable url and a string.
    unsafe {
        assert_eq!(git_net_url_parse(raw.as_mut_ptr(), url.as_ptr()), 0);
        let u = &*raw.as_ptr();
        assert_eq!(cs(&u.host).unwrap(), c"example.com");
        assert!(git_net_url_valid(raw.as_mut_ptr()));
        assert!(git_net_url_is_default_port(raw.as_mut_ptr()));
        assert!(!git_net_url_is_ipv6(raw.as_mut_ptr()));
        git_net_url_dispose(raw.as_mut_ptr());
        assert!((*raw.as_ptr()).host.is_none());

        // a failing parse leaves the url cleared
        raw.as_mut_ptr()
            .cast::<u8>()
            .write_bytes(0xcd, size_of::<GitNetUrl>());
        let bad = c("host");
        assert_eq!(git_net_url_parse_scp(raw.as_mut_ptr(), bad.as_ptr()), -12);
        assert!((*raw.as_ptr()).scheme.is_none() && (*raw.as_ptr()).path.is_none());
        git_net_url_dispose(raw.as_mut_ptr());
    }
}

#[test]
fn exports_alias_and_asserts() {
    let _g = test_support::init();
    let mut u = GitNetUrl::new();
    let mut b = GitStr::new();
    let s1 = c("http://u:p@example.com/a/b?q");
    let p = c("c?d");
    // SAFETY: valid urls and strings; aliasing is allowed for these.
    unsafe {
        assert_eq!(git_net_url_parse(&mut u, s1.as_ptr()), 0);
        // dup onto itself: copies, leaking the originals as C does; free
        // them here
        let old: Vec<*mut c_char> = [
            &u.scheme,
            &u.host,
            &u.port,
            &u.path,
            &u.query,
            &u.username,
            &u.password,
        ]
        .iter()
        .map(|f| f.as_ref().unwrap().as_ptr().cast_mut())
        .collect();
        let up: *mut GitNetUrl = &mut u;
        assert_eq!(git_net_url_dup(up, up), 0);
        for p in old {
            libgit2_rs::__test::util::alloc::free(p.cast());
        }
        assert_eq!(cs(&u.host).unwrap(), c"example.com");
        // joinpath onto itself: `out` is disposed first, so `one` is empty
        assert_eq!(git_net_url_joinpath(up, up, p.as_ptr()), 0);
        assert_eq!(cs(&u.path).unwrap(), c"/c");
        assert_eq!(cs(&u.query).unwrap(), c"d");
        assert!(u.host.is_none());
        git_net_url_swap(up, up);
        assert_eq!(cs(&u.path).unwrap(), c"/c");

        assert_eq!(git_net_url_fmt(&mut b, &mut u), -1);
        assert_eq!(msg(), "invalid argument: 'url->scheme'");
        assert_eq!(git_net_url_fmt(&mut b, core::ptr::null_mut()), -1);
        assert_eq!(msg(), "invalid argument: 'url'");
        let loc = c("/x");
        assert_eq!(
            git_net_url_apply_redirect(
                core::ptr::null_mut(),
                loc.as_ptr(),
                false,
                core::ptr::null()
            ),
            -1
        );
        assert_eq!(msg(), "unrecoverable internal error: 'url'");
        assert_eq!(
            git_net_url_apply_redirect(&mut u, core::ptr::null(), false, core::ptr::null()),
            -1
        );
        assert_eq!(msg(), "unrecoverable internal error: 'redirect_location'");
        assert!(!git_net_url_matches_pattern_list(
            &mut u,
            core::ptr::null::<c_char>()
        ));
        let h = c("www.example.com");
        let pat = c("*.example.com");
        assert!(git_net_hostname_matches_cert(h.as_ptr(), pat.as_ptr()));
        assert!(git_net_str_is_url(s1.as_ptr()));
        git_net_url_dispose(&mut u);
    }
}

#[test]
fn safe_url_api() {
    let _g = test_support::init();
    let mut u =
        libgit2_rs::__test::util::net::Url::parse(b"https://user@example.com/repo.git").unwrap();
    assert_eq!(u.host(), Some(&b"example.com"[..]));
    assert!(u.is_default_port() && !u.port_specified());
    u.set_credentials(Some(b"me"), Some(b"secret")).unwrap();
    assert_eq!(
        u.to_bytes().unwrap(),
        b"https://me:secret@example.com/repo.git"
    );
    let e = libgit2_rs::__test::util::net::Url::parse(b"http://[::1").unwrap_err();
    assert_eq!(e.code(), ErrorCode::INVALID_SPEC);
    assert_eq!(e.class(), ErrorClass::NET);
}
