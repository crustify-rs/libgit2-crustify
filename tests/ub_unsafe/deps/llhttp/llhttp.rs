//! `deps/llhttp/llhttp.c`'s parser: [`ParserMut::set_data`] and
//! [`ParserMut::set_settings_ptr`], the two `unsafe` setters, called within their `# Safety`
//! contracts, one suite per setter.
//!
//! Each parser is initialised by `llhttp_init` and fed input by `llhttp_execute`, through
//! the raw bindings, the parser's own wrappers being the next batch's: the C runs the
//! callbacks the settings install, which read the parser's `data` as their context, so
//! that `ASan` and `UBSan` see every access C makes through what the setters stored, and the
//! edges of each contract: NULL, a value replaced between calls, a parser reset to new
//! settings, a table whose callbacks never read `data`.
//!
//! `set_settings_ptr`'s suite also installs Rust handlers, through [`Callback`] and
//! [`DataCallback`], and parses with [`ParserMut::parse`], so that C calls the trampolines:
//! handlers that collect each run, pause, return their own error, panic, parse another
//! parser, and try to parse or initialise the parser whose parse runs them.

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use ffibox::{CMutSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, Errno, EventHandler, Parsed, Parser, ParserMut, ParserType,
};
use libgit2_crustify::sys::libgit2 as sys;

/// A parser `llhttp_init` initialised with no settings.
fn c_init(parser_type: ParserType) -> Parser {
    let mut parser = Parser::zeroed();
    // SAFETY: the handle addresses `parser`, a live `llhttp_t` Rust owns, which
    // `llhttp_init` fills in place; with NULL settings no callback runs.
    unsafe { sys::llhttp_init(parser.as_mut().as_mut_ptr(), parser_type.raw(), ptr::null()) };
    parser
}

/// Feeds `data` to `parser` through C.
fn c_execute(parser: &mut Parser, data: &[u8]) -> sys::llhttp_errno {
    // SAFETY: `parser` is initialised, `data` is `data.len()` readable bytes, and the
    // settings the tests store outlive every call (each test's contract).
    unsafe {
        sys::llhttp_execute(
            parser.as_mut().as_mut_ptr(),
            data.as_ptr().cast(),
            data.len(),
        )
    }
}

/// The context the callbacks below read through `data`.
#[derive(Default)]
struct Seen {
    body: Vec<u8>,
    headers: usize,
    messages: usize,
}

/// The `Seen` a callback's parser carries in `data`.
///
/// # Safety
///
/// `parser` is a live parser whose `data` addresses a live `Seen` nothing else accesses
/// during the callback.
unsafe fn seen<'a>(parser: *mut sys::llhttp_t) -> &'a mut Seen {
    // SAFETY: `parser` is live (the caller's contract), and `data` a field of it.
    let data = unsafe { (*parser).data };
    // SAFETY: `data` addresses a live `Seen`, accessed only here (the caller's contract).
    unsafe { &mut *data.cast::<Seen>() }
}

unsafe extern "C" fn on_body(parser: *mut sys::llhttp_t, at: *const c_char, len: usize) -> c_int {
    // SAFETY: llhttp calls back with the parser it runs, whose `data` the test set to a `Seen`.
    let seen = unsafe { seen(parser) };
    // SAFETY: llhttp hands the callback `len` readable bytes of its input at `at`.
    seen.body
        .extend_from_slice(unsafe { std::slice::from_raw_parts(at.cast(), len) });
    0
}

unsafe extern "C" fn on_header_value_complete(parser: *mut sys::llhttp_t) -> c_int {
    // SAFETY: as in `on_body`.
    unsafe { seen(parser) }.headers += 1;
    0
}

unsafe extern "C" fn on_message_complete(parser: *mut sys::llhttp_t) -> c_int {
    // SAFETY: as in `on_body`.
    unsafe { seen(parser) }.messages += 1;
    0
}

