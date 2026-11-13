//! Unit tests for `deps/llhttp/api.c`'s items bound ahead of its port: the settings table
//! [`Settings`] through its handles, its constructor and its slots' getters and setters,
//! and the parser's typed setter, [`ParserMut::set_settings`], with the callbacks a parse
//! then runs, decided by Rust assertions.
//!
//! The parser's API api.c gives, decided the same way: [`ParserMut::execute`], the pause and
//! its two resumptions from each state the parser can be in, inside a parse and between two,
//! the ten lenient setters, each against the bit it owns, and [`ParserRef::debug_trace`]'s
//! write and refusal. llhttp has no C tests in the tree, so none is carried over.
//!
//! The parser's lifecycle: [`ParserMut::init`] and [`ParserMut::reset`], each against the
//! fields it sets and keeps, and [`ParserMut::finish`] from each finish state, its outcomes and
//! errors; and the 23 dispatchers, each against the slot it calls, what it returns, and the
//! span dispatchers' `HPE_USER` and reason.
//!
//! The parses call C, and are ignored under Miri; the tests on a zeroed parser and on tables
//! [`Settings::new`] builds, which calls none since `llhttp_settings_init`'s port, run there.

use std::cell::RefCell;
use std::ffi::c_int;
use std::sync::{LazyLock, Mutex};

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::api::{
    FinishError, HttpParserSettings, Settings, SettingsMut, SettingsRef, SettingsT,
};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Parsed, Parser, ParserMut, ParserRef,
    ParserType,
};
use libgit2_crustify::sys::libgit2 as sys;

/// The slots of `llhttp_settings_s`, in its order: `true` for a data slot.
const SLOTS: [(&str, bool); 23] = [
    ("on_message_begin", false),
    ("on_url", true),
    ("on_status", true),
    ("on_method", true),
    ("on_version", true),
    ("on_header_field", true),
    ("on_header_value", true),
    ("on_chunk_extension_name", true),
    ("on_chunk_extension_value", true),
    ("on_headers_complete", false),
    ("on_body", true),
    ("on_message_complete", false),
    ("on_url_complete", false),
    ("on_status_complete", false),
    ("on_method_complete", false),
    ("on_version_complete", false),
    ("on_header_field_complete", false),
    ("on_header_value_complete", false),
    ("on_chunk_extension_name_complete", false),
    ("on_chunk_extension_value_complete", false),
    ("on_chunk_header", false),
    ("on_chunk_complete", false),
    ("on_reset", false),
];

/// What a slot holds, as an address: `None` for NULL.
type Held = Option<usize>;

/// Calls `$m!(index, getter, setter, kind)` for each slot, in `SLOTS`' order.
macro_rules! for_each_slot {
    ($m:ident) => {
        $m!(0, on_message_begin, set_on_message_begin, event);
        $m!(1, on_url, set_on_url, data);
        $m!(2, on_status, set_on_status, data);
        $m!(3, on_method, set_on_method, data);
        $m!(4, on_version, set_on_version, data);
        $m!(5, on_header_field, set_on_header_field, data);
        $m!(6, on_header_value, set_on_header_value, data);
        $m!(
            7,
            on_chunk_extension_name,
            set_on_chunk_extension_name,
            data
        );
        $m!(
            8,
            on_chunk_extension_value,
            set_on_chunk_extension_value,
            data
        );
        $m!(9, on_headers_complete, set_on_headers_complete, event);
        $m!(10, on_body, set_on_body, data);
        $m!(11, on_message_complete, set_on_message_complete, event);
        $m!(12, on_url_complete, set_on_url_complete, event);
        $m!(13, on_status_complete, set_on_status_complete, event);
        $m!(14, on_method_complete, set_on_method_complete, event);
        $m!(15, on_version_complete, set_on_version_complete, event);
        $m!(
            16,
            on_header_field_complete,
            set_on_header_field_complete,
            event
        );
        $m!(
            17,
            on_header_value_complete,
            set_on_header_value_complete,
            event
        );
        $m!(
            18,
            on_chunk_extension_name_complete,
            set_on_chunk_extension_name_complete,
            event
        );
        $m!(
            19,
            on_chunk_extension_value_complete,
            set_on_chunk_extension_value_complete,
            event
        );
        $m!(20, on_chunk_header, set_on_chunk_header, event);
        $m!(21, on_chunk_complete, set_on_chunk_complete, event);
        $m!(22, on_reset, set_on_reset, event);
    };
}

/// Every slot of `settings`, through its getter.
fn held(settings: SettingsRef<'_>) -> [Held; 23] {
    let mut out = [None; 23];
    macro_rules! read {
        ($i:expr, $get:ident, $set:ident,event) => {
            out[$i] = settings.$get().and_then(Callback::raw).map(|f| f as usize);
        };
        ($i:expr, $get:ident, $set:ident,data) => {
            out[$i] = settings
                .$get()
                .and_then(DataCallback::raw)
                .map(|f| f as usize);
        };
    }
    for_each_slot!(read);
    out
}

