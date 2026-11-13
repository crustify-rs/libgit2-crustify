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
//! also sets the starting state, which the binding of `llhttp_init` gives.
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
//! program, and Rust sets only to a `'static` one, so it is lent for the handle's borrow.
//! The error position points into the input of the `llhttp_execute` call that failed, which
//! the caller may since have freed, so it is never followed: [`ParserRef::error_offset`]
//! compares it with the input it is given. `data` and `settings` are read as the addresses
//! they hold, and written only through `unsafe` setters, since the callbacks read them as
//! the types they expect, beyond any lifetime the parser could carry.
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

use core::fmt;

pub use self::ffi_import::{Parser, ParserMut, ParserRef};
use crate::sys::libgit2 as sys;

/// Declares a closed enum of `llhttp.h` over its bindgen constants: each variant's
/// discriminant is its constant, `from_raw` maps the C integer back to the variant carrying
/// it, and `raw` gives the integer C takes. The second item is the error `TryFrom` reports,
/// carrying the integer the enum does not name.
macro_rules! llhttp_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident: $raw:ty {
            $($(#[$vmeta:meta])* $variant:ident = $c:path,)*
        }
        $(#[$umeta:meta])*
        pub struct $unnamed:ident($what:literal);
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$vmeta])* $variant = $c,)*
        }

        impl $name {
            /// The variant whose C value is `raw`; `None` for a value `llhttp.h` does not
            /// name.
            #[must_use]
            pub const fn from_raw(raw: $raw) -> Option<Self> {
                match raw {
                    $($c => Some(Self::$variant),)*
                    _ => None,
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
        MSearch = sys::HTTP_MSEARCH,
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
    pub fn method(&self) -> Result<Method, UnnamedMethod> {
        Method::try_from(u32::from(self.method_raw()))
    }
}

#[expect(
    unsafe_code,
    reason = "reads and writes llhttp__internal_s's fields through its handles' pointers"
)]
mod ffi_import {
    use core::ffi::{CStr, c_void};

    use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};

    use super::{Errno, ParserType};
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
        #[must_use]
        pub fn reason(&self) -> Option<&'a CStr> {
            let reason = get!(self, reason);
            if reason.is_null() {
                return None;
            }
            // SAFETY: a non-NULL `reason` is a NUL-terminated string that lives, unchanged,
            // as long as the program, or as the parser where C code set it: llhttp stores only
            // its string literals, `set_reason` only a `&'static CStr`, and a handle built
            // over C's parser vouches for the strings that C set (the handle's invariant).
            // Nothing writes the string, and it outlives the parser's borrow `'a`.
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
        /// `llhttp_init` stored, which api.c's dispatchers read on every event; NULL when
        /// none is stored, in which case no callback runs.
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
        #[must_use]
        pub fn http_major(&self) -> u8 {
            get!(self, http_major)
        }

        /// The minor HTTP version of the message, `http_minor`.
        ///
        /// Wraps `fld::llhttp__internal_s.http_minor`
        #[doc(alias = "llhttp__internal_s.http_minor")]
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
        pub fn set_reason(&mut self, reason: Option<&'static CStr>) {
            set!(self, reason, reason.map_or(core::ptr::null(), CStr::as_ptr));
        }

        /// Stores the user's context in `data`, which llhttp carries without reading, and
        /// `llhttp_reset` keeps.
        ///
        /// # Safety
        ///
        /// The callbacks of the parser's settings read `data` as their own context: while
        /// the parser executes with those settings, `data` must be what they expect it to
        /// be, valid for each access they make through it. A parser whose settings are NULL,
        /// or whose callbacks never read `data`, takes any value.
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
        /// for as long as the parser executes with it: api.c's dispatchers read it on every
        /// event of `llhttp_execute` and `llhttp_finish`, and call each callback it sets.
        ///
        /// Wraps `fld::llhttp__internal_s.settings`
        #[doc(alias = "llhttp__internal_s.settings")]
        pub unsafe fn set_settings_ptr(&mut self, settings: *const sys::llhttp_settings_t) {
            set!(self, settings, settings.cast_mut().cast::<c_void>());
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
}
