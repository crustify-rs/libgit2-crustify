//! `deps/llhttp/http.c`: the callbacks llhttp's generated parser makes around a message's
//! headers and its end, ported to Rust.
//!
//! # The hooks
//!
//! llhttp's generated parser, `llhttp.c`, hands three decisions to http.c, each a hook it
//! calls with the parser at one point of every message, the same `int (parser, p, endp)`
//! signature as api.c's dispatchers, whose `p` and `endp` the hooks never read:
//!
//! - `llhttp__before_headers_complete`, once the headers are read and before `on_headers_complete`
//!   runs: it sets whether the message upgrades the connection, so that the callback can see it. It
//!   is [`ParserMut::before_headers_complete`], which cannot fail and returns nothing, as C's `0`
//!   reports nothing.
//! - `llhttp__after_headers_complete`, after `on_headers_complete`: it decides how the message's
//!   body follows its headers, one of the six codes the parser's states switch on. It only reads
//!   the parser, so it is [`ParserRef::after_headers_complete`], whose [`Body`] is one of the five
//!   codes the parser goes on from, and whose [`InvalidTransferEncoding`] is the sixth, a request
//!   whose `Transfer-Encoding` does not end with `chunked`, which the parser refuses with
//!   `HPE_INVALID_TRANSFER_ENCODING`.
//! - `llhttp__after_message_complete`, after `on_message_complete`: it settles whether the
//!   connection may carry another message, marks the parser safe to finish and clears the message's
//!   flags for the next. It is [`ParserMut::after_message_complete`], whose `bool` is C's `int`,
//!   `1` to keep the connection.
//!
//! # The predicates
//!
//! `llhttp.h` declares http.c's two others for llhttp's users, which the hooks call too:
//! `llhttp_message_needs_eof`, whether the message's body ends only with the input, as a
//! response with neither a `Content-Length` nor a chunked `Transfer-Encoding` does, and
//! `llhttp_should_keep_alive`, whether another message may follow the one parsed on the same
//! connection, as libgit2's HTTP client asks once a response's headers are read
//! (`transports/httpclient.c`, through `httpparser.h`'s `git_http_parser_keep_alive`). They
//! are [`ParserRef::message_needs_eof`] and [`ParserRef::should_keep_alive`].
//!
//! Each reads the fields the parser fills from a message's start line and headers through
//! `llhttp.rs`'s accessors: its type, method and status code, its HTTP version, its
//! `Content-Length`, its upgrade, and its flag words, which stay integers until
//! `llhttp_flags` and `llhttp_lenient_flags` have types of their own, their bits the
//! constants the raw bindings generate (`F_*`, `LENIENT_*`). The type and the method are
//! compared as llhttp's enums, which a byte no constant names never equals, as C's `==` on
//! the byte has it. The version is read as llhttp reads it: a message is HTTP/1.1 or later
//! only when its major and its minor are both above zero, so `HTTP/2.0` keeps its connection
//! only with `Connection: keep-alive`, as `HTTP/1.0` does.
//!
//! # The exports
//!
//! The C that remains calls four of these functions: `llhttp.c`'s generated parser the three
//! hooks, which no header declares, and libgit2's HTTP client `llhttp_should_keep_alive`.
//! They are exported under their C names, compiled under the export feature
//! `ffi-export-deps-llhttp-http-c` until the module switches from C. `llhttp_message_needs_eof`
//! has no C caller outside http.c, so it has no export and leaves with http.c's C.
//!
//! [`ParserMut::before_headers_complete`]: super::llhttp::ParserMut::before_headers_complete
//! [`ParserMut::after_message_complete`]: super::llhttp::ParserMut::after_message_complete
//! [`ParserRef::after_headers_complete`]: super::llhttp::ParserRef::after_headers_complete
//! [`ParserRef::message_needs_eof`]: super::llhttp::ParserRef::message_needs_eof
//! [`ParserRef::should_keep_alive`]: super::llhttp::ParserRef::should_keep_alive

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_export`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use core::fmt;

use super::llhttp::{Errno, Method, ParserMut, ParserRef, ParserType};
use crate::sys::libgit2 as sys;

/// `llhttp_finish`'s `HTTP_FINISH_SAFE`, as the parser's `finish` byte holds it: its low byte,
/// which is the whole of it (`unit_priv`'s `finish_safe_is_http_finish_safe`).
const FINISH_SAFE: u8 = sys::HTTP_FINISH_SAFE.to_le_bytes()[0];

/// How a message's body follows its headers, as `llhttp__after_headers_complete` decides it
/// for llhttp's generated parser: the code it returns, but for the one it refuses
/// ([`InvalidTransferEncoding`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    /// No body: the message is complete, and the parser reads the next (`0`). A `100`, `102`,
    /// `103`, `204` or `304` response, the response to a `HEAD` request (`F_SKIPBODY`), a
    /// message of `Content-Length: 0`, and one with no length that does not need the input's
    /// end.
    Empty,
    /// The rest of the input is in another protocol (`1`): a `101` response, or a message that
    /// upgrades the connection with no body to read first, as a `CONNECT` request. The parser
    /// completes the message, and pauses at the upgrade when it is one.
    Upgrade,
    /// A body in chunks, `Transfer-Encoding: chunked`, whatever its `Content-Length` (`2`).
    Chunked,
    /// A body of `Content-Length` bytes, more than zero (`3`).
    Length,
    /// A body that ends with the input (`4`): a response with neither a length nor a chunked
    /// `Transfer-Encoding`, or one whose `Transfer-Encoding` does not end with `chunked`, and
    /// such a request when the parser is lenient with either.
    UntilEof,
}

/// A request whose `Transfer-Encoding` does not end with `chunked`, whose body's length
/// cannot be told (RFC 7230, 3.3.3): `llhttp__after_headers_complete`'s `5`, which llhttp's
/// parser refuses with `HPE_INVALID_TRANSFER_ENCODING`, unless it is lenient with a chunked
/// length or with the `Transfer-Encoding`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InvalidTransferEncoding;

impl InvalidTransferEncoding {
    /// The error llhttp's parser stops at for it, `HPE_INVALID_TRANSFER_ENCODING`.
    #[must_use]
    pub const fn errno(self) -> Errno {
        Errno::InvalidTransferEncoding
    }
}

impl fmt::Display for InvalidTransferEncoding {
    /// The reason llhttp's parser gives the error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Request has invalid `Transfer-Encoding`")
    }
}

impl std::error::Error for InvalidTransferEncoding {}

impl ParserRef<'_> {
    /// Whether the message's flags hold any of the `llhttp_flags` bits of `flags`.
    fn flagged(self, flags: sys::llhttp_flags) -> bool {
        u32::from(self.flags_raw()) & flags != 0
    }

    /// Whether the parser is lenient with any of the `llhttp_lenient_flags` bits of
    /// `flags`.
    fn lenient(self, flags: sys::llhttp_lenient_flags) -> bool {
        u32::from(self.lenient_flags_raw()) & flags != 0
    }

    /// Whether the message being read is a request: its parser reads requests, or one of
    /// both kinds settled on a request.
    fn is_request(self) -> bool {
        self.parser_type() == Ok(ParserType::Request)
    }

    /// Whether the message being read is a response.
    fn is_response(self) -> bool {
        self.parser_type() == Ok(ParserType::Response)
    }

    /// Whether the message's body ends only with the input, so that the parser completes it
    /// once [`finish`](ParserMut::finish) says the input has ended: a response that has a
    /// body (not a `1xx`, `204` or `304` one, nor one to a `HEAD` request) but no
    /// `Content-Length` and no chunked `Transfer-Encoding`, or whose `Transfer-Encoding` does
    /// not end with `chunked`. A request never does.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
    /// assert!(parser.as_ref().message_needs_eof());
    ///
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n")
    ///     .unwrap();
    /// assert!(!parser.as_ref().message_needs_eof());
    /// ```
    ///
    /// Replaces `fn::llhttp_message_needs_eof`
    #[doc(alias = "llhttp_message_needs_eof")]
    #[must_use]
    pub fn message_needs_eof(&self) -> bool {
        if self.is_request() {
            return false;
        }
        // RFC 2616, 4.4.
        let status = self.status_code();
        if status / 100 == 1 || status == 204 || status == 304 || self.flagged(sys::F_SKIPBODY) {
            return false;
        }
        // RFC 7230, 3.3.3: see `after_headers_complete`.
        if self.flagged(sys::F_TRANSFER_ENCODING) && !self.flagged(sys::F_CHUNKED) {
            return true;
        }
        !self.flagged(sys::F_CHUNKED | sys::F_CONTENT_LENGTH)
    }

    /// Whether another message may follow the one parsed on the same connection: an
    /// HTTP/1.1 one without `Connection: close`, or an earlier one with `Connection:
    /// keep-alive`, whose body does not end with the input
    /// ([`message_needs_eof`](Self::message_needs_eof)).
    ///
    /// The version is HTTP/1.1 or later only when its major and minor are both above zero, as
    /// llhttp reads it, so an `HTTP/2.0` message is read as an earlier one.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.1 204 No Content\r\n\r\n")
    ///     .unwrap();
    /// assert!(parser.as_ref().should_keep_alive());
    ///
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.0 204 No Content\r\n\r\n")
    ///     .unwrap();
    /// assert!(!parser.as_ref().should_keep_alive());
    /// ```
    ///
    /// Replaces `fn::llhttp_should_keep_alive`
    #[doc(alias = "llhttp_should_keep_alive")]
    #[must_use]
    pub fn should_keep_alive(&self) -> bool {
        if self.http_major() > 0 && self.http_minor() > 0 {
            // HTTP/1.1.
            if self.flagged(sys::F_CONNECTION_CLOSE) {
                return false;
            }
        } else if !self.flagged(sys::F_CONNECTION_KEEP_ALIVE) {
            // HTTP/1.0 or earlier.
            return false;
        }
        !self.message_needs_eof()
    }

    /// How the message's body follows its headers, as llhttp's generated parser asks once
    /// `on_headers_complete` has run: none, another protocol, chunks, `Content-Length` bytes
    /// or the rest of the input, each [`Body`] as it describes. A response's status decides
    /// first (RFC 2616, 4.4), then the response to a `HEAD` request, a chunked
    /// `Transfer-Encoding`, which overrides a `Content-Length`, any other
    /// `Transfer-Encoding`, and the `Content-Length`.
    ///
    /// # Errors
    ///
    /// [`InvalidTransferEncoding`] for a request whose `Transfer-Encoding` does not end with
    /// `chunked`, unless the parser is lenient with a chunked length or with the
    /// `Transfer-Encoding` (RFC 7230, 3.3.3).
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::http::Body;
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
    ///     .unwrap();
    /// assert_eq!(parser.as_ref().after_headers_complete(), Ok(Body::Chunked));
    /// ```
    ///
    /// Replaces `fn::llhttp__after_headers_complete`
    #[doc(alias = "llhttp__after_headers_complete")]
    pub fn after_headers_complete(&self) -> Result<Body, InvalidTransferEncoding> {
        let status = self.status_code();
        let response = self.is_response();
        let chunked = self.flagged(sys::F_CHUNKED);
        let skip_body = self.flagged(sys::F_SKIPBODY);
        let has_body = chunked || self.content_length() > 0;
        if (self.upgrade() && (self.method() == Ok(Method::Connect) || skip_body || !has_body))
            // RFC 2616, 4.4: 1xx, as `Continue`.
            || (response && status == 101)
        {
            // The rest of the message is in another protocol.
            return Ok(Body::Upgrade);
        }
        if response && status == 100 {
            // No body: the message is complete.
            return Ok(Body::Empty);
        }
        // RFC 2616, 4.4: the response to a `HEAD` request, `Processing`, `Early Hints`,
        // `No Content` and `Not Modified`.
        if skip_body || (response && matches!(status, 102 | 103 | 204 | 304)) {
            Ok(Body::Empty)
        } else if chunked {
            // A chunked encoding overrides the `Content-Length`.
            Ok(Body::Chunked)
        } else if self.flagged(sys::F_TRANSFER_ENCODING) {
            // RFC 7230, 3.3.3: a request's body length cannot be told when its last coding is
            // not `chunked`, which the server answers with a 400; a response's body is read
            // until the connection closes.
            if self.is_request()
                && !self.lenient(sys::LENIENT_CHUNKED_LENGTH)
                && !self.lenient(sys::LENIENT_TRANSFER_ENCODING)
            {
                Err(InvalidTransferEncoding)
            } else {
                Ok(Body::UntilEof)
            }
        } else if !self.flagged(sys::F_CONTENT_LENGTH) {
            if self.message_needs_eof() {
                Ok(Body::UntilEof)
            } else {
                // A length of 0 is assumed: the next message is read.
                Ok(Body::Empty)
            }
        } else if self.content_length() == 0 {
            Ok(Body::Empty)
        } else {
            Ok(Body::Length)
        }
    }
}