thread_local! {
    /// The slots a parse called, by index, with the run each data slot was handed.
    static LOG: RefCell<Vec<(usize, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

/// Takes the log.
fn take_log() -> Vec<(usize, Vec<u8>)> {
    LOG.take()
}

/// The log's slots, by name, consecutive runs of one data slot joined.
fn named(log: &[(usize, Vec<u8>)]) -> Vec<(&'static str, Vec<u8>)> {
    let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();
    for (slot, data) in log {
        let name = SLOTS[*slot].0;
        match out.last_mut() {
            Some((last, joined)) if *last == name && SLOTS[*slot].1 => {
                joined.extend_from_slice(data);
            }
            _ => out.push((name, data.clone())),
        }
    }
    out
}

/// Records that slot `N` was called, and returns `0`.
struct Event<const N: usize>;

impl<const N: usize> EventHandler for Event<N> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        LOG.with_borrow_mut(|log| log.push((N, Vec::new())));
        0
    }
}

/// Records that slot `N` was called with `data`, and returns `0`.
struct Data<const N: usize>;

impl<const N: usize> DataHandler for Data<N> {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        LOG.with_borrow_mut(|log| log.push((N, data.to_vec())));
        0
    }
}

/// A table that lives as long as the test process, for the parsers given it: leaked, its
/// address kept in a global so that the leak checker the address sanitizer runs at exit finds it
/// still reachable.
fn kept() -> &'static mut Settings {
    static KEPT: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let settings: &'static mut Settings = Box::leak(Box::default());
    KEPT.lock()
        .unwrap()
        .push(std::ptr::from_mut(settings).addr());
    settings
}

/// Sets every slot of `m` to its recording handler.
fn record_all(m: &mut SettingsMut<'_>) {
    macro_rules! set {
        ($i:expr, $get:ident, $set:ident,event) => {
            m.$set(Some(Callback::new::<Event<$i>>()));
        };
        ($i:expr, $get:ident, $set:ident,data) => {
            m.$set(Some(DataCallback::new::<Data<$i>>()));
        };
    }
    for_each_slot!(set);
}

/// A table every slot of which records, kept for the parsers given it.
fn recording() -> &'static Settings {
    let settings: &'static mut Settings = kept();
    record_all(&mut settings.as_mut());
    settings
}

#[test]
fn new_and_default_are_empty_tables() {
    for settings in [Settings::new(), Settings::default()] {
        assert_eq!(held(settings.as_ref()), [None; 23]);
        assert_eq!(format!("{settings:?}"), "{}");
    }
}

#[test]
fn a_zeroed_table_is_empty() {
    let settings = Settings::zeroed();
    assert_eq!(held(settings.as_ref()), [None; 23]);
    assert_eq!(format!("{:?}", settings.as_ref()), "{}");
}

#[test]
fn each_setter_sets_its_own_slot_and_none_clears_it() {
    macro_rules! check {
        ($i:expr, $get:ident, $set:ident,event) => {{
            let callback = Callback::new::<Event<$i>>();
            let mut settings = Settings::zeroed();
            settings.as_mut().$set(Some(callback));
            let mut expected = [None; 23];
            expected[$i] = callback.raw().map(|f| f as usize);
            assert_eq!(held(settings.as_ref()), expected, "{}", SLOTS[$i].0);
            assert_eq!(format!("{settings:?}"), format!("{{{:?}}}", SLOTS[$i].0));
            assert!(!SLOTS[$i].1);
            settings.as_mut().$set(None);
            assert_eq!(held(settings.as_ref()), [None; 23], "{}", SLOTS[$i].0);
        }};
        ($i:expr, $get:ident, $set:ident,data) => {{
            let callback = DataCallback::new::<Data<$i>>();
            let mut settings = Settings::zeroed();
            settings.as_mut().$set(Some(callback));
            let mut expected = [None; 23];
            expected[$i] = callback.raw().map(|f| f as usize);
            assert_eq!(held(settings.as_ref()), expected, "{}", SLOTS[$i].0);
            assert_eq!(format!("{settings:?}"), format!("{{{:?}}}", SLOTS[$i].0));
            assert!(SLOTS[$i].1);
            settings.as_mut().$set(None);
            assert_eq!(held(settings.as_ref()), [None; 23], "{}", SLOTS[$i].0);
        }};
    }
    for_each_slot!(check);
}

