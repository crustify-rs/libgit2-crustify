//! Equivalence tests for `deps/llhttp/api.c`'s settings table against the C.
//!
//! [`Settings::new`] is compared with `llhttp_settings_init` byte for byte, from a table
//! filled with garbage, and a table filled through [`SettingsMut`]'s setters with one C
//! fills field by field, slot for slot. [`ParserMut::set_settings`] is compared with
//! `llhttp_init`: two parsers given one table, one by C and one through the safe setter,
//! are compared whole after the call, after `llhttp_reset`, and by the callbacks C's
//! dispatchers run for each of a sequence of runs, `llhttp_finish` included, the two
//! parsers' outcomes and fields compared after each run.

use std::cell::RefCell;
use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::sync::Mutex;

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::api::{Settings, SettingsRef};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, EventHandler, Parsed, Parser, ParserMut, ParserRef,
    ParserType,
};
use libgit2_crustify::sys::libgit2 as sys;

/// The table's 23 slots as the addresses they hold, read through its raw layout.
fn raw_slots(settings: SettingsRef<'_>) -> [usize; 23] {
    const { assert!(size_of::<sys::llhttp_settings_t>() == size_of::<[usize; 23]>()) };
    // SAFETY: the handle addresses a live table; `llhttp_settings_s` is 23 nullable function
    // pointers, each the size and alignment of a `usize`, with no padding (asserted above),
    // and reading their bits as integers forms no function pointer.
    unsafe { settings.as_ptr().cast::<[usize; 23]>().read() }
}

/// The parser's bytes, private state included: every field is an integer or a pointer, and
/// `llhttp__internal_s` has no padding but what `memset` and the field writes cover.
fn raw_parser(parser: ParserRef<'_>) -> sys::llhttp__internal_s {
    // SAFETY: the handle addresses a live parser; `llhttp__internal_s` is `Copy`, its fields
    // integers and pointers, so a copy of it forms nothing it does not hold.
    unsafe { parser.as_ptr().read() }
}

/// `parser`'s fields that a parse writes, compared as integers and addresses, `data` and
/// `settings` included.
fn fields(p: &sys::llhttp__internal_s) -> impl PartialEq + std::fmt::Debug + use<> {
    (
        (p._index, p._span_pos0.addr(), p._span_cb0.addr()),
        (p.error, p.reason.addr(), p.error_pos.addr()),
        (p.data.addr(), p._current.addr(), p.content_length, p.type_),
        (p.method, p.http_major, p.http_minor, p.header_state),
        (p.lenient_flags, p.upgrade, p.finish, p.flags, p.status_code),
        (p.initial_message_completed, p.settings.addr()),
    )
}

thread_local! {
    /// The slots C's dispatchers called, by index, with each data slot's run.
    static LOG: RefCell<Vec<(usize, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

/// Records that slot `N` was called, and returns `0`.
struct Event<const N: usize>;

impl<const N: usize> EventHandler for Event<N> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        LOG.with_borrow_mut(|log| log.push((N, Vec::new())));
        0
    }
}

/// Records that slot `N` was called with `data`, and returns `0`.
struct Data<const N: usize>;

impl<const N: usize> DataHandler for Data<N> {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        LOG.with_borrow_mut(|log| log.push((N, data.to_vec())));
        0
    }
}

/// Calls `$m!(index, field, setter, kind)` for each slot, in `llhttp_settings_s`' order.
macro_rules! for_each_slot {
    ($m:ident) => {
        $m!(0, on_message_begin, set_on_message_begin, event);
        $m!(1, on_url, set_on_url, data);
        $m!(2, on_status, set_on_status, data);
        $m!(3, on_method, set_on_method, data);
        $m!(4, on_version, set_on_version, data);
        $m!(5, on_header_field, set_on_header_field, data);
        $m!(6, on_header_value, set_on_header_value, data);
        $m!(
            7,
            on_chunk_extension_name,
            set_on_chunk_extension_name,
            data
        );
        $m!(
            8,
            on_chunk_extension_value,
            set_on_chunk_extension_value,
            data
        );
        $m!(9, on_headers_complete, set_on_headers_complete, event);
        $m!(10, on_body, set_on_body, data);
        $m!(11, on_message_complete, set_on_message_complete, event);
        $m!(12, on_url_complete, set_on_url_complete, event);
        $m!(13, on_status_complete, set_on_status_complete, event);
        $m!(14, on_method_complete, set_on_method_complete, event);
        $m!(15, on_version_complete, set_on_version_complete, event);
        $m!(
            16,
            on_header_field_complete,
            set_on_header_field_complete,
            event
        );
        $m!(
            17,
            on_header_value_complete,
            set_on_header_value_complete,
            event
        );
        $m!(
            18,
            on_chunk_extension_name_complete,
            set_on_chunk_extension_name_complete,
            event
        );
        $m!(
            19,
            on_chunk_extension_value_complete,
            set_on_chunk_extension_value_complete,
            event
        );
        $m!(20, on_chunk_header, set_on_chunk_header, event);
        $m!(21, on_chunk_complete, set_on_chunk_complete, event);
        $m!(22, on_reset, set_on_reset, event);
    };
}

