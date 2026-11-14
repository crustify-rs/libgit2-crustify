//! Unit tests for `deps/llhttp/http.c`'s port: the three hooks llhttp's generated parser
//! calls, [`ParserMut::before_headers_complete`], [`ParserRef::after_headers_complete`] and
//! [`ParserMut::after_message_complete`], and the two predicates `llhttp.h` declares,
//! [`ParserRef::message_needs_eof`] and [`ParserRef::should_keep_alive`], decided by Rust
//! assertions.
//!
//! Each is first decided on zeroed parsers whose type, flags, lenient flags, upgrade and finish
//! the setters set, which calls no C and runs under Miri: a status code of 0, the method
//! `DELETE` and the version HTTP/0.0. Then on the states real messages leave, parsed by
//! llhttp's generated parser, which calls C: at `on_headers_complete`, where the parser stands
//! as `llhttp__after_headers_complete` reads it, and at `on_message_complete`, as
//! `llhttp__after_message_complete` reads it. llhttp has no C tests in the tree, so none is
//! carried over.

use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::sync::LazyLock;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
use libgit2_crustify::libgit2::deps::llhttp::http::{Body, InvalidTransferEncoding};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, Errno, EventHandler, Parsed, Parser, ParserMut, ParserType,
};
use libgit2_crustify::sys::libgit2 as sys;

/// A zeroed parser of `parser_type`, with `flags` and `lenient` set.
fn zeroed(parser_type: ParserType, flags: u32, lenient: u32) -> Parser {
    let mut parser = Parser::zeroed();
    let mut m = parser.as_mut();
    m.set_parser_type(parser_type);
    m.set_flags_raw(u16::try_from(flags).unwrap());
    m.set_lenient_flags_raw(u16::try_from(lenient).unwrap());
    parser
}

// --- On zeroed parsers -----------------------------------------------------------------

#[test]
fn a_request_never_needs_eof() {
    for flags in 0..=0x3FF {
        let parser = zeroed(ParserType::Request, flags, 0);
        assert!(!parser.as_ref().message_needs_eof(), "{flags:#x}");
    }
}

#[test]
fn a_response_needs_eof_without_a_length_or_a_final_chunked() {
    let cases = [
        (0, true),
        (sys::F_CONTENT_LENGTH, false),
        (sys::F_CHUNKED, false),
        (sys::F_CHUNKED | sys::F_TRANSFER_ENCODING, false),
        (sys::F_TRANSFER_ENCODING, true),
        // A `Transfer-Encoding` not ending with `chunked` overrides the length.
        (sys::F_TRANSFER_ENCODING | sys::F_CONTENT_LENGTH, true),
        (sys::F_SKIPBODY, false),
        (sys::F_SKIPBODY | sys::F_TRANSFER_ENCODING, false),
        (sys::F_CONNECTION_KEEP_ALIVE | sys::F_CONNECTION_CLOSE, true),
        // The bits llhttp leaves unused decide nothing.
        (0x100 | 0x8000, true),
        (sys::F_CONTENT_LENGTH | 0x100 | 0x8000, false),
    ];
    for (flags, needs_eof) in cases {
        for parser_type in [ParserType::Response, ParserType::Both] {
            let parser = zeroed(parser_type, flags, 0);
            assert_eq!(
                parser.as_ref().message_needs_eof(),
                needs_eof,
                "{parser_type:?} {flags:#x}"
            );
        }
    }
}

#[test]
fn http_0_0_keeps_alive_only_with_keep_alive() {
    let cases = [
        (ParserType::Request, 0, false),
        (ParserType::Request, sys::F_CONNECTION_CLOSE, false),
        (ParserType::Request, sys::F_CONNECTION_KEEP_ALIVE, true),
        // `Connection: close` is read only from HTTP/1.1 on.
        (
            ParserType::Request,
            sys::F_CONNECTION_KEEP_ALIVE | sys::F_CONNECTION_CLOSE,
            true,
        ),
        // A body that ends with the input ends the connection.
        (ParserType::Response, sys::F_CONNECTION_KEEP_ALIVE, false),
        (
            ParserType::Response,
            sys::F_CONNECTION_KEEP_ALIVE | sys::F_CONTENT_LENGTH,
            true,
        ),
    ];
    for (parser_type, flags, keep_alive) in cases {
        let parser = zeroed(parser_type, flags, 0);
        assert_eq!(
            parser.as_ref().should_keep_alive(),
            keep_alive,
            "{parser_type:?} {flags:#x}"
        );
    }
}