#[test]
fn a_setter_replaces_what_its_slot_held_and_leaves_the_others() {
    let mut settings = Settings::zeroed();
    record_all(&mut settings.as_mut());
    let before = held(settings.as_ref());
    assert!(before.iter().all(Option::is_some));
    let mut m = settings.as_mut();
    m.set_on_body(Some(DataCallback::new::<Data<99>>()));
    m.set_on_reset(None);
    let after = held(settings.as_ref());
    for i in 0..23 {
        match SLOTS[i].0 {
            "on_body" => assert_ne!(after[i], before[i]),
            "on_reset" => assert_eq!(after[i], None),
            _ => assert_eq!(after[i], before[i], "{}", SLOTS[i].0),
        }
    }
    let debug = format!("{settings:?}");
    assert_eq!(debug.matches('"').count(), 2 * 22);
    assert!(debug.contains("\"on_body\"") && !debug.contains("on_reset"));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_slot_read_back_calls_the_handler_it_was_set_to() {
    let mut settings = Settings::zeroed();
    record_all(&mut settings.as_mut());
    let mut parser = Parser::new();
    let r = settings.as_ref();
    assert_eq!(r.on_message_begin().unwrap().call(parser.as_mut()), 0);
    assert_eq!(r.on_header_value().unwrap().call(parser.as_mut(), b"v"), 0);
    assert_eq!(r.on_reset().unwrap().call(parser.as_mut()), 0);
    assert_eq!(
        take_log(),
        [(0, Vec::new()), (6, b"v".to_vec()), (22, Vec::new())]
    );
}

#[test]
fn the_typedefs_name_the_table() {
    let a: SettingsT = Settings::zeroed();
    let b: HttpParserSettings = a;
    assert_eq!(held(b.as_ref()), [None; 23]);
    assert_eq!(size_of::<Settings>(), size_of::<sys::llhttp_settings_t>());
    assert_eq!(align_of::<Settings>(), align_of::<sys::llhttp_settings_t>());
}

#[test]
fn the_table_is_send_and_sync() {
    fn shared<T: Send + Sync>() {}
    shared::<Settings>();
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_settings_stores_the_tables_address_and_none_clears_it() {
    let settings = recording();
    let mut parser = Parser::new();
    assert!(parser.as_ref().settings_ptr().is_null());
    parser.as_mut().set_settings(Some(settings.as_ref()));
    assert_eq!(
        parser.as_ref().settings_ptr().addr(),
        std::ptr::from_ref(settings).addr()
    );
    parser.as_mut().set_settings(None);
    assert!(parser.as_ref().settings_ptr().is_null());
    // A parse runs no callback without a table.
    assert_eq!(
        parser.as_mut().parse(b"HTTP/1.1 204 No Content\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(take_log(), Vec::new());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_response_runs_its_slots_in_order() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    parser.as_mut().set_settings(Some(recording().as_ref()));
    let input = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A: b\r\n\r\nhello";
    // In runs of three bytes, so that each span crosses runs.
    for run in input.chunks(3) {
        assert_eq!(parser.as_mut().parse(run), Ok(Parsed::All));
    }
    let log = take_log();
    assert_eq!(
        named(&log),
        [
            ("on_message_begin", b"".to_vec()),
            ("on_version", b"1.1".to_vec()),
            ("on_version_complete", b"".to_vec()),
            ("on_status", b"OK".to_vec()),
            ("on_status_complete", b"".to_vec()),
            ("on_header_field", b"Content-Length".to_vec()),
            ("on_header_field_complete", b"".to_vec()),
            ("on_header_value", b"5".to_vec()),
            ("on_header_value_complete", b"".to_vec()),
            ("on_header_field", b"X-A".to_vec()),
            ("on_header_field_complete", b"".to_vec()),
            ("on_header_value", b"b".to_vec()),
            ("on_header_value_complete", b"".to_vec()),
            ("on_headers_complete", b"".to_vec()),
            ("on_body", b"hello".to_vec()),
            ("on_message_complete", b"".to_vec()),
        ]
    );
    // The body came in several runs, one call each.
    assert!(log.iter().filter(|(slot, _)| *slot == 10).count() > 1);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_chunked_request_and_the_next_run_their_slots_in_order() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Request);
    parser.as_mut().set_settings(Some(recording().as_ref()));
    let input = b"POST /p HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n\
                  3;n=v\r\nabc\r\n0\r\n\r\nGET / HTTP/1.1\r\n\r\n";
    assert_eq!(parser.as_mut().parse(input), Ok(Parsed::All));
    let names: Vec<_> = named(&take_log());
    let expected: Vec<(&str, &[u8])> = vec![
        ("on_message_begin", b""),
        ("on_method", b"POST"),
        ("on_method_complete", b""),
        ("on_url", b"/p"),
        ("on_url_complete", b""),
        ("on_version", b"1.1"),
        ("on_version_complete", b""),
        ("on_header_field", b"Transfer-Encoding"),
        ("on_header_field_complete", b""),
        ("on_header_value", b"chunked"),
        ("on_header_value_complete", b""),
        ("on_headers_complete", b""),
        ("on_chunk_extension_name", b"n"),
        ("on_chunk_extension_name_complete", b""),
        ("on_chunk_extension_value", b"v"),
        ("on_chunk_extension_value_complete", b""),
        ("on_chunk_header", b""),
        ("on_body", b"abc"),
        ("on_chunk_complete", b""),
        ("on_chunk_header", b""),
        ("on_chunk_complete", b""),
        ("on_message_complete", b""),
        ("on_reset", b""),
        ("on_message_begin", b""),
        ("on_method", b"GET"),
        ("on_method_complete", b""),
        ("on_url", b"/"),
        ("on_url_complete", b""),
        ("on_version", b"1.1"),
        ("on_version_complete", b""),
        ("on_headers_complete", b""),
        ("on_message_complete", b""),
    ];
    let expected: Vec<_> = expected.into_iter().map(|(n, d)| (n, d.to_vec())).collect();
    assert_eq!(names, expected);
}

/// Returns `R` from `on_headers_complete`.
struct HeadersReturn<const R: c_int>;

impl<const R: c_int> EventHandler for HeadersReturn<R> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        LOG.with_borrow_mut(|log| log.push((9, Vec::new())));
        R
    }
}

/// A kept table that records the body and the end of a message, and answers the end of
/// the headers with `R`.
fn answering<const R: c_int>() -> &'static Settings {
    let settings: &'static mut Settings = kept();
    let mut m = settings.as_mut();
    m.set_on_headers_complete(Some(Callback::new::<HeadersReturn<R>>()));
    m.set_on_body(Some(DataCallback::new::<Data<10>>()));
    m.set_on_message_complete(Some(Callback::new::<Event<11>>()));
    settings
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn on_headers_complete_returning_1_reads_no_body() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    parser
        .as_mut()
        .set_settings(Some(answering::<1>().as_ref()));
    // A response to `HEAD`: its `Content-Length` announces a body that does not follow.
    let input = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n";
    assert_eq!(parser.as_mut().parse(input), Ok(Parsed::All));
    assert_eq!(
        take_log(),
        [
            (9, Vec::new()),
            (11, Vec::new()),
            (9, Vec::new()),
            (11, Vec::new())
        ]
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn on_headers_complete_returning_2_pauses_as_an_upgrade() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    parser
        .as_mut()
        .set_settings(Some(answering::<2>().as_ref()));
    let input = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
    assert_eq!(parser.as_mut().parse(input), Ok(Parsed::PausedUpgrade));
    assert_eq!(take_log(), [(9, Vec::new()), (11, Vec::new())]);
    assert_eq!(parser.as_ref().error(), Ok(Errno::PausedUpgrade));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn on_headers_complete_returning_an_error_stops_the_parse() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    parser
        .as_mut()
        .set_settings(Some(answering::<-1>().as_ref()));
    let error = parser
        .as_mut()
        .parse(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
        .unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::CbHeadersComplete));
    assert_eq!(take_log(), [(9, Vec::new())]);
}

