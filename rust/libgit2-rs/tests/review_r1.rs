//! Regression tests for review round 1 (`/work/reviews/foundation-r1.md`):
//! C-contract states, cross-type conversions, self-aliasing arguments,
//! panics during formatting, over-reads. Each test names its finding.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char};
use libgit2_rs::__test::buf::{GitBuf, ffi as buf_ffi};
use libgit2_rs::__test::util::errors::{self, ErrorClass, ffi as err_ffi};
use libgit2_rs::__test::util::str::{GitStr, ffi as str_ffi};
use libgit2_rs::__test::util::test_support;
use libgit2_rs::__test::util::util::ffi as util_ffi;

fn c_last() -> (i32, Vec<u8>) {
    // SAFETY: as C: read the returned error before the next error call.
    unsafe {
        let e = err_ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_bytes().to_vec())
    }
}

/// A `git_str` in each state C may hand over.
fn states(data: &'static [u8; 3]) -> Vec<(&'static str, GitStr)> {
    let mut v = vec![("initstr", GitStr::new())];
    // SAFETY: an all-zero `git_str` is the empty state (NULL ptr).
    v.push(("null", unsafe {
        GitStr::from_raw_parts(core::ptr::null_mut(), 0, 0)
    }));
    v.push(("owned", GitStr::from_bytes(b"owned").unwrap()));
    let mut b = GitStr::new();
    // SAFETY: `data` is static.
    unsafe { b.attach_notowned(data.as_ptr().cast(), 3) };
    v.push(("borrowed", b));
    let mut c = GitStr::new();
    // SAFETY: as above; then cleared: empty, `ptr` still foreign.
    unsafe { c.attach_notowned(data.as_ptr().cast(), 3) };
    c.clear();
    v.push(("borrowed-cleared", c));
    let mut d = GitStr::new();
    // SAFETY: as above; consumed entirely: empty, `ptr` moved past `data`.
    unsafe { d.attach_notowned(data.as_ptr().cast(), 3) };
    d.consume_bytes(3);
    v.push(("borrowed-consumed", d));
    let mut o = GitStr::from_bytes(b"hello").unwrap();
    assert!(o.grow(usize::MAX).is_err());
    assert!(o.is_oom());
    v.push(("oom", o));
    v
}

static DATA: [u8; 3] = *b"xyz";

/// F-1, F-2, F-12: every state through `GitBuf::from_git_str`, and through
/// `git_buf_fromstr` + `git_buf_tostr`; nothing is freed that is not owned
/// (Miri checks the frees), and an out-of-memory string stays so.
#[test]
fn str_buf_state_matrix() {
    let _g = test_support::init();
    for (name, s) in states(&DATA) {
        let oom = s.is_oom();
        let contents = s.as_bytes().to_vec();
        match GitBuf::from_git_str(s) {
            Ok(b) => {
                assert!(!oom, "{name}");
                assert_eq!(b.as_bytes(), contents, "{name}");
                drop(b);
            }
            Err(_) => assert!(oom, "{name}"),
        }
    }
    for (name, mut s) in states(&DATA) {
        let oom = s.is_oom();
        let owned = s.owns_buffer();
        let mut b = GitBuf::new();
        let mut t = core::mem::MaybeUninit::<GitStr>::uninit();
        // SAFETY: as C's `GIT_BUF_WRAP_PRIVATE`: valid structs throughout.
        unsafe {
            assert_eq!(
                buf_ffi::git_buf_fromstr(&mut b, s.as_mut_ptr()),
                0,
                "{name}"
            );
            assert_eq!(buf_ffi::git_buf_tostr(t.as_mut_ptr(), &mut b), 0, "{name}");
            let mut t = t.assume_init();
            assert_eq!(t.is_oom(), oom, "{name}");
            if oom {
                // As C: every later operation fails; nothing is freed.
                assert_eq!(
                    str_ffi::git_str_puts(t.as_mut_ptr(), c"x".as_ptr()),
                    -1,
                    "{name}"
                );
            } else {
                assert_eq!(t.owns_buffer(), owned, "{name}");
                assert_eq!(
                    str_ffi::git_str_puts(t.as_mut_ptr(), c"x".as_ptr()),
                    0,
                    "{name}"
                );
                assert_eq!(t.as_bytes(), b"x", "{name}");
            }
            str_ffi::git_str_dispose(t.as_mut_ptr());
            drop(t);
        }
        // `git_buf_tostr` left `b` empty: dropping it frees nothing.
        assert!(b.as_bytes().is_empty());
    }
}