/// A zeroed parser's type, flags, lenient flags and upgrade, and what
/// `after_headers_complete` decides for them.
type HeadersCase = (
    ParserType,
    u32,
    u32,
    bool,
    Result<Body, InvalidTransferEncoding>,
);

/// Decides each case on a zeroed parser, which `after_headers_complete` only reads.
fn assert_after_headers_complete(cases: &[HeadersCase]) {
    for &(parser_type, flags, lenient, upgrade, body) in cases {
        let mut parser = zeroed(parser_type, flags, lenient);
        parser.as_mut().set_upgrade(upgrade);
        assert_eq!(
            parser.as_ref().after_headers_complete(),
            body,
            "{parser_type:?} {flags:#x} {lenient:#x} {upgrade}"
        );
        // It only reads the parser.
        assert_eq!(u32::from(parser.as_ref().flags_raw()), flags);
        assert_eq!(parser.as_ref().upgrade(), upgrade);
    }
}

#[test]
fn after_headers_complete_on_a_zeroed_parser() {
    let te = sys::F_TRANSFER_ENCODING;
    let cases: [HeadersCase; 16] = [
        (ParserType::Request, 0, 0, false, Ok(Body::Empty)),
        (
            ParserType::Request,
            sys::F_CONTENT_LENGTH,
            0,
            false,
            Ok(Body::Empty),
        ),
        // An upgrade with no body to read first.
        (ParserType::Request, 0, 0, true, Ok(Body::Upgrade)),
        (
            ParserType::Request,
            sys::F_SKIPBODY,
            0,
            true,
            Ok(Body::Upgrade),
        ),
        // An upgrade with a body: the body is read first.
        (
            ParserType::Request,
            sys::F_CHUNKED,
            0,
            true,
            Ok(Body::Chunked),
        ),
        (
            ParserType::Request,
            sys::F_CHUNKED,
            0,
            false,
            Ok(Body::Chunked),
        ),
        (
            ParserType::Request,
            sys::F_CHUNKED | te,
            0,
            false,
            Ok(Body::Chunked),
        ),
        (
            ParserType::Request,
            sys::F_CHUNKED | sys::F_SKIPBODY,
            0,
            false,
            Ok(Body::Empty),
        ),
        (
            ParserType::Request,
            te,
            0,
            false,
            Err(InvalidTransferEncoding),
        ),
        (
            ParserType::Request,
            te | sys::F_CONTENT_LENGTH,
            0,
            false,
            Err(InvalidTransferEncoding),
        ),
        (
            ParserType::Request,
            te,
            sys::LENIENT_CHUNKED_LENGTH,
            false,
            Ok(Body::UntilEof),
        ),
        (
            ParserType::Request,
            te,
            sys::LENIENT_TRANSFER_ENCODING,
            false,
            Ok(Body::UntilEof),
        ),
        (ParserType::Response, te, 0, false, Ok(Body::UntilEof)),
        // Status 0 is no 1xx, 204 or 304: a response without a length reads to the end.
        (ParserType::Response, 0, 0, false, Ok(Body::UntilEof)),
        (
            ParserType::Response,
            sys::F_CONTENT_LENGTH,
            0,
            false,
            Ok(Body::Empty),
        ),
        (ParserType::Both, sys::F_SKIPBODY, 0, false, Ok(Body::Empty)),
    ];
    assert_after_headers_complete(&cases);
}

#[test]
fn after_headers_complete_on_a_zeroed_parser_s_rarer_states() {
    let te = sys::F_TRANSFER_ENCODING;
    assert_after_headers_complete(&[
        // A skipped body upgrades at once, even a chunked one.
        (
            ParserType::Request,
            sys::F_CHUNKED | sys::F_SKIPBODY,
            0,
            true,
            Ok(Body::Upgrade),
        ),
        // Only a request is refused: a parser that has not settled on one reads to the end,
        // as a response does.
        (ParserType::Both, te, 0, false, Ok(Body::UntilEof)),
        (ParserType::Both, 0, 0, false, Ok(Body::UntilEof)),
        (
            ParserType::Both,
            sys::F_CONTENT_LENGTH,
            0,
            false,
            Ok(Body::Empty),
        ),
    ]);
}

