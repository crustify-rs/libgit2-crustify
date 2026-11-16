//! Tests of `git_str`: the safe API, and the exports called as C calls them.
//! Expected values follow the C (`tests/util/str/*.c` and the C reference).

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char};
use libgit2_rs::__test::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::__test::util::str::{Bom, GitStr, ffi};
use libgit2_rs::__test::util::test_support;

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

#[test]
fn growth_matches_c() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    assert_eq!(s.capacity(), 0);
    let mut caps = vec![];
    for _ in 0..64 {
        s.puts(b"abcdefg").unwrap();
        caps.push(s.capacity());
    }
    caps.dedup();
    // C: first allocation rounded to 8, then x1.5 rounded up to 8.
    assert_eq!(caps, [8, 16, 24, 40, 64, 96, 144, 216, 328, 496]);
    assert_eq!(s.len(), 7 * 64);
    assert_eq!(s.as_c_str().unwrap().to_bytes().len(), 7 * 64);
}

#[test]
fn set_put_clear_truncate() {
    let _g = test_support::init();
    let mut s = GitStr::from_bytes(b"hello").unwrap();
    s.putc(b' ').unwrap();
    s.putcn(b'x', 3).unwrap();
    s.put(b"!").unwrap();
    assert_eq!(s.as_bytes(), b"hello xxx!");
    s.truncate(5);
    assert_eq!(s.as_c_str().unwrap(), c"hello");
    s.shorten(2);
    assert_eq!(s.as_bytes(), b"hel");
    s.shorten(10);
    assert!(s.is_empty());
    s.set(b"a/b/c//").unwrap();
    s.rtruncate_at_char(b'/');
    assert_eq!(s.as_bytes(), b"a/b");
    s.truncate_at_char(b'/');
    assert_eq!(s.as_bytes(), b"a");
    s.set(b"  x \t\n").unwrap();
    s.rtrim();
    assert_eq!(s.as_bytes(), b"  x");
    s.clear();
    assert_eq!(s.as_c_str().unwrap(), c"");
    assert!(s.capacity() > 0);
    s.set(b"").unwrap();
    assert!(s.is_empty());
}

#[test]
fn find_and_consume() {
    let _g = test_support::init();
    let mut s = GitStr::from_bytes(b"abcabc").unwrap();
    assert_eq!(s.find(b'c'), Some(2));
    assert_eq!(s.rfind(b'a'), Some(3));
    assert_eq!(s.rfind_next(b'c'), Some(2));
    assert_eq!(s.find(b'z'), None);
    s.consume_bytes(2);
    assert_eq!(s.as_bytes(), b"cabc");
    s.consume_bytes(0);
    s.consume_bytes(100);
    assert_eq!(s.as_bytes(), b"cabc");
}

#[test]
fn joins() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    s.join(b'/', Some(b"a/"), b"//b").unwrap();
    assert_eq!(s.as_bytes(), b"a/b");
    s.join(b'/', None, b"/b").unwrap();
    assert_eq!(s.as_bytes(), b"/b");
    s.joinpath(b"", b"x").unwrap();
    assert_eq!(s.as_bytes(), b"x");
    s.joinpath_self(b"y").unwrap();
    s.joinpath_self(b"/z").unwrap();
    assert_eq!(s.as_bytes(), b"x/y/z");
    s.join3(b'/', b"a/", b"/b/", b"/c").unwrap();
    assert_eq!(s.as_bytes(), b"a/b/c");
    s.join3(b'/', b"", b"", b"/c").unwrap();
    assert_eq!(s.as_bytes(), b"/c");

    // join_n, as tests/util/str/basic.c's `check_joinbuf_n_2` / `_4`.
    let mut s = GitStr::new();
    s.join_n(b'/', &[Some(b"" as &[u8]), Some(b"a")]).unwrap();
    assert_eq!(s.as_bytes(), b"a");
    let mut s = GitStr::from_bytes(b"a").unwrap();
    s.join_n(b'/', &[Some(b"/b" as &[u8]), Some(b"c/"), None, Some(b"d")])
        .unwrap();
    assert_eq!(s.as_bytes(), b"a/b/c/d");
    // Empty segments into an empty string write nothing (C only writes a
    // separator after something).
    let mut s = GitStr::new();
    s.join_n(b';', &[Some(b"" as &[u8]), Some(b""), Some(b"")])
        .unwrap();
    assert_eq!(s.as_bytes(), b"");
    let mut s = GitStr::from_bytes(b"a").unwrap();
    s.join_n(b';', &[Some(b"" as &[u8]), Some(b""), Some(b"b")])
        .unwrap();
    assert_eq!(s.as_bytes(), b"a;b");
    let mut s = GitStr::new();
    s.join_n(b'/', &[]).unwrap();
    assert!(s.is_empty());
}

