//! `deps/llhttp/llhttp.c`: llhttp's generated parser and the types of `llhttp.h`. Its
//! items are bound here ahead of their consumers until the module is ported.
//!
//! # The parser
//!
//! A [`Parser`] is llhttp's parser state, `struct llhttp__internal_s`, which `llhttp.h` also
//! names `llhttp__internal_t` and `llhttp_t`, and libgit2's `transports/httpparser.h`
//! `git_http_parser` ([`InternalT`], [`LlhttpT`], [`HttpParser`]). It owns nothing: libgit2
//! holds it by value, embedded in its HTTP client, and no routine allocates or frees one
//! (`llhttp_alloc` and `llhttp_free` are WebAssembly's alone). `llhttp_init` and
//! `llhttp_reset` fill it in place, through `llhttp__internal_init`, and `llhttp_execute`
//! advances it over each run of input. C keeps no pointer to a parser between calls: each
//! callback is handed the parser it runs for. So a `Parser` moves freely, and a parser Rust
//! holds is a plain value, read through [`ParserRef`] and written through [`ParserMut`].
//!
//! Its fields fall in two groups:
//!
//! - the message's state, which the parser writes as it reads a message and api.c's and http.c's
//!   routines read and partly write: the [`ParserType`], the last [`Errno`] and its reason, the
//!   position of the error, the method or status code, the HTTP version, the `Content-Length`, the
//!   upgrade, finish and flag words, and the lenient flags a user sets; and
//! - two pointers a user stores, which llhttp carries without reading: `data`, the user's own
//!   context, and `settings`, the `llhttp_settings_t` table of callbacks api.c's dispatchers read
//!   on every event.
//!
//! The parser's private state, `_index`, `_span_pos0`, `_span_cb0`, `_current`,
//! `header_state` and `initial_message_completed`, is only `llhttp.c`'s generated code's,
//! which reads and writes it alone: no consumer of the parser touches it, so it has no
//! accessor. `_current` and `_span_cb0` are the state and span callback that code jumps
//! through, and `_index` an index into its match tables, so a value written from outside
//! would send C through a wild function pointer or out of a table's bounds.
//!
//! An all-zero parser is valid for every accessor: [`Parser`] is `CZeroable`, and
//! `Parser::zeroed()` builds one whose fields read as zero, `HTTP_BOTH`, `HPE_OK`, no
//! reason and no error position. It is not an initialised parser: `llhttp__internal_init`
//! also sets the starting state, which [`Parser::new`] gives, and [`ParserMut::reinit`]
//! sets again in place, as `llhttp_init` and `llhttp_reset` do.
//!
//! # Reading the fields
//!
//! The fields C fills with a value of one of `llhttp.h`'s enums are read as the enum
//! ([`ParserRef::parser_type`], [`ParserRef::error`], [`ParserRef::method`]), fallibly,
//! since C may store a value the enum does not name: a callback's own return becomes the
//! error, and the method of a response is never set. Their bytes are also read as C stores
//! them (`*_raw`), as api.c's getters return them. The flag words are read and written as
//! their integers (`flags_raw`, `lenient_flags_raw`, `finish_raw`), until `llhttp_flags`,
//! `llhttp_lenient_flags` and `llhttp_finish` have types of their own.
//!
//! The reason is a C string llhttp sets from its own literals, which lives as long as the
//! program, Rust sets only to a `'static` one, and a C callback, as `llhttp_set_error_reason`
//! lets it, only to one that lives while the parser holds it (`set_settings_ptr`'s contract),
//! so it is lent for the handle's borrow.
//! The error position points into the input of the `llhttp_execute` call that failed, which
//! the caller may since have freed, so it is never followed: [`ParserRef::error_offset`]
//! compares it with the input it is given. `data` and `settings` are read as the addresses
//! they hold, since the callbacks read them as the types they expect, beyond any lifetime the
//! parser could carry. `data` is written only through an `unsafe` setter, and `settings` too,
//! but for a table Rust built, api.c's [`Settings`](super::api::Settings), which
//! [`ParserMut::set_settings`] stores safely: its callbacks are sound with any parser.
//!
//! # Parsing
//!
//! [`ParserMut::parse`] runs llhttp's generated parser, `llhttp__internal_execute`, over a
//! run of input. It returns how the run left the parser: read through ([`Parsed::All`]), or
//! paused by a callback or at an upgrade, which llhttp's callers treat as an early stop; or
//! the [`ParseError`] it stopped at, an `llhttp_errno` or a data callback's own return value,
//! which the parser keeps until it is initialised again.
//!
//! The parser calls back, from the parse and on its thread, the callbacks its settings name
//! (through api.c's dispatchers), handing each the parser itself: a callback reads and
//! writes the parser through the exclusive handle it is given, but cannot parse it, nor
//! initialise it, which would leave the outer parse resuming from a state it did not leave,
//! with a span pointing into another input: both panic, and the trampoline turns the panic
//! into the callback's error. A parser that has no state to run from, as
//! `Parser::zeroed()` builds it, makes the parse panic where C would abort.
//!
//! # Callbacks
//!
//! `llhttp_cb` and `llhttp_data_cb`, the callbacks a parser's settings name, are
//! [`Callback`] and [`DataCallback`]: a function pointer each, `Copy` and never released,
//! since neither type has a context parameter. The parser is their context: its `data` is
//! the user's, read through the parser they are handed. A Rust implementation is a type
//! implementing [`EventHandler`] or [`DataHandler`], which carries no state of its own, and
//! is reached through a trampoline that converts C's arguments and catches a panic; a C one
//! is made only inside the crate, through an `unsafe` constructor. Either is called safely
//! with [`Callback::call`] and [`DataCallback::call`], as api.c's dispatchers call them. The
//! settings table that holds them, `llhttp_settings_s`, is api.c's
//! [`Settings`](super::api::Settings), whose slots hold only these two types.
//!
//! # The X-macros
//!
//! `HTTP_ERRNO_MAP`, `HTTP_ALL_METHOD_MAP` and `HTTP_STATUS_MAP` list the constants of
//! `llhttp_errno`, `llhttp_method` and `llhttp_status` as `XX(NUM, NAME, STRING)` entries,
//! for `llhttp.h`'s users to expand: api.c's `llhttp_errno_name`, `llhttp_method_name` and
//! `llhttp_status_name` turn each into a `case` returning its text. Their entries are the
//! enums' variants, in the enums' order, so they are not a second table: each enum's `MAP`
//! lists its variants in the X-macro's order, and its `map_name`, `map_string` and
//! `constant_name` give an entry's `NAME`, its `STRING` and the constant its users paste
//! from `NAME`, each a `&'static CStr` taken from the constant's own name, which C hands
//! back as it is.
//!
//! # The enums
//!
//! [`Errno`], [`Method`], [`Status`] and [`ParserType`] are `llhttp.h`'s closed enums
//! `llhttp_errno`, `llhttp_method`, `llhttp_status` and `llhttp_type`, each a Rust enum over
//! the constants bindgen generates, with a variant per constant and the C integer as its
//! discriminant. A value from C enters through [`Errno::from_raw`] and its siblings, or
//! `TryFrom`, which give `None`, or an `Unnamed*` error holding the value, for one the enum
//! does not name; [`raw`](Errno::raw) and `From` give the integer C takes. No integer is
//! transmuted into a variant.

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

use core::ffi::{CStr, c_int};
use core::fmt;

pub use self::ffi_import::{Callback, DataCallback, Parser, ParserMut, ParserRef};
use crate::sys::libgit2 as sys;

/// Declares a closed enum of `llhttp.h` over its bindgen constants: each variant's
/// discriminant is its constant, `from_raw` maps the C integer back to the variant carrying
/// it, and `raw` gives the integer C takes. The second item is the error `TryFrom` reports,
/// carrying the integer the enum does not name.
///
/// It also keeps, privately, what the X-macros that list an enum's constants need of each
/// variant: its variants in declaration order, which is their X-macro's (`ALL`), its
/// constant's name as C spells it (`constant_cstr`), and the text an X-macro gives in place
/// of the name, where it gives one (`=> c"M-SEARCH"`, read by `string_override`).
macro_rules! llhttp_enum {
    (@string) => { None };
    (@string $string:literal) => { Some($string) };
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $raw:ty {
            $($(#[$vmeta:meta])* $variant:ident = sys::$c:ident $(=> $string:literal)?,)*
        }
        $(#[$umeta:meta])*
        pub struct $unnamed:ident($what:literal);
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$vmeta])* $variant = sys::$c,)*
        }

        impl $name {
            /// The variant whose C value is `raw`; `None` for a value `llhttp.h` does not
            /// name.
            #[must_use]
            pub const fn from_raw(raw: $raw) -> Option<Self> {
                match raw {
                    $(sys::$c => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// Every variant, in the order `llhttp.h` declares its constants.
            const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The name of the variant's constant, as `llhttp.h` spells it.
            const fn constant_cstr(self) -> &'static CStr {
                match self {
                    $(Self::$variant => cstr(concat!(stringify!($c), "\0").as_bytes()),)*
                }
            }

            /// The text an X-macro gives the variant in place of its name, where it gives
            /// one.
            const fn string_override(self) -> Option<&'static CStr> {
                match self {
                    $(Self::$variant => llhttp_enum!(@string $($string)?),)*
                }
            }

            /// The C value, as llhttp's functions take and return it.
            #[must_use]
            pub const fn raw(self) -> $raw {
                self as $raw
            }
        }

        impl TryFrom<$raw> for $name {
            type Error = $unnamed;

            fn try_from(raw: $raw) -> Result<Self, $unnamed> {
                Self::from_raw(raw).ok_or($unnamed(raw))
            }
        }

        impl From<$name> for $raw {
            fn from(value: $name) -> $raw {
                value.raw()
            }
        }

        $(#[$umeta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $unnamed($raw);

        impl $unnamed {
            /// The value, as it was converted.
            #[must_use]
            pub const fn raw(self) -> $raw {
                self.0
            }
        }

        impl fmt::Display for $unnamed {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!("llhttp ", $what, " "))?;
                fmt::Display::fmt(&self.0, f)?;
                f.write_str(" is not named by llhttp.h")
            }
        }

        impl std::error::Error for $unnamed {}
    };
}

llhttp_enum! {
    /// The outcome of a parse, `enum llhttp_errno`: what `llhttp_execute` and
    /// `llhttp_finish` return and a parser keeps as its error.
    ///
    /// [`Ok`](Self::Ok) is success, and [`Paused`](Self::Paused) and
    /// [`PausedUpgrade`](Self::PausedUpgrade) pauses a parse resumes from; every other
    /// variant is an error, which the parser keeps until it is initialised again. A
    /// callback's own return value can stand in its place, and need not be a variant.
    ///
    /// Wraps `enm::llhttp_errno`
    #[doc(alias = "llhttp_errno")]
    pub enum Errno: sys::llhttp_errno {
        /// No error: the data parsed, or the call succeeded.
        ///
        /// Wraps `enm::HPE_OK`
        #[doc(alias = "HPE_OK")]
        Ok = sys::HPE_OK,
        /// An internal error of the parser.
        ///
        /// Wraps `enm::HPE_INTERNAL`
        #[doc(alias = "HPE_INTERNAL")]
        Internal = sys::HPE_INTERNAL,
        /// Input that only strict parsing rejects.
        ///
        /// Wraps `enm::HPE_STRICT`
        #[doc(alias = "HPE_STRICT")]
        Strict = sys::HPE_STRICT,
        /// A carriage return was expected.
        ///
        /// Wraps `enm::HPE_CR_EXPECTED`
        #[doc(alias = "HPE_CR_EXPECTED")]
        CrExpected = sys::HPE_CR_EXPECTED,
        /// A line feed was expected.
        ///
        /// Wraps `enm::HPE_LF_EXPECTED`
        #[doc(alias = "HPE_LF_EXPECTED")]
        LfExpected = sys::HPE_LF_EXPECTED,
        /// A `Content-Length` header where the message may carry none.
        ///
        /// Wraps `enm::HPE_UNEXPECTED_CONTENT_LENGTH`
        #[doc(alias = "HPE_UNEXPECTED_CONTENT_LENGTH")]
        UnexpectedContentLength = sys::HPE_UNEXPECTED_CONTENT_LENGTH,
        /// A space where none is allowed.
        ///
        /// Wraps `enm::HPE_UNEXPECTED_SPACE`
        #[doc(alias = "HPE_UNEXPECTED_SPACE")]
        UnexpectedSpace = sys::HPE_UNEXPECTED_SPACE,
        /// Data after the message on a connection that closes.
        ///
        /// Wraps `enm::HPE_CLOSED_CONNECTION`
        #[doc(alias = "HPE_CLOSED_CONNECTION")]
        ClosedConnection = sys::HPE_CLOSED_CONNECTION,
        /// A request method llhttp does not know.
        ///
        /// Wraps `enm::HPE_INVALID_METHOD`
        #[doc(alias = "HPE_INVALID_METHOD")]
        InvalidMethod = sys::HPE_INVALID_METHOD,
        /// An invalid request target.
        ///
        /// Wraps `enm::HPE_INVALID_URL`
        #[doc(alias = "HPE_INVALID_URL")]
        InvalidUrl = sys::HPE_INVALID_URL,
        /// An invalid fixed token, such as `HTTP/`.
        ///
        /// Wraps `enm::HPE_INVALID_CONSTANT`
        #[doc(alias = "HPE_INVALID_CONSTANT")]
        InvalidConstant = sys::HPE_INVALID_CONSTANT,
        /// An invalid HTTP version.
        ///
        /// Wraps `enm::HPE_INVALID_VERSION`
        #[doc(alias = "HPE_INVALID_VERSION")]
        InvalidVersion = sys::HPE_INVALID_VERSION,
        /// An invalid character in a header field or value.
        ///
        /// Wraps `enm::HPE_INVALID_HEADER_TOKEN`
        #[doc(alias = "HPE_INVALID_HEADER_TOKEN")]
        InvalidHeaderToken = sys::HPE_INVALID_HEADER_TOKEN,
        /// An invalid `Content-Length` value.
        ///
        /// Wraps `enm::HPE_INVALID_CONTENT_LENGTH`
        #[doc(alias = "HPE_INVALID_CONTENT_LENGTH")]
        InvalidContentLength = sys::HPE_INVALID_CONTENT_LENGTH,
        /// An invalid chunk size of a chunked body.
        ///
        /// Wraps `enm::HPE_INVALID_CHUNK_SIZE`
        #[doc(alias = "HPE_INVALID_CHUNK_SIZE")]
        InvalidChunkSize = sys::HPE_INVALID_CHUNK_SIZE,
        /// An invalid status code.
        ///
        /// Wraps `enm::HPE_INVALID_STATUS`
        #[doc(alias = "HPE_INVALID_STATUS")]
        InvalidStatus = sys::HPE_INVALID_STATUS,
        /// The input ended where a message may not end.
        ///
        /// Wraps `enm::HPE_INVALID_EOF_STATE`
        #[doc(alias = "HPE_INVALID_EOF_STATE")]
        InvalidEofState = sys::HPE_INVALID_EOF_STATE,
        /// An invalid `Transfer-Encoding`.
        ///
        /// Wraps `enm::HPE_INVALID_TRANSFER_ENCODING`
        #[doc(alias = "HPE_INVALID_TRANSFER_ENCODING")]
        InvalidTransferEncoding = sys::HPE_INVALID_TRANSFER_ENCODING,
        /// The `on_message_begin` callback failed.
        ///
        /// Wraps `enm::HPE_CB_MESSAGE_BEGIN`
        #[doc(alias = "HPE_CB_MESSAGE_BEGIN")]
        CbMessageBegin = sys::HPE_CB_MESSAGE_BEGIN,
        /// The `on_headers_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_HEADERS_COMPLETE`
        #[doc(alias = "HPE_CB_HEADERS_COMPLETE")]
        CbHeadersComplete = sys::HPE_CB_HEADERS_COMPLETE,
        /// The `on_message_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_MESSAGE_COMPLETE`
        #[doc(alias = "HPE_CB_MESSAGE_COMPLETE")]
        CbMessageComplete = sys::HPE_CB_MESSAGE_COMPLETE,
        /// The `on_chunk_header` callback failed.
        ///
        /// Wraps `enm::HPE_CB_CHUNK_HEADER`
        #[doc(alias = "HPE_CB_CHUNK_HEADER")]
        CbChunkHeader = sys::HPE_CB_CHUNK_HEADER,
        /// The `on_chunk_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_CHUNK_COMPLETE`
        #[doc(alias = "HPE_CB_CHUNK_COMPLETE")]
        CbChunkComplete = sys::HPE_CB_CHUNK_COMPLETE,
        /// Parsing is paused, by a callback or `llhttp_pause`.
        ///
        /// Wraps `enm::HPE_PAUSED`
        #[doc(alias = "HPE_PAUSED")]
        Paused = sys::HPE_PAUSED,
        /// Parsing paused after a message that upgrades the connection.
        ///
        /// Wraps `enm::HPE_PAUSED_UPGRADE`
        #[doc(alias = "HPE_PAUSED_UPGRADE")]
        PausedUpgrade = sys::HPE_PAUSED_UPGRADE,
        /// Parsing paused at an HTTP/2 connection preface.
        ///
        /// Wraps `enm::HPE_PAUSED_H2_UPGRADE`
        #[doc(alias = "HPE_PAUSED_H2_UPGRADE")]
        PausedH2Upgrade = sys::HPE_PAUSED_H2_UPGRADE,
        /// A data callback failed (returned `-1`).
        ///
        /// Wraps `enm::HPE_USER`
        #[doc(alias = "HPE_USER")]
        User = sys::HPE_USER,
        /// The `on_url_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_URL_COMPLETE`
        #[doc(alias = "HPE_CB_URL_COMPLETE")]
        CbUrlComplete = sys::HPE_CB_URL_COMPLETE,
        /// The `on_status_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_STATUS_COMPLETE`
        #[doc(alias = "HPE_CB_STATUS_COMPLETE")]
        CbStatusComplete = sys::HPE_CB_STATUS_COMPLETE,
        /// The `on_method_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_METHOD_COMPLETE`
        #[doc(alias = "HPE_CB_METHOD_COMPLETE")]
        CbMethodComplete = sys::HPE_CB_METHOD_COMPLETE,
        /// The `on_version_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_VERSION_COMPLETE`
        #[doc(alias = "HPE_CB_VERSION_COMPLETE")]
        CbVersionComplete = sys::HPE_CB_VERSION_COMPLETE,
        /// The `on_header_field_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_HEADER_FIELD_COMPLETE`
        #[doc(alias = "HPE_CB_HEADER_FIELD_COMPLETE")]
        CbHeaderFieldComplete = sys::HPE_CB_HEADER_FIELD_COMPLETE,
        /// The `on_header_value_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_HEADER_VALUE_COMPLETE`
        #[doc(alias = "HPE_CB_HEADER_VALUE_COMPLETE")]
        CbHeaderValueComplete = sys::HPE_CB_HEADER_VALUE_COMPLETE,
        /// The `on_chunk_extension_name_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_CHUNK_EXTENSION_NAME_COMPLETE`
        #[doc(alias = "HPE_CB_CHUNK_EXTENSION_NAME_COMPLETE")]
        CbChunkExtensionNameComplete = sys::HPE_CB_CHUNK_EXTENSION_NAME_COMPLETE,
        /// The `on_chunk_extension_value_complete` callback failed.
        ///
        /// Wraps `enm::HPE_CB_CHUNK_EXTENSION_VALUE_COMPLETE`
        #[doc(alias = "HPE_CB_CHUNK_EXTENSION_VALUE_COMPLETE")]
        CbChunkExtensionValueComplete = sys::HPE_CB_CHUNK_EXTENSION_VALUE_COMPLETE,
        /// The `on_reset` callback failed.
        ///
        /// Wraps `enm::HPE_CB_RESET`
        #[doc(alias = "HPE_CB_RESET")]
        CbReset = sys::HPE_CB_RESET,
    }
    /// An errno that `llhttp.h` does not name: a callback's own return value, kept as the
    /// parser's error. [`raw`](Self::raw) has it.
    pub struct UnnamedErrno("errno");
}

