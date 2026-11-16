//! Unit tests of the commit parser, the summary/body rules, header fields,
//! signature extraction and the commit buffer: pure Rust (run under Miri).

use super::*;
use crate::util::test_support;

const T: &str = "1810dff58d8a660512d4832e740f692884338ccd";

fn sha1() -> git_oid_t {
    OidType::Sha1.raw()
}

/// A zeroed commit, as `object.c` allocates one.
struct Fresh(*mut GitCommit);

impl Fresh {
    fn new() -> Fresh {
        let p = alloc::calloc(1, size_of::<GitCommit>()).cast::<GitCommit>();
        assert!(!p.is_null());
        Fresh(p)
    }
    fn get(&mut self) -> &mut GitCommit {
        // SAFETY: a zeroed commit (all-zero is a valid empty `GitCommit`),
        // ours alone.
        unsafe { &mut *self.0 }
    }
}

impl Drop for Fresh {
    fn drop(&mut self) {
        // SAFETY: the commit made in `new`, freed once.
        unsafe { ffi::git_commit__free(self.0.cast()) }
    }
}

fn cstr<'a>(p: *const c_char) -> Option<&'a [u8]> {
    // SAFETY: NULL or one of the parsed commit's strings, live during the
    // test.
    unsafe { opt_cbytes(p) }
}

fn sig_of(p: *const GitSignature) -> (Vec<u8>, Vec<u8>, GitTime) {
    assert!(!p.is_null());
    // SAFETY: a signature the parse allocated.
    unsafe {
        (
            cstr((*p).name).unwrap().to_vec(),
            cstr((*p).email).unwrap().to_vec(),
            (*p).when,
        )
    }
}

fn parse_full(data: &[u8]) -> (Result<()>, Fresh) {
    let mut c = Fresh::new();
    let r = parse_into(
        c.get(),
        data,
        &GitCommitParseOptions {
            oid_type: sha1(),
            flags: 0,
        },
    );
    (r, c)
}

#[test]
fn signatures() {
    let _g = test_support::init();
    let line = b"author  A U Thor  <a@b.c> 1234567890 +0130\nrest";
    let mut pos = 0;
    let s = parse_signature(line, &mut pos, b"author ").unwrap();
    assert_eq!(s.name, b"A U Thor");
    assert_eq!(s.email, b"a@b.c");
    assert_eq!(
        s.when,
        GitTime {
            time: 1234567890,
            offset: 90,
            sign: b'+' as c_char
        }
    );
    assert_eq!(&line[pos..], b"rest");

    let mut pos = 0;
    let s = parse_signature(b"c x <y> 5 -0100\n", &mut pos, b"c ").unwrap();
    assert_eq!((s.when.offset, s.when.sign), (-60, b'-' as c_char));

    // Out-of-range time zones are dropped, malformed ones are zero.
    for (l, off, sign) in [
        (&b"c x <y> 5 +1500\n"[..], 0, 0),
        (b"c x <y> 5 +0160\n", 0, 0),
        (b"c x <y> 5 x0100\n", 0, b'x'),
        (b"c x <y> 5 +ab\n", 0, b'+'),
        (b"c x <y>\n", 0, 0),
    ] {
        let mut pos = 0;
        let s = parse_signature(l, &mut pos, b"c ").unwrap();
        assert_eq!(
            (s.when.offset, s.when.sign),
            (off, sign as c_char),
            "{:?}",
            String::from_utf8_lossy(l)
        );
    }

    let errs: [(&[u8], &str); 5] = [
        (b"c x <y> 5", "failed to parse signature - no newline given"),
        (
            b"d x <y> 5\n",
            "failed to parse signature - expected prefix doesn't match actual",
        ),
        (
            b"c \n",
            "failed to parse signature - expected prefix doesn't match actual",
        ),
        (
            b"c x >y< 5\n",
            "failed to parse signature - malformed e-mail",
        ),
        (
            b"c x <y> zz\n",
            "failed to parse signature - invalid Unix timestamp",
        ),
    ];
    for (l, msg) in errs {
        let mut pos = 0;
        assert_eq!(
            parse_signature(l, &mut pos, b"c ").unwrap_err(),
            ErrorCode::INVALID
        );
        assert_eq!(errors::last_message().unwrap(), msg.as_bytes());
        assert_eq!(pos, 0);
    }
    // Crud is trimmed from both ends.
    let mut pos = 0;
    let s = parse_signature(b"c ,;:\"N\\'. <<'m'>> 1\n", &mut pos, b"c ").unwrap();
    assert_eq!((s.name, s.email), (&b"N\\'."[..], &b"m"[..]));
}