#[test]
fn encodings() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    s.encode_base64(b"this").unwrap();
    assert_eq!(s.as_bytes(), b"dGhpcw==");
    let mut d = GitStr::new();
    d.decode_base64(s.as_bytes()).unwrap();
    // C decodes each `=` as a 0 byte: the string is "this\0\0".
    assert_eq!(d.as_bytes(), b"this\0\0");
    assert_eq!(d.as_c_str().unwrap(), c"this");
    assert_eq!(d.decode_base64(b"abc"), Err(ErrorCode::GENERIC));
    assert_eq!(msg(), "invalid base64 input");
    assert_eq!(d.decode_base64(b"ab!d"), Err(ErrorCode::GENERIC));
    assert_eq!(d.as_bytes(), b"this\0\0", "unchanged on error");

    for input in [
        &b""[..],
        b"a",
        b"ab",
        b"abc",
        b"abcd",
        b"\x00\xff\x10\x20\x30\x40\x50",
    ] {
        let mut e = GitStr::new();
        e.encode_base85(input).unwrap();
        assert_eq!(e.len(), input.len().div_ceil(4) * 5);
        let mut d = GitStr::new();
        d.decode_base85(e.as_bytes(), input.len()).unwrap();
        assert_eq!(d.as_bytes(), input);
    }
    let mut e = GitStr::new();
    e.encode_base85(b"this is base 85 encoded").unwrap();
    assert_eq!(e.as_bytes(), b"bZBXFAZc?TVqtS-AUHK3Wo~0{WMyOk");
    let mut d = GitStr::new();
    assert!(d.decode_base85(b"abcd", 1).is_err());
    assert_eq!(msg(), "invalid base85 input");
    assert!(d.decode_base85(b"~~~~~", 4).is_err(), "overflow");

    let mut h = GitStr::new();
    h.encode_hexstr(b"\x01\xab").unwrap();
    assert_eq!(h.as_bytes(), b"01ab");

    let mut p = GitStr::new();
    p.decode_percent(b"a%20b%2%zz%41").unwrap();
    assert_eq!(p.as_bytes(), b"a b%2%zzA");
}

#[test]
fn quote_unquote() {
    let _g = test_support::init();
    let cases: &[(&[u8], &[u8])] = &[
        (b"noquote", b"noquote"),
        (b"foo bar", b"foo bar"),
        (b"foo\"bar", b"\"foo\\\"bar\""),
        (b"foo\\bar", b"\"foo\\\\bar\""),
        (b"foo\tbar", b"\"foo\\tbar\""),
        (b"\x01\x7f\xe2\x98\x83", b"\"\\001\\177\\342\\230\\203\""),
        (b"!bang", b"\"!bang\""),
    ];
    for (input, quoted) in cases {
        let mut s = GitStr::from_bytes(input).unwrap();
        s.quote().unwrap();
        assert_eq!(s.as_bytes(), *quoted, "quote {:?}", errors::Bytes(input));
        if quoted != input {
            s.unquote().unwrap();
            assert_eq!(s.as_bytes(), *input);
        }
    }
    for (bad, m) in [
        (&b"\"foo"[..], "invalid quoted line"),
        (b"foo\"", "invalid quoted line"),
        (b"\"foo\\\"", "invalid quoted line"),
        (b"\"\\q\"", "invalid quoted character \\q"),
        // C reads the NUL after the contents: a NUL in the message.
        (b"\"\\0\"", "truncated quoted character \\0\"\0"),
        (b"\"\\01\"", "truncated quoted character \\0"),
        (b"\"\\08x\"", "truncated quoted character \\08x"),
    ] {
        let mut s = GitStr::from_bytes(bad).unwrap();
        assert_eq!(s.unquote(), Err(ErrorCode::GENERIC));
        assert_eq!(msg(), m);
    }
}

