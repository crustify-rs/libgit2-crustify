//! `deps/llhttp/api.c`: llhttp's public API over its generated parser, ported to Rust.
//!
//! # The settings
//!
//! A [`Settings`] is llhttp's table of callbacks, `struct llhttp_settings_s`, which
//! `llhttp.h` also names `llhttp_settings_t` and libgit2's `transports/httpparser.h`
//! `git_http_parser_settings` ([`SettingsT`], [`HttpParserSettings`]). It has a slot per event
//! of a message, each NULL or a callback: a [`Callback`] for an event, such as the start of a
//! message or the end of its headers, and a [`DataCallback`] for a run of the input that makes
//! up a part of it, such as a header's name or the body. A parser keeps the address of the table
//! it is given (`llhttp_init`, [`ParserMut::set_settings`]), and api.c's dispatchers read the
//! table on every event of a parse, calling the slot's callback with the parser, or nothing when
//! the slot is NULL.
//!
//! The table owns nothing and holds no context: llhttp's callbacks are handed the parser, whose
//! `data` is their context. It keeps its C layout until its `port/layout` stage, since libgit2's
//! HTTP client builds one in C, a `static` filled on first use, and hands it to `llhttp_init`.
//! A table Rust holds is a plain value, built empty by [`Settings::new`] (`llhttp_settings_init`)
//! and filled through [`SettingsMut`]'s setters, one per slot; [`SettingsRef`]'s getters read
//! the slots back as the callable types.
//!
//! # What a table may hold
//!
//! Each slot is set only from a [`Callback`] or a [`DataCallback`]: a Rust handler behind its
//! trampoline, or, inside the crate, a C function that is sound to call with any parser, whatever
//! its `data` holds. So every callback of a table Rust builds is sound with every parser, and a
//! table installed with [`ParserMut::set_settings`] needs no promise about the parser's `data`:
//! that setter is safe, taking a `SettingsRef<'static>`, whose table lives and stays unchanged for
//! as long as any parser can keep its address.
//!
//! A C callback that reads its context through the parser's `data`, as each of libgit2's HTTP
//! client's does with its `http_parser_context`, is sound only with the parsers whose `data` it
//! expects, so it is no `Callback`, and a table of them is no `Settings` Rust can build. Such a
//! table is installed by C, or through the `unsafe` [`ParserMut::set_settings_ptr`], whose caller
//! vouches for the parser's `data`, and its callbacks are reached only through the parser that
//! holds it, by api.c's dispatchers. That is why the parser's `settings` is read back as the
//! address it holds ([`ParserRef::settings_ptr`]), not as a [`SettingsRef`], whose getters
//! hand out callbacks callable with any parser.
//!
//!
//! # The parser's API
//!
//! api.c's functions over a parser are methods of its handles, [`ParserRef`] and
//! [`ParserMut`]. Most of them read or write one field, and are that field's accessor in
//! `llhttp.rs`, which stands in for both: `llhttp_get_type`, `llhttp_get_method` and
//! `llhttp_get_errno` are [`ParserRef::parser_type`], [`ParserRef::method`] and
//! [`ParserRef::error`], read as llhttp's enums; `llhttp_get_http_major`,
//! `llhttp_get_http_minor`, `llhttp_get_status_code` and `llhttp_get_upgrade` are
//! [`ParserRef::http_major`], [`ParserRef::http_minor`], [`ParserRef::status_code`] and
//! [`ParserRef::upgrade`]; `llhttp_get_error_reason` and `llhttp_set_error_reason` are
//! [`ParserRef::reason`] and [`ParserMut::set_reason`]; and `llhttp_get_error_pos`, whose
//! pointer into a past input Rust never follows, is [`ParserRef::error_offset`], which
//! locates it in the input it is given. The X-macro generators api.c's name functions
//! expand, `HTTP_ERRNO_GEN`, `HTTP_METHOD_GEN` and `HTTP_STATUS_GEN`, turn a map's entry into
//! the text its `case` returns, which is [`Errno::constant_name`], [`Method::map_string`]
//! and [`Status::map_string`].
//!
//! What has logic of its own is here: [`ParserMut::execute`] (`llhttp_execute`), the
//! pause and its two resumptions ([`ParserMut::pause`], [`ParserMut::resume`],
//! [`ParserMut::resume_after_upgrade`]), which move the parser between its running and
//! paused states only from the state each expects, the ten lenient setters, each of which
//! sets or clears one bit of the lenient flags, and [`ParserRef::debug_trace`]
//! (`llhttp__debug`), the trace llhttp's generated parser writes to C's `stderr` when it is
//! generated in debug mode, which this build's is not. The parser's lifecycle is here too:
//! [`ParserMut::init`] (`llhttp_init`) starts it afresh with a type and a table,
//! [`ParserMut::reset`] (`llhttp_reset`) starts it afresh between two connections' messages,
//! keeping its type, table, `data` and lenient flags, and [`ParserMut::finish`]
//! (`llhttp_finish`) tells it the input has ended. The name functions `llhttp_errno_name`,
//! `llhttp_method_name` and `llhttp_status_name` are the maps' [`Errno::constant_name`],
//! [`Method::map_string`] and [`Status::map_string`], whose `abort` on a value no entry names
//! an enum cannot reach.
//!
//! # The dispatchers
//!
//! llhttp's generated parser signals each event of a message through one of api.c's 23
//! dispatchers, `llhttp__on_<slot>`, which call the slot of the parser's table of the same
//! name with the parser, or return `0` when the parser has no table or the slot is NULL. The
//! macro `CALLBACK_MAYBE` makes the 14 that signal an event, such as
//! [`ParserMut::on_message_begin`], and `SPAN_CALLBACK_MAYBE` the 9 that hand over a run of the
//! input, such as [`ParserMut::on_url`], which also turn the callback's `-1` into `HPE_USER`,
//! with the reason `"Span callback error in <slot>"`. They are the parser's methods of the
//! same name, each over one of two private functions that stand in for the macros.
//!
//! The table they read is the one the parser holds, which C may have built of callbacks that
//! read the parser's `data` as their context: they reach it through the parser alone, so
//! that each callback is called only with the parser whose table holds it, as the contracts
//! of [`ParserMut::set_settings_ptr`] and [`ParserMut::set_data`] promise it may be.
//!
//! # The exports
//!
//! C outside api.c calls nine of these functions and the 23 dispatchers. libgit2's
//! `transports/httpparser.c` calls `llhttp_execute`, `llhttp_finish`, `llhttp_get_error_pos`
//! and `llhttp_resume_after_upgrade`, and the macros of `transports/httpparser.h`, which
//! `transports/httpclient.c` expands, `llhttp_init`, `llhttp_pause`, `llhttp_resume` and
//! `llhttp_get_error_reason`; `llhttp.c`'s generated parser calls the dispatchers. They are
//! exported, under the module's export feature, `ffi-export-deps-llhttp-api-c`, which stays
//! off until api.c switches from C.
//!
//! [`Callback`]: super::llhttp::Callback
//! [`DataCallback`]: super::llhttp::DataCallback
//! [`ParserMut::set_settings`]: super::llhttp::ParserMut::set_settings
//! [`ParserMut::set_settings_ptr`]: super::llhttp::ParserMut::set_settings_ptr
//! [`ParserRef::settings_ptr`]: super::llhttp::ParserRef::settings_ptr
//! [`ParserRef`]: super::llhttp::ParserRef
//! [`ParserMut`]: super::llhttp::ParserMut
//! [`ParserRef::parser_type`]: super::llhttp::ParserRef::parser_type
//! [`ParserRef::method`]: super::llhttp::ParserRef::method
//! [`ParserRef::error`]: super::llhttp::ParserRef::error
//! [`ParserRef::http_major`]: super::llhttp::ParserRef::http_major
//! [`ParserRef::http_minor`]: super::llhttp::ParserRef::http_minor
//! [`ParserRef::status_code`]: super::llhttp::ParserRef::status_code
//! [`ParserRef::upgrade`]: super::llhttp::ParserRef::upgrade
//! [`ParserRef::reason`]: super::llhttp::ParserRef::reason
//! [`ParserMut::set_reason`]: super::llhttp::ParserMut::set_reason
//! [`ParserRef::error_offset`]: super::llhttp::ParserRef::error_offset
//! [`Errno::constant_name`]: super::llhttp::Errno::constant_name
//! [`Method::map_string`]: super::llhttp::Method::map_string
//! [`Status::map_string`]: super::llhttp::Status::map_string
//! [`ParserMut::set_data`]: super::llhttp::ParserMut::set_data

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use core::ffi::{CStr, c_int};
use core::fmt;
use core::num::NonZero;
use std::io;

use ffibox::{CRefSeam, CZeroable};

pub use self::ffi_import::{Settings, SettingsMut, SettingsRef};
use super::llhttp::{Errno, ParseError, Parsed, ParserMut, ParserRef, ParserType, UnnamedErrno};
use crate::sys::libgit2 as sys;

