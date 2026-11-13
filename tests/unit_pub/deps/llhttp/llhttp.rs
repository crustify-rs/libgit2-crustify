//! Unit tests for `deps/llhttp/llhttp.c`'s items, bound ahead of their consumers: the four
//! enums' conversions and their errors, the parser [`Parser`] through its handles, its
//! constructor and its parse, the two callback types, and the X-macros' entries, decided by
//! Rust assertions.
//!
//! The parses call C, and are ignored under Miri; the rest runs there.

use std::collections::HashSet;
use std::error::Error as _;
use std::ffi::{CStr, c_int};

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, ErrnoT, EventHandler, HttpParser, InternalT,
    LlhttpT, Method, MethodT, ParseError, Parsed, Parser, ParserMut, ParserRef, ParserType,
    ParserTypeT, Status, StatusT, UnnamedErrno, UnnamedMethod, UnnamedParserType, UnnamedStatus,
};
use libgit2_crustify::sys::libgit2 as sys;

/// The values in `0..=limit` that `from_raw` names, each checked to convert back to itself.
fn named<T: Copy>(limit: u32, from_raw: fn(u32) -> Option<T>, raw: fn(T) -> u32) -> Vec<u32> {
    (0..=limit)
        .chain([u32::MAX])
        .filter(|&value| {
            from_raw(value).is_some_and(|variant| {
                assert_eq!(raw(variant), value);
                true
            })
        })
        .collect()
}

#[test]
fn errno_names_the_36_codes_of_llhttp_errno() {
    let codes = named(1024, Errno::from_raw, Errno::raw);
    // `HPE_OK` .. `HPE_CB_CHUNK_EXTENSION_VALUE_COMPLETE`: `0..=35`, none missing.
    assert_eq!(codes, (0..=35).collect::<Vec<_>>());
    assert_eq!(Errno::from_raw(sys::HPE_OK), Some(Errno::Ok));
    assert_eq!(Errno::from_raw(21), Some(Errno::Paused));
    assert_eq!(Errno::from_raw(22), Some(Errno::PausedUpgrade));
    assert_eq!(Errno::from_raw(24), Some(Errno::User));
    assert_eq!(Errno::from_raw(25), Some(Errno::CrExpected));
    assert_eq!(Errno::from_raw(31), Some(Errno::CbReset));
    assert_eq!(Errno::InvalidEofState.raw(), 14);
    assert_eq!(Errno::CbChunkExtensionValueComplete.raw(), 35);
}

#[test]
fn method_names_the_47_methods_of_llhttp_method() {
    let codes = named(1024, Method::from_raw, Method::raw);
    assert_eq!(codes, (0..=46).collect::<Vec<_>>());
    assert_eq!(Method::from_raw(0), Some(Method::Delete));
    assert_eq!(Method::from_raw(1), Some(Method::Get));
    assert_eq!(Method::from_raw(5), Some(Method::Connect));
    assert_eq!(Method::from_raw(24), Some(Method::MSearch));
    assert_eq!(Method::from_raw(34), Some(Method::Pri));
    assert_eq!(Method::from_raw(41), Some(Method::GetParameter));
    assert_eq!(Method::Query.raw(), 46);
}

#[test]
fn status_names_the_99_codes_of_llhttp_status_by_their_value() {
    let codes = named(1024, Status::from_raw, Status::raw);
    assert_eq!(codes.len(), 99);
    assert_eq!(codes.first(), Some(&100));
    assert_eq!(codes.last(), Some(&599));
    for code in [100, 101, 200, 204, 301, 304, 404, 418, 500, 599] {
        assert!(codes.contains(&code), "{code}");
    }
    for code in [0, 1, 99, 104, 209, 309, 427, 600, 999] {
        assert!(!codes.contains(&code), "{code}");
    }
    assert_eq!(Status::from_raw(200), Some(Status::Ok));
    assert_eq!(Status::from_raw(418), Some(Status::ImATeapot));
    assert_eq!(Status::from_raw(524), Some(Status::TimeoutOccured));
    assert_eq!(Status::NotFound.raw(), 404);
}

