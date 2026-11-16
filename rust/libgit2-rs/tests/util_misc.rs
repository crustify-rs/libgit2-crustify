//! Tests of the allocator, runtime, util helpers, `git_buf` and
//! `git_strarray` exports, called as C calls them.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_void};
use libgit2_rs::buf::{GitBuf, ffi as buf_ffi};
use libgit2_rs::strarray::{GitStrArray, StrArray, ffi as sa_ffi};
use libgit2_rs::util::alloc::{self, GitAllocator, ffi as alloc_ffi};
use libgit2_rs::util::allocators::{failalloc, stdalloc};
use libgit2_rs::util::errors::{self, ErrorClass};
use libgit2_rs::util::str::GitStr;
use libgit2_rs::util::test_support;
use libgit2_rs::util::util::ffi as util_ffi;

#[test]
fn alloc_exports() {
    let _g = test_support::init();
    // SAFETY: as C; each allocation is freed once with `git__free`.
    unsafe {
        let p = alloc_ffi::git__strdup(c"dup".as_ptr());
        assert_eq!(CStr::from_ptr(p), c"dup");
        alloc::free(p.cast());
        let p = alloc_ffi::git__strndup(c"abcdef".as_ptr(), 3);
        assert_eq!(CStr::from_ptr(p), c"abc");
        alloc::free(p.cast());
        let p = alloc_ffi::git__substrdup(c"abcdef".as_ptr(), 4);
        assert_eq!(CStr::from_ptr(p), c"abcd");
        alloc::free(p.cast());
        let p = alloc_ffi::git__calloc(3, 5).cast::<u8>();
        assert_eq!(core::slice::from_raw_parts(p, 15), [0; 15]);
        let p = alloc_ffi::git__reallocarray(p.cast(), 4, 8).cast::<u8>();
        assert!(!p.is_null());
        assert_eq!(core::slice::from_raw_parts(p, 15), [0; 15]);
        alloc::free(p.cast());
        errors::clear();
        assert!(alloc_ffi::git__mallocarray(usize::MAX, 2).is_null());
        assert_eq!(errors::last_class(), Some(ErrorClass::NOMEMORY));
    }
}

#[test]
fn allocator_init_functions() {
    let mut a = GitAllocator {
        gmalloc: None,
        grealloc: None,
        gfree: None,
    };
    // SAFETY: as C: fill an allocator struct, and use the allocators'
    // functions as their contract says.
    unsafe {
        assert_eq!(stdalloc::git_stdalloc_init_allocator(&mut a), 0);
        let p = a.gmalloc.unwrap()(8, core::ptr::null(), 0);
        assert!(!p.is_null());
        let p = a.grealloc.unwrap()(p, 16, core::ptr::null(), 0);
        a.gfree.unwrap()(p);
        assert!(failalloc::git_failalloc_malloc(1, core::ptr::null(), 0).is_null());
        assert!(failalloc::git_failalloc_realloc(p, 1, core::ptr::null(), 0).is_null());
        failalloc::git_failalloc_free(core::ptr::null_mut());
    }
}

#[test]
fn allocator_setup_and_limit() {
    let g = test_support::exclusive();
    // Initialization keeps an allocator installed before it (C compares
    // `gmalloc` with the failing one).
    assert!(alloc::global_init().is_ok());
    let before = alloc::current();
    assert!(before.gmalloc.is_some());
    test_support::with_alloc_limit(&g, 3, || {
        let mut p: *mut c_void = core::ptr::null_mut();
        // SAFETY: as tests/util/alloc.c's `realloc` test.
        unsafe {
            p = alloc::realloc(p, 1);
            assert!(!p.is_null());
            p.cast::<u8>().write(b'x');
            p = alloc::realloc(p, 1);
            assert_eq!(p.cast::<u8>().read(), b'x');
            p = alloc::realloc(p, 2);
            assert_eq!(p.cast::<u8>().read(), b'x');
            assert!(alloc::realloc(p, 2).is_null());
            p = alloc::realloc(p, 1);
            assert_eq!(p.cast::<u8>().read(), b'x');
            alloc::free(p);
        }
        assert!(alloc::strdup(c"test").is_null());
        assert!(alloc::calloc(1, 4).is_null());
    });
    // SAFETY: no allocation is in progress (exclusive guard).
    unsafe { assert_eq!(alloc_ffi::git_allocator_setup(core::ptr::null_mut()), 0) };
    assert_eq!(
        alloc::current().gmalloc.map(|f| f as usize),
        before.gmalloc.map(|f| f as usize)
    );
}

