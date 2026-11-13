//! Equivalence of `src/cli/opt.c` with its port, and of `opt.h`'s structs with their Rust layouts.
//!
//! - `opt.h`'s enums: each Rust variant's discriminant and each flag's bits against the constant
//!   `opt.h` defines, which the raw bindings reach. The CLI's remaining C passes these values to
//!   the ported parser, and reads those it returns, as these integers. Only constants are read: no
//!   C runs, so Miri runs these.
//!
//! The raw bindings reach neither `spec_is_option_type`, a macro, nor `support_gnu_style`, a
//! static function: each is compared through the C that uses it, as the parser's exported
//! entry points reach it.
//!
//! - `spec_is_option_type` selects, in `spec_for_long`, the specs a long argument `--name` matches
//!   by name. `cli_opt_parser_next` parses `--opt` against a list of specs all named `opt`, each
//!   with no value to write, and returns the first spec the macro accepts, or none; the port
//!   answers with the first spec whose kind `is_option`. Each list is a subset of the seven kinds a
//!   listed spec can have, in order and reversed.
//! - `support_gnu_style` is `cli_opt_parser_init`'s `needs_sort`, compared with `uses_gnu_style`
//!   for each combination of the style flags. The environment it reads is set in a child process of
//!   this binary, never in the test's own, whose other threads may read it.
//! - `opt.h`'s structs: C's own parser, `cli_opt_parser_next` and `cli_opt_parse` through the raw
//!   bindings, runs over spec tables, arguments and parsers built through the safe API, and what it
//!   writes, to the outcome, the parser and the specs' variables, is read back through the getters
//!   and compared with the fields as C laid them out; state the setters write is read by C. Those
//!   call C, so Miri stops there.
//! - `cli_opt_parser_init`: the parser `Parser::new` builds, byte for byte, against the one C sets
//!   up from the same table, arguments and flags.
//! - `cli_opt_status_fprint`: the message `OptRef::write_status` writes, against what C writes to a
//!   memory stream, for each status, choice and name, the strings C writes as glibc's `(null)`
//!   included.

use core::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::process::Command;
use std::{mem, ptr, slice};

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use glibc_bindings::libc::ffi as libc;
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, Kind, Opt, OptRef, ParseFlags, Parser, Spec, SpecRef, SpecTable, Status, Target,
    Usage, uses_gnu_style,
};
use libgit2_crustify::sys::git2 as sys;

/// Each kind's discriminant is its `CLI_OPT_TYPE_*` constant's value.
#[test]
fn kinds_match_the_c_constants() {
    for (kind, c) in [
        (Kind::None, sys::CLI_OPT_TYPE_NONE),
        (Kind::Bool, sys::CLI_OPT_TYPE_BOOL),
        (Kind::Switch, sys::CLI_OPT_TYPE_SWITCH),
        (Kind::Accumulator, sys::CLI_OPT_TYPE_ACCUMULATOR),
        (Kind::Value, sys::CLI_OPT_TYPE_VALUE),
        (Kind::Literal, sys::CLI_OPT_TYPE_LITERAL),
        (Kind::Arg, sys::CLI_OPT_TYPE_ARG),
        (Kind::Args, sys::CLI_OPT_TYPE_ARGS),
    ] {
        assert_eq!(kind as sys::cli_opt_type_t, c, "{kind:?}");
    }
}

/// Each outcome's discriminant is its `CLI_OPT_STATUS_*` constant's value.
#[test]
fn statuses_match_the_c_constants() {
    for (status, c) in [
        (Status::Done, sys::CLI_OPT_STATUS_DONE),
        (Status::Ok, sys::CLI_OPT_STATUS_OK),
        (Status::UnknownOption, sys::CLI_OPT_STATUS_UNKNOWN_OPTION),
        (Status::MissingValue, sys::CLI_OPT_STATUS_MISSING_VALUE),
        (
            Status::MissingArgument,
            sys::CLI_OPT_STATUS_MISSING_ARGUMENT,
        ),
    ] {
        assert_eq!(status as sys::cli_opt_status_t, c, "{status:?}");
    }
}