/// Refuses each run with `-1`.
struct RefusingData;

impl DataHandler for RefusingData {
    fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
        -1
    }
}

/// Pauses at each event.
struct Pausing;

impl EventHandler for Pausing {
    fn on_event(_: ParserMut<'_>) -> c_int {
        Errno::Paused.raw().cast_signed()
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_data_slots_minus_one_becomes_hpe_user_with_its_reason() {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_header_field(Some(DataCallback::new::<RefusingData>()));
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(settings.as_ref()));
    let error = parser
        .as_mut()
        .parse(b"HTTP/1.1 200 OK\r\nHost: a\r\n\r\n")
        .unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::User));
    assert_eq!(
        parser.as_ref().reason(),
        Some(c"Span callback error in on_header_field")
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_event_slot_may_pause_the_parse() {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_message_begin(Some(Callback::new::<Pausing>()));
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(settings.as_ref()));
    let input = b"HTTP/1.1 204 No Content\r\n\r\n";
    assert_eq!(parser.as_mut().parse(input), Ok(Parsed::Paused));
    let offset = parser.as_ref().error_offset(input);
    assert_eq!(offset, Some(0));
    parser.as_mut().set_error(Errno::Ok);
    assert_eq!(parser.as_mut().parse(input), Ok(Parsed::All));
    assert_eq!(parser.as_ref().status_code(), 204);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reinit_clears_the_table_as_llhttp_internal_init_does() {
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(recording().as_ref()));
    parser.as_mut().reinit();
    assert!(parser.as_ref().settings_ptr().is_null());
    assert_eq!(
        parser.as_mut().parse(b"HTTP/1.1 204 No Content\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(take_log(), Vec::new());
}

// --- The parser's API ------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn execute_reads_a_message_in_runs_as_parse_does() {
    let input: &[u8] =
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhiHTTP/1.1 204 No Content\r\n\r\n";
    for size in [1, 7, input.len()] {
        let (mut by_execute, mut by_parse) = (Parser::new(), Parser::new());
        for run in input.chunks(size) {
            assert_eq!(
                by_execute.as_mut().execute(run),
                by_parse.as_mut().parse(run)
            );
            assert_eq!(
                by_execute.as_ref().status_code(),
                by_parse.as_ref().status_code()
            );
            assert_eq!(
                by_execute.as_ref().content_length(),
                by_parse.as_ref().content_length()
            );
        }
        assert_eq!(by_execute.as_ref().status_code(), 204);
    }
    let mut parser = Parser::new();
    // A parser of either kind reads a request first, whose method `garbage` is not.
    let error = parser.as_mut().execute(b"garbage").unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::InvalidMethod));
    assert_eq!(parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n"), Err(error));
}