/// `enum llhttp_errno` under its typedef's name, `llhttp_errno_t`.
///
/// Wraps `tdf::llhttp_errno_t`
#[doc(alias = "llhttp_errno_t")]
pub type ErrnoT = Errno;

llhttp_enum! {
    /// A request method, `enum llhttp_method`: HTTP's, RTSP's and HTTP/2's preface, as
    /// the parser stores the method of the request it reads.
    ///
    /// Wraps `enm::llhttp_method`
    #[doc(alias = "llhttp_method")]
    pub enum Method: sys::llhttp_method {
        /// `DELETE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_DELETE`
        #[doc(alias = "HTTP_DELETE")]
        Delete = sys::HTTP_DELETE,
        /// `GET`, an HTTP and RTSP method.
        ///
        /// Wraps `enm::HTTP_GET`
        #[doc(alias = "HTTP_GET")]
        Get = sys::HTTP_GET,
        /// `HEAD`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_HEAD`
        #[doc(alias = "HTTP_HEAD")]
        Head = sys::HTTP_HEAD,
        /// `POST`, an HTTP and RTSP method.
        ///
        /// Wraps `enm::HTTP_POST`
        #[doc(alias = "HTTP_POST")]
        Post = sys::HTTP_POST,
        /// `PUT`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_PUT`
        #[doc(alias = "HTTP_PUT")]
        Put = sys::HTTP_PUT,
        /// `CONNECT`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_CONNECT`
        #[doc(alias = "HTTP_CONNECT")]
        Connect = sys::HTTP_CONNECT,
        /// `OPTIONS`, an HTTP and RTSP method.
        ///
        /// Wraps `enm::HTTP_OPTIONS`
        #[doc(alias = "HTTP_OPTIONS")]
        Options = sys::HTTP_OPTIONS,
        /// `TRACE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_TRACE`
        #[doc(alias = "HTTP_TRACE")]
        Trace = sys::HTTP_TRACE,
        /// `COPY`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_COPY`
        #[doc(alias = "HTTP_COPY")]
        Copy = sys::HTTP_COPY,
        /// `LOCK`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_LOCK`
        #[doc(alias = "HTTP_LOCK")]
        Lock = sys::HTTP_LOCK,
        /// `MKCOL`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MKCOL`
        #[doc(alias = "HTTP_MKCOL")]
        Mkcol = sys::HTTP_MKCOL,
        /// `MOVE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MOVE`
        #[doc(alias = "HTTP_MOVE")]
        Move = sys::HTTP_MOVE,
        /// `PROPFIND`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_PROPFIND`
        #[doc(alias = "HTTP_PROPFIND")]
        Propfind = sys::HTTP_PROPFIND,
        /// `PROPPATCH`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_PROPPATCH`
        #[doc(alias = "HTTP_PROPPATCH")]
        Proppatch = sys::HTTP_PROPPATCH,
        /// `SEARCH`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_SEARCH`
        #[doc(alias = "HTTP_SEARCH")]
        Search = sys::HTTP_SEARCH,
        /// `UNLOCK`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_UNLOCK`
        #[doc(alias = "HTTP_UNLOCK")]
        Unlock = sys::HTTP_UNLOCK,
        /// `BIND`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_BIND`
        #[doc(alias = "HTTP_BIND")]
        Bind = sys::HTTP_BIND,
        /// `REBIND`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_REBIND`
        #[doc(alias = "HTTP_REBIND")]
        Rebind = sys::HTTP_REBIND,
        /// `UNBIND`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_UNBIND`
        #[doc(alias = "HTTP_UNBIND")]
        Unbind = sys::HTTP_UNBIND,
        /// `ACL`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_ACL`
        #[doc(alias = "HTTP_ACL")]
        Acl = sys::HTTP_ACL,
        /// `REPORT`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_REPORT`
        #[doc(alias = "HTTP_REPORT")]
        Report = sys::HTTP_REPORT,
        /// `MKACTIVITY`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MKACTIVITY`
        #[doc(alias = "HTTP_MKACTIVITY")]
        Mkactivity = sys::HTTP_MKACTIVITY,
        /// `CHECKOUT`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_CHECKOUT`
        #[doc(alias = "HTTP_CHECKOUT")]
        Checkout = sys::HTTP_CHECKOUT,
        /// `MERGE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MERGE`
        #[doc(alias = "HTTP_MERGE")]
        Merge = sys::HTTP_MERGE,
        /// `M-SEARCH`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MSEARCH`
        #[doc(alias = "HTTP_MSEARCH")]
        MSearch = sys::HTTP_MSEARCH => c"M-SEARCH",
        /// `NOTIFY`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_NOTIFY`
        #[doc(alias = "HTTP_NOTIFY")]
        Notify = sys::HTTP_NOTIFY,
        /// `SUBSCRIBE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_SUBSCRIBE`
        #[doc(alias = "HTTP_SUBSCRIBE")]
        Subscribe = sys::HTTP_SUBSCRIBE,
        /// `UNSUBSCRIBE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_UNSUBSCRIBE`
        #[doc(alias = "HTTP_UNSUBSCRIBE")]
        Unsubscribe = sys::HTTP_UNSUBSCRIBE,
        /// `PATCH`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_PATCH`
        #[doc(alias = "HTTP_PATCH")]
        Patch = sys::HTTP_PATCH,
        /// `PURGE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_PURGE`
        #[doc(alias = "HTTP_PURGE")]
        Purge = sys::HTTP_PURGE,
        /// `MKCALENDAR`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_MKCALENDAR`
        #[doc(alias = "HTTP_MKCALENDAR")]
        Mkcalendar = sys::HTTP_MKCALENDAR,
        /// `LINK`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_LINK`
        #[doc(alias = "HTTP_LINK")]
        Link = sys::HTTP_LINK,
        /// `UNLINK`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_UNLINK`
        #[doc(alias = "HTTP_UNLINK")]
        Unlink = sys::HTTP_UNLINK,
        /// `SOURCE`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_SOURCE`
        #[doc(alias = "HTTP_SOURCE")]
        Source = sys::HTTP_SOURCE,
        /// `PRI`, the method of the HTTP/2 connection preface.
        ///
        /// Wraps `enm::HTTP_PRI`
        #[doc(alias = "HTTP_PRI")]
        Pri = sys::HTTP_PRI,
        /// `DESCRIBE`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_DESCRIBE`
        #[doc(alias = "HTTP_DESCRIBE")]
        Describe = sys::HTTP_DESCRIBE,
        /// `ANNOUNCE`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_ANNOUNCE`
        #[doc(alias = "HTTP_ANNOUNCE")]
        Announce = sys::HTTP_ANNOUNCE,
        /// `SETUP`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_SETUP`
        #[doc(alias = "HTTP_SETUP")]
        Setup = sys::HTTP_SETUP,
        /// `PLAY`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_PLAY`
        #[doc(alias = "HTTP_PLAY")]
        Play = sys::HTTP_PLAY,
        /// `PAUSE`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_PAUSE`
        #[doc(alias = "HTTP_PAUSE")]
        Pause = sys::HTTP_PAUSE,
        /// `TEARDOWN`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_TEARDOWN`
        #[doc(alias = "HTTP_TEARDOWN")]
        Teardown = sys::HTTP_TEARDOWN,
        /// `GET_PARAMETER`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_GET_PARAMETER`
        #[doc(alias = "HTTP_GET_PARAMETER")]
        GetParameter = sys::HTTP_GET_PARAMETER,
        /// `SET_PARAMETER`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_SET_PARAMETER`
        #[doc(alias = "HTTP_SET_PARAMETER")]
        SetParameter = sys::HTTP_SET_PARAMETER,
        /// `REDIRECT`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_REDIRECT`
        #[doc(alias = "HTTP_REDIRECT")]
        Redirect = sys::HTTP_REDIRECT,
        /// `RECORD`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_RECORD`
        #[doc(alias = "HTTP_RECORD")]
        Record = sys::HTTP_RECORD,
        /// `FLUSH`, an RTSP method.
        ///
        /// Wraps `enm::HTTP_FLUSH`
        #[doc(alias = "HTTP_FLUSH")]
        Flush = sys::HTTP_FLUSH,
        /// `QUERY`, an HTTP method.
        ///
        /// Wraps `enm::HTTP_QUERY`
        #[doc(alias = "HTTP_QUERY")]
        Query = sys::HTTP_QUERY,
    }
    /// A method that `llhttp.h` does not name. [`raw`](Self::raw) has it.
    pub struct UnnamedMethod("method");
}

/// `enum llhttp_method` under its typedef's name, `llhttp_method_t`.
///
/// Wraps `tdf::llhttp_method_t`
#[doc(alias = "llhttp_method_t")]
pub type MethodT = Method;