/// Each usage flag's bits are its `CLI_OPT_USAGE_*` constant's value, and the combinations
/// the spec tables spell are C's `|` of them.
#[test]
fn usage_flags_match_the_c_constants() {
    for (flag, c) in [
        (Usage::DEFAULT, sys::CLI_OPT_USAGE_DEFAULT),
        (Usage::REQUIRED, sys::CLI_OPT_USAGE_REQUIRED),
        (Usage::CHOICE, sys::CLI_OPT_USAGE_CHOICE),
        (Usage::STOP_PARSING, sys::CLI_OPT_USAGE_STOP_PARSING),
        (Usage::VALUE_OPTIONAL, sys::CLI_OPT_USAGE_VALUE_OPTIONAL),
        (Usage::HIDDEN, sys::CLI_OPT_USAGE_HIDDEN),
        (Usage::SHOW_LONG, sys::CLI_OPT_USAGE_SHOW_LONG),
    ] {
        assert_eq!(flag.bits(), c, "{flag:?}");
        assert_eq!(Usage::from_bits_retain(c), flag, "{c}");
    }
    assert_eq!(
        (Usage::HIDDEN | Usage::STOP_PARSING).bits(),
        sys::CLI_OPT_USAGE_HIDDEN | sys::CLI_OPT_USAGE_STOP_PARSING
    );
    assert_eq!(
        (Usage::CHOICE | Usage::SHOW_LONG).bits(),
        sys::CLI_OPT_USAGE_CHOICE | sys::CLI_OPT_USAGE_SHOW_LONG
    );
}

/// Each parse flag's bits are its `CLI_OPT_PARSE_*` constant's value.
#[test]
fn parse_flags_match_the_c_constants() {
    for (flag, c) in [
        (ParseFlags::DEFAULT, sys::CLI_OPT_PARSE_DEFAULT),
        (ParseFlags::GNU, sys::CLI_OPT_PARSE_GNU),
        (ParseFlags::FORCE_GNU, sys::CLI_OPT_PARSE_FORCE_GNU),
    ] {
        assert_eq!(flag.bits(), c, "{flag:?}");
        assert_eq!(ParseFlags::from_bits_retain(c), flag, "{c}");
    }
    assert_eq!(
        (ParseFlags::GNU | ParseFlags::FORCE_GNU).bits(),
        sys::CLI_OPT_PARSE_GNU | sys::CLI_OPT_PARSE_FORCE_GNU
    );
}

/// Every kind a spec in a list can have, with its C constant: every constant of
/// `cli_opt_type_t` but the `NONE` that ends the list.
const LISTED_KINDS: [(Kind, sys::cli_opt_type_t); 7] = [
    (Kind::Bool, sys::CLI_OPT_TYPE_BOOL),
    (Kind::Switch, sys::CLI_OPT_TYPE_SWITCH),
    (Kind::Accumulator, sys::CLI_OPT_TYPE_ACCUMULATOR),
    (Kind::Value, sys::CLI_OPT_TYPE_VALUE),
    (Kind::Literal, sys::CLI_OPT_TYPE_LITERAL),
    (Kind::Arg, sys::CLI_OPT_TYPE_ARG),
    (Kind::Args, sys::CLI_OPT_TYPE_ARGS),
];

/// An all-zero spec, the `NONE` that ends a list, and the base of every other.
fn zeroed_spec() -> sys::cli_opt_spec {
    // SAFETY: `cli_opt_spec` is integers and raw pointers, for which all-zero bytes are a
    // valid value: `CLI_OPT_TYPE_NONE` and NULL.
    unsafe { mem::zeroed() }
}

/// The index of the spec of `types` that C's `cli_opt_parser_next` returns for `--opt`, each
/// spec named `opt`, or `None` when it returns none.
fn c_long_match(types: &[sys::cli_opt_type_t]) -> Option<usize> {
    let name = c"opt";
    let mut specs: Vec<sys::cli_opt_spec> = types
        .iter()
        .map(|&type_| sys::cli_opt_spec {
            type_,
            name: name.as_ptr(),
            ..zeroed_spec()
        })
        .collect();
    specs.push(zeroed_spec());
    let arg = CString::new("--opt").unwrap().into_raw();
    let mut args = [arg];

    // SAFETY: an all-zero parser is a valid value, which `cli_opt_parser_init` overwrites.
    let mut parser: sys::cli_opt_parser = unsafe { mem::zeroed() };
    // SAFETY: `specs` ends with a NONE spec and, like `args`, outlives the parser's use
    // below; every spec's `value` is NULL, so the parser writes through none.
    unsafe {
        sys::cli_opt_parser_init(&raw mut parser, specs.as_ptr(), args.as_mut_ptr(), 1, 0);
    }
    // SAFETY: as above, `opt` is a valid all-zero `cli_opt`, which the call overwrites.
    let mut opt: sys::cli_opt = unsafe { mem::zeroed() };
    // SAFETY: `parser` was initialised over `specs` and `args`, both still live.
    let status = unsafe { sys::cli_opt_parser_next(&raw mut opt, &raw mut parser) };
    let matched = (!opt.spec.is_null()).then(|| {
        // SAFETY: a non-NULL `opt.spec` points into `specs`, as the parser returns one of
        // the specs it was given.
        let at = unsafe { opt.spec.offset_from(specs.as_ptr()) };
        usize::try_from(at).unwrap()
    });
    let expected_status = match matched {
        None => sys::CLI_OPT_STATUS_UNKNOWN_OPTION,
        // `--opt` gives a value spec no value.
        Some(at) if types[at] == sys::CLI_OPT_TYPE_VALUE => sys::CLI_OPT_STATUS_MISSING_VALUE,
        Some(_) => sys::CLI_OPT_STATUS_OK,
    };
    assert_eq!(status, expected_status, "{types:?}");
    // SAFETY: `arg` came from `CString::into_raw`, and the parser, which read it, is done.
    drop(unsafe { CString::from_raw(args[0]) });
    matched
}

