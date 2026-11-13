//! Unit tests for `deps/llhttp/api.c`'s items bound ahead of its port: the settings table
//! [`Settings`] through its handles, its constructor and its slots' getters and setters,
//! and the parser's typed setter, [`ParserMut::set_settings`], with the callbacks a parse
//! then runs, decided by Rust assertions.
//!
//! The constructor and the parses call C, and are ignored under Miri; the slots' tests on a
//! zeroed table run there.

use std::cell::RefCell;
use std::ffi::c_int;
use std::sync::Mutex;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::api::{
    HttpParserSettings, Settings, SettingsMut, SettingsRef, SettingsT,
};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Parsed, Parser, ParserMut, ParserType,
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
#[cfg_attr(miri, ignore = "calls C")]
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