/// Hands `$then!` api.c's 23 dispatchers, `llhttp__on_<slot>`, in api.c's order within each
/// kind: those `SPAN_CALLBACK_MAYBE` makes, which hand a run of the input to a data slot, then
/// those `CALLBACK_MAYBE` makes, which signal an event to an event slot. Each is its export's
/// name, its slot, which names its method too, and its slot's variant of [`SpanSlot`] or
/// [`EventSlot`], with its method's anchor and what it signals. The native methods, the
/// region's reads of each slot and the gateway's exports are made from this one list.
macro_rules! with_dispatchers {
    ($then:ident) => {
        $then! {
            spans {
                /// Replaces `fn::llhttp__on_url`
                #[doc(alias = "llhttp__on_url")]
                llhttp__on_url, on_url, Url: "a run of a request's URL";
                /// Replaces `fn::llhttp__on_status`
                #[doc(alias = "llhttp__on_status")]
                llhttp__on_status, on_status, Status: "a run of a response's status text";
                /// Replaces `fn::llhttp__on_method`
                #[doc(alias = "llhttp__on_method")]
                llhttp__on_method, on_method, Method: "a run of a request's method";
                /// Replaces `fn::llhttp__on_version`
                #[doc(alias = "llhttp__on_version")]
                llhttp__on_version, on_version, Version: "a run of the message's HTTP version";
                /// Replaces `fn::llhttp__on_header_field`
                #[doc(alias = "llhttp__on_header_field")]
                llhttp__on_header_field, on_header_field, HeaderField: "a run of a header's name";
                /// Replaces `fn::llhttp__on_header_value`
                #[doc(alias = "llhttp__on_header_value")]
                llhttp__on_header_value, on_header_value, HeaderValue: "a run of a header's value";
                /// Replaces `fn::llhttp__on_body`
                #[doc(alias = "llhttp__on_body")]
                llhttp__on_body, on_body, Body: "a run of the message's body";
                /// Replaces `fn::llhttp__on_chunk_extension_name`
                #[doc(alias = "llhttp__on_chunk_extension_name")]
                llhttp__on_chunk_extension_name, on_chunk_extension_name, ChunkExtensionName:
                    "a run of a chunk extension's name";
                /// Replaces `fn::llhttp__on_chunk_extension_value`
                #[doc(alias = "llhttp__on_chunk_extension_value")]
                llhttp__on_chunk_extension_value, on_chunk_extension_value, ChunkExtensionValue:
                    "a run of a chunk extension's value";
            }
            events {
                /// Replaces `fn::llhttp__on_message_begin`
                #[doc(alias = "llhttp__on_message_begin")]
                llhttp__on_message_begin, on_message_begin, MessageBegin:
                    "the start of a message";
                /// Replaces `fn::llhttp__on_url_complete`
                #[doc(alias = "llhttp__on_url_complete")]
                llhttp__on_url_complete, on_url_complete, UrlComplete: "the end of a request's URL";
                /// Replaces `fn::llhttp__on_status_complete`
                #[doc(alias = "llhttp__on_status_complete")]
                llhttp__on_status_complete, on_status_complete, StatusComplete:
                    "the end of a response's status text";
                /// Replaces `fn::llhttp__on_method_complete`
                #[doc(alias = "llhttp__on_method_complete")]
                llhttp__on_method_complete, on_method_complete, MethodComplete:
                    "the end of a request's method";
                /// Replaces `fn::llhttp__on_version_complete`
                #[doc(alias = "llhttp__on_version_complete")]
                llhttp__on_version_complete, on_version_complete, VersionComplete:
                    "the end of the message's HTTP version";
                /// Replaces `fn::llhttp__on_header_field_complete`
                #[doc(alias = "llhttp__on_header_field_complete")]
                llhttp__on_header_field_complete, on_header_field_complete, HeaderFieldComplete:
                    "the end of a header's name";
                /// Replaces `fn::llhttp__on_header_value_complete`
                #[doc(alias = "llhttp__on_header_value_complete")]
                llhttp__on_header_value_complete, on_header_value_complete, HeaderValueComplete:
                    "the end of a header's value";
                /// Replaces `fn::llhttp__on_headers_complete`
                #[doc(alias = "llhttp__on_headers_complete")]
                llhttp__on_headers_complete, on_headers_complete, HeadersComplete:
                    "the end of the message's headers";
                /// Replaces `fn::llhttp__on_message_complete`
                #[doc(alias = "llhttp__on_message_complete")]
                llhttp__on_message_complete, on_message_complete, MessageComplete:
                    "the end of a message";
                /// Replaces `fn::llhttp__on_chunk_header`
                #[doc(alias = "llhttp__on_chunk_header")]
                llhttp__on_chunk_header, on_chunk_header, ChunkHeader: "a chunk's header";
                /// Replaces `fn::llhttp__on_chunk_extension_name_complete`
                #[doc(alias = "llhttp__on_chunk_extension_name_complete")]
                llhttp__on_chunk_extension_name_complete, on_chunk_extension_name_complete,
                    ChunkExtensionNameComplete: "the end of a chunk extension's name";
                /// Replaces `fn::llhttp__on_chunk_extension_value_complete`
                #[doc(alias = "llhttp__on_chunk_extension_value_complete")]
                llhttp__on_chunk_extension_value_complete, on_chunk_extension_value_complete,
                    ChunkExtensionValueComplete: "the end of a chunk extension's value";
                /// Replaces `fn::llhttp__on_chunk_complete`
                #[doc(alias = "llhttp__on_chunk_complete")]
                llhttp__on_chunk_complete, on_chunk_complete, ChunkComplete: "the end of a chunk";
                /// Replaces `fn::llhttp__on_reset`
                #[doc(alias = "llhttp__on_reset")]
                llhttp__on_reset, on_reset, Reset:
                    "the reset of the parser between two messages of one input";
            }
        }
    };
}

/// `struct llhttp_settings_s` under the name `llhttp.h` gives it beside its tag,
/// `llhttp_settings_t`.
///
/// Wraps `tdf::llhttp_settings_t`
#[doc(alias = "llhttp_settings_t")]
pub type SettingsT = Settings;

/// `struct llhttp_settings_s` under the name libgit2's HTTP client gives it,
/// `git_http_parser_settings` (`transports/httpparser.h`), when llhttp is its parser.
///
/// Wraps `tdf::git_http_parser_settings`
#[doc(alias = "git_http_parser_settings")]
pub type HttpParserSettings = Settings;

// --- The parser's API ------------------------------------------------------------------

impl ParserMut<'_> {
    /// Parses `data`, the next run of the input, calling the callbacks of the parser's
    /// settings as it reads each part of a message: llhttp's entry point, which runs its
    /// generated parser, [`parse`](Self::parse), over the run.
    ///
    /// The parser picks up where the previous run stopped. A pause, by a callback or by
    /// [`pause`](Self::pause), or an upgrade stops the run early, at the parser's error
    /// position ([`ParserRef::error_offset`]), and each later run returns the same outcome
    /// without reading until the parser is resumed.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parsed, Parser};
    ///
    /// let mut parser = Parser::new();
    /// let response = b"HTTP/1.1 204 No Content\r\n\r\n";
    /// assert_eq!(parser.as_mut().execute(response), Ok(Parsed::All));
    /// assert_eq!(parser.as_ref().status_code(), 204);
    /// ```
    ///
    /// # Errors
    ///
    /// The [`ParseError`] the parser stopped at, which it keeps until it is initialised
    /// again, as [`parse`](Self::parse) returns it.
    ///
    /// # Panics
    ///
    /// As [`parse`](Self::parse): when the parser has no state to run from, where C would
    /// abort, and when a callback of the parser's own parse executes it again.
    ///
    /// Replaces `fn::llhttp_execute`
    #[doc(alias = "llhttp_execute")]
    pub fn execute(&mut self, data: &[u8]) -> Result<Parsed, ParseError> {
        self.parse(data)
    }

    /// Pauses the parser: a parser with no error takes [`Errno::Paused`], with the reason
    /// `"Paused"`, so that its next run returns [`Parsed::Paused`] without reading. A parser
    /// that already has an error, a pause among them, keeps it, and its reason.
    ///
    /// llhttp asks a callback that pauses its parse to return `HPE_PAUSED` rather than call
    /// this; libgit2's HTTP client does both at the end of a response's headers. Called from
    /// a callback that returns `0`, it leaves the run reading on, and the next run paused.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Errno, Parsed, Parser};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().pause();
    /// assert_eq!(parser.as_ref().error(), Ok(Errno::Paused));
    /// assert_eq!(parser.as_ref().reason(), Some(c"Paused"));
    /// assert_eq!(
    ///     parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n"),
    ///     Ok(Parsed::Paused)
    /// );
    /// parser.as_mut().resume();
    /// assert_eq!(
    ///     parser.as_mut().execute(b"HTTP/1.1 200 OK\r\n"),
    ///     Ok(Parsed::All)
    /// );
    /// ```
    ///
    /// Replaces `fn::llhttp_pause`
    #[doc(alias = "llhttp_pause")]
    pub fn pause(&mut self) {
        if self.as_ref().error_raw() != 0 {
            return;
        }
        self.set_error(Errno::Paused);
        self.set_reason(Some(c"Paused"));
    }

    /// Resumes a paused parser: one whose error is [`Errno::Paused`] has it cleared, so that
    /// its next run reads on from where it stopped. Any other error, an upgrade's pause
    /// among them, stays; the reason stays too.
    ///
    /// Replaces `fn::llhttp_resume`
    #[doc(alias = "llhttp_resume")]
    pub fn resume(&mut self) {
        self.resume_from(Errno::Paused);
    }

    /// Resumes a parser paused at an upgrade: one whose error is [`Errno::PausedUpgrade`]
    /// has it cleared, so that its next run reads what follows as HTTP again. Any other
    /// error stays, and the reason too.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Errno, Parsed, Parser};
    ///
    /// let mut parser = Parser::new();
    /// let upgrade = b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\n\
    ///                 Upgrade: websocket\r\n\r\n";
    /// assert_eq!(parser.as_mut().execute(upgrade), Ok(Parsed::PausedUpgrade));
    /// parser.as_mut().resume();
    /// assert_eq!(parser.as_ref().error(), Ok(Errno::PausedUpgrade));
    /// parser.as_mut().resume_after_upgrade();
    /// assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
    /// ```
    ///
    /// Replaces `fn::llhttp_resume_after_upgrade`
    #[doc(alias = "llhttp_resume_after_upgrade")]
    pub fn resume_after_upgrade(&mut self) {
        self.resume_from(Errno::PausedUpgrade);
    }

    /// Clears the parser's error when it is `pause`, and leaves it otherwise.
    fn resume_from(&mut self, pause: Errno) {
        if self.as_ref().error_raw() == pause.raw().cast_signed() {
            self.set_error(Errno::Ok);
        }
    }

    /// Sets the lenient flag `flag` when `enabled`, and clears it otherwise, leaving the
    /// others as they are.
    fn set_lenient(&mut self, flag: u16, enabled: bool) {
        let flags = self.as_ref().lenient_flags_raw();
        self.set_lenient_flags_raw(if enabled { flags | flag } else { flags & !flag });
    }
}

/// An `llhttp_lenient_flags` bit as the `uint16_t` `lenient_flags` holds it.
const fn lenient(flag: sys::llhttp_lenient_flags) -> u16 {
    let [low, high, 0, 0] = flag.to_le_bytes() else {
        panic!("a lenient flag is wider than `lenient_flags`")
    };
    u16::from_le_bytes([low, high])
}

/// The lenient setters, each over one bit of `llhttp_lenient_flags`, the setter's anchor on
/// it.
macro_rules! lenient_setters {
    (
        $(
            $(#[$anchor:meta])*
            $setter:ident, $flag:ident: $what:literal;
        )*
    ) => {
        impl ParserMut<'_> {
            $(
                #[doc = concat!("Relaxes, when `enabled`, or restores, the parser's check that ",
                                $what, ": the lenient flag `", stringify!($flag), "`. \
                                The other flags stay as they are.")]
                ///
                /// Each relaxation lets through input the HTTP specification refuses, which
                /// llhttp's documentation warns may expose a user to request smuggling or
                /// cache poisoning.
                ///
                $(#[$anchor])*
                pub fn $setter(&mut self, enabled: bool) {
                    self.set_lenient(const { lenient(sys::$flag) }, enabled);
                }
            )*
        }
    };
}