/// `spec_is_option_type` in `spec_for_long`, against `is_option`: for every type alone, and
/// for every subset of them, in order and reversed, the first option-type spec matches.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn long_names_select_the_specs_the_c_macro_selects() {
    for subset in 1..(1u32 << LISTED_KINDS.len()) {
        let chosen: Vec<_> = LISTED_KINDS
            .iter()
            .enumerate()
            .filter(|(i, _)| subset & (1 << i) != 0)
            .map(|(_, &t)| t)
            .collect();
        for order in [chosen.clone(), chosen.iter().rev().copied().collect()] {
            let c_types: Vec<_> = order.iter().map(|&(_, c)| c).collect();
            let rust = order.iter().position(|&(ty, _)| ty.is_option());
            assert_eq!(rust, c_long_match(&c_types), "{order:?}");
        }
    }
}

/// `cli_opt_parser_init`'s `needs_sort` for `flags`: `uses_gnu_style(flags)` as C computes
/// it.
fn c_uses_gnu_style(flags: c_uint) -> bool {
    let specs = [zeroed_spec()];
    let mut args: [*mut c_char; 0] = [];
    // SAFETY: an all-zero parser is a valid value, which `cli_opt_parser_init` overwrites.
    let mut parser: sys::cli_opt_parser = unsafe { mem::zeroed() };
    // SAFETY: `specs` holds the NONE spec that ends a list, and `args` is empty; the parser
    // is only read after the call.
    unsafe {
        sys::cli_opt_parser_init(&raw mut parser, specs.as_ptr(), args.as_mut_ptr(), 0, flags);
    }
    assert!(ptr::eq(parser.specs, specs.as_ptr()));
    assert_eq!(parser.flags, flags);
    parser.needs_sort() != 0
}

/// Every combination of the style flags, the default first, and each beside bits no
/// constant names, which C keeps in the parser's `flags` and does not test.
fn flag_cases() -> Vec<ParseFlags> {
    let styles = [
        ParseFlags::DEFAULT,
        ParseFlags::GNU,
        ParseFlags::FORCE_GNU,
        ParseFlags::GNU | ParseFlags::FORCE_GNU,
    ];
    let unnamed = [0, 1 << 2, 1 << 31, !0b11].map(ParseFlags::from_bits_retain);
    unnamed
        .iter()
        .flat_map(|&extra| styles.map(|style| style | extra))
        .collect()
}

/// The variable that tells the child process which environment its parent gave it.
const CHILD: &str = "CRUSTIFY_TEST_OPT_POSIXLY_CORRECT";