#[test]
fn full_parse() {
    let _g = test_support::init();
    let data = format!(
        "tree {T}\nparent {T}\nparent {T}\nauthor A <a@b> 1 +0000\nauthor B <b@b> 2 +0000\n\
         committer C <c@d> 3 -0130\nencoding ISO-8859-1\nencoding UTF-8\ngpgsig x\n y\n\n\nhello\n"
    );
    let (r, mut c) = parse_full(data.as_bytes());
    r.unwrap();
    let c = c.get();
    assert_eq!(
        c.tree_id,
        Oid::parse_hex(T.as_bytes(), OidType::Sha1).unwrap()
    );
    assert_eq!(c.parent_ids.len(), 2);
    assert_eq!(sig_of(c.author).0, b"A");
    let (n, e, w) = sig_of(c.committer);
    assert_eq!(
        (n.as_slice(), e.as_slice(), w.time, w.offset),
        (&b"C"[..], &b"c@d"[..], 3, -90)
    );
    assert_eq!(cstr(c.message_encoding).unwrap(), b"UTF-8");
    let header = cstr(c.raw_header).unwrap();
    assert!(header.ends_with(b"gpgsig x\n y\n"));
    assert_eq!(cstr(c.raw_message).unwrap(), b"\nhello\n");
    // SAFETY: a live commit.
    let m = unsafe { CStr::from_ptr(message_ptr(c)) };
    assert_eq!(m.to_bytes(), b"hello\n");
}

#[test]
fn parse_edges() {
    let _g = test_support::init();
    // No message at all: an empty one.
    let d = format!("tree {T}\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 3 +0000\n");
    let (r, mut c) = parse_full(d.as_bytes());
    r.unwrap();
    assert_eq!(cstr(c.get().raw_message).unwrap(), b"");
    assert_eq!(cstr(c.get().raw_header).unwrap(), d.as_bytes());

    // A NUL in the header cuts the raw header (strndup), as C.
    let d = format!("tree {T}\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 3 +0000\nx \0y\n\nmsg");
    let (r, mut c) = parse_full(d.as_bytes());
    r.unwrap();
    assert!(cstr(c.get().raw_header).unwrap().ends_with(b"x "));
    assert_eq!(cstr(c.get().raw_message).unwrap(), b"msg");

    // Bad trees.
    for d in [
        String::new(),
        format!("tree {}\n", &T[..39]),
        format!("tree {T}x"),
        format!("tree zz{}\n", &T[2..]),
    ] {
        let (r, _c) = parse_full(d.as_bytes());
        assert_eq!(r.unwrap_err(), ErrorCode::INVALID);
        assert_eq!(
            errors::last_message().unwrap(),
            b"failed to parse bad commit object"
        );
    }
    // A bad parent line ends the parents; the author is then expected.
    let d = format!("tree {T}\nparent zz\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 3 +0000\n");
    let (r, _c) = parse_full(d.as_bytes());
    assert_eq!(r.unwrap_err(), ErrorCode::INVALID);
    assert_eq!(
        errors::last_message().unwrap(),
        b"failed to parse signature - expected prefix doesn't match actual"
    );
}

