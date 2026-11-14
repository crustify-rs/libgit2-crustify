//! Equivalence tests for `deps/llhttp/llhttp.c`'s parser and enums against the C.
//!
//! The constructor and the parse are compared with `llhttp__internal_init` and
//! `llhttp__internal_execute`, called through the raw bindings on a second parser in the
//! same state: [`Parser::new`] and [`ParserMut::reinit`] byte for byte, private state
//! included, and [`ParserMut::parse`] by its outcome, the whole parser after each run, and
//! the callbacks its settings run, which are Rust handlers C calls through
//! [`DataCallback`] and [`Callback`]. The X-macros are compared with what C expands them
//! to, their text in `llhttp.h`.
//!
//! The accessors and maps api.c's getters and name functions are ported onto are compared
//! with those functions in api.c's tests (`api.rs`'s `getters`), which retire with api.c's C:
//! none here calls api.c, so that these keep linking once its exports replace it.

use std::ffi::{CStr, c_int};

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Method, ParseError, Parsed, Parser,
    ParserMut, ParserRef, Status,
};
use libgit2_crustify::sys::libgit2 as sys;

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
