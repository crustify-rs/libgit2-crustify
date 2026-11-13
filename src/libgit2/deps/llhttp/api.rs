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
//! generated in debug mode, which this build's is not.
//!
//! C outside api.c calls six of them, libgit2's `transports/httpparser.c` and the macros of
//! `transports/httpparser.h`, which `transports/httpclient.c` expands: `llhttp_execute`,
//! `llhttp_get_error_pos`, `llhttp_get_error_reason`, `llhttp_pause`, `llhttp_resume` and
//! `llhttp_resume_after_upgrade`. They are exported, under the module's export feature,
//! `ffi-export-deps-llhttp-api-c`, which stays off until api.c switches from C.
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

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use std::io;

use ffibox::CRefSeam;

pub use self::ffi_import::{Settings, SettingsMut, SettingsRef};
use super::llhttp::{Errno, ParseError, Parsed, ParserMut, ParserRef};
use crate::sys::libgit2 as sys;

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

#[cfg(test)]
mod unit_priv {
    use ffibox::{CRefSeam, CZeroable};

    use super::lenient;
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
}

#[expect(
    unsafe_code,
    reason = "reads and writes llhttp_settings_s's slots through its handles' pointers, calls \
              llhttp_settings_init, asserts the table's `Send` and `Sync`, and borrows C's \
              `stderr`"
)]
mod ffi_import {
    use std::io;

    use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};
    use glibc_bindings::libc::libio::stdio::stderr;

    use crate::libgit2::deps::llhttp::llhttp::{Callback, DataCallback};
    use crate::sys::libgit2 as sys;

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

    impl Settings {
        /// An empty table: every slot NULL, so that a parser given it runs no callback, as
        /// `llhttp_settings_init` leaves it.
        ///
        /// ```
        /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
        ///
        /// let settings = Settings::new();
        /// assert!(settings.as_ref().on_body().is_none());
        /// assert!(settings.as_ref().on_message_complete().is_none());
        /// ```
        ///
        /// Wraps `fn::llhttp_settings_init`
        #[doc(alias = "llhttp_settings_init")]
        #[must_use]
        pub fn new() -> Self {
            let mut settings = <Self as CZeroable>::zeroed();
            let raw = settings.as_mut().as_mut_ptr();
            // SAFETY: `raw` comes from the exclusive handle of `settings`, a live table Rust
            // owns, which nothing else accesses for the call. `llhttp_settings_init` only
            // `memset`s it to zero, which is a table every handle reads (`CZeroable`), and
            // keeps no pointer to it.
            unsafe { sys::llhttp_settings_init(raw) };
            settings
        }
    }

    impl Default for Settings {
        /// [`Settings::new`].
        fn default() -> Self {
            Self::new()
        }
    }

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
    use core::ffi::{CStr, c_char};
    use core::slice;

    use ffibox::callback::barrier;
    use ffibox::{CMutSeam, CRefSeam};

    use super::{Errno, Parsed, ParserMut, ParserRef};
    use crate::sys::libgit2 as sys;

    // Each export has its raw binding's type: the array's elements coerce to one function
    // pointer type only when their signatures agree.
    const _: () = {
        let _ = [sys::llhttp_execute, llhttp_execute];
        let _ = [sys::llhttp_get_error_pos, llhttp_get_error_pos];
        let _ = [sys::llhttp_get_error_reason, llhttp_get_error_reason];
        let _ = [sys::llhttp_pause, llhttp_pause];
        let _ = [sys::llhttp_resume, llhttp_resume];
        let _ = [
            sys::llhttp_resume_after_upgrade,
            llhttp_resume_after_upgrade,
        ];
    };

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
        // the handle's lifetime. Its `reason` is NULL or one of llhttp's literals, which live
        // as long as the program, since the remaining C sets it only through llhttp; and its
        // `settings` and `data` meet `set_settings_ptr`'s and `set_data`'s contracts, which
        // is what C's own callers of `llhttp_execute` promise: the handle's invariant.
        unsafe { ParserMut::from_ptr(parser) }.expect("api.c's functions take a parser")
    }

    /// `llhttp_execute`: [`ParserMut::execute`] over the `len` bytes at `data`, its outcome
    /// returned as C's `llhttp_errno_t`: `HPE_OK`, `HPE_PAUSED` or `HPE_PAUSED_UPGRADE` for the
    /// run's [`Parsed`], or the error the parser stopped at. A panic, which `execute` raises
    /// where C would abort or corrupt the parser, is reported as `HPE_INTERNAL`.
    ///
    /// With `len` 0, `data` may be NULL, as C's `data + len` allows: no bytes are read.
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
        let bytes: &[u8] = if len == 0 {
            &[]
        } else {
            // SAFETY: the caller guarantees that `data` is valid for reads of `len` bytes,
            // which nothing writes during the call; `u8` and `c_char` have one size and
            // alignment.
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
}