/// `uses_gnu_style` against `cli_opt_parser_init`'s `needs_sort`, for every combination of
/// the style flags, beside unnamed bits or not, with `POSIXLY_CORRECT` unset, set to a value, and
/// set empty, each in a child process; each child prints what both sides gave, which the parent
/// checks too.
#[test]
#[cfg_attr(miri, ignore = "spawns a process")]
fn gnu_style_matches_the_c_in_each_environment() {
    let name = "src::cli::opt::gnu_style_matches_the_c_in_each_environment";
    if std::env::var_os(CHILD).is_some() {
        for flags in flag_cases() {
            let (rust, c) = (uses_gnu_style(flags), c_uses_gnu_style(flags.bits()));
            assert_eq!(rust, c, "{flags:?}");
            println!("STEP {} {rust}", flags.bits());
        }
        return;
    }
    let posix = [None, Some("1"), Some("")];
    for value in posix {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, "1");
        match value {
            Some(value) => cmd.env("POSIXLY_CORRECT", value),
            None => cmd.env_remove("POSIXLY_CORRECT"),
        };
        let out = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child with POSIXLY_CORRECT={value:?} failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let steps: Vec<_> = stdout
            .lines()
            .filter_map(|l| l.split_once("STEP ").map(|(_, step)| step.to_owned()))
            .collect();
        let gnu = value.is_none();
        let expected: Vec<_> = flag_cases()
            .iter()
            .map(|flags| {
                let style = match flags.bits() & 0b11 {
                    0 => false,
                    1 => gnu,
                    _ => true,
                };
                format!("{} {style}", flags.bits())
            })
            .collect();
        assert_eq!(steps, expected, "POSIXLY_CORRECT={value:?}");
    }
}
/// One step of C's parser: `cli_opt_parser_next` over `opt` and `parser`, its status checked
/// against the outcome's.
fn next<'p>(opt: &mut Opt<'p>, parser: &mut Parser<'p>) -> Option<Status> {
    let (o, p) = (opt.as_mut().as_mut_ptr(), parser.as_mut().as_mut_ptr());
    // SAFETY: both point at live values Rust holds, borrowed exclusively for the call; the
    // parser's table and arguments, built through the safe API, are borrowed for `'p`, so what
    // C stores in the outcome and the specs' variables lives as long as they do.
    let status = unsafe { sys::cli_opt_parser_next(o, p) };
    assert_eq!(opt.as_ref().status().map(|s| s as u32), Some(status));
    opt.as_ref().status()
}

/// The getters of an outcome against its fields as C laid them out.
fn assert_opt_matches_c(opt: OptRef<'_, '_>) {
    // SAFETY: the handle borrows a live, initialised outcome; this copies its bytes out.
    let c = unsafe { opt.as_ptr().read() };
    assert_eq!(opt.status().map(|s| s as u32), Some(c.status));
    assert_eq!(opt.spec().map_or(ptr::null(), |s| s.as_ptr()), c.spec);
    assert_eq!(
        opt.arg().map_or(ptr::null(), CStr::as_ptr),
        c.arg.cast_const()
    );
    assert_eq!(
        opt.value().map_or(ptr::null(), CStr::as_ptr),
        c.value.cast_const()
    );
    assert_eq!(opt.args_len(), c.args_len);
}

/// The getters of a parser against its fields as C laid them out, the bit-fields through
/// bindgen's accessors.
fn assert_parser_matches_c(parser: &Parser<'_>) {
    let r = parser.as_ref();
    // SAFETY: the handle borrows a live, initialised parser; this copies its bytes out.
    let c = unsafe { r.as_ptr().read() };
    let table = r.specs().expect("a table");
    assert_eq!(table.get(0).map_or(ptr::null(), |s| s.as_ptr()), c.specs);
    assert_eq!(r.args().as_ptr().cast::<*mut c_char>(), c.args.cast_const());
    assert_eq!((r.args_len(), r.flags().bits()), (c.args_len, c.flags));
    assert_eq!(
        (r.idx(), r.arg_idx(), r.in_args(), r.in_short()),
        (c.idx, c.arg_idx, c.in_args, c.in_short)
    );
    assert_eq!(
        (u32::from(r.needs_sort()), u32::from(r.in_literal())),
        (c.needs_sort(), c.in_literal())
    );
}

/// The strings of a parser's arguments, in their current order.
fn order<'p>(parser: &Parser<'p>) -> Vec<&'p CStr> {
    parser
        .as_ref()
        .args()
        .iter()
        .map(|a| a.as_c_str())
        .collect()
}

