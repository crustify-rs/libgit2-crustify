//! libgit2 builds with `-fvisibility=hidden`, so `libgit2.so` exports none of its internal
//! functions. Campaign mode links a variant's static archive instead, in which they link:
//! this test calls `deps/reftable/basics.c`'s buffer functions, which `nm -D libgit2.so`
//! does not list, through their raw bindings.

use libgit2_sys::{reftable_buf, reftable_buf_add, reftable_buf_init, reftable_buf_release};

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn hidden_reftable_buffer_functions_link_and_run() {
    let mut buf = reftable_buf {
        alloc: 7,
        len: 3,
        buf: std::ptr::dangling_mut(),
    };
    // SAFETY: `reftable_buf_init` writes an empty buffer over `*buf` without reading it.
    unsafe { reftable_buf_init(&raw mut buf) };
    assert_eq!((buf.alloc, buf.len), (0, 0));
    assert!(buf.buf.is_null());

    let data = b"hidden";
    // SAFETY: `buf` is an initialised buffer, and `data` is readable for its length.
    let rc = unsafe { reftable_buf_add(&raw mut buf, data.as_ptr().cast(), data.len()) };
    assert_eq!(rc, 0);
    assert_eq!(buf.len, data.len());
    assert!(buf.alloc > buf.len);
    // SAFETY: `reftable_buf_add` keeps `buf.buf` NUL-terminated and `buf.len` bytes long.
    let held = unsafe { std::slice::from_raw_parts(buf.buf.cast::<u8>(), buf.len + 1) };
    assert_eq!(held, b"hidden\0");

    // SAFETY: `buf` owns the allocation `reftable_buf_add` made; release frees it once and
    // leaves an empty buffer, which the leak checker of the ASan variant confirms.
    unsafe { reftable_buf_release(&raw mut buf) };
    assert_eq!((buf.alloc, buf.len), (0, 0));
    assert!(buf.buf.is_null());
}