#[test]
fn runtime_exports() {
    let _g = test_support::init();
    // SAFETY: reads the count only.
    let n = unsafe { libgit2_rs::util::runtime::ffi::git_runtime_init_count() };
    assert!(n >= 1);
}

#[test]
fn util_exports() {
    let _g = test_support::init();
    let mut v: i64 = 0;
    let mut end: *const c_char = core::ptr::null();
    let s = c"  0x1fz";
    // SAFETY: as C.
    unsafe {
        assert_eq!(
            util_ffi::git__strntol64(&mut v, s.as_ptr(), 7, &mut end, 0),
            0
        );
        assert_eq!((v, end.offset_from(s.as_ptr())), (31, 6));
        let big = c"99999999999999999999";
        end = core::ptr::null();
        assert_eq!(
            util_ffi::git__strntol64(&mut v, big.as_ptr(), 20, &mut end, 10),
            -1
        );
        assert_eq!(
            end.offset_from(big.as_ptr()),
            20,
            "endptr is set on overflow"
        );
        end = core::ptr::null();
        assert_eq!(
            util_ffi::git__strntol64(&mut v, c"x".as_ptr(), 1, &mut end, 10),
            -1
        );
        assert!(end.is_null(), "endptr is not set when there is no number");
        let mut v32 = 0i32;
        assert_eq!(
            util_ffi::git__strntol32(&mut v32, c"123".as_ptr(), 2, core::ptr::null_mut(), 10),
            0
        );
        assert_eq!(v32, 12);

        assert_eq!(
            util_ffi::git__prefixcmp(c"refs/heads".as_ptr(), c"refs/".as_ptr()),
            0
        );
        assert!(util_ffi::git__prefixcmp(c"ref".as_ptr(), c"refs/".as_ptr()) < 0);
        assert_eq!(
            util_ffi::git__prefixncmp(c"refsXYZ".as_ptr(), 4, c"refs".as_ptr()),
            0
        );
        assert_eq!(
            util_ffi::git__suffixcmp(c"a.txt".as_ptr(), c".txt".as_ptr()),
            0
        );
        assert_eq!(
            util_ffi::git__strcasecmp(c"ABC".as_ptr(), c"abc".as_ptr()),
            0
        );
        assert_eq!(
            util_ffi::git__strncasecmp(c"ABCx".as_ptr(), c"abcy".as_ptr(), 3),
            0
        );

        let mut b = *b"Mixed CASE\0";
        util_ffi::git__strtolower(b.as_mut_ptr().cast());
        assert_eq!(&b, b"mixed case\0");
        let mut e = *b"a\\,b\0";
        assert_eq!(util_ffi::git__unescape(e.as_mut_ptr().cast()), 3);
        assert_eq!(&e[..4], b"a,b\0");
        assert_eq!(util_ffi::git__unescape(core::ptr::null_mut()), 0);

        let mut t = *b"a b\0";
        let mut p = t.as_mut_ptr().cast::<c_char>();
        let tok = util_ffi::git__strtok(&mut p, c" ".as_ptr());
        assert_eq!(CStr::from_ptr(tok), c"a");
        let tok = util_ffi::git__strtok(&mut p, c" ".as_ptr());
        assert_eq!(CStr::from_ptr(tok), c"b");
        assert!(util_ffi::git__strtok(&mut p, c" ".as_ptr()).is_null());

        assert_eq!(util_ffi::git__linenlen(c"ab\ncd".as_ptr(), 5), 3);
        let h = b"hello";
        let f = util_ffi::git__memmem(h.as_ptr().cast(), 5, b"ll".as_ptr().cast(), 2);
        assert_eq!(f.cast::<u8>().offset_from(h.as_ptr()), 2);
        assert_eq!(
            util_ffi::git__hash(b"abc".as_ptr().cast(), 3, 0),
            0x723a_6130
        );

        let mut out = 7;
        assert_eq!(util_ffi::git__parse_bool(&mut out, c"off".as_ptr()), 0);
        assert_eq!(out, 0);
        assert_eq!(util_ffi::git__parse_bool(&mut out, c"nope".as_ptr()), -1);

        unsafe extern "C" fn cmp(k: *const c_void, e: *const c_void) -> c_int {
            (k.addr() as isize - e.addr() as isize).signum() as c_int
        }
        let mut arr: Vec<*mut c_void> = [1usize, 3, 5]
            .iter()
            .map(|&i| core::ptr::without_provenance_mut(i))
            .collect();
        let mut pos = 0;
        let key = core::ptr::without_provenance(4);
        assert_eq!(
            util_ffi::git__bsearch(arr.as_mut_ptr(), 3, key, cmp, &mut pos),
            -3
        );
        assert_eq!(pos, 2);
    }
}