llhttp_enum! {
    /// A response status code that `llhttp.h` names, `enum llhttp_status`: its
    /// discriminant is the code.
    ///
    /// The parser stores any three-digit code it reads, named or not
    /// ([`ParserRef::status_code`]); [`from_raw`](Self::from_raw) gives `None` for one
    /// without a variant.
    ///
    /// Wraps `enm::llhttp_status`
    #[doc(alias = "llhttp_status")]
    pub enum Status: sys::llhttp_status {
        /// `100`, `CONTINUE`.
        ///
        /// Wraps `enm::HTTP_STATUS_CONTINUE`
        #[doc(alias = "HTTP_STATUS_CONTINUE")]
        Continue = sys::HTTP_STATUS_CONTINUE,
        /// `101`, `SWITCHING_PROTOCOLS`.
        ///
        /// Wraps `enm::HTTP_STATUS_SWITCHING_PROTOCOLS`
        #[doc(alias = "HTTP_STATUS_SWITCHING_PROTOCOLS")]
        SwitchingProtocols = sys::HTTP_STATUS_SWITCHING_PROTOCOLS,
        /// `102`, `PROCESSING`.
        ///
        /// Wraps `enm::HTTP_STATUS_PROCESSING`
        #[doc(alias = "HTTP_STATUS_PROCESSING")]
        Processing = sys::HTTP_STATUS_PROCESSING,
        /// `103`, `EARLY_HINTS`.
        ///
        /// Wraps `enm::HTTP_STATUS_EARLY_HINTS`
        #[doc(alias = "HTTP_STATUS_EARLY_HINTS")]
        EarlyHints = sys::HTTP_STATUS_EARLY_HINTS,
        /// `110`, `RESPONSE_IS_STALE`.
        ///
        /// Wraps `enm::HTTP_STATUS_RESPONSE_IS_STALE`
        #[doc(alias = "HTTP_STATUS_RESPONSE_IS_STALE")]
        ResponseIsStale = sys::HTTP_STATUS_RESPONSE_IS_STALE,
        /// `111`, `REVALIDATION_FAILED`.
        ///
        /// Wraps `enm::HTTP_STATUS_REVALIDATION_FAILED`
        #[doc(alias = "HTTP_STATUS_REVALIDATION_FAILED")]
        RevalidationFailed = sys::HTTP_STATUS_REVALIDATION_FAILED,
        /// `112`, `DISCONNECTED_OPERATION`.
        ///
        /// Wraps `enm::HTTP_STATUS_DISCONNECTED_OPERATION`
        #[doc(alias = "HTTP_STATUS_DISCONNECTED_OPERATION")]
        DisconnectedOperation = sys::HTTP_STATUS_DISCONNECTED_OPERATION,
        /// `113`, `HEURISTIC_EXPIRATION`.
        ///
        /// Wraps `enm::HTTP_STATUS_HEURISTIC_EXPIRATION`
        #[doc(alias = "HTTP_STATUS_HEURISTIC_EXPIRATION")]
        HeuristicExpiration = sys::HTTP_STATUS_HEURISTIC_EXPIRATION,
        /// `199`, `MISCELLANEOUS_WARNING`.
        ///
        /// Wraps `enm::HTTP_STATUS_MISCELLANEOUS_WARNING`
        #[doc(alias = "HTTP_STATUS_MISCELLANEOUS_WARNING")]
        MiscellaneousWarning = sys::HTTP_STATUS_MISCELLANEOUS_WARNING,
        /// `200`, `OK`.
        ///
        /// Wraps `enm::HTTP_STATUS_OK`
        #[doc(alias = "HTTP_STATUS_OK")]
        Ok = sys::HTTP_STATUS_OK,
        /// `201`, `CREATED`.
        ///
        /// Wraps `enm::HTTP_STATUS_CREATED`
        #[doc(alias = "HTTP_STATUS_CREATED")]
        Created = sys::HTTP_STATUS_CREATED,
        /// `202`, `ACCEPTED`.
        ///
        /// Wraps `enm::HTTP_STATUS_ACCEPTED`
        #[doc(alias = "HTTP_STATUS_ACCEPTED")]
        Accepted = sys::HTTP_STATUS_ACCEPTED,
        /// `203`, `NON_AUTHORITATIVE_INFORMATION`.
        ///
        /// Wraps `enm::HTTP_STATUS_NON_AUTHORITATIVE_INFORMATION`
        #[doc(alias = "HTTP_STATUS_NON_AUTHORITATIVE_INFORMATION")]
        NonAuthoritativeInformation = sys::HTTP_STATUS_NON_AUTHORITATIVE_INFORMATION,
        /// `204`, `NO_CONTENT`.
        ///
        /// Wraps `enm::HTTP_STATUS_NO_CONTENT`
        #[doc(alias = "HTTP_STATUS_NO_CONTENT")]
        NoContent = sys::HTTP_STATUS_NO_CONTENT,
        /// `205`, `RESET_CONTENT`.
        ///
        /// Wraps `enm::HTTP_STATUS_RESET_CONTENT`
        #[doc(alias = "HTTP_STATUS_RESET_CONTENT")]
        ResetContent = sys::HTTP_STATUS_RESET_CONTENT,
        /// `206`, `PARTIAL_CONTENT`.
        ///
        /// Wraps `enm::HTTP_STATUS_PARTIAL_CONTENT`
        #[doc(alias = "HTTP_STATUS_PARTIAL_CONTENT")]
        PartialContent = sys::HTTP_STATUS_PARTIAL_CONTENT,
        /// `207`, `MULTI_STATUS`.
        ///
        /// Wraps `enm::HTTP_STATUS_MULTI_STATUS`
        #[doc(alias = "HTTP_STATUS_MULTI_STATUS")]
        MultiStatus = sys::HTTP_STATUS_MULTI_STATUS,
        /// `208`, `ALREADY_REPORTED`.
        ///
        /// Wraps `enm::HTTP_STATUS_ALREADY_REPORTED`
        #[doc(alias = "HTTP_STATUS_ALREADY_REPORTED")]
        AlreadyReported = sys::HTTP_STATUS_ALREADY_REPORTED,
        /// `214`, `TRANSFORMATION_APPLIED`.
        ///
        /// Wraps `enm::HTTP_STATUS_TRANSFORMATION_APPLIED`
        #[doc(alias = "HTTP_STATUS_TRANSFORMATION_APPLIED")]
        TransformationApplied = sys::HTTP_STATUS_TRANSFORMATION_APPLIED,
        /// `226`, `IM_USED`.
        ///
        /// Wraps `enm::HTTP_STATUS_IM_USED`
        #[doc(alias = "HTTP_STATUS_IM_USED")]
        ImUsed = sys::HTTP_STATUS_IM_USED,
        /// `299`, `MISCELLANEOUS_PERSISTENT_WARNING`.
        ///
        /// Wraps `enm::HTTP_STATUS_MISCELLANEOUS_PERSISTENT_WARNING`
        #[doc(alias = "HTTP_STATUS_MISCELLANEOUS_PERSISTENT_WARNING")]
        MiscellaneousPersistentWarning = sys::HTTP_STATUS_MISCELLANEOUS_PERSISTENT_WARNING,
        /// `300`, `MULTIPLE_CHOICES`.
        ///
        /// Wraps `enm::HTTP_STATUS_MULTIPLE_CHOICES`
        #[doc(alias = "HTTP_STATUS_MULTIPLE_CHOICES")]
        MultipleChoices = sys::HTTP_STATUS_MULTIPLE_CHOICES,
        /// `301`, `MOVED_PERMANENTLY`.
        ///
        /// Wraps `enm::HTTP_STATUS_MOVED_PERMANENTLY`
        #[doc(alias = "HTTP_STATUS_MOVED_PERMANENTLY")]
        MovedPermanently = sys::HTTP_STATUS_MOVED_PERMANENTLY,
        /// `302`, `FOUND`.
        ///
        /// Wraps `enm::HTTP_STATUS_FOUND`
        #[doc(alias = "HTTP_STATUS_FOUND")]
        Found = sys::HTTP_STATUS_FOUND,
        /// `303`, `SEE_OTHER`.
        ///
        /// Wraps `enm::HTTP_STATUS_SEE_OTHER`
        #[doc(alias = "HTTP_STATUS_SEE_OTHER")]
        SeeOther = sys::HTTP_STATUS_SEE_OTHER,
        /// `304`, `NOT_MODIFIED`.
        ///
        /// Wraps `enm::HTTP_STATUS_NOT_MODIFIED`
        #[doc(alias = "HTTP_STATUS_NOT_MODIFIED")]
        NotModified = sys::HTTP_STATUS_NOT_MODIFIED,
        /// `305`, `USE_PROXY`.
        ///
        /// Wraps `enm::HTTP_STATUS_USE_PROXY`
        #[doc(alias = "HTTP_STATUS_USE_PROXY")]
        UseProxy = sys::HTTP_STATUS_USE_PROXY,
        /// `306`, `SWITCH_PROXY`.
        ///
        /// Wraps `enm::HTTP_STATUS_SWITCH_PROXY`
        #[doc(alias = "HTTP_STATUS_SWITCH_PROXY")]
        SwitchProxy = sys::HTTP_STATUS_SWITCH_PROXY,
        /// `307`, `TEMPORARY_REDIRECT`.
        ///
        /// Wraps `enm::HTTP_STATUS_TEMPORARY_REDIRECT`
        #[doc(alias = "HTTP_STATUS_TEMPORARY_REDIRECT")]
        TemporaryRedirect = sys::HTTP_STATUS_TEMPORARY_REDIRECT,
        /// `308`, `PERMANENT_REDIRECT`.
        ///
        /// Wraps `enm::HTTP_STATUS_PERMANENT_REDIRECT`
        #[doc(alias = "HTTP_STATUS_PERMANENT_REDIRECT")]
        PermanentRedirect = sys::HTTP_STATUS_PERMANENT_REDIRECT,
        /// `400`, `BAD_REQUEST`.
        ///
        /// Wraps `enm::HTTP_STATUS_BAD_REQUEST`
        #[doc(alias = "HTTP_STATUS_BAD_REQUEST")]
        BadRequest = sys::HTTP_STATUS_BAD_REQUEST,
        /// `401`, `UNAUTHORIZED`.
        ///
        /// Wraps `enm::HTTP_STATUS_UNAUTHORIZED`
        #[doc(alias = "HTTP_STATUS_UNAUTHORIZED")]
        Unauthorized = sys::HTTP_STATUS_UNAUTHORIZED,
        /// `402`, `PAYMENT_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_PAYMENT_REQUIRED`
        #[doc(alias = "HTTP_STATUS_PAYMENT_REQUIRED")]
        PaymentRequired = sys::HTTP_STATUS_PAYMENT_REQUIRED,
        /// `403`, `FORBIDDEN`.
        ///
        /// Wraps `enm::HTTP_STATUS_FORBIDDEN`
        #[doc(alias = "HTTP_STATUS_FORBIDDEN")]
        Forbidden = sys::HTTP_STATUS_FORBIDDEN,
        /// `404`, `NOT_FOUND`.
        ///
        /// Wraps `enm::HTTP_STATUS_NOT_FOUND`
        #[doc(alias = "HTTP_STATUS_NOT_FOUND")]
        NotFound = sys::HTTP_STATUS_NOT_FOUND,
        /// `405`, `METHOD_NOT_ALLOWED`.
        ///
        /// Wraps `enm::HTTP_STATUS_METHOD_NOT_ALLOWED`
        #[doc(alias = "HTTP_STATUS_METHOD_NOT_ALLOWED")]
        MethodNotAllowed = sys::HTTP_STATUS_METHOD_NOT_ALLOWED,
        /// `406`, `NOT_ACCEPTABLE`.
        ///
        /// Wraps `enm::HTTP_STATUS_NOT_ACCEPTABLE`
        #[doc(alias = "HTTP_STATUS_NOT_ACCEPTABLE")]
        NotAcceptable = sys::HTTP_STATUS_NOT_ACCEPTABLE,
        /// `407`, `PROXY_AUTHENTICATION_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_PROXY_AUTHENTICATION_REQUIRED`
        #[doc(alias = "HTTP_STATUS_PROXY_AUTHENTICATION_REQUIRED")]
        ProxyAuthenticationRequired = sys::HTTP_STATUS_PROXY_AUTHENTICATION_REQUIRED,
        /// `408`, `REQUEST_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_REQUEST_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_REQUEST_TIMEOUT")]
        RequestTimeout = sys::HTTP_STATUS_REQUEST_TIMEOUT,
        /// `409`, `CONFLICT`.
        ///
        /// Wraps `enm::HTTP_STATUS_CONFLICT`
        #[doc(alias = "HTTP_STATUS_CONFLICT")]
        Conflict = sys::HTTP_STATUS_CONFLICT,
        /// `410`, `GONE`.
        ///
        /// Wraps `enm::HTTP_STATUS_GONE`
        #[doc(alias = "HTTP_STATUS_GONE")]
        Gone = sys::HTTP_STATUS_GONE,
        /// `411`, `LENGTH_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_LENGTH_REQUIRED`
        #[doc(alias = "HTTP_STATUS_LENGTH_REQUIRED")]
        LengthRequired = sys::HTTP_STATUS_LENGTH_REQUIRED,
        /// `412`, `PRECONDITION_FAILED`.
        ///
        /// Wraps `enm::HTTP_STATUS_PRECONDITION_FAILED`
        #[doc(alias = "HTTP_STATUS_PRECONDITION_FAILED")]
        PreconditionFailed = sys::HTTP_STATUS_PRECONDITION_FAILED,
        /// `413`, `PAYLOAD_TOO_LARGE`.
        ///
        /// Wraps `enm::HTTP_STATUS_PAYLOAD_TOO_LARGE`
        #[doc(alias = "HTTP_STATUS_PAYLOAD_TOO_LARGE")]
        PayloadTooLarge = sys::HTTP_STATUS_PAYLOAD_TOO_LARGE,
        /// `414`, `URI_TOO_LONG`.
        ///
        /// Wraps `enm::HTTP_STATUS_URI_TOO_LONG`
        #[doc(alias = "HTTP_STATUS_URI_TOO_LONG")]
        UriTooLong = sys::HTTP_STATUS_URI_TOO_LONG,
        /// `415`, `UNSUPPORTED_MEDIA_TYPE`.
        ///
        /// Wraps `enm::HTTP_STATUS_UNSUPPORTED_MEDIA_TYPE`
        #[doc(alias = "HTTP_STATUS_UNSUPPORTED_MEDIA_TYPE")]
        UnsupportedMediaType = sys::HTTP_STATUS_UNSUPPORTED_MEDIA_TYPE,
        /// `416`, `RANGE_NOT_SATISFIABLE`.
        ///
        /// Wraps `enm::HTTP_STATUS_RANGE_NOT_SATISFIABLE`
        #[doc(alias = "HTTP_STATUS_RANGE_NOT_SATISFIABLE")]
        RangeNotSatisfiable = sys::HTTP_STATUS_RANGE_NOT_SATISFIABLE,
        /// `417`, `EXPECTATION_FAILED`.
        ///
        /// Wraps `enm::HTTP_STATUS_EXPECTATION_FAILED`
        #[doc(alias = "HTTP_STATUS_EXPECTATION_FAILED")]
        ExpectationFailed = sys::HTTP_STATUS_EXPECTATION_FAILED,
        /// `418`, `IM_A_TEAPOT`.
        ///
        /// Wraps `enm::HTTP_STATUS_IM_A_TEAPOT`
        #[doc(alias = "HTTP_STATUS_IM_A_TEAPOT")]
        ImATeapot = sys::HTTP_STATUS_IM_A_TEAPOT,
        /// `419`, `PAGE_EXPIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_PAGE_EXPIRED`
        #[doc(alias = "HTTP_STATUS_PAGE_EXPIRED")]
        PageExpired = sys::HTTP_STATUS_PAGE_EXPIRED,
        /// `420`, `ENHANCE_YOUR_CALM`.
        ///
        /// Wraps `enm::HTTP_STATUS_ENHANCE_YOUR_CALM`
        #[doc(alias = "HTTP_STATUS_ENHANCE_YOUR_CALM")]
        EnhanceYourCalm = sys::HTTP_STATUS_ENHANCE_YOUR_CALM,
        /// `421`, `MISDIRECTED_REQUEST`.
        ///
        /// Wraps `enm::HTTP_STATUS_MISDIRECTED_REQUEST`
        #[doc(alias = "HTTP_STATUS_MISDIRECTED_REQUEST")]
        MisdirectedRequest = sys::HTTP_STATUS_MISDIRECTED_REQUEST,
        /// `422`, `UNPROCESSABLE_ENTITY`.
        ///
        /// Wraps `enm::HTTP_STATUS_UNPROCESSABLE_ENTITY`
        #[doc(alias = "HTTP_STATUS_UNPROCESSABLE_ENTITY")]
        UnprocessableEntity = sys::HTTP_STATUS_UNPROCESSABLE_ENTITY,
        /// `423`, `LOCKED`.
        ///
        /// Wraps `enm::HTTP_STATUS_LOCKED`
        #[doc(alias = "HTTP_STATUS_LOCKED")]
        Locked = sys::HTTP_STATUS_LOCKED,
        /// `424`, `FAILED_DEPENDENCY`.
        ///
        /// Wraps `enm::HTTP_STATUS_FAILED_DEPENDENCY`
        #[doc(alias = "HTTP_STATUS_FAILED_DEPENDENCY")]
        FailedDependency = sys::HTTP_STATUS_FAILED_DEPENDENCY,
        /// `425`, `TOO_EARLY`.
        ///
        /// Wraps `enm::HTTP_STATUS_TOO_EARLY`
        #[doc(alias = "HTTP_STATUS_TOO_EARLY")]
        TooEarly = sys::HTTP_STATUS_TOO_EARLY,
        /// `426`, `UPGRADE_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_UPGRADE_REQUIRED`
        #[doc(alias = "HTTP_STATUS_UPGRADE_REQUIRED")]
        UpgradeRequired = sys::HTTP_STATUS_UPGRADE_REQUIRED,
        /// `428`, `PRECONDITION_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_PRECONDITION_REQUIRED`
        #[doc(alias = "HTTP_STATUS_PRECONDITION_REQUIRED")]
        PreconditionRequired = sys::HTTP_STATUS_PRECONDITION_REQUIRED,
        /// `429`, `TOO_MANY_REQUESTS`.
        ///
        /// Wraps `enm::HTTP_STATUS_TOO_MANY_REQUESTS`
        #[doc(alias = "HTTP_STATUS_TOO_MANY_REQUESTS")]
        TooManyRequests = sys::HTTP_STATUS_TOO_MANY_REQUESTS,
        /// `430`, `REQUEST_HEADER_FIELDS_TOO_LARGE_UNOFFICIAL`.
        ///
        /// Wraps `enm::HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE_UNOFFICIAL`
        #[doc(alias = "HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE_UNOFFICIAL")]
        RequestHeaderFieldsTooLargeUnofficial = sys::HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE_UNOFFICIAL,
        /// `431`, `REQUEST_HEADER_FIELDS_TOO_LARGE`.
        ///
        /// Wraps `enm::HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE`
        #[doc(alias = "HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE")]
        RequestHeaderFieldsTooLarge = sys::HTTP_STATUS_REQUEST_HEADER_FIELDS_TOO_LARGE,
        /// `440`, `LOGIN_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_LOGIN_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_LOGIN_TIMEOUT")]
        LoginTimeout = sys::HTTP_STATUS_LOGIN_TIMEOUT,
        /// `444`, `NO_RESPONSE`.
        ///
        /// Wraps `enm::HTTP_STATUS_NO_RESPONSE`
        #[doc(alias = "HTTP_STATUS_NO_RESPONSE")]
        NoResponse = sys::HTTP_STATUS_NO_RESPONSE,
        /// `449`, `RETRY_WITH`.
        ///
        /// Wraps `enm::HTTP_STATUS_RETRY_WITH`
        #[doc(alias = "HTTP_STATUS_RETRY_WITH")]
        RetryWith = sys::HTTP_STATUS_RETRY_WITH,
        /// `450`, `BLOCKED_BY_PARENTAL_CONTROL`.
        ///
        /// Wraps `enm::HTTP_STATUS_BLOCKED_BY_PARENTAL_CONTROL`
        #[doc(alias = "HTTP_STATUS_BLOCKED_BY_PARENTAL_CONTROL")]
        BlockedByParentalControl = sys::HTTP_STATUS_BLOCKED_BY_PARENTAL_CONTROL,
        /// `451`, `UNAVAILABLE_FOR_LEGAL_REASONS`.
        ///
        /// Wraps `enm::HTTP_STATUS_UNAVAILABLE_FOR_LEGAL_REASONS`
        #[doc(alias = "HTTP_STATUS_UNAVAILABLE_FOR_LEGAL_REASONS")]
        UnavailableForLegalReasons = sys::HTTP_STATUS_UNAVAILABLE_FOR_LEGAL_REASONS,
        /// `460`, `CLIENT_CLOSED_LOAD_BALANCED_REQUEST`.
        ///
        /// Wraps `enm::HTTP_STATUS_CLIENT_CLOSED_LOAD_BALANCED_REQUEST`
        #[doc(alias = "HTTP_STATUS_CLIENT_CLOSED_LOAD_BALANCED_REQUEST")]
        ClientClosedLoadBalancedRequest = sys::HTTP_STATUS_CLIENT_CLOSED_LOAD_BALANCED_REQUEST,
        /// `463`, `INVALID_X_FORWARDED_FOR`.
        ///
        /// Wraps `enm::HTTP_STATUS_INVALID_X_FORWARDED_FOR`
        #[doc(alias = "HTTP_STATUS_INVALID_X_FORWARDED_FOR")]
        InvalidXForwardedFor = sys::HTTP_STATUS_INVALID_X_FORWARDED_FOR,
        /// `494`, `REQUEST_HEADER_TOO_LARGE`.
        ///
        /// Wraps `enm::HTTP_STATUS_REQUEST_HEADER_TOO_LARGE`
        #[doc(alias = "HTTP_STATUS_REQUEST_HEADER_TOO_LARGE")]
        RequestHeaderTooLarge = sys::HTTP_STATUS_REQUEST_HEADER_TOO_LARGE,
        /// `495`, `SSL_CERTIFICATE_ERROR`.
        ///
        /// Wraps `enm::HTTP_STATUS_SSL_CERTIFICATE_ERROR`
        #[doc(alias = "HTTP_STATUS_SSL_CERTIFICATE_ERROR")]
        SslCertificateError = sys::HTTP_STATUS_SSL_CERTIFICATE_ERROR,
        /// `496`, `SSL_CERTIFICATE_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_SSL_CERTIFICATE_REQUIRED`
        #[doc(alias = "HTTP_STATUS_SSL_CERTIFICATE_REQUIRED")]
        SslCertificateRequired = sys::HTTP_STATUS_SSL_CERTIFICATE_REQUIRED,
        /// `497`, `HTTP_REQUEST_SENT_TO_HTTPS_PORT`.
        ///
        /// Wraps `enm::HTTP_STATUS_HTTP_REQUEST_SENT_TO_HTTPS_PORT`
        #[doc(alias = "HTTP_STATUS_HTTP_REQUEST_SENT_TO_HTTPS_PORT")]
        HttpRequestSentToHttpsPort = sys::HTTP_STATUS_HTTP_REQUEST_SENT_TO_HTTPS_PORT,
        /// `498`, `INVALID_TOKEN`.
        ///
        /// Wraps `enm::HTTP_STATUS_INVALID_TOKEN`
        #[doc(alias = "HTTP_STATUS_INVALID_TOKEN")]
        InvalidToken = sys::HTTP_STATUS_INVALID_TOKEN,
        /// `499`, `CLIENT_CLOSED_REQUEST`.
        ///
        /// Wraps `enm::HTTP_STATUS_CLIENT_CLOSED_REQUEST`
        #[doc(alias = "HTTP_STATUS_CLIENT_CLOSED_REQUEST")]
        ClientClosedRequest = sys::HTTP_STATUS_CLIENT_CLOSED_REQUEST,
        /// `500`, `INTERNAL_SERVER_ERROR`.
        ///
        /// Wraps `enm::HTTP_STATUS_INTERNAL_SERVER_ERROR`
        #[doc(alias = "HTTP_STATUS_INTERNAL_SERVER_ERROR")]
        InternalServerError = sys::HTTP_STATUS_INTERNAL_SERVER_ERROR,
        /// `501`, `NOT_IMPLEMENTED`.
        ///
        /// Wraps `enm::HTTP_STATUS_NOT_IMPLEMENTED`
        #[doc(alias = "HTTP_STATUS_NOT_IMPLEMENTED")]
        NotImplemented = sys::HTTP_STATUS_NOT_IMPLEMENTED,
        /// `502`, `BAD_GATEWAY`.
        ///
        /// Wraps `enm::HTTP_STATUS_BAD_GATEWAY`
        #[doc(alias = "HTTP_STATUS_BAD_GATEWAY")]
        BadGateway = sys::HTTP_STATUS_BAD_GATEWAY,
        /// `503`, `SERVICE_UNAVAILABLE`.
        ///
        /// Wraps `enm::HTTP_STATUS_SERVICE_UNAVAILABLE`
        #[doc(alias = "HTTP_STATUS_SERVICE_UNAVAILABLE")]
        ServiceUnavailable = sys::HTTP_STATUS_SERVICE_UNAVAILABLE,
        /// `504`, `GATEWAY_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_GATEWAY_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_GATEWAY_TIMEOUT")]
        GatewayTimeout = sys::HTTP_STATUS_GATEWAY_TIMEOUT,
        /// `505`, `HTTP_VERSION_NOT_SUPPORTED`.
        ///
        /// Wraps `enm::HTTP_STATUS_HTTP_VERSION_NOT_SUPPORTED`
        #[doc(alias = "HTTP_STATUS_HTTP_VERSION_NOT_SUPPORTED")]
        HttpVersionNotSupported = sys::HTTP_STATUS_HTTP_VERSION_NOT_SUPPORTED,
        /// `506`, `VARIANT_ALSO_NEGOTIATES`.
        ///
        /// Wraps `enm::HTTP_STATUS_VARIANT_ALSO_NEGOTIATES`
        #[doc(alias = "HTTP_STATUS_VARIANT_ALSO_NEGOTIATES")]
        VariantAlsoNegotiates = sys::HTTP_STATUS_VARIANT_ALSO_NEGOTIATES,
        /// `507`, `INSUFFICIENT_STORAGE`.
        ///
        /// Wraps `enm::HTTP_STATUS_INSUFFICIENT_STORAGE`
        #[doc(alias = "HTTP_STATUS_INSUFFICIENT_STORAGE")]
        InsufficientStorage = sys::HTTP_STATUS_INSUFFICIENT_STORAGE,
        /// `508`, `LOOP_DETECTED`.
        ///
        /// Wraps `enm::HTTP_STATUS_LOOP_DETECTED`
        #[doc(alias = "HTTP_STATUS_LOOP_DETECTED")]
        LoopDetected = sys::HTTP_STATUS_LOOP_DETECTED,
        /// `509`, `BANDWIDTH_LIMIT_EXCEEDED`.
        ///
        /// Wraps `enm::HTTP_STATUS_BANDWIDTH_LIMIT_EXCEEDED`
        #[doc(alias = "HTTP_STATUS_BANDWIDTH_LIMIT_EXCEEDED")]
        BandwidthLimitExceeded = sys::HTTP_STATUS_BANDWIDTH_LIMIT_EXCEEDED,
        /// `510`, `NOT_EXTENDED`.
        ///
        /// Wraps `enm::HTTP_STATUS_NOT_EXTENDED`
        #[doc(alias = "HTTP_STATUS_NOT_EXTENDED")]
        NotExtended = sys::HTTP_STATUS_NOT_EXTENDED,
        /// `511`, `NETWORK_AUTHENTICATION_REQUIRED`.
        ///
        /// Wraps `enm::HTTP_STATUS_NETWORK_AUTHENTICATION_REQUIRED`
        #[doc(alias = "HTTP_STATUS_NETWORK_AUTHENTICATION_REQUIRED")]
        NetworkAuthenticationRequired = sys::HTTP_STATUS_NETWORK_AUTHENTICATION_REQUIRED,
        /// `520`, `WEB_SERVER_UNKNOWN_ERROR`.
        ///
        /// Wraps `enm::HTTP_STATUS_WEB_SERVER_UNKNOWN_ERROR`
        #[doc(alias = "HTTP_STATUS_WEB_SERVER_UNKNOWN_ERROR")]
        WebServerUnknownError = sys::HTTP_STATUS_WEB_SERVER_UNKNOWN_ERROR,
        /// `521`, `WEB_SERVER_IS_DOWN`.
        ///
        /// Wraps `enm::HTTP_STATUS_WEB_SERVER_IS_DOWN`
        #[doc(alias = "HTTP_STATUS_WEB_SERVER_IS_DOWN")]
        WebServerIsDown = sys::HTTP_STATUS_WEB_SERVER_IS_DOWN,
        /// `522`, `CONNECTION_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_CONNECTION_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_CONNECTION_TIMEOUT")]
        ConnectionTimeout = sys::HTTP_STATUS_CONNECTION_TIMEOUT,
        /// `523`, `ORIGIN_IS_UNREACHABLE`.
        ///
        /// Wraps `enm::HTTP_STATUS_ORIGIN_IS_UNREACHABLE`
        #[doc(alias = "HTTP_STATUS_ORIGIN_IS_UNREACHABLE")]
        OriginIsUnreachable = sys::HTTP_STATUS_ORIGIN_IS_UNREACHABLE,
        /// `524`, `TIMEOUT_OCCURED`.
        ///
        /// Wraps `enm::HTTP_STATUS_TIMEOUT_OCCURED`
        #[doc(alias = "HTTP_STATUS_TIMEOUT_OCCURED")]
        TimeoutOccured = sys::HTTP_STATUS_TIMEOUT_OCCURED,
        /// `525`, `SSL_HANDSHAKE_FAILED`.
        ///
        /// Wraps `enm::HTTP_STATUS_SSL_HANDSHAKE_FAILED`
        #[doc(alias = "HTTP_STATUS_SSL_HANDSHAKE_FAILED")]
        SslHandshakeFailed = sys::HTTP_STATUS_SSL_HANDSHAKE_FAILED,
        /// `526`, `INVALID_SSL_CERTIFICATE`.
        ///
        /// Wraps `enm::HTTP_STATUS_INVALID_SSL_CERTIFICATE`
        #[doc(alias = "HTTP_STATUS_INVALID_SSL_CERTIFICATE")]
        InvalidSslCertificate = sys::HTTP_STATUS_INVALID_SSL_CERTIFICATE,
        /// `527`, `RAILGUN_ERROR`.
        ///
        /// Wraps `enm::HTTP_STATUS_RAILGUN_ERROR`
        #[doc(alias = "HTTP_STATUS_RAILGUN_ERROR")]
        RailgunError = sys::HTTP_STATUS_RAILGUN_ERROR,
        /// `529`, `SITE_IS_OVERLOADED`.
        ///
        /// Wraps `enm::HTTP_STATUS_SITE_IS_OVERLOADED`
        #[doc(alias = "HTTP_STATUS_SITE_IS_OVERLOADED")]
        SiteIsOverloaded = sys::HTTP_STATUS_SITE_IS_OVERLOADED,
        /// `530`, `SITE_IS_FROZEN`.
        ///
        /// Wraps `enm::HTTP_STATUS_SITE_IS_FROZEN`
        #[doc(alias = "HTTP_STATUS_SITE_IS_FROZEN")]
        SiteIsFrozen = sys::HTTP_STATUS_SITE_IS_FROZEN,
        /// `561`, `IDENTITY_PROVIDER_AUTHENTICATION_ERROR`.
        ///
        /// Wraps `enm::HTTP_STATUS_IDENTITY_PROVIDER_AUTHENTICATION_ERROR`
        #[doc(alias = "HTTP_STATUS_IDENTITY_PROVIDER_AUTHENTICATION_ERROR")]
        IdentityProviderAuthenticationError = sys::HTTP_STATUS_IDENTITY_PROVIDER_AUTHENTICATION_ERROR,
        /// `598`, `NETWORK_READ_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_NETWORK_READ_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_NETWORK_READ_TIMEOUT")]
        NetworkReadTimeout = sys::HTTP_STATUS_NETWORK_READ_TIMEOUT,
        /// `599`, `NETWORK_CONNECT_TIMEOUT`.
        ///
        /// Wraps `enm::HTTP_STATUS_NETWORK_CONNECT_TIMEOUT`
        #[doc(alias = "HTTP_STATUS_NETWORK_CONNECT_TIMEOUT")]
        NetworkConnectTimeout = sys::HTTP_STATUS_NETWORK_CONNECT_TIMEOUT,
    }
    /// A status code that `llhttp.h` does not name. [`raw`](Self::raw) has it.
    pub struct UnnamedStatus("status");
}

