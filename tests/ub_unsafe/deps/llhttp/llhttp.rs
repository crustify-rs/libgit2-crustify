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

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use ffibox::{CMutSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
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