impl ParserMut<'_> {
    /// Sets whether the message upgrades the connection, [`upgrade`](ParserRef::upgrade),
    /// as llhttp's generated parser asks once the headers are read and before
    /// `on_headers_complete` runs, so that the callback sees it. A message with both
    /// `Upgrade` and `Connection: upgrade` upgrades when it is a request, or a `101
    /// Switching Protocols` response: for any other response the headers only announce
    /// support. Any other message upgrades when it is a `CONNECT` request.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.1 200 OK\r\nConnection: upgrade\r\nUpgrade: h2c\r\n\r\n")
    ///     .unwrap();
    /// assert!(!parser.as_ref().upgrade());
    /// parser.as_mut().set_upgrade(true);
    /// parser.as_mut().before_headers_complete();
    /// assert!(!parser.as_ref().upgrade());
    /// ```
    ///
    /// Replaces `fn::llhttp__before_headers_complete`
    #[doc(alias = "llhttp__before_headers_complete")]
    pub fn before_headers_complete(&mut self) {
        let upgrade = {
            let parser = self.as_ref();
            if parser.flagged(sys::F_UPGRADE) && parser.flagged(sys::F_CONNECTION_UPGRADE) {
                parser.is_request() || parser.status_code() == 101
            } else {
                parser.method() == Ok(Method::Connect)
            }
        };
        self.set_upgrade(upgrade);
    }

    /// Ends the message, as llhttp's generated parser does once `on_message_complete` has
    /// run: returns whether another message may follow on the same connection
    /// ([`should_keep_alive`](ParserRef::should_keep_alive)), then marks the parser safe to
    /// finish, `HTTP_FINISH_SAFE`, and clears the message's flags for the next.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser
    ///     .as_mut()
    ///     .execute(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 1\r\n\r\n")
    ///     .unwrap();
    /// assert_ne!(parser.as_ref().flags_raw(), 0);
    /// assert!(!parser.as_mut().after_message_complete());
    /// assert_eq!(parser.as_ref().flags_raw(), 0);
    /// ```
    ///
    /// Replaces `fn::llhttp__after_message_complete`
    #[doc(alias = "llhttp__after_message_complete")]
    pub fn after_message_complete(&mut self) -> bool {
        let keep_alive = self.as_ref().should_keep_alive();
        self.set_finish_raw(FINISH_SAFE);
        self.set_flags_raw(0);
        keep_alive
    }
}