lenient_setters! {
    /// Replaces `fn::llhttp_set_lenient_headers`
    #[doc(alias = "llhttp_set_lenient_headers")]
    set_lenient_headers, LENIENT_HEADERS:
        "a header's value holds only the characters HTTP allows there";
    /// Replaces `fn::llhttp_set_lenient_chunked_length`
    #[doc(alias = "llhttp_set_lenient_chunked_length")]
    set_lenient_chunked_length, LENIENT_CHUNKED_LENGTH:
        "a message does not have both `Transfer-Encoding` and `Content-Length`";
    /// Replaces `fn::llhttp_set_lenient_keep_alive`
    #[doc(alias = "llhttp_set_lenient_keep_alive")]
    set_lenient_keep_alive, LENIENT_KEEP_ALIVE:
        "no message follows one after which `Connection: close` or HTTP/1.0 ends the \
         connection";
    /// Replaces `fn::llhttp_set_lenient_transfer_encoding`
    #[doc(alias = "llhttp_set_lenient_transfer_encoding")]
    set_lenient_transfer_encoding, LENIENT_TRANSFER_ENCODING:
        "no value follows `chunked` in a `Transfer-Encoding`";
    /// Replaces `fn::llhttp_set_lenient_version`
    #[doc(alias = "llhttp_set_lenient_version")]
    set_lenient_version, LENIENT_VERSION:
        "the HTTP version is `0.9`, `1.0`, `1.1` or `2.0`";
    /// Replaces `fn::llhttp_set_lenient_data_after_close`
    #[doc(alias = "llhttp_set_lenient_data_after_close")]
    set_lenient_data_after_close, LENIENT_DATA_AFTER_CLOSE:
        "no data follows a message marked `Connection: close` (data that does is discarded \
         when relaxed)";
    /// Replaces `fn::llhttp_set_lenient_optional_lf_after_cr`
    #[doc(alias = "llhttp_set_lenient_optional_lf_after_cr")]
    set_lenient_optional_lf_after_cr, LENIENT_OPTIONAL_LF_AFTER_CR:
        "each CR ending the request or status line, a header or a chunk header is followed by \
         an LF";
    /// Replaces `fn::llhttp_set_lenient_optional_crlf_after_chunk`
    #[doc(alias = "llhttp_set_lenient_optional_crlf_after_chunk")]
    set_lenient_optional_crlf_after_chunk, LENIENT_OPTIONAL_CRLF_AFTER_CHUNK:
        "a CRLF follows each chunk's data before the next chunk";
    /// Replaces `fn::llhttp_set_lenient_optional_cr_before_lf`
    #[doc(alias = "llhttp_set_lenient_optional_cr_before_lf")]
    set_lenient_optional_cr_before_lf, LENIENT_OPTIONAL_CR_BEFORE_LF:
        "each LF ending the request or status line, a header, a chunk header or a chunk's \
         data is preceded by a CR";
    /// Replaces `fn::llhttp_set_lenient_spaces_after_chunk_size`
    #[doc(alias = "llhttp_set_lenient_spaces_after_chunk_size")]
    set_lenient_spaces_after_chunk_size, LENIENT_SPACES_AFTER_CHUNK_SIZE:
        "a CRLF or `;`, and no space, follows a chunk's size";
}

impl ParserRef<'_> {
    /// Writes a trace of the parser to C's standard error stream, `stderr`, as llhttp's
    /// generated parser does at each step when it is generated in debug mode: the parser's
    /// address, its type and flags, the next byte of `rest`, the input still to read (`null`
    /// when it is empty), and `msg`, one line in one write:
    ///
    /// ```text
    /// p=0x7ffc3a1c2e40 type=1 flags=00 next=48   debug=on_message_begin
    /// ```
    ///
    /// The line goes through C's stream, in order with what the remaining C writes there.
    /// `type` is printed in decimal and the flags and the byte in hexadecimal, two digits at
    /// least. A byte of 0x80 or more is printed as two digits, where C, which prints a
    /// signed `char` through `%02x`, prints it sign-extended to an `unsigned int`
    /// (`ffffff80`), a value `%x` is not defined for.
    ///
    /// # Errors
    ///
    /// When `msg` holds a NUL byte, which C's stream cannot carry, or the write fails, as
    /// [`FileRef::print`](glibc_bindings::libc::libio::genops::FileRef::print) reports it.
    ///
    /// Replaces `fn::llhttp__debug`
    #[doc(alias = "llhttp__debug")]
    pub fn debug_trace(&self, rest: &[u8], msg: &str) -> io::Result<()> {
        ffi_import::print_to_stderr(&self.debug_line(rest, msg))
    }

    /// The line [`debug_trace`](Self::debug_trace) writes, its newline included.
    ///
    /// It is built from its parts, without `format_args!`, whose expansion holds an
    /// `unsafe` block outside the module's regions.
    fn debug_line(&self, rest: &[u8], msg: &str) -> String {
        let mut line = String::from("p=0x");
        push_hex(&mut line, self.as_ptr().addr(), 1);
        line.push_str(" type=");
        line.push_str(&self.parser_type_raw().to_string());
        line.push_str(" flags=");
        push_hex(&mut line, usize::from(self.flags_raw()), 2);
        match rest.first() {
            None => line.push_str(" next=null"),
            Some(&next) => {
                line.push_str(" next=");
                push_hex(&mut line, usize::from(next), 2);
                line.push_str("  ");
            }
        }
        line.push_str(" debug=");
        line.push_str(msg);
        line.push('\n');
        line
    }
}

/// Appends `value` in lowercase hexadecimal, at least `digits` digits, zero-padded, as
/// printf's `%0<digits>x` and `%p`'s digits print it.
fn push_hex(line: &mut String, value: usize, digits: usize) {
    let width = usize::try_from(value.bit_width().div_ceil(4)).unwrap_or(digits);
    for i in (0..width.max(digits)).rev() {
        let nibble = (value >> (i * 4)) & 0xF;
        line.push(char::from(b"0123456789abcdef"[nibble]));
    }
}

// --- The table's constructor -------------------------------------------------------------

impl Settings {
    /// An empty table: every slot NULL, so that a parser given it runs no callback, as
    /// `llhttp_settings_init` leaves the table it is given, which it `memset`s to zero.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
    ///
    /// let settings = Settings::new();
    /// assert!(settings.as_ref().on_body().is_none());
    /// assert!(settings.as_ref().on_message_complete().is_none());
    /// ```
    ///
    /// Replaces `fn::llhttp_settings_init`
    #[doc(alias = "llhttp_settings_init")]
    #[must_use]
    pub fn new() -> Self {
        // All-zero is the table `memset` leaves, every slot NULL (`Settings`' `CZeroable`).
        <Self as CZeroable>::zeroed()
    }
}

impl Default for Settings {
    /// [`Settings::new`].
    fn default() -> Self {
        Self::new()
    }
}

// --- The parser's lifecycle ------------------------------------------------------------

impl ParserMut<'_> {
    /// Initialises the parser, in place, to read messages of `parser_type` and signal their
    /// events to `settings`: as [`reinit`](Self::reinit) leaves it, every field zero but its
    /// start state, `data` included, then its type and its table stored, which `reset` keeps.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parsed, Parser, ParserType};
    ///
    /// static SETTINGS: std::sync::LazyLock<Settings> = std::sync::LazyLock::new(Settings::new);
    ///
    /// let mut parser = Parser::new();
    /// parser
    ///     .as_mut()
    ///     .init(ParserType::Request, Some(SETTINGS.as_ref()));
    /// assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Request));
    /// assert_eq!(
    ///     parser.as_mut().execute(b"GET / HTTP/1.1\r\n\r\n"),
    ///     Ok(Parsed::All)
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// As [`reinit`](Self::reinit): when a callback of the parser's own parse calls it.
    ///
    /// Replaces `fn::llhttp_init`
    #[doc(alias = "llhttp_init")]
    pub fn init(&mut self, parser_type: ParserType, settings: Option<SettingsRef<'static>>) {
        self.reinit();
        self.set_parser_type(parser_type);
        self.set_settings(settings);
    }

    /// Initialises the parser again, in place, as [`init`](Self::init) does, but keeping its
    /// type, its table, its `data` and its lenient flags: for the next message of another
    /// input, with the parser's error, reason and the state of its message cleared.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parsed, Parser, ParserType};
    ///
    /// let mut parser = Parser::new();
    /// parser.as_mut().init(ParserType::Response, None);
    /// parser.as_mut().set_lenient_version(true);
    /// assert!(parser.as_mut().execute(b"GET / HTTP/1.1\r\n").is_err());
    /// parser.as_mut().reset();
    /// assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Response));
    /// assert_eq!(parser.as_ref().reason(), None);
    /// assert_eq!(
    ///     parser.as_mut().execute(b"HTTP/3.7 200 OK\r\n"),
    ///     Ok(Parsed::All)
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// As [`reinit`](Self::reinit): when a callback of the parser's own parse calls it.
    ///
    /// Replaces `fn::llhttp_reset`
    #[doc(alias = "llhttp_reset")]
    pub fn reset(&mut self) {
        let parser_type = self.as_ref().parser_type_raw();
        let lenient_flags = self.as_ref().lenient_flags_raw();
        self.reinit_keeping_context();
        self.set_parser_type_byte(parser_type);
        self.set_lenient_flags_raw(lenient_flags);
    }

    /// Tells the parser that its input has ended, as at the close of the connection it reads.
    /// A message whose end only the input's marks, such as a response with neither a
    /// `Content-Length` nor chunks, ends here, and its table's `on_message_complete` runs.
    ///
    /// The outcome follows the parser's `finish` state, which its parse keeps: between two
    /// messages, or inside one that needs nothing more, the input may end, and the parser
    /// finishes; inside a message whose end must be read, it may not.
    ///
    /// A parser that has an error, a pause among them, finishes at once, without a callback,
    /// and keeps its error.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::api::FinishError;
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Parsed, Parser};
    ///
    /// let mut parser = Parser::new();
    /// let response = b"HTTP/1.1 200 OK\r\n\r\nthe body runs to the end of the input";
    /// assert_eq!(parser.as_mut().execute(response), Ok(Parsed::All));
    /// assert_eq!(parser.as_mut().finish(), Ok(()));
    ///
    /// parser.as_mut().reinit();
    /// assert_eq!(parser.as_mut().execute(b"HTTP/1.1 200"), Ok(Parsed::All));
    /// assert_eq!(parser.as_mut().finish(), Err(FinishError::InvalidEofState));
    /// assert_eq!(parser.as_ref().reason(), Some(c"Invalid EOF state"));
    /// ```
    ///
    /// # Errors
    ///
    /// [`FinishError::InvalidEofState`] when the input ended inside a message, with the
    /// reason `"Invalid EOF state"`, and [`FinishError::Callback`] when `on_message_complete`
    /// returned a value other than `0`. The parser's error is left as it was.
    ///
    /// # Panics
    ///
    /// When the parser's `finish` state is no `llhttp_finish`, as only
    /// [`set_finish_raw`](Self::set_finish_raw) stores, where C would abort.
    ///
    /// Replaces `fn::llhttp_finish`
    #[doc(alias = "llhttp_finish")]
    pub fn finish(&mut self) -> Result<(), FinishError> {
        // We're in an error state. Don't bother doing anything.
        if self.as_ref().error_raw() != 0 {
            return Ok(());
        }
        match u32::from(self.as_ref().finish_raw()) {
            sys::HTTP_FINISH_SAFE_WITH_CB => {
                match NonZero::new(self.callback_maybe(EventSlot::MessageComplete)) {
                    None => Ok(()),
                    Some(code) => Err(FinishError::Callback(code)),
                }
            }
            sys::HTTP_FINISH_SAFE => Ok(()),
            sys::HTTP_FINISH_UNSAFE => {
                self.set_reason(Some(c"Invalid EOF state"));
                Err(FinishError::InvalidEofState)
            }
            _ => panic!("the llhttp parser's finish state is no `llhttp_finish`"),
        }
    }
}

