//! UB tests for `deps/llhttp/api.c`'s settings table on the safe API: tables built,
//! filled and read through every handle, installed in parsers with
//! [`ParserMut::set_settings`] and run by C's dispatchers on every event, shared between
//! threads from a `static`, replaced and cleared from inside their own callbacks, and
//! parsers moved while they hold a table.
//!
//! The illegal paths, a table that does not live as long as the program given to a parser
//! and a table written while a parser holds it, are `Settings`' and
//! `ParserMut::set_settings`' compile-fail doctests. The parser's API api.c gives is run
//! from inside the callbacks of a parse, on the parser being parsed: the pause, the
//! resumptions, the lenient setters and the trace, which change what the parse reads next
//! but not where it stands, and [`ParserMut::execute`], which is refused there, as
//! `ParserMut::parse` is. So are [`ParserMut::init`] and [`ParserMut::reset`], which would
//! restart the parse under it; the dispatchers and [`ParserMut::finish`], which call the
//! parser's table with the parser and leave where the parse stands, run there. The tests
//! that parse call C, and are ignored under Miri; the rest runs there.

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::sync::{LazyLock, Mutex};
use std::thread;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::api::{Settings, SettingsMut};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Parsed, Parser, ParserMut, ParserType,
};

thread_local! {
    /// The bytes of the body this thread's parses read.
    static BODY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The messages this thread's parses completed.
    static COMPLETE: Cell<u32> = const { Cell::new(0) };
}

/// Records each run of the body.
struct Body;

impl DataHandler for Body {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        BODY.with_borrow_mut(|body| body.extend_from_slice(data));
        0
    }
}

/// Counts each message.
struct Complete;

impl EventHandler for Complete {
    fn on_event(_: ParserMut<'_>) -> c_int {
        COMPLETE.set(COMPLETE.get() + 1);
        0
    }
}