/// The `int` `llhttp__after_headers_complete` returns for `outcome`, which llhttp's generated
/// parser switches on.
#[cfg_attr(
    not(any(test, feature = "ffi-export-deps-llhttp-http-c")),
    expect(
        dead_code,
        reason = "only the export feature's gateway hands C the code"
    )
)]
const fn headers_code(outcome: Result<Body, InvalidTransferEncoding>) -> core::ffi::c_int {
    match outcome {
        Ok(Body::Empty) => 0,
        Ok(Body::Upgrade) => 1,
        Ok(Body::Chunked) => 2,
        Ok(Body::Length) => 3,
        Ok(Body::UntilEof) => 4,
        Err(InvalidTransferEncoding) => 5,
    }
}

#[cfg(test)]
mod unit_priv {
    use super::{Body, InvalidTransferEncoding, headers_code};

    /// Each outcome is the code `llhttp.c`'s `switch` goes on from: `1` to complete the
    /// message for another protocol, `2` to a chunk's size, `3` to a `Content-Length` body,
    /// `4` to a body until the input's end, `5` to `HPE_INVALID_TRANSFER_ENCODING`, and any
    /// other, `0`, to complete the message.
    #[test]
    fn headers_code_is_the_code_the_parser_switches_on() {
        assert_eq!(headers_code(Ok(Body::Empty)), 0);
        assert_eq!(headers_code(Ok(Body::Upgrade)), 1);
        assert_eq!(headers_code(Ok(Body::Chunked)), 2);
        assert_eq!(headers_code(Ok(Body::Length)), 3);
        assert_eq!(headers_code(Ok(Body::UntilEof)), 4);
        assert_eq!(headers_code(Err(InvalidTransferEncoding)), 5);
    }

