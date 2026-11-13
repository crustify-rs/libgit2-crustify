//! `src/cli/opt.c`: the enums of `opt.h`, `Kind::is_option`, the port of
//! `spec_is_option_type`, and `uses_gnu_style`, the port of `support_gnu_style`, checked
//! against their contract through Rust assertions alone. The CLI has no C tests to carry
//! over; these cover the flag sets' operations over the combinations the CLI's spec tables
//! use, the closed enums' C values, which the CLI's remaining C relies on, and the parsing
//! style each combination of flags selects.
//!
//! None reaches C. `uses_gnu_style` reads `POSIXLY_CORRECT`, which a test does not set in its
//! own process: setting the environment is unsafe while other threads may read it, as the
//! harness's do. The test that depends on it runs its body in a child process of its own
//! binary, whose environment the parent sets, and Miri, which spawns no process, ignores it;
//! it runs all the others.

use std::process::Command;

use libgit2_crustify::git2::src::cli::opt::{Kind, ParseFlags, Status, Usage, uses_gnu_style};

/// Every usage flag a constant names, `DEFAULT` aside.
const USAGE_FLAGS: [Usage; 6] = [
    Usage::REQUIRED,
    Usage::CHOICE,
    Usage::STOP_PARSING,
    Usage::VALUE_OPTIONAL,
    Usage::HIDDEN,
    Usage::SHOW_LONG,
];

/// Every parse flag a constant names, `DEFAULT` aside.
const PARSE_FLAGS: [ParseFlags; 2] = [ParseFlags::GNU, ParseFlags::FORCE_GNU];

/// Each named flag is one bit of its own, and `DEFAULT`, the set's default value, is empty.
#[test]
fn each_named_flag_is_a_distinct_bit() {
    assert!(Usage::DEFAULT.is_empty());
    assert_eq!(Usage::default(), Usage::DEFAULT);
    let mut seen = 0_u32;
    for flag in USAGE_FLAGS {
        assert_eq!(flag.bits().count_ones(), 1, "{flag:?}");
        assert_eq!(seen & flag.bits(), 0, "{flag:?}");
        seen |= flag.bits();
    }

    assert!(ParseFlags::DEFAULT.is_empty());
    assert_eq!(ParseFlags::default(), ParseFlags::DEFAULT);
    let mut seen = 0_u32;
    for flag in PARSE_FLAGS {
        assert_eq!(flag.bits().count_ones(), 1, "{flag:?}");
        assert_eq!(seen & flag.bits(), 0, "{flag:?}");
        seen |= flag.bits();
    }
}

/// The combinations the CLI's spec tables spell, `HIDDEN | STOP_PARSING` (`common.h`'s
/// `--help`) and `CHOICE | SHOW_LONG` (`cmd_config.c`), contain each of their flags and
/// none other, as C's `usage & flag` tests read them.
#[test]
fn combined_usage_contains_its_flags_alone() {
    for (a, b) in [
        (Usage::HIDDEN, Usage::STOP_PARSING),
        (Usage::CHOICE, Usage::SHOW_LONG),
    ] {
        let both = a | b;
        assert!(both.contains(a) && both.contains(b));
        assert!(both.contains(both));
        assert!(both.intersects(a) && both.intersects(b));
        assert!(!both.is_empty());
        for other in USAGE_FLAGS.into_iter().filter(|&f| f != a && f != b) {
            assert!(!both.contains(other), "{other:?}");
            assert!(!both.intersects(other), "{other:?}");
        }
        assert_eq!(both & a, a);
        assert_eq!(both & Usage::REQUIRED, Usage::DEFAULT);
    }
}

/// Every set contains the empty set and intersects nothing through it.
#[test]
fn the_empty_set_is_contained_and_intersects_nothing() {
    for flag in USAGE_FLAGS {
        assert!(flag.contains(Usage::DEFAULT));
        assert!(!flag.intersects(Usage::DEFAULT));
    }
    for flag in PARSE_FLAGS {
        assert!(flag.contains(ParseFlags::DEFAULT));
        assert!(!flag.intersects(ParseFlags::DEFAULT));
    }
    assert!(!Usage::DEFAULT.contains(Usage::REQUIRED));
    assert!(!ParseFlags::DEFAULT.contains(ParseFlags::GNU));
}

