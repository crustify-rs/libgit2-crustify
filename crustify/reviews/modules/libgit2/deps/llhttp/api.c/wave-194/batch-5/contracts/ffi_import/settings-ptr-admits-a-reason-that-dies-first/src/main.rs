//! `ParserMut::set_settings_ptr`'s `# Safety` asks for a table that stays live and unchanged
//! "whose callbacks are sound to call with this parser and its `data`", and `set_data`'s for a
//! `data` valid for each access those callbacks make "while the parser executes". Neither
//! says how long a reason a callback sets must live. llhttp lets a callback set any string,
//! `llhttp_set_error_reason` "right before returning the errno" (`llhttp.h`), and libgit2's
//! own C may hand it one its context owns. Yet the safe `ParserRef::reason` follows the
//! pointer the parser holds, for any borrow of the parser, however long after the parse.
//!
//! This program honours both contracts: its callback is sound with this parser and its
//! `data`, which stays valid while the parser executes, and both are cleared before the
//! context is freed. The safe `reason()` then reads the freed string.
//!
//! Run under the `asan-ubsan` variant: AddressSanitizer reports a heap use after free.

use std::ffi::{CString, c_char, c_int};
use std::ptr;

use libgit2::libgit2::deps::llhttp::llhttp::Parser;
use libgit2::sys::libgit2 as sys;

/// The context a C caller hangs on the parser's `data`, as libgit2's HTTP client hangs its
/// `http_parser_context`: here, the text of the error its callback reports.
struct Context {
    reason: CString,
}

/// An `on_url` callback as C writes one: it reads its context from the parser's `data` and
/// refuses the URL with the reason its context owns, through `llhttp_set_error_reason`,
/// returning `HPE_USER`.
///
/// # Safety
///
/// `parser` is a live parser whose `data` points at a live `Context`.
unsafe extern "C" fn refuse_url(parser: *mut sys::llhttp_t, _: *const c_char, _: usize) -> c_int {
    // SAFETY: per this function's contract, `parser` is live and its `data` is a `Context`.
    let context = unsafe { (*parser).data.cast::<Context>() };
    // SAFETY: as above; the call stores the pointer, which C reads only on request.
    unsafe { sys::llhttp_set_error_reason(parser, (*context).reason.as_ptr()) };
    sys::HPE_USER.cast_signed()
}

/// The table, live and unchanged for the rest of the program.
static SETTINGS: sys::llhttp_settings_t = sys::llhttp_settings_t {
    on_url: Some(refuse_url),
    ..zeroed_settings()
};

const fn zeroed_settings() -> sys::llhttp_settings_t {
    sys::llhttp_settings_t {
        on_message_begin: None,
        on_url: None,
        on_status: None,
        on_method: None,
        on_version: None,
        on_header_field: None,
        on_header_value: None,
        on_chunk_extension_name: None,
        on_chunk_extension_value: None,
        on_headers_complete: None,
        on_body: None,
        on_message_complete: None,
        on_url_complete: None,
        on_status_complete: None,
        on_method_complete: None,
        on_version_complete: None,
        on_header_field_complete: None,
        on_header_value_complete: None,
        on_chunk_extension_name_complete: None,
        on_chunk_extension_value_complete: None,
        on_chunk_header: None,
        on_chunk_complete: None,
        on_reset: None,
    }
}

fn main() {
    let context = Box::into_raw(Box::new(Context {
        reason: CString::new("URL refused by the caller's context").unwrap(),
    }));
    let mut parser = Parser::new();
    // SAFETY: `SETTINGS` lives, unchanged, for the rest of the program, and its one callback
    // is sound with this parser while its `data` is a live `Context`, which it is while the
    // parser executes with it (below, until `data` and `settings` are cleared).
    unsafe { parser.as_mut().set_settings_ptr(&raw const SETTINGS) };
    // SAFETY: `context` is the live `Context` the callbacks read, until it is cleared below.
    unsafe { parser.as_mut().set_data(context.cast()) };
    let error = parser.as_mut().parse(b"GET /refused HTTP/1.1\r\n").unwrap_err();
    assert_eq!(error.code(), sys::HPE_USER.cast_signed());
    println!("during the context: {:?}", parser.as_ref().reason());

    // The caller is done with the parser's callbacks: it clears them, and its context.
    // SAFETY: NULL runs no callback, and with no callback nothing reads `data`.
    unsafe { parser.as_mut().set_settings_ptr(ptr::null()) };
    // SAFETY: as above.
    unsafe { parser.as_mut().set_data(ptr::null_mut()) };
    // SAFETY: `context` came from `Box::into_raw` and nothing refers to it now but the
    // parser's `reason`, which no unsafe contract made the caller keep alive.
    drop(unsafe { Box::from_raw(context) });

    // Safe code: the getter follows the freed string.
    println!("after the context: {:?}", parser.as_ref().reason());
}
