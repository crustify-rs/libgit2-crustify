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
//! [`Callback`]: super::llhttp::Callback
//! [`DataCallback`]: super::llhttp::DataCallback
//! [`ParserMut::set_settings`]: super::llhttp::ParserMut::set_settings
//! [`ParserMut::set_settings_ptr`]: super::llhttp::ParserMut::set_settings_ptr
//! [`ParserRef::settings_ptr`]: super::llhttp::ParserRef::settings_ptr

// The workspace does not set `unsafe_code` yet; this module confines its unsafe code to
// `ffi_import`, which takes the lint back, and holds it to that here.
#![warn(unsafe_code)]

pub use self::ffi_import::{Settings, SettingsMut, SettingsRef};

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

#[expect(
    unsafe_code,
    reason = "reads and writes llhttp_settings_s's slots through its handles' pointers, calls \
              llhttp_settings_init, and asserts the table's `Send` and `Sync`"
)]
mod ffi_import {
    use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};

    use crate::libgit2::deps::llhttp::llhttp::{Callback, DataCallback};
    use crate::sys::libgit2 as sys;

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