/// F-1: the reviewer's sequence (t2.c).
#[test]
fn oom_through_buf_stays_oom() {
    let _g = test_support::init();
    let mut s = GitStr::from_bytes(b"hello").unwrap();
    assert!(s.grow(usize::MAX).is_err());
    let mut b = GitBuf::new();
    let mut t = core::mem::MaybeUninit::<GitStr>::uninit();
    // SAFETY: as C.
    unsafe {
        buf_ffi::git_buf_fromstr(&mut b, s.as_mut_ptr());
        buf_ffi::git_buf_tostr(t.as_mut_ptr(), &mut b);
        let mut t = t.assume_init();
        assert!(t.is_oom());
        assert_eq!(str_ffi::git_str_puts(t.as_mut_ptr(), c"x".as_ptr()), -1);
        str_ffi::git_str_dispose(t.as_mut_ptr());
    }
}

/// F-3: `git_error_set_str(c, git_error_last()->message)` empties the
/// message, as C (no aliasing UB under Miri).
#[test]
fn set_str_with_own_message() {
    let _g = test_support::init();
    // SAFETY: as C.
    unsafe {
        assert_eq!(
            err_ffi::git_error_set_str(3, c"original message".as_ptr()),
            0
        );
        let m = (*err_ffi::git_error_last()).message;
        assert_eq!(err_ffi::git_error_set_str(7, m), 0);
        assert_eq!(c_last(), (7, Vec::new()));
        // A pointer into the middle of the buffer, too.
        err_ffi::git_error_set_str(3, c"original message".as_ptr());
        let m = (*err_ffi::git_error_last()).message.add(4);
        assert_eq!(err_ffi::git_error_set_str(5, m), 0);
        assert_eq!(c_last(), (5, Vec::new()));
    }
}

/// F-12: restoring static errors and saved ones; the saved copy is
/// independent of the thread's buffer.
#[test]
fn restore_static_and_saved() {
    let _g = test_support::init();
    let mut e = core::ptr::null_mut();
    // SAFETY: as C.
    unsafe {
        err_ffi::git_error_set_oom();
        assert_eq!(err_ffi::git_error_save(&mut e), 0);
        assert_eq!(err_ffi::git_error_restore(e), 0);
        assert_eq!(c_last().0, ErrorClass::NOMEMORY.0);
        err_ffi::git_error_set_str(3, c"saved".as_ptr());
        assert_eq!(err_ffi::git_error_save(&mut e), 0);
        err_ffi::git_error_set_str(3, c"something else, longer, to grow the buffer".as_ptr());
        assert_eq!(err_ffi::git_error_restore(e), 0);
        assert_eq!(c_last(), (3, b"saved".to_vec()));
    }
}

/// F-7: a panic in an argument's `Display` leaves C a valid error.
#[test]
fn panic_in_display_leaves_a_valid_error() {
    struct Boom;
    impl core::fmt::Display for Boom {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(&"x".repeat(4096))?;
            panic!("boom")
        }
    }
    let _g = test_support::init();
    let _ = errors::set_str(ErrorClass::INVALID, "short");
    let r =
        std::panic::catch_unwind(|| errors::set_fmt(ErrorClass::INVALID, format_args!("{}", Boom)));
    assert!(r.is_err());
    // Readable (Miri checks), and consistent: no error.
    assert_eq!(c_last(), (0, b"no error".to_vec()));
    assert!(!errors::exists());
    // The state works afterwards.
    let _ = errors::set_str(ErrorClass::NET, "after");
    assert_eq!(c_last(), (12, b"after".to_vec()));
}

/// F-8: `git__prefixncmp`'s result for a prefix byte >= 0x80 past `str_n`
/// (C's signed `char`).
#[test]
fn prefixncmp_signed_char() {
    // SAFETY: valid strings, as C.
    unsafe {
        let p = c"ab\xC3".as_ptr();
        assert_eq!(util_ffi::git__prefixncmp(c"ab".as_ptr(), 2, p), 61);
        assert_eq!(util_ffi::git__prefixncmp_icase(c"AB".as_ptr(), 2, p), 61);
        assert_eq!(
            util_ffi::git__prefixncmp(c"ab".as_ptr(), 2, c"abc".as_ptr()),
            -99
        );
    }
}

/// F-9: `git__substrdup` with a length no allocation can have fails as
/// out of memory without forming a slice.
#[test]
fn substrdup_huge() {
    let _g = test_support::init();
    errors::clear();
    // SAFETY: the length is refused before anything is read.
    let p =
        unsafe { libgit2_rs::__test::util::alloc::ffi::git__substrdup(c"x".as_ptr(), usize::MAX) };
    assert!(p.is_null());
    assert_eq!(errors::last_class(), Some(ErrorClass::NOMEMORY));
}