#[test]
fn quick_parse_sink() {
    let _g = test_support::init();
    #[derive(Default)]
    struct S {
        parents: Vec<Oid>,
        time: i64,
        calls: Vec<&'static str>,
    }
    impl<'a> CommitSink<'a> for S {
        fn tree(&mut self, _: Oid) {
            self.calls.push("tree");
        }
        fn parent(&mut self, id: Oid) -> Result<()> {
            self.parents.push(id);
            Ok(())
        }
        fn author(&mut self, _: &SigFields<'a>) -> Result<()> {
            self.calls.push("author");
            Ok(())
        }
        fn committer(&mut self, s: &SigFields<'a>) -> Result<()> {
            self.time = s.when.time;
            Ok(())
        }
        fn encoding(&mut self, _: &'a [u8]) -> Result<()> {
            self.calls.push("encoding");
            Ok(())
        }
        fn header_and_message(&mut self, _: &'a [u8], _: Option<&'a [u8]>) -> Result<()> {
            self.calls.push("message");
            Ok(())
        }
    }
    // The tree is not checked by a quick parse, only skipped.
    let d = format!(
        "tree {}\nparent {T}\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 77 +0000\nencoding x\n\nm",
        "z".repeat(40)
    );
    let mut s = S::default();
    parse(d.as_bytes(), sha1(), true, &mut s).unwrap();
    assert_eq!((s.parents.len(), s.time), (1, 77));
    assert!(s.calls.is_empty());
    // But the authors are.
    let d = format!("tree {T}\nauthor A <a@b> x +0000\ncommitter C <c@d> 77 +0000\n");
    assert!(parse(d.as_bytes(), sha1(), true, &mut S::default()).is_err());
    // Too short to skip the tree.
    assert!(parse(b"tree 1810", sha1(), true, &mut S::default()).is_err());
    let mut s = S::default();
    let d =
        format!("tree {T}\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 77 +0000\nencoding x\n\nm");
    parse(d.as_bytes(), sha1(), false, &mut s).unwrap();
    assert_eq!(s.calls, ["tree", "author", "encoding", "message"]);
}

fn summary(m: &str) -> String {
    String::from_utf8(summary_of(message_of(m.as_bytes()))).unwrap()
}

fn body(m: &str) -> Option<String> {
    body_of(message_of(m.as_bytes())).map(|b| String::from_utf8(b.to_vec()).unwrap())
}

#[test]
fn summaries_and_bodies() {
    let _g = test_support::init();
    // The cases of tests/libgit2/commit/commit.c (summary, body).
    let cases: &[(&str, &str, Option<&str>)] = &[
        (
            "One-liner with no trailing newline",
            "One-liner with no trailing newline",
            None,
        ),
        (
            "One-liner with trailing newline\n",
            "One-liner with trailing newline",
            None,
        ),
        (
            "\n\nTrimmed leading&trailing newlines\n\n",
            "Trimmed leading&trailing newlines",
            None,
        ),
        (
            "First paragraph only\n\nSecond paragraph\n",
            "First paragraph only",
            Some("Second paragraph"),
        ),
        (
            "\n\nFirst paragraph only with surrounding newlines\n\nSecond paragraph\n\n",
            "First paragraph only with surrounding newlines",
            Some("Second paragraph"),
        ),
        (
            "First paragraph\nwith multiple lines\n\nSecond paragraph\n",
            "First paragraph with multiple lines",
            Some("Second paragraph"),
        ),
        (
            "First paragraph\nwith multiple lines\n\n",
            "First paragraph with multiple lines",
            None,
        ),
        (
            "\nFirst paragraph\nwith trailing whitespace  \n\n",
            "First paragraph with trailing whitespace",
            None,
        ),
        ("  Leading whitespace\n\n", "  Leading whitespace", None),
        ("Tab\tseparated\n\n", "Tab\tseparated", None),
        (
            "trailing spaces and tabs   \t\n\nbody",
            "trailing spaces and tabs",
            Some("body"),
        ),
        ("line one\n   \nline two", "line one", None),
        ("line one\n\t\r\nline two", "line one", None),
        ("a  \n  b", "a b", None),
        ("", "", None),
        ("\n\n\n", "", None),
        ("   ", "", None),
        (
            "Summary\n\n  body with leading and trailing spaces  \n\n",
            "Summary",
            Some("body with leading and trailing spaces"),
        ),
        (
            "Summary\n\nPara 1\n\nPara 2\n",
            "Summary",
            Some("Para 1\n\nPara 2"),
        ),
    ];
    for &(m, s, b) in cases {
        assert_eq!(summary(m), s, "summary of {m:?}");
        assert_eq!(body(m).as_deref(), b, "body of {m:?}");
    }
}