#[test]
fn pause_pauses_a_parser_without_an_error() {
    let mut parser = Parser::zeroed();
    parser.as_mut().pause();
    assert_eq!(parser.as_ref().error(), Ok(Errno::Paused));
    assert_eq!(parser.as_ref().reason(), Some(c"Paused"));
    parser.as_mut().pause();
    assert_eq!(parser.as_ref().error(), Ok(Errno::Paused));
}

#[test]
fn pause_keeps_an_error_and_its_reason() {
    for error in [Errno::PausedUpgrade, Errno::InvalidVersion, Errno::User] {
        let mut parser = Parser::zeroed();
        parser.as_mut().set_error(error);
        parser.as_mut().set_reason(Some(c"kept"));
        parser.as_mut().pause();
        assert_eq!(parser.as_ref().error(), Ok(error));
        assert_eq!(parser.as_ref().reason(), Some(c"kept"));
    }
}

#[test]
fn resume_clears_only_a_pause_and_keeps_the_reason() {
    let mut parser = Parser::zeroed();
    parser.as_mut().pause();
    parser.as_mut().resume();
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
    assert_eq!(parser.as_ref().reason(), Some(c"Paused"));
    for error in [
        Errno::Ok,
        Errno::PausedUpgrade,
        Errno::PausedH2Upgrade,
        Errno::User,
    ] {
        parser.as_mut().set_error(error);
        parser.as_mut().resume();
        assert_eq!(parser.as_ref().error(), Ok(error));
    }
}

#[test]
fn resume_after_upgrade_clears_only_an_upgrades_pause() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_error(Errno::PausedUpgrade);
    parser.as_mut().resume_after_upgrade();
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
    for error in [
        Errno::Ok,
        Errno::Paused,
        Errno::PausedH2Upgrade,
        Errno::User,
    ] {
        parser.as_mut().set_error(error);
        parser.as_mut().resume_after_upgrade();
        assert_eq!(parser.as_ref().error(), Ok(error));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_paused_parser_reads_nothing_until_resumed() {
    let input = b"HTTP/1.1 204 No Content\r\n\r\n";
    let mut parser = Parser::new();
    parser.as_mut().pause();
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::Paused));
    assert_eq!(parser.as_ref().status_code(), 0);
    parser.as_mut().resume_after_upgrade();
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::Paused));
    parser.as_mut().resume();
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::All));
    assert_eq!(parser.as_ref().status_code(), 204);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_parse_paused_by_a_callback_resumes_where_it_stopped() {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_headers_complete(Some(Callback::new::<Pausing>()));
    settings
        .as_mut()
        .set_on_body(Some(DataCallback::new::<Data<10>>()));
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(settings.as_ref()));
    let input = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::Paused));
    let offset = parser.as_ref().error_offset(input).unwrap();
    assert_eq!(&input[offset..], b"hi");
    parser.as_mut().resume();
    assert_eq!(parser.as_mut().execute(&input[offset..]), Ok(Parsed::All));
    assert_eq!(take_log(), [(10, b"hi".to_vec())]);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_upgrade_resumes_after_resume_after_upgrade() {
    let upgrade = b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: x\r\n\r\n";
    let mut parser = Parser::new();
    assert_eq!(parser.as_mut().execute(upgrade), Ok(Parsed::PausedUpgrade));
    assert_eq!(parser.as_ref().error_offset(upgrade), Some(upgrade.len()));
    assert!(parser.as_ref().upgrade());
    parser.as_mut().resume();
    assert_eq!(parser.as_mut().execute(b"x"), Ok(Parsed::PausedUpgrade));
    parser.as_mut().resume_after_upgrade();
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
}

/// Pauses its parser, as libgit2's HTTP client does, but returns `0`.
struct PausingAndGoingOn;

impl EventHandler for PausingAndGoingOn {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.pause();
        0
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_that_pauses_and_returns_0_pauses_the_next_run() {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_message_begin(Some(Callback::new::<PausingAndGoingOn>()));
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(settings.as_ref()));
    let input = b"HTTP/1.1 204 No Content\r\n\r\n";
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::All));
    assert_eq!(parser.as_ref().status_code(), 204);
    assert_eq!(parser.as_ref().error(), Ok(Errno::Paused));
    assert_eq!(parser.as_mut().execute(input), Ok(Parsed::Paused));
}

