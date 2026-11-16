//! The exports of `delta.c`, called as C calls them.

#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::c_void;
use core::ptr;

use libgit2_rs::__test::delta::{self, ffi};
use libgit2_rs::__test::util::{alloc, errors, test_support};

fn msg() -> Vec<u8> {
    errors::last_message().unwrap_or_default()
}

#[test]
fn apply_and_create() {
    let _g = test_support::init();
    let src: Vec<u8> = (0..5000u32).map(|i| (i % 97) as u8).collect();
    let mut trg = src.clone();
    trg[1000] = 1;
    trg.extend_from_slice(b"appended");
    // SAFETY: C's calls: valid out-parameters and buffers; the index's
    // source outlives it; outputs are freed with git__free.
    unsafe {
        let mut index = ptr::null_mut();
        assert_eq!(
            ffi::git_delta_index_init(&mut index, src.as_ptr().cast(), src.len()),
            0
        );
        assert!(!index.is_null());
        let entries = (src.len() - 1) / 16;
        let mut h = 4;
        while h < 31 && (1usize << h) < entries / 4 {
            h += 1;
        }
        assert_eq!(
            ffi::git_delta_index_size(index),
            32 + entries * 24 + (1 << h) * 8
        );
        let (mut out, mut out_len) = (ptr::null_mut::<c_void>(), 0usize);
        assert_eq!(
            ffi::git_delta_create_from_index(
                &mut out,
                &mut out_len,
                index,
                trg.as_ptr().cast(),
                trg.len(),
                0
            ),
            0
        );
        let d = core::slice::from_raw_parts(out.cast::<u8>(), out_len).to_vec();
        assert_eq!(delta::diff(&src, &trg, 0).unwrap().unwrap(), d);
        let (mut b, mut r) = (0usize, 0usize);
        assert_eq!(
            ffi::git_delta_read_header(&mut b, &mut r, d.as_ptr(), d.len()),
            0
        );
        assert_eq!((b, r), (src.len(), trg.len()));

        let (mut res, mut res_len) = (ptr::null_mut::<c_void>(), 1usize);
        assert_eq!(
            ffi::git_delta_apply(
                &mut res,
                &mut res_len,
                src.as_ptr(),
                src.len(),
                out.cast(),
                out_len
            ),
            0
        );
        assert_eq!(res_len, trg.len());
        let got = core::slice::from_raw_parts(res.cast::<u8>(), res_len + 1);
        assert_eq!(&got[..res_len], &trg[..]);
        assert_eq!(got[res_len], 0);
        alloc::free(res);
        alloc::free(out);

        // max_size too small: GIT_EBUFS, NULL.
        assert_eq!(
            ffi::git_delta_create_from_index(
                &mut out,
                &mut out_len,
                index,
                trg.as_ptr().cast(),
                trg.len(),
                5
            ),
            -6
        );
        assert!(out.is_null());
        assert_eq!(out_len, 0);
        // Empty target: 0, NULL.
        assert_eq!(
            ffi::git_delta_create_from_index(&mut out, &mut out_len, index, ptr::null(), 0, 0),
            0
        );
        assert!(out.is_null());
        ffi::git_delta_index_free(index);
        ffi::git_delta_index_free(ptr::null_mut());

        assert_eq!(ffi::git_delta_index_init(&mut index, ptr::null(), 10), 0);
        assert!(index.is_null());
        assert_eq!(ffi::git_delta_index_size(ptr::null_mut()), usize::MAX);
        assert_eq!(msg(), b"invalid argument: 'index'");
    }
}

#[test]
fn apply_failures() {
    let _g = test_support::init();
    let base = [0u8; 16];
    let delta = [0x10u8, 0x10, 0xff, 0xff, 0xff, 0xff, 0xff, 0x10, 0x00, 0x00];
    // SAFETY: as C calls it.
    unsafe {
        let (mut out, mut len) = (ptr::dangling_mut::<c_void>(), 7usize);
        assert_eq!(
            ffi::git_delta_apply(
                &mut out,
                &mut len,
                base.as_ptr(),
                base.len(),
                delta.as_ptr(),
                delta.len()
            ),
            -1
        );
        assert!(out.is_null());
        assert_eq!(len, 0);
        assert_eq!(msg(), b"failed to apply delta");
        let (mut b, mut r) = (0usize, 99usize);
        // The base size is written before the result's fails.
        assert_eq!(
            ffi::git_delta_read_header(&mut b, &mut r, [0x05u8, 0x80].as_ptr(), 2),
            -1
        );
        assert_eq!((b, r), (5, 99));
        assert_eq!(msg(), b"truncated delta");
    }
}

#[test]
fn apply_under_alloc_limit() {
    let g = test_support::exclusive();
    let base = [1u8; 16];
    let delta = [0x10u8, 0x10, 0x90, 0x10];
    test_support::with_alloc_limit(&g, 0, || {
        // SAFETY: as C calls it.
        unsafe {
            let (mut out, mut len) = (ptr::null_mut::<c_void>(), 0usize);
            assert_eq!(
                ffi::git_delta_apply(
                    &mut out,
                    &mut len,
                    base.as_ptr(),
                    base.len(),
                    delta.as_ptr(),
                    delta.len()
                ),
                -1
            );
            assert!(out.is_null());
        }
    });
    assert_eq!(errors::last().0, errors::ErrorClass::NOMEMORY);
}

#[test]
fn index_freed_with_git_free() {
    let _g = test_support::init();
    let src = vec![3u8; 1000];
    // SAFETY: as `pack-objects.c` does: init, use, then `git__free`.
    unsafe {
        let mut index = ptr::null_mut();
        assert_eq!(
            ffi::git_delta_index_init(&mut index, src.as_ptr().cast(), src.len()),
            0
        );
        let (mut out, mut out_len) = (ptr::null_mut::<c_void>(), 0usize);
        assert_eq!(
            ffi::git_delta_create_from_index(
                &mut out,
                &mut out_len,
                index,
                src.as_ptr().cast(),
                500,
                0
            ),
            0
        );
        assert!(out_len >= delta::GIT_DELTA_SIZE_MIN);
        alloc::free(out);
        alloc::free(index.cast());
    }
}
