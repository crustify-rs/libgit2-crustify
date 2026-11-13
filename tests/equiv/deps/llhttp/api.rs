//! Equivalence tests for `deps/llhttp/api.c`'s settings table against the C.
//!
//! [`Settings::new`] is compared with `llhttp_settings_init` byte for byte, from a table
//! filled with garbage, and a table filled through [`SettingsMut`]'s setters with one C
//! fills field by field, slot for slot. [`ParserMut::set_settings`] is compared with
//! `llhttp_init`: two parsers given one table, one by C and one through the safe setter,
//! are compared whole after the call, after `llhttp_reset`, and by the callbacks C's
//! dispatchers run for each of a sequence of runs, `llhttp_finish` included, the two
//! parsers' outcomes and fields compared after each run.
//!
//! The parser's API api.c gives is compared with its C the same way, on pairs of parsers
//! brought to one state: [`ParserMut::pause`], [`ParserMut::resume`] and
//! [`ParserMut::resume_after_upgrade`] against `llhttp_pause`, `llhttp_resume` and
//! `llhttp_resume_after_upgrade` from each state, every pair of them in turn, the whole
//! parser compared after each call; [`ParserMut::execute`] against `llhttp_execute` over runs
//! interleaved with pauses and resumptions; the ten lenient setters against theirs, `int`
//! flags C reads as true included; and [`ParserMut::set_reason`] against
//! `llhttp_set_error_reason`.
//!
//! The lifecycle the same way: [`ParserMut::init`] against `llhttp_init` for each parser type,
//! [`ParserMut::reset`] against `llhttp_reset`, and [`ParserMut::finish`] against
//! `llhttp_finish` with `on_message_complete` returning `0`, `-1` and `HPE_PAUSED`, each from
//! a parser fresh, inside a message, between two, failed, paused at an upgrade and of both kinds
//! settled on a request, `data` and the lenient flags set, the whole parser compared after
//! each call and the next message parsed through C and Rust. The 23 dispatchers are compared
//! with api.c's, declared here since no header declares them, over tables that record, refuse
//! with `-1`, pause and return any other value, and with no table: their returns, the slots
//! they call with each run, and the whole parser after each call.

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int};
use std::mem::MaybeUninit;
use std::sync::Mutex;

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::libgit2::deps::llhttp::api::{Settings, SettingsRef};
use libgit2_crustify::libgit2::deps::llhttp::llhttp::{
    Callback, DataCallback, DataHandler, EventHandler, ParseError, Parsed, Parser, ParserMut,
    ParserRef, ParserType,
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

// --- The parser's API ------------------------------------------------------------------

/// `parser`'s fields as [`fields`] compares them, but for its reason, which is compared by
/// its text: llhttp's literals and Rust's are distinct copies of one text.
fn state(parser: ParserRef<'_>) -> impl PartialEq + std::fmt::Debug + use<> {
    let mut raw = raw_parser(parser);
    raw.reason = core::ptr::null();
    (fields(&raw), parser.reason().map(CStr::to_owned))
}

/// One of the three calls that move a parser between running and paused.
#[derive(Clone, Copy, Debug)]
enum Call {
    Pause,
    Resume,
    ResumeAfterUpgrade,
}

impl Call {
    /// Makes the call through C.
    fn c(self, parser: &mut Parser) {
        let raw = parser.as_mut().as_mut_ptr();
        match self {
            // SAFETY: `raw` addresses a live parser Rust owns; each call reads and writes
            // only its `error` and `reason`.
            Self::Pause => unsafe { sys::llhttp_pause(raw) },
            // SAFETY: as above.
            Self::Resume => unsafe { sys::llhttp_resume(raw) },
            // SAFETY: as above.
            Self::ResumeAfterUpgrade => unsafe { sys::llhttp_resume_after_upgrade(raw) },
        }
    }

    /// Makes the call through Rust.
    fn rust(self, parser: &mut Parser) {
        match self {
            Self::Pause => parser.as_mut().pause(),
            Self::Resume => parser.as_mut().resume(),
            Self::ResumeAfterUpgrade => parser.as_mut().resume_after_upgrade(),
        }
    }
}

/// A response parser fed `input` through C, from `Parser::new`.
fn fed(input: &[u8]) -> Parser {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(ParserType::Response);
    // SAFETY: the parser is initialised and has no settings; `input` is live for the call.
    unsafe {
        sys::llhttp_execute(
            parser.as_mut().as_mut_ptr(),
            input.as_ptr().cast(),
            input.len(),
        )
    };
    parser
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn pause_and_the_resumptions_move_the_parser_as_c_does() {
    const UPGRADE: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\n\
                             Upgrade: x\r\n\r\n";
    // A parser running, paused at an upgrade, and failed; the pause each call makes from
    // them reaches the paused state too.
    let starts: [&[u8]; 4] = [b"", b"HTTP/1.1 200 OK\r\n", UPGRADE, b"HTTP/9"];
    let calls = [Call::Pause, Call::Resume, Call::ResumeAfterUpgrade];
    for start in starts {
        for first in calls {
            for second in calls {
                let (mut c, mut rust) = (fed(start), fed(start));
                for call in [first, second] {
                    call.c(&mut c);
                    call.rust(&mut rust);
                    assert_eq!(
                        state(rust.as_ref()),
                        state(c.as_ref()),
                        "{start:?}: {first:?}, {second:?}"
                    );
                }
            }
        }
    }
}

/// What a run returned through `llhttp_execute`'s convention.
fn code(outcome: Result<Parsed, ParseError>) -> u32 {
    match outcome {
        Ok(Parsed::All) => sys::HPE_OK,
        Ok(Parsed::Paused) => sys::HPE_PAUSED,
        Ok(Parsed::PausedUpgrade) => sys::HPE_PAUSED_UPGRADE,
        Err(error) => error.code().cast_unsigned(),
    }
}

/// A step of a sequence fed to two parsers: a run of the input, or a call.
enum Action {
    Run(&'static [u8]),
    Do(Call),
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn execute_reads_runs_between_pauses_as_llhttp_execute_does() {
    use Action::{Do, Run};
    let sequences: [&[Action]; 3] = [
        &[
            Run(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n"),
            Do(Call::Pause),
            Run(b"\r\nhi"),
            Do(Call::Resume),
            Run(b"\r\nhi"),
            Run(b"HTTP/1.1 204 No Content\r\n\r\n"),
        ],
        &[
            Run(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: x\r\n\r\nbytes"),
            Run(b"more"),
            Do(Call::Resume),
            Run(b"more"),
            Do(Call::ResumeAfterUpgrade),
            Run(b"HTTP/1.1 200 OK\r\n"),
        ],
        &[
            Run(b"HTTP/1.1 200 OK\r\nBad Header\r\n"),
            Do(Call::Pause),
            Do(Call::Resume),
            Run(b"\r\n"),
        ],
    ];
    for steps in sequences {
        let (mut c, mut rust) = (Parser::new(), Parser::new());
        for step in steps {
            match step {
                Run(run) => {
                    // SAFETY: the parser is initialised and has no settings; `run` is live.
                    let c_code = unsafe {
                        sys::llhttp_execute(c.as_mut().as_mut_ptr(), run.as_ptr().cast(), run.len())
                    };
                    assert_eq!(code(rust.as_mut().execute(run)), c_code);
                }
                Do(call) => {
                    call.c(&mut c);
                    call.rust(&mut rust);
                }
            }
            assert_eq!(state(rust.as_ref()), state(c.as_ref()));
        }
    }
}

/// A lenient setter through Rust and through C.
type LenientPair = (
    fn(&mut ParserMut<'_>, bool),
    unsafe extern "C" fn(*mut sys::llhttp_t, c_int),
);

/// The ten lenient setters, Rust's beside C's.
const LENIENT: [LenientPair; 10] = [
    (
        |p, on| p.set_lenient_headers(on),
        sys::llhttp_set_lenient_headers,
    ),
    (
        |p, on| p.set_lenient_chunked_length(on),
        sys::llhttp_set_lenient_chunked_length,
    ),
    (
        |p, on| p.set_lenient_keep_alive(on),
        sys::llhttp_set_lenient_keep_alive,
    ),
    (
        |p, on| p.set_lenient_transfer_encoding(on),
        sys::llhttp_set_lenient_transfer_encoding,
    ),
    (
        |p, on| p.set_lenient_version(on),
        sys::llhttp_set_lenient_version,
    ),
    (
        |p, on| p.set_lenient_data_after_close(on),
        sys::llhttp_set_lenient_data_after_close,
    ),
    (
        |p, on| p.set_lenient_optional_lf_after_cr(on),
        sys::llhttp_set_lenient_optional_lf_after_cr,
    ),
    (
        |p, on| p.set_lenient_optional_crlf_after_chunk(on),
        sys::llhttp_set_lenient_optional_crlf_after_chunk,
    ),
    (
        |p, on| p.set_lenient_optional_cr_before_lf(on),
        sys::llhttp_set_lenient_optional_cr_before_lf,
    ),
    (
        |p, on| p.set_lenient_spaces_after_chunk_size(on),
        sys::llhttp_set_lenient_spaces_after_chunk_size,
    ),
];

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_lenient_setters_set_the_flags_as_c_does() {
    // C reads any nonzero `enabled` as true.
    let sequence: [c_int; 6] = [1, 1, 0, 7, -1, 0];
    for start in [0, 0x155, 0x3FF] {
        for (rust_set, c_set) in LENIENT {
            let (mut c, mut rust) = (Parser::zeroed(), Parser::zeroed());
            c.as_mut().set_lenient_flags_raw(start);
            rust.as_mut().set_lenient_flags_raw(start);
            for enabled in sequence {
                // SAFETY: the handle addresses a live parser Rust owns; the setter writes
                // only its `lenient_flags`.
                unsafe { c_set(c.as_mut().as_mut_ptr(), enabled) };
                rust_set(&mut rust.as_mut(), enabled != 0);
                assert_eq!(
                    fields(&raw_parser(rust.as_ref())),
                    fields(&raw_parser(c.as_ref()))
                );
            }
        }
    }
    // Every setter's bit, from the same start, gives every flag.
    let (mut c, mut rust) = (Parser::zeroed(), Parser::zeroed());
    for (rust_set, c_set) in LENIENT {
        // SAFETY: as above.
        unsafe { c_set(c.as_mut().as_mut_ptr(), 1) };
        rust_set(&mut rust.as_mut(), true);
    }
    assert_eq!(
        rust.as_ref().lenient_flags_raw(),
        c.as_ref().lenient_flags_raw()
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn set_reason_stores_the_string_as_llhttp_set_error_reason_does() {
    for reason in [Some(c"Callback refused"), None] {
        let (mut c, mut rust) = (Parser::zeroed(), Parser::zeroed());
        let raw = reason.map_or(core::ptr::null(), CStr::as_ptr);
        // SAFETY: the handle addresses a live parser Rust owns; the call stores `raw`, a
        // `'static` string or NULL, and reads nothing through it.
        unsafe { sys::llhttp_set_error_reason(c.as_mut().as_mut_ptr(), raw) };
        rust.as_mut().set_reason(reason);
        assert_eq!(
            fields(&raw_parser(rust.as_ref())),
            fields(&raw_parser(c.as_ref()))
        );
    }
}

// --- The parser's lifecycle -------------------------------------------------------------

/// The states init, reset and finish are compared from, each a response parser fed its input
/// through C from `Parser::new`: fresh, inside a message, between two, failed, paused, and
/// a parser of both kinds that settled on a request.
const STARTS: [(ParserType, &[u8]); 6] = [
    (ParserType::Response, b""),
    (
        ParserType::Response,
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhi",
    ),
    (ParserType::Response, b"HTTP/1.1 204 No Content\r\n\r\n"),
    (ParserType::Response, b"HTTP/9"),
    (
        ParserType::Response,
        b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: x\r\n\r\n",
    ),
    (ParserType::Both, b"GET / HTTP/1.1\r\n\r\n"),
];

/// A parser brought to `start` through C, with the table `settings`, lenient flags and a
/// `data` no callback reads.
fn started((parser_type, input): (ParserType, &[u8]), settings: &'static Settings) -> Parser {
    let mut parser = Parser::new();
    parser.as_mut().set_parser_type(parser_type);
    parser.as_mut().set_settings(Some(settings.as_ref()));
    parser.as_mut().set_lenient_keep_alive(true);
    parser.as_mut().set_lenient_version(true);
    // SAFETY: the table's callbacks are Rust handlers that never read `data`, so any value
    // meets `set_data`'s contract; this one is only compared.
    unsafe { parser.as_mut().set_data(core::ptr::dangling_mut()) };
    // SAFETY: the parser is initialised, its table `'static` and of Rust handlers; `input` is
    // live for the call.
    unsafe {
        sys::llhttp_execute(
            parser.as_mut().as_mut_ptr(),
            input.as_ptr().cast(),
            input.len(),
        )
    };
    LOG.take();
    parser
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn init_leaves_the_parser_as_llhttp_init_does() {
    let settings = leaked((1 << 23) - 1);
    let other = leaked(0x55_5555);
    for start in STARTS {
        for parser_type in [ParserType::Both, ParserType::Request, ParserType::Response] {
            let (mut c, mut rust) = (started(start, settings), started(start, settings));
            // SAFETY: the parser is live and Rust's; `other` is `'static` and of Rust handlers.
            unsafe {
                sys::llhttp_init(
                    c.as_mut().as_mut_ptr(),
                    parser_type.raw(),
                    other.as_ref().as_ptr(),
                );
            };
            rust.as_mut().init(parser_type, Some(other.as_ref()));
            assert_eq!(
                state(rust.as_ref()),
                state(c.as_ref()),
                "{start:?} {parser_type:?}"
            );
            // Both parse the same next message.
            let next = b"HTTP/1.1 200 OK\r\n\r\n";
            assert_eq!(rust_runs(&mut rust, &[next]), c_runs(&mut c, &[next]));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reset_leaves_the_parser_as_llhttp_reset_does() {
    let settings = leaked((1 << 23) - 1);
    for start in STARTS {
        let (mut c, mut rust) = (started(start, settings), started(start, settings));
        // SAFETY: the parser is live and Rust's, its table `'static` and of Rust handlers.
        unsafe { sys::llhttp_reset(c.as_mut().as_mut_ptr()) };
        rust.as_mut().reset();
        assert_eq!(state(rust.as_ref()), state(c.as_ref()), "{start:?}");
        assert!(!rust.as_ref().data().is_null());
        let next = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx";
        assert_eq!(
            rust_runs(&mut rust, &[next]),
            c_runs(&mut c, &[next]),
            "{start:?}"
        );
    }
}

/// Returns `R` from `on_message_complete`, recording it as slot 11.
struct CompleteReturn<const R: c_int>;

impl<const R: c_int> EventHandler for CompleteReturn<R> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        LOG.with_borrow_mut(|log| log.push((11, Vec::new())));
        R
    }
}

/// A kept table that records every slot, its `on_message_complete` returning `R`.
fn completing<const R: c_int>() -> &'static Settings {
    let settings = kept();
    let mut c = c_settings_init();
    fill_both((1 << 23) - 1, settings, &mut c);
    settings
        .as_mut()
        .set_on_message_complete(Some(Callback::new::<CompleteReturn<R>>()));
    settings
}

/// `ParserMut::finish` on `parser`, through `llhttp_finish`'s convention.
fn rust_finish(parser: &mut Parser) -> Step {
    let errno = match parser.as_mut().finish() {
        Ok(()) => sys::HPE_OK,
        Err(error) => error.code().cast_unsigned(),
    };
    (errno, LOG.take())
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn finish_finishes_the_parser_as_llhttp_finish_does() {
    let tables = [completing::<0>(), completing::<-1>(), completing::<21>()];
    let ends: [&[u8]; 3] = [
        b"",
        b"HTTP/1.0 200 OK\r\n\r\nto the end",
        b"HTTP/1.1 200 OK\r\n",
    ];
    for settings in tables {
        for start in STARTS
            .into_iter()
            .chain(ends.map(|end| (ParserType::Response, end)))
        {
            let (mut c, mut rust) = (started(start, settings), started(start, settings));
            // Twice: a finish that ended a message leaves one that may end again.
            for _ in 0..2 {
                assert_eq!(rust_finish(&mut rust), c_finish(&mut c), "{start:?}");
                assert_eq!(state(rust.as_ref()), state(c.as_ref()), "{start:?}");
            }
            // A finish state no `llhttp_finish` names is refused, where C aborts; the others
            // are each reached above or here.
            for finish in 0..3 {
                rust.as_mut().set_finish_raw(finish);
                c.as_mut().set_finish_raw(finish);
                assert_eq!(
                    rust_finish(&mut rust),
                    c_finish(&mut c),
                    "{start:?} {finish}"
                );
                assert_eq!(
                    state(rust.as_ref()),
                    state(c.as_ref()),
                    "{start:?} {finish}"
                );
            }
        }
    }
}

// --- The dispatchers ---------------------------------------------------------------------

// api.c's dispatchers, which no header declares (`llhttp.c` declares them for itself), so
// that the raw bindings have none: declared here as api.c defines them, to call its C.
unsafe extern "C" {
    fn llhttp__on_message_begin(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_url(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
    fn llhttp__on_url_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_status(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
    fn llhttp__on_status_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_method(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
    fn llhttp__on_method_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_version(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
    fn llhttp__on_version_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_header_field(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_header_field_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_header_value(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_header_value_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_headers_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_message_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_body(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
    fn llhttp__on_chunk_header(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_chunk_extension_name(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_chunk_extension_name_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_chunk_extension_value(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_chunk_extension_value_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_chunk_complete(
        s: *mut sys::llhttp_t,
        p: *const c_char,
        endp: *const c_char,
    ) -> c_int;
    fn llhttp__on_reset(s: *mut sys::llhttp_t, p: *const c_char, endp: *const c_char) -> c_int;
}

/// Calls `$m!(c dispatcher, method, kind)` for each of api.c's dispatchers.
macro_rules! for_each_dispatcher {
    ($m:ident) => {
        $m!(llhttp__on_message_begin, on_message_begin, event);
        $m!(llhttp__on_url, on_url, data);
        $m!(llhttp__on_url_complete, on_url_complete, event);
        $m!(llhttp__on_status, on_status, data);
        $m!(llhttp__on_status_complete, on_status_complete, event);
        $m!(llhttp__on_method, on_method, data);
        $m!(llhttp__on_method_complete, on_method_complete, event);
        $m!(llhttp__on_version, on_version, data);
        $m!(llhttp__on_version_complete, on_version_complete, event);
        $m!(llhttp__on_header_field, on_header_field, data);
        $m!(
            llhttp__on_header_field_complete,
            on_header_field_complete,
            event
        );
        $m!(llhttp__on_header_value, on_header_value, data);
        $m!(
            llhttp__on_header_value_complete,
            on_header_value_complete,
            event
        );
        $m!(llhttp__on_headers_complete, on_headers_complete, event);
        $m!(llhttp__on_message_complete, on_message_complete, event);
        $m!(llhttp__on_body, on_body, data);
        $m!(llhttp__on_chunk_header, on_chunk_header, event);
        $m!(
            llhttp__on_chunk_extension_name,
            on_chunk_extension_name,
            data
        );
        $m!(
            llhttp__on_chunk_extension_name_complete,
            on_chunk_extension_name_complete,
            event
        );
        $m!(
            llhttp__on_chunk_extension_value,
            on_chunk_extension_value,
            data
        );
        $m!(
            llhttp__on_chunk_extension_value_complete,
            on_chunk_extension_value_complete,
            event
        );
        $m!(llhttp__on_chunk_complete, on_chunk_complete, event);
        $m!(llhttp__on_reset, on_reset, event);
    };
}

/// Returns `R` from every slot, recording each call as `fill_both`'s handlers do.
struct Returning<const R: c_int>;

impl<const R: c_int> EventHandler for Returning<R> {
    fn on_event(_: ParserMut<'_>) -> c_int {
        LOG.with_borrow_mut(|log| log.push((usize::MAX, Vec::new())));
        R
    }
}

impl<const R: c_int> DataHandler for Returning<R> {
    fn on_data(_: ParserMut<'_>, data: &[u8]) -> c_int {
        LOG.with_borrow_mut(|log| log.push((usize::MAX, data.to_vec())));
        R
    }
}

/// A kept table every slot of which returns `R`.
fn returning<const R: c_int>() -> &'static Settings {
    let settings = kept();
    let mut m = settings.as_mut();
    macro_rules! set {
        ($i:expr, $field:ident, $set:ident,event) => {
            m.$set(Some(Callback::new::<Returning<R>>()));
        };
        ($i:expr, $field:ident, $set:ident,data) => {
            m.$set(Some(DataCallback::new::<Returning<R>>()));
        };
    }
    for_each_slot!(set);
    settings
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_dispatchers_call_the_parsers_table_as_api_cs_do() {
    let tables = [
        leaked((1 << 23) - 1),
        leaked(0x55_5555),
        returning::<-1>(),
        returning::<21>(),
        returning::<-7>(),
    ];
    let input = b"a run of the input";
    let runs: [&[u8]; 3] = [b"", b"x", input];
    for settings in tables.into_iter().map(Some).chain([None]) {
        let fresh = || {
            let mut parser = Parser::new();
            parser.as_mut().set_settings(settings.map(Settings::as_ref));
            parser
        };
        let (mut c, mut rust) = (fresh(), fresh());
        macro_rules! compare {
            ($c:ident, $method:ident,event) => {
                let (raw, ptr) = (c.as_mut().as_mut_ptr(), input.as_ptr_range());
                // SAFETY: the parser is live and Rust's, its table NULL or `'static` and of
                // Rust handlers; an event dispatcher reads neither `p` nor `endp`.
                let c_ret = unsafe { $c(raw, ptr.start.cast(), ptr.end.cast()) };
                let c_log = LOG.take();
                let rust_ret = rust.as_mut().$method();
                assert_eq!((rust_ret, LOG.take()), (c_ret, c_log), stringify!($method));
                assert_eq!(state(rust.as_ref()), state(c.as_ref()), stringify!($method));
            };
            ($c:ident, $method:ident,data) => {
                for run in runs {
                    let (raw, ptr) = (c.as_mut().as_mut_ptr(), run.as_ptr_range());
                    // SAFETY: as above; `p` to `endp` is `run`, live for the call.
                    let c_ret = unsafe { $c(raw, ptr.start.cast(), ptr.end.cast()) };
                    let c_log = LOG.take();
                    let rust_ret = rust.as_mut().$method(run);
                    assert_eq!((rust_ret, LOG.take()), (c_ret, c_log), stringify!($method));
                    assert_eq!(state(rust.as_ref()), state(c.as_ref()), stringify!($method));
                }
            };
        }
        for_each_dispatcher!(compare);
    }
}