/// A lenient setter and the `llhttp_lenient_flags` bit it owns.
type Lenient = (fn(&mut ParserMut<'_>, bool), u32);

/// The ten lenient setters, in `llhttp_lenient_flags`' order.
const LENIENT: [Lenient; 10] = [
    (|p, on| p.set_lenient_headers(on), sys::LENIENT_HEADERS),
    (
        |p, on| p.set_lenient_chunked_length(on),
        sys::LENIENT_CHUNKED_LENGTH,
    ),
    (
        |p, on| p.set_lenient_keep_alive(on),
        sys::LENIENT_KEEP_ALIVE,
    ),
    (
        |p, on| p.set_lenient_transfer_encoding(on),
        sys::LENIENT_TRANSFER_ENCODING,
    ),
    (|p, on| p.set_lenient_version(on), sys::LENIENT_VERSION),
    (
        |p, on| p.set_lenient_data_after_close(on),
        sys::LENIENT_DATA_AFTER_CLOSE,
    ),
    (
        |p, on| p.set_lenient_optional_lf_after_cr(on),
        sys::LENIENT_OPTIONAL_LF_AFTER_CR,
    ),
    (
        |p, on| p.set_lenient_optional_crlf_after_chunk(on),
        sys::LENIENT_OPTIONAL_CRLF_AFTER_CHUNK,
    ),
    (
        |p, on| p.set_lenient_optional_cr_before_lf(on),
        sys::LENIENT_OPTIONAL_CR_BEFORE_LF,
    ),
    (
        |p, on| p.set_lenient_spaces_after_chunk_size(on),
        sys::LENIENT_SPACES_AFTER_CHUNK_SIZE,
    ),
];

#[test]
fn each_lenient_setter_sets_and_clears_its_own_bit() {
    for (set, bit) in LENIENT {
        let bit = u16::try_from(bit).unwrap();
        for others in [0, !bit] {
            let mut parser = Parser::zeroed();
            parser.as_mut().set_lenient_flags_raw(others);
            set(&mut parser.as_mut(), true);
            assert_eq!(parser.as_ref().lenient_flags_raw(), others | bit);
            set(&mut parser.as_mut(), true);
            assert_eq!(parser.as_ref().lenient_flags_raw(), others | bit);
            set(&mut parser.as_mut(), false);
            assert_eq!(parser.as_ref().lenient_flags_raw(), others);
            set(&mut parser.as_mut(), false);
            assert_eq!(parser.as_ref().lenient_flags_raw(), others);
        }
    }
}

#[test]
fn the_lenient_setters_together_set_every_flag() {
    let mut parser = Parser::zeroed();
    for (set, _) in LENIENT {
        set(&mut parser.as_mut(), true);
    }
    assert_eq!(parser.as_ref().lenient_flags_raw(), 0x3FF);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_lenient_setter_relaxes_its_check() {
    let input = b"HTTP/3.4 200 OK\r\n\r\n";
    let mut strict = Parser::new();
    assert_eq!(
        strict.as_mut().execute(input).unwrap_err().errno(),
        Ok(Errno::InvalidVersion)
    );
    let mut lenient = Parser::new();
    lenient.as_mut().set_lenient_version(true);
    assert_eq!(lenient.as_mut().execute(input), Ok(Parsed::All));
    assert_eq!(
        (lenient.as_ref().http_major(), lenient.as_ref().http_minor()),
        (3, 4)
    );
}

#[test]
#[cfg_attr(miri, ignore = "reads a C global")]
fn debug_trace_writes_to_stderr_and_refuses_a_nul() {
    let parser = Parser::zeroed();
    let trace: ParserRef<'_> = parser.as_ref();
    trace.debug_trace(b"", "unit_pub: next=null").unwrap();
    trace.debug_trace(b"\x80", "unit_pub: next=80").unwrap();
    let error = trace.debug_trace(b"H", "a\0b").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

// --- The parser's lifecycle --------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn init_starts_a_used_parser_afresh_with_its_type_and_table() {
    let table = recording();
    let mut parser = Parser::new();
    parser.as_mut().set_lenient_headers(true);
    assert!(parser.as_mut().execute(b"garbage").is_err());
    parser
        .as_mut()
        .init(ParserType::Request, Some(table.as_ref()));
    let this = parser.as_ref();
    assert_eq!(this.parser_type(), Ok(ParserType::Request));
    assert_eq!(this.settings_ptr().addr(), std::ptr::from_ref(table).addr());
    assert_eq!(this.error(), Ok(Errno::Ok));
    assert_eq!(this.reason(), None);
    assert_eq!(this.lenient_flags_raw(), 0);
    assert!(this.data().is_null());
    assert_eq!(
        parser.as_mut().execute(b"GET / HTTP/1.1\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(named(&take_log())[0], ("on_message_begin", Vec::new()));
    // A request parser refuses a response, once the table has seen its message begin.
    assert!(parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n").is_err());
    let log = named(&take_log());
    assert_eq!(
        log,
        [("on_reset", Vec::new()), ("on_message_begin", Vec::new())]
    );
    parser.as_mut().init(ParserType::Response, None);
    assert!(parser.as_ref().settings_ptr().is_null());
    assert_eq!(
        parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(take_log(), Vec::new());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reset_keeps_the_type_table_and_lenient_flags_and_clears_the_rest() {
    let table = recording();
    let mut parser = Parser::new();
    parser
        .as_mut()
        .init(ParserType::Response, Some(table.as_ref()));
    parser.as_mut().set_lenient_version(true);
    parser.as_mut().set_lenient_keep_alive(true);
    let flags = parser.as_ref().lenient_flags_raw();
    let error = parser.as_mut().execute(b"GET / HTTP/1.1\r\n").unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::InvalidConstant));
    take_log();
    parser.as_mut().reset();
    let this = parser.as_ref();
    assert_eq!(this.parser_type(), Ok(ParserType::Response));
    assert_eq!(this.settings_ptr().addr(), std::ptr::from_ref(table).addr());
    assert_eq!(this.lenient_flags_raw(), flags);
    assert_eq!(this.error(), Ok(Errno::Ok));
    assert_eq!(this.reason(), None);
    assert_eq!(this.error_offset(b""), None);
    // The kept table runs again, and the kept flag relaxes the version check.
    assert_eq!(
        parser.as_mut().execute(b"HTTP/9.9 204 No Content\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(parser.as_ref().http_major(), 9);
    assert_eq!(
        named(&take_log()).last(),
        Some(&("on_message_complete", Vec::new()))
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reset_keeps_the_type_a_parser_of_both_kinds_settled_on() {
    // A parser of both kinds takes the type of the first message it reads, and keeps it.
    let mut parser = Parser::new();
    assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Both));
    assert_eq!(
        parser.as_mut().execute(b"GET / HTTP/1.1\r\n\r\n"),
        Ok(Parsed::All)
    );
    assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Request));
    parser.as_mut().reset();
    assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Request));
    parser.as_mut().reinit();
    assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Both));
}

// --- Finishing ---------------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_ends_a_message_the_end_of_the_input_marks() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .init(ParserType::Response, Some(recording().as_ref()));
    let response = b"HTTP/1.1 200 OK\r\n\r\nall of it";
    assert_eq!(parser.as_mut().execute(response), Ok(Parsed::All));
    let log = named(&take_log());
    assert_eq!(log.last(), Some(&("on_body", b"all of it".to_vec())));
    assert_eq!(parser.as_mut().finish(), Ok(()));
    assert_eq!(named(&take_log()), [("on_message_complete", Vec::new())]);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_between_messages_runs_no_callback() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .init(ParserType::Response, Some(recording().as_ref()));
    assert_eq!(parser.as_mut().finish(), Ok(()));
    let response = b"HTTP/1.1 204 No Content\r\n\r\n";
    assert_eq!(parser.as_mut().execute(response), Ok(Parsed::All));
    take_log();
    assert_eq!(parser.as_mut().finish(), Ok(()));
    assert_eq!(take_log(), Vec::new());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_inside_a_message_is_an_invalid_eof_state_the_parser_does_not_keep() {
    let mut parser = Parser::new();
    let head = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhi";
    assert_eq!(parser.as_mut().execute(head), Ok(Parsed::All));
    let error = parser.as_mut().finish().unwrap_err();
    assert_eq!(error, FinishError::InvalidEofState);
    assert_eq!(error.code(), 14);
    assert_eq!(error.errno(), Ok(Errno::InvalidEofState));
    assert_eq!(parser.as_ref().reason(), Some(c"Invalid EOF state"));
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
    // The rest of the body may still come.
    assert_eq!(parser.as_mut().execute(b"llo"), Ok(Parsed::All));
    assert_eq!(parser.as_mut().finish(), Ok(()));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_leaves_a_parser_with_an_error_as_it_is() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .init(ParserType::Response, Some(recording().as_ref()));
    assert_eq!(
        parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n\r\nbody"),
        Ok(Parsed::All)
    );
    take_log();
    parser.as_mut().pause();
    assert_eq!(parser.as_mut().finish(), Ok(()));
    assert_eq!(parser.as_ref().error(), Ok(Errno::Paused));
    assert_eq!(take_log(), Vec::new());
    parser.as_mut().resume();
    assert_eq!(parser.as_mut().finish(), Ok(()));
    assert_eq!(named(&take_log()), [("on_message_complete", Vec::new())]);
}

/// Returns `R` from `on_message_complete`.
struct CompleteReturn<const R: c_int>;

impl<const R: c_int> EventHandler for CompleteReturn<R> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        R
    }
}

