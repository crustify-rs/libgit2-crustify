//! Equivalence of `src/cli/opt.h`'s enums with their ports: each Rust variant's
//! discriminant and each flag's bits against the constant `opt.h` defines, which the raw
//! bindings reach. The CLI's remaining C passes these values to the ported parser, and reads
//! those it returns, as these integers. Only constants are read: no C runs, so Miri runs
//! them all.

use libgit2_crustify::git2::src::cli::opt::{Kind, ParseFlags, Status, Usage};
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