/// Sets the two recording slots of `m`.
fn record(m: &mut SettingsMut<'_>) {
    m.set_on_body(Some(DataCallback::new::<Body>()));
    m.set_on_message_complete(Some(Callback::new::<Complete>()));
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

/// The table the threads share.
static SHARED: LazyLock<Settings> = LazyLock::new(|| {
    let mut settings = Settings::new();
    record(&mut settings.as_mut());
    settings
});

/// A response whose body is `body`.
fn response(body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

#[test]
fn handles_alternate_over_a_zeroed_table() {
    let mut settings = Settings::zeroed();
    let mut m = settings.as_mut();
    record(&mut m);
    let shared = m.as_ref();
    assert!(shared.on_body().is_some());
    let mut inner = m.as_mut();
    inner.set_on_body(None);
    assert!(m.as_ref().on_body().is_none());
    assert!(settings.as_ref().on_message_complete().is_some());
    // Tables in a `Vec`, written and read independently, and moved as it grows.
    let mut tables: Vec<Settings> = Vec::new();
    for i in 0..64 {
        let mut table = Settings::zeroed();
        if i % 2 == 0 {
            record(&mut table.as_mut());
        }
        tables.push(table);
    }
    for (i, table) in tables.iter().enumerate() {
        assert_eq!(table.as_ref().on_body().is_some(), i % 2 == 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn threads_share_a_static_table_each_with_its_own_parser() {
    let workers: Vec<_> = (0..8u8)
        .map(|seed| {
            thread::spawn(move || {
                let body = vec![b'a' + seed; 64 + usize::from(seed)];
                let input = response(&body);
                let mut parser = Parser::new();
                parser.as_mut().set_parser_type(ParserType::Response);
                parser.as_mut().set_settings(Some(SHARED.as_ref()));
                for _ in 0..16 {
                    for run in input.chunks(7) {
                        assert_eq!(parser.as_mut().parse(run), Ok(Parsed::All));
                    }
                }
                // The table is read from this thread too while the others parse with it.
                assert!(SHARED.as_ref().on_body().is_some());
                (COMPLETE.get(), BODY.take() == body.repeat(16))
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), (16, true));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_parser_holding_a_table_moves_between_runs() {
    let input = response(b"moved about");
    let mut parsers: Vec<Parser> = Vec::new();
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(SHARED.as_ref()));
    for run in input.chunks(5) {
        assert_eq!(parser.as_mut().parse(run), Ok(Parsed::All));
        // Moved into a growing `Vec` and out again.
        parsers.push(parser);
        parsers.extend((0..8).map(|_| Parser::new()));
        parser = parsers.swap_remove(0);
        parsers.clear();
    }
    assert_eq!(BODY.take(), b"moved about");
    assert_eq!(COMPLETE.replace(0), 1);
}

/// The table [`Swapping`] installs, which only counts messages.
static COUNTING: LazyLock<Settings> = LazyLock::new(|| {
    let mut settings = Settings::new();
    settings
        .as_mut()
        .set_on_message_complete(Some(Callback::new::<Complete>()));
    settings
});

/// Replaces its parser's table with [`COUNTING`] at the end of the headers.
struct Swapping;

impl EventHandler for Swapping {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.set_settings(Some(COUNTING.as_ref()));
        0
    }
}

/// Clears its parser's table at the end of the headers.
struct Clearing;

impl EventHandler for Clearing {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.set_settings(None);
        0
    }
}

/// A kept table that records the body and the end of a message, and runs `H` at the end
/// of the headers.
fn headers_then<H: EventHandler>() -> &'static Settings {
    let settings: &'static mut Settings = kept();
    let mut m = settings.as_mut();
    record(&mut m);
    m.set_on_headers_complete(Some(Callback::new::<H>()));
    settings
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_replaces_its_parsers_table_mid_parse() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .set_settings(Some(headers_then::<Swapping>().as_ref()));
    // The body runs after the swap, under a table without `on_body`.
    assert_eq!(parser.as_mut().parse(&response(b"unseen")), Ok(Parsed::All));
    assert_eq!(BODY.take(), b"");
    assert_eq!(COMPLETE.replace(0), 1);
    assert_eq!(
        parser.as_ref().settings_ptr().addr(),
        std::ptr::from_ref::<Settings>(&COUNTING).addr()
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_clears_its_parsers_table_mid_parse() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .set_settings(Some(headers_then::<Clearing>().as_ref()));
    assert_eq!(parser.as_mut().parse(&response(b"unseen")), Ok(Parsed::All));
    assert_eq!(BODY.take(), b"");
    assert_eq!(COMPLETE.get(), 0);
    assert!(parser.as_ref().settings_ptr().is_null());
}

/// Calls its parser's own body slot again, from the table it reads, with its run.
struct Echoing;

impl DataHandler for Echoing {
    fn on_data(parser: ParserMut<'_>, data: &[u8]) -> c_int {
        // A slot read back from a table is callable from inside another slot's callback,
        // with the parser being parsed.
        let body = SHARED.as_ref().on_body().unwrap();
        body.call(parser, data)
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_calls_a_slot_read_back_from_a_table_with_its_parser() {
    let settings: &'static mut Settings = kept();
    settings
        .as_mut()
        .set_on_body(Some(DataCallback::new::<Echoing>()));
    let mut parser = Parser::new();
    parser.as_mut().set_settings(Some(settings.as_ref()));
    let input = response(b"echo");
    for run in input.chunks(2) {
        assert_eq!(parser.as_mut().parse(run), Ok(Parsed::All));
    }
    assert_eq!(BODY.take(), b"echo");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parsers_given_different_tables_run_their_own() {
    let mut a = Parser::new();
    let mut b = Parser::new();
    a.as_mut().set_settings(Some(SHARED.as_ref()));
    b.as_mut().set_settings(Some(COUNTING.as_ref()));
    let input = response(b"ab");
    // Interleaved, a run each.
    for run in input.chunks(3) {
        assert_eq!(a.as_mut().parse(run), Ok(Parsed::All));
        assert_eq!(b.as_mut().parse(run), Ok(Parsed::All));
    }
    assert_eq!(BODY.take(), b"ab");
    assert_eq!(COMPLETE.replace(0), 2);
}

// --- The parser's API inside a parse ---------------------------------------------------

/// Pauses, resumes, relaxes every check and traces its parser at each event, then goes on.
struct Fidgeting;

impl EventHandler for Fidgeting {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.pause();
        parser.resume_after_upgrade();
        parser.resume();
        parser.set_lenient_headers(true);
        parser.set_lenient_optional_cr_before_lf(true);
        parser.set_lenient_headers(false);
        drop(parser.as_ref().debug_trace(b"", "ub_safe: inside a parse"));
        0
    }
}

/// Executes its own parser again, which is refused, and returns what the refusal gave.
struct Reexecuting;

impl EventHandler for Reexecuting {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        let _ = parser.execute(b"HTTP/1.1 200 OK\r\n\r\n");
        0
    }
}

/// A kept table whose every event slot calls `H`.
fn every_event<H: EventHandler>() -> &'static Settings {
    let settings = kept();
    let mut m = settings.as_mut();
    let callback = Some(Callback::new::<H>());
    m.set_on_message_begin(callback);
    m.set_on_headers_complete(callback);
    m.set_on_header_field_complete(callback);
    m.set_on_header_value_complete(callback);
    m.set_on_chunk_header(callback);
    m.set_on_chunk_complete(callback);
    m.set_on_message_complete(callback);
    settings
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn callbacks_pause_resume_and_relax_their_parser_mid_parse() {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    parser
        .as_mut()
        .set_settings(Some(every_event::<Fidgeting>().as_ref()));
    let input = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
    for run in input.chunks(3) {
        assert_eq!(parser.as_mut().execute(run), Ok(Parsed::All));
    }
    assert_eq!(parser.as_ref().status_code(), 200);
    assert_eq!(parser.as_ref().lenient_flags_raw(), 0x100);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_cannot_execute_the_parser_whose_parse_runs_it() {
    let mut parser = Parser::new();
    parser
        .as_mut()
        .set_settings(Some(every_event::<Reexecuting>().as_ref()));
    // The nested `execute` panics, and the trampoline turns the panic into the callback's
    // `-1`, which stops the outer parse with `HPE_CB_MESSAGE_BEGIN`.
    let error = parser
        .as_mut()
        .execute(b"HTTP/1.1 204 No Content\r\n\r\n")
        .unwrap_err();
    assert_eq!(error.errno(), Ok(Errno::CbMessageBegin));
}

// --- The lifecycle and the dispatchers inside a parse -----------------------------------

/// Initialises its own parser again, which is refused.
struct Reinitialising;

impl EventHandler for Reinitialising {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.init(ParserType::Request, None);
        0
    }
}

/// Resets its own parser, which is refused.
struct Resetting;

impl EventHandler for Resetting {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        parser.reset();
        0
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_callback_cannot_init_or_reset_the_parser_whose_parse_runs_it() {
    for settings in [every_event::<Reinitialising>(), every_event::<Resetting>()] {
        let mut parser = Parser::new();
        parser
            .as_mut()
            .init(ParserType::Response, Some(settings.as_ref()));
        // The refusal panics before the parser changes, and the trampoline turns the panic
        // into the callback's `-1`, which stops the parse at its first event.
        let error = parser
            .as_mut()
            .execute(b"HTTP/1.1 204 No Content\r\n\r\n")
            .unwrap_err();
        assert_eq!(error.errno(), Ok(Errno::CbMessageBegin));
        assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Response));
        assert_eq!(
            parser.as_ref().settings_ptr().addr(),
            std::ptr::from_ref(settings).addr()
        );
        // Outside its parse, the parser may be reset and parse again.
        parser.as_mut().reset();
        parser.as_mut().set_settings(None);
        assert_eq!(
            parser.as_mut().execute(b"HTTP/1.1 204 No Content\r\n\r\n"),
            Ok(Parsed::All)
        );
    }
}

/// Hands its parser's table a run of its own, and finishes its parser, at each event it
/// signals, then goes on.
struct Dispatching;

impl EventHandler for Dispatching {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        let run = b"!".to_vec();
        let ret = parser.on_body(&run);
        drop(run);
        let _ = parser.finish();
        ret
    }
}

static DISPATCHING: LazyLock<Settings> = LazyLock::new(|| {
    let mut settings = Settings::new();
    let mut m = settings.as_mut();
    record(&mut m);
    m.set_on_message_begin(Some(Callback::new::<Dispatching>()));
    m.set_on_headers_complete(Some(Callback::new::<Dispatching>()));
    settings
});

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn callbacks_dispatch_to_and_finish_their_parser_mid_parse() {
    BODY.take();
    COMPLETE.set(0);
    let mut parser = Parser::new();
    parser
        .as_mut()
        .init(ParserType::Response, Some(DISPATCHING.as_ref()));
    let input = response(b"hello");
    for run in input.chunks(4) {
        assert_eq!(parser.as_mut().execute(run), Ok(Parsed::All));
    }
    // Each of the two events handed the table a `!`, and the parse its body; neither
    // `finish` inside the message signalled its end, which the parse did once.
    assert_eq!(BODY.take(), b"!!hello");
    assert_eq!(COMPLETE.get(), 1);
    assert_eq!(parser.as_ref().status_code(), 200);
    assert_eq!(parser.as_ref().reason(), Some(c"Invalid EOF state"));
    assert_eq!(parser.as_mut().finish(), Ok(()));
}
