//! UB tests for `deps/llhttp/http.c`'s port on the safe API: its hooks and predicates run
//! from inside the callbacks of a parse, on the parser being parsed, where llhttp's generated
//! parser calls the hooks itself before and after them. They change what the parse reads
//! next, the upgrade, the flags and the finish byte, which C reads as integers, but not where
//! it stands, so the parse goes on over states C never makes, such as a message whose flags
//! were cleared before its body, and ends or fails as llhttp's states lead it. The tests that
//! parse call C, and are ignored under Miri; the rest runs there.

use std::cell::Cell;
use std::ffi::c_int;
use std::sync::LazyLock;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, EventHandler, Parser, ParserMut, ParserType,
};

thread_local! {
    /// The callbacks this thread's parses ran.
    static CALLS: Cell<u32> = const { Cell::new(0) };
}

/// Runs every hook and predicate on its parser, as llhttp's generated parser runs them, and
/// the upgrade's flip besides.
struct Meddling;

impl EventHandler for Meddling {
    fn on_event(mut parser: ParserMut<'_>) -> c_int {
        CALLS.set(CALLS.get() + 1);
        let _ = parser.as_ref().message_needs_eof();
        let _ = parser.as_ref().should_keep_alive();
        let _ = parser.as_ref().after_headers_complete();
        parser.before_headers_complete();
        let upgrade = parser.as_ref().upgrade();
        let _ = parser.after_message_complete();
        parser.set_upgrade(!upgrade);
        0
    }
}

static MEDDLING: LazyLock<Settings> = LazyLock::new(|| {
    let mut settings = Settings::new();
    let mut m = settings.as_mut();
    m.set_on_message_begin(Some(Callback::new::<Meddling>()));
    m.set_on_headers_complete(Some(Callback::new::<Meddling>()));
    m.set_on_message_complete(Some(Callback::new::<Meddling>()));
    m.set_on_chunk_header(Some(Callback::new::<Meddling>()));
    m.set_on_chunk_complete(Some(Callback::new::<Meddling>()));
    settings
});

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_hooks_run_inside_their_parsers_parse() {
    let inputs: [(ParserType, &[u8]); 6] = [
        (
            ParserType::Response,
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
        ),
        (
            ParserType::Response,
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcHTTP/1.1 204 No Content\r\n\r\n",
        ),
        (
            ParserType::Response,
            b"HTTP/1.0 200 OK\r\n\r\nuntil the end",
        ),
        (
            ParserType::Response,
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: x\r\n\r\nraw",
        ),
        (
            ParserType::Request,
            b"POST / HTTP/1.1\r\nConnection: upgrade\r\nUpgrade: x\r\nContent-Length: 2\r\n\r\nab",
        ),
        (
            ParserType::Both,
            b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip, chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n",
        ),
    ];
    for (parser_type, input) in inputs {
        CALLS.set(0);
        let mut parser = Parser::new();
        parser.as_mut().init(parser_type, Some(MEDDLING.as_ref()));
        // Each run byte by byte too, so that the hooks run between two inputs.
        let _ = parser.as_mut().execute(input);
        let mut bytewise = Parser::new();
        bytewise.as_mut().init(parser_type, Some(MEDDLING.as_ref()));
        for byte in input.chunks(1) {
            if bytewise.as_mut().execute(byte).is_err() {
                break;
            }
        }
        let _ = bytewise.as_mut().finish();
        assert!(CALLS.get() > 0);
    }
}

#[test]
fn the_hooks_run_on_a_zeroed_parser() {
    let mut parser = Parser::zeroed();
    for flags in (0..0x400).chain([0x8000, u16::MAX]) {
        let mut m = parser.as_mut();
        m.set_flags_raw(flags);
        m.set_lenient_flags_raw(flags);
        m.before_headers_complete();
        let _ = m.as_ref().after_headers_complete();
        let _ = m.as_ref().should_keep_alive();
        let _ = m.after_message_complete();
        assert_eq!(m.as_ref().flags_raw(), 0);
    }
}