/// A kept table whose `on_message_complete` returns `R`.
fn completing<const R: c_int>() -> &'static Settings {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_message_complete(Some(Callback::new::<CompleteReturn<R>>()));
    settings
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_returns_on_message_completes_value() {
    let paused = Errno::Paused.raw().cast_signed();
    for (settings, code) in [(completing::<-1>(), -1), (completing::<21>(), paused)] {
        let mut parser = Parser::new();
        parser
            .as_mut()
            .init(ParserType::Response, Some(settings.as_ref()));
        assert_eq!(
            parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n\r\nx"),
            Ok(Parsed::All)
        );
        let error = parser.as_mut().finish().unwrap_err();
        assert_eq!(error, FinishError::Callback(code.try_into().unwrap()));
        assert_eq!(error.code(), code);
        assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
        assert_eq!(parser.as_ref().reason(), None);
    }
    let error = FinishError::Callback((-1).try_into().unwrap());
    assert!(error.errno().is_err());
    assert_eq!(
        error.to_string(),
        "llhttp's on_message_complete refused the end of the input: -1"
    );
    assert_eq!(
        FinishError::InvalidEofState.to_string(),
        "the llhttp parser's input ended inside a message"
    );
}

#[test]
#[should_panic(expected = "finish state is no `llhttp_finish`")]
fn finish_refuses_a_finish_state_no_llhttp_finish_names() {
    let mut parser = Parser::zeroed();
    parser.as_mut().set_finish_raw(3);
    let _ = parser.as_mut().finish();
}