    /// `FINISH_SAFE` is `HTTP_FINISH_SAFE`'s byte.
    #[test]
    fn finish_safe_is_http_finish_safe() {
        assert_eq!(u32::from(super::FINISH_SAFE), super::sys::HTTP_FINISH_SAFE);
    }
}

// The feature's `cfg` is the attribute right above the module, as build.rs's `gateway_lines`
// reads a gateway's feature from the lines before it.
#[expect(
    unsafe_code,
    reason = "exports http.c's functions the remaining C calls, adopting the parser C passes"
)]
#[cfg(feature = "ffi-export-deps-llhttp-http-c")]
mod ffi_export {
    use core::ffi::{c_int, c_uchar};

    use ffibox::callback::barrier;
    use ffibox::{CMutSeam, CRefSeam};

    use super::headers_code;
    use crate::libgit2::deps::llhttp::llhttp::{ParserMut, ParserRef};
    use crate::sys::libgit2 as sys;

    // `llhttp_should_keep_alive` has its raw binding's type: the array's elements coerce to
    // one function pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::llhttp_should_keep_alive, llhttp_should_keep_alive];
    };

    /// A hook's type as `llhttp.c` declares the three it calls (`c/deps/llhttp/llhttp.c`,
    /// lines 711, 753 and 761), `int llhttp__<hook>(llhttp__internal_t* s, const unsigned
    /// char* p, const unsigned char* endp)`: no header declares them, so they have no raw
    /// binding, and each export is checked against this type instead. http.c defines them over
    /// `llhttp_t` and `const char*`, the same types to the ABI.
    type Hook =
        unsafe extern "C" fn(*mut sys::llhttp__internal_t, *const c_uchar, *const c_uchar) -> c_int;

    const _: () = {
        let _: Hook = llhttp__before_headers_complete;
        let _: Hook = llhttp__after_headers_complete;
        let _: Hook = llhttp__after_message_complete;
    };

    /// The exclusive handle of the parser C passes a hook.
    ///
    /// # Safety
    ///
    /// `parser` is NULL, or points at a live `llhttp_t` which nothing else accesses while the
    /// handle lives, and whose `reason` is NULL or one of llhttp's literals: the parser
    /// `llhttp.c`'s generated parser passes a hook from inside its parse.
    unsafe fn parser_mut<'a>(parser: *mut sys::llhttp__internal_t) -> Option<ParserMut<'a>> {
        // SAFETY: the caller guarantees that `parser` is NULL or a live parser, exclusively
        // held for the handle's lifetime, which is the export's call: each export makes its
        // one operation with the handle and drops it before returning. Its `reason` is NULL or
        // one of llhttp's literals, which live as long as the program. The hooks read and
        // write only the parser's integers, never its `data` or `settings`, whose contracts
        // are therefore not relied on.
        unsafe { ParserMut::from_ptr(parser) }
    }

    /// `llhttp__before_headers_complete`: [`ParserMut::before_headers_complete`], returning
    /// `0`; `p` and `endp` are not read. It cannot fail, so a panic aborts, as a NULL parser,
    /// which C would follow, does.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp__before_headers_complete(
        parser: *mut sys::llhttp__internal_t,
        _p: *const c_uchar,
        _endp: *const c_uchar,
    ) -> c_int {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let parser = unsafe { parser_mut(parser) };
        barrier(
            || {
                parser
                    .expect("llhttp.c passes its parser")
                    .before_headers_complete();
                0
            },
            || std::process::abort(),
        )
    }

    /// `llhttp__after_headers_complete`: [`ParserRef::after_headers_complete`], as the code
    /// `llhttp.c` switches on; `p` and `endp` are not read. It has no failure a panic could
    /// report, its `5` being a request's own, so a panic aborts, as a NULL parser does.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp__after_headers_complete(
        parser: *mut sys::llhttp__internal_t,
        _p: *const c_uchar,
        _endp: *const c_uchar,
    ) -> c_int {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let parser = unsafe { parser_mut(parser) };
        barrier(
            || {
                let parser = parser.expect("llhttp.c passes its parser");
                headers_code(parser.as_ref().after_headers_complete())
            },
            || std::process::abort(),
        )
    }

    /// `llhttp__after_message_complete`: [`ParserMut::after_message_complete`], `1` to keep
    /// the connection and `0` not to; `p` and `endp` are not read. It cannot fail, so a panic
    /// aborts, as a NULL parser does.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp__after_message_complete(
        parser: *mut sys::llhttp__internal_t,
        _p: *const c_uchar,
        _endp: *const c_uchar,
    ) -> c_int {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let parser = unsafe { parser_mut(parser) };
        barrier(
            || {
                let keep_alive = parser
                    .expect("llhttp.c passes its parser")
                    .after_message_complete();
                c_int::from(keep_alive)
            },
            || std::process::abort(),
        )
    }

    /// `llhttp_should_keep_alive`: [`ParserRef::should_keep_alive`], `1` or `0`. It cannot
    /// fail, so a panic aborts, as a NULL parser does.
    ///
    /// # Safety
    ///
    /// `parser` is NULL, or points at a live `llhttp_t` which nothing writes during the call,
    /// and whose `reason` is NULL or one of llhttp's literals.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_should_keep_alive(parser: *const sys::llhttp_t) -> c_int {
        // SAFETY: the caller guarantees that `parser` is NULL or a live parser nothing writes
        // while the handle lives, the call, and whose `reason` outlives it: the shared
        // handle's invariant. The handle is only read through.
        let parser = unsafe { ParserRef::from_ptr(parser.cast_mut()) };
        barrier(
            || {
                let keep_alive = parser
                    .expect("libgit2's HTTP client passes its parser")
                    .should_keep_alive();
                c_int::from(keep_alive)
            },
            || std::process::abort(),
        )
    }

    /// The exports called as C calls them, over parsers Rust builds: what each reads and
    /// writes of the parser, and the `int` it returns. They compile with the gateway, under
    /// the export feature, and link only where http.c's C is out of the link.
    #[cfg(test)]
    mod unit_priv {
        use core::ffi::c_int;
        use core::ptr;

        use ffibox::{CMutSeam, CRefSeam, CZeroable};

        use super::{
            llhttp__after_headers_complete, llhttp__after_message_complete,
            llhttp__before_headers_complete, llhttp_should_keep_alive,
        };
        use crate::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
        use crate::sys::libgit2 as sys;

        /// A zeroed parser of `parser_type` with `flags` and `upgrade` set.
        fn parser(parser_type: ParserType, flags: u32, upgrade: bool) -> Parser {
            let mut parser = Parser::zeroed();
            let mut handle = parser.as_mut();
            handle.set_parser_type(parser_type);
            handle.set_flags_raw(u16::try_from(flags).unwrap());
            handle.set_upgrade(upgrade);
            parser
        }

        /// Calls `hook` with `parser` as `llhttp.c` does, its `p` and `endp` dangling, since
        /// the hooks never read them.
        fn call(
            hook: unsafe extern "C" fn(*mut sys::llhttp__internal_t, *const u8, *const u8) -> c_int,
            parser: &mut Parser,
        ) -> c_int {
            let p = ptr::dangling::<u8>();
            // SAFETY: the handle addresses a live parser, which nothing else accesses during
            // the call, and whose `reason` is NULL; the hooks never read `p` and `endp`.
            unsafe { hook(parser.as_mut().as_mut_ptr(), p, p) }
        }

        #[test]
        fn before_headers_complete_sets_upgrade_and_returns_0() {
            let both = sys::F_UPGRADE | sys::F_CONNECTION_UPGRADE;
            let mut request = parser(ParserType::Request, both, false);
            assert_eq!(call(llhttp__before_headers_complete, &mut request), 0);
            assert!(request.as_ref().upgrade());
            let mut response = parser(ParserType::Response, both, true);
            assert_eq!(call(llhttp__before_headers_complete, &mut response), 0);
            assert!(!response.as_ref().upgrade());
        }

        #[test]
        fn after_headers_complete_returns_cs_codes() {
            let cases = [
                (ParserType::Request, 0, false, 0),
                (ParserType::Request, 0, true, 1),
                (ParserType::Request, sys::F_CHUNKED, false, 2),
                (ParserType::Response, 0, false, 4),
                (ParserType::Request, sys::F_TRANSFER_ENCODING, false, 5),
            ];
            for (parser_type, flags, upgrade, code) in cases {
                let mut p = parser(parser_type, flags, upgrade);
                let before = p.as_ref().flags_raw();
                assert_eq!(
                    call(llhttp__after_headers_complete, &mut p),
                    code,
                    "{parser_type:?} {flags:#x} {upgrade}"
                );
                assert_eq!(p.as_ref().flags_raw(), before);
            }
        }

        #[test]
        fn after_message_complete_returns_keep_alive_and_clears_the_message() {
            let mut p = parser(ParserType::Request, sys::F_CONNECTION_KEEP_ALIVE, true);
            p.as_mut().set_finish_raw(2);
            assert_eq!(call(llhttp__after_message_complete, &mut p), 1);
            assert_eq!(p.as_ref().flags_raw(), 0);
            assert_eq!(u32::from(p.as_ref().finish_raw()), sys::HTTP_FINISH_SAFE);
            assert!(p.as_ref().upgrade());
            // HTTP/0.0 without keep-alive, its flags now clear.
            assert_eq!(call(llhttp__after_message_complete, &mut p), 0);
        }

        #[test]
        fn should_keep_alive_reads_a_const_parser() {
            let keep = parser(ParserType::Request, sys::F_CONNECTION_KEEP_ALIVE, false);
            let close = parser(ParserType::Request, 0, false);
            for (p, expected) in [(&keep, 1), (&close, 0)] {
                let before = p.as_ref().flags_raw();
                // SAFETY: the handle addresses a live parser, which nothing writes during the
                // call, and whose `reason` is NULL.
                let got = unsafe { llhttp_should_keep_alive(p.as_ref().as_ptr()) };
                assert_eq!(got, expected);
                assert_eq!(p.as_ref().flags_raw(), before);
            }
        }
    }
}
