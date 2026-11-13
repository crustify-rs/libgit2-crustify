//! Equivalence tests for `deps/llhttp/llhttp.c`'s parser and enums against the C.
//!
//! Each test builds two parsers from one description, each initialised by `llhttp_init`
//! and fed the same input by `llhttp_execute`: one is read through api.c's getters
//! (`llhttp_get_type`, `llhttp_get_errno`, `llhttp_get_error_pos`, …), the other through
//! the safe handles, and the two are compared after every step of a multi-call sequence:
//! a response read in pieces, a request with a method, an upgrade, an invalid message, a
//! callback's own error and a pause. The setters are compared by their effect on the C: a
//! field set through [`ParserMut`] on one parser and as C sets it on the other, then read
//! back by the C getters, and by the next `llhttp_execute`.
//!
//! The enums' discriminants are their bindgen constants by construction; the tests compare
//! the values C stores with the variants the safe getters return for them.

use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use ffibox::{CMutSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Errno, Method, Parser, ParserRef, ParserType, Status,
};
use libgit2_crustify::sys::libgit2 as sys;

/// `LENIENT_HEADERS`, `llhttp_lenient_flags`' first bit.
const LENIENT_HEADERS: u16 = 0x1;

/// A parser initialised by C, as `llhttp_init(parser, parser_type, settings)` leaves it.
fn c_init(parser_type: ParserType, settings: *const sys::llhttp_settings_t) -> Parser {
    let mut parser = Parser::zeroed();
    // SAFETY: the handle addresses `parser`, a live `llhttp_t` Rust owns, which
    // `llhttp_init` fills in place; `settings` is NULL or a table the caller keeps alive
    // while the parser executes.
    unsafe { sys::llhttp_init(parser.as_mut().as_mut_ptr(), parser_type.raw(), settings) };
    parser
}

/// Feeds `data` to `parser` through C, returning the `llhttp_errno` it reports.
fn c_execute(parser: &mut Parser, data: &[u8]) -> sys::llhttp_errno {
    // SAFETY: `parser` is initialised (`c_init`), `data` is `data.len()` readable bytes, and
    // its settings, if any, are live (`c_init`'s callers).
    unsafe {
        sys::llhttp_execute(
            parser.as_mut().as_mut_ptr(),
            data.as_ptr().cast(),
            data.len(),
        )
    }
}

/// What api.c's getters read of a parser, in the safe API's terms.
#[derive(Debug, PartialEq, Eq)]
struct Fields {
    parser_type: u8,
    http_major: u8,
    http_minor: u8,
    method: u8,
    status_code: i32,
    upgrade: u8,
    errno: u32,
    reason: Option<Vec<u8>>,
    error_offset: Option<usize>,
}

/// `parser`'s fields through api.c's getters, its error position as an offset into `data`.
fn c_fields(parser: &mut Parser, data: &[u8]) -> Fields {
    let p = parser.as_mut().as_mut_ptr();
    // SAFETY: `p` addresses a live, initialised `llhttp_t`; each getter reads its fields.
    let parser_type = unsafe { sys::llhttp_get_type(p) };
    // SAFETY: as above.
    let http_major = unsafe { sys::llhttp_get_http_major(p) };
    // SAFETY: as above.
    let http_minor = unsafe { sys::llhttp_get_http_minor(p) };
    // SAFETY: as above.
    let method = unsafe { sys::llhttp_get_method(p) };
    // SAFETY: as above.
    let status_code = unsafe { sys::llhttp_get_status_code(p) };
    // SAFETY: as above.
    let upgrade = unsafe { sys::llhttp_get_upgrade(p) };
    // SAFETY: as above.
    let errno = unsafe { sys::llhttp_get_errno(p) };
    // SAFETY: as above.
    let reason = unsafe { sys::llhttp_get_error_reason(p) };
    // SAFETY: a non-NULL reason is one of llhttp's string literals.
    let reason = (!reason.is_null()).then(|| unsafe { CStr::from_ptr(reason) }.to_bytes().to_vec());
    // SAFETY: as above.
    let pos = unsafe { sys::llhttp_get_error_pos(p) };
    let error_offset = (!pos.is_null())
        .then(|| pos.addr().checked_sub(data.as_ptr().addr()))
        .flatten()
        .filter(|&offset| offset <= data.len());
    Fields {
        parser_type,
        http_major,
        http_minor,
        method,
        status_code,
        upgrade,
        errno,
        reason,
        error_offset,
    }
}