#[test]
fn before_headers_complete_on_a_zeroed_parser() {
    let both = sys::F_UPGRADE | sys::F_CONNECTION_UPGRADE;
    let cases = [
        (ParserType::Request, both, true),
        (ParserType::Request, both | sys::F_CHUNKED, true),
        // A response's headers only announce support, but for a 101.
        (ParserType::Response, both, false),
        (ParserType::Both, both, false),
        // Either alone is no upgrade, and the method, `DELETE`, is no `CONNECT`.
        (ParserType::Request, sys::F_UPGRADE, false),
        (ParserType::Request, sys::F_CONNECTION_UPGRADE, false),
        (ParserType::Request, 0, false),
    ];
    for (parser_type, flags, upgrade) in cases {
        for before in [false, true] {
            let mut parser = zeroed(parser_type, flags, 0);
            parser.as_mut().set_upgrade(before);
            parser.as_mut().before_headers_complete();
            assert_eq!(
                parser.as_ref().upgrade(),
                upgrade,
                "{parser_type:?} {flags:#x} {before}"
            );
            assert_eq!(u32::from(parser.as_ref().flags_raw()), flags);
        }
    }
}

#[test]
fn after_message_complete_settles_keep_alive_then_clears_the_message() {
    let cases = [
        (ParserType::Request, sys::F_CONNECTION_KEEP_ALIVE, true),
        (ParserType::Request, sys::F_CONNECTION_CLOSE, false),
        (ParserType::Response, sys::F_CONNECTION_KEEP_ALIVE, false),
    ];
    for (parser_type, flags, keep_alive) in cases {
        let mut parser = zeroed(parser_type, flags, sys::LENIENT_HEADERS);
        parser.as_mut().set_upgrade(true);
        parser
            .as_mut()
            .set_finish_raw(u8::try_from(sys::HTTP_FINISH_UNSAFE).unwrap());
        assert_eq!(parser.as_mut().after_message_complete(), keep_alive);
        let parser = parser.as_ref();
        assert_eq!(parser.flags_raw(), 0);
        assert_eq!(u32::from(parser.finish_raw()), sys::HTTP_FINISH_SAFE);
        // The rest is kept.
        assert!(parser.upgrade());
        assert_eq!(parser.parser_type(), Ok(parser_type));
        assert_eq!(u32::from(parser.lenient_flags_raw()), sys::LENIENT_HEADERS);
    }
}

#[test]
fn invalid_transfer_encoding_is_llhttps_error() {
    assert_eq!(
        InvalidTransferEncoding.errno(),
        Errno::InvalidTransferEncoding
    );
    assert_eq!(
        InvalidTransferEncoding.to_string(),
        "Request has invalid `Transfer-Encoding`"
    );
}

// --- On the states real messages leave ---------------------------------------------------

/// What the ported functions read of a parser at one of its callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    /// The parser's upgrade, as `llhttp__before_headers_complete` set it.
    upgrade: bool,
    body: Result<Body, InvalidTransferEncoding>,
    needs_eof: bool,
    keep_alive: bool,
}

impl Seen {
    fn of(mut parser: ParserMut<'_>) -> Self {
        let upgrade = parser.as_ref().upgrade();
        // Deciding again from the same headers leaves the upgrade as it was.
        parser.before_headers_complete();
        assert_eq!(parser.as_ref().upgrade(), upgrade);
        let parser = parser.as_ref();
        Self {
            upgrade,
            body: parser.after_headers_complete(),
            needs_eof: parser.message_needs_eof(),
            keep_alive: parser.should_keep_alive(),
        }
    }
}

thread_local! {
    /// What each `on_headers_complete` and `on_message_complete` saw, in order.
    static HEADERS: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
    static MESSAGES: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
    /// What `on_headers_complete` returns: `1` to skip the body, as for a `HEAD` request.
    static RETURN: Cell<c_int> = const { Cell::new(0) };
}