/// C parses a table of every kind built in Rust: each outcome it writes reads back through the
/// getters, by spec position, argument and value, and each variable holds what C stored, the
/// remaining arguments found through the slot in the parser's array.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn c_parses_a_table_built_in_rust() {
    let debug = Cell::new(0);
    let mode = Cell::new(0);
    let verbose = Cell::new(0);
    let output = Cell::new(None);
    let file = Cell::new(None);
    let rest = ArgsSlot::new();
    let specs = [
        Spec::new(Target::Bool(Some(&debug)))
            .with_name(c"debug")
            .with_alias(b'd'),
        Spec::new(Target::Switch(Some(&mode)))
            .with_name(c"read")
            .with_switch_value(1),
        Spec::new(Target::Switch(Some(&mode)))
            .with_name(c"write")
            .with_switch_value(2),
        Spec::new(Target::Accumulator(Some(&verbose))).with_alias(b'v'),
        Spec::new(Target::Value(Some(&output)))
            .with_name(c"output")
            .with_alias(b'o'),
        Spec::new(Target::Arg(Some(&file))),
        Spec::new(Target::Args(Some(&rest))),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [
        c"--debug",
        c"-vv",
        c"--write",
        c"-ofile.txt",
        c"--output=x",
        c"--no-debug",
        c"input",
        c"more",
        c"stuff",
    ]
    .map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let mut opt = Opt::zeroed();

    // (status, spec, arg, value, the parser's next index, its position in a bundle)
    let expected = [
        (Status::Ok, Some(0), Some(c"--debug"), None, 1, 0),
        (Status::Ok, Some(3), Some(c"-vv"), None, 1, 1),
        (Status::Ok, Some(3), Some(c"-vv"), None, 2, 0),
        (Status::Ok, Some(2), Some(c"--write"), None, 3, 0),
        (
            Status::Ok,
            Some(4),
            Some(c"-ofile.txt"),
            Some(c"file.txt"),
            4,
            0,
        ),
        (Status::Ok, Some(4), Some(c"--output=x"), Some(c"x"), 5, 0),
        (Status::Ok, Some(0), Some(c"--no-debug"), None, 6, 0),
        (Status::Ok, Some(5), Some(c"input"), None, 7, 0),
        (Status::Ok, Some(6), Some(c"more"), None, 9, 0),
    ];
    for (i, (status, spec, arg, value, idx, in_short)) in expected.into_iter().enumerate() {
        assert_eq!(next(&mut opt, &mut parser), Some(status), "step {i}");
        let o = opt.as_ref();
        assert_opt_matches_c(o);
        assert_parser_matches_c(&parser);
        assert_eq!(
            o.spec().map(|s| table.position(s)),
            spec.map(Some),
            "step {i}"
        );
        assert_eq!((o.arg(), o.value()), (arg, value), "step {i}");
        assert_eq!(
            (parser.as_ref().idx(), parser.as_ref().in_short()),
            (idx, in_short),
            "step {i}"
        );
    }
    assert_eq!(opt.as_ref().args_len(), 2);
    assert_eq!(parser.as_ref().in_args(), 2);
    assert_eq!(parser.as_ref().arg_idx(), 1);
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Done));
    assert_opt_matches_c(opt.as_ref());
    assert_eq!(opt.as_ref().args_len(), 2);
    assert!(opt.as_ref().spec().is_none());

    assert_eq!((debug.get(), mode.get(), verbose.get()), (0, 2, 2));
    assert_eq!(output.get().map(Arg::as_c_str), Some(c"x"));
    assert_eq!(file.get().map(Arg::as_c_str), Some(c"input"));
    let tail = rest
        .get_in(parser.as_ref().args())
        .expect("in the parser's arguments");
    let tail: Vec<_> = tail.iter().map(|a| a.as_c_str()).collect();
    assert_eq!(tail, [c"more", c"stuff"]);
}

/// C reorders a GNU-style parser's arguments in the array the parser lends, options and their
/// values ahead of the bare arguments, and clears `needs_sort` once it has.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn c_reorders_the_arguments_rust_lent() {
    let debug = Cell::new(0);
    let output = Cell::new(None);
    let file = Cell::new(None);
    let rest = ArgsSlot::new();
    let specs = [
        Spec::new(Target::Bool(Some(&debug))).with_name(c"debug"),
        Spec::new(Target::Value(Some(&output))).with_alias(b'o'),
        Spec::new(Target::Arg(Some(&file))),
        Spec::new(Target::Args(Some(&rest))),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [c"file", c"--debug", c"-o", c"out", c"more"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    assert!(parser.as_ref().needs_sort());
    let mut opt = Opt::zeroed();
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Ok));
    assert_parser_matches_c(&parser);
    assert!(!parser.as_ref().needs_sort());
    assert_eq!(
        order(&parser),
        [c"--debug", c"-o", c"out", c"file", c"more"]
    );
    assert_eq!(opt.as_ref().arg(), Some(c"--debug"));
    while next(&mut opt, &mut parser) == Some(Status::Ok) {
        assert_opt_matches_c(opt.as_ref());
    }
    assert_eq!(opt.as_ref().status(), Some(Status::Done));
    assert_eq!(debug.get(), 1);
    assert_eq!(output.get().map(Arg::as_c_str), Some(c"out"));
    assert_eq!(file.get().map(Arg::as_c_str), Some(c"file"));
    let tail = rest
        .get_in(parser.as_ref().args())
        .expect("in the parser's arguments");
    assert_eq!(
        tail.iter().map(|a| a.as_c_str()).collect::<Vec<_>>(),
        [c"more"]
    );
}