/// Why [`ParserMut::finish`] refused the end of the input, as `llhttp_finish` reports it: an
/// `llhttp_errno_t` other than `HPE_OK`, which the parser does not keep as its error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FinishError {
    /// `HPE_INVALID_EOF_STATE`: the input ended inside a message whose end must be read,
    /// such as one whose `Content-Length` it had not reached.
    InvalidEofState,
    /// The value `on_message_complete` returned, ending the message the input's end ended:
    /// `-1` for an error, `HPE_PAUSED` for a pause, or any other.
    Callback(NonZero<c_int>),
}

impl FinishError {
    /// The `llhttp_errno_t` `llhttp_finish` returns, as an `int`: `HPE_INVALID_EOF_STATE`, or
    /// the callback's value.
    #[must_use]
    pub const fn code(self) -> c_int {
        match self {
            Self::InvalidEofState => Errno::InvalidEofState.raw().cast_signed(),
            Self::Callback(code) => code.get(),
        }
    }

    /// The error as llhttp's enum.
    ///
    /// # Errors
    ///
    /// [`UnnamedErrno`] when the code is no `llhttp_errno`: a callback's own value, its bits
    /// as C's conversion to the unsigned enum gives them.
    pub fn errno(self) -> Result<Errno, UnnamedErrno> {
        Errno::try_from(self.code().cast_unsigned())
    }
}

impl fmt::Display for FinishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEofState => {
                f.write_str("the llhttp parser's input ended inside a message")
            }
            Self::Callback(code) => {
                f.write_str("llhttp's on_message_complete refused the end of the input: ")?;
                fmt::Display::fmt(code, f)
            }
        }
    }
}

impl std::error::Error for FinishError {}

// --- The dispatchers -------------------------------------------------------------------

/// The NUL-terminated `text`, as a C string, at compile time.
const fn c_str(text: &'static str) -> &'static CStr {
    match CStr::from_bytes_with_nul(text.as_bytes()) {
        Ok(text) => text,
        Err(_) => panic!("a dispatcher's reason holds a NUL or lacks its terminator"),
    }
}

/// The two kinds of dispatcher, their slots and their methods.
macro_rules! dispatchers {
    (
        spans {
            $(
                $(#[$span_anchor:meta])*
                $span_export:ident, $span:ident, $span_slot:ident: $span_what:literal;
            )*
        }
        events {
            $(
                $(#[$event_anchor:meta])*
                $event_export:ident, $event:ident, $event_slot:ident: $event_what:literal;
            )*
        }
    ) => {
        /// A slot of a parser's table that holds a data callback, one per span dispatcher.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum SpanSlot {
            $(
                #[doc = concat!("`", stringify!($span), "`.")]
                $span_slot,
            )*
        }

        impl SpanSlot {
            /// The reason the slot's dispatcher gives the parser when its callback returns
            /// `-1`: `SPAN_CALLBACK_MAYBE`'s `"Span callback error in " #NAME`.
            const fn error_reason(self) -> &'static CStr {
                match self {
                    $(
                        Self::$span_slot => const {
                            c_str(concat!("Span callback error in ", stringify!($span), "\0"))
                        },
                    )*
                }
            }
        }

        /// A slot of a parser's table that holds an event callback, one per event dispatcher.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum EventSlot {
            $(
                #[doc = concat!("`", stringify!($event), "`.")]
                $event_slot,
            )*
        }

        impl ParserMut<'_> {
            $(
                #[doc = concat!("Signals ", $span_what, ", `span`, to the parser's table: calls \
                                 its `", stringify!($span), "` with the parser and `span`, and \
                                 returns what it returns, `-1` as `HPE_USER` with the reason \
                                 `\"Span callback error in ", stringify!($span), "\"`; `0` \
                                 when the parser has no table or the slot is NULL.")]
                ///
                /// llhttp's generated parser calls it with each run of the part it reads,
                /// from where the part starts in the input, or the input's start, to where it
                /// ends, or the input's end, and stops its parse with any value other than
                /// `0`, pausing it at `HPE_PAUSED`.
                ///
                $(#[$span_anchor])*
                pub fn $span(&mut self, span: &[u8]) -> c_int {
                    self.span_callback_maybe(SpanSlot::$span_slot, span)
                }
            )*
            $(
                #[doc = concat!("Signals ", $event_what, " to the parser's table: calls its `",
                                stringify!($event), "` with the parser, and returns what it \
                                returns; `0` when the parser has no table or the slot is NULL.")]
                ///
                /// llhttp's generated parser calls it as the event happens, and stops its
                /// parse with any value other than `0`, pausing it at `HPE_PAUSED`, but where
                /// the slot's own documentation gives the value another meaning.
                ///
                $(#[$event_anchor])*
                pub fn $event(&mut self) -> c_int {
                    self.callback_maybe(EventSlot::$event_slot)
                }
            )*
        }
    };
}

with_dispatchers!(dispatchers);

impl ParserMut<'_> {
    /// Calls the callback the parser's table holds in `slot` with the parser, and returns
    /// what it returns; `0` when the parser has no table or the slot is NULL.
    ///
    /// Replaces `mac::CALLBACK_MAYBE`
    #[doc(alias = "CALLBACK_MAYBE")]
    fn callback_maybe(&mut self, slot: EventSlot) -> c_int {
        self.call_event(slot).unwrap_or(0)
    }

    /// Calls the callback the parser's table holds in `slot` with the parser and `span`, and
    /// returns what it returns, `-1` as `HPE_USER` with the slot's reason; `0` when the
    /// parser has no table or the slot is NULL.
    ///
    /// Replaces `mac::SPAN_CALLBACK_MAYBE`
    #[doc(alias = "SPAN_CALLBACK_MAYBE")]
    fn span_callback_maybe(&mut self, slot: SpanSlot, span: &[u8]) -> c_int {
        match self.call_span(slot, span) {
            None => 0,
            Some(-1) => {
                self.set_reason(Some(slot.error_reason()));
                Errno::User.raw().cast_signed()
            }
            Some(err) => err,
        }
    }
}

#[cfg(test)]
mod unit_priv {
    use ffibox::{CRefSeam, CZeroable};

    use super::{SpanSlot, c_str, lenient};
    use crate::libgit2::deps::llhttp::llhttp::{Parser, ParserType};
    use crate::sys::libgit2 as sys;

    #[test]
    fn the_trace_line_is_llhttp_debugs() {
        let mut parser = Parser::zeroed();
        parser.as_mut().set_flags_raw(0x5);
        parser.as_mut().set_parser_type(ParserType::Response);
        let p = parser.as_ref().as_ptr();
        assert_eq!(
            parser.as_ref().debug_line(b"", "span_start"),
            format!("p={p:p} type=2 flags=05 next=null debug=span_start\n")
        );
        assert_eq!(
            parser.as_ref().debug_line(b"HTTP", "on_message_begin"),
            format!("p={p:p} type=2 flags=05 next=48   debug=on_message_begin\n")
        );
        parser.as_mut().set_flags_raw(0x200);
        assert_eq!(
            parser.as_ref().debug_line(b"\n", "x"),
            format!("p={p:p} type=2 flags=200 next=0a   debug=x\n")
        );
        // As glibc's `%p` prints a pointer, in lowercase hexadecimal after `0x`.
        assert!(format!("{p:p}").starts_with("0x"));
    }

    #[test]
    fn a_high_byte_is_traced_as_two_digits_not_sign_extended() {
        let parser = Parser::zeroed();
        let line = parser.as_ref().debug_line(b"\xFF\x80", "m");
        assert!(line.ends_with(" next=ff   debug=m\n"), "{line}");
        assert!(!line.contains("ffffff"));
    }

    #[test]
    fn each_lenient_flag_fits_lenient_flags() {
        for (flag, bit) in [
            (sys::LENIENT_HEADERS, 0x1),
            (sys::LENIENT_VERSION, 0x10),
            (sys::LENIENT_SPACES_AFTER_CHUNK_SIZE, 0x200),
        ] {
            assert_eq!(lenient(flag), bit);
        }
    }

    #[test]
    #[should_panic(expected = "wider than `lenient_flags`")]
    fn a_flag_wider_than_lenient_flags_is_refused() {
        let _ = lenient(0x1_0000);
    }

    #[test]
    fn each_span_slots_reason_names_its_slot() {
        assert_eq!(
            c_str("Span callback error in on_url\0"),
            c"Span callback error in on_url"
        );
        assert_eq!(
            SpanSlot::Url.error_reason(),
            c"Span callback error in on_url"
        );
        assert_eq!(
            SpanSlot::ChunkExtensionValue.error_reason(),
            c"Span callback error in on_chunk_extension_value"
        );
    }

    #[test]
    #[should_panic(expected = "holds a NUL or lacks its terminator")]
    fn a_reason_without_its_terminator_is_refused() {
        let _ = c_str("on_url");
    }
}

#[expect(
    unsafe_code,
    reason = "reads and writes llhttp_settings_s's slots through its handles' pointers, asserts \
              the table's `Send` and `Sync`, borrows C's `stderr`, and reads and calls the slots \
              of the table a parser holds, restoring its context and type"
)]
mod ffi_import {
    use core::ffi::c_int;
    use std::io;