#[test]
fn header_fields() {
    let _g = test_support::init();
    let h = format!(
        "tree {T}\nauthor A <a@b> 1 +0000\ngpgsig -----BEGIN\n line2\n \n end\nx y\nxx z\n"
    );
    let get = |f: &str| {
        let mut out = GitStr::new();
        header_field(h.as_bytes(), f.as_bytes(), &mut out)
            .map(|()| String::from_utf8(out.as_bytes().to_vec()).unwrap())
    };
    assert_eq!(get("tree").unwrap(), T);
    assert_eq!(get("gpgsig").unwrap(), "-----BEGIN\nline2\n\nend");
    assert_eq!(get("x").unwrap(), "y");
    assert_eq!(get("xx").unwrap(), "z");
    assert_eq!(get("autho").unwrap_err(), ErrorCode::NOT_FOUND);
    assert_eq!(errors::last_message().unwrap(), b"no such field 'autho'");
    assert_eq!(get("line2").unwrap_err(), ErrorCode::NOT_FOUND);
    // A continuation without its end.
    let mut out = GitStr::new();
    assert_eq!(
        header_field(b"a b\n c", b"a", &mut out).unwrap_err(),
        ErrorCode::GENERIC
    );
    assert_eq!(errors::last_message().unwrap(), b"malformed header");
    // An empty field, or one starting with SP, matches no line (C skips
    // continuation lines before comparing).
    assert_eq!(get("").unwrap_err(), ErrorCode::NOT_FOUND);
    assert_eq!(errors::last_message().unwrap(), b"no such field ''");
    assert_eq!(get(" line2").unwrap_err(), ErrorCode::NOT_FOUND);
    // The header is read as a C string: up to its first NUL.
    let mut out = GitStr::new();
    assert_eq!(
        header_field(b"a 1\n\0b 2\n", b"b", &mut out).unwrap_err(),
        ErrorCode::NOT_FOUND
    );
    header_field(b"a 1\nb 2\0\n", b"a", &mut out).unwrap();
    assert_eq!(out.as_bytes(), b"1");
    // A field with a newline matching across lines (C: a negative length).
    assert!(header_field(b"a\nb c\n", b"a\nb", &mut out).is_err());
}

#[test]
fn signature_extraction() {
    let _g = test_support::init();
    let data = format!("tree {T}\ngpgsig -----BEGIN\n mid\n -----END\nauthor A\n\nmsg\n");
    let (mut sig, mut signed) = (GitStr::new(), GitStr::new());
    extract_signature(data.as_bytes(), b"gpgsig", &mut sig, &mut signed).unwrap();
    assert_eq!(sig.as_bytes(), b"-----BEGIN\nmid\n-----END");
    assert_eq!(
        signed.as_bytes(),
        format!("tree {T}\nauthor A\n\nmsg\n").as_bytes()
    );

    let (mut sig, mut signed) = (GitStr::new(), GitStr::new());
    assert_eq!(
        extract_signature(b"tree x\n\nmsg\n", b"gpgsig", &mut sig, &mut signed).unwrap_err(),
        ErrorCode::NOT_FOUND
    );
    assert_eq!(
        errors::last_message().unwrap(),
        b"this commit is not signed"
    );

    // A field name prefix of another header is skipped, and its name dropped
    // from the signed data, as C.
    let (mut sig, mut signed) = (GitStr::new(), GitStr::new());
    extract_signature(
        b"gpgsigx a\ngpgsig s\nz\n",
        b"gpgsig",
        &mut sig,
        &mut signed,
    )
    .unwrap();
    assert_eq!(sig.as_bytes(), b"s");
    assert_eq!(signed.as_bytes(), b"x a\nz\n");
}