/// `enum llhttp_status` under its typedef's name, `llhttp_status_t`.
///
/// Wraps `tdf::llhttp_status_t`
#[doc(alias = "llhttp_status_t")]
pub type StatusT = Status;

llhttp_enum! {
    /// Which messages a parser reads, `enum llhttp_type`: requests, responses, or either,
    /// until the first message decides.
    ///
    /// Wraps `enm::llhttp_type`
    #[doc(alias = "llhttp_type")]
    pub enum ParserType: sys::llhttp_type {
        /// Requests or responses: the parser takes the type of the first message it reads.
        ///
        /// Wraps `enm::HTTP_BOTH`
        #[doc(alias = "HTTP_BOTH")]
        Both = sys::HTTP_BOTH,
        /// Requests.
        ///
        /// Wraps `enm::HTTP_REQUEST`
        #[doc(alias = "HTTP_REQUEST")]
        Request = sys::HTTP_REQUEST,
        /// Responses.
        ///
        /// Wraps `enm::HTTP_RESPONSE`
        #[doc(alias = "HTTP_RESPONSE")]
        Response = sys::HTTP_RESPONSE,
    }
    /// A parser type that `llhttp.h` does not name. [`raw`](Self::raw) has it.
    pub struct UnnamedParserType("parser type");
}

