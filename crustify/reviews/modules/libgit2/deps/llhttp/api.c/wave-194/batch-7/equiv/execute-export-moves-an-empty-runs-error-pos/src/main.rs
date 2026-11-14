//! api.c's `llhttp_execute(parser, data, len)` is `llhttp__internal_execute(parser, data,
//! data + len)`: the generated parser restarts an open span at `data`, hands the span's
//! callback the run from there to `data + len`, and, when the callback refuses it, stores
//! `data + len` as the error position, which `llhttp_get_error_pos` returns and libgit2's
//! `git_http_parser_execute` subtracts `data` from (`transports/httpparser.c:118`). llhttp.h
//! documents the position as "relative to the `data` argument of `llhttp_execute()`".
//!
//! The export of `llhttp_execute` under `ffi-export-deps-llhttp-api-c` reads a run of 0
//! bytes as `&[]`, whose address is the dangling `0x1`, not `data`. So when a run of 0 bytes
//! ends a header's name the previous run left open and the callback refuses it, the error
//! position is `0x1`, and `error_pos - data` is no offset into the run.
//!
//! The scenario runs two parsers alike, one through the export and one through the C body
//! api.c gives `llhttp_execute`, and compares what each leaves. It links the export in place
//! of api.c's C: run it with `CRUSTIFY_BUILD_DIR` naming a copy of the `plain` variant whose
//! archive lacks `api.c.o` (`report.md`).

use std::ffi::{c_char, c_int};
use std::mem::MaybeUninit;

use libgit2::sys::libgit2 as sys;

/// An `on_header_field` that takes every run but an empty one, which it refuses with `7`.
unsafe extern "C" fn refuse_empty(_: *mut sys::llhttp_t, _: *const c_char, len: usize) -> c_int {
    if len == 0 { 7 } else { 0 }
}

/// A response parser initialised by `llhttp_init` with `settings`.
fn parser(settings: &sys::llhttp_settings_t) -> Box<sys::llhttp_t> {
    let mut parser = Box::new(MaybeUninit::<sys::llhttp_t>::uninit());
    // SAFETY: `parser` is a parser's storage, ours alone; `settings` outlives every call
    // below, and its callback reads no `data`.
    unsafe { sys::llhttp_init(parser.as_mut_ptr(), sys::HTTP_RESPONSE, settings) };
    // SAFETY: `llhttp_init` initialised every byte.
    unsafe { parser.assume_init() }
}

fn main() {
    let settings = sys::llhttp_settings_t {
        on_header_field: Some(refuse_empty),
        ..sys::llhttp_settings_t::default()
    };
    let (mut export, mut c) = (parser(&settings), parser(&settings));
    // A first run leaves the header's name open, its span pending at the run's end.
    let head: &[u8] = b"HTTP/1.1 200 OK\r\nConten";
    // SAFETY: both parsers are initialised, `head` is live, and the table outlives them.
    let (e, k) = unsafe {
        let range = head.as_ptr_range();
        (
            sys::llhttp_execute(&raw mut *export, head.as_ptr().cast(), head.len()),
            sys::llhttp__internal_execute(&raw mut *c, range.start.cast(), range.end.cast()),
        )
    };
    assert_eq!((e, k.cast_unsigned()), (sys::HPE_OK, sys::HPE_OK));

    // A run of 0 bytes at `data`, a live buffer's start: the open span ends there.
    let buffer = *b"t-Length: 0\r\n\r\n";
    let data: *const c_char = buffer.as_ptr().cast();
    // SAFETY: as above; `data` is valid for reads of 0 bytes.
    let (e, k) = unsafe {
        (
            sys::llhttp_execute(&raw mut *export, data, 0),
            sys::llhttp__internal_execute(&raw mut *c, data, data.add(0)),
        )
    };
    println!("llhttp_execute: {e}, api.c's body: {k}");
    assert_eq!(e, k.cast_unsigned());
    // SAFETY: both parsers are live; the getter reads their `error_pos` field.
    let (e_pos, k_pos) = unsafe {
        (
            sys::llhttp_get_error_pos(&raw const *export),
            sys::llhttp_get_error_pos(&raw const *c),
        )
    };
    println!("error_pos: export {e_pos:p}, api.c's body {k_pos:p}, data {data:p}");
    println!(
        "git_http_parser_execute's parsed_len: export {}, api.c's body {}",
        e_pos.addr().wrapping_sub(data.addr()),
        k_pos.addr().wrapping_sub(data.addr()),
    );
    assert_eq!(e_pos, k_pos, "the export's error position is not relative to `data`");
}
