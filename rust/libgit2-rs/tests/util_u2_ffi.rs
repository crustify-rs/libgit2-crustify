//! The C ABI of hash, zstream, regexp, wildmatch, date and rand, called as
//! C calls it.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_void};
use core::mem::MaybeUninit;

use libgit2_rs::__test::util::date::ffi as date_ffi;
use libgit2_rs::__test::util::errors::{self, ErrorClass, ErrorCode};
use libgit2_rs::__test::util::hash::{GitStrVec, HashCtx, ffi as hash_ffi, raw_algorithm};
use libgit2_rs::__test::util::rand::ffi as rand_ffi;
use libgit2_rs::__test::util::regexp::{RegMatch, ffi as re_ffi};
use libgit2_rs::__test::util::str::GitStr;
use libgit2_rs::__test::util::test_support;
use libgit2_rs::__test::util::wildmatch::ffi as wm_ffi;
use libgit2_rs::__test::util::zstream::{GitZstream, ZstreamKind, ffi as zs_ffi};

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn hash_ctx_as_c_uses_it() {
    let _g = test_support::init();
    // C embeds the context by value, zeroed (calloc) or not.
    let mut blob = MaybeUninit::<HashCtx>::zeroed();
    let ctx = blob.as_mut_ptr();
    let mut out = [0u8; 32];
    // SAFETY: `ctx` is a context-sized, aligned blob used by one thread;
    // the data and output buffers are live locals.
    unsafe {
        // A zeroed context has no algorithm.
        assert_eq!(hash_ffi::git_hash_update(ctx, b"x".as_ptr().cast(), 1), -1);
        assert_eq!(
            errors::last(),
            (ErrorClass::INTERNAL, b"unknown hash algorithm".to_vec())
        );
        hash_ffi::git_hash_ctx_cleanup(ctx);

        assert_eq!(hash_ffi::git_hash_ctx_init(ctx, raw_algorithm::SHA1), 0);
        assert_eq!(
            hash_ffi::git_hash_update(ctx, b"hello ".as_ptr().cast(), 6),
            0
        );
        assert_eq!(hash_ffi::git_hash_update(ctx, core::ptr::null(), 0), 0);
        assert_eq!(
            hash_ffi::git_hash_update(ctx, b"world".as_ptr().cast(), 5),
            0
        );
        assert_eq!(hash_ffi::git_hash_final(out.as_mut_ptr(), ctx), 0);
        assert_eq!(hex(&out[..20]), "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
        assert_eq!(hash_ffi::git_hash_init(ctx), 0);
        assert_eq!(hash_ffi::git_hash_final(out.as_mut_ptr(), ctx), 0);
        assert_eq!(hex(&out[..20]), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        hash_ffi::git_hash_ctx_cleanup(ctx);

        // SHA-256 is OpenSSL's: C, which Miri does not run.
        if !cfg!(miri) {
            assert_eq!(hash_ffi::git_hash_ctx_init(ctx, raw_algorithm::SHA256), 0);
            assert_eq!(hash_ffi::git_hash_update(ctx, b"abc".as_ptr().cast(), 3), 0);
            assert_eq!(hash_ffi::git_hash_final(out.as_mut_ptr(), ctx), 0);
            assert_eq!(
                hex(&out),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
            hash_ffi::git_hash_ctx_cleanup(ctx);
        }

        errors::clear();
        assert_eq!(hash_ffi::git_hash_ctx_init(ctx, raw_algorithm::NONE), -1);
        assert_eq!(
            errors::last(),
            (ErrorClass::INTERNAL, b"unknown hash algorithm".to_vec())
        );
        assert_eq!(hash_ffi::git_hash_init(ctx), -1);
        assert_eq!(hash_ffi::git_hash_final(out.as_mut_ptr(), ctx), -1);

        assert_eq!(
            hash_ffi::git_hash_ctx_init(core::ptr::null_mut(), raw_algorithm::SHA1),
            -1
        );
        assert_eq!(
            errors::last(),
            (ErrorClass::INVALID, b"invalid argument: 'ctx'".to_vec())
        );
    }
}

#[test]
fn hash_buf_vec_fmt() {
    let _g = test_support::init();
    let mut out = [0u8; 32];
    let mut s = [0 as c_char; 65];
    let vec = [
        GitStrVec {
            data: b"hello".as_ptr().cast_mut().cast::<c_void>(),
            len: 5,
        },
        GitStrVec {
            data: core::ptr::null_mut(),
            len: 0,
        },
        GitStrVec {
            data: b" world".as_ptr().cast_mut().cast::<c_void>(),
            len: 6,
        },
    ];
    // SAFETY: live local buffers of the sizes passed.
    unsafe {
        assert_eq!(
            hash_ffi::git_hash_buf(out.as_mut_ptr(), b"hello world".as_ptr().cast(), 11, 1),
            0
        );
        assert_eq!(hash_ffi::git_hash_fmt(s.as_mut_ptr(), out.as_ptr(), 20), 0);
        assert_eq!(
            CStr::from_ptr(s.as_ptr()).to_bytes(),
            b"2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
        out = [0; 32];
        assert_eq!(
            hash_ffi::git_hash_vec(out.as_mut_ptr(), vec.as_ptr(), 3, 1),
            0
        );
        assert_eq!(hex(&out[..20]), "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
        assert_eq!(out[20..], [0; 12]);
        if !cfg!(miri) {
            assert_eq!(
                hash_ffi::git_hash_vec(out.as_mut_ptr(), core::ptr::null(), 0, 2),
                0
            );
            assert_eq!(
                hex(&out),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
        }
        assert_eq!(
            hash_ffi::git_hash_buf(out.as_mut_ptr(), b"x".as_ptr().cast(), 1, 9),
            -1
        );
        assert_eq!(
            errors::last(),
            (ErrorClass::INTERNAL, b"unknown hash algorithm".to_vec())
        );
        assert_eq!(hash_ffi::git_hash_fmt(s.as_mut_ptr(), out.as_ptr(), 0), 0);
        assert_eq!(s[0], 0);
        assert_eq!(hash_ffi::git_hash_global_init(), 0);
    }
}

#[test]
fn zstream_as_c_uses_it() {
    let _g = test_support::init();
    // zlib.compress(b"hello world")
    let z: [u8; 19] = [
        0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x28, 0xcf, 0x2f, 0xca, 0x49, 0x01, 0x00,
        0x1a, 0x0b, 0x04, 0x5d,
    ];
    let mut zs = GitZstream::new();
    let mut out = [0u8; 4];
    // SAFETY: `zs` is a local stream that does not move between init and
    // free; the input and output buffers are live locals.
    unsafe {
        // Freeing GIT_ZSTREAM_INIT is harmless.
        zs_ffi::git_zstream_free(&mut zs);
        assert_eq!(
            zs_ffi::git_zstream_init(&mut zs, ZstreamKind::Inflate as c_int),
            0
        );
        assert_eq!(
            zs_ffi::git_zstream_set_input(&mut zs, z.as_ptr().cast(), z.len()),
            0
        );
        assert_eq!(zs_ffi::git_zstream_suggest_output_len(&mut zs), 19);
        let mut got = Vec::new();
        while !zs_ffi::git_zstream_done(&mut zs) {
            let mut n = out.len();
            assert_eq!(
                zs_ffi::git_zstream_get_output_chunk(out.as_mut_ptr(), &mut n, &mut zs),
                0
            );
            got.extend_from_slice(&out[..n]);
        }
        assert_eq!(got, b"hello world");
        assert!(zs_ffi::git_zstream_eos(&mut zs));
        assert_eq!(zs.in_len(), 0);
        // Reuse after a reset.
        zs_ffi::git_zstream_reset(&mut zs);
        assert_eq!(
            zs_ffi::git_zstream_set_input(&mut zs, z.as_ptr().cast(), 2),
            0
        );
        let mut big = [0u8; 64];
        let mut n = big.len();
        assert_eq!(
            zs_ffi::git_zstream_get_output(big.as_mut_ptr(), &mut n, &mut zs),
            -1
        );
        assert_eq!(
            errors::last(),
            (ErrorClass::ZLIB, b"premature end of zlib stream".to_vec())
        );
        zs_ffi::git_zstream_free(&mut zs);
        zs_ffi::git_zstream_free(&mut zs);

        let mut s = GitStr::new();
        assert_eq!(
            zs_ffi::git_zstream_inflatebuf(&mut s, z.as_ptr().cast(), z.len()),
            0
        );
        assert_eq!(s.as_bytes(), b"hello world");
    }
}

#[test]
#[cfg_attr(miri, ignore = "deflates with the system zlib")]
fn zstream_deflatebuf_as_c_uses_it() {
    let _g = test_support::init();
    let mut s = GitStr::new();
    let mut back = GitStr::new();
    // SAFETY: valid strings and buffers.
    unsafe {
        assert_eq!(
            zs_ffi::git_zstream_deflatebuf(&mut s, b"foobar!!".as_ptr().cast(), 8),
            0
        );
        // zlib's own bytes.
        assert_eq!(&s.as_bytes()[..2], [0x78, 0x9c]);
        assert_eq!(
            zs_ffi::git_zstream_inflatebuf(&mut back, s.as_ptr(), s.len()),
            0
        );
        assert_eq!(back.as_bytes(), b"foobar!!");
    }
}

#[test]
#[cfg_attr(miri, ignore = "PCRE2 is C")]
fn regexp_as_c_uses_it() {
    let _g = test_support::init();
    let mut r: re_ffi::GitRegexp = core::ptr::null_mut();
    let mut m = [RegMatch { start: 7, end: 7 }; 3];
    // SAFETY: `r` is a local `git_regexp`; strings are NUL-terminated;
    // `m` holds the entries passed.
    unsafe {
        assert_eq!(
            re_ffi::git_regexp_compile(&mut r, c"(a+)(x)?b".as_ptr(), 0),
            0
        );
        assert!(!r.is_null());
        assert_eq!(re_ffi::git_regexp_match(&r, c"zaab".as_ptr()), 0);
        assert_eq!(
            re_ffi::git_regexp_match(&r, c"zzz".as_ptr()),
            ErrorCode::NOT_FOUND.raw()
        );
        assert_eq!(
            re_ffi::git_regexp_search(&r, c"zaab".as_ptr(), 3, m.as_mut_ptr()),
            0
        );
        assert_eq!(
            m,
            [
                RegMatch { start: 1, end: 4 },
                RegMatch { start: 1, end: 3 },
                RegMatch { start: -1, end: -1 }
            ]
        );
        assert_eq!(
            re_ffi::git_regexp_search(&r, c"zaab".as_ptr(), 0, core::ptr::null_mut()),
            0
        );
        re_ffi::git_regexp_dispose(&mut r);
        assert!(r.is_null());
        re_ffi::git_regexp_dispose(&mut r);
        assert_eq!(
            re_ffi::git_regexp_match(&r, c"a".as_ptr()),
            ErrorCode::INVALID_SPEC.raw()
        );

        assert_eq!(
            re_ffi::git_regexp_compile(&mut r, c"[".as_ptr(), 1),
            ErrorCode::INVALID_SPEC.raw()
        );
        assert!(r.is_null());
        assert_eq!(
            errors::last(),
            (
                ErrorClass::REGEX,
                b"missing terminating ] for character class".to_vec()
            )
        );
    }
}

#[test]
fn wildmatch_as_c_uses_it() {
    // SAFETY: NUL-terminated strings.
    unsafe {
        assert_eq!(
            wm_ffi::wildmatch(c"foo/**/bar".as_ptr(), c"foo/a/b/bar".as_ptr(), 2),
            0
        );
        assert_eq!(
            wm_ffi::wildmatch(c"foo/*".as_ptr(), c"foo/a/b".as_ptr(), 2),
            1
        );
        assert_eq!(
            wm_ffi::wildmatch(c"[[:nope:]]".as_ptr(), c"a".as_ptr(), 0),
            -1
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "the system clock is C (libc)")]
fn date_as_c_uses_it() {
    let _g = test_support::init();
    let mut t: i64 = -7;
    let mut off: c_int = -7;
    let mut s = GitStr::new();
    // SAFETY: locals and NUL-terminated strings.
    unsafe {
        assert_eq!(
            date_ffi::git_date_offset_parse(&mut t, &mut off, c"@100 -0130".as_ptr()),
            0
        );
        assert_eq!((t, off), (100, -90));
        assert_eq!(
            date_ffi::git_date_parse(&mut t, c"1970-1-1 01:00:00+03".as_ptr()),
            0
        );
        assert_eq!(t, -7200);
        // approxidate: the offset is not written; on failure the time is.
        off = -7;
        assert_eq!(
            date_ffi::git_date_offset_parse(&mut t, &mut off, c"never".as_ptr()),
            0
        );
        assert_eq!((t, off), (0, -7));
        t = -7;
        assert_eq!(
            date_ffi::git_date_offset_parse(&mut t, &mut off, c"xyzzy".as_ptr()),
            -1
        );
        assert_ne!(t, -7);
        assert_eq!(date_ffi::git_date_rfc2822_fmt(&mut s, 1_397_031_663, 0), 0);
        assert_eq!(s.as_bytes(), b"Wed, 9 Apr 2014 08:21:03 +0000");
        assert_eq!(
            date_ffi::git_date_rfc2822_fmt(core::ptr::null_mut(), 0, 0),
            -1
        );
        assert_eq!(
            errors::last(),
            (ErrorClass::INVALID, b"invalid argument: 'out'".to_vec())
        );
    }
}

#[test]
fn rand_as_c_uses_it() {
    // SAFETY: no preconditions.
    unsafe {
        assert_eq!(rand_ffi::git_rand_global_init(), 0);
        rand_ffi::git_rand_seed(42);
        let a = rand_ffi::git_rand_next();
        rand_ffi::git_rand_seed(42);
        assert_eq!(rand_ffi::git_rand_next(), a);
    }
}