#[test]
fn parser_type_names_both_request_and_response() {
    assert_eq!(
        named(1024, ParserType::from_raw, ParserType::raw),
        [0, 1, 2]
    );
    assert_eq!(ParserType::from_raw(0), Some(ParserType::Both));
    assert_eq!(ParserType::from_raw(1), Some(ParserType::Request));
    assert_eq!(ParserType::from_raw(2), Some(ParserType::Response));
}

#[test]
fn enums_convert_to_and_from_their_c_integer() {
    assert_eq!(Errno::try_from(sys::HPE_PAUSED), Ok(Errno::Paused));
    assert_eq!(sys::llhttp_errno::from(Errno::User), sys::HPE_USER);
    assert_eq!(Method::try_from(sys::HTTP_POST), Ok(Method::Post));
    assert_eq!(sys::llhttp_method::from(Method::Head), sys::HTTP_HEAD);
    assert_eq!(Status::try_from(sys::HTTP_STATUS_OK), Ok(Status::Ok));
    assert_eq!(sys::llhttp_status::from(Status::Gone), 410);
    assert_eq!(
        ParserType::try_from(sys::HTTP_RESPONSE),
        Ok(ParserType::Response)
    );
    assert_eq!(
        sys::llhttp_type::from(ParserType::Request),
        sys::HTTP_REQUEST
    );
    // The typedefs' names are the enums.
    let errno: ErrnoT = Errno::Ok;
    let method: MethodT = Method::Get;
    let status: StatusT = Status::Continue;
    let parser_type: ParserTypeT = ParserType::Both;
    assert_eq!(
        (errno, method, status, parser_type),
        (Errno::Ok, Method::Get, Status::Continue, ParserType::Both)
    );
}

#[test]
fn unnamed_values_are_refused_with_their_value() {
    let err = Errno::try_from(36).unwrap_err();
    let _: UnnamedErrno = err;
    assert_eq!(err.raw(), 36);
    assert_eq!(err.to_string(), "llhttp errno 36 is not named by llhttp.h");
    assert!(err.source().is_none());

    let err = Method::try_from(47).unwrap_err();
    assert_eq!(err.raw(), 47);
    assert_eq!(err.to_string(), "llhttp method 47 is not named by llhttp.h");
    let _: UnnamedMethod = err;

    let err = Status::try_from(999).unwrap_err();
    assert_eq!(err.raw(), 999);
    assert_eq!(
        err.to_string(),
        "llhttp status 999 is not named by llhttp.h"
    );
    let _: UnnamedStatus = err;

    let err = ParserType::try_from(3).unwrap_err();
    assert_eq!(err.raw(), 3);
    assert_eq!(
        err.to_string(),
        "llhttp parser type 3 is not named by llhttp.h"
    );
    let _: UnnamedParserType = err;

    // Errors of one value are equal, and hash alike.
    let set: HashSet<_> = [
        Status::try_from(1).unwrap_err(),
        Status::try_from(1).unwrap_err(),
    ]
    .into();
    assert_eq!(set.len(), 1);
}

#[test]
fn a_zeroed_parser_reads_as_zero() {
    let parser = Parser::zeroed();
    let p = parser.as_ref();
    assert_eq!(p.parser_type(), Ok(ParserType::Both));
    assert_eq!(p.parser_type_raw(), 0);
    assert_eq!(p.error(), Ok(Errno::Ok));
    assert_eq!(p.error_raw(), 0);
    assert_eq!(p.method(), Ok(Method::Delete));
    assert_eq!(p.method_raw(), 0);
    assert_eq!(p.reason(), None);
    assert_eq!(p.error_offset(b"HTTP/1.1"), None);
    assert_eq!(p.error_offset(&[]), None);
    assert!(p.data().is_null());
    assert!(p.settings_ptr().is_null());
    assert_eq!(p.content_length(), 0);
    assert_eq!((p.http_major(), p.http_minor()), (0, 0));
    assert_eq!(p.status_code(), 0);
    assert!(!p.upgrade());
    assert_eq!(p.finish_raw(), 0);
    assert_eq!(p.flags_raw(), 0);
    assert_eq!(p.lenient_flags_raw(), 0);
}