/// The assigning operators give what the binary ones give.
#[test]
fn assigning_operators_match_the_binary_ones() {
    let mut usage = Usage::REQUIRED;
    usage |= Usage::VALUE_OPTIONAL;
    assert_eq!(usage, Usage::REQUIRED | Usage::VALUE_OPTIONAL);
    usage &= Usage::VALUE_OPTIONAL | Usage::HIDDEN;
    assert_eq!(usage, Usage::VALUE_OPTIONAL);

    // `cli_opt_parse`'s callers pass `CLI_OPT_PARSE_GNU`; `FORCE_GNU` adds to it.
    let mut flags = ParseFlags::GNU;
    flags |= ParseFlags::FORCE_GNU;
    assert_eq!(flags, ParseFlags::GNU | ParseFlags::FORCE_GNU);
    assert!(flags.contains(ParseFlags::GNU) && flags.contains(ParseFlags::FORCE_GNU));
    assert_eq!(flags & ParseFlags::FORCE_GNU, ParseFlags::FORCE_GNU);
    assert_eq!(ParseFlags::GNU & ParseFlags::FORCE_GNU, ParseFlags::DEFAULT);
    flags &= ParseFlags::FORCE_GNU;
    assert_eq!(flags, ParseFlags::FORCE_GNU);
    assert!(!flags.contains(ParseFlags::GNU));
}

/// Bits no constant names are kept, as C keeps them in a spec's `usage` or a parser's
/// `flags`, and do not read as a named flag.
#[test]
fn unnamed_bits_are_retained() {
    for bits in [0, 1 << 6, 1 << 31, u32::MAX, 0b10_1010] {
        assert_eq!(Usage::from_bits_retain(bits).bits(), bits);
        assert_eq!(ParseFlags::from_bits_retain(bits).bits(), bits);
    }
    let unnamed = Usage::from_bits_retain(1 << 6);
    assert!(!unnamed.is_empty());
    assert!(USAGE_FLAGS.iter().all(|&f| !unnamed.intersects(f)));
    let unnamed = ParseFlags::from_bits_retain(1 << 2);
    assert!(PARSE_FLAGS.iter().all(|&f| !unnamed.intersects(f)));

    let all = Usage::from_bits_retain(u32::MAX);
    assert!(USAGE_FLAGS.iter().all(|&f| all.contains(f)));
    assert_eq!(all & Usage::SHOW_LONG, Usage::SHOW_LONG);
}

/// The kinds' discriminants are C's, `0` to `7` in `opt.h`'s order: `None`, `0`, is the
/// kind of the zeroed spec that ends a C spec table, which C tests as false.
#[test]
fn kinds_keep_their_c_values() {
    let kinds = [
        Kind::None,
        Kind::Bool,
        Kind::Switch,
        Kind::Accumulator,
        Kind::Value,
        Kind::Literal,
        Kind::Arg,
        Kind::Args,
    ];
    for (value, kind) in (0_u32..).zip(kinds) {
        assert_eq!(kind as u32, value, "{kind:?}");
    }
}

/// The outcomes' discriminants are C's, `0` to `4` in `opt.h`'s order: `Done`, `0`, is the
/// one `cli_opt_parse`'s callers test as success.
#[test]
fn statuses_keep_their_c_values() {
    let statuses = [
        Status::Done,
        Status::Ok,
        Status::UnknownOption,
        Status::MissingValue,
        Status::MissingArgument,
    ];
    for (value, status) in (0_u32..).zip(statuses) {
        assert_eq!(status as u32, value, "{status:?}");
    }
}

/// The derived traits: copies compare equal, distinct variants do not, and `Debug` names the
/// variant.
#[test]
fn derived_traits_behave() {
    let kind = Kind::Value;
    let copy = kind;
    assert_eq!(kind, copy);
    assert_ne!(Kind::Arg, Kind::Args);
    assert_eq!(format!("{:?}", Kind::Accumulator), "Accumulator");

    let status = Status::MissingValue;
    let copy = status;
    assert_eq!(status, copy);
    assert_ne!(Status::Done, Status::Ok);
    assert_eq!(format!("{:?}", Status::UnknownOption), "UnknownOption");

    assert_eq!(format!("{:?}", Usage::REQUIRED), "Usage(1)");
    assert_eq!(format!("{:?}", ParseFlags::FORCE_GNU), "ParseFlags(2)");
}