#[test]
fn escapes_splice_crlf() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    s.puts_escaped(b"a.b*c", b".*", b"[", b"]").unwrap();
    assert_eq!(s.as_bytes(), b"a[.]b[*]c");
    let mut s = GitStr::new();
    s.puts_escape_regex(b"^a.b$").unwrap();
    assert_eq!(s.as_bytes(), b"\\^a\\.b\\$");
    let mut s = GitStr::from_bytes(b"a\\b\\\\c").unwrap();
    s.unescape();
    assert_eq!(s.as_bytes(), b"ab\\c");

    let mut s = GitStr::from_bytes(b"hello world").unwrap();
    s.splice(6, 5, b"there, friend").unwrap();
    assert_eq!(s.as_bytes(), b"hello there, friend");
    s.splice(0, 6, b"").unwrap();
    assert_eq!(s.as_bytes(), b"there, friend");
    assert!(s.splice(100, 0, b"x").is_err());
    assert_eq!(msg(), "unrecoverable internal error: 'where <= buf->size'");
    assert_eq!(errors::last_class(), Some(ErrorClass::INTERNAL));

    let mut t = GitStr::new();
    t.crlf_to_lf(b"a\r\nb\rc\r\n\r").unwrap();
    assert_eq!(t.as_bytes(), b"a\nb\rc\n\r");
    t.lf_to_crlf(b"a\nb\r\nc\n").unwrap();
    assert_eq!(t.as_bytes(), b"a\r\nb\r\nc\r\n");
    t.lf_to_crlf(b"no newline").unwrap();
    assert_eq!(t.as_bytes(), b"no newline");

    let mut p = GitStr::new();
    p.common_prefix(&[b"refs/heads/a", b"refs/heads/b", b"refs/tags"])
        .unwrap();
    assert_eq!(p.as_bytes(), b"refs/");
    p.common_prefix(&[b"abc", b"xyz"]).unwrap();
    assert!(p.is_empty());
}

#[test]
fn text_stats() {
    let _g = test_support::init();
    let b = GitStr::borrowed(b"\xef\xbb\xbfhello\r\nworld\n");
    assert_eq!(b.detect_bom(), (Bom::Utf8, 3));
    assert!(!b.is_binary());
    let (st, bin) = b.gather_text_stats(true);
    assert_eq!((st.cr, st.lf, st.crlf, st.printable), (1, 2, 1, 10));
    assert!(!bin);
    assert!(GitStr::borrowed(b"a\0b").is_binary());
    assert!(GitStr::borrowed(b"a\0b").contains_nul());
    assert_eq!(
        GitStr::borrowed(b"\xff\xfe\0\0").detect_bom(),
        (Bom::Utf32Le, 4)
    );
    assert_eq!(
        GitStr::borrowed(b"\xff\xfea").detect_bom(),
        (Bom::Utf16Le, 2)
    );
    assert!(GitStr::borrowed(b"\xfe\xff").is_binary());
    assert!(GitStr::borrowed(b"bare\rcr").gather_text_stats(false).1);
}

