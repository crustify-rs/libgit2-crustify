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
//!
//! The constructor and the parse are compared with `llhttp__internal_init` and
//! `llhttp__internal_execute`, called through the raw bindings on a second parser in the
//! same state: [`Parser::new`] and [`ParserMut::reinit`] byte for byte, private state
//! included, and [`ParserMut::parse`] by its outcome, the whole parser after each run, and
//! the callbacks its settings run, which are Rust handlers C calls through
//! [`DataCallback`] and [`Callback`]. The X-macros are compared with what C expands them
//! to: their text in `llhttp.h`, and api.c's `llhttp_errno_name`, `llhttp_method_name` and
//! `llhttp_status_name`, the three `switch`es over them.

use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Method, ParseError, Parsed, Parser,
    ParserMut, ParserRef, ParserType, Status,
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

// --- The X-macros ----------------------------------------------------------------------

/// The `XX(NUM, NAME, STRING)` entries of the X-macro `name` in `llhttp.h`, in its order.
fn header_entries(name: &str) -> Vec<(u32, String, String)> {
    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/deps/llhttp/llhttp.h"
    ))
    .unwrap();
    let start = header.find(&format!("#define {name}(XX) \\\n")).unwrap();
    header[start..]
        .lines()
        .skip(1)
        .take_while(|line| line.trim_start().starts_with("XX("))
        .map(|line| {
            let inner = line
                .trim()
                .trim_start_matches("XX(")
                .trim_end_matches(") \\");
            let mut parts = inner.split(", ");
            let num = parts.next().unwrap().parse().unwrap();
            let entry_name = parts.next().unwrap().to_owned();
            let string = parts.next().unwrap().to_owned();
            assert_eq!(parts.next(), None, "{line}");
            (num, entry_name, string)
        })
        .collect()
}

/// `map`'s entries as `(NUM, NAME, STRING)`.
fn rust_entries<T: Copy>(
    map: &[T],
    raw: impl Fn(T) -> u32,
    name: impl Fn(T) -> &'static CStr,
    string: impl Fn(T) -> &'static CStr,
) -> Vec<(u32, String, String)> {
    map.iter()
        .map(|&e| {
            let text = |s: &CStr| s.to_str().unwrap().to_owned();
            (raw(e), text(name(e)), text(string(e)))
        })
        .collect()
}

/// The string a C name function returns, which is never NULL for a value its X-macro lists.
fn c_name(name: *const c_char) -> &'static CStr {
    assert!(!name.is_null());
    // SAFETY: api.c's name functions return one of their string literals, which live as
    // long as the program.
    unsafe { CStr::from_ptr(name) }
}