#[test]
fn buffers() {
    let _g = test_support::init();
    let tree = Oid::parse_hex(T.as_bytes(), OidType::Sha1).unwrap();
    let a = SigOut {
        name: Some(b"A U"),
        email: Some(b"a@b"),
        when: GitTime {
            time: 1,
            offset: -90,
            sign: 0,
        },
    };
    let c = SigOut {
        name: None,
        email: Some(b"c"),
        when: GitTime {
            time: -1,
            offset: 60,
            sign: b'-' as c_char,
        },
    };
    let mut out = GitStr::new();
    create_buffer_internal(
        &mut out,
        &a,
        &c,
        b"msg",
        &tree,
        &[tree],
        Some(b"UTF-8"),
        &[(b"x", b"1\n2")],
    )
    .unwrap();
    let expect = format!(
        "tree {T}\nparent {T}\nauthor A U <a@b> 1 -0130\ncommitter (null) <c> 4294967295 -0100\nencoding UTF-8\nx 1\n 2\n\nmsg"
    );
    assert_eq!(out.as_bytes(), expect.as_bytes());

    let mut out2 = GitStr::new();
    append_header(&mut out2, out.as_bytes(), b"gpgsig", b"s1\ns2").unwrap();
    assert!(String::from_utf8_lossy(out2.as_bytes()).contains("x 1\n 2\ngpgsig s1\n s2\n\nmsg"));
    let mut out3 = GitStr::new();
    assert!(append_header(&mut out3, b"no header end", b"a", b"b").is_err());
    assert_eq!(
        errors::last_message().unwrap(),
        b"malformed commit contents"
    );

    // An id of unknown type fails, and the buffer is disposed.
    let mut out = GitStr::new();
    let bad = Oid::default();
    assert!(create_buffer_internal(&mut out, &a, &a, b"m", &bad, &[], None, &[]).is_err());
    assert!(out.is_empty());
}

#[test]
fn c_abi_parse_and_accessors() {
    let _g = test_support::init();
    let data = format!(
        "tree {T}\nparent {T}\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 3 -0130\n\n\nSum\nmary\n\nBody\n"
    );
    let c = Fresh::new();
    // SAFETY: a fresh commit and readable data.
    unsafe {
        assert_eq!(
            ffi::git_commit__parse_raw(c.0.cast(), data.as_ptr().cast(), data.len(), sha1()),
            0
        );
        assert_eq!(ffi::git_commit_parentcount(c.0), 1);
        assert_eq!(ffi::git_commit_time(c.0), 3);
        assert_eq!(ffi::git_commit_time_offset(c.0), -90);
        assert_eq!(
            *ffi::git_commit_tree_id(c.0),
            Oid::parse_hex(T.as_bytes(), OidType::Sha1).unwrap()
        );
        assert!(ffi::git_commit_parent_id(c.0, 1).is_null());
        assert_eq!(
            CStr::from_ptr(ffi::git_commit_message(c.0)).to_bytes(),
            b"Sum\nmary\n\nBody\n"
        );
        let s = ffi::git_commit_summary(c.0);
        assert_eq!(CStr::from_ptr(s).to_bytes(), b"Sum mary");
        assert_eq!(ffi::git_commit_summary(c.0), s, "kept in the commit");
        assert_eq!(
            CStr::from_ptr(ffi::git_commit_body(c.0)).to_bytes(),
            b"Body"
        );
        let mut out = GitStr::new();
        assert_eq!(
            ffi::git_commit__header_field(Some(&mut out), c.0, c"author".as_ptr()),
            0
        );
        assert_eq!(out.as_bytes(), b"A <a@b> 1 +0000");
        assert!(ffi::git_commit_author(core::ptr::null()).is_null());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid argument: 'commit'"
        );
        assert_eq!(ffi::git_commit_time(core::ptr::null()), i64::MIN);
    }
    // A failed parse leaves a commit that frees cleanly.
    let mut c = Fresh::new();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            ffi::git_commit__parse_raw(c.0.cast(), b"tree x".as_ptr().cast(), 6, sha1()),
            ErrorCode::INVALID.raw()
        )
    };
    let _ = c.get();
}