#[test]
fn borrowed_and_attached() {
    let _g = test_support::init();
    let data = b"borrowed!";
    let b = GitStr::borrowed(data);
    assert_eq!(b.as_bytes(), data);
    assert_eq!(b.as_c_str(), None);
    assert_eq!(b.capacity(), 0);

    let mut s = GitStr::new();
    // SAFETY: `data` outlives `s`'s borrow (until `dispose` below).
    unsafe { s.attach_notowned(data.as_ptr().cast(), data.len()) };
    assert_eq!(s.grow(100), Err(ErrorCode::INVALID));
    assert_eq!(msg(), "cannot grow a borrowed buffer");
    assert_eq!(
        s.put(b"x"),
        Err(ErrorCode::GENERIC),
        "ENSURE_SIZE maps the error to -1"
    );
    s.consume_bytes(3);
    assert_eq!(s.as_bytes(), b"rowed!");
    assert_eq!(data, b"borrowed!", "borrowed memory is not written");
    assert!(s.unquote().is_err());
    s.dispose();

    let mut s = GitStr::from_bytes(b"detach me").unwrap();
    let d = s.detach().unwrap();
    assert_eq!(&*d, c"detach me");
    assert!(s.is_empty() && s.capacity() == 0);
    assert!(s.detach().is_none());
    let p = d.into_raw();
    // SAFETY: `p` is a `git__allocator` string, adopted by the string.
    unsafe { s.attach(p, 0).unwrap() };
    assert_eq!(s.as_bytes(), b"detach me");
    assert_eq!(s.capacity(), 10);
}

#[test]
fn oom_marks_the_string() {
    let g = test_support::exclusive();
    test_support::with_alloc_limit(&g, 100, || {
        let mut s = GitStr::new();
        s.grow(42).unwrap();
        assert!(!s.is_oom());
        assert_eq!(s.grow(101), Err(ErrorCode::GENERIC));
        assert!(s.is_oom());
        assert_eq!(errors::last_class(), Some(ErrorClass::NOMEMORY));
        assert_eq!(s.puts(b"x"), Err(ErrorCode::GENERIC));
        assert_eq!(s.as_bytes(), b"");
        s.dispose();
        assert!(!s.is_oom());
    });
    test_support::with_alloc_limit(&g, 10, || {
        let mut s = GitStr::new();
        s.puts(b"foobar").unwrap();
        assert!(s.puts(b"foobar").is_err());
    });
    let mut s = GitStr::new();
    assert!(s.grow_by(usize::MAX).is_err());
    assert!(s.is_oom());
}

#[test]
fn fmt_write() {
    use core::fmt::Write;
    let _g = test_support::init();
    let mut s = GitStr::new();
    let a = "a";
    write!(s, "{a}/{:08x}", 255).unwrap();
    let seven = 7;
    s.printf(3, format_args!("-{seven}")).unwrap();
    assert_eq!(s.as_bytes(), b"a/000000ff-7");
    // C's growth: room for 2 * strlen(format) first, then exactly enough.
    let mut s = GitStr::new();
    s.printf(2, format_args!("{}", "x".repeat(30))).unwrap();
    assert_eq!((s.len(), s.capacity()), (30, 32));
    let mut s = GitStr::new();
    let n = 12345;
    s.printf(4, format_args!("{n}")).unwrap();
    assert_eq!((s.len(), s.capacity()), (5, 8));
}

// ---------------------------------------------------------------------------
// The exports, as C calls them
// ---------------------------------------------------------------------------

fn bytes(s: &GitStr) -> &[u8] {
    s.as_bytes()
}