/// Every kind, every constant of `cli_opt_type_t`.
const ALL_KINDS: [Kind; 8] = [
    Kind::None,
    Kind::Bool,
    Kind::Switch,
    Kind::Accumulator,
    Kind::Value,
    Kind::Literal,
    Kind::Arg,
    Kind::Args,
];

/// The kinds a long name selects are exactly `Bool`, `Switch` and `Value`: neither the
/// `None` that ends a list nor an accumulator, nor the literal and argument kinds.
#[test]
fn only_bools_switches_and_values_are_options() {
    for ty in ALL_KINDS {
        let expected = matches!(ty, Kind::Bool | Kind::Switch | Kind::Value);
        assert_eq!(ty.is_option(), expected, "{ty:?}");
    }
}

/// The predicate is usable in constant evaluation.
#[test]
fn is_option_is_const() {
    const {
        assert!(Kind::Bool.is_option());
        assert!(!Kind::Args.is_option());
    }
}

/// `FORCE_GNU` asks for GNU style whatever else the flags hold, and reads no environment.
#[test]
fn force_gnu_is_gnu_style() {
    assert!(uses_gnu_style(ParseFlags::FORCE_GNU));
    assert!(uses_gnu_style(ParseFlags::GNU | ParseFlags::FORCE_GNU));
}

/// Without `GNU` or `FORCE_GNU` the parser is POSIX, whatever the environment holds.
#[test]
fn no_style_flag_is_posix_style() {
    assert!(!uses_gnu_style(ParseFlags::DEFAULT));
    assert!(!uses_gnu_style(ParseFlags::default()));
}

/// The bits no constant names change neither style: `FORCE_GNU` stays GNU and the default
/// stays POSIX beside each of them.
#[test]
fn unnamed_flags_do_not_change_the_style() {
    for extra in (2..32).map(|bit| 1 << bit).chain([!0b11]) {
        let extra = ParseFlags::from_bits_retain(extra);
        assert!(!uses_gnu_style(extra), "{extra:?}");
        assert!(uses_gnu_style(extra | ParseFlags::FORCE_GNU), "{extra:?}");
    }
}

/// The variable that tells the child process which environment its parent gave it.
const CHILD: &str = "CRUSTIFY_TEST_OPT_POSIXLY_CORRECT";

/// Runs `name`, a test of this module, alone in a child process whose `POSIXLY_CORRECT` is
/// `value`, or unset; the child knows it is one by [`CHILD`].
fn run_child(name: &str, value: Option<&str>) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        &format!("src::cli::opt::{name}"),
        "--exact",
        "--test-threads=1",
    ]);
    match value {
        Some(value) => cmd.env("POSIXLY_CORRECT", value).env(CHILD, "set"),
        None => cmd.env_remove("POSIXLY_CORRECT").env(CHILD, "unset"),
    };
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child with POSIXLY_CORRECT={value:?} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `GNU` alone asks for GNU style unless `POSIXLY_CORRECT` is set, to any value, the empty
/// one included, and `FORCE_GNU` ignores the variable.
#[test]
#[cfg_attr(miri, ignore = "spawns a process")]
fn gnu_yields_to_posixly_correct() {
    match std::env::var(CHILD).as_deref() {
        Ok("unset") => {
            assert!(uses_gnu_style(ParseFlags::GNU));
            assert!(uses_gnu_style(ParseFlags::FORCE_GNU));
            assert!(uses_gnu_style(
                ParseFlags::GNU | ParseFlags::from_bits_retain(1 << 2)
            ));
            assert!(!uses_gnu_style(ParseFlags::DEFAULT));
        }
        Ok("set") => {
            assert!(!uses_gnu_style(ParseFlags::GNU));
            assert!(!uses_gnu_style(
                ParseFlags::GNU | ParseFlags::from_bits_retain(1 << 2)
            ));
            assert!(uses_gnu_style(ParseFlags::FORCE_GNU));
            assert!(uses_gnu_style(ParseFlags::GNU | ParseFlags::FORCE_GNU));
            assert!(!uses_gnu_style(ParseFlags::DEFAULT));
        }
        _ => {
            let name = "gnu_yields_to_posixly_correct";
            run_child(name, None);
            run_child(name, Some("1"));
            run_child(name, Some(""));
            run_child(name, Some("0"));
        }
    }
}