/// `enum llhttp_type` under its typedef's name, `llhttp_type_t`.
///
/// Wraps `tdf::llhttp_type_t`
#[doc(alias = "llhttp_type_t")]
pub type ParserTypeT = ParserType;

// --- The X-macros ----------------------------------------------------------------------

/// The `&'static CStr` over a literal's bytes, which end in its NUL and hold no other.
const fn cstr(bytes: &'static [u8]) -> &'static CStr {
    match CStr::from_bytes_with_nul(bytes) {
        Ok(s) => s,
        Err(_) => panic!("a constant's name holds a NUL"),
    }
}

/// `name` without its first `prefix.len()` bytes, once `name` is checked to start with
/// `prefix`: the name an X-macro pastes after the prefix of its enum's constants.
const fn strip_prefix(name: &'static CStr, prefix: &CStr) -> &'static CStr {
    let bytes = name.to_bytes_with_nul();
    let prefix = prefix.to_bytes();
    assert!(
        bytes.len() > prefix.len(),
        "a constant is longer than its prefix"
    );
    let mut i = 0;
    while i < prefix.len() {
        assert!(
            bytes[i] == prefix[i],
            "a constant does not start with its prefix"
        );
        i += 1;
    }
    cstr(bytes.split_at(prefix.len()).1)
}

/// Whether `a` and `b` hold the same bytes.
const fn same_bytes(a: &CStr, b: &CStr) -> bool {
    let (a, b) = (a.to_bytes(), b.to_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Checks, at compile time, that each of an enum's constants starts with `prefix`, so that
/// stripping it gives the name an X-macro pastes after it, and that each text in place of a
/// name is a C string other than the name.
macro_rules! assert_prefixed {
    ($enum:ty, $prefix:expr) => {
        const _: () = {
            let all = <$enum>::ALL;
            let mut i = 0;
            while i < all.len() {
                let name = strip_prefix(all[i].constant_cstr(), $prefix);
                if let Some(string) = all[i].string_override() {
                    assert!(
                        !same_bytes(string, name),
                        "an X-macro's text repeats its name"
                    );
                }
                i += 1;
            }
        };
    };
}

assert_prefixed!(Errno, ERRNO_PREFIX);
assert_prefixed!(Method, METHOD_PREFIX);
assert_prefixed!(Status, STATUS_PREFIX);
// `llhttp_type` is in no X-macro, but its constants take llhttp's `HTTP_` prefix too.
assert_prefixed!(ParserType, METHOD_PREFIX);

/// The prefix `HTTP_ERRNO_MAP`'s users paste before each entry's name: `HPE_##NAME`.
const ERRNO_PREFIX: &CStr = c"HPE_";
/// The prefix `HTTP_ALL_METHOD_MAP`'s users paste before each entry's name: `HTTP_##NAME`.
const METHOD_PREFIX: &CStr = c"HTTP_";
/// The prefix `HTTP_STATUS_MAP`'s users paste before each entry's name:
/// `HTTP_STATUS_##NAME`.
const STATUS_PREFIX: &CStr = c"HTTP_STATUS_";

impl Errno {
    /// The entries of `llhttp.h`'s X-macro `HTTP_ERRNO_MAP`, in its order: one per
    /// constant of `enum llhttp_errno`, which lists them in the same order.
    ///
    /// Each entry `XX(NUM, NAME, STRING)` is a variant: `NUM` is its [`raw`](Self::raw)
    /// value, `NAME` its [`map_name`](Self::map_name) and `STRING` its
    /// [`map_string`](Self::map_string), and the constant its users paste, `HPE_##NAME`, is
    /// its [`constant_name`](Self::constant_name), which `llhttp_errno_name` returns.
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Errno;
    ///
    /// assert_eq!(Errno::MAP.len(), 36);
    /// assert_eq!(Errno::MAP[3], Errno::CrExpected);
    /// assert_eq!(Errno::MAP[3].raw(), 25);
    /// assert_eq!(Errno::MAP[3].constant_name(), c"HPE_CR_EXPECTED");
    /// ```
    ///
    /// Replaces `mac::HTTP_ERRNO_MAP`
    #[doc(alias = "HTTP_ERRNO_MAP")]
    pub const MAP: &'static [Self] = Self::ALL;

    /// The entry's `NAME` in `HTTP_ERRNO_MAP`: its constant's name without `HPE_`, as
    /// `OK` for [`Ok`](Self::Ok).
    ///
    /// Replaces `mac::HTTP_ERRNO_MAP`
    #[doc(alias = "HTTP_ERRNO_MAP")]
    #[must_use]
    pub const fn map_name(self) -> &'static CStr {
        strip_prefix(self.constant_cstr(), ERRNO_PREFIX)
    }

    /// The entry's `STRING` in `HTTP_ERRNO_MAP`, which is its `NAME` for every errno.
    ///
    /// Replaces `mac::HTTP_ERRNO_MAP`
    #[doc(alias = "HTTP_ERRNO_MAP")]
    #[must_use]
    pub const fn map_string(self) -> &'static CStr {
        match self.string_override() {
            Some(string) => string,
            None => self.map_name(),
        }
    }

    /// The constant the entry's `NAME` pastes into, `HPE_##NAME`, as `HPE_OK` for
    /// [`Ok`](Self::Ok): the case label `HTTP_ERRNO_MAP`'s users match, and the string
    /// `llhttp_errno_name` returns, `"HPE_" #NAME`.
    ///
    /// Replaces `mac::HTTP_ERRNO_MAP`
    #[doc(alias = "HTTP_ERRNO_MAP")]
    /// It is the text of api.c's generator `HTTP_ERRNO_GEN(CODE, NAME, _)`, `case HPE_##NAME:
    /// return "HPE_" #NAME;`, which `llhttp_errno_name` expands `HTTP_ERRNO_MAP` with.
    ///
    /// Replaces `mac::HTTP_ERRNO_GEN`
    #[doc(alias = "HTTP_ERRNO_GEN")]
    /// It is api.c's `llhttp_errno_name` too, a `switch` of those `case`s, whose `default:
    /// abort();` no `Errno` reaches.
    ///
    /// Replaces `fn::llhttp_errno_name`
    #[doc(alias = "llhttp_errno_name")]
    #[must_use]
    pub const fn constant_name(self) -> &'static CStr {
        self.constant_cstr()
    }
}

impl Method {
    /// The entries of `llhttp.h`'s X-macro `HTTP_ALL_METHOD_MAP`, in its order: one per
    /// constant of `enum llhttp_method`, which lists them in the same order. Its subsets,
    /// `HTTP_METHOD_MAP` and `RTSP_METHOD_MAP`, are not bound.
    ///
    /// Each entry `XX(NUM, NAME, STRING)` is a variant: `NUM` is its [`raw`](Self::raw)
    /// value, `NAME` its [`map_name`](Self::map_name) and `STRING` its
    /// [`map_string`](Self::map_string), which `llhttp_method_name` returns; the constant
    /// its users paste, `HTTP_##NAME`, is its [`constant_name`](Self::constant_name).
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Method;
    ///
    /// assert_eq!(Method::MAP.len(), 47);
    /// let msearch = Method::MAP[24];
    /// assert_eq!(msearch, Method::MSearch);
    /// assert_eq!(msearch.map_name(), c"MSEARCH");
    /// assert_eq!(msearch.map_string(), c"M-SEARCH");
    /// assert_eq!(msearch.constant_name(), c"HTTP_MSEARCH");
    /// ```
    ///
    /// Replaces `mac::HTTP_ALL_METHOD_MAP`
    #[doc(alias = "HTTP_ALL_METHOD_MAP")]
    pub const MAP: &'static [Self] = Self::ALL;

    /// The entry's `NAME` in `HTTP_ALL_METHOD_MAP`: its constant's name without `HTTP_`,
    /// as `MSEARCH` for [`MSearch`](Self::MSearch).
    ///
    /// Replaces `mac::HTTP_ALL_METHOD_MAP`
    #[doc(alias = "HTTP_ALL_METHOD_MAP")]
    #[must_use]
    pub const fn map_name(self) -> &'static CStr {
        strip_prefix(self.constant_cstr(), METHOD_PREFIX)
    }

    /// The entry's `STRING` in `HTTP_ALL_METHOD_MAP`, the method as a request line spells
    /// it: its `NAME` for every method but [`MSearch`](Self::MSearch), `M-SEARCH`.
    ///
    /// Replaces `mac::HTTP_ALL_METHOD_MAP`
    #[doc(alias = "HTTP_ALL_METHOD_MAP")]
    /// It is the text of api.c's generator `HTTP_METHOD_GEN(NUM, NAME, STRING)`, `case
    /// HTTP_##NAME: return #STRING;`, which `llhttp_method_name` expands
    /// `HTTP_ALL_METHOD_MAP` with.
    ///
    /// Replaces `mac::HTTP_METHOD_GEN`
    #[doc(alias = "HTTP_METHOD_GEN")]
    /// It is api.c's `llhttp_method_name` too, a `switch` of those `case`s, whose `default:
    /// abort();` no `Method` reaches.
    ///
    /// Replaces `fn::llhttp_method_name`
    #[doc(alias = "llhttp_method_name")]
    #[must_use]
    pub const fn map_string(self) -> &'static CStr {
        match self.string_override() {
            Some(string) => string,
            None => self.map_name(),
        }
    }

    /// The constant the entry's `NAME` pastes into, `HTTP_##NAME`, as `HTTP_MSEARCH`: the
    /// case label `HTTP_ALL_METHOD_MAP`'s users match.
    ///
    /// Replaces `mac::HTTP_ALL_METHOD_MAP`
    #[doc(alias = "HTTP_ALL_METHOD_MAP")]
    #[must_use]
    pub const fn constant_name(self) -> &'static CStr {
        self.constant_cstr()
    }
}

impl Status {
    /// The entries of `llhttp.h`'s X-macro `HTTP_STATUS_MAP`, in its order: one per
    /// constant of `enum llhttp_status`, which lists them in the same order.
    ///
    /// Each entry `XX(NUM, NAME, STRING)` is a variant: `NUM` is its [`raw`](Self::raw)
    /// value, `NAME` its [`map_name`](Self::map_name) and `STRING` its
    /// [`map_string`](Self::map_string), which `llhttp_status_name` returns; the constant
    /// its users paste, `HTTP_STATUS_##NAME`, is its [`constant_name`](Self::constant_name).
    ///
    /// ```
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Status;
    ///
    /// assert_eq!(Status::MAP.len(), 99);
    /// assert_eq!(Status::MAP[0].raw(), 100);
    /// assert_eq!(Status::MAP[0].map_string(), c"CONTINUE");
    /// assert_eq!(Status::MAP[0].constant_name(), c"HTTP_STATUS_CONTINUE");
    /// ```
    ///
    /// Replaces `mac::HTTP_STATUS_MAP`
    #[doc(alias = "HTTP_STATUS_MAP")]
    pub const MAP: &'static [Self] = Self::ALL;

    /// The entry's `NAME` in `HTTP_STATUS_MAP`: its constant's name without
    /// `HTTP_STATUS_`, as `NOT_FOUND` for [`NotFound`](Self::NotFound).
    ///
    /// Replaces `mac::HTTP_STATUS_MAP`
    #[doc(alias = "HTTP_STATUS_MAP")]
    #[must_use]
    pub const fn map_name(self) -> &'static CStr {
        strip_prefix(self.constant_cstr(), STATUS_PREFIX)
    }

    /// The entry's `STRING` in `HTTP_STATUS_MAP`, which is its `NAME` for every status.
    ///
    /// Replaces `mac::HTTP_STATUS_MAP`
    #[doc(alias = "HTTP_STATUS_MAP")]
    /// It is the text of api.c's generator `HTTP_STATUS_GEN(NUM, NAME, STRING)`, `case
    /// HTTP_STATUS_##NAME: return #STRING;`, which `llhttp_status_name` expands
    /// `HTTP_STATUS_MAP` with.
    ///
    /// Replaces `mac::HTTP_STATUS_GEN`
    #[doc(alias = "HTTP_STATUS_GEN")]
    /// It is api.c's `llhttp_status_name` too, a `switch` of those `case`s, whose `default:
    /// abort();` no `Status` reaches.
    ///
    /// Replaces `fn::llhttp_status_name`
    #[doc(alias = "llhttp_status_name")]
    #[must_use]
    pub const fn map_string(self) -> &'static CStr {
        match self.string_override() {
            Some(string) => string,
            None => self.map_name(),
        }
    }

    /// The constant the entry's `NAME` pastes into, `HTTP_STATUS_##NAME`, as
    /// `HTTP_STATUS_NOT_FOUND`: the case label `HTTP_STATUS_MAP`'s users match.
    ///
    /// Replaces `mac::HTTP_STATUS_MAP`
    #[doc(alias = "HTTP_STATUS_MAP")]
    #[must_use]
    pub const fn constant_name(self) -> &'static CStr {
        self.constant_cstr()
    }
}

/// `struct llhttp__internal_s` under the name `llhttp.h` gives it beside its
/// tag, `llhttp__internal_t`.
///
/// Wraps `tdf::llhttp__internal_t`
#[doc(alias = "llhttp__internal_t")]
pub type InternalT = Parser;

/// `struct llhttp__internal_s` under the name llhttp's API gives it, `llhttp_t`.
///
/// Wraps `tdf::llhttp_t`
#[doc(alias = "llhttp_t")]
pub type LlhttpT = Parser;

/// `struct llhttp__internal_s` under the name libgit2's HTTP client gives it,
/// `git_http_parser` (`transports/httpparser.h`), when llhttp is its parser.
///
/// Wraps `tdf::git_http_parser`
#[doc(alias = "git_http_parser")]
pub type HttpParser = Parser;