/// A table whose callbacks read `data` as a [`Seen`].
fn counting_settings() -> sys::llhttp_settings_t {
    let mut settings = sys::llhttp_settings_t::default();
    // SAFETY: `settings` is a live table, which `llhttp_settings_init` zeroes.
    unsafe { sys::llhttp_settings_init(&raw mut settings) };
    settings.on_body = Some(on_body);
    settings.on_header_value_complete = Some(on_header_value_complete);
    settings.on_message_complete = Some(on_message_complete);
    settings
}

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nA: 1\r\nContent-Length: 4\r\n\r\nbody";

mod set_data {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn callbacks_read_the_context_it_stores() {
        let settings = counting_settings();
        let mut seen = Seen::default();
        let mut parser = c_init(ParserType::Response);
        let mut m = parser.as_mut();
        // SAFETY: `settings` outlives the parser's executions below.
        unsafe { m.set_settings_ptr(&raw const settings) };
        // SAFETY: the callbacks read `data` as a `Seen`, and `seen` is one, live and accessed
        // only by them until the parser is done executing.
        unsafe { m.set_data((&raw mut seen).cast()) };
        assert_eq!(m.as_ref().data(), (&raw mut seen).cast::<c_void>());
        for piece in RESPONSE.chunks(5) {
            assert_eq!(c_execute(&mut parser, piece), sys::HPE_OK);
        }
        assert_eq!(seen.body, b"body");
        assert_eq!((seen.headers, seen.messages), (2, 1));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_context_replaced_between_calls_takes_the_rest() {
        let settings = counting_settings();
        let (mut first, mut second) = (Seen::default(), Seen::default());
        let mut parser = c_init(ParserType::Response);
        // SAFETY: as in `callbacks_read_the_context_it_stores`, for `first`.
        unsafe {
            parser.as_mut().set_settings_ptr(&raw const settings);
        }
        // SAFETY: as above.
        unsafe { parser.as_mut().set_data((&raw mut first).cast()) };
        let (head, body) = RESPONSE.split_at(RESPONSE.len() - 4);
        assert_eq!(c_execute(&mut parser, head), sys::HPE_OK);
        // SAFETY: as above, for `second`, from here on.
        unsafe { parser.as_mut().set_data((&raw mut second).cast()) };
        assert_eq!(c_execute(&mut parser, body), sys::HPE_OK);
        assert_eq!((first.headers, first.messages, first.body.len()), (2, 0, 0));
        assert_eq!((second.headers, second.messages), (0, 1));
        assert_eq!(second.body, b"body");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn any_value_is_carried_where_no_callback_reads_it() {
        let mut parser = c_init(ParserType::Response);
        for value in [
            ptr::null_mut(),
            ptr::dangling_mut::<u64>().cast(),
            ptr::without_provenance_mut(usize::MAX),
        ] {
            // SAFETY: the parser's settings are NULL, so nothing reads `data`.
            unsafe { parser.as_mut().set_data(value) };
            assert_eq!(
                c_execute(&mut parser, b"HTTP/1.1 204 No Content\r\n\r\n"),
                sys::HPE_OK
            );
            assert_eq!(parser.as_ref().data(), value);
        }
    }
}

mod set_settings_ptr {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn the_table_it_stores_runs_its_callbacks() {
        let settings = counting_settings();
        let mut seen = Seen::default();
        let mut parser = c_init(ParserType::Response);
        // SAFETY: `seen` is the `Seen` the callbacks read, live until the parser is done.
        unsafe { parser.as_mut().set_data((&raw mut seen).cast()) };
        // SAFETY: `settings` is live and unchanged while the parser executes, and its
        // callbacks are sound with this parser's `data`.
        unsafe { parser.as_mut().set_settings_ptr(&raw const settings) };
        assert_eq!(parser.as_ref().settings_ptr(), &raw const settings);
        assert_eq!(c_execute(&mut parser, RESPONSE), sys::HPE_OK);
        assert_eq!(
            (seen.headers, seen.messages, seen.body.as_slice()),
            (2, 1, &b"body"[..])
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn null_runs_no_callback() {
        let settings = counting_settings();
        let mut seen = Seen::default();
        let mut parser = c_init(ParserType::Response);
        // SAFETY: as in `the_table_it_stores_runs_its_callbacks`.
        unsafe { parser.as_mut().set_data((&raw mut seen).cast()) };
        // SAFETY: as above.
        unsafe { parser.as_mut().set_settings_ptr(&raw const settings) };
        let (head, body) = RESPONSE.split_at(RESPONSE.len() - 4);
        assert_eq!(c_execute(&mut parser, head), sys::HPE_OK);
        // SAFETY: NULL is always allowed: no callback runs.
        unsafe { parser.as_mut().set_settings_ptr(ptr::null()) };
        assert!(parser.as_ref().settings_ptr().is_null());
        assert_eq!(c_execute(&mut parser, body), sys::HPE_OK);
        assert_eq!((seen.headers, seen.messages, seen.body.len()), (2, 0, 0));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_table_whose_callbacks_ignore_data_takes_any_data() {
        let mut settings = sys::llhttp_settings_t::default();
        // SAFETY: `settings` is a live table, which `llhttp_settings_init` zeroes: it
        // installs no callback, so none reads `data`.
        unsafe { sys::llhttp_settings_init(&raw mut settings) };
        let mut parser = c_init(ParserType::Request);
        // SAFETY: no callback of the table reads `data`.
        unsafe { parser.as_mut().set_data(ptr::dangling_mut::<u8>().cast()) };
        // SAFETY: `settings` outlives the parser's executions below.
        unsafe { parser.as_mut().set_settings_ptr(&raw const settings) };
        assert_eq!(
            c_execute(&mut parser, b"GET / HTTP/1.1\r\n\r\n"),
            sys::HPE_OK
        );
        assert_eq!(parser.as_ref().status_code(), 0);
    }
}

/// Rust handlers C calls through the trampolines, recording what they see on the test's
/// thread.
mod handlers {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        /// Each event and run the handlers saw, in order.
        static SEEN: RefCell<Vec<(&'static str, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    }

    /// The events and runs seen since the last call, runs of one part joined.
    pub(super) fn take() -> Vec<(&'static str, Vec<u8>)> {
        let seen = SEEN.with(RefCell::take);
        let mut joined: Vec<(&'static str, Vec<u8>)> = Vec::new();
        for (what, run) in seen {
            match joined.last_mut() {
                Some((last, data)) if *last == what && !run.is_empty() => data.extend(run),
                _ => joined.push((what, run)),
            }
        }
        joined
    }

    fn record(what: &'static str, data: &[u8]) {
        SEEN.with_borrow_mut(|seen| seen.push((what, data.to_vec())));
    }

    macro_rules! data_handler {
        ($name:ident, $what:literal) => {
            pub(super) struct $name;

            impl DataHandler for $name {
                fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
                    // A parse that ends where a span does hands it an empty run.
                    if !data.is_empty() {
                        record($what, data);
                    }
                    0
                }
            }
        };
    }

    data_handler!(Url, "url");
    data_handler!(Status, "status");
    data_handler!(HeaderField, "field");
    data_handler!(HeaderValue, "value");
    data_handler!(Body, "body");

    pub(super) struct MessageComplete;

    impl EventHandler for MessageComplete {
        fn on_event(parser: ParserMut<'_>) -> c_int {
            let code = parser.as_ref().status_code().to_string();
            record("complete", code.as_bytes());
            0
        }
    }

    /// Pauses at the end of the headers, as libgit2's HTTP client does.
    pub(super) struct PausingAtHeaders;

    impl EventHandler for PausingAtHeaders {
        fn on_event(_: ParserMut<'_>) -> c_int {
            record("headers", b"");
            c_int::try_from(sys::HPE_PAUSED).unwrap()
        }
    }

    /// Stops the parse with a code no errno names.
    pub(super) struct Refusing;

    impl DataHandler for Refusing {
        fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
            1000
        }
    }

    /// Panics.
    pub(super) struct Panicking;

    impl EventHandler for Panicking {
        fn on_event(_: ParserMut<'_>) -> c_int {
            panic!("a handler panicked under C");
        }
    }

    /// Parses the parser whose parse runs it, which panics.
    pub(super) struct Reentering;

    impl DataHandler for Reentering {
        fn on_data(mut parser: ParserMut<'_>, _: &[u8]) -> c_int {
            record("reentering", b"");
            let _ = parser.parse(b"HTTP/1.1 200 OK\r\n\r\n");
            record("reentered", b"");
            0
        }
    }

    /// Initialises the parser whose parse runs it, which panics.
    pub(super) struct Reinitialising;

    impl EventHandler for Reinitialising {
        fn on_event(mut parser: ParserMut<'_>) -> c_int {
            parser.reinit();
            record("reinitialised", b"");
            0
        }
    }

    /// Parses a parser of its own, with the same handlers, while its parser's parse runs.
    pub(super) struct ParsingAnother;

    impl EventHandler for ParsingAnother {
        fn on_event(_: ParserMut<'_>) -> c_int {
            let settings = collecting();
            let mut other = Parser::new();
            // SAFETY: `settings` outlives `other`'s parse, and its handlers read no `data`.
            unsafe { other.as_mut().set_settings_ptr(&raw const settings) };
            match other
                .as_mut()
                .parse(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
            {
                Ok(Parsed::All) => 0,
                _ => -1,
            }
        }
    }

    /// A table of the collecting handlers.
    pub(super) fn collecting() -> sys::llhttp_settings_t {
        sys::llhttp_settings_t {
            on_url: DataCallback::new::<Url>().raw(),
            on_status: DataCallback::new::<Status>().raw(),
            on_header_field: DataCallback::new::<HeaderField>().raw(),
            on_header_value: DataCallback::new::<HeaderValue>().raw(),
            on_body: DataCallback::new::<Body>().raw(),
            on_message_complete: Callback::new::<MessageComplete>().raw(),
            ..sys::llhttp_settings_t::default()
        }
    }
}

mod rust_handlers {
    use super::handlers::{self, collecting};
    use super::*;

    /// A parser whose settings are `settings`, which the handlers' `data` is not read by.
    ///
    /// # Safety
    ///
    /// `settings` outlives every parse of the parser.
    unsafe fn with(settings: &sys::llhttp_settings_t) -> Parser {
        let mut parser = Parser::new();
        // SAFETY: the caller keeps `settings` live; its handlers are Rust's, which read no
        // `data`.
        unsafe { parser.as_mut().set_settings_ptr(settings) };
        parser
    }

    fn owned(seen: &[(&'static str, &[u8])]) -> Vec<(&'static str, Vec<u8>)> {
        seen.iter().map(|(w, d)| (*w, d.to_vec())).collect()
    }

    const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nHost: x\r\nContent-Length: 4\r\n\r\nbody";

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn c_calls_the_handlers_with_each_run_in_any_number_of_parses() {
        let settings = collecting();
        let expected = owned(&[
            ("status", b"OK"),
            ("field", b"Host"),
            ("value", b"x"),
            ("field", b"Content-Length"),
            ("value", b"4"),
            ("body", b"body"),
            ("complete", b"200"),
        ]);
        for size in 1..=RESPONSE.len() {
            // SAFETY: `settings` outlives the parser.
            let mut parser = unsafe { with(&settings) };
            for run in RESPONSE.chunks(size) {
                assert_eq!(parser.as_mut().parse(run), Ok(Parsed::All));
            }
            assert_eq!(handlers::take(), expected, "runs of {size}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_pause_resumes_where_it_stopped() {
        let mut settings = collecting();
        settings.on_headers_complete = Callback::new::<handlers::PausingAtHeaders>().raw();
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        assert_eq!(parser.as_mut().parse(RESPONSE), Ok(Parsed::Paused));
        let at = parser.as_ref().error_offset(RESPONSE).unwrap();
        assert_eq!(&RESPONSE[at..], b"body");
        assert_eq!(parser.as_mut().parse(&RESPONSE[at..]), Ok(Parsed::Paused));
        parser.as_mut().set_error(Errno::Ok);
        assert_eq!(parser.as_mut().parse(&RESPONSE[at..]), Ok(Parsed::All));
        let seen = handlers::take();
        assert_eq!(seen[5], ("headers", Vec::new()));
        assert_eq!(
            &seen[6..],
            owned(&[("body", b"body"), ("complete", b"200")])
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_handlers_own_code_is_the_parse_error() {
        let settings = sys::llhttp_settings_t {
            on_header_field: DataCallback::new::<handlers::Refusing>().raw(),
            ..collecting()
        };
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        let error = parser.as_mut().parse(RESPONSE).unwrap_err();
        assert_eq!(error.code(), 1000);
        assert_eq!(error.errno().unwrap_err().raw(), 1000);
        assert_eq!(
            error.to_string(),
            "llhttp stopped at a callback's error 1000"
        );
        assert_eq!(parser.as_ref().error_raw(), 1000);
        assert_eq!(parser.as_mut().parse(b"more"), Err(error));
        assert_eq!(handlers::take(), owned(&[("status", b"OK")]));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_handlers_panic_stops_the_parse_with_its_events_error() {
        let settings = sys::llhttp_settings_t {
            on_message_begin: Callback::new::<handlers::Panicking>().raw(),
            ..collecting()
        };
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        let error = parser.as_mut().parse(RESPONSE).unwrap_err();
        assert_eq!(error.errno(), Ok(Errno::CbMessageBegin));
        assert_eq!(handlers::take(), Vec::new());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_handler_cannot_parse_the_parser_whose_parse_runs_it() {
        let settings = sys::llhttp_settings_t {
            on_status: DataCallback::new::<handlers::Reentering>().raw(),
            ..collecting()
        };
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        let error = parser.as_mut().parse(RESPONSE).unwrap_err();
        // api.c's dispatcher turns the span callback's -1 into `HPE_USER`.
        assert_eq!(error.errno(), Ok(Errno::User));
        assert_eq!(
            parser.as_ref().reason(),
            Some(c"Span callback error in on_status")
        );
        assert_eq!(handlers::take(), owned(&[("reentering", b"")]));
        // The refused parse registered nothing: once the parse is over, the parser parses.
        parser.as_mut().reinit();
        assert_eq!(
            parser.as_mut().parse(b"HTTP/1.1 204 No Content\r\n\r\n"),
            Ok(Parsed::All)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_handler_cannot_initialise_the_parser_whose_parse_runs_it() {
        let settings = sys::llhttp_settings_t {
            on_message_begin: Callback::new::<handlers::Reinitialising>().raw(),
            ..collecting()
        };
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        let error = parser.as_mut().parse(RESPONSE).unwrap_err();
        assert_eq!(error.errno(), Ok(Errno::CbMessageBegin));
        assert_eq!(handlers::take(), Vec::new());
        assert_eq!(parser.as_ref().settings_ptr(), &raw const settings);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_handler_may_parse_another_parser() {
        let settings = sys::llhttp_settings_t {
            on_message_begin: Callback::new::<handlers::ParsingAnother>().raw(),
            ..collecting()
        };
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        assert_eq!(parser.as_mut().parse(RESPONSE), Ok(Parsed::All));
        let seen = handlers::take();
        assert_eq!(
            &seen[..2],
            owned(&[("status", b"Created"), ("field", b"Content-Length")])
        );
        assert_eq!(seen.last(), Some(&("complete", b"200".to_vec())));
        assert_eq!(seen.iter().filter(|(w, _)| *w == "complete").count(), 2);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_request_hands_its_url_and_a_null_table_runs_nothing() {
        let settings = collecting();
        // SAFETY: `settings` outlives the parser.
        let mut parser = unsafe { with(&settings) };
        assert_eq!(
            parser.as_mut().parse(b"GET /a/b?c HTTP/1.1\r\n"),
            Ok(Parsed::All)
        );
        // SAFETY: NULL is always allowed.
        unsafe { parser.as_mut().set_settings_ptr(ptr::null()) };
        assert_eq!(parser.as_mut().parse(b"Host: x\r\n\r\n"), Ok(Parsed::All));
        assert_eq!(handlers::take(), owned(&[("url", b"/a/b?c")]));
    }
}