/// `parser`'s fields through the safe handles.
fn rust_fields(parser: ParserRef<'_>, data: &[u8]) -> Fields {
    Fields {
        parser_type: parser.parser_type_raw(),
        http_major: parser.http_major(),
        http_minor: parser.http_minor(),
        method: parser.method_raw(),
        status_code: i32::from(parser.status_code()),
        upgrade: u8::from(parser.upgrade()),
        errno: parser.error_raw().cast_unsigned(),
        reason: parser.reason().map(|r| r.to_bytes().to_vec()),
        error_offset: parser.error_offset(data),
    }
}

/// Feeds each piece of `input` to two parsers of `parser_type`, comparing their fields read
/// through C and through Rust before the first and after every piece; returns the parser
/// Rust read, and the errno of the last piece.
fn compare_pieces(parser_type: ParserType, pieces: &[&[u8]]) -> (Parser, sys::llhttp_errno) {
    let mut c = c_init(parser_type, ptr::null());
    let mut rust = c_init(parser_type, ptr::null());
    assert_eq!(c_fields(&mut c, &[]), rust_fields(rust.as_ref(), &[]));
    let mut last = sys::HPE_OK;
    for (step, piece) in pieces.iter().enumerate() {
        let c_errno = c_execute(&mut c, piece);
        let rust_errno = c_execute(&mut rust, piece);
        assert_eq!(c_errno, rust_errno, "step {step}");
        assert_eq!(
            c_fields(&mut c, piece),
            rust_fields(rust.as_ref(), piece),
            "step {step}"
        );
        last = rust_errno;
    }
    (rust, last)
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_response_read_in_pieces_reads_alike() {
    let (parser, errno) = compare_pieces(
        ParserType::Response,
        &[
            b"HTTP/1.",
            b"1 404 Not Fou",
            b"nd\r\nContent-Length: 5\r\n",
            b"\r\nab",
            b"cde",
        ],
    );
    assert_eq!(errno, sys::HPE_OK);
    let p = parser.as_ref();
    assert_eq!(p.parser_type(), Ok(ParserType::Response));
    assert_eq!(
        Status::from_raw(u32::from(p.status_code())),
        Some(Status::NotFound)
    );
    assert_eq!((p.http_major(), p.http_minor()), (1, 1));
    assert_eq!(p.error(), Ok(Errno::Ok));
    // A response sets no method: it stays as `llhttp__internal_init` left it.
    assert_eq!(p.method(), Ok(Method::Delete));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_status_code_llhttp_status_does_not_name_reads_alike() {
    let (parser, errno) = compare_pieces(ParserType::Response, &[b"HTTP/1.0 209 Odd\r\n\r\n"]);
    assert_eq!(errno, sys::HPE_OK);
    assert_eq!(parser.as_ref().status_code(), 209);
    assert_eq!(Status::from_raw(209), None);
    assert_eq!(parser.as_ref().http_minor(), 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn requests_read_their_method_and_upgrade_alike() {
    let (parser, _) = compare_pieces(
        ParserType::Request,
        &[b"POST /x HTTP/1.1\r\n", b"Host: a\r\n\r\n"],
    );
    assert_eq!(parser.as_ref().method(), Ok(Method::Post));
    assert!(!parser.as_ref().upgrade());

    // A `CONNECT` request upgrades the connection: `llhttp_execute` pauses after it.
    let (parser, errno) = compare_pieces(
        ParserType::Both,
        &[b"CONNECT a:443 HTTP/1.1\r\nHost: a\r\n\r\n"],
    );
    assert_eq!(errno, sys::HPE_PAUSED_UPGRADE);
    let p = parser.as_ref();
    assert_eq!(p.method(), Ok(Method::Connect));
    assert!(p.upgrade());
    assert_eq!(p.error(), Ok(Errno::PausedUpgrade));
    // A parser of `HTTP_BOTH` takes the type of the first message it reads.
    assert_eq!(p.parser_type(), Ok(ParserType::Request));

    let (parser, _) = compare_pieces(ParserType::Request, &[b"M-SEARCH * HTTP/1.1\r\n\r\n"]);
    assert_eq!(parser.as_ref().method(), Ok(Method::MSearch));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_invalid_message_reports_its_error_alike_and_keeps_it() {
    let input: &[u8] = b"HTTP/1.1 200 OK\r\nBad Header: x\r\n\r\n";
    let (mut parser, errno) = compare_pieces(ParserType::Response, &[input, b"more"]);
    assert_eq!(errno, sys::HPE_INVALID_HEADER_TOKEN);
    let p = parser.as_ref();
    assert_eq!(p.error(), Ok(Errno::InvalidHeaderToken));
    assert!(p.reason().is_some());
    // The second call kept the error without parsing, and the position still points into
    // the first input, where the space of the header name is.
    assert_eq!(p.error_offset(input), Some(20));
    assert_eq!(p.error_offset(b"more"), None);
    // Offsets are counted from the slice given: the position may be its end, but neither
    // before its start nor past its end.
    assert_eq!(p.error_offset(&input[3..]), Some(17));
    assert_eq!(p.error_offset(&input[..20]), Some(20));
    assert_eq!(p.error_offset(&input[..19]), None);
    assert_eq!(p.error_offset(&input[21..]), None);
    // An error stays until the parser is initialised again.
    assert_eq!(
        c_execute(&mut parser, b"HTTP/1.1 200 OK\r\n"),
        sys::HPE_INVALID_HEADER_TOKEN
    );
}

/// A header-field callback that fails with its own value, `1000`.
unsafe extern "C" fn fail_header_field(_: *mut sys::llhttp_t, _: *const c_char, _: usize) -> c_int {
    1000
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callbacks_own_error_reads_as_unnamed_alike() {
    let mut settings = sys::llhttp_settings_t::default();
    // SAFETY: `settings` is a live table, which `llhttp_settings_init` zeroes.
    unsafe { sys::llhttp_settings_init(&raw mut settings) };
    settings.on_header_field = Some(fail_header_field);
    let input: &[u8] = b"HTTP/1.1 200 OK\r\nX: y\r\n\r\n";
    let mut c = c_init(ParserType::Response, &raw const settings);
    let mut rust = c_init(ParserType::Response, &raw const settings);
    assert_eq!(c_execute(&mut c, input), 1000);
    assert_eq!(c_execute(&mut rust, input), 1000);
    assert_eq!(c_fields(&mut c, input), rust_fields(rust.as_ref(), input));
    let p = rust.as_ref();
    assert_eq!(p.error_raw(), 1000);
    assert_eq!(p.error().unwrap_err().raw(), 1000);
    // The field's span ends at its colon; llhttp reports the byte after it.
    assert_eq!(p.error_offset(input), Some(19));
    assert_eq!(p.settings_ptr(), &raw const settings);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn setting_the_error_pauses_and_resumes_as_c_does() {
    let input: &[u8] = b"HTTP/1.1 200 OK\r\n";
    let mut c = c_init(ParserType::Response, ptr::null());
    let mut rust = c_init(ParserType::Response, ptr::null());

    // `llhttp_pause`'s body, through the C struct and through the setters.
    let p = c.as_mut().as_mut_ptr();
    // SAFETY: `p` addresses `c`, a live parser, nothing else accessing it while it is written.
    unsafe { (*p).error = sys::HPE_PAUSED.cast_signed() };
    // SAFETY: as above; the reason is a static string, as `llhttp_pause`'s is.
    unsafe { (*p).reason = c"Paused".as_ptr() };
    rust.as_mut().set_error(Errno::Paused);
    rust.as_mut().set_reason(Some(c"Paused"));
    assert_eq!(c_fields(&mut c, input), rust_fields(rust.as_ref(), input));
    // A paused parser returns the pause without parsing.
    assert_eq!(c_execute(&mut c, input), sys::HPE_PAUSED);
    assert_eq!(c_execute(&mut rust, input), sys::HPE_PAUSED);
    assert_eq!(c_fields(&mut c, input), rust_fields(rust.as_ref(), input));

    // `llhttp_resume`'s body: the error is cleared, the reason kept.
    // SAFETY: as above.
    unsafe { (*c.as_mut().as_mut_ptr()).error = 0 };
    rust.as_mut().set_error(Errno::Ok);
    assert_eq!(c_fields(&mut c, input), rust_fields(rust.as_ref(), input));
    assert_eq!(rust.as_ref().reason(), Some(c"Paused"));
    assert_eq!(c_execute(&mut c, input), sys::HPE_OK);
    assert_eq!(c_execute(&mut rust, input), sys::HPE_OK);
    assert_eq!(c_fields(&mut c, input), rust_fields(rust.as_ref(), input));
    assert_eq!(rust.as_ref().status_code(), 200);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_lenient_flags_set_through_rust_relax_the_c_parser() {
    let input: &[u8] = b"HTTP/1.1 200 OK\r\nX: a\x01b\r\n\r\n";
    let (strict, errno) = compare_pieces(ParserType::Response, &[input]);
    assert_eq!(errno, sys::HPE_INVALID_HEADER_TOKEN);
    assert_eq!(strict.as_ref().lenient_flags_raw(), 0);

    // `llhttp_set_lenient_headers(parser, 1)` sets the bit, here through the setter.
    let mut lenient = c_init(ParserType::Response, ptr::null());
    lenient.as_mut().set_lenient_flags_raw(LENIENT_HEADERS);
    assert_eq!(c_execute(&mut lenient, input), sys::HPE_OK);
    assert_eq!(lenient.as_ref().lenient_flags_raw(), LENIENT_HEADERS);
    assert_eq!(lenient.as_ref().error(), Ok(Errno::Ok));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_setters_write_what_api_c_reads() {
    let mut rust = c_init(ParserType::Request, ptr::null());
    rust.as_mut().set_parser_type(ParserType::Response);
    rust.as_mut().set_upgrade(true);
    let mut m = rust.as_mut();
    let p = m.as_mut_ptr();
    // SAFETY: `p` addresses a live, initialised parser; the getter reads it.
    let parser_type = unsafe { sys::llhttp_get_type(p) };
    // SAFETY: as above.
    let upgrade = unsafe { sys::llhttp_get_upgrade(p) };
    assert_eq!((parser_type, upgrade), (2, 1));

    // The type set through Rust decides what the parser reads next.
    assert_eq!(
        c_execute(&mut rust, b"HTTP/1.1 100 Continue\r\n\r\n"),
        sys::HPE_OK
    );
    assert_eq!(rust.as_ref().status_code(), 100);

    // `llhttp__after_message_complete` clears the finish byte and the flags, as here.
    let mut c = c_init(ParserType::Response, ptr::null());
    assert_eq!(
        c_execute(&mut c, b"HTTP/1.1 204 No Content\r\n\r\n"),
        sys::HPE_OK
    );
    let mut rust = c_init(ParserType::Response, ptr::null());
    rust.as_mut().set_flags_raw(0x20);
    rust.as_mut().set_finish_raw(1);
    rust.as_mut().set_finish_raw(0);
    rust.as_mut().set_flags_raw(0);
    assert_eq!(c.as_ref().finish_raw(), rust.as_ref().finish_raw());
    assert_eq!(c.as_ref().flags_raw(), rust.as_ref().flags_raw());
}