#[test]
fn options_init() {
    let _g = test_support::init();
    let mut o = core::mem::MaybeUninit::<GitCommitCreateOptions>::uninit();
    // SAFETY: writable.
    unsafe {
        assert_eq!(ffi::git_commit_create_options_init(o.as_mut_ptr(), 7), 0);
        let o = o.assume_init();
        assert_eq!(
            (o.version, o.flags.into_inner(), o.extra_headers_len),
            (7, 0, 0)
        );
        assert!(o.sign.is_none());
        assert_eq!(
            ffi::git_commit_create_ext_options_init(core::ptr::null_mut(), 1),
            -1
        );
    }
}

#[test]
fn commitarray_dispose_empty() {
    let _g = test_support::init();
    let mut a = crate::commit::deps::GitCommitArray {
        commits: core::ptr::null(),
        count: 0,
    };
    // SAFETY: an empty array.
    unsafe {
        ffi::git_commitarray_dispose(&raw mut a);
        ffi::git_commitarray_dispose(core::ptr::null_mut());
    }
    assert!(a.commits.is_null());
}

#[test]
fn review_r1_edges() {
    let _g = test_support::init();
    // LC-5: an empty field is never found (the C loops forever).
    let (mut sig, mut signed) = (GitStr::new(), GitStr::new());
    assert_eq!(
        extract_signature(b"tree x\nauthor a\n\nmsg\n", b"", &mut sig, &mut signed).unwrap_err(),
        ErrorCode::NOT_FOUND
    );
    // LC-3: INT_MIN offsets are written as the C build does.
    let mut out = GitStr::new();
    let when = GitTime {
        time: 0,
        offset: i32::MIN,
        sign: 0,
    };
    write_signature(&mut out, b"author ", Some(b"N"), Some(b"e"), when);
    assert_eq!(out.as_bytes(), b"author N <e> 0 -3579139408\n");
}

/// `header_field_cstr` (the C ABI's line walk) and `header_field` (the
/// whole-buffer search) agree on result, contents and error message.
#[test]
fn header_field_line_walk_matches_slices() {
    let _g = test_support::init();
    let headers: &[&[u8]] = &[
        b"",
        b"\n",
        b"tree 1\nparent 2\nparent 3\nauthor A\ncommitter C\n",
        b"tree 1\nmergetag object x\n type commit\n tag v1\n\n -----BEGIN\nauthor A\n",
        b"a b\n c",
        b"a b\n c\n",
        b"a b\n\n",
        b"a  b\nab c\na\nb c",
        b" x y\nx z\n",
        b"gpgsig -----BEGIN\n line2\n \n end\nx y\nxx z\n",
        b"k v\n k2\n",
        b"k\n",
        b"k \n",
    ];
    let fields: &[&[u8]] = &[
        b"",
        b"tree",
        b"parent",
        b"author",
        b"committer",
        b"mergetag",
        b"a",
        b"ab",
        b"b",
        b"x",
        b"xx",
        b" x",
        b"gpgsig",
        b"k",
        b"k ",
        b"a\nb",
        b"b\na",
        b"line2",
        b"zz",
    ];
    for &h in headers {
        let c = std::ffi::CString::new(h).unwrap();
        for &f in fields {
            let (mut a, mut b) = (GitStr::new(), GitStr::new());
            let ra = header_field(h, f, &mut a);
            let ma = errors::last_message();
            errors::clear();
            let rb = header_field_cstr(&c, f, &mut b);
            let mb = errors::last_message();
            errors::clear();
            assert_eq!(ra, rb, "{h:?} {f:?}");
            assert_eq!(ma, mb, "{h:?} {f:?}");
            assert_eq!(a.as_bytes(), b.as_bytes(), "{h:?} {f:?}");
        }
    }
}