/// The state the setters write is the state C reads: a literal parser takes `--debug` as a
/// bare argument, a set index skips arguments, a position in a bundle resumes there, and a
/// sort asked for is done.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn c_reads_the_state_rust_set() {
    let debug = Cell::new(0);
    let verbose = Cell::new(0);
    let file = Cell::new(None);
    let specs = [
        Spec::new(Target::Bool(Some(&debug)))
            .with_name(c"debug")
            .with_alias(b'd'),
        Spec::new(Target::Accumulator(Some(&verbose))).with_alias(b'v'),
        Spec::new(Target::Arg(Some(&file))),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut opt = Opt::zeroed();

    let mut literal_args = [Arg::new(c"--debug")];
    let mut parser = Parser::new(table, &mut literal_args, ParseFlags::DEFAULT);
    parser.as_mut().set_in_literal(true);
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Ok));
    assert_eq!(
        opt.as_ref().spec().map(|s| table.position(s)),
        Some(Some(2))
    );
    assert_eq!(file.get().map(Arg::as_c_str), Some(c"--debug"));
    assert_eq!(debug.get(), 0);

    let mut skipped = [c"--nope", c"-dv"].map(Arg::new);
    let mut parser = Parser::new(table, &mut skipped, ParseFlags::DEFAULT);
    parser.as_mut().set_idx(1);
    parser.as_mut().set_in_short(1);
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Ok));
    assert_eq!(
        opt.as_ref().spec().map(|s| table.position(s)),
        Some(Some(1))
    );
    assert_eq!((verbose.get(), debug.get()), (1, 0));
    assert_parser_matches_c(&parser);
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Done));

    let mut sorted = [c"x", c"-d"].map(Arg::new);
    let mut parser = Parser::new(table, &mut sorted, ParseFlags::DEFAULT);
    parser.as_mut().set_needs_sort(true);
    parser.as_mut().set_flags(ParseFlags::GNU);
    assert_eq!(next(&mut opt, &mut parser), Some(Status::Ok));
    assert_eq!(order(&parser), [c"-d", c"x"]);
    assert_eq!(debug.get(), 1);
    assert_eq!(parser.as_ref().flags(), ParseFlags::GNU);
}

/// A `cli_opt_parse` case: the arguments, and the status, spec position and argument of the
/// outcome.
type Case<'a> = (&'a [&'a CStr], Status, Option<usize>, Option<&'a CStr>);

/// `cli_opt_parse` reports its failures in an outcome Rust reads: the unknown option with no
/// spec, the value a spec lacks, and the required argument missing, with its spec.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn c_parse_reports_failures_through_the_outcome() {
    let output = Cell::new(None);
    let file = Cell::new(None);
    let specs = [
        Spec::new(Target::Value(Some(&output))).with_name(c"output"),
        Spec::new(Target::Arg(Some(&file))).with_usage(Usage::REQUIRED),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let c_table = table.get(0).expect("a spec").as_ptr();

    let cases: [Case<'_>; 3] = [
        (&[c"--nope"], Status::UnknownOption, None, Some(c"--nope")),
        (
            &[c"--output"],
            Status::MissingValue,
            Some(0),
            Some(c"--output"),
        ),
        (&[c"--output=x"], Status::MissingArgument, Some(1), None),
    ];
    for (given, status, spec, arg) in cases {
        let mut args: Vec<Arg<'_>> = given.iter().copied().map(Arg::new).collect();
        let mut opt = Opt::zeroed();
        // SAFETY: the outcome is live and borrowed exclusively for the call; the table is a
        // `SpecTable`'s, ended by its zeroed spec, and the arguments a live array of C strings,
        // which C reorders in place under `CLI_OPT_PARSE_DEFAULT` never.
        let rc = unsafe {
            sys::cli_opt_parse(
                opt.as_mut().as_mut_ptr(),
                c_table,
                args.as_mut_ptr().cast(),
                args.len(),
                ParseFlags::DEFAULT.bits(),
            )
        };
        let o = opt.as_ref();
        assert_eq!(o.status(), Some(status), "{given:?}");
        assert_eq!(o.status().map(|s| s as u32), Some(rc), "{given:?}");
        assert_opt_matches_c(o);
        assert_eq!(
            o.spec().map(|s| table.position(s)),
            spec.map(Some),
            "{given:?}"
        );
        assert_eq!(o.arg(), arg, "{given:?}");
    }
}

