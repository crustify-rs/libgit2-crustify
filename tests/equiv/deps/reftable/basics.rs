//! Equivalence tests for `deps/reftable/basics.c`'s types against the C contract.
//!
//! `libgit2.so` exports none of `basics.c`'s functions, so the C side here is the header:
//! `enum reftable_hash`'s constants, `REFTABLE_BUF_INIT`, and the fields of
//! `struct reftable_buf` as reftable's consumers read them (`reftable_buf_size` returns
//! `len`; the stack passes `buf` to `unlink` and `tmpfile_rename` as a path). Each buffer is
//! a C struct over Rust storage, in a state `basics.c`'s routines leave one in, read once
//! through its fields and once through a handle. Nothing calls C, so these also run under
//! Miri.

use std::ffi::{CStr, c_char};
use std::{ptr, slice};

use ffibox::{CRefSeam, CZeroable};
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{Buf, BufRef, HashId};

#[test]
fn hash_id_names_exactly_the_constants_of_reftable_basics_h() {
    assert_eq!(HashId::Sha1.raw(), ffi::REFTABLE_HASH_SHA1);
    assert_eq!(HashId::Sha256.raw(), ffi::REFTABLE_HASH_SHA256);
    let named = [ffi::REFTABLE_HASH_SHA1, ffi::REFTABLE_HASH_SHA256];
    for raw in (0..=1024).chain([ffi::reftable_hash::MAX]) {
        assert_eq!(
            HashId::from_raw(raw).is_some(),
            named.contains(&raw),
            "{raw}"
        );
    }
}

#[test]
fn zeroed_buf_is_reftable_buf_init() {
    assert_eq!(size_of::<Buf>(), size_of::<ffi::reftable_buf>());
    assert_eq!(align_of::<Buf>(), align_of::<ffi::reftable_buf>());
    let value = Buf::zeroed();
    // SAFETY: the handle addresses `value`, a live `reftable_buf`, which is read once.
    let raw = unsafe { value.as_ref().as_ptr().read() };
    // `REFTABLE_BUF_INIT` is `{ 0 }`.
    assert_eq!(raw.alloc, 0);
    assert_eq!(raw.len, 0);
    assert!(raw.buf.is_null());
}

/// What a C consumer reads of `raw`'s fields, and what the handle's getters read: `len`,
/// `alloc`, the contents and, where they are terminated, the C string.
fn compare(raw: &mut ffi::reftable_buf) {
    let c_len = raw.len;
    let c_alloc = raw.alloc;
    let c_bytes: Vec<u8> = if raw.alloc == 0 {
        Vec::new()
    } else {
        // SAFETY: `raw.buf` addresses `raw.alloc > raw.len` bytes of live storage.
        unsafe { slice::from_raw_parts(raw.buf.cast::<u8>(), raw.len) }.to_vec()
    };
    let ptr: *mut ffi::reftable_buf = raw;
    // SAFETY: `raw` is a live `reftable_buf` in a state `basics.c` leaves, borrowed for
    // this call.
    let b = unsafe { BufRef::from_ptr(ptr) }.expect("non-null");
    assert_eq!(b.len(), c_len);
    assert_eq!(b.capacity(), c_alloc);
    assert_eq!(b.as_bytes(), c_bytes.as_slice());
    if c_alloc > 0 && !c_bytes.contains(&0) {
        // SAFETY: as above; `buf[len]` is within the allocation, and is NUL here.
        let c_str = unsafe { CStr::from_ptr(raw.buf) };
        assert_eq!(b.as_c_str(), Some(c_str));
    }
}

#[test]
fn getters_read_the_fields_as_c_consumers_do() {
    // `REFTABLE_BUF_INIT`, then a buffer `reftable_buf_addstr` filled with a table name,
    // then that buffer after `reftable_buf_reset` and after `reftable_buf_setlen`.
    let mut init = ffi::reftable_buf {
        alloc: 0,
        len: 0,
        buf: ptr::null_mut(),
    };
    compare(&mut init);

    // Each state over storage of its own: a buffer as `basics.c` leaves it.
    let name = *b"0x000000000001-0x000000000002-1a2b3c4d.ref\0\0\0\0\0";
    for (len, terminator_at) in [(42, None), (14, Some(14)), (0, Some(0))] {
        let mut storage = name;
        if let Some(i) = terminator_at {
            storage[i] = 0;
        }
        let mut raw = ffi::reftable_buf {
            alloc: storage.len(),
            len,
            buf: storage.as_mut_ptr().cast::<c_char>(),
        };
        compare(&mut raw);
    }
}

#[test]
fn contents_with_nul_bytes_read_whole() {
    // `reftable_buf_add` takes arbitrary bytes, such as a binary record key.
    let mut storage = [0x01, 0x00, b'r', 0xFF, 0x00, 0x00];
    let mut raw = ffi::reftable_buf {
        alloc: storage.len(),
        len: 4,
        buf: storage.as_mut_ptr().cast::<c_char>(),
    };
    compare(&mut raw);
    let ptr: *mut ffi::reftable_buf = &raw mut raw;
    // SAFETY: as in `compare`.
    let b = unsafe { BufRef::from_ptr(ptr) }.expect("non-null");
    assert_eq!(b.as_bytes(), [0x01, 0x00, b'r', 0xFF]);
    assert_eq!(b.as_c_str(), None, "a NUL comes before the terminator");
}