#[test]
fn ffi_init_dispose_and_states() {
    let _g = test_support::init();
    let mut s = core::mem::MaybeUninit::<GitStr>::uninit();
    // SAFETY: as C: init, use, dispose a git_str on the stack.
    unsafe {
        assert_eq!(ffi::git_str_init(s.as_mut_ptr(), 0), 0);
        let p = s.as_mut_ptr();
        assert_eq!((*p).capacity(), 0);
        assert_eq!(ffi::git_str_puts(p, c"hello".as_ptr()), 0);
        assert_eq!(ffi::git_str_put(p, c" world".as_ptr(), 6), 0);
        assert_eq!(bytes(&*p), b"hello world");
        assert_eq!(ffi::git_str_put(p, core::ptr::null(), 0), 0);
        assert_eq!(ffi::git_str_put(p, core::ptr::null(), 1), -1);
        assert_eq!(msg(), "invalid argument: 'data'");
        assert_eq!(ffi::git_str_puts(p, core::ptr::null()), -1);
        assert_eq!(msg(), "invalid argument: 'string'");
        ffi::git_str_dispose(p);
        assert_eq!((*p).capacity(), 0);
        ffi::git_str_dispose(core::ptr::null_mut());
        s.assume_init_drop();
    }
    // A zeroed git_str (NULL ptr), as C's calloc'd structs hold.
    let mut z = core::mem::MaybeUninit::<GitStr>::zeroed();
    // SAFETY: an all-zero git_str is the empty state.
    unsafe {
        let p = z.as_mut_ptr();
        ffi::git_str_clear(p);
        assert_eq!(ffi::git_str_sets(p, c"x".as_ptr()), 0);
        assert_eq!(bytes(&*p), b"x");
        z.assume_init_drop();
    }
}

#[test]
fn ffi_aliasing_arguments() {
    let _g = test_support::init();
    let mut s = GitStr::from_bytes(b"abc").unwrap();
    let p = s.as_mut_ptr();
    // SAFETY: as C, with data pointing into the buffer itself.
    unsafe {
        // Appending the buffer to itself, through growth.
        for _ in 0..6 {
            let (d, n) = ((*p).as_ptr(), (*p).len());
            assert_eq!(ffi::git_str_put(p, d, n), 0);
        }
        assert_eq!((*p).len(), 3 * 64);
        assert!(bytes(&*p).chunks(3).all(|c| c == b"abc"));
        // set from inside.
        let d = (*p).as_ptr().add(3);
        assert_eq!(ffi::git_str_set(p, d.cast(), 4), 0);
        assert_eq!(bytes(&*p), b"abca");
        // set from ptr itself: a truncation.
        assert_eq!(ffi::git_str_set(p, (*p).as_ptr().cast(), 2), 0);
        assert_eq!(bytes(&*p), b"ab");
        // join with str_a inside (the joinpath idiom), and at an offset.
        assert_eq!(
            ffi::git_str_join(p, b'/' as c_char, (*p).as_ptr(), c"c".as_ptr()),
            0
        );
        assert_eq!(bytes(&*p), b"ab/c");
        assert_eq!(
            ffi::git_str_join(p, b'/' as c_char, (*p).as_ptr().add(1), c"d".as_ptr()),
            0
        );
        assert_eq!(bytes(&*p), b"b/c/d");
        // str_b inside is refused.
        assert_eq!(
            ffi::git_str_join(p, b'/' as c_char, c"x".as_ptr(), (*p).as_ptr()),
            -1
        );
        assert_eq!(
            msg(),
            "invalid argument: 'str_b < buf->ptr || str_b >= buf->ptr + buf->size'"
        );
        assert_eq!(
            ffi::git_str_join3(
                p,
                b'/' as c_char,
                (*p).as_ptr(),
                c"x".as_ptr(),
                c"y".as_ptr()
            ),
            -1
        );
        assert_eq!(
            msg(),
            "unrecoverable internal error: 'str_a < buf->ptr || str_a >= buf->ptr + buf->size'"
        );
        // join_n with a segment inside: C uses the rest of the original contents.
        let segs = [(*p).as_ptr().add(2), c"e".as_ptr()];
        assert_eq!(ffi::git_str__join_n(p, b'/' as c_char, 2, segs.as_ptr()), 0);
        assert_eq!(bytes(&*p), b"b/c/d/c/d/e");
        // splice with data inside.
        let d = (*p).as_ptr();
        assert_eq!(ffi::git_str_splice(p, 0, 0, d, 4), 0);
        assert_eq!(bytes(&*p), b"b/c/b/c/d/c/d/e");
    }
}