#[test]
fn finish_reads_the_finish_state_of_a_zeroed_parser() {
    let mut parser = Parser::zeroed();
    assert_eq!(parser.as_mut().finish(), Ok(()));
    parser.as_mut().set_finish_raw(2);
    assert_eq!(parser.as_mut().finish(), Err(FinishError::InvalidEofState));
}

// --- The dispatchers ---------------------------------------------------------------------

/// Calls each dispatcher of `parser`, in `SLOTS`' order, each data one with its slot's name,
/// and returns what each returned.
fn dispatch_all(parser: &mut ParserMut<'_>) -> [c_int; 23] {
    let mut out = [0; 23];
    macro_rules! call {
        ($i:expr, $get:ident, $set:ident,event) => {
            out[$i] = parser.$get();
        };
        ($i:expr, $get:ident, $set:ident,data) => {
            out[$i] = parser.$get(stringify!($get).as_bytes());
        };
    }
    for_each_slot!(call);
    out
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn each_dispatcher_calls_its_own_slot_with_its_run() {
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(recording().as_ref()));
    assert_eq!(dispatch_all(&mut parser.as_mut()), [0; 23]);
    let log = take_log();
    let expected: Vec<(usize, Vec<u8>)> = SLOTS
        .iter()
        .enumerate()
        .map(|(i, (name, data))| {
            (
                i,
                if *data {
                    name.as_bytes().to_vec()
                } else {
                    Vec::new()
                },
            )
        })
        .collect();
    assert_eq!(log, expected);
    assert_eq!(parser.as_ref().reason(), None);
}

#[test]
fn a_dispatcher_returns_0_without_a_table_or_a_slot() {
    static EMPTY: LazyLock<Settings> = LazyLock::new(Settings::new);
    let mut parser = Parser::zeroed();
    assert_eq!(dispatch_all(&mut parser.as_mut()), [0; 23]);
    parser.as_mut().set_settings(Some(EMPTY.as_ref()));
    assert_eq!(dispatch_all(&mut parser.as_mut()), [0; 23]);
    assert_eq!(parser.as_ref().reason(), None);
}

/// Returns `R` from every slot, as a data or an event callback.
struct Returning<const R: c_int>;

impl<const R: c_int> EventHandler for Returning<R> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        R
    }
}

impl<const R: c_int> DataHandler for Returning<R> {
    fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
        R
    }
}

/// A table every slot of which returns `R`.
fn returning<const R: c_int>() -> Settings {
    let mut settings = Settings::new();
    let mut m = settings.as_mut();
    macro_rules! set {
        ($i:expr, $get:ident, $set:ident,event) => {
            m.$set(Some(Callback::new::<Returning<R>>()));
        };
        ($i:expr, $get:ident, $set:ident,data) => {
            m.$set(Some(DataCallback::new::<Returning<R>>()));
        };
    }
    for_each_slot!(set);
    settings
}

#[test]
fn a_span_dispatchers_minus_one_is_hpe_user_with_its_slots_reason() {
    static REFUSING: LazyLock<Settings> = LazyLock::new(returning::<-1>);
    let user = Errno::User.raw().cast_signed();
    let mut parser = Parser::zeroed();
    parser.as_mut().set_settings(Some(REFUSING.as_ref()));
    macro_rules! check {
        ($i:expr, $get:ident, $set:ident,event) => {
            parser.as_mut().set_reason(None);
            // `CALLBACK_MAYBE` hands the value back as it is, and gives no reason.
            assert_eq!(parser.as_mut().$get(), -1);
            assert_eq!(parser.as_ref().reason(), None);
        };
        ($i:expr, $get:ident, $set:ident,data) => {
            parser.as_mut().set_reason(None);
            assert_eq!(parser.as_mut().$get(b"x"), user);
            let reason = concat!("Span callback error in ", stringify!($get));
            assert_eq!(
                parser.as_ref().reason().map(|r| r.to_str().unwrap()),
                Some(reason)
            );
        };
    }
    for_each_slot!(check);
    // The parser's error is the parse's to set, from the value: the dispatcher sets none.
    assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
}

#[test]
fn a_dispatcher_hands_back_any_other_value_unchanged() {
    static PAUSING: LazyLock<Settings> = LazyLock::new(returning::<21>);
    static ODD: LazyLock<Settings> = LazyLock::new(returning::<-2>);
    let mut parser = Parser::zeroed();
    for (settings, value) in [(&PAUSING, 21), (&ODD, -2)] {
        parser.as_mut().set_settings(Some(settings.as_ref()));
        assert_eq!(dispatch_all(&mut parser.as_mut()), [value; 23]);
        assert_eq!(parser.as_ref().reason(), None);
    }
}
