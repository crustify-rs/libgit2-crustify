//! Equivalence of `src/cli/opt.c` with its port.
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

use std::ffi::{CString, c_char, c_uint};
use std::process::Command;
use std::{mem, ptr};

use libgit2_crustify::git2::src::cli::opt::{Kind, ParseFlags, Status, Usage, uses_gnu_style};
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