/// Values C can hold that `opt.h` does not name read as no kind, no target and no status.
#[test]
fn unnamed_c_values_are_none() {
    let raw = sys::cli_opt_spec {
        type_: sys::CLI_OPT_TYPE_ARGS + 1,
        ..Default::default()
    };
    // SAFETY: `raw` is a live spec with `NULL` strings and variable, borrowed for the handle.
    let spec = unsafe { SpecRef::from_ptr((&raw const raw).cast_mut()) }.expect("non-null");
    assert_eq!(spec.kind(), None);
    assert!(spec.target().is_none());

    let raw = sys::cli_opt {
        status: sys::CLI_OPT_STATUS_MISSING_ARGUMENT + 1,
        ..Default::default()
    };
    // SAFETY: `raw` is a live outcome with `NULL` pointers, borrowed for the handle.
    let opt = unsafe { OptRef::from_ptr((&raw const raw).cast_mut()) }.expect("non-null");
    assert_eq!(opt.status(), None);
}

/// The bytes of a parser, its padding included, which C's `memset` and `Parser::zeroed` both
/// clear before the fields are written.
fn parser_bytes(parser: &sys::cli_opt_parser) -> Vec<u8> {
    // SAFETY: `parser` is a live parser, whose every byte was written: zeroed whole, then its
    // fields stored.
    unsafe { slice::from_raw_parts(ptr::from_ref(parser).cast::<u8>(), size_of_val(parser)) }
        .to_vec()
}

/// `Parser::new` builds, byte for byte, the parser `cli_opt_parser_init` sets up from the same
/// table, arguments and flags, for each combination of the style flags beside unnamed bits, in
/// this process's environment, which both read.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parser_new_sets_up_what_cli_opt_parser_init_sets_up() {
    for flags in flag_cases() {
        for given in [&[][..], &[c"--debug", c"file"][..]] {
            let mut args: Vec<Arg<'_>> = given.iter().copied().map(Arg::new).collect();
            let specs = [
                Spec::new(Target::Bool(None)).with_name(c"debug"),
                Spec::new(Target::Arg(None)),
                Spec::zeroed(),
            ];
            let table = SpecTable::new(&specs).expect("ended");
            let c_ptr = args.as_mut_ptr();
            // SAFETY: an all-zero parser is a valid value, which `cli_opt_parser_init` overwrites.
            let mut c: sys::cli_opt_parser = unsafe { mem::zeroed() };
            // SAFETY: the table is ended by its zeroed spec, and `args` a live array of C
            // strings; C stores both without reading them.
            unsafe {
                sys::cli_opt_parser_init(
                    &raw mut c,
                    table.get(0).map_or(ptr::null(), |s| s.as_ptr()),
                    c_ptr.cast(),
                    args.len(),
                    flags.bits(),
                );
            }
            let parser = Parser::new(table, &mut args, flags);
            // SAFETY: the handle borrows a live, initialised parser; this copies its bytes out.
            let rust = unsafe { parser.as_ref().as_ptr().read() };
            if given.is_empty() {
                // An empty vector's pointer is dangling, which both store as given.
                assert_eq!(rust.args, c.args);
            }
            assert_eq!(parser_bytes(&rust), parser_bytes(&c), "{flags:?} {given:?}");
        }
    }
}

/// A memory stream, `open_memstream`'s, and the bytes it holds once closed.
struct MemStream {
    file: *mut libc::FILE,
    buf: Box<(*mut c_char, usize)>,
}

impl MemStream {
    fn open() -> Self {
        let mut buf = Box::new((ptr::null_mut(), 0));
        // SAFETY: both locations are boxed, so they stay put until `close` reads them.
        let file = unsafe { libc::open_memstream(&raw mut buf.0, &raw mut buf.1) };
        assert!(!file.is_null());
        Self { file, buf }
    }