    use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};
    use glibc_bindings::libc::libio::stdio::stderr;

    use super::{EventSlot, SpanSlot};
    use crate::libgit2::deps::llhttp::llhttp::{Callback, DataCallback, ParserMut};
    use crate::sys::libgit2 as sys;

    /// The slot of `table` that each [`SpanSlot`] and [`EventSlot`] names.
    macro_rules! slot_reads {
        (
            spans {
                $(
                    $(#[$span_anchor:meta])*
                    $span_export:ident, $span:ident, $span_slot:ident: $span_what:literal;
                )*
            }
            events {
                $(
                    $(#[$event_anchor:meta])*
                    $event_export:ident, $event:ident, $event_slot:ident: $event_what:literal;
                )*
            }
        ) => {
            impl SpanSlot {
                /// What `table` holds in this slot.
                fn read(self, table: &sys::llhttp_settings_s) -> sys::llhttp_data_cb {
                    match self {
                        $(Self::$span_slot => table.$span,)*
                    }
                }
            }

            impl EventSlot {
                /// What `table` holds in this slot.
                fn read(self, table: &sys::llhttp_settings_s) -> sys::llhttp_cb {
                    match self {
                        $(Self::$event_slot => table.$event,)*
                    }
                }
            }
        };
    }

    with_dispatchers!(slot_reads);

    impl ParserMut<'_> {
        /// The parser's table, the one its `settings` addresses; `None` when it is NULL.
        fn table(&self) -> Option<&sys::llhttp_settings_s> {
            let table = self.as_ref().settings_ptr();
            // SAFETY: a non-NULL `settings` addresses an `llhttp_settings_t` that stays live,
            // and that nothing writes, for as long as the parser holds it: the contract of
            // `set_settings_ptr`, which `set_settings` meets with a `SettingsRef<'static>`, and
            // which a handle the gateway builds over C's parser meets for its call alone, its
            // lifetime, C's caller keeping its table for the call (`parser_mut`). llhttp never
            // writes a table. The reference lives for this handle's borrow, within either.
            unsafe { table.as_ref() }
        }

        /// Calls the callback the parser's table holds in `slot` with the parser, and returns
        /// what it returns; `None` when the parser has no table or the slot is NULL.
        pub(super) fn call_event(&mut self, slot: EventSlot) -> Option<c_int> {
            let callback = slot.read(self.table()?)?;
            // SAFETY: `callback` is a callback of the table this parser holds, read from it
            // just now, which is sound to call with this parser and the `data` it holds, at any
            // time and in any order while it holds the table (`set_settings_ptr`'s and
            // `set_data`'s contracts; a `Callback` of a table `set_settings` stored is sound
            // with any parser). The pointer is the handle's, exclusive for the call, as llhttp's
            // own calls hand it.
            Some(unsafe { callback(self.as_mut_ptr()) })
        }

        /// Calls the callback the parser's table holds in `slot` with the parser and `span`,
        /// and returns what it returns; `None` when the parser has no table or the slot is
        /// NULL.
        pub(super) fn call_span(&mut self, slot: SpanSlot, span: &[u8]) -> Option<c_int> {
            let callback = slot.read(self.table()?)?;
            // SAFETY: as `call_event`'s, `callback` being a data callback of the table this
            // parser holds; `span` is a run of `span.len()` initialised bytes, live and
            // unchanged for the call, the run a data callback is handed.
            Some(unsafe { callback(self.as_mut_ptr(), span.as_ptr().cast(), span.len()) })
        }

        /// Stores `byte` in the parser's `type`, as C's `llhttp_init` stores the
        /// `llhttp_type_t` it is given and `llhttp_reset` the one it kept, whatever its value.
        pub(super) fn set_parser_type_byte(&mut self, byte: u8) {
            let raw = self.as_mut_ptr();
            // SAFETY: the handle addresses a live parser, which it alone accesses for its
            // borrow; `type` is a `uint8_t`, which every value is valid for, and llhttp.c only
            // switches on it, with a `default` for any value no `llhttp_type` names.
            unsafe { (*raw).type_ = byte };
        }

        /// Initialises the parser again, as [`reinit`](ParserMut::reinit) does, and stores back
        /// the `data` and `settings` it held, as `llhttp_reset` keeps them.
        ///
        /// # Panics
        ///
        /// As `reinit`, before it changes anything.
        pub(super) fn reinit_keeping_context(&mut self) {
            let data = self.as_ref().data();
            let settings = self.as_ref().settings_ptr();
            self.reinit();
            // SAFETY: `data` is what this parser held beside `settings` before `reinit`, so it
            // met `set_data`'s contract for this parser and these settings then, and still
            // does: the parser, its table and the context its callbacks expect are the same.
            unsafe { self.set_data(data) };
            // SAFETY: likewise, `settings` is the table this parser held beside `data`, which
            // met `set_settings_ptr`'s contract for it, and still does.
            unsafe { self.set_settings_ptr(settings) };
        }
    }

    /// Writes `text` to C's standard error stream, `stderr`, in one `fprintf` under the
    /// stream's lock, as `llhttp__debug`'s `fprintf(stderr, …)` does.
    ///
    /// # Errors
    ///
    /// Those of `FileRef::print`: a NUL byte in `text`, or the write's failure.
    pub(super) fn print_to_stderr(text: &str) -> io::Result<()> {
        // SAFETY: the stream is borrowed for this call alone, as C's `fprintf(stderr, …)`
        // borrows it, and asks of the program what that expression asks. No C of libgit2, of
        // its bundled dependencies or of its tests assigns `stderr`, closes it, reopens it, or
        // writes it but through stdio's locking routines (`fprintf`, `fputs`, `vfprintf`): none
        // calls `fclose`, `freopen` or `__fsetlocking` on it, nor an `_unlocked` routine, so
        // that no write of `stderr` races with the read, the stream stays open for the call,
        // and every other access to it holds its lock. Rust's `std::io::stderr` writes the
        // descriptor, not the stream.
        let stream = unsafe { stderr() };
        stream.print(text).map(drop)
    }

    define_ctype!(
        /// llhttp's table of callbacks, `struct llhttp_settings_s`: a slot per event of a
        /// message, each NULL or the callback api.c's dispatchers call with the parser.
        ///
        /// It owns nothing, so it is held by value, built empty by [`new`](Self::new), read
        /// through [`as_ref`](Self::as_ref) and written through [`as_mut`](Self::as_mut). A
        /// parser keeps its address, so the table a parser is given is lent for `'static`
        /// ([`ParserMut::set_settings`](crate::libgit2::deps::llhttp::llhttp::ParserMut::set_settings)),
        /// which neither moves nor changes while it is shared; the module documentation
        /// says what its slots may hold.
        ///
        /// Its slots are plain function pointers, read only through a shared borrow, so a
        /// table is `Send` and `Sync`, and a `static` may hold one:
        ///
        /// ```
        /// use std::ffi::c_int;
        /// use std::sync::LazyLock;
        ///
        /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
        ///     DataCallback, DataHandler, Parsed, Parser, ParserMut,
        /// };
        ///
        /// struct Refusing;
        ///
        /// impl DataHandler for Refusing {
        ///     fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
        ///         -1
        ///     }
        /// }
        ///
        /// static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
        ///     let mut settings = Settings::new();
        ///     settings
        ///         .as_mut()
        ///         .set_on_body(Some(DataCallback::new::<Refusing>()));
        ///     settings
        /// });
        ///
        /// let mut parser = Parser::new();
        /// parser.as_mut().set_settings(Some(SETTINGS.as_ref()));
        /// let head = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n";
        /// assert_eq!(parser.as_mut().parse(head), Ok(Parsed::All));
        /// // `on_body` refuses the body: `-1` becomes `HPE_USER`.
        /// assert!(parser.as_mut().parse(b"hi").is_err());
        /// ```
        ///
        /// A parser is given only a table that lives as long as the program, so a local one,
        /// which could be written or dropped while the parser keeps its address, is refused:
        ///
        /// ```compile_fail,E0597
        /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let settings = Settings::new();
        /// let mut parser = Parser::new();
        /// parser.as_mut().set_settings(Some(settings.as_ref()));
        /// ```
        ///
        /// Wraps `st::llhttp_settings_s`
        #[doc(alias = "llhttp_settings_s")]
        Settings,
        SettingsRef,
        SettingsMut,
        sys::llhttp_settings_s
    );

    // SAFETY: all-zero is the table `llhttp_settings_init` makes: each slot an
    // `Option<extern "C" fn>`, for which all-zero is `None`, NULL, which api.c's dispatchers
    // read as no callback and the getters as `None`. A table owns nothing, so no teardown is
    // paired with the value.
    unsafe impl CZeroable for Settings {}

    // SAFETY: a table is 23 function pointers and a zero-sized marker, owning nothing and
    // tied to no thread: each slot holds a `Callback` or a `DataCallback` (the handles'
    // invariant), which are plain function pointers callable on any thread, the parser's.
    // Moving the value to another thread moves only those pointers.
    unsafe impl Send for Settings {}

    // SAFETY: through `&Settings` nothing writes the table: `as_ref` hands out a
    // `SettingsRef`, whose getters only read, and the setters need `as_mut`'s `&mut`. C reads
    // a table it is given and never writes it (api.c's dispatchers and `llhttp_reset` read
    // `settings` as `const`). So threads sharing a table only read the same pointers, and
    // calling one from several threads at once is what `Callback` and `DataCallback` allow.
    unsafe impl Sync for Settings {}

    impl core::fmt::Debug for Settings {
        /// The slots that hold a callback, by their field names.
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            core::fmt::Debug::fmt(&self.as_ref(), f)
        }
    }

    // The handles' invariant, beside the one every handle has (it addresses a live table for
    // its borrow): each slot is NULL or a function that a `Callback` or `DataCallback` may
    // hold, that is, one their `from_fn`'s contract accepts. The setters store only those,
    // `Settings::zeroed()` and `Settings::new()` store NULL, and a handle built over a C table
    // with `from_ptr` vouches that the table meets it: a table whose callbacks read the
    // parser's `data` as their own context does not.

    /// Reads a slot through a shared handle's pointer.
    macro_rules! get {
        ($handle:expr, $field:ident) => {{
            let raw = $handle.as_ptr();
            // SAFETY: the handle addresses a live `llhttp_settings_s` for its borrow (its
            // invariant); the projection forms no reference and reads nothing.
            let field = unsafe { &raw const (*raw).$field };
            // SAFETY: `field` is in bounds of that struct, aligned and initialised, each slot
            // being an `Option` of a function pointer, and no exclusive handle writes it while
            // this shared one lives.
            unsafe { field.read() }
        }};
    }

    /// Writes a slot through an exclusive handle's pointer.
    macro_rules! set {
        ($handle:expr, $field:ident, $value:expr) => {{
            let value = $value;
            let raw = $handle.as_mut_ptr();
            // SAFETY: the handle addresses a live `llhttp_settings_s` for its borrow (its
            // invariant); the projection forms no reference and writes nothing.
            let field = unsafe { &raw mut (*raw).$field };
            // SAFETY: `field` is in bounds of that struct and aligned, and the exclusive
            // handle is the only access to it while it lives. The value is a function pointer
            // or NULL, which needs no drop of what it replaces.
            unsafe { field.write(value) }
        }};
    }

    /// The getter and the setter of each slot, the slot's anchor on both: an `event` slot
    /// holds a [`Callback`], a `data` slot a [`DataCallback`].
    macro_rules! slots {
        (
            $(
                $(#[$anchor:meta])*
                $kind:ident $field:ident, $setter:ident: $what:literal, $returns:literal;
            )*
        ) => {
            impl SettingsRef<'_> {
                $(
                    slots!(@get $kind $field, $what, $returns, [$(#[$anchor])*]);
                )*
            }

            impl SettingsMut<'_> {
                $(
                    slots!(@set $kind $field, $setter, $what, [$(#[$anchor])*]);
                )*
            }

            impl core::fmt::Debug for SettingsRef<'_> {
                /// The slots that hold a callback, by their field names.
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    let mut set = f.debug_set();
                    $(
                        if self.$field().is_some() {
                            set.entry(&stringify!($field));
                        }
                    )*
                    set.finish()
                }
            }
        };
        (@get event $field:ident, $what:literal, $returns:literal, [$(#[$anchor:meta])*]) => {
            #[doc = concat!("The callback for ", $what, ", `", stringify!($field), "`; `None` \
                             when the slot is NULL, which the parser reads as `0`.")]
            ///
            #[doc = concat!("It returns ", $returns, ".")]
            ///
            $(#[$anchor])*
            #[must_use]
            pub fn $field(&self) -> Option<Callback> {
                let f = get!(self, $field)?;
                // SAFETY: a non-NULL slot holds a function `Callback::from_fn` accepts (the
                // handles' invariant).
                Some(unsafe { Callback::from_fn(f) })
            }
        };
        (@get data $field:ident, $what:literal, $returns:literal, [$(#[$anchor:meta])*]) => {
            #[doc = concat!("The callback for ", $what, ", `", stringify!($field), "`, called \
                             with each run of it the parse reads; `None` when the slot is NULL, \
                             which the parser reads as `0`.")]
            ///
            #[doc = concat!("It returns ", $returns, ".")]
            ///
            $(#[$anchor])*
            #[must_use]
            pub fn $field(&self) -> Option<DataCallback> {
                let f = get!(self, $field)?;
                // SAFETY: a non-NULL slot holds a function `DataCallback::from_fn` accepts
                // (the handles' invariant).
                Some(unsafe { DataCallback::from_fn(f) })
            }
        };
        (@set event $field:ident, $setter:ident, $what:literal, [$(#[$anchor:meta])*]) => {
            #[doc = concat!("Sets the callback for ", $what, ", `", stringify!($field), "`, or \
                             clears the slot with `None`.")]
            ///
            $(#[$anchor])*
            pub fn $setter(&mut self, callback: Option<Callback>) {
                set!(self, $field, callback.and_then(Callback::raw));
            }
        };
        (@set data $field:ident, $setter:ident, $what:literal, [$(#[$anchor:meta])*]) => {
            #[doc = concat!("Sets the callback for ", $what, ", `", stringify!($field), "`, or \
                             clears the slot with `None`.")]
            ///
            $(#[$anchor])*
            pub fn $setter(&mut self, callback: Option<DataCallback>) {
                set!(self, $field, callback.and_then(DataCallback::raw));
            }
        };
    }

    slots! {
        /// Wraps `fld::llhttp_settings_s.on_message_begin`
        #[doc(alias = "llhttp_settings_s.on_message_begin")]
        event on_message_begin, set_on_message_begin:
            "the start of a message, before its first byte is read",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_url`
        #[doc(alias = "llhttp_settings_s.on_url")]
        data on_url, set_on_url:
            "a request's URL",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_status`
        #[doc(alias = "llhttp_settings_s.on_status")]
        data on_status, set_on_status:
            "a response's status text",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_method`
        #[doc(alias = "llhttp_settings_s.on_method")]
        data on_method, set_on_method:
            "a request's method",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_version`
        #[doc(alias = "llhttp_settings_s.on_version")]
        data on_version, set_on_version:
            "the message's HTTP version",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_header_field`
        #[doc(alias = "llhttp_settings_s.on_header_field")]
        data on_header_field, set_on_header_field:
            "a header's name",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_header_value`
        #[doc(alias = "llhttp_settings_s.on_header_value")]
        data on_header_value, set_on_header_value:
            "a header's value",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_chunk_extension_name`
        #[doc(alias = "llhttp_settings_s.on_chunk_extension_name")]
        data on_chunk_extension_name, set_on_chunk_extension_name:
            "a chunk extension's name",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_chunk_extension_value`
        #[doc(alias = "llhttp_settings_s.on_chunk_extension_value")]
        data on_chunk_extension_value, set_on_chunk_extension_value:
            "a chunk extension's value",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_headers_complete`
        #[doc(alias = "llhttp_settings_s.on_headers_complete")]
        event on_headers_complete, set_on_headers_complete:
            "the end of the message's headers, once the parser has read its method or status, \
             version, upgrade and flags",
            "`0` to go on, `1` to read no body and go on to the next message, `2` to read no \
             body and pause as at an upgrade, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_body`
        #[doc(alias = "llhttp_settings_s.on_body")]
        data on_body, set_on_body:
            "the message's body",
            "`0` to go on, `HPE_PAUSED` to pause, or any other value to stop the parse with it, \
             `-1` as `HPE_USER`";
        /// Wraps `fld::llhttp_settings_s.on_message_complete`
        #[doc(alias = "llhttp_settings_s.on_message_complete")]
        event on_message_complete, set_on_message_complete:
            "the end of a message, which `llhttp_finish` also signals at the end of the input \
             when the message ends there",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_url_complete`
        #[doc(alias = "llhttp_settings_s.on_url_complete")]
        event on_url_complete, set_on_url_complete:
            "the end of a request's URL",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_status_complete`
        #[doc(alias = "llhttp_settings_s.on_status_complete")]
        event on_status_complete, set_on_status_complete:
            "the end of a response's status text",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_method_complete`
        #[doc(alias = "llhttp_settings_s.on_method_complete")]
        event on_method_complete, set_on_method_complete:
            "the end of a request's method",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_version_complete`
        #[doc(alias = "llhttp_settings_s.on_version_complete")]
        event on_version_complete, set_on_version_complete:
            "the end of the message's HTTP version",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_header_field_complete`
        #[doc(alias = "llhttp_settings_s.on_header_field_complete")]
        event on_header_field_complete, set_on_header_field_complete:
            "the end of a header's name",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_header_value_complete`
        #[doc(alias = "llhttp_settings_s.on_header_value_complete")]
        event on_header_value_complete, set_on_header_value_complete:
            "the end of a header's value",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_chunk_extension_name_complete`
        #[doc(alias = "llhttp_settings_s.on_chunk_extension_name_complete")]
        event on_chunk_extension_name_complete, set_on_chunk_extension_name_complete:
            "the end of a chunk extension's name",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_chunk_extension_value_complete`
        #[doc(alias = "llhttp_settings_s.on_chunk_extension_value_complete")]
        event on_chunk_extension_value_complete, set_on_chunk_extension_value_complete:
            "the end of a chunk extension's value",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_chunk_header`
        #[doc(alias = "llhttp_settings_s.on_chunk_header")]
        event on_chunk_header, set_on_chunk_header:
            "a chunk's header, once the parser has read its size into `content_length`",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_chunk_complete`
        #[doc(alias = "llhttp_settings_s.on_chunk_complete")]
        event on_chunk_complete, set_on_chunk_complete:
            "the end of a chunk",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
        /// Wraps `fld::llhttp_settings_s.on_reset`
        #[doc(alias = "llhttp_settings_s.on_reset")]
        event on_reset, set_on_reset:
            "the reset of the parser between two messages of one input, before the second \
             begins",
            "`0` to go on, `-1` for an error or `HPE_PAUSED` to pause";
    }
}

#[cfg(feature = "ffi-export-deps-llhttp-api-c")]
#[expect(
    unsafe_code,
    reason = "exports api.c's functions the remaining C calls, adopting the parser and the input \
              C passes"
)]
mod ffi_export {
    use core::ffi::{CStr, c_char, c_int, c_uchar};
    use core::slice;

    use ffibox::callback::barrier;
    use ffibox::{CMutSeam, CRefSeam};

    use super::{Errno, FinishError, Parsed, ParserMut, ParserRef};
    use crate::sys::libgit2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::llhttp_execute, llhttp_execute];
        let _ = [sys::llhttp_finish, llhttp_finish];
        let _ = [sys::llhttp_get_error_pos, llhttp_get_error_pos];
        let _ = [sys::llhttp_get_error_reason, llhttp_get_error_reason];
        let _ = [sys::llhttp_init, llhttp_init];
        let _ = [sys::llhttp_pause, llhttp_pause];
        let _ = [sys::llhttp_resume, llhttp_resume];
        let _ = [
            sys::llhttp_resume_after_upgrade,
            llhttp_resume_after_upgrade,
        ];
    };

    /// A dispatcher's type as `llhttp.c` declares the 23 it calls, `int
    /// llhttp__on_<slot>(llhttp__internal_t* s, const unsigned char* p, const unsigned char*
    /// endp)`: no header declares them, so they have no raw binding, and each export is
    /// checked against this type instead. api.c defines them over `llhttp_t` and `const char*`,
    /// the same types to the ABI.
    type Dispatcher =
        unsafe extern "C" fn(*mut sys::llhttp__internal_t, *const c_uchar, *const c_uchar) -> c_int;

    /// The `int` a dispatcher or `llhttp_finish` returns for a panic of the Rust behind it:
    /// `HPE_INTERNAL`, which stops the parse.
    const INTERNAL: c_int = Errno::Internal.raw().cast_signed();

    /// The run of bytes from `p` to `endp`: empty when they are equal, both NULL included.
    ///
    /// # Safety
    ///
    /// `p` and `endp` are equal, or `p` is at or before `endp` in one allocation and the bytes
    /// between them are initialised, and live and unchanged for `'a`: a span llhttp.c passes,
    /// from its start in the input to where it ended or the input's end.
    unsafe fn span<'a>(p: *const c_uchar, endp: *const c_uchar) -> &'a [u8] {
        if p == endp {
            return &[];
        }
        // SAFETY: the caller guarantees that `p` is at or before `endp` in one allocation.
        let len = unsafe { endp.offset_from_unsigned(p) };
        // SAFETY: the caller guarantees that the `len` bytes from `p` are initialised, live and
        // unchanged for `'a`; `u8` is aligned to 1.
        unsafe { slice::from_raw_parts(p, len) }
    }

    /// `llhttp_init`: initialises the parser at `parser` as [`ParserMut::init`] does, with the
    /// table C passes stored as its address and the type as its low byte, as C stores them.
    /// It cannot fail, so a panic, which `reinit` raises for a callback of the parser's own
    /// parse, aborts, as `llhttp_init` there would corrupt the parse.
    ///
    /// # Safety
    ///
    /// `parser` points at storage for an `llhttp_t`, initialised or not, which nothing else
    /// accesses during the call; `settings` is NULL or meets
    /// [`ParserMut::set_settings_ptr`]'s contract for the parser and the `data` C stores in it
    /// before it executes it.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_init(
        parser: *mut sys::llhttp_t,
        type_: sys::llhttp_type_t,
        settings: *const sys::llhttp_settings_t,
    ) {
        // SAFETY: the caller guarantees that `parser` is storage for a parser nothing else
        // accesses during the call. The handle reads none of it before `reinit` initialises
        // it: `reinit` only compares its address with the parses this thread runs.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(
            || {
                parser.reinit();
                parser.set_parser_type_byte(type_.to_le_bytes()[0]);
                // SAFETY: the caller guarantees `set_settings_ptr`'s contract for `settings`.
                unsafe { parser.set_settings_ptr(settings) };
            },
            || std::process::abort(),
        );
    }

    /// `llhttp_finish`: [`ParserMut::finish`], its outcome returned as C's `llhttp_errno_t`:
    /// `HPE_OK`, `HPE_INVALID_EOF_STATE` or `on_message_complete`'s value. A panic, which
    /// `finish` raises where C would abort, is reported as `HPE_INTERNAL`.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_finish(parser: *mut sys::llhttp_t) -> sys::llhttp_errno_t {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(
            || parser.finish().map_or_else(FinishError::code, |()| 0),
            || INTERNAL,
        )
        .cast_unsigned()
    }

    /// The 23 dispatchers' exports, each over its method.
    macro_rules! dispatcher_exports {
        (
            spans {
                $(
                    $(#[$span_anchor:meta])*
                    $span_export:ident, $span:ident, $span_slot:ident: $span_what:literal;
                )*
            }
            events {
                $(
                    $(#[$event_anchor:meta])*
                    $event_export:ident, $event:ident, $event_slot:ident: $event_what:literal;
                )*
            }
        ) => {
            const _: () = {
                $(let _: Dispatcher = $span_export;)*
                $(let _: Dispatcher = $event_export;)*
            };

            $(
                #[doc = concat!("`", stringify!($span_export), "`: [`ParserMut::",
                                stringify!($span), "`] over the run from `p` to `endp`. A \
                                panic is reported as `HPE_INTERNAL`.")]
                ///
                /// # Safety
                ///
                /// `s` as `parser_mut` requires, and `p` and `endp` as `span` requires.
                #[unsafe(no_mangle)]
                unsafe extern "C" fn $span_export(
                    s: *mut sys::llhttp__internal_t,
                    p: *const c_uchar,
                    endp: *const c_uchar,
                ) -> c_int {
                    // SAFETY: the caller guarantees `span`'s contract for the call.
                    let span = unsafe { span(p, endp) };
                    // SAFETY: the caller guarantees `parser_mut`'s contract.
                    let mut parser = unsafe { parser_mut(s) };
                    barrier(|| parser.$span(span), || INTERNAL)
                }
            )*

            $(
                #[doc = concat!("`", stringify!($event_export), "`: [`ParserMut::",
                                stringify!($event), "`]; `p` and `endp`, where the parse is in \
                                its input, are not read. A panic is reported as \
                                `HPE_INTERNAL`.")]
                ///
                /// # Safety
                ///
                /// `s` as `parser_mut` requires.
                #[unsafe(no_mangle)]
                unsafe extern "C" fn $event_export(
                    s: *mut sys::llhttp__internal_t,
                    _p: *const c_uchar,
                    _endp: *const c_uchar,
                ) -> c_int {
                    // SAFETY: the caller guarantees `parser_mut`'s contract.
                    let mut parser = unsafe { parser_mut(s) };
                    barrier(|| parser.$event(), || INTERNAL)
                }
            )*
        };
    }

    with_dispatchers!(dispatcher_exports);

    /// The exclusive handle of the parser C passes.
    ///
    /// # Safety
    ///
    /// `parser` points at a live `llhttp_t` C initialised, with `llhttp_init` or after
    /// `llhttp__internal_init`, which nothing else accesses while the handle lives but the
    /// callbacks its parse hands it to, and whose `settings` and `data` are what its callbacks
    /// expect: the contract of api.c's functions.
    unsafe fn parser_mut<'a>(parser: *mut sys::llhttp_t) -> ParserMut<'a> {
        // SAFETY: the caller guarantees that `parser` is a live parser, exclusively held for
        // the handle's lifetime, which is the export's call: each export makes its one
        // operation with the handle and drops it before returning. Its `reason` is NULL or one
        // of llhttp's literals, which live as long as the program, since the remaining C sets
        // it only through llhttp. Its `settings` and `data` meet `set_settings_ptr`'s and
        // `set_data`'s contracts for that call, which is all C promises: a C caller keeps its
        // table, and the context its callbacks read through `data`, for its own calls into
        // llhttp, and no longer. libgit2's HTTP client leaves its table installed with `data`
        // NULL between calls (`transports/httpclient.c`), where its callbacks would follow
        // NULL, so no handle over a C parser may outlive the call that builds it.
        unsafe { ParserMut::from_ptr(parser) }.expect("api.c's functions take a parser")
    }

    /// `llhttp_execute`: [`ParserMut::execute`] over the `len` bytes at `data`, its outcome
    /// returned as C's `llhttp_errno_t`: `HPE_OK`, `HPE_PAUSED` or `HPE_PAUSED_UPGRADE` for the
    /// run's [`Parsed`], or the error the parser stopped at. A panic, which `execute` raises
    /// where C would abort or corrupt the parser, is reported as `HPE_INTERNAL`.
    ///
    /// The run is the `len` bytes at `data` themselves, an empty one included: the parser
    /// restarts a span the previous run left open at `data` and keeps `data + len` as its
    /// error position when that span's callback refuses it, as C's `llhttp_execute` does, so
    /// that `llhttp_get_error_pos` stays relative to `data`. With `len` 0, `data` may be NULL,
    /// which C's `data + len` does not allow: the run is then empty at no address of C's.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires, and `data` valid for reads of `len` bytes, which
    /// nothing writes during the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_execute(
        parser: *mut sys::llhttp_t,
        data: *const c_char,
        len: usize,
    ) -> sys::llhttp_errno_t {
        let bytes: &[u8] = if data.is_null() {
            &[]
        } else {
            // SAFETY: the caller guarantees that `data` is valid for reads of `len` bytes,
            // which nothing writes during the call, and it is not NULL, so it is valid for a
            // run of 0 bytes too; `u8` and `c_char` have one size and alignment.
            unsafe { slice::from_raw_parts(data.cast::<u8>(), len) }
        };
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(
            || match parser.execute(bytes) {
                Ok(Parsed::All) => Errno::Ok.raw(),
                Ok(Parsed::Paused) => Errno::Paused.raw(),
                Ok(Parsed::PausedUpgrade) => Errno::PausedUpgrade.raw(),
                Err(error) => error.code().cast_unsigned(),
            },
            || Errno::Internal.raw(),
        )
    }

    /// `llhttp_get_error_pos`: the parser's `error_pos` as C stored it, a pointer into the
    /// input of the `llhttp_execute` call that stopped, which C compares with that input.
    /// Rust never follows it ([`ParserRef::error_offset`] locates it in a slice instead), so
    /// the address is handed back unread.
    ///
    /// # Safety
    ///
    /// `parser` points at a live `llhttp_t`, which nothing writes during the call.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_get_error_pos(parser: *const sys::llhttp_t) -> *const c_char {
        // SAFETY: the caller guarantees that `parser` is a live parser nothing writes during
        // the call; reading its `error_pos` field follows no pointer it holds.
        unsafe { (*parser).error_pos }
    }

    /// `llhttp_get_error_reason`: [`ParserRef::reason`], NULL when it is `None`.
    ///
    /// # Safety
    ///
    /// `parser` points at a live `llhttp_t`, which nothing writes during the call, and whose
    /// `reason` is NULL or a string that outlives the parser, as llhttp's literals do.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_get_error_reason(parser: *const sys::llhttp_t) -> *const c_char {
        // SAFETY: the caller guarantees that `parser` is a live parser nothing writes while
        // the handle lives, the call, and whose `reason` outlives it: the shared handle's
        // invariant.
        let parser = unsafe { ParserRef::from_ptr(parser.cast_mut()) };
        // `reason` cannot panic, so there is no unwind to stop.
        parser
            .and_then(|p| p.reason())
            .map_or(core::ptr::null(), CStr::as_ptr)
    }

    /// `llhttp_pause`: [`ParserMut::pause`]. It cannot fail, so a panic aborts.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_pause(parser: *mut sys::llhttp_t) {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(|| parser.pause(), || std::process::abort());
    }

    /// `llhttp_resume`: [`ParserMut::resume`]. It cannot fail, so a panic aborts.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_resume(parser: *mut sys::llhttp_t) {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(|| parser.resume(), || std::process::abort());
    }

    /// `llhttp_resume_after_upgrade`: [`ParserMut::resume_after_upgrade`]. It cannot fail,
    /// so a panic aborts.
    ///
    /// # Safety
    ///
    /// `parser` as `parser_mut` requires.
    #[unsafe(no_mangle)]
    unsafe extern "C" fn llhttp_resume_after_upgrade(parser: *mut sys::llhttp_t) {
        // SAFETY: the caller guarantees `parser_mut`'s contract.
        let mut parser = unsafe { parser_mut(parser) };
        barrier(|| parser.resume_after_upgrade(), || std::process::abort());
    }

    /// The exports called as C calls them, over parsers and tables Rust builds: what each
    /// hands its method and what it returns to C. They compile with the gateway, under the
    /// export feature, and run with it linked in place of api.c's C.
    #[cfg(test)]
    mod unit_priv {
        use core::ffi::c_int;
        use core::mem::MaybeUninit;
        use std::cell::RefCell;
        use std::sync::LazyLock;

        use ffibox::{CMutSeam, CRefSeam, CZeroable};

        use super::{
            llhttp__on_body, llhttp__on_message_begin, llhttp__on_url, llhttp_execute,
            llhttp_finish, llhttp_get_error_pos, llhttp_init,
        };
        use crate::libgit2::deps::llhttp::api::Settings;
        use crate::libgit2::deps::llhttp::llhttp::{
            Callback, DataCallback, DataHandler, Errno, EventHandler, Parser, ParserMut,
        };
        use crate::sys::libgit2 as sys;

        thread_local! {
            /// The runs `Url` was handed and the events `Begin` saw, in order.
            static LOG: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
        }

        /// Logs its run and returns `0`.
        struct Url;

        impl DataHandler for Url {
            fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
                LOG.with_borrow_mut(|log| log.push(data.to_vec()));
                0
            }
        }

        /// Refuses its run with `-1`.
        struct Refusing;

        impl DataHandler for Refusing {
            fn on_data(_: ParserMut<'_>, _: &[u8]) -> c_int {
                -1
            }
        }

        /// Logs an empty entry and returns `7`.
        struct Begin;

        impl EventHandler for Begin {
            fn on_event(_: ParserMut<'_>) -> c_int {
                LOG.with_borrow_mut(|log| log.push(Vec::new()));
                7
            }
        }

        static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
            let mut settings = Settings::new();
            let mut m = settings.as_mut();
            m.set_on_url(Some(DataCallback::new::<Url>()));
            m.set_on_body(Some(DataCallback::new::<Refusing>()));
            m.set_on_message_begin(Some(Callback::new::<Begin>()));
            m.set_on_message_complete(Some(Callback::new::<Begin>()));
            settings
        });

        /// A zeroed parser holding `SETTINGS`: the dispatchers and `llhttp_finish` read only
        /// its table, error and finish state, so it needs no C.
        fn parser() -> Parser {
            let mut parser = Parser::zeroed();
            parser.as_mut().set_settings(Some(SETTINGS.as_ref()));
            parser
        }

        #[test]
        fn a_span_export_hands_its_method_the_run_from_p_to_endp() {
            let mut parser = parser();
            let input = Box::<[u8]>::from(*b"/index.html");
            let raw = parser.as_mut().as_mut_ptr();
            let run = input.as_ptr_range();
            // SAFETY: the parser is live and exclusively ours; the run is `input`.
            let ret = unsafe { llhttp__on_url(raw, run.start, run.end) };
            assert_eq!(ret, 0);
            // The last byte alone, ending at the allocation's end.
            let run = input[10..].as_ptr_range();
            // SAFETY: as above.
            let ret = unsafe { llhttp__on_url(raw, run.start, run.end) };
            assert_eq!(ret, 0);
            assert_eq!(LOG.take(), [b"/index.html".to_vec(), b"l".to_vec()]);
        }

        #[test]
        fn an_empty_span_reads_nothing_null_or_dangling() {
            let mut parser = parser();
            let dangling = core::ptr::NonNull::<u8>::dangling().as_ptr().cast_const();
            for p in [core::ptr::null(), dangling] {
                // SAFETY: the parser is live and exclusively ours; the run is empty.
                let ret = unsafe { llhttp__on_url(parser.as_mut().as_mut_ptr(), p, p) };
                assert_eq!(ret, 0);
            }
            assert_eq!(LOG.take(), [Vec::new(), Vec::new()]);
        }

        #[test]
        fn a_span_exports_minus_one_is_hpe_user_with_its_reason() {
            let mut parser = parser();
            let body = *b"x";
            let run = body.as_ptr_range();
            let raw = parser.as_mut().as_mut_ptr();
            // SAFETY: the parser is live and exclusively ours; the run is `body`.
            let ret = unsafe { llhttp__on_body(raw, run.start, run.end) };
            assert_eq!(ret, Errno::User.raw().cast_signed());
            assert_eq!(
                parser.as_ref().reason(),
                Some(c"Span callback error in on_body")
            );
        }

        /// Records its run, or `event` for an event, and returns `0`.
        struct Recording;

        impl DataHandler for Recording {
            fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
                LOG.with_borrow_mut(|log| log.push(data.to_vec()));
                0
            }
        }

        impl EventHandler for Recording {
            fn on_event(_: ParserMut<'_>) -> c_int {
                LOG.with_borrow_mut(|log| log.push(b"event".to_vec()));
                0
            }
        }

        /// Each of the 23 exports, which `llhttp.c` calls by name, calls its own slot: with a
        /// table that holds that slot alone, it calls it, and with one that holds every other,
        /// none. Carries over what `equiv`'s `the_dispatchers_call_the_parsers_table_as_api_cs_do`
        /// established of the exports, which retires with api.c's C.
        #[test]
        fn each_dispatcher_export_calls_its_own_slot() {
            macro_rules! each_export {
                (
                    spans {
                        $(
                            $(#[$span_anchor:meta])*
                            $span_export:ident, $span:ident, $span_slot:ident: $span_what:literal;
                        )*
                    }
                    events {
                        $(
                            $(#[$event_anchor:meta])*
                            $event_export:ident, $event:ident, $event_slot:ident: $event_what:literal;
                        )*
                    }
                ) => {
                    let mut all = sys::llhttp_settings_s::default();
                    $(all.$span = DataCallback::new::<Recording>().raw();)*
                    $(all.$event = Callback::new::<Recording>().raw();)*
                    $(
                        let run = stringify!($span).as_bytes();
                        let mut only = sys::llhttp_settings_s::default();
                        only.$span = all.$span;
                        let mut others = all;
                        others.$span = None;
                        for (table, called) in [(&only, vec![run.to_vec()]), (&others, Vec::new())] {
                            let mut parser = Parser::zeroed();
                            // SAFETY: `table` outlives the parser, and its slots are Rust handlers,
                            // sound with any parser whatever its `data`.
                            unsafe { parser.as_mut().set_settings_ptr(table) };
                            let range = run.as_ptr_range();
                            // SAFETY: the parser is live and exclusively ours; the run is `run`.
                            let ret = unsafe {
                                super::$span_export(parser.as_mut().as_mut_ptr(), range.start, range.end)
                            };
                            assert_eq!((ret, LOG.take()), (0, called), stringify!($span_export));
                        }
                    )*
                    $(
                        let mut only = sys::llhttp_settings_s::default();
                        only.$event = all.$event;
                        let mut others = all;
                        others.$event = None;
                        for (table, called) in [(&only, vec![b"event".to_vec()]), (&others, Vec::new())] {
                            let mut parser = Parser::zeroed();
                            // SAFETY: as above.
                            unsafe { parser.as_mut().set_settings_ptr(table) };
                            let null = core::ptr::null();
                            // SAFETY: the parser is live and exclusively ours; `p` and `endp` are
                            // not read.
                            let ret = unsafe {
                                super::$event_export(parser.as_mut().as_mut_ptr(), null, null)
                            };
                            assert_eq!((ret, LOG.take()), (0, called), stringify!($event_export));
                        }
                    )*
                };
            }
            with_dispatchers!(each_export);
        }

        #[test]
        fn an_event_export_reads_neither_p_nor_endp() {
            let mut parser = parser();
            let dangling = core::ptr::NonNull::<u8>::dangling().as_ptr().cast_const();
            // SAFETY: the parser is live and exclusively ours; `p` and `endp` are not read.
            let ret = unsafe {
                llhttp__on_message_begin(parser.as_mut().as_mut_ptr(), dangling, core::ptr::null())
            };
            assert_eq!(ret, 7);
            assert_eq!(LOG.take(), [Vec::new()]);
            let mut bare = Parser::zeroed();
            // SAFETY: as above; a parser with no table returns `0`.
            let ret =
                unsafe { llhttp__on_message_begin(bare.as_mut().as_mut_ptr(), dangling, dangling) };
            assert_eq!(ret, 0);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn llhttp_init_initialises_storage_and_stores_what_c_passes() {
            let mut storage = MaybeUninit::<sys::llhttp_t>::uninit();
            // SAFETY: filling the storage with garbage, as C's would hold, writes only bytes.
            unsafe {
                storage
                    .as_mut_ptr()
                    .cast::<u8>()
                    .write_bytes(0xA5, size_of::<sys::llhttp_t>());
            }
            let table = SETTINGS.as_ref().as_ptr();
            // SAFETY: `storage` is a parser's storage, ours alone; `table` is a `'static` table
            // of callbacks sound with any parser.
            unsafe { llhttp_init(storage.as_mut_ptr(), sys::HTTP_RESPONSE, table) };
            // SAFETY: `llhttp_init` initialised every byte.
            let mut parser = unsafe { storage.assume_init() };
            assert_eq!(parser.type_, 2);
            assert_eq!(parser.settings.cast_const().cast(), table);
            assert!(parser.data.is_null());
            assert_eq!(parser.error, 0);
            // An unnamed type is stored as its low byte, as C's `uint8_t` field takes it.
            // SAFETY: as above, over the initialised parser, with no table.
            unsafe { llhttp_init(&raw mut parser, 0x0103, core::ptr::null()) };
            assert_eq!(parser.type_, 3);
            assert!(parser.settings.is_null());
        }

        /// Refuses an empty run with `7`, and takes every other.
        struct RefusingEmpty;

        impl DataHandler for RefusingEmpty {
            fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
                if data.is_empty() { 7 } else { 0 }
            }
        }

        static REFUSING_EMPTY: LazyLock<Settings> = LazyLock::new(|| {
            let mut settings = Settings::new();
            settings
                .as_mut()
                .set_on_header_field(Some(DataCallback::new::<RefusingEmpty>()));
            settings
        });

        /// The regression test of
        /// `equiv/execute-export-moves-an-empty-runs-error-pos` (`wave-194/batch-7`).
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn llhttp_execute_keeps_an_empty_runs_error_pos_at_data() {
            let mut parser = Parser::new();
            parser.as_mut().init(
                crate::libgit2::deps::llhttp::llhttp::ParserType::Response,
                Some(REFUSING_EMPTY.as_ref()),
            );
            let raw = parser.as_mut().as_mut_ptr();
            let head = b"HTTP/1.1 200 OK\r\nConten";
            // SAFETY: the parser is live and exclusively ours, its table `'static`; `head` is
            // live for the call. The run leaves the header's name open.
            let ret = unsafe { llhttp_execute(raw, head.as_ptr().cast(), head.len()) };
            assert_eq!(ret, sys::HPE_OK);
            let tail = *b"t-Length: 0\r\n\r\n";
            let data = tail.as_ptr().cast();
            // SAFETY: as above, over a run of 0 bytes at `data`, where the open name ends,
            // which its callback refuses.
            let ret = unsafe { llhttp_execute(raw, data, 0) };
            assert_eq!(ret, 7);
            // SAFETY: the parser is live; the getter reads its `error_pos`.
            assert_eq!(unsafe { llhttp_get_error_pos(raw) }, data);
            // A NULL run of 0 bytes is read as empty, its callback refusing it too.
            parser.as_mut().reset();
            let raw = parser.as_mut().as_mut_ptr();
            // SAFETY: as above.
            let ret = unsafe { llhttp_execute(raw, head.as_ptr().cast(), head.len()) };
            assert_eq!(ret, sys::HPE_OK);
            // SAFETY: as above; a NULL `data` with `len` 0 is read as an empty run.
            assert_eq!(unsafe { llhttp_execute(raw, core::ptr::null(), 0) }, 7);
        }

        #[test]
        fn llhttp_finish_returns_cs_codes() {
            let mut parser = parser();
            let raw = parser.as_mut().as_mut_ptr();
            // SAFETY: the parser is live and exclusively ours, for each call below.
            assert_eq!(unsafe { llhttp_finish(raw) }, sys::HPE_OK);
            parser.as_mut().set_finish_raw(1);
            let raw = parser.as_mut().as_mut_ptr();
            // `on_message_complete`'s value, unchanged.
            // SAFETY: as above.
            assert_eq!(unsafe { llhttp_finish(raw) }, 7);
            parser.as_mut().set_finish_raw(2);
            let raw = parser.as_mut().as_mut_ptr();
            // SAFETY: as above.
            assert_eq!(unsafe { llhttp_finish(raw) }, sys::HPE_INVALID_EOF_STATE);
            // A finish state no `llhttp_finish` names, where C aborts, is `HPE_INTERNAL`.
            parser.as_mut().set_finish_raw(9);
            let raw = parser.as_mut().as_mut_ptr();
            // SAFETY: as above.
            assert_eq!(unsafe { llhttp_finish(raw) }, sys::HPE_INTERNAL);
            LOG.take();
        }
    }
}