/// The same slots of two tables, `rust` through the setters and `c` as C fills its fields:
/// each slot whose bit is set in `mask` records.
fn fill_both(mask: u32, rust: &mut Settings, c: &mut sys::llhttp_settings_t) {
    let mut m = rust.as_mut();
    macro_rules! fill {
        ($i:expr, $field:ident, $set:ident,event) => {
            if mask & (1 << $i) != 0 {
                let callback = Callback::new::<Event<$i>>();
                m.$set(Some(callback));
                c.$field = callback.raw();
            }
        };
        ($i:expr, $field:ident, $set:ident,data) => {
            if mask & (1 << $i) != 0 {
                let callback = DataCallback::new::<Data<$i>>();
                m.$set(Some(callback));
                c.$field = callback.raw();
            }
        };
    }
    for_each_slot!(fill);
}

/// A table that lives as long as the test process, for the parsers given it: leaked, its
/// address kept in a global so that the leak checker the address sanitizer runs at exit finds it
/// still reachable.
fn kept() -> &'static mut Settings {
    static KEPT: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let settings: &'static mut Settings = Box::leak(Box::default());
    KEPT.lock()
        .unwrap()
        .push(std::ptr::from_mut(settings).addr());
    settings
}

/// A table C initialised with `llhttp_settings_init`, from garbage.
fn c_settings_init() -> sys::llhttp_settings_t {
    let mut c = MaybeUninit::<sys::llhttp_settings_t>::uninit();
    // SAFETY: `c` is writable storage of the table's size; the bytes are only overwritten.
    unsafe {
        c.as_mut_ptr()
            .cast::<u8>()
            .write_bytes(0xA5, size_of::<sys::llhttp_settings_t>());
    };
    // SAFETY: `c` is a table's writable storage, which `llhttp_settings_init` `memset`s whole.
    unsafe { sys::llhttp_settings_init(c.as_mut_ptr()) };
    // SAFETY: `llhttp_settings_init` wrote every byte: all-zero, every slot NULL.
    unsafe { c.assume_init() }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn new_leaves_the_table_as_llhttp_settings_init_does() {
    let c = c_settings_init();
    let rust = Settings::new();
    // SAFETY: `c` is a live table Rust owns, read for the handle's borrow.
    let c_ref = unsafe { SettingsRef::from_ptr((&raw const c).cast_mut()) }.unwrap();
    assert_eq!(raw_slots(rust.as_ref()), raw_slots(c_ref));
    assert_eq!(raw_slots(rust.as_ref()), [0; 23]);
    assert_eq!(raw_slots(Settings::zeroed().as_ref()), raw_slots(c_ref));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_setters_fill_the_table_as_c_fills_its_fields() {
    // Every slot alone, then a few combinations, then all of them.
    let masks = (0..23)
        .map(|i| 1u32 << i)
        .chain([0x55_5555, 0x2A_AAAA, (1 << 23) - 1]);
    for mask in masks {
        let mut rust = Settings::new();
        let mut c = c_settings_init();
        fill_both(mask, &mut rust, &mut c);
        // SAFETY: `c` is a live table Rust owns, filled only with `Callback` and
        // `DataCallback` values, as a `SettingsRef`'s invariant asks.
        let c_ref = unsafe { SettingsRef::from_ptr((&raw const c).cast_mut()) }.unwrap();
        assert_eq!(raw_slots(rust.as_ref()), raw_slots(c_ref), "{mask:#x}");
        // A table C filled reads back through the getters as the one the setters filled.
        assert_eq!(format!("{:?}", rust.as_ref()), format!("{c_ref:?}"));
    }
}

/// Two parsers given one table: `c` by `llhttp_init`, `rust` by `Parser::new` and the safe
/// setters.
fn both_given(parser_type: ParserType, settings: &'static Settings) -> (Parser, Parser) {
    let mut c = Parser::zeroed();
    // SAFETY: the handle addresses `c`, a live parser Rust owns, which `llhttp_init` fills in
    // place; `settings` is `'static`, and its slots are Rust handlers sound with any parser.
    unsafe {
        sys::llhttp_init(
            c.as_mut().as_mut_ptr(),
            parser_type.raw(),
            settings.as_ref().as_ptr(),
        );
    };
    let mut rust = Parser::new();
    rust.as_mut().set_parser_type(parser_type);
    rust.as_mut().set_settings(Some(settings.as_ref()));
    (c, rust)
}

/// A kept table whose slots in `mask` record.
fn leaked(mask: u32) -> &'static Settings {
    let rust: &'static mut Settings = kept();
    let mut c = c_settings_init();
    fill_both(mask, rust, &mut c);
    rust
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_settings_leaves_the_parser_as_llhttp_init_does() {
    for parser_type in [ParserType::Both, ParserType::Request, ParserType::Response] {
        let settings = leaked((1 << 23) - 1);
        let (mut c, mut rust) = both_given(parser_type, settings);
        assert_eq!(
            fields(&raw_parser(rust.as_ref())),
            fields(&raw_parser(c.as_ref()))
        );
        // `llhttp_reset` keeps the table either way.
        for parser in [&mut c, &mut rust] {
            // SAFETY: the parser is initialised, and its table is `'static`.
            unsafe { sys::llhttp_reset(parser.as_mut().as_mut_ptr()) };
            assert_eq!(
                parser.as_ref().settings_ptr().addr(),
                std::ptr::from_ref(settings).addr()
            );
        }
        assert_eq!(
            fields(&raw_parser(rust.as_ref())),
            fields(&raw_parser(c.as_ref()))
        );
    }
}

/// What a run returned, and the slots it called.
type Step = (u32, Vec<(usize, Vec<u8>)>);

/// `llhttp_finish` on `parser`, which calls `on_message_complete` through its table.
fn c_finish(parser: &mut Parser) -> Step {
    // SAFETY: `parser` is initialised, and its table is `'static` and holds only Rust
    // handlers.
    let errno = unsafe { sys::llhttp_finish(parser.as_mut().as_mut_ptr()) };
    (errno, LOG.take())
}

/// Feeds `runs` to `parser` through `llhttp_execute`, then finishes it.
fn c_runs(parser: &mut Parser, runs: &[&[u8]]) -> Vec<Step> {
    let mut out = Vec::new();
    for run in runs {
        // SAFETY: `parser` is initialised, `run` is `run.len()` live bytes, and its table is
        // `'static` and holds only Rust handlers.
        let errno = unsafe {
            sys::llhttp_execute(parser.as_mut().as_mut_ptr(), run.as_ptr().cast(), run.len())
        };
        out.push((errno, LOG.take()));
    }
    out.push(c_finish(parser));
    out
}

/// Feeds `runs` to `parser` through [`ParserMut::parse`], then finishes it through C.
fn rust_runs(parser: &mut Parser, runs: &[&[u8]]) -> Vec<Step> {
    let mut out = Vec::new();
    for run in runs {
        let errno = match parser.as_mut().parse(run) {
            Ok(Parsed::All) => sys::HPE_OK,
            Ok(Parsed::Paused) => sys::HPE_PAUSED,
            Ok(Parsed::PausedUpgrade) => sys::HPE_PAUSED_UPGRADE,
            Err(error) => error.code().cast_unsigned(),
        };
        out.push((errno, LOG.take()));
    }
    out.push(c_finish(parser));
    out
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_table_given_by_set_settings_runs_as_one_given_by_llhttp_init() {
    let cases: [(ParserType, u32, &[&[u8]]); 4] = [
        (
            ParserType::Response,
            (1 << 23) - 1,
            &[
                b"HTTP/1.1 200 OK\r\nContent-",
                b"Length: 3\r\nX: y\r\n\r\nab",
                b"cHTTP/1.1 204 No Content\r\n\r\n",
            ],
        ),
        (
            ParserType::Request,
            0x55_5555,
            &[
                b"POST /u?q HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n2;e=f\r\nhi\r\n",
                b"0\r\n\r\n",
            ],
        ),
        (
            ParserType::Both,
            0x2A_AAAA,
            &[
                b"GET / HTTP/1.1\r\nHost: h\r\n\r\n",
                b"HTTP/1.1 304 X\r\n\r\n",
            ],
        ),
        // A response read to the end of its connection, which `llhttp_finish` completes.
        (
            ParserType::Response,
            (1 << 23) - 1,
            &[b"HTTP/1.0 200 OK\r\n\r\nto the end"],
        ),
    ];
    for (parser_type, mask, runs) in cases {
        let settings = leaked(mask);
        let (mut c, mut rust) = both_given(parser_type, settings);
        let c_out = c_runs(&mut c, runs);
        let rust_out = rust_runs(&mut rust, runs);
        assert_eq!(rust_out, c_out, "{parser_type:?} {mask:#x}");
        assert!(c_out.iter().any(|(_, log)| !log.is_empty()));
        assert_eq!(
            fields(&raw_parser(rust.as_ref())),
            fields(&raw_parser(c.as_ref()))
        );
    }
}