#[test]
fn the_parser_has_its_c_layout_and_its_typedefs_name_it() {
    assert_eq!(size_of::<Parser>(), size_of::<sys::llhttp__internal_s>());
    assert_eq!(align_of::<Parser>(), align_of::<sys::llhttp__internal_s>());
    let a: InternalT = Parser::zeroed();
    let b: LlhttpT = a;
    let c: HttpParser = b;
    assert_eq!(c.as_ref().status_code(), 0);
}

#[test]
fn each_setter_writes_what_its_getter_reads() {
    let mut parser = Parser::zeroed();
    let mut m: ParserMut<'_> = parser.as_mut();

    m.set_parser_type(ParserType::Response);
    m.set_error(Errno::Paused);
    m.set_reason(Some(c"Paused"));
    m.set_lenient_flags_raw(0x0201);
    m.set_upgrade(true);
    m.set_finish_raw(2);
    m.set_flags_raw(0x0228);

    let r = m.as_ref();
    assert_eq!(r.parser_type(), Ok(ParserType::Response));
    assert_eq!(r.parser_type_raw(), 2);
    assert_eq!(r.error(), Ok(Errno::Paused));
    assert_eq!(r.error_raw(), 21);
    assert_eq!(r.reason(), Some(c"Paused"));
    assert_eq!(r.lenient_flags_raw(), 0x0201);
    assert!(r.upgrade());
    assert_eq!(r.finish_raw(), 2);
    assert_eq!(r.flags_raw(), 0x0228);
    // The fields no setter writes are untouched.
    assert_eq!(r.method_raw(), 0);
    assert_eq!(r.status_code(), 0);
    assert_eq!(r.content_length(), 0);
    assert!(r.data().is_null());
    assert!(r.settings_ptr().is_null());

    // Each setter replaces what it set before.
    m.set_parser_type(ParserType::Request);
    m.set_error(Errno::Ok);
    m.set_reason(None);
    m.set_lenient_flags_raw(0);
    m.set_upgrade(false);
    m.set_finish_raw(0);
    m.set_flags_raw(u16::MAX);
    let r = parser.as_ref();
    assert_eq!(r.parser_type(), Ok(ParserType::Request));
    assert_eq!(r.error(), Ok(Errno::Ok));
    assert_eq!(r.reason(), None);
    assert_eq!(r.lenient_flags_raw(), 0);
    assert!(!r.upgrade());
    assert_eq!(r.finish_raw(), 0);
    assert_eq!(r.flags_raw(), u16::MAX);
}

#[test]
fn every_parser_type_and_errno_round_trips_through_its_field() {
    let mut parser = Parser::zeroed();
    for raw in 0..=2 {
        let parser_type = ParserType::from_raw(raw).unwrap();
        parser.as_mut().set_parser_type(parser_type);
        assert_eq!(parser.as_ref().parser_type(), Ok(parser_type));
        assert_eq!(u32::from(parser.as_ref().parser_type_raw()), raw);
    }
    for raw in 0..=35 {
        let errno = Errno::from_raw(raw).unwrap();
        parser.as_mut().set_error(errno);
        assert_eq!(parser.as_ref().error(), Ok(errno));
        assert_eq!(parser.as_ref().error_raw(), i32::try_from(raw).unwrap());
    }
}

#[test]
fn shared_handles_copy_and_read_the_same_parser() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_reason(Some(c"Invalid EOF state"));
    let a: ParserRef<'_> = parser.as_ref();
    let b = a;
    assert_eq!(a.reason(), b.reason());
    // A reason outlives the handle it was read through, for the parser's borrow.
    let reason = {
        let handle = parser.as_ref();
        handle.reason()
    };
    assert_eq!(reason, Some(c"Invalid EOF state"));
}