#[test]
fn ffi_misc() {
    let _g = test_support::init();
    let mut s = GitStr::from_bytes(b"0123456789").unwrap();
    let p = s.as_mut_ptr();
    let mut out = [0x55u8; 4];
    // SAFETY: as C.
    unsafe {
        assert_eq!(ffi::git_str_copy_cstr(out.as_mut_ptr().cast(), 4, p), 0);
        assert_eq!(&out, b"012\0");
        assert_eq!(ffi::git_str_copy_cstr(out.as_mut_ptr().cast(), 0, p), -1);
        assert_eq!(msg(), "invalid argument: 'datasize'");
        ffi::git_str_consume(p, (*p).as_ptr().add(4));
        assert_eq!(bytes(&*p), b"456789");
        ffi::git_str_consume(p, (*p).as_ptr());
        assert_eq!(bytes(&*p), b"456789");
        let d = ffi::git_str_detach(p);
        assert_eq!(CStr::from_ptr(d), c"456789");
        assert_eq!(ffi::git_str_attach(p, d, 0), 0);
        assert_eq!((*p).capacity(), 7);
        let mut other = GitStr::from_bytes(b"other").unwrap();
        ffi::git_str_swap(p, other.as_mut_ptr());
        assert_eq!(bytes(&*p), b"other");
        // `memcmp`'s sign (glibc's magnitude varies by implementation).
        assert!(ffi::git_str_cmp(p, other.as_mut_ptr()) > 0);
        assert_eq!(ffi::git_str_cmp(p, p), 0);
        ffi::git_str_swap(p, p);
        let t = GitStr::from_bytes(b"x\r\ny").unwrap();
        assert_eq!(ffi::git_str_crlf_to_lf(p, &t), 0);
        assert_eq!(bytes(&*p), b"x\ny");
        assert_eq!(ffi::git_str_crlf_to_lf(p, p), -1);
        assert_eq!(msg(), "unrecoverable internal error: 'tgt != src'");
        let strs = [c"abc".as_ptr(), c"abd".as_ptr()];
        assert_eq!(ffi::git_str_common_prefix(p, strs.as_ptr(), 2), 0);
        assert_eq!(bytes(&*p), b"ab");
        let mut bom = Bom::Utf8;
        assert_eq!(ffi::git_str_detect_bom(&mut bom, p), 0);
        assert_eq!(bom, Bom::None);
        assert_eq!(ffi::git_str_is_binary(p), 0);
        assert_eq!(ffi::git_str_grow_by(p, 100), 0);
        assert!((*p).capacity() >= 102);
        assert_eq!(ffi::git_str_try_grow(p, 1000, false), 0);
        assert_eq!((*p).capacity(), 1000);
    }
}

#[test]
fn ffi_vprintf_protocol() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    let p = s.as_mut_ptr();
    // SAFETY: as the C shim drives it: begin, then report what vsnprintf
    // wrote.
    unsafe {
        assert_eq!(ffi::git_str__vprintf_begin(p, 3), 0);
        assert_eq!((*p).capacity(), 8);
        // Pretend vsnprintf wanted 20 bytes: grow and retry.
        assert_eq!(ffi::git_str__vprintf_written(p, 20), 1);
        assert!((*p).capacity() >= 21);
        // Pretend it wrote 0 bytes (and the NUL).
        assert_eq!(ffi::git_str__vprintf_written(p, 0), 0);
        assert_eq!(ffi::git_str__vprintf_written(p, -1), -1);
        assert!((*p).is_oom());
        (*p).dispose();
    }
}