impl ParserRef<'_> {
    /// Which messages the parser reads, `type`, as llhttp's enum.
    ///
    /// # Errors
    ///
    /// [`UnnamedParserType`] when the byte is no `llhttp_type`, which C never stores.
    ///
    /// Wraps `fld::llhttp__internal_s.type`
    #[doc(alias = "llhttp__internal_s.type")]
    /// It is api.c's `llhttp_get_type` too, which returns the byte.
    ///
    /// Replaces `fn::llhttp_get_type`
    #[doc(alias = "llhttp_get_type")]
    pub fn parser_type(&self) -> Result<ParserType, UnnamedParserType> {
        ParserType::try_from(u32::from(self.parser_type_raw()))
    }

    /// The parser's error, `error`, as llhttp's enum: [`Errno::Ok`] while it has none.
    ///
    /// # Errors
    ///
    /// [`UnnamedErrno`] when the error is no `llhttp_errno`: a callback's own return value,
    /// which `llhttp_execute` keeps as the error. Its value is the `int`'s bits, as C's
    /// conversion to the unsigned enum gives them; [`error_raw`](Self::error_raw) has the
    /// `int`.
    ///
    /// Wraps `fld::llhttp__internal_s.error`
    #[doc(alias = "llhttp__internal_s.error")]
    /// It is api.c's `llhttp_get_errno` too.
    ///
    /// Replaces `fn::llhttp_get_errno`
    #[doc(alias = "llhttp_get_errno")]
    pub fn error(&self) -> Result<Errno, UnnamedErrno> {
        Errno::try_from(self.error_raw().cast_unsigned())
    }

    /// The method of the request the parser reads, `method`, as llhttp's enum.
    ///
    /// A response parser never sets it, so it reads as it was initialised:
    /// [`Method::Delete`], `0`.
    ///
    /// # Errors
    ///
    /// [`UnnamedMethod`] when the byte is no `llhttp_method`, which C never stores.
    ///
    /// Wraps `fld::llhttp__internal_s.method`
    #[doc(alias = "llhttp__internal_s.method")]
    /// It is api.c's `llhttp_get_method` too, which returns the byte.
    ///
    /// Replaces `fn::llhttp_get_method`
    #[doc(alias = "llhttp_get_method")]
    pub fn method(&self) -> Result<Method, UnnamedMethod> {
        Method::try_from(u32::from(self.method_raw()))
    }
}

// --- Parsing ---------------------------------------------------------------------------

/// How a run of input left the parser when [`ParserMut::parse`] returns without an error:
/// the outcomes `llhttp__internal_execute` reports through `HPE_OK` and its two resumable
/// pauses, which llhttp's callers treat as an early stop, not a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Parsed {
    /// `HPE_OK`: the parser read every byte, and waits for more.
    All,
    /// `HPE_PAUSED`: a callback returned `HPE_PAUSED`, or the parser was paused before the
    /// call, as `llhttp_pause` and [`ParserMut::set_error`] pause it. It stopped at its
    /// error position ([`ParserRef::error_offset`]), and parses again once its error is
    /// cleared, as `llhttp_resume` clears it; until then each parse returns `Paused` and
    /// reads nothing.
    Paused,
    /// `HPE_PAUSED_UPGRADE`: the message upgraded the connection, so what follows its end,
    /// from the error position on, is the new protocol's, not HTTP. The parser parses again
    /// once its error is cleared, as `llhttp_resume_after_upgrade` clears it.
    PausedUpgrade,
}

/// The error [`ParserMut::parse`] stopped at, as `llhttp__internal_execute` returns it: a
/// nonzero `int` other than the two pauses, which the parser keeps as its error, so that
/// every later parse returns it again until the parser is initialised again.
///
/// It is an `llhttp_errno` the parser set, or a data callback's own return value, which
/// need be none ([`errno`](Self::errno)). The reason the parser or the callback gave is the
/// parser's ([`ParserRef::reason`]), and so is where it stopped
/// ([`ParserRef::error_offset`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParseError(c_int);

impl ParseError {
    /// The `int` C returned.
    #[must_use]
    pub const fn code(self) -> c_int {
        self.0
    }

    /// The error as llhttp's enum.
    ///
    /// # Errors
    ///
    /// [`UnnamedErrno`] when the code is no `llhttp_errno`: a data callback's own return
    /// value, its bits as C's conversion to the unsigned enum gives them.
    pub fn errno(self) -> Result<Errno, UnnamedErrno> {
        Errno::try_from(self.0.cast_unsigned())
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(errno) = Errno::from_raw(self.0.cast_unsigned()) {
            f.write_str("llhttp stopped at ")?;
            f.write_str(&errno.constant_name().to_string_lossy())
        } else {
            f.write_str("llhttp stopped at a callback's error ")?;
            fmt::Display::fmt(&self.0, f)
        }
    }
}

impl std::error::Error for ParseError {}

impl ParseError {
    /// The outcome of a parse whose `llhttp__internal_execute` returned `code`.
    const fn outcome(code: c_int) -> Result<Parsed, Self> {
        match code.cast_unsigned() {
            sys::HPE_OK => Ok(Parsed::All),
            sys::HPE_PAUSED => Ok(Parsed::Paused),
            sys::HPE_PAUSED_UPGRADE => Ok(Parsed::PausedUpgrade),
            _ => Err(Self(code)),
        }
    }
}

// --- Callbacks -------------------------------------------------------------------------

/// A Rust implementation of an [`Callback`]: the function C calls with the parser an event
/// happened on.
///
/// It carries no state: `llhttp_cb` has no context parameter, and a parser's context is its
/// `data`, which the callback reads through the parser it is handed. Its return value is
/// the C callback's, whose meaning the settings slot it sits in gives: `0` to go on, `-1`
/// for an error, `HPE_PAUSED` to pause, and for `on_headers_complete` also `1` to read no
/// body and `2` to read none and pause as an upgrade. A panic is caught, and returns `-1`.
///
/// The parser is the parsing thread's: a handler cannot send it to another, which could
/// parse it beside the parse that runs the handler.
///
/// ```compile_fail,E0277
/// use std::ffi::c_int;
/// use std::thread;
///
/// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{EventHandler, ParserMut};
///
/// struct Sending;
///
/// impl EventHandler for Sending {
///     fn on_event(mut parser: ParserMut<'_>) -> c_int {
///         thread::scope(|s| {
///             s.spawn(move || parser.reinit());
///         });
///         0
///     }
/// }
/// ```
pub trait EventHandler {
    /// Handles the event on `parser`, the one being parsed.
    fn on_event(parser: ParserMut<'_>) -> c_int;
}

/// A Rust implementation of a [`DataCallback`]: the function C calls with the parser and
/// a run of the input it read, such as a header's name or a part of the body.
///
/// As [`EventHandler`], it carries no state, and its return value is the C callback's: `0`
/// to go on, `HPE_PAUSED` to pause, and any other value to stop the parse with it as the
/// error, `-1` becoming `HPE_USER` when api.c's dispatchers make the call. A panic is
/// caught, and returns `-1`.
pub trait DataHandler {
    /// Handles `data`, a run of the input `parser` read, borrowed for the call.
    fn on_data(parser: ParserMut<'_>, data: &[u8]) -> c_int;
}

#[expect(
    unsafe_code,
    reason = "reads and writes llhttp__internal_s's fields through its handles' pointers, calls \
              llhttp__internal_init and llhttp__internal_execute, adapts llhttp's callbacks, and \
              keeps a thread-local register (`thread_local!`)"
)]
mod ffi_import {
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::fmt;
    use std::cell::RefCell;

    use ffibox::{CMutSeam, CRefSeam, CZeroable, callback, define_ctype};

    use super::{DataHandler, Errno, EventHandler, ParseError, Parsed, ParserType};
    use crate::libgit2::deps::llhttp::api::SettingsRef;
    use crate::sys::libgit2 as sys;

    define_ctype!(
        /// llhttp's parser, `struct llhttp__internal_s`: the state of the message it reads,
        /// the error it stopped at, and the two pointers its user stores.
        ///
        /// It owns nothing and C keeps no pointer to it, so it is held by value, as libgit2's
        /// HTTP client embeds it, and read through [`as_ref`](Self::as_ref) and written
        /// through [`as_mut`](Self::as_mut). `Parser::zeroed()` builds one whose fields all
        /// read as zero; the module documentation describes the fields and the private state
        /// that has no accessor.
        ///
        /// Two exclusive handles to one parser cannot coexist:
        ///
        /// ```compile_fail,E0499
        /// use ffibox::CZeroable;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let mut parser = Parser::zeroed();
        /// let a = parser.as_mut();
        /// let b = parser.as_mut();
        /// let _ = (a, b);
        /// ```
        ///
        /// nor a shared one beside an exclusive one:
        ///
        /// ```compile_fail,E0502
        /// use ffibox::CZeroable;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let mut parser = Parser::zeroed();
        /// let m = parser.as_mut();
        /// let r = parser.as_ref();
        /// let _ = (m, r.status_code());
        /// ```
        ///
        /// and a handle cannot outlive the parser:
        ///
        /// ```compile_fail,E0597
        /// use ffibox::CZeroable;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let handle = {
        ///     let parser = Parser::zeroed();
        ///     parser.as_ref()
        /// };
        /// let _ = handle.status_code();
        /// ```
        ///
        /// Wraps `st::llhttp__internal_s`
        #[doc(alias = "llhttp__internal_s")]
        Parser,
        ParserRef,
        ParserMut,
        sys::llhttp__internal_s
    );

    // SAFETY: all-zero is a parser every accessor reads: its scalars are integers, for which
    // every bit pattern is valid and which the getters convert fallibly, and its pointers are
    // NULL. The one pointer a getter follows, `reason`, is read as absent when NULL; the
    // others are read as addresses and never followed. A parser owns nothing, so no teardown
    // is paired with the value, and C keeps no pointer to it (module documentation), so a
    // value moves freely.
    unsafe impl CZeroable for Parser {}

    /// Reads a field through a shared handle's pointer.
    macro_rules! get {
        ($handle:expr, $field:ident) => {{
            let raw = $handle.as_ptr();
            // SAFETY: the handle addresses a live `llhttp__internal_s` for its borrow (its
            // invariant); the projection forms no reference and reads nothing.
            let field = unsafe { &raw const (*raw).$field };
            // SAFETY: `field` is in bounds of that struct, aligned and initialised, every
            // field being an integer or a pointer, and no exclusive handle writes it while
            // this shared one lives.
            unsafe { field.read() }
        }};
    }

    /// Writes a field through an exclusive handle's pointer.
    macro_rules! set {
        ($handle:expr, $field:ident, $value:expr) => {{
            let value = $value;
            let raw = $handle.as_mut_ptr();
            // SAFETY: the handle addresses a live `llhttp__internal_s` for its borrow (its
            // invariant); the projection forms no reference and writes nothing.
            let field = unsafe { &raw mut (*raw).$field };
            // SAFETY: `field` is in bounds of that struct and aligned, and the exclusive
            // handle is the only access to it while it lives. The value is a plain integer or
            // pointer, which needs no drop of what it replaces.
            unsafe { field.write(value) }
        }};
    }

    impl<'a> ParserRef<'a> {
        /// The byte `type` holds: an `llhttp_type`, as `llhttp_get_type` returns it.
        ///
        /// Wraps `fld::llhttp__internal_s.type`
        #[doc(alias = "llhttp__internal_s.type")]
        #[must_use]
        pub fn parser_type_raw(&self) -> u8 {
            get!(self, type_)
        }

        /// The `int` `error` holds: an `llhttp_errno`, or a callback's own return value
        /// that stopped the parse; `0`, `HPE_OK`, while the parser has no error.
        ///
        /// Wraps `fld::llhttp__internal_s.error`
        #[doc(alias = "llhttp__internal_s.error")]
        #[must_use]
        pub fn error_raw(&self) -> i32 {
            get!(self, error)
        }

        /// The reason of the parser's error, `reason`: llhttp's text for the error or the
        /// pause it stopped at, or the one a callback or a user set; `None` before any.
        ///
        /// It stays set once the error is cleared, as by `llhttp_resume`, until the parser is
        /// initialised again. It is lent for the parser's borrow, which it cannot outlive:
        ///
        /// ```compile_fail,E0597
        /// use ffibox::CZeroable;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let reason = {
        ///     let mut parser = Parser::zeroed();
        ///     parser.as_mut().set_reason(Some(c"Paused"));
        ///     parser.as_ref().reason()
        /// };
        /// let _ = reason;
        /// ```
        ///
        /// Wraps `fld::llhttp__internal_s.reason`
        #[doc(alias = "llhttp__internal_s.reason")]
        /// It is api.c's `llhttp_get_error_reason` too, which returns NULL for `None`.
        ///
        /// Replaces `fn::llhttp_get_error_reason`
        #[doc(alias = "llhttp_get_error_reason")]
        #[must_use]
        pub fn reason(&self) -> Option<&'a CStr> {
            let reason = get!(self, reason);
            if reason.is_null() {
                return None;
            }
            // SAFETY: a non-NULL `reason` is a NUL-terminated string that lives, unchanged,
            // as long as the program, or for as long as the parser holds it where C code set
            // it: llhttp stores only its string literals, `set_reason` only a `&'static CStr`,
            // the callbacks of a table stored by `set_settings_ptr` only strings its contract
            // keeps alive so, those of a `Callback` or `DataCallback` only strings that live
            // as long as the program (`from_fn`'s contract), and a handle built over C's
            // parser vouches for the strings that C set (the handle's invariant). Nothing
            // writes the string, and the parser holds it, unreplaced, for the borrow `'a`.
            Some(unsafe { CStr::from_ptr(reason) })
        }

        /// Where the parser's error is in `data`, from `error_pos`: the offset from `data`'s
        /// start of the byte llhttp stopped at, when `data` is the input of the
        /// `llhttp_execute` call that failed; `None` when the parser has no error position
        /// or it is outside `data`. An offset of `data.len()` is its end.
        ///
        /// `error_pos` points into the input of that call, which may since have been freed,
        /// so it is only compared with `data`'s bounds, never followed: an input that reuses
        /// the old one's addresses gives the offset there.
        ///
        /// Wraps `fld::llhttp__internal_s.error_pos`
        #[doc(alias = "llhttp__internal_s.error_pos")]
        /// It is api.c's `llhttp_get_error_pos` too, whose pointer it locates in `data`
        /// rather than return.
        ///
        /// Replaces `fn::llhttp_get_error_pos`
        #[doc(alias = "llhttp_get_error_pos")]
        #[must_use]
        pub fn error_offset(&self, data: &[u8]) -> Option<usize> {
            let pos = get!(self, error_pos);
            if pos.is_null() {
                return None;
            }
            let offset = pos.addr().checked_sub(data.as_ptr().addr())?;
            (offset <= data.len()).then_some(offset)
        }

        /// The address the parser's user stored in `data`, its own context, which llhttp
        /// carries without reading; NULL when none is stored.
        ///
        /// Wraps `fld::llhttp__internal_s.data`
        #[doc(alias = "llhttp__internal_s.data")]
        #[must_use]
        pub fn data(&self) -> *mut c_void {
            get!(self, data)
        }

        /// The address of the parser's callbacks, `settings`: the `llhttp_settings_t`
        /// `llhttp_init` or a setter stored, which api.c's dispatchers read on every event;
        /// NULL when none is stored, in which case no callback runs.
        ///
        /// It is read as an address, not as a [`SettingsRef`](super::super::api::SettingsRef):
        /// the table may be one C built, stored through
        /// [`set_settings_ptr`](ParserMut::set_settings_ptr), whose callbacks are sound only
        /// with the parsers whose `data` they expect, where a `SettingsRef`'s getters hand out
        /// callbacks callable with any parser.
        ///
        /// Wraps `fld::llhttp__internal_s.settings`
        #[doc(alias = "llhttp__internal_s.settings")]
        #[must_use]
        pub fn settings_ptr(&self) -> *const sys::llhttp_settings_t {
            get!(self, settings).cast_const().cast()
        }