#[test]
fn a_parser_moves_with_its_state() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_error(Errno::User);
    let moved = Box::new(parser);
    assert_eq!(Parser::as_ref(&moved).error(), Ok(Errno::User));
}

// --- The X-macros ----------------------------------------------------------------------

/// Checks one X-macro's entries: `all` in order, each entry's `NAME` its constant's name less
/// `prefix`, its `STRING` its `NAME` but where `strings` gives another, and every text a
/// C identifier's or a method's spelling.
fn check_map<T: Copy + PartialEq + std::fmt::Debug>(
    map: &[T],
    all: &[T],
    prefix: &str,
    columns: impl Fn(T) -> (&'static CStr, &'static CStr, &'static CStr),
    strings: &[(T, &CStr)],
) {
    assert_eq!(map, all);
    for &entry in map {
        let (name, string, constant) = columns(entry);
        let (name, constant) = (name.to_str().unwrap(), constant.to_str().unwrap());
        assert_eq!(constant, format!("{prefix}{name}"), "{entry:?}");
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
            "{entry:?}: {name}"
        );
        let expected = strings
            .iter()
            .find(|(e, _)| *e == entry)
            .map_or(name.as_bytes(), |(_, s)| s.to_bytes());
        assert_eq!(string.to_bytes(), expected, "{entry:?}");
    }
}

#[test]
fn errno_map_lists_every_errno_as_hpe_constants() {
    let all: Vec<Errno> = named(64, Errno::from_raw, Errno::raw)
        .into_iter()
        .map(|raw| Errno::from_raw(raw).unwrap())
        .collect();
    let mut sorted = Errno::MAP.to_vec();
    sorted.sort_by_key(|e| e.raw());
    assert_eq!(sorted, all);
    check_map(
        Errno::MAP,
        Errno::MAP,
        "HPE_",
        |e| (e.map_name(), e.map_string(), e.constant_name()),
        &[],
    );
    // The map's order is the header's, not the codes': CR_EXPECTED (25) follows STRICT (2).
    assert_eq!(
        &Errno::MAP[..4],
        &[Errno::Ok, Errno::Internal, Errno::Strict, Errno::CrExpected]
    );
    assert_eq!(Errno::CbReset.constant_name(), c"HPE_CB_RESET");
    assert_eq!(Errno::MAP.last(), Some(&Errno::CbReset));
}

#[test]
fn all_method_map_lists_every_method_and_spells_m_search() {
    let all: Vec<Method> = (0..47).map(|raw| Method::from_raw(raw).unwrap()).collect();
    check_map(
        Method::MAP,
        &all,
        "HTTP_",
        |m| (m.map_name(), m.map_string(), m.constant_name()),
        &[(Method::MSearch, c"M-SEARCH")],
    );
    assert_eq!(Method::Get.map_string(), c"GET");
    assert_eq!(Method::Query.constant_name(), c"HTTP_QUERY");
}

#[test]
fn status_map_lists_every_status_by_increasing_code() {
    let all: Vec<Status> = named(999, Status::from_raw, Status::raw)
        .into_iter()
        .map(|raw| Status::from_raw(raw).unwrap())
        .collect();
    check_map(
        Status::MAP,
        &all,
        "HTTP_STATUS_",
        |s| (s.map_name(), s.map_string(), s.constant_name()),
        &[],
    );
    assert_eq!(Status::NotFound.map_string(), c"NOT_FOUND");
    assert_eq!(Status::MAP.last().map(|s| s.raw()), Some(599));
}