    /// Closes the stream and returns what was written to it.
    fn close(self) -> Vec<u8> {
        // SAFETY: the stream is open, and nothing uses it after this.
        assert_eq!(unsafe { libc::fclose(self.file) }, 0);
        // SAFETY: `fclose` left the buffer holding `size` bytes, which `free` releases.
        let bytes = unsafe { slice::from_raw_parts(self.buf.0.cast::<u8>(), self.buf.1) }.to_vec();
        // SAFETY: the buffer is `open_memstream`'s, freed once, with `free`.
        unsafe { libc::free(self.buf.0.cast()) };
        bytes
    }
}

/// What C's `cli_opt_status_fprint` writes for the outcome at `opt`, and what it returns.
fn c_status(opt: *const sys::cli_opt, command: Option<&CStr>) -> (Vec<u8>, c_int) {
    let stream = MemStream::open();
    // SAFETY: an open stream, the command `NULL` or a C string, and a live outcome whose spec,
    // if it has one, is an element of a live table.
    let rc = unsafe {
        sys::cli_opt_status_fprint(stream.file, command.map_or(ptr::null(), CStr::as_ptr), opt)
    };
    (stream.close(), rc)
}

/// The message `write_status` writes for `opt`, whose spec, if it has one, is in `table`.
fn rust_status<'p>(opt: OptRef<'_, 'p>, command: Option<&CStr>, table: SpecTable<'p>) -> Vec<u8> {
    let mut out = Vec::new();
    let command = command.map(|c| c.to_str().expect("UTF-8"));
    opt.write_status(&mut out, command, table).expect("written");
    out
}

/// `write_status` writes what `cli_opt_status_fprint` writes, for each status, an argument, a
/// name or a value name C writes as `(null)`, an alias that is not ASCII, an argument that is not
/// UTF-8, every spec of a table of choices, and a status `opt.h` does not name, with the command
/// and without; C reports each message written.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn write_status_writes_what_cli_opt_status_fprint_writes() {
    let specs = [
        Spec::new(Target::Switch(None)).with_alias(b'a'),
        Spec::new(Target::Switch(None))
            .with_name(c"bee")
            .with_usage(Usage::CHOICE),
        Spec::new(Target::Arg(None))
            .with_value_name(c"<c>")
            .with_usage(Usage::CHOICE),
        Spec::new(Target::Args(None)).with_usage(Usage::CHOICE),
        Spec::new(Target::Value(None))
            .with_alias(b'o')
            .with_name(c"out"),
        Spec::new(Target::Value(None))
            .with_alias(b'o')
            .with_name(c"out")
            .with_usage(Usage::SHOW_LONG | Usage::CHOICE),
        Spec::new(Target::Bool(None)),
        Spec::new(Target::Switch(None)).with_alias(0xE9),
        Spec::new(Target::Switch(None))
            .with_alias(b'x')
            .with_usage(Usage::CHOICE),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut cases: Vec<(Status, Option<usize>, Option<&CStr>)> = vec![
        (Status::Done, None, None),
        (Status::Ok, Some(0), Some(c"-a")),
        (Status::UnknownOption, None, Some(c"--nope")),
        (Status::UnknownOption, None, None),
        (Status::UnknownOption, None, Some(c"--caf\xe9")),
    ];
    for spec in 0..table.len() {
        cases.push((Status::MissingValue, Some(spec), None));
        cases.push((Status::MissingArgument, Some(spec), None));
    }
    for (status, spec, arg) in cases {
        let mut opt = Opt::zeroed();
        opt.as_mut().set_status(status);
        opt.as_mut().set_spec(spec.and_then(|i| table.get(i)));
        opt.as_mut().set_arg(arg);
        for command in [None, Some(c"git2")] {
            let (c, rc) = c_status(opt.as_ref().as_ptr(), command);
            assert!(rc >= 0, "{status:?} {spec:?}");
            assert_eq!(
                rust_status(opt.as_ref(), command, table),
                c,
                "{status:?} {spec:?} {command:?}"
            );
        }
    }
    for raw in [sys::CLI_OPT_STATUS_MISSING_ARGUMENT + 1, 1 << 31, u32::MAX] {
        let mut c_opt = sys::cli_opt {
            status: raw,
            ..Default::default()
        };
        let (c, rc) = c_status(&raw const c_opt, Some(c"git2"));
        assert!(rc >= 0);
        // SAFETY: `c_opt` is a live outcome with `NULL` pointers, borrowed for the handle.
        let opt = unsafe { OptRef::from_ptr(&raw mut c_opt) }.expect("non-null");
        assert_eq!(rust_status(opt, Some(c"git2"), table), c, "{raw}");
    }
}