/// F-10: memory may cross the limited allocator's boundary both ways.
#[test]
fn alloc_limit_boundary_crossing() {
    let g = test_support::exclusive();
    let before = GitStr::from_bytes(b"allocated before").unwrap();
    let mut grown = GitStr::from_bytes(b"grown inside").unwrap();
    let inside = test_support::with_alloc_limit(&g, 1000, || {
        drop(before);
        grown.puts(&[b'x'; 200]).unwrap();
        GitStr::from_bytes(b"allocated inside").unwrap()
    });
    assert_eq!(inside.as_bytes(), b"allocated inside");
    drop(inside);
    drop(grown);
}

/// F-14: `git_str_copy_cstr` into the string's own contents needs no
/// allocation (and matches C's `data[0] = 0` first).
#[test]
fn copy_cstr_overlapping_without_allocation() {
    let g = test_support::exclusive();
    let mut s = GitStr::from_bytes(b"abcdef").unwrap();
    test_support::with_alloc_limit(&g, 0, || {
        let p = s.as_mut_ptr();
        // SAFETY: as C: `data` is the string's own buffer, writable for 4.
        unsafe {
            let data = (*p).as_ptr().cast_mut().add(2);
            assert_eq!(str_ffi::git_str_copy_cstr(data, 4, p), 0);
            // C writes `data[0] = 0` (contents "ab\0def"), then moves 3
            // bytes ("ab\0") to offset 2 and adds a NUL: "abab".
            assert_eq!(CStr::from_ptr((*p).as_ptr()).to_bytes(), b"abab");
        }
    });
}

/// F-17: `git_str_join_n` with more than 8 segments.
#[test]
fn join_n_many_segments() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    let segs: Vec<*const c_char> = (0..20).map(|_| c"a".as_ptr()).collect();
    // SAFETY: 20 NUL-terminated segments.
    unsafe {
        assert_eq!(
            str_ffi::git_str__join_n(s.as_mut_ptr(), b'/' as c_char, 20, segs.as_ptr()),
            0
        )
    };
    assert_eq!(
        s.as_bytes(),
        "a/".repeat(19)
            .as_bytes()
            .iter()
            .chain(b"a")
            .copied()
            .collect::<Vec<_>>()
    );
    let parts: Vec<Option<&[u8]>> = (0..12).map(|_| Some(&b"b"[..])).collect();
    let mut t = GitStr::new();
    t.join_n(b'-', &parts).unwrap();
    assert_eq!(t.len(), 23);
}

/// F-4, F-12: counted inputs are read no further than C reads them: each
/// string ends right before an inaccessible page.
#[test]
#[cfg_attr(miri, ignore = "mmap with PROT_NONE")]
fn page_edge_reads() {
    let _g = test_support::init();
    // SAFETY: a private anonymous mapping of two pages; the second is made
    // inaccessible; the mapping is unmapped at the end.
    unsafe {
        let page = libc::sysconf(libc::_SC_PAGESIZE) as usize;
        let m = libc::mmap(
            core::ptr::null_mut(),
            2 * page,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        assert_ne!(m, libc::MAP_FAILED);
        assert_eq!(
            libc::mprotect(m.cast::<u8>().add(page).cast(), page, libc::PROT_NONE),
            0
        );
        let end = m.cast::<u8>().add(page);
        let at_edge = |b: &[u8]| {
            let p = end.sub(b.len());
            p.copy_from(b.as_ptr(), b.len());
            p.cast::<c_char>()
        };
        let a = at_edge(b"abc");
        assert_eq!(util_ffi::git__strncasecmp(a, c"ABC".as_ptr(), 3), 0);
        assert_eq!(util_ffi::git__strncasecmp(c"ABC".as_ptr(), a, 3), 0);
        let a = at_edge(b"ab");
        assert_eq!(util_ffi::git__prefixncmp(a, 2, c"abc".as_ptr()), -99);
        // C's last return uses the prefix byte as is (not lowered).
        assert_eq!(util_ffi::git__prefixncmp_icase(a, 2, c"ABC".as_ptr()), -67);
        let n = at_edge(b"1234");
        let mut v = 0i64;
        assert_eq!(
            util_ffi::git__strntol64(&mut v, n, 4, core::ptr::null_mut(), 10),
            0
        );
        assert_eq!(v, 1234);
        let mut s = GitStr::new();
        assert_eq!(str_ffi::git_str_put(s.as_mut_ptr(), n, 4), 0);
        assert_eq!(str_ffi::git_str_set(s.as_mut_ptr(), n.cast(), 4), 0);
        assert_eq!(str_ffi::git_str_decode_percent(s.as_mut_ptr(), n, 4), 0);
        assert_eq!(util_ffi::git__linenlen(n, 4), 4);
        assert!(!util_ffi::git__memmem(n.cast(), 4, c"34".as_ptr().cast(), 2).is_null());
        libc::munmap(m, 2 * page);
    }
}