/// Records what the parser's headers decide.
struct Headers;

impl EventHandler for Headers {
    fn on_event(parser: ParserMut<'_>) -> c_int {
        HEADERS.with_borrow_mut(|seen| seen.push(Seen::of(parser)));
        RETURN.get()
    }
}

/// Records how the message ends.
struct Message;

impl EventHandler for Message {
    fn on_event(parser: ParserMut<'_>) -> c_int {
        MESSAGES.with_borrow_mut(|seen| seen.push(Seen::of(parser)));
        0
    }
}

static RECORDING: LazyLock<Settings> = LazyLock::new(|| {
    let mut settings = Settings::new();
    let mut m = settings.as_mut();
    m.set_on_headers_complete(Some(Callback::new::<Headers>()));
    m.set_on_message_complete(Some(Callback::new::<Message>()));
    settings
});

/// Parses `input` with a fresh parser of `parser_type`, its lenient flags `lenient`, and
/// `on_headers_complete` returning `ret`: the outcome, and what each callback saw.
fn parse(
    parser_type: ParserType,
    lenient: u32,
    ret: c_int,
    input: &[u8],
) -> (Result<Parsed, Errno>, Vec<Seen>, Vec<Seen>) {
    HEADERS.with_borrow_mut(Vec::clear);
    MESSAGES.with_borrow_mut(Vec::clear);
    RETURN.set(ret);
    let mut parser = Parser::new();
    parser.as_mut().init(parser_type, Some(RECORDING.as_ref()));
    parser
        .as_mut()
        .set_lenient_flags_raw(u16::try_from(lenient).unwrap());
    let outcome = parser
        .as_mut()
        .execute(input)
        .map_err(|e| e.errno().unwrap());
    (
        outcome,
        HEADERS.with_borrow_mut(std::mem::take),
        MESSAGES.with_borrow_mut(std::mem::take),
    )
}