        /// The body length still to read, `content_length`: the `Content-Length` of the
        /// message, counted down as its body is read, and the size of the current chunk of a
        /// chunked one.
        ///
        /// Wraps `fld::llhttp__internal_s.content_length`
        #[doc(alias = "llhttp__internal_s.content_length")]
        #[must_use]
        pub fn content_length(&self) -> u64 {
            get!(self, content_length)
        }

        /// The byte `method` holds: an `llhttp_method`, as `llhttp_get_method` returns it.
        ///
        /// Wraps `fld::llhttp__internal_s.method`
        #[doc(alias = "llhttp__internal_s.method")]
        #[must_use]
        pub fn method_raw(&self) -> u8 {
            get!(self, method)
        }

        /// The major HTTP version of the message, `http_major`.
        ///
        /// Wraps `fld::llhttp__internal_s.http_major`
        #[doc(alias = "llhttp__internal_s.http_major")]
        /// It is api.c's `llhttp_get_http_major` too.
        ///
        /// Replaces `fn::llhttp_get_http_major`
        #[doc(alias = "llhttp_get_http_major")]
        #[must_use]
        pub fn http_major(&self) -> u8 {
            get!(self, http_major)
        }

        /// The minor HTTP version of the message, `http_minor`.
        ///
        /// Wraps `fld::llhttp__internal_s.http_minor`
        #[doc(alias = "llhttp__internal_s.http_minor")]
        /// It is api.c's `llhttp_get_http_minor` too.
        ///
        /// Replaces `fn::llhttp_get_http_minor`
        #[doc(alias = "llhttp_get_http_minor")]
        #[must_use]
        pub fn http_minor(&self) -> u8 {
            get!(self, http_minor)
        }

        /// The lenient flags, `lenient_flags`: the `llhttp_lenient_flags` bits a user set to
        /// relax a check.
        ///
        /// Wraps `fld::llhttp__internal_s.lenient_flags`
        #[doc(alias = "llhttp__internal_s.lenient_flags")]
        #[must_use]
        pub fn lenient_flags_raw(&self) -> u16 {
            get!(self, lenient_flags)
        }

        /// Whether the message upgrades the connection, `upgrade`: set before
        /// `on_headers_complete` runs, for a `CONNECT` request, or one whose `Upgrade` and
        /// `Connection: upgrade` headers ask it, as a `101` response does.
        ///
        /// Wraps `fld::llhttp__internal_s.upgrade`
        #[doc(alias = "llhttp__internal_s.upgrade")]
        /// It is api.c's `llhttp_get_upgrade` too, which returns the byte, `0` or `1`.
        ///
        /// Replaces `fn::llhttp_get_upgrade`
        #[doc(alias = "llhttp_get_upgrade")]
        #[must_use]
        pub fn upgrade(&self) -> bool {
            get!(self, upgrade) != 0
        }

        /// The byte `finish` holds: an `llhttp_finish`, which tells `llhttp_finish` whether
        /// the input may end where the parser is.
        ///
        /// Wraps `fld::llhttp__internal_s.finish`
        #[doc(alias = "llhttp__internal_s.finish")]
        #[must_use]
        pub fn finish_raw(&self) -> u8 {
            get!(self, finish)
        }

        /// The message's flags, `flags`: the `llhttp_flags` bits the parser sets from its
        /// headers, such as `F_CHUNKED` and `F_CONNECTION_CLOSE`.
        ///
        /// Wraps `fld::llhttp__internal_s.flags`
        #[doc(alias = "llhttp__internal_s.flags")]
        #[must_use]
        pub fn flags_raw(&self) -> u16 {
            get!(self, flags)
        }