#[test]
#[cfg_attr(miri, ignore = "reads llhttp.h")]
fn the_maps_are_llhttp_hs_x_macros_entry_for_entry() {
    assert_eq!(
        rust_entries(Errno::MAP, Errno::raw, Errno::map_name, Errno::map_string),
        header_entries("HTTP_ERRNO_MAP")
    );
    assert_eq!(
        rust_entries(
            Method::MAP,
            Method::raw,
            Method::map_name,
            Method::map_string
        ),
        header_entries("HTTP_ALL_METHOD_MAP")
    );
    assert_eq!(
        rust_entries(
            Status::MAP,
            Status::raw,
            Status::map_name,
            Status::map_string
        ),
        header_entries("HTTP_STATUS_MAP")
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_maps_give_what_api_cs_name_functions_return() {
    for &errno in Errno::MAP {
        // SAFETY: `errno` is one `HTTP_ERRNO_MAP` lists, for which the C returns.
        let c = c_name(unsafe { sys::llhttp_errno_name(errno.raw()) });
        assert_eq!(c, errno.constant_name(), "{errno:?}");
    }
    for &method in Method::MAP {
        // SAFETY: `method` is one `HTTP_ALL_METHOD_MAP` lists, for which the C returns.
        let c = c_name(unsafe { sys::llhttp_method_name(method.raw()) });
        assert_eq!(c, method.map_string(), "{method:?}");
    }
    for &status in Status::MAP {
        // SAFETY: `status` is one `HTTP_STATUS_MAP` lists, for which the C returns.
        let c = c_name(unsafe { sys::llhttp_status_name(status.raw()) });
        assert_eq!(c, status.map_string(), "{status:?}");
    }
}

// --- The constructor and the parse -----------------------------------------------------

/// Every field of `parser`, private state included, read through its raw layout.
fn raw_state(parser: ParserRef<'_>) -> String {
    // SAFETY: the handle addresses a live parser, which is plain data with no padding read
    // here: each field is read by name, and `Debug` formats the integers and addresses.
    let raw = unsafe { parser.as_ptr().read() };
    format!(
        "{:?}",
        (
            (
                raw._index,
                raw._span_pos0,
                raw._span_cb0,
                raw.error,
                raw.reason
            ),
            (
                raw.error_pos,
                raw.data,
                raw._current,
                raw.content_length,
                raw.type_
            ),
            (
                raw.method,
                raw.http_major,
                raw.http_minor,
                raw.header_state,
                raw.lenient_flags
            ),
            (raw.upgrade, raw.finish, raw.flags, raw.status_code),
            (raw.initial_message_completed, raw.settings),
        )
    )
}

/// `parser` initialised through the raw binding of `llhttp__internal_init`.
fn c_internal_init(parser: &mut Parser) {
    // SAFETY: the handle addresses a live parser, which `llhttp__internal_init` fills.
    let rc = unsafe { sys::llhttp__internal_init(parser.as_mut().as_mut_ptr()) };
    assert_eq!(rc, 0);
}

/// `data` fed to `parser` through the raw binding of `llhttp__internal_execute`.
fn c_internal_execute(parser: &mut Parser, data: &[u8]) -> c_int {
    let range = data.as_ptr_range();
    // SAFETY: `parser` was initialised, `range` bounds the live `data`, and its settings, if
    // any, are the tests' tables of Rust handlers, live for the call.
    unsafe {
        sys::llhttp__internal_execute(
            parser.as_mut().as_mut_ptr(),
            range.start.cast(),
            range.end.cast(),
        )
    }
}

/// The C outcome, as [`ParserMut::parse`] maps it.
fn c_outcome(code: c_int) -> Result<Parsed, c_int> {
    match code.cast_unsigned() {
        sys::HPE_OK => Ok(Parsed::All),
        sys::HPE_PAUSED => Ok(Parsed::Paused),
        sys::HPE_PAUSED_UPGRADE => Ok(Parsed::PausedUpgrade),
        _ => Err(code),
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn new_and_reinit_leave_the_parser_as_llhttp_internal_init_does() {
    let mut c = Parser::zeroed();
    c_internal_init(&mut c);
    let mut rust = Parser::new();
    assert_eq!(raw_state(rust.as_ref()), raw_state(c.as_ref()));
    // From a parser in the middle of a message, with its fields set.
    let dirty = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nbo";
    assert_eq!(c_internal_execute(&mut c, dirty), 0);
    assert_eq!(rust.as_mut().parse(dirty), Ok(Parsed::All));
    for parser in [&mut c, &mut rust] {
        let mut m = parser.as_mut();
        m.set_reason(Some(c"set"));
        m.set_lenient_flags_raw(0x3FF);
    }
    assert_eq!(raw_state(rust.as_ref()), raw_state(c.as_ref()));
    c_internal_init(&mut c);
    rust.as_mut().reinit();
    assert_eq!(raw_state(rust.as_ref()), raw_state(c.as_ref()));
}

/// Runs the same input through both parsers, `rust` by [`ParserMut::parse`] and `c` by
/// `llhttp__internal_execute`, comparing the outcomes, the whole parser and what the
/// handlers saw after every run; each run is one slice both parse, so that the pointers
/// they keep into it compare equal.
fn compare_parses(c: &mut Parser, rust: &mut Parser, runs: &[&[u8]]) {
    for (step, run) in runs.iter().enumerate() {
        let c_out = c_outcome(c_internal_execute(c, run));
        let c_seen = seen::take();
        let rust_out = rust.as_mut().parse(run).map_err(ParseError::code);
        let rust_seen = seen::take();
        assert_eq!(rust_out, c_out, "step {step}");
        assert_eq!(rust_seen, c_seen, "step {step}");
        assert_eq!(
            raw_state(rust.as_ref()),
            raw_state(c.as_ref()),
            "step {step}"
        );
    }
}

/// Two parsers initialised alike, with `settings`.
fn pair(settings: &sys::llhttp_settings_t) -> (Parser, Parser) {
    let mut c = Parser::zeroed();
    c_internal_init(&mut c);
    let mut rust = Parser::new();
    for parser in [&mut c, &mut rust] {
        // SAFETY: `settings` outlives both parsers' parses (each test's), and its handlers
        // read no `data`.
        unsafe { parser.as_mut().set_settings_ptr(settings) };
    }
    (c, rust)
}

mod seen {
    use std::cell::RefCell;

    thread_local! {
        static SEEN: RefCell<Vec<(&'static str, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(what: &'static str, data: &[u8]) {
        SEEN.with_borrow_mut(|seen| seen.push((what, data.to_vec())));
    }

    pub(super) fn take() -> Vec<(&'static str, Vec<u8>)> {
        SEEN.with(RefCell::take)
    }
}

/// Records each run of a header's name.
struct Field;

impl DataHandler for Field {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        seen::record("field", data);
        0
    }
}

/// Records each run of the body, and stops the parse with its own code at a `!`.
struct Body;

impl DataHandler for Body {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        seen::record("body", data);
        if data.contains(&b'!') { 4242 } else { 0 }
    }
}

/// Records the end of the headers, and pauses for a `Pause` header's message.
struct Headers;

impl EventHandler for Headers {
    fn on_event(parser: ParserMut<'_>) -> c_int {
        seen::record("headers", &parser.as_ref().status_code().to_be_bytes());
        if parser.as_ref().status_code() == 299 {
            c_int::try_from(sys::HPE_PAUSED).unwrap()
        } else {
            0
        }
    }
}

/// Records the end of a message.
struct Complete;

impl EventHandler for Complete {
    fn on_event(_: ParserMut<'_>) -> c_int {
        seen::record("complete", b"");
        0
    }
}

fn recording() -> sys::llhttp_settings_t {
    sys::llhttp_settings_t {
        on_header_field: DataCallback::new::<Field>().raw(),
        on_body: DataCallback::new::<Body>().raw(),
        on_headers_complete: Callback::new::<Headers>().raw(),
        on_message_complete: Callback::new::<Complete>().raw(),
        ..sys::llhttp_settings_t::default()
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parse_runs_as_llhttp_internal_execute_does() {
    let settings = recording();
    let message: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX: y\r\n\r\nhello";
    for size in [1, 2, 3, 7, 16, message.len()] {
        let (mut c, mut rust) = pair(&settings);
        let runs: Vec<&[u8]> = message.chunks(size).chain([&b""[..], message]).collect();
        compare_parses(&mut c, &mut rust, &runs);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn pauses_errors_and_upgrades_parse_as_c_does() {
    let settings = recording();
    // A pause from `on_headers_complete`, then a resume and the body.
    let paused: &[u8] = b"HTTP/1.1 299 X\r\nContent-Length: 2\r\n\r\nok";
    let (mut c, mut rust) = pair(&settings);
    compare_parses(&mut c, &mut rust, &[paused, paused]);
    for parser in [&mut c, &mut rust] {
        parser.as_mut().set_error(Errno::Ok);
    }
    compare_parses(&mut c, &mut rust, &[b"ok"]);
    // A body callback's own code, which is kept.
    let refused: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n!!";
    let (mut c, mut rust) = pair(&settings);
    compare_parses(&mut c, &mut rust, &[refused, b"more"]);
    assert_eq!(rust.as_ref().error_raw(), 4242);
    // An invalid message, kept.
    let (mut c, mut rust) = pair(&settings);
    compare_parses(
        &mut c,
        &mut rust,
        &[b"HTTP/1.1 200 OK\r\nA B: c\r\n", b"\r\n"],
    );
    // An upgrade, and what follows it once resumed.
    let upgrade: &[u8] = b"GET / HTTP/1.1\r\nConnection: upgrade\r\nUpgrade: ws\r\n\r\nframes";
    let (mut c, mut rust) = pair(&settings);
    compare_parses(&mut c, &mut rust, &[upgrade]);
    for parser in [&mut c, &mut rust] {
        parser.as_mut().set_error(Errno::Ok);
    }
    compare_parses(&mut c, &mut rust, &[b"GET / HTTP/1.1\r\n\r\n"]);
}