/// A `Seen` with `body` and the three booleans.
const fn seen(
    body: Result<Body, InvalidTransferEncoding>,
    upgrade: bool,
    needs_eof: bool,
    keep_alive: bool,
) -> Seen {
    Seen {
        upgrade,
        body,
        needs_eof,
        keep_alive,
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_headers_of_responses_decide_their_bodies() {
    let empty = Ok(Body::Empty);
    let cases: [(&[u8], Seen); 18] = [
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n",
            seen(Ok(Body::Length), false, false, true),
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            seen(empty, false, false, true),
        ),
        (
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            seen(Ok(Body::Chunked), false, false, true),
        ),
        (
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
            seen(Ok(Body::UntilEof), false, true, false),
        ),
        (
            b"HTTP/1.1 200 OK\r\n\r\n",
            seen(Ok(Body::UntilEof), false, true, false),
        ),
        (b"HTTP/1.1 204 No Content\r\n\r\n", seen(empty, false, false, true)),
        (b"HTTP/1.1 304 Not Modified\r\n\r\n", seen(empty, false, false, true)),
        (b"HTTP/1.1 100 Continue\r\n\r\n", seen(empty, false, false, true)),
        (b"HTTP/1.1 102 Processing\r\n\r\n", seen(empty, false, false, true)),
        (b"HTTP/1.1 103 Early Hints\r\n\r\n", seen(empty, false, false, true)),
        // Any other 1xx has no body either, for want of a length that the input's end gives.
        (b"HTTP/1.1 199 Other\r\n\r\n", seen(empty, false, false, true)),
        // The status overrides a length.
        (
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\n",
            seen(empty, false, false, true),
        ),
        // A 101 switches protocols without its upgrade headers too, though it does not set
        // the upgrade.
        (
            b"HTTP/1.1 101 Switching Protocols\r\n\r\n",
            seen(Ok(Body::Upgrade), false, false, true),
        ),
        (
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
            seen(Ok(Body::Upgrade), false, false, true),
        ),
        (
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n",
            seen(Ok(Body::Upgrade), true, false, true),
        ),
        // A 200's upgrade headers only announce support.
        (
            b"HTTP/1.1 200 OK\r\nConnection: upgrade\r\nUpgrade: h2c\r\nContent-Length: 2\r\n\r\n",
            seen(Ok(Body::Length), false, false, true),
        ),
        (
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            seen(empty, false, false, false),
        ),
        (
            b"HTTP/1.0 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n",
            seen(empty, false, false, true),
        ),
    ];
    for (input, expected) in cases {
        let (_, headers, _) = parse(ParserType::Response, 0, 0, input);
        assert_eq!(headers, [expected], "{}", String::from_utf8_lossy(input));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_headers_of_requests_decide_their_bodies() {
    let te = b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n";
    let cases: [(&[u8], u32, Seen); 8] = [
        (
            b"GET / HTTP/1.1\r\n\r\n",
            0,
            seen(Ok(Body::Empty), false, false, true),
        ),
        (
            b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n",
            0,
            seen(Ok(Body::Length), false, false, true),
        ),
        (
            b"CONNECT example.com:443 HTTP/1.1\r\n\r\n",
            0,
            seen(Ok(Body::Upgrade), true, false, true),
        ),
        // A `CONNECT` upgrades before its body, which is the tunnel's.
        (
            b"CONNECT example.com:443 HTTP/1.1\r\nContent-Length: 5\r\n\r\n",
            0,
            seen(Ok(Body::Upgrade), true, false, true),
        ),
        (
            b"GET / HTTP/1.1\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n",
            0,
            seen(Ok(Body::Upgrade), true, false, true),
        ),
        // An upgrade with a body reads the body first.
        (
            b"POST / HTTP/1.1\r\nConnection: upgrade\r\nUpgrade: x\r\nContent-Length: 2\r\n\r\n",
            0,
            seen(Ok(Body::Length), true, false, true),
        ),
        (
            te,
            sys::LENIENT_TRANSFER_ENCODING,
            seen(Ok(Body::UntilEof), false, false, true),
        ),
        (
            b"GET / HTTP/1.0\r\n\r\n",
            0,
            seen(Ok(Body::Empty), false, false, false),
        ),
    ];
    for (input, lenient, expected) in cases {
        let (_, headers, _) = parse(ParserType::Request, lenient, 0, input);
        assert_eq!(headers, [expected], "{}", String::from_utf8_lossy(input));
    }
    // Without leniency, the parser refuses the request as `after_headers_complete` does.
    let (outcome, headers, messages) = parse(ParserType::Request, 0, 0, te);
    assert_eq!(outcome, Err(InvalidTransferEncoding.errno()));
    assert_eq!(
        headers,
        [seen(Err(InvalidTransferEncoding), false, false, true)]
    );
    assert_eq!(messages, []);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn an_upgrade_pauses_the_parse_where_after_headers_complete_says_so() {
    let (outcome, _, messages) = parse(
        ParserType::Request,
        0,
        0,
        b"CONNECT example.com:443 HTTP/1.1\r\n\r\nnot http",
    );
    assert_eq!(outcome, Ok(Parsed::PausedUpgrade));
    assert_eq!(messages.len(), 1);
    // A 101 too.
    let (outcome, _, _) = parse(
        ParserType::Response,
        0,
        0,
        b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: x\r\n\r\n",
    );
    assert_eq!(outcome, Ok(Parsed::PausedUpgrade));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_skipped_body_is_empty_at_the_messages_end() {
    // `on_headers_complete` returns 1 for the response to a `HEAD` request: the parser sets
    // `F_SKIPBODY` and completes the message without reading its 10 bytes.
    let (outcome, headers, messages) = parse(
        ParserType::Response,
        0,
        1,
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n",
    );
    assert_eq!(outcome, Ok(Parsed::All));
    assert_eq!(headers[0].body, Ok(Body::Length));
    assert_eq!(
        messages,
        [
            seen(Ok(Body::Empty), false, false, true),
            seen(Ok(Body::Empty), false, false, true),
        ]
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn after_message_complete_clears_the_flags_for_the_next_message() {
    // Were the first message's `F_CHUNKED` kept, the second would be read as chunked.
    let (outcome, headers, messages) = parse(
        ParserType::Response,
        0,
        0,
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n\
          HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(outcome, Ok(Parsed::All));
    let bodies: Vec<_> = headers.iter().map(|s| s.body).collect();
    assert_eq!(bodies, [Ok(Body::Chunked), Ok(Body::Empty)]);
    assert_eq!(messages.len(), 2);

    // Called on a parser whose message has begun, it ends it as the hook does.
    let mut parser = Parser::new();
    parser.as_mut().init(ParserType::Response, None);
    parser
        .as_mut()
        .execute(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 5\r\n\r\n")
        .unwrap();
    assert_ne!(parser.as_ref().flags_raw(), 0);
    assert!(!parser.as_mut().after_message_complete());
    assert_eq!(parser.as_ref().flags_raw(), 0);
    assert_eq!(
        u32::from(parser.as_ref().finish_raw()),
        sys::HTTP_FINISH_SAFE
    );
    // It clears the flags alone of what the message's start line and headers set.
    let ended = parser.as_ref();
    assert_eq!(ended.status_code(), 200);
    assert_eq!((ended.http_major(), ended.http_minor()), (1, 1));
    assert_eq!(ended.content_length(), 5);
    assert_eq!(ended.parser_type(), Ok(ParserType::Response));
    assert_eq!(ended.error(), Ok(Errno::Ok));
    // The input may now end where the parser is.
    assert!(parser.as_mut().finish().is_ok());
}

/// What `should_keep_alive` and `after_message_complete` settle for a response of `version`,
/// whose `Content-Length` of 0 ends its body, so that only the version and the `Connection`
/// header decide: the first as `on_headers_complete` and `on_message_complete` ask it, the
/// second called on a parser that has read every header line but the empty one.
fn keep_alive(version: &str, connection: &str) -> (bool, bool) {
    let input = format!("HTTP/{version} 200 OK\r\n{connection}Content-Length: 0\r\n\r\n");
    let (outcome, headers, messages) = parse(ParserType::Response, 0, 0, input.as_bytes());
    assert_eq!(outcome, Ok(Parsed::All), "{input}");
    let mut parser = Parser::new();
    parser.as_mut().init(ParserType::Response, None);
    parser
        .as_mut()
        .execute(&input.as_bytes()[..input.len() - 2])
        .unwrap();
    let after = parser.as_mut().after_message_complete();
    assert_eq!(messages[0].keep_alive, headers[0].keep_alive, "{input}");
    (headers[0].keep_alive, after)
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn http_1_1_keeps_alive_unless_it_closes() {
    assert_eq!(keep_alive("1.1", ""), (true, true));
    assert_eq!(keep_alive("1.1", "Connection: close\r\n"), (false, false));
    // `keep-alive` is no exception to a close.
    assert_eq!(
        keep_alive("1.1", "Connection: close, keep-alive\r\n"),
        (false, false)
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn http_1_0_keeps_alive_only_with_keep_alive() {
    assert_eq!(keep_alive("1.0", ""), (false, false));
    assert_eq!(
        keep_alive("1.0", "Connection: keep-alive\r\n"),
        (true, true)
    );
    // `close` is read only from HTTP/1.1 on.
    assert_eq!(
        keep_alive("1.0", "Connection: keep-alive, close\r\n"),
        (true, true)
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn http_0_9_keeps_alive_only_with_keep_alive() {
    // A minor above zero is not HTTP/1.1 without a major above zero.
    assert_eq!(keep_alive("0.9", ""), (false, false));
    assert_eq!(
        keep_alive("0.9", "Connection: keep-alive\r\n"),
        (true, true)
    );
}

/// The departure the port keeps from llhttp, documented on `should_keep_alive`: a version is
/// HTTP/1.1 or later only when both its major and its minor are above zero, so HTTP/2.0 is
/// read as HTTP/1.0 is, where RFC 9112's "HTTP/1.1 (or later)" would keep it alive.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn http_2_0_is_read_as_an_earlier_version_for_keep_alive() {
    assert_eq!(keep_alive("2.0", ""), (false, false));
    assert_eq!(
        keep_alive("2.0", "Connection: keep-alive\r\n"),
        (true, true)
    );
    assert_eq!(
        keep_alive("2.0", "Connection: keep-alive, close\r\n"),
        (true, true)
    );
}