        /// The status code of the response the parser reads, `status_code`: the three-digit
        /// code it read, whether or not [`Status`](super::Status) names it.
        ///
        /// Wraps `fld::llhttp__internal_s.status_code`
        #[doc(alias = "llhttp__internal_s.status_code")]
        /// It is api.c's `llhttp_get_status_code` too, which returns it as an `int`.
        ///
        /// Replaces `fn::llhttp_get_status_code`
        #[doc(alias = "llhttp_get_status_code")]
        #[must_use]
        pub fn status_code(&self) -> u16 {
            get!(self, status_code)
        }
    }

    impl ParserMut<'_> {
        /// Sets which messages the parser reads, `type`, as `llhttp_init` and `llhttp_reset`
        /// do after `llhttp__internal_init`.
        ///
        /// Wraps `fld::llhttp__internal_s.type`
        #[doc(alias = "llhttp__internal_s.type")]
        pub fn set_parser_type(&mut self, parser_type: ParserType) {
            // Every `llhttp_type` is below 3, so the byte holds it.
            set!(self, type_, parser_type.raw().to_le_bytes()[0]);
        }

        /// Sets the parser's error, `error`: [`Errno::Paused`] pauses it, as `llhttp_pause`
        /// does, and [`Errno::Ok`] clears it, as `llhttp_resume` does. `llhttp_execute`
        /// returns a kept error without parsing.
        ///
        /// Wraps `fld::llhttp__internal_s.error`
        #[doc(alias = "llhttp__internal_s.error")]
        pub fn set_error(&mut self, error: Errno) {
            // Every `llhttp_errno` is below 36, so the `int` holds it.
            set!(self, error, error.raw().cast_signed());
        }

        /// Sets the reason of the parser's error, `reason`, as `llhttp_set_error_reason`
        /// does, or clears it with `None`. The string is `'static`, as llhttp's own are,
        /// since the parser keeps it beyond any borrow:
        ///
        /// ```compile_fail,E0716
        /// use std::ffi::CString;
        ///
        /// use ffibox::CZeroable;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let mut parser = Parser::zeroed();
        /// parser
        ///     .as_mut()
        ///     .set_reason(Some(CString::new("gone").unwrap().as_c_str()));
        /// ```
        ///
        /// Wraps `fld::llhttp__internal_s.reason`
        #[doc(alias = "llhttp__internal_s.reason")]
        /// It is api.c's `llhttp_set_error_reason` too, which a callback calls before it
        /// returns an error.
        ///
        /// Replaces `fn::llhttp_set_error_reason`
        #[doc(alias = "llhttp_set_error_reason")]
        pub fn set_reason(&mut self, reason: Option<&'static CStr>) {
            set!(self, reason, reason.map_or(core::ptr::null(), CStr::as_ptr));
        }

        /// Stores the user's context in `data`, which llhttp carries without reading, and
        /// `llhttp_reset` keeps.
        ///
        /// # Safety
        ///
        /// The callbacks of the parser's settings read `data` as their own context: for as
        /// long as the parser holds both `data` and those settings, `data` must be what they
        /// expect it to be, valid for each access they make through it. The safe API calls
        /// them at any time while the parser holds them, not only while it parses: through
        /// its parses, its `finish` and api.c's dispatchers (`ParserMut::on_url` and the
        /// others), which are safe. A parser whose settings are NULL, or whose callbacks never
        /// read `data`, takes any value; so a context that goes before the parser does is
        /// given up by clearing or replacing those settings first, then `data`.
        ///
        /// Wraps `fld::llhttp__internal_s.data`
        #[doc(alias = "llhttp__internal_s.data")]
        pub unsafe fn set_data(&mut self, data: *mut c_void) {
            set!(self, data, data);
        }

        /// Stores the parser's callbacks in `settings`, as `llhttp_init` does, which
        /// `llhttp_reset` keeps; NULL runs no callback.
        ///
        /// # Safety
        ///
        /// `settings` must be NULL, or point at an `llhttp_settings_t` that stays live and
        /// unchanged, and whose callbacks are sound to call with this parser and its `data`,
        /// for as long as the parser holds it: until another table or NULL replaces it
        /// (this setter, [`set_settings`](Self::set_settings), `init`) or
        /// [`reinit`](Self::reinit) clears it, which `reset` does not, or else for as long as
        /// the parser lives. api.c's dispatchers read it on every event of a parse and of
        /// `finish`, and call each callback it sets; and they are the parser's safe methods
        /// (`ParserMut::on_url` and the others), which read it and call its callbacks at any
        /// time while the parser holds it, in any order and, for a data callback, with any run
        /// of bytes, a parse or none.
        ///
        /// A callback that sets the parser's `reason`, as `llhttp_set_error_reason` lets it,
        /// must set it NULL or to a NUL-terminated string that stays live and unchanged for as
        /// long as the parser holds it: until the reason is replaced or the parser is
        /// initialised again, or else for as long as the parser lives. The safe
        /// [`reason`](ParserRef::reason) reads it at any time, long after the callback ran.
        ///
        /// Wraps `fld::llhttp__internal_s.settings`
        #[doc(alias = "llhttp__internal_s.settings")]
        pub unsafe fn set_settings_ptr(&mut self, settings: *const sys::llhttp_settings_t) {
            set!(self, settings, settings.cast_mut().cast::<c_void>());
        }

        /// Stores the parser's callbacks in `settings`, as `llhttp_init` does, which
        /// `llhttp_reset` keeps; `None` runs no callback.
        ///
        /// The table is lent for `'static`, `SETTINGS.as_ref()` of a `static` or of a leaked
        /// table, since the parser keeps its address however long it lives and wherever it
        /// moves, and api.c's dispatchers read it on every event: a table under a `'static`
        /// shared borrow neither moves, changes nor goes while they may. Its callbacks are
        /// [`Callback`]s and [`DataCallback`]s, each sound with any parser whatever its `data`
        /// holds, so the parser's `data` takes any value beside it.
        ///
        /// ```
        /// use std::cell::Cell;
        /// use std::ffi::c_int;
        /// use std::sync::LazyLock;
        ///
        /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
        ///     Callback, EventHandler, Parsed, Parser, ParserMut,
        /// };
        ///
        /// thread_local!(static COMPLETE: Cell<u32> = const { Cell::new(0) });
        ///
        /// struct Counting;
        ///
        /// impl EventHandler for Counting {
        ///     fn on_event(_: ParserMut<'_>) -> c_int {
        ///         COMPLETE.set(COMPLETE.get() + 1);
        ///         0
        ///     }
        /// }
        ///
        /// static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
        ///     let mut settings = Settings::new();
        ///     settings
        ///         .as_mut()
        ///         .set_on_message_complete(Some(Callback::new::<Counting>()));
        ///     settings
        /// });
        ///
        /// let mut parser = Parser::new();
        /// parser.as_mut().set_settings(Some(SETTINGS.as_ref()));
        /// let responses = b"HTTP/1.1 204 No Content\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n";
        /// assert_eq!(parser.as_mut().parse(responses), Ok(Parsed::All));
        /// assert_eq!(COMPLETE.get(), 2);
        /// ```
        ///
        /// The table cannot be written once a parser holds it:
        ///
        /// ```compile_fail,E0502
        /// use libgit2_crustify::libgit2::deps::llhttp::api::Settings;
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Parser;
        ///
        /// let settings: &'static mut Settings = Box::leak(Box::default());
        /// let mut parser = Parser::new();
        /// parser.as_mut().set_settings(Some(settings.as_ref()));
        /// settings.as_mut().set_on_body(None);
        /// ```
        ///
        /// Wraps `fld::llhttp__internal_s.settings`
        #[doc(alias = "llhttp__internal_s.settings")]
        pub fn set_settings(&mut self, settings: Option<SettingsRef<'static>>) {
            let settings = settings.map_or(core::ptr::null(), |s| s.as_ptr());
            // SAFETY: `settings` is NULL or comes from a `SettingsRef<'static>`, which
            // addresses a table live for the rest of the program, unmoved, and which no
            // exclusive handle writes while that shared borrow exists, which is forever (the
            // handle's invariant), and C never writes: it stays live and unchanged for as long
            // as the parser can execute with it. Each of its slots is NULL or a `Callback` or
            // `DataCallback` (`SettingsRef`'s invariant), sound to call with any live parser
            // whatever its `data` and `settings` hold (`Callback::from_fn`'s contract, which
            // `new`'s trampolines meet), on the thread that executes it.
            unsafe { self.set_settings_ptr(settings.cast()) };
        }

        /// Sets the lenient flags, `lenient_flags`, as `llhttp_set_lenient_headers` and
        /// its siblings set or clear one bit each.
        ///
        /// Wraps `fld::llhttp__internal_s.lenient_flags`
        #[doc(alias = "llhttp__internal_s.lenient_flags")]
        pub fn set_lenient_flags_raw(&mut self, lenient_flags: u16) {
            set!(self, lenient_flags, lenient_flags);
        }

        /// Sets whether the message upgrades the connection, `upgrade`, as
        /// `llhttp__before_headers_complete` does.
        ///
        /// Wraps `fld::llhttp__internal_s.upgrade`
        #[doc(alias = "llhttp__internal_s.upgrade")]
        pub fn set_upgrade(&mut self, upgrade: bool) {
            set!(self, upgrade, u8::from(upgrade));
        }

        /// Sets the byte `finish` holds, as `llhttp__after_message_complete` sets
        /// `HTTP_FINISH_SAFE`. `llhttp_finish` aborts on a value that is no `llhttp_finish`.
        ///
        /// Wraps `fld::llhttp__internal_s.finish`
        #[doc(alias = "llhttp__internal_s.finish")]
        pub fn set_finish_raw(&mut self, finish: u8) {
            set!(self, finish, finish);
        }

        /// Sets the message's flags, `flags`, as `llhttp__after_message_complete` clears
        /// them.
        ///
        /// Wraps `fld::llhttp__internal_s.flags`
        #[doc(alias = "llhttp__internal_s.flags")]
        pub fn set_flags_raw(&mut self, flags: u16) {
            set!(self, flags, flags);
        }
    }

    thread_local! {
        /// The addresses of the parsers this thread is inside `llhttp__internal_execute` for,
        /// innermost last: their callbacks, which run on the parsing thread, may parse other
        /// parsers, but neither parse nor initialise one of these.
        static EXECUTING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    /// A parser this thread is executing, registered for as long as the value lives.
    struct Executing(usize);

    impl Executing {
        /// Registers the parser at `addr` as executing on this thread.
        ///
        /// # Panics
        ///
        /// When it is already: a callback of its own parse is parsing it again.
        fn enter(addr: usize) -> Self {
            EXECUTING.with_borrow_mut(|executing| {
                assert!(
                    !executing.contains(&addr),
                    "a callback parsed the llhttp parser whose parse runs it"
                );
                executing.push(addr);
            });
            Self(addr)
        }

        /// Refuses to change the state of the parser at `addr` under its own parse.
        ///
        /// # Panics
        ///
        /// When the parser is executing on this thread: a callback of its parse is
        /// initialising it.
        fn refuse(addr: usize) {
            EXECUTING.with_borrow(|executing| {
                assert!(
                    !executing.contains(&addr),
                    "a callback initialised the llhttp parser whose parse runs it"
                );
            });
        }
    }

    impl Drop for Executing {
        fn drop(&mut self) {
            // Registrations nest as the parses that make them do, so this one is the last.
            let last = EXECUTING.with_borrow_mut(Vec::pop);
            debug_assert_eq!(last, Some(self.0), "llhttp parses registered out of order");
        }
    }

    impl Parser {
        /// A parser ready to read its first message: all-zero but for the start state, as
        /// `llhttp__internal_init` leaves it. It reads requests or responses, whichever comes
        /// first ([`ParserType::Both`]), has no error, no reason and no lenient flags, and its
        /// `data` and `settings` are NULL, so that a parse runs no callback.
        ///
        /// ```
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Errno, Parsed, Parser, ParserType};
        ///
        /// let mut parser = Parser::new();
        /// assert_eq!(parser.as_ref().parser_type(), Ok(ParserType::Both));
        /// assert_eq!(parser.as_ref().error(), Ok(Errno::Ok));
        /// let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        /// assert_eq!(parser.as_mut().parse(response), Ok(Parsed::All));
        /// assert_eq!(parser.as_ref().status_code(), 204);
        /// ```
        ///
        /// Wraps `fn::llhttp__internal_init`
        #[doc(alias = "llhttp__internal_init")]
        #[must_use]
        pub fn new() -> Self {
            let mut parser = <Self as CZeroable>::zeroed();
            init(parser.as_mut().as_mut_ptr());
            parser
        }
    }

    impl Default for Parser {
        /// [`Parser::new`].
        fn default() -> Self {
            Self::new()
        }
    }

    /// Runs `llhttp__internal_init` on the parser `parser` addresses.
    fn init(parser: *mut sys::llhttp__internal_t) {
        // SAFETY: `parser` comes from an exclusive handle, so it addresses a live
        // `llhttp__internal_s` nothing else accesses for the call. `llhttp__internal_init`
        // only `memset`s it and writes its start state into `_current`, keeping no pointer;
        // it returns 0 on its one path, so its result reports nothing.
        unsafe { sys::llhttp__internal_init(parser) };
    }

    impl ParserMut<'_> {
        /// Initialises the parser again, in place, as [`Parser::new`] builds one: every
        /// field zero but the start state, its type, error, reason, flags, `data` and
        /// `settings` included. `llhttp_init` and `llhttp_reset` start so, and set back the
        /// fields they keep.
        ///
        /// # Panics
        ///
        /// When a callback of the parser's own [`parse`](Self::parse) calls it, which would
        /// leave the outer parse resuming from a state it did not leave.
        ///
        /// Wraps `fn::llhttp__internal_init`
        #[doc(alias = "llhttp__internal_init")]
        pub fn reinit(&mut self) {
            let parser = self.as_mut_ptr();
            Executing::refuse(parser.addr());
            init(parser);
        }

        /// Parses `data`, a run of the input, calling the callbacks of the parser's settings
        /// as it reads each part of a message: llhttp's generated parser.
        ///
        /// The parser picks up where the previous run stopped, so a message may arrive in
        /// any number of runs. It keeps no pointer into `data` that it follows after the
        /// call: a span the run leaves open is passed to its callback before the call
        /// returns, and its error position is only compared with an input, never followed
        /// ([`ParserRef::error_offset`]).
        ///
        /// ```
        /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{Errno, Parsed, Parser};
        ///
        /// let mut parser = Parser::new();
        /// assert_eq!(
        ///     parser.as_mut().parse(b"HTTP/1.1 200 OK\r\nContent-"),
        ///     Ok(Parsed::All)
        /// );
        /// assert_eq!(
        ///     parser.as_mut().parse(b"Length: 2\r\n\r\nhi"),
        ///     Ok(Parsed::All)
        /// );
        /// assert_eq!(parser.as_ref().status_code(), 200);
        ///
        /// let error = parser.as_mut().parse(b"garbage").unwrap_err();
        /// assert_eq!(error.errno(), Ok(Errno::InvalidConstant));
        /// // The error is kept until the parser is initialised again.
        /// assert_eq!(parser.as_mut().parse(b"HTTP/1.1 200 OK\r\n"), Err(error));
        /// parser.as_mut().reinit();
        /// assert_eq!(
        ///     parser.as_mut().parse(b"HTTP/1.1 200 OK\r\n"),
        ///     Ok(Parsed::All)
        /// );
        /// ```
        ///
        /// # Errors
        ///
        /// The [`ParseError`] the parser stopped at, which it keeps: every later parse
        /// returns it again without reading, until the parser is initialised again.
        ///
        /// # Panics
        ///
        /// When the parser cannot run, where C would abort: it was never initialised, as
        /// [`CZeroable::zeroed`] builds it, or its error was cleared after one that leaves
        /// it nothing to resume from, such as `HPE_PAUSED_H2_UPGRADE`. And when a callback
        /// of the parser's own parse parses it again, which would corrupt the state the
        /// outer parse resumes from.
        ///
        /// Wraps `fn::llhttp__internal_execute`
        #[doc(alias = "llhttp__internal_execute")]
        pub fn parse(&mut self, data: &[u8]) -> Result<Parsed, ParseError> {
            let parser = self.as_mut_ptr();
            let _executing = Executing::enter(parser.addr());
            let runnable = {
                let this = self.as_ref();
                this.error_raw() != 0 || !get!(this, _current).is_null()
            };
            assert!(
                runnable,
                "the llhttp parser has no state to run from: it was never initialised, \
                 or was cleared of an error it cannot resume from"
            );
            let range = data.as_ptr_range();
            // SAFETY: `parser` comes from this exclusive handle, a live parser nothing else
            // accesses while the call runs but the callbacks C hands it to. Its private state
            // is C's own, from `llhttp__internal_init` or an earlier run: no accessor writes
            // `_current`, `_index` or the span fields. It is either in error, which the call
            // returns without running, or has a state to run from (checked above), so C's
            // `abort` on `s_error` is not reached. `range` bounds `data`, which is live and
            // unchanged for the call; C reads only within it, keeps `_span_pos0` and
            // `error_pos` pointing into it, and never follows either after the call: the
            // next run restarts an open span at its own input, and `error_pos` is never
            // followed by Rust. The settings' callbacks are NULL, or sound to call with this
            // parser and its `data` (`set_settings_ptr`'s and `set_data`'s contracts), and
            // none of them parses or initialises the parser again from Rust (`Executing`).
            let code = unsafe {
                sys::llhttp__internal_execute(parser, range.start.cast(), range.end.cast())
            };
            ParseError::outcome(code)
        }
    }

    /// `llhttp_cb`'s function type: what a [`Callback`] holds and `llhttp.h`'s settings
    /// slots point to.
    type CallbackFn = unsafe extern "C" fn(*mut sys::llhttp_t) -> c_int;

    /// `llhttp_data_cb`'s function type: what a [`DataCallback`] holds and `llhttp.h`'s
    /// settings slots point to.
    type DataCallbackFn = unsafe extern "C" fn(*mut sys::llhttp_t, *const c_char, usize) -> c_int;

    /// An event callback (`llhttp_cb`): the C function a parser's settings name for an
    /// event of a message, such as its start or the end of its headers, which C calls with
    /// the parser being parsed.
    ///
    /// Built over an [`EventHandler`] with [`new`](Self::new), whose `extern "C"`
    /// trampoline hands the handler the parser's exclusive handle and catches its panics,
    /// or, inside the crate, over a C function, which its `unsafe` constructor vouches is
    /// sound to call with any parser. Either way [`call`](Self::call) is safe. A function
    /// pointer and nothing else: `Copy`, owning nothing, never released, and callable from
    /// any thread.
    ///
    /// ```
    /// use std::ffi::c_int;
    ///
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    ///     Callback, EventHandler, Parser, ParserMut,
    /// };
    ///
    /// struct Upgrading;
    ///
    /// impl EventHandler for Upgrading {
    ///     fn on_event(mut parser: ParserMut<'_>) -> c_int {
    ///         parser.set_upgrade(true);
    ///         0
    ///     }
    /// }
    ///
    /// const CALLBACK: Callback = Callback::new::<Upgrading>();
    /// let mut parser = Parser::new();
    /// assert_eq!(CALLBACK.call(parser.as_mut()), 0);
    /// assert!(parser.as_ref().upgrade());
    /// assert!(CALLBACK.raw().is_some());
    /// ```
    ///
    /// Its function is private: only `new` builds one outside the crate.
    ///
    /// ```compile_fail,E0423
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::Callback;
    /// use libgit2_crustify::sys::libgit2 as sys;
    ///
    /// unsafe extern "C" fn c_callback(_: *mut sys::llhttp_t) -> std::ffi::c_int {
    ///     0
    /// }
    /// let _ = Callback(c_callback);
    /// ```
    ///
    /// Wraps `fnp::llhttp_cb`
    #[doc(alias = "llhttp_cb")]
    #[derive(Clone, Copy)]
    pub struct Callback(CallbackFn);

    impl Callback {
        /// The callback that hands C's calls to `H`.
        #[must_use]
        pub const fn new<H: EventHandler>() -> Self {
            Self(event_with::<H>)
        }

        /// The callback over a C function.
        ///
        /// # Safety
        ///
        /// For as long as this value or a copy of it lives, `f` must be sound to call with
        /// every live, exclusively held parser, in any state and whatever its `data` and
        /// `settings` hold, on the thread that holds the parser, and must not unwind: those
        /// are the calls [`call`](Self::call) makes, and those of the parsers a
        /// [`Settings`](crate::libgit2::deps::llhttp::api::Settings) holding it is given. Where
        /// it sets the parser's `reason`, it sets it NULL or to a NUL-terminated string that
        /// lives, unchanged, as long as the program, since it may serve any parser, which
        /// [`ParserRef::reason`] reads at any time. A C
        /// callback that reads the parser's `data` as its own context does not meet this,
        /// and is reached through the settings its parser executes with instead, stored by
        /// [`ParserMut::set_settings_ptr`], whose contract vouches for it.
        pub(crate) const unsafe fn from_fn(f: CallbackFn) -> Self {
            Self(f)
        }

        /// Calls the callback with `parser`, as llhttp does, and returns what it returns.
        #[must_use = "the callback's return tells the parse how to go on"]
        pub fn call(self, mut parser: ParserMut<'_>) -> c_int {
            // SAFETY: `parser` addresses a live parser this exclusive handle lends for the
            // call: `CallbackFn`'s only argument. `new`'s trampolines need nothing more, and
            // `from_fn`'s caller vouched that the function is sound with any such parser.
            unsafe { (self.0)(parser.as_mut_ptr()) }
        }

        /// The C value, as a settings slot holds it: never NULL.
        #[must_use]
        pub const fn raw(self) -> sys::llhttp_cb {
            Some(self.0)
        }
    }

    impl fmt::Debug for Callback {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_tuple("Callback")
                .field(&(self.0 as *const ()))
                .finish()
        }
    }

    /// `llhttp_cb` over [`EventHandler::on_event`].
    ///
    /// # Safety
    ///
    /// `parser` is NULL or addresses a live parser nothing else accesses for the call but
    /// the handles this call makes: llhttp's call with the parser it is parsing, and
    /// [`Callback::call`]'s.
    unsafe extern "C" fn event_with<H: EventHandler>(parser: *mut sys::llhttp_t) -> c_int {
        // SAFETY: per this function's contract, a non-NULL `parser` is a live parser lent
        // to the call alone; the handle's borrow ends with the call.
        let Some(parser) = (unsafe { ParserMut::from_ptr(parser) }) else {
            return -1;
        };
        callback::barrier(|| H::on_event(parser), || -1)
    }

    /// A data callback (`llhttp_data_cb`): the C function a parser's settings name for a
    /// run of the input that makes up a part of a message, such as its URL, a header's
    /// name or value, or its body, which C calls with the parser and the run.
    ///
    /// Built over a [`DataHandler`] with [`new`](Self::new), whose trampoline hands the
    /// handler the parser's handle and the run as a slice borrowed for the call, or, inside
    /// the crate, over a C function, as [`Callback`] is. A part may come in several runs,
    /// one per call, when it spans several parses. A function pointer and nothing else.
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::ffi::c_int;
    ///
    /// use libgit2_crustify::libgit2::deps::llhttp::llhttp::{DataCallback, DataHandler, Parser, ParserMut};
    ///
    /// thread_local!(static SEEN: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) });
    ///
    /// struct Collect;
    ///
    /// impl DataHandler for Collect {
    ///     fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
    ///         SEEN.with_borrow_mut(|seen| seen.extend_from_slice(data));
    ///         0
    ///     }
    /// }
    ///
    /// const CALLBACK: DataCallback = DataCallback::new::<Collect>();
    /// let mut parser = Parser::new();
    /// assert_eq!(CALLBACK.call(parser.as_mut(), b"Host"), 0);
    /// assert_eq!(SEEN.with_borrow(Vec::clone), b"Host");
    /// ```
    ///
    /// Wraps `fnp::llhttp_data_cb`
    #[doc(alias = "llhttp_data_cb")]
    #[derive(Clone, Copy)]
    pub struct DataCallback(DataCallbackFn);

    impl DataCallback {
        /// The callback that hands C's calls to `H`.
        #[must_use]
        pub const fn new<H: DataHandler>() -> Self {
            Self(data_with::<H>)
        }

        /// The callback over a C function.
        ///
        /// # Safety
        ///
        /// As [`Callback::from_fn`]'s, the calls being [`call`](Self::call)'s: with every
        /// live, exclusively held parser, and a run of bytes live and unchanged for the
        /// call, its start and its length.
        pub(crate) const unsafe fn from_fn(f: DataCallbackFn) -> Self {
            Self(f)
        }

        /// Calls the callback with `parser` and `data`, as llhttp does, and returns what it
        /// returns.
        #[must_use = "the callback's return tells the parse how to go on"]
        pub fn call(self, mut parser: ParserMut<'_>, data: &[u8]) -> c_int {
            // SAFETY: `parser` addresses a live parser this exclusive handle lends for the
            // call, and `data` a run of `data.len()` bytes live and unchanged for it:
            // `DataCallbackFn`'s arguments. `new`'s trampolines need nothing more, and
            // `from_fn`'s caller vouched for the function with any such arguments.
            unsafe { (self.0)(parser.as_mut_ptr(), data.as_ptr().cast(), data.len()) }
        }

        /// The C value, as a settings slot holds it: never NULL.
        #[must_use]
        pub const fn raw(self) -> sys::llhttp_data_cb {
            Some(self.0)
        }
    }

    impl fmt::Debug for DataCallback {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_tuple("DataCallback")
                .field(&(self.0 as *const ()))
                .finish()
        }
    }

    /// `llhttp_data_cb` over [`DataHandler::on_data`].
    ///
    /// # Safety
    ///
    /// As [`event_with`]'s for `parser`; and `at` is NULL or addresses `length` bytes live
    /// and unchanged for the call: llhttp's span, from its start to where it read, within
    /// the input of the parse, or [`DataCallback::call`]'s slice.
    unsafe extern "C" fn data_with<H: DataHandler>(
        parser: *mut sys::llhttp_t,
        at: *const c_char,
        length: usize,
    ) -> c_int {
        // SAFETY: per this function's contract, a non-NULL `parser` is a live parser lent
        // to the call alone; the handle's borrow ends with the call.
        let Some(parser) = (unsafe { ParserMut::from_ptr(parser) }) else {
            return -1;
        };
        let data: &[u8] = if length == 0 {
            &[]
        } else if at.is_null() {
            return -1;
        } else {
            // SAFETY: per this function's contract, a non-NULL `at` addresses `length`
            // initialised bytes, which nothing writes or frees for the call, the slice's
            // borrow; `u8` is aligned to 1.
            unsafe { core::slice::from_raw_parts(at.cast::<u8>(), length) }
        };
        callback::barrier(|| H::on_data(parser, data), || -1)
    }
}