#[test]
#[cfg_attr(miri, ignore = "sysconf and getenv reach the C library")]
fn util_system() {
    let _g = test_support::init();
    assert!(libgit2_rs::util::util::online_cpus() >= 1);
    let mut s = GitStr::new();
    assert!(libgit2_rs::util::util::getenv(&mut s, c"PATH").is_ok());
    assert!(!s.is_empty());
    assert_eq!(
        libgit2_rs::util::util::getenv(&mut s, c"LIBGIT2_RS_SURELY_UNSET"),
        Err(libgit2_rs::ErrorCode::NOT_FOUND)
    );
    assert!(s.is_empty());
}

#[test]
fn buf_exports() {
    let _g = test_support::init();
    let mut buf = GitBuf::new();
    let mut s = GitStr::from_bytes(b"contents").unwrap();
    // SAFETY: as C (`GIT_BUF_WRAP_PRIVATE`'s sequence).
    unsafe {
        assert_eq!(buf_ffi::git_buf_fromstr(&mut buf, s.as_mut_ptr()), 0);
        assert!(s.is_empty());
        assert_eq!(buf.as_bytes(), b"contents");
        let mut t = core::mem::MaybeUninit::<GitStr>::uninit();
        assert_eq!(buf_ffi::git_buf_tostr(t.as_mut_ptr(), &mut buf), 0);
        let mut t = t.assume_init();
        assert!(
            t.is_empty() && t.capacity() > 0,
            "the allocation is reused, emptied"
        );
        t.puts(b"again").unwrap();
        assert_eq!(buf_ffi::git_buf_fromstr(&mut buf, t.as_mut_ptr()), 0);
        assert_eq!(buf_ffi::git_buf_sanitize(&mut buf), 0);
        assert!(buf.as_bytes().is_empty());
        buf_ffi::git_buf_dispose(&mut buf);
        buf_ffi::git_buf_dispose(core::ptr::null_mut());
        assert_eq!(buf_ffi::git_buf_sanitize(core::ptr::null_mut()), -1);
    }
    // GIT_BUF_INIT is { NULL, 0, 0 }.
    let mut zero = core::mem::MaybeUninit::<GitBuf>::zeroed();
    // SAFETY: an all-zero git_buf is GIT_BUF_INIT.
    unsafe {
        assert_eq!(buf_ffi::git_buf_sanitize(zero.as_mut_ptr()), 0);
        buf_ffi::git_buf_dispose(zero.as_mut_ptr());
        zero.assume_init_drop();
    }
}

#[test]
fn strarray_api() {
    let _g = test_support::init();
    let a = StrArray::from_cstrs(&[c"a", c"bb"]).unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(a.get(1), Some(c"bb"));
    let raw: GitStrArray = a.into_raw();
    let mut copy = GitStrArray {
        strings: core::ptr::null_mut(),
        count: 0,
    };
    // SAFETY: as C.
    unsafe {
        assert_eq!(sa_ffi::git_strarray_copy(&mut copy, &raw), 0);
        let mut raw = raw;
        sa_ffi::git_strarray_dispose(&mut raw);
        assert_eq!(raw.count, 0);
        let owned = StrArray::from_raw(copy);
        assert_eq!(owned.to_vec(), [b"a".to_vec(), b"bb".to_vec()]);
        assert_eq!(sa_ffi::git_strarray_copy(core::ptr::null_mut(), &copy), -1);
        sa_ffi::git_strarray_dispose(core::ptr::null_mut());
    }
}

#[test]
fn strarray_copy_oom() {
    let g = test_support::exclusive();
    let src = StrArray::from_cstrs(&[c"first", c"second"]).unwrap();
    let raw = src.into_raw();
    test_support::with_alloc_limit(&g, 16 + 6, || {
        let mut tgt = GitStrArray {
            strings: core::ptr::null_mut(),
            count: 5,
        };
        // SAFETY: as C; the second strdup fails.
        unsafe {
            assert_eq!(sa_ffi::git_strarray_copy(&mut tgt, &raw), -1);
        }
        assert!(tgt.strings.is_null() && tgt.count == 0);
        assert_eq!(errors::last_class(), Some(ErrorClass::NOMEMORY));
    });
    // SAFETY: `raw` came from `into_raw`.
    drop(unsafe { StrArray::from_raw(raw) });
}