// --- The parser ------------------------------------------------------------------------

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody";

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn new_is_a_parser_at_its_start() {
    let parser = Parser::new();
    let r = parser.as_ref();
    assert_eq!(r.parser_type(), Ok(ParserType::Both));
    assert_eq!(r.error(), Ok(Errno::Ok));
    assert_eq!(r.reason(), None);
    assert!(r.data().is_null());
    assert!(r.settings_ptr().is_null());
    assert_eq!(
        (
            r.content_length(),
            r.http_major(),
            r.http_minor(),
            r.status_code()
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        (r.flags_raw(), r.lenient_flags_raw(), r.finish_raw()),
        (0, 0, 0)
    );
    assert!(!r.upgrade());
    let default = Parser::default();
    assert_eq!(default.as_ref().error(), Ok(Errno::Ok));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_response_parses_in_any_number_of_runs() {
    for size in 1..=RESPONSE.len() {
        let mut parser = Parser::new();
        parser.as_mut().set_parser_type(ParserType::Response);
        for run in RESPONSE.chunks(size) {
            assert_eq!(
                parser.as_mut().parse(run),
                Ok(Parsed::All),
                "runs of {size}"
            );
        }
        let r = parser.as_ref();
        assert_eq!(
            (r.status_code(), r.http_major(), r.http_minor()),
            (200, 1, 1)
        );
        assert_eq!(r.content_length(), 0, "the body was read");
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_empty_run_parses_nothing() {
    let mut parser = Parser::new();
    assert_eq!(parser.as_mut().parse(&[]), Ok(Parsed::All));
    assert_eq!(parser.as_mut().parse(b"HTTP/1.1 2"), Ok(Parsed::All));
    assert_eq!(parser.as_mut().parse(&[]), Ok(Parsed::All));
    assert_eq!(
        parser.as_mut().parse(b"04 No Content\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(parser.as_ref().status_code(), 204);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_error_is_kept_until_the_parser_is_initialised_again() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Request);
    let error = parser
        .as_mut()
        .parse(b"GET / HTTP/1.1\r\nA B: c\r\n")
        .unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::InvalidHeaderToken));
    assert_eq!(error.code(), 10);
    assert_eq!(parser.as_ref().error(), Ok(Errno::InvalidHeaderToken));
    assert!(parser.as_ref().reason().is_some());
    assert_eq!(parser.as_mut().parse(b"GET / HTTP/1.1\r\n\r\n"), Err(error));
    parser.as_mut().reinit();
    assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Both));
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
    assert_eq!(parser.as_ref().reason(), None);
    assert_eq!(
        parser.as_mut().parse(b"GET / HTTP/1.1\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(parser.as_ref().method(), Ok(Method::Get));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_paused_parser_reads_nothing_until_resumed() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.as_mut().parse(b"HTTP/1.1 200 OK\r\n"),
        Ok(Parsed::All)
    );
    parser.as_mut().set_error(Errno::Paused);
    assert_eq!(parser.as_mut().parse(b"A: b\r\n"), Ok(Parsed::Paused));
    assert_eq!(parser.as_mut().parse(b"A: b\r\n"), Ok(Parsed::Paused));
    parser.as_mut().set_error(Errno::Ok);
    assert_eq!(
        parser.as_mut().parse(b"Content-Length: 0\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(parser.as_ref().status_code(), 200);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_upgrade_pauses_where_the_new_protocol_starts() {
    let request = b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\nTLS";
    let mut parser = Parser::new();
    assert_eq!(parser.as_mut().parse(request), Ok(Parsed::PausedUpgrade));
    assert!(parser.as_ref().upgrade());
    assert_eq!(parser.as_ref().method(), Ok(Method::Connect));
    assert_eq!(
        parser.as_ref().error_offset(request),
        Some(request.len() - 3)
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_http2_preface_stops_the_parse_for_good() {
    let mut parser = Parser::new();
    let error = parser
        .as_mut()
        .parse(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
        .unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::PausedH2Upgrade));
    assert_eq!(parser.as_mut().parse(b"x"), Err(error));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
#[should_panic(expected = "no state to run from")]
fn a_parser_cleared_of_an_error_it_cannot_resume_from_panics() {
    let mut parser = Parser::new();
    let _ = parser.as_mut().parse(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    parser.as_mut().set_error(Errno::Ok);
    let _ = parser.as_mut().parse(b"x");
}

#[test]
#[should_panic(expected = "no state to run from")]
fn a_zeroed_parser_does_not_parse() {
    let mut parser = Parser::zeroed();
    let _ = parser.as_mut().parse(b"GET / HTTP/1.1\r\n");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_zeroed_parser_with_an_error_returns_it() {
    // C returns a kept error before it runs, so a parser with an error has no state to need.
    let mut parser = Parser::zeroed();
    parser.as_mut().set_error(Errno::Paused);
    assert_eq!(parser.as_mut().parse(b"x"), Ok(Parsed::Paused));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_parse_error_names_its_errno() {
    // A callback's own value, which no errno names, is the equivalence tests': only C sets
    // one.
    let mut parser = Parser::zeroed();
    parser.as_mut().set_error(Errno::User);
    let error: ParseError = parser.as_mut().parse(b"x").unwrap_err();
    assert_eq!((error.code(), error.errno()), (24, Ok(Errno::User)));
    assert_eq!(error.to_string(), "llhttp stopped at HPE_USER");
    assert!(error.source().is_none());
    // Every kept error is returned as it is, but the pauses, which are no error.
    for &errno in &Errno::MAP[1..] {
        parser.as_mut().set_error(errno);
        match parser.as_mut().parse(b"x") {
            Ok(parsed) => assert!(matches!(
                (errno, parsed),
                (Errno::Paused, Parsed::Paused) | (Errno::PausedUpgrade, Parsed::PausedUpgrade)
            )),
            Err(error) => assert_eq!(error.errno(), Ok(errno)),
        }
    }
}

// --- Callbacks -------------------------------------------------------------------------

struct Upgrading;

impl EventHandler for Upgrading {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.set_upgrade(true);
        7
    }
}

struct Panicking;

impl EventHandler for Panicking {
    fn on_event(_: ParserMut<'_>) -> c_int {
        panic!("a handler's panic");
    }
}

impl DataHandler for Panicking {
    fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
        panic!("a handler's panic");
    }
}

/// Returns the length of the run it is handed, and records it as the parser's content
/// length's stand-in, the flags word.
struct Measuring;

impl DataHandler for Measuring {
    fn on_data(mut parser: ParserMut<'_>, data: &[u8]) -> c_int {
        parser.set_flags_raw(u16::try_from(data.len()).unwrap());
        c_int::try_from(data.iter().map(|&b| u32::from(b)).sum::<u32>()).unwrap()
    }
}

#[test]
fn a_callback_hands_its_handler_the_parser_and_returns_its_value() {
    let callback = Callback::new::<Upgrading>();
    let copy = callback;
    let mut parser = Parser::zeroed();
    assert_eq!(copy.call(parser.as_mut()), 7);
    assert!(parser.as_ref().upgrade());
    assert!(callback.raw().is_some());
    assert!(format!("{callback:?}").starts_with("Callback("));
}

#[test]
fn a_data_callback_hands_its_handler_the_run() {
    let callback = DataCallback::new::<Measuring>();
    let mut parser = Parser::zeroed();
    assert_eq!(callback.call(parser.as_mut(), b"\x01\x02\x03"), 6);
    assert_eq!(parser.as_ref().flags_raw(), 3);
    assert_eq!(callback.call(parser.as_mut(), &[]), 0);
    assert_eq!(parser.as_ref().flags_raw(), 0);
    assert!(callback.raw().is_some());
    assert!(format!("{callback:?}").starts_with("DataCallback("));
}

#[test]
fn a_handlers_panic_is_the_callbacks_error() {
    let mut parser = Parser::zeroed();
    let event = Callback::new::<Panicking>().call(parser.as_mut());
    let data = DataCallback::new::<Panicking>().call(parser.as_mut(), b"x");
    assert_eq!((event, data), (-1, -1));
}
