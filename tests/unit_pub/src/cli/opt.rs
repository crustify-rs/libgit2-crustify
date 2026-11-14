//! `src/cli/opt.c`: the enums and structs of `opt.h`, `Kind::is_option`, the port of
//! `spec_is_option_type`, and `uses_gnu_style`, the port of `support_gnu_style`, checked
//! against their contract through Rust assertions alone. The CLI has no C tests to carry
//! over; these cover the flag sets' operations over the combinations the CLI's spec tables
//! use, the closed enums' C values, which the CLI's remaining C relies on, the parsing
//! style each combination of flags selects, and the accessors of the spec, outcome and parser
//! layouts, built in Rust storage; `Parser::new`, the port of `cli_opt_parser_init`, the
//! message `OptRef::write_status`, the port of `cli_opt_status_fprint`, writes for each outcome,
//! and the callback `ForeachCallbackFn` over a Rust closure.
//!
//! None reaches C: the flags' C values are `opt.h`'s constants, which the raw bindings read.
//! `uses_gnu_style` reads `POSIXLY_CORRECT`, which a test does not set in its own process:
//! setting the environment is unsafe while other threads may read it, as the harness's do. The test
//! that depends on it runs its body in a child process of its own binary, whose environment the
//! parent sets, and Miri, which spawns no process, ignores it; it runs all the others.

use core::cell::Cell;
use core::ffi::{CStr, c_int};
use core::num::NonZero;
use core::ops::ControlFlow;
use std::io;
use std::process::Command;

use ffibox::CZeroable;
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, ForeachCallbackFn, Kind, Opt, OptRef, ParseFlags, Parser, Spec, SpecTable,
    Status, Target, Usage, for_each, parse, uses_gnu_style,
};
use libgit2_crustify::sys::git2 as sys;

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

/// Each usage flag's bits are its `CLI_OPT_USAGE_*` constant's value, as the raw bindings read
/// it from `opt.h`, and the combinations the CLI's spec tables spell are C's `|` of them: the
/// tables, which stay C, set these bits, and the parser reads them. Carried over from the
/// equivalence test the module's switch retired.
#[test]
fn usage_flags_keep_their_c_values() {
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

/// Each parse flag's bits are its `CLI_OPT_PARSE_*` constant's value, as the raw bindings read it
/// from `opt.h`: the CLI's C passes them to the exports. Carried over from the equivalence test
/// the module's switch retired.
#[test]
fn parse_flags_keep_their_c_values() {
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
/// A zeroed spec is the end of a table: kind `None`, and no variable, name, alias, switch
/// value, usage flag, value name or help, as `Default` builds it too.
#[test]
fn a_zeroed_spec_is_empty() {
    for spec in [Spec::zeroed(), Spec::default()] {
        let r = spec.as_ref();
        assert_eq!(r.kind(), Some(Kind::None));
        assert!(matches!(r.target(), Some(Target::None)));
        assert_eq!(r.name(), None);
        assert_eq!(r.alias(), None);
        assert_eq!(r.switch_value(), 0);
        assert_eq!(r.usage(), Usage::DEFAULT);
        assert_eq!(r.value_name(), None);
        assert_eq!(r.help(), None);
    }
}

/// Each target reads back with its kind and the very variable it was given, or none.
#[test]
fn targets_round_trip_with_their_kinds() {
    let flag = Cell::new(0);
    let value = Cell::new(None);
    let rest = ArgsSlot::new();
    let targets = [
        Target::None,
        Target::Bool(Some(&flag)),
        Target::Switch(Some(&flag)),
        Target::Accumulator(Some(&flag)),
        Target::Value(Some(&value)),
        Target::Literal,
        Target::Arg(Some(&value)),
        Target::Args(Some(&rest)),
        Target::Bool(None),
        Target::Value(None),
        Target::Args(None),
    ];
    for target in targets {
        let spec = Spec::new(target);
        let read = spec.as_ref().target().expect("a kind opt.h names");
        assert_eq!(spec.as_ref().kind(), Some(target.kind()), "{target:?}");
        assert_eq!(read.kind(), target.kind(), "{target:?}");
        match (target, read) {
            (Target::Bool(a), Target::Bool(b))
            | (Target::Switch(a), Target::Switch(b))
            | (Target::Accumulator(a), Target::Accumulator(b)) => {
                assert_eq!(a.map(core::ptr::from_ref), b.map(core::ptr::from_ref));
            }
            (Target::Value(a), Target::Value(b)) | (Target::Arg(a), Target::Arg(b)) => {
                assert_eq!(a.map(core::ptr::from_ref), b.map(core::ptr::from_ref));
            }
            (Target::Args(a), Target::Args(b)) => {
                assert_eq!(a.map(core::ptr::from_ref), b.map(core::ptr::from_ref));
            }
            (Target::None, Target::None) | (Target::Literal, Target::Literal) => {}
            _ => panic!("{target:?} read back as {read:?}"),
        }
    }
}

/// Setting a target replaces the kind and the variable together: a switch turned into a value
/// spec no longer reaches the switch's variable.
#[test]
fn set_target_replaces_kind_and_variable() {
    let mode = Cell::new(7);
    let value = Cell::new(None);
    let mut spec = Spec::new(Target::Switch(Some(&mode)));
    spec.as_mut().set_target(Target::Value(Some(&value)));
    let Some(Target::Value(Some(v))) = spec.as_ref().target() else {
        panic!("{:?}", spec.as_ref().target());
    };
    v.set(Some(Arg::new(c"x")));
    assert_eq!(value.get().map(Arg::as_c_str), Some(c"x"));
    assert_eq!(mode.get(), 7);
    spec.as_mut().set_target(Target::Literal);
    assert!(matches!(spec.as_ref().target(), Some(Target::Literal)));
}

/// A variable shared by two specs is written through either, as two switches selecting one
/// mode share it.
#[test]
fn specs_share_a_variable() {
    let mode = Cell::new(0);
    let mut read = Spec::new(Target::Switch(Some(&mode)));
    read.as_mut().set_switch_value(1);
    let mut write = Spec::new(Target::Switch(Some(&mode)));
    write.as_mut().set_switch_value(2);
    for spec in [&read, &write] {
        let r = spec.as_ref();
        let Some(Target::Switch(Some(v))) = r.target() else {
            panic!("{:?}", r.target());
        };
        v.set(r.switch_value());
        assert_eq!(mode.get(), r.switch_value());
    }
    assert_eq!(mode.get(), 2);
}

/// The strings, alias, switch value and usage read back as set, and clear back to none.
#[test]
fn spec_fields_round_trip() {
    let mut spec = Spec::new(Target::Value(None));
    let mut m = spec.as_mut();
    m.set_name(Some(c"config-env"));
    m.set_alias(NonZero::new(b'c'));
    m.set_switch_value(-3);
    m.set_usage(Usage::HIDDEN | Usage::VALUE_OPTIONAL);
    m.set_value_name(Some(c"key=value"));
    m.set_help(Some(c"set configuration value to environment variable"));
    let r = spec.as_ref();
    assert_eq!(r.name(), Some(c"config-env"));
    assert_eq!(r.alias().map(NonZero::get), Some(b'c'));
    assert_eq!(r.switch_value(), -3);
    assert_eq!(r.usage(), Usage::HIDDEN | Usage::VALUE_OPTIONAL);
    assert_eq!(r.value_name(), Some(c"key=value"));
    assert_eq!(
        r.help(),
        Some(c"set configuration value to environment variable")
    );

    let mut m = spec.as_mut();
    m.set_name(None);
    m.set_alias(None);
    m.set_value_name(None);
    m.set_help(None);
    let r = spec.as_ref();
    assert_eq!(
        (r.name(), r.alias(), r.value_name(), r.help()),
        (None, None, None, None)
    );
}

/// The builders set what the setters set, a zero alias leaving the spec without one, and
/// their specs chain into a table as C's initialisers do.
#[test]
fn builders_set_what_the_setters_set() {
    let mode = Cell::new(0);
    let built = Spec::new(Target::Switch(Some(&mode)))
        .with_name(c"read")
        .with_alias(b'r')
        .with_switch_value(2)
        .with_usage(Usage::CHOICE | Usage::SHOW_LONG)
        .with_value_name(c"mode")
        .with_help(c"read the mode");
    let mut set = Spec::new(Target::Switch(Some(&mode)));
    let mut m = set.as_mut();
    m.set_name(Some(c"read"));
    m.set_alias(NonZero::new(b'r'));
    m.set_switch_value(2);
    m.set_usage(Usage::CHOICE | Usage::SHOW_LONG);
    m.set_value_name(Some(c"mode"));
    m.set_help(Some(c"read the mode"));
    let (b, s) = (built.as_ref(), set.as_ref());
    assert_eq!(b.kind(), s.kind());
    assert_eq!(b.name(), s.name());
    assert_eq!(b.alias(), s.alias());
    assert_eq!(b.switch_value(), s.switch_value());
    assert_eq!(b.usage(), s.usage());
    assert_eq!(b.value_name(), s.value_name());
    assert_eq!(b.help(), s.help());
    assert_eq!(format!("{built:?}"), format!("{set:?}"));

    assert_eq!(Spec::default().with_alias(0).as_ref().alias(), None);
    let specs = [built, Spec::new(Target::Literal), Spec::default()];
    let table = SpecTable::new(&specs).expect("ended");
    assert_eq!(table.len(), 2);
    assert_eq!(table.get(0).and_then(|s| s.name()), Some(c"read"));
}

/// An alias above ASCII keeps its byte, as C's `char` holds it.
#[test]
fn a_non_ascii_alias_keeps_its_byte() {
    let mut spec = Spec::zeroed();
    spec.as_mut().set_alias(NonZero::new(0xE9));
    assert_eq!(spec.as_ref().alias().map(NonZero::get), Some(0xE9));
}

/// A spec's `Debug` shows its fields through its handles.
#[test]
fn spec_debug_shows_its_fields() {
    let mut spec = Spec::new(Target::Literal);
    spec.as_mut().set_name(Some(c"lit"));
    let shown = format!("{spec:?}");
    assert!(shown.contains("Literal"), "{shown}");
    assert!(shown.contains("\"lit\""), "{shown}");
    assert_eq!(format!("{:?}", spec.as_mut()), shown);
}

/// A table runs up to its first zeroed spec, and needs one.
#[test]
fn a_table_ends_at_its_first_zeroed_spec() {
    let flag = Cell::new(0);
    let specs = [
        Spec::new(Target::Bool(Some(&flag))),
        Spec::new(Target::Literal),
        Spec::zeroed(),
        Spec::new(Target::Bool(None)),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    assert_eq!(table.len(), 2);
    assert!(!table.is_empty());
    let kinds: Vec<_> = table.iter().map(|s| s.kind()).collect();
    assert_eq!(kinds, [Some(Kind::Bool), Some(Kind::Literal)]);
    assert_eq!(table.get(1).and_then(|s| s.kind()), Some(Kind::Literal));
    assert!(table.get(2).is_none());
    assert_eq!(format!("{table:?}").matches("Spec").count(), 2);

    let unended = [Spec::new(Target::Literal)];
    assert!(SpecTable::new(&unended).is_none());
    assert!(SpecTable::new(&[]).is_none());

    let only_end = [Spec::zeroed()];
    let empty = SpecTable::new(&only_end).expect("ended");
    assert!(empty.is_empty());
    assert_eq!(empty.iter().count(), 0);
}

/// A spec's position is found by address in its own table only, never at the end.
#[test]
fn positions_are_found_by_address() {
    let specs = [
        Spec::new(Target::Literal),
        Spec::new(Target::Arg(None)),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    for (i, spec) in table.iter().enumerate() {
        assert_eq!(table.position(spec), Some(i));
    }
    assert_eq!(table.position(specs[2].as_ref()), None);
    let other = Spec::new(Target::Literal);
    assert_eq!(table.position(other.as_ref()), None);
    let tail = SpecTable::new(&specs[1..]).expect("ended");
    assert_eq!(tail.position(specs[0].as_ref()), None);
    assert_eq!(tail.position(specs[1].as_ref()), Some(0));
}

/// An argument lends back the string it was built from, and shows it in `Debug`.
#[test]
fn an_arg_lends_its_string() {
    let s = c"--debug";
    let arg = Arg::new(s);
    assert_eq!(arg.as_c_str(), s);
    assert_eq!(arg.as_c_str().as_ptr(), s.as_ptr());
    assert_eq!(Arg::from(s).as_c_str(), s);
    assert_eq!(format!("{arg:?}"), format!("{s:?}"));
}

/// An empty slot finds nothing; a set one finds the tail it was given in its own array, and
/// nothing in another; clearing empties it.
#[test]
fn an_args_slot_finds_its_tail() {
    let mut args = [Arg::new(c"a"), Arg::new(c"b"), Arg::new(c"c")];
    let other = [Arg::new(c"a")];
    let slot = ArgsSlot::default();
    assert!(!slot.is_set());
    assert!(slot.get_in(&args).is_none());

    slot.set(&args[1..]);
    assert!(slot.is_set());
    let tail: Vec<_> = slot
        .get_in(&args)
        .expect("in args")
        .iter()
        .map(|a| a.as_c_str())
        .collect();
    assert_eq!(tail, [c"b", c"c"]);
    assert!(slot.get_in(&other).is_none());
    assert!(slot.get_in(&args[2..]).is_none());
    assert_eq!(slot.get_in(&args[..2]).map(<[Arg<'_>]>::len), Some(1));

    slot.set(&args[3..]);
    assert_eq!(slot.get_in(&args).map(<[Arg<'_>]>::len), Some(0));
    args[0] = Arg::new(c"z");
    assert_eq!(slot.get_in(&args).map(<[Arg<'_>]>::len), Some(0));
    assert!(format!("{slot:?}").starts_with("ArgsSlot("));

    slot.clear();
    assert!(!slot.is_set());
    assert!(slot.get_in(&args).is_none());
}

/// A zeroed outcome is done, with no spec, argument, value or arguments taken.
#[test]
fn a_zeroed_opt_is_done() {
    for opt in [Opt::zeroed(), Opt::default()] {
        let r = opt.as_ref();
        assert_eq!(r.status(), Some(Status::Done));
        assert!(r.spec().is_none());
        assert_eq!((r.arg(), r.value(), r.args_len()), (None, None, 0));
    }
}

/// Every field of an outcome reads back as set, its spec by address.
#[test]
fn opt_fields_round_trip() {
    let specs = [Spec::new(Target::Value(None)), Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let arg = c"--name=value";
    let mut opt = Opt::zeroed();
    let spec = table.get(0).expect("one spec");
    let mut m = opt.as_mut();
    m.set_status(Status::MissingValue);
    m.set_spec(Some(spec));
    m.set_arg(Some(arg));
    m.set_value(Some(c"value"));
    m.set_args_len(4);
    let r = opt.as_ref();
    assert_eq!(r.status(), Some(Status::MissingValue));
    assert_eq!(r.spec().map(|s| table.position(s)), Some(Some(0)));
    assert_eq!(r.arg(), Some(arg));
    assert_eq!(r.value(), Some(c"value"));
    assert_eq!(r.args_len(), 4);
    assert!(format!("{opt:?}").contains("MissingValue"));

    let mut m = opt.as_mut();
    m.set_spec(None);
    m.set_arg(None);
    m.set_value(None);
    assert!(format!("{m:?}").contains("spec: None"));
    let r = opt.as_ref();
    assert!(r.spec().is_none());
    assert_eq!((r.arg(), r.value()), (None, None));
}

/// Every status round-trips through an outcome.
#[test]
fn statuses_round_trip_through_an_opt() {
    let mut opt = Opt::zeroed();
    for status in [
        Status::Done,
        Status::Ok,
        Status::UnknownOption,
        Status::MissingValue,
        Status::MissingArgument,
    ] {
        opt.as_mut().set_status(status);
        assert_eq!(opt.as_ref().status(), Some(status));
    }
}

/// A zeroed parser has no table and no arguments, every position and bit clear.
#[test]
fn a_zeroed_parser_is_empty() {
    for parser in [Parser::zeroed(), Parser::default()] {
        let r = parser.as_ref();
        assert!(r.specs().is_none());
        assert!(r.args().is_empty());
        assert_eq!(r.args_len(), 0);
        assert_eq!(r.flags(), ParseFlags::DEFAULT);
        assert_eq!(
            (r.idx(), r.arg_idx(), r.in_args(), r.in_short()),
            (0, 0, 0, 0)
        );
        assert!(!r.needs_sort());
        assert!(!r.in_literal());
    }
    let mut parser = Parser::zeroed();
    assert!(parser.as_mut().args_mut().is_empty());
}

/// A parser's table, arguments and their count, and its state read back as set; its two bits
/// are independent of each other.
#[test]
fn parser_fields_round_trip() {
    let specs = [Spec::new(Target::Arg(None)), Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"one"), Arg::new(c"--two")];
    let mut parser = Parser::zeroed();
    let mut m = parser.as_mut();
    m.set_specs(Some(table));
    m.set_args(&mut args);
    m.set_flags(ParseFlags::GNU);
    m.set_idx(1);
    m.set_arg_idx(2);
    m.set_in_args(3);
    m.set_in_short(4);
    m.set_needs_sort(true);
    let r = parser.as_ref();
    let read = r.specs().expect("a table");
    assert_eq!(read.len(), 1);
    assert_eq!(read.position(table.get(0).expect("one spec")), Some(0));
    let args_read: Vec<_> = r.args().iter().map(|a| a.as_c_str()).collect();
    assert_eq!(args_read, [c"one", c"--two"]);
    assert_eq!(r.args_len(), 2);
    assert_eq!(r.flags(), ParseFlags::GNU);
    assert_eq!(
        (r.idx(), r.arg_idx(), r.in_args(), r.in_short()),
        (1, 2, 3, 4)
    );
    assert!(r.needs_sort());
    assert!(!r.in_literal());

    let mut m = parser.as_mut();
    m.set_in_literal(true);
    assert!(m.as_ref().needs_sort() && m.as_ref().in_literal());
    m.set_needs_sort(false);
    assert!(!m.as_ref().needs_sort() && m.as_ref().in_literal());
    m.set_in_literal(false);
    assert!(!m.as_ref().needs_sort() && !m.as_ref().in_literal());
    m.set_specs(None);
    assert!(m.as_ref().specs().is_none());
    assert!(format!("{m:?}").contains("specs: None"));
}

/// The arguments a parser holds reorder in place, as a GNU-style parse moves its options, and
/// an outcome or variable keeps the strings it took.
#[test]
fn a_parsers_arguments_reorder_in_place() {
    let mut args = [Arg::new(c"file"), Arg::new(c"--help")];
    let taken = Cell::new(None);
    let mut parser = Parser::zeroed();
    parser.as_mut().set_args(&mut args);
    taken.set(parser.as_ref().args().first().copied());
    parser.as_mut().args_mut().swap(0, 1);
    let order: Vec<_> = parser
        .as_ref()
        .args()
        .iter()
        .map(|a| a.as_c_str())
        .collect();
    assert_eq!(order, [c"--help", c"file"]);
    assert_eq!(taken.get().map(Arg::as_c_str), Some(c"file"));
    assert!(format!("{parser:?}").contains("\"--help\""));
}

/// A new parser holds its table, its arguments and its flags, unnamed bits included, at its
/// first argument with every other piece of state clear; it reorders under `FORCE_GNU` alone
/// of the flags whose style no environment changes.
#[test]
fn a_new_parser_starts_at_its_first_argument() {
    for (flags, sorts) in [
        (ParseFlags::DEFAULT, false),
        (ParseFlags::FORCE_GNU, true),
        (ParseFlags::from_bits_retain(1 << 7), false),
        (
            ParseFlags::FORCE_GNU | ParseFlags::from_bits_retain(1 << 31),
            true,
        ),
    ] {
        let mut args = [Arg::new(c"one"), Arg::new(c"--two")];
        let specs = [Spec::new(Target::Arg(None)), Spec::zeroed()];
        let table = SpecTable::new(&specs).expect("ended");
        let parser = Parser::new(table, &mut args, flags);
        let r = parser.as_ref();
        assert_eq!(
            r.specs().map(|t| t.position(table.get(0).unwrap())),
            Some(Some(0))
        );
        let read: Vec<_> = r.args().iter().map(|a| a.as_c_str()).collect();
        assert_eq!(read, [c"one", c"--two"]);
        assert_eq!((r.args_len(), r.flags()), (2, flags));
        assert_eq!(
            (r.idx(), r.arg_idx(), r.in_args(), r.in_short()),
            (0, 0, 0, 0)
        );
        assert_eq!(r.needs_sort(), sorts, "{flags:?}");
        assert!(!r.in_literal());
    }
    let mut none: [Arg<'_>; 0] = [];
    let specs = [Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let parser = Parser::new(table, &mut none, ParseFlags::DEFAULT);
    assert!(parser.as_ref().args().is_empty());
}

/// The message for `opt`, written to a `Vec`.
fn status<'p>(opt: &Opt<'p>, command: Option<&str>, table: SpecTable<'p>) -> Vec<u8> {
    let mut out = Vec::new();
    opt.as_ref()
        .write_status(&mut out, command, table)
        .expect("written");
    out
}

/// An outcome of `status` matching `spec` of `table`, for argument `arg`.
fn outcome<'p>(
    status: Status,
    spec: Option<usize>,
    arg: Option<&'p CStr>,
    table: SpecTable<'p>,
) -> Opt<'p> {
    let mut opt = Opt::zeroed();
    opt.as_mut().set_status(status);
    opt.as_mut().set_spec(spec.and_then(|i| table.get(i)));
    opt.as_mut().set_arg(arg);
    opt
}

/// An outcome's status, spec position and argument, with its message.
type Case<'a> = (Status, Option<usize>, Option<&'a CStr>, &'a [u8]);

/// The message of each status, after the command when it is given.
#[test]
fn each_status_has_its_message() {
    let specs = [
        Spec::new(Target::Value(None))
            .with_alias(b'o')
            .with_name(c"output"),
        Spec::new(Target::Arg(None)).with_value_name(c"<path>"),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let cases: [Case<'_>; 7] = [
        (
            Status::Done,
            None,
            None,
            b"finished processing arguments (no error)\n",
        ),
        (Status::Ok, Some(0), Some(c"-o"), b"no error\n"),
        (
            Status::UnknownOption,
            None,
            Some(c"--nope"),
            b"unknown option: --nope\n",
        ),
        (
            Status::UnknownOption,
            None,
            None,
            b"unknown option: (null)\n",
        ),
        (
            Status::MissingValue,
            Some(0),
            Some(c"-o"),
            b"argument '-o' requires a value.\n",
        ),
        (
            Status::MissingArgument,
            Some(1),
            None,
            b"argument '<path>' is required.\n",
        ),
        (
            Status::MissingArgument,
            Some(0),
            None,
            b"argument '-o' is required.\n",
        ),
    ];
    for (st, spec, arg, message) in cases {
        let opt = outcome(st, spec, arg, table);
        assert_eq!(status(&opt, None, table), message, "{st:?}");
        let mut prefixed = b"git2: ".to_vec();
        prefixed.extend_from_slice(message);
        assert_eq!(status(&opt, Some("git2"), table), prefixed, "{st:?}");
        assert_eq!(status(&opt, Some(""), table)[..2], *b": ");
    }
}

/// A required choice names each of its options in order: `either` two, `one of` more, from the
/// spec the outcome names to the end of its choice.
#[test]
fn a_required_choice_names_its_options() {
    let specs = [
        Spec::new(Target::Switch(None)).with_alias(b'a'),
        Spec::new(Target::Switch(None))
            .with_name(c"bee")
            .with_usage(Usage::CHOICE),
        Spec::new(Target::Arg(None))
            .with_value_name(c"<c>")
            .with_usage(Usage::CHOICE),
        Spec::new(Target::Args(None))
            .with_value_name(c"<d>...")
            .with_usage(Usage::CHOICE),
        Spec::new(Target::Switch(None)).with_alias(b'e'),
        Spec::new(Target::Switch(None))
            .with_alias(b'f')
            .with_usage(Usage::CHOICE),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let cases: [(usize, &[u8]); 5] = [
        (0, b"one of '-a', '--bee', '<c>' or '<d>...' is required.\n"),
        (1, b"one of '--bee', '<c>' or '<d>...' is required.\n"),
        (2, b"either '<c>' or '<d>...' is required.\n"),
        (3, b"argument '<d>...' is required.\n"),
        (4, b"either '-e' or '-f' is required.\n"),
    ];
    for (spec, message) in cases {
        let opt = outcome(Status::MissingArgument, Some(spec), None, table);
        assert_eq!(status(&opt, None, table), message, "{spec}");
    }
}

/// A spec is named by its alias, by its long name when shown long or without an alias, and by
/// its value name for a bare argument; a missing name, and an argument's bytes that are not
/// UTF-8, are written as C writes them.
#[test]
fn specs_are_named_as_c_names_them() {
    let specs = [
        Spec::new(Target::Switch(None))
            .with_alias(b'f')
            .with_name(c"force")
            .with_usage(Usage::SHOW_LONG),
        Spec::new(Target::Bool(None)),
        Spec::new(Target::Arg(None)),
        Spec::new(Target::Switch(None)).with_alias(0xE9),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let named = |i| {
        status(
            &outcome(Status::MissingValue, Some(i), None, table),
            None,
            table,
        )
    };
    assert_eq!(named(0), b"argument '--force' requires a value.\n");
    assert_eq!(named(1), b"argument '--(null)' requires a value.\n");
    assert_eq!(named(2), b"argument '(null)' requires a value.\n");
    assert_eq!(named(3), b"argument '-\xe9' requires a value.\n");
    let opt = outcome(Status::UnknownOption, None, Some(c"--caf\xe9"), table);
    assert_eq!(status(&opt, None, table), b"unknown option: --caf\xe9\n");
}

/// A spec outside the table given is named alone, its choices unknown.
#[test]
fn a_spec_outside_the_table_is_named_alone() {
    let specs = [
        Spec::new(Target::Switch(None)).with_alias(b'a'),
        Spec::new(Target::Switch(None))
            .with_alias(b'b')
            .with_usage(Usage::CHOICE),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let others = [Spec::zeroed()];
    let empty = SpecTable::new(&others).expect("ended");
    let mut opt = Opt::zeroed();
    opt.as_mut().set_status(Status::MissingArgument);
    opt.as_mut().set_spec(table.get(0));
    let mut out = Vec::new();
    opt.as_ref().write_status(&mut out, None, empty).unwrap();
    assert_eq!(out, b"argument '-a' is required.\n");
}

/// Each write a writer is given, kept apart, failing from the `fail_at`th on.
struct Pieces {
    pieces: Vec<Vec<u8>>,
    fail_at: usize,
}

impl io::Write for Pieces {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.pieces.len() == self.fail_at {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        self.pieces.push(buf.to_vec());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The message is written in the pieces C's `fprintf` calls write, and stops at the first that
/// fails, whose error it returns, the pieces before it written.
#[test]
fn the_message_is_written_in_c_pieces_until_one_fails() {
    let specs = [
        Spec::new(Target::Switch(None)).with_alias(b'a'),
        Spec::new(Target::Switch(None))
            .with_alias(b'b')
            .with_usage(Usage::CHOICE),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let opt = outcome(Status::MissingArgument, Some(0), None, table);
    let all: [&[u8]; 10] = [
        b"git2: ",
        b"either",
        b" '",
        b"-a",
        b"'",
        b" or",
        b" '",
        b"-b",
        b"'",
        b" is required.\n",
    ];
    let mut out = Pieces {
        pieces: Vec::new(),
        fail_at: usize::MAX,
    };
    opt.as_ref()
        .write_status(&mut out, Some("git2"), table)
        .unwrap();
    assert_eq!(out.pieces, all);
    for fail_at in 0..10 {
        let mut out = Pieces {
            pieces: Vec::new(),
            fail_at,
        };
        let e = opt
            .as_ref()
            .write_status(&mut out, Some("git2"), table)
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(out.pieces, all[..fail_at], "{fail_at}");
    }
    let opt = outcome(Status::MissingValue, Some(1), None, table);
    let mut out = Pieces {
        pieces: Vec::new(),
        fail_at: usize::MAX,
    };
    opt.as_ref().write_status(&mut out, None, table).unwrap();
    assert_eq!(
        out.pieces,
        [&b"argument '"[..], b"-b", b"' requires a value.\n"]
    );
}

/// An outcome that misses a value or an argument names a spec: one that names none is refused,
/// with nothing written, where the C reads through a null pointer.
#[test]
fn a_missing_spec_is_refused_unwritten() {
    let specs = [Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    for st in [Status::MissingValue, Status::MissingArgument] {
        let opt = outcome(st, None, Some(c"-x"), table);
        let mut out = Vec::new();
        let e = opt
            .as_ref()
            .write_status(&mut out, Some("git2"), table)
            .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{st:?}");
        assert_eq!(out, b"");
    }
}

/// A closure's callback is called with each outcome and returns what the closure returns; the
/// strings the outcomes lend outlive them.
#[test]
fn a_closure_callback_sees_each_outcome() {
    let specs = [
        Spec::new(Target::Value(None)).with_alias(b'o'),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut kept: Vec<(Option<Status>, Option<&CStr>)> = Vec::new();
    let mut calls = 0;
    {
        let mut f = |opt: OptRef<'_, '_>| {
            calls += 1;
            if opt.status() == Some(Status::UnknownOption) {
                ControlFlow::Break(NonZero::new(-3).unwrap())
            } else {
                ControlFlow::Continue(())
            }
        };
        let mut callback = ForeachCallbackFn::new(&mut f);
        for (st, arg, flow) in [
            (Status::Ok, c"-ofile", ControlFlow::Continue(())),
            (
                Status::UnknownOption,
                c"--x",
                ControlFlow::Break(NonZero::new(-3).unwrap()),
            ),
        ] {
            let opt = outcome(st, Some(0), Some(arg), table);
            assert_eq!(callback.call(opt.as_ref()), flow);
        }
        assert_eq!(format!("{callback:?}"), "ForeachCallbackFn(Rust)");
    }
    assert_eq!(calls, 2);
    let mut keep = |opt: OptRef<'_, 'static>| {
        kept.push((opt.status(), opt.arg()));
        ControlFlow::Continue(())
    };
    {
        let mut callback = ForeachCallbackFn::new(&mut keep);
        let mut opt = Opt::zeroed();
        opt.as_mut().set_status(Status::Ok);
        opt.as_mut().set_arg(Some(c"--kept"));
        assert_eq!(callback.call(opt.as_ref()), ControlFlow::Continue(()));
    }
    assert_eq!(kept, [(Some(Status::Ok), Some(c"--kept"))]);
}

/// What one outcome of a parse shows: its status, its spec's position in the table, and its
/// argument and value.
type Step<'p> = (
    Option<Status>,
    Option<usize>,
    Option<&'p CStr>,
    Option<&'p CStr>,
);

/// Every outcome `parser` gives until it is done, the done one included, as [`Step`]s.
fn steps<'p>(parser: &mut Parser<'p>, table: SpecTable<'p>) -> Vec<Step<'p>> {
    let mut steps = Vec::new();
    loop {
        let opt = parser.as_mut().parse_next();
        let o = opt.as_ref();
        steps.push((
            o.status(),
            o.spec().and_then(|s| table.position(s)),
            o.arg(),
            o.value(),
        ));
        if o.status() == Some(Status::Done) {
            return steps;
        }
        assert!(steps.len() < 32, "no end");
    }
}

/// The strings of `args`, in order.
fn strings<'p>(args: &[Arg<'p>]) -> Vec<&'p CStr> {
    args.iter().map(|a| a.as_c_str()).collect()
}

/// `f`, as the callback of a parse over a table borrowed for `'p`.
fn callback<'p, F>(f: F) -> F
where
    F: FnMut(OptRef<'_, 'p>) -> ControlFlow<NonZero<c_int>>,
{
    f
}

/// Each kind of option writes its variable as it is given, long and short, with its value from
/// after an `=`, from the rest of its argument or from the next one, and a bundle gives one
/// outcome per option, each naming the whole argument.
#[test]
fn each_option_writes_its_variable() {
    let (debug, verbose, mode) = (Cell::new(7), Cell::new(0), Cell::new(0));
    let (output, file) = (Cell::new(None), Cell::new(None));
    let specs = [
        Spec::new(Target::Bool(Some(&debug)))
            .with_name(c"debug")
            .with_alias(b'd'),
        Spec::new(Target::Accumulator(Some(&verbose))).with_alias(b'v'),
        Spec::new(Target::Accumulator(Some(&verbose)))
            .with_alias(b'q')
            .with_switch_value(-2),
        Spec::new(Target::Switch(Some(&mode)))
            .with_name(c"write")
            .with_switch_value(2),
        Spec::new(Target::Value(Some(&output)))
            .with_name(c"output")
            .with_alias(b'o'),
        Spec::new(Target::Arg(Some(&file))),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [
        c"--no-debug",
        c"-vvq",
        c"--write",
        c"--output=a",
        c"--output",
        c"b",
        c"-oc",
        c"-do",
        c"d",
        c"file",
    ]
    .map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let ok = Some(Status::Ok);
    assert_eq!(
        steps(&mut parser, table),
        [
            (ok, Some(0), Some(c"--no-debug"), None),
            (ok, Some(1), Some(c"-vvq"), None),
            (ok, Some(1), Some(c"-vvq"), None),
            (ok, Some(2), Some(c"-vvq"), None),
            (ok, Some(3), Some(c"--write"), None),
            (ok, Some(4), Some(c"--output=a"), Some(c"a")),
            (ok, Some(4), Some(c"--output"), Some(c"b")),
            (ok, Some(4), Some(c"-oc"), Some(c"c")),
            (ok, Some(0), Some(c"-do"), None),
            (ok, Some(4), Some(c"-do"), Some(c"d")),
            (ok, Some(5), Some(c"file"), None),
            (Some(Status::Done), None, None, None),
        ]
    );
    assert_eq!((debug.get(), verbose.get(), mode.get()), (1, 0, 2));
    assert_eq!(output.get().map(Arg::as_c_str), Some(c"d"));
    assert_eq!(file.get().map(Arg::as_c_str), Some(c"file"));
}

/// A value option with nothing to take is a missing value, unless its value is optional, and
/// `--name=` gives it none: the next argument is taken whatever it looks like.
#[test]
fn a_value_option_takes_what_follows_or_lacks_it() {
    let (output, sign) = (Cell::new(None), Cell::new(None));
    let specs = [
        Spec::new(Target::Value(Some(&output)))
            .with_name(c"output")
            .with_alias(b'o'),
        Spec::new(Target::Value(Some(&sign)))
            .with_alias(b'S')
            .with_usage(Usage::VALUE_OPTIONAL),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"--output=", c"-o", c"-S", c"-S"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let missing = Some(Status::MissingValue);
    assert_eq!(
        steps(&mut parser, table),
        [
            (missing, Some(0), Some(c"--output="), None),
            (Some(Status::Ok), Some(0), Some(c"-o"), Some(c"-S")),
            (Some(Status::Ok), Some(1), Some(c"-S"), None),
            (Some(Status::Done), None, None, None),
        ]
    );
    assert_eq!(output.get().map(Arg::as_c_str), Some(c"-S"));
    assert_eq!(sign.get().map(Arg::as_c_str), None);
    let mut args = [c"-o"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    assert_eq!(steps(&mut parser, table)[0].0, missing);
}

/// Bare arguments take the argument specs in order and then the remaining-arguments spec, which
/// takes every argument left, options included, and is counted once the parse is done; with
/// every argument spec taken, a bare argument is unknown.
#[test]
fn bare_arguments_take_the_argument_specs() {
    let (first, rest) = (Cell::new(None), ArgsSlot::new());
    let specs = [
        Spec::new(Target::Bool(None)).with_name(c"debug"),
        Spec::new(Target::Arg(Some(&first))),
        Spec::new(Target::Args(Some(&rest))),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"a", c"b", c"--debug", c"c"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let opt = parser.as_mut().parse_next();
    assert_eq!(opt.as_ref().spec().and_then(|s| table.position(s)), Some(1));
    let opt = parser.as_mut().parse_next();
    assert_eq!(opt.as_ref().spec().and_then(|s| table.position(s)), Some(2));
    assert_eq!(opt.as_ref().args_len(), 3);
    let done = parser.as_mut().parse_next();
    assert_eq!(done.as_ref().status(), Some(Status::Done));
    assert_eq!(done.as_ref().args_len(), 3);
    let tail = rest.get_in(parser.as_ref().args()).unwrap();
    assert_eq!(strings(tail), [c"b", c"--debug", c"c"]);
    assert_eq!(first.get().map(Arg::as_c_str), Some(c"a"));

    let specs = [Spec::new(Target::Arg(None)), Spec::default()];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"a", c"b"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let unknown = Some(Status::UnknownOption);
    assert_eq!(
        steps(&mut parser, table)[1],
        (unknown, None, Some(c"b"), None)
    );
}

/// GNU style moves the options after the first bare argument ahead of it, with the values they
/// take, once, keeping both orders; without it, every argument after the first bare one is bare.
#[test]
fn gnu_style_moves_options_ahead_of_bare_arguments() {
    let (debug, output, rest) = (Cell::new(0), Cell::new(None), ArgsSlot::new());
    let specs = [
        Spec::new(Target::Bool(Some(&debug))).with_name(c"debug"),
        Spec::new(Target::Value(Some(&output))).with_alias(b'o'),
        Spec::new(Target::Args(Some(&rest))),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let line = [c"a", c"-o", c"out", c"b", c"--debug", c"-ox", c"c"];
    let mut args = line.map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    let opt = parser.as_mut().parse_next();
    assert_eq!(opt.as_ref().value(), Some(c"out"));
    assert!(!parser.as_ref().needs_sort());
    assert_eq!(
        strings(parser.as_ref().args()),
        [c"-o", c"out", c"--debug", c"-ox", c"a", c"b", c"c"]
    );
    while parser.as_mut().parse_next().as_ref().status() != Some(Status::Done) {}
    assert_eq!(
        (debug.get(), output.get().map(Arg::as_c_str)),
        (1, Some(c"x"))
    );
    let tail = rest.get_in(parser.as_ref().args()).unwrap();
    assert_eq!(strings(tail), [c"a", c"b", c"c"]);

    let mut args = line.map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    assert_eq!(steps(&mut parser, table).len(), 2);
    assert_eq!(strings(parser.as_ref().args()), line);
}

/// A GNU-style parse that meets a required value with nothing after it reports it before the
/// bare arguments, and a literal `--` it meets ends the reordering.
#[test]
fn gnu_style_stops_at_a_missing_value_and_a_literal() {
    let specs = [
        Spec::new(Target::Bool(None)).with_alias(b'd'),
        Spec::new(Target::Value(None)).with_alias(b'o'),
        Spec::new(Target::Literal),
        Spec::new(Target::Args(None)),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"a", c"-d", c"-o"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    let missing = (Some(Status::MissingValue), Some(1), Some(c"-o"), None);
    assert_eq!(steps(&mut parser, table)[0], missing);

    let mut args = [c"a", c"-d", c"--", c"-d", c"b"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    let _ = parser.as_mut().parse_next();
    assert_eq!(
        strings(parser.as_ref().args()),
        [c"-d", c"a", c"--", c"-d", c"b"]
    );
}

/// Parsing ends at the arguments' end, at any position past it, and in a parser with no table,
/// whose arguments are unknown; a position in a bundle past its argument's end is unknown too.
#[test]
fn a_parser_ends_and_refuses_without_reading_past() {
    let mut p = Parser::default();
    assert_eq!(
        p.as_mut().parse_next().as_ref().status(),
        Some(Status::Done)
    );

    let specs = [
        Spec::new(Target::Bool(None)).with_alias(b'd'),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"-d", c"x"].map(Arg::new);
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    parser.as_mut().set_idx(usize::MAX);
    assert_eq!(
        parser.as_mut().parse_next().as_ref().status(),
        Some(Status::Done)
    );
    parser.as_mut().set_idx(0);
    parser.as_mut().set_in_short(usize::MAX);
    let opt = parser.as_mut().parse_next();
    assert_eq!(opt.as_ref().status(), Some(Status::UnknownOption));
    assert_eq!(parser.as_ref().in_short(), 0);
    parser.as_mut().set_idx(0);
    parser.as_mut().set_in_short(5);
    assert_eq!(
        parser.as_mut().parse_next().as_ref().status(),
        Some(Status::UnknownOption)
    );

    let mut args = [c"--debug", c"-d", c"x"].map(Arg::new);
    let mut tableless = Parser::default();
    tableless.as_mut().set_args(&mut args);
    let statuses: Vec<_> = (0..4)
        .map(|_| tableless.as_mut().parse_next().as_ref().status())
        .collect();
    let unknown = Some(Status::UnknownOption);
    assert_eq!(statuses, [unknown, unknown, unknown, Some(Status::Done)]);
}

/// The bare `-` is a bare argument, in either style, as POSIX `getopt` and GNU `getopt_long` take
/// it, where the C matches the first spec without an alias, the CLI's `--help`, and reads past the
/// argument: it takes an argument spec, stays among the bare arguments a GNU-style parse moves an
/// option ahead of, and is unknown once the argument specs are taken, as any bare argument is.
///
/// The regression test of `logic/bare-dash-is-an-operand` (`wave-186/batch-7`).
#[test]
fn a_bare_dash_is_a_bare_argument() {
    let help = Cell::new(0);
    for flags in [ParseFlags::DEFAULT, ParseFlags::FORCE_GNU] {
        let (quiet, file) = (Cell::new(0), Cell::new(None));
        let specs = [
            Spec::new(Target::Switch(Some(&help)))
                .with_name(c"help")
                .with_switch_value(1)
                .with_usage(Usage::HIDDEN | Usage::STOP_PARSING),
            Spec::new(Target::Bool(Some(&quiet))).with_alias(b'q'),
            Spec::new(Target::Arg(Some(&file))),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();

        let mut alone = [c"-"].map(Arg::new);
        let mut parser = Parser::new(table, &mut alone, flags);
        let bare = (Some(Status::Ok), Some(2), Some(c"-"), None);
        let done = (Some(Status::Done), None, None, None);
        assert_eq!(steps(&mut parser, table), [bare, done], "{flags:?}");
        assert_eq!(file.get().map(Arg::as_c_str), Some(c"-"), "{flags:?}");

        file.set(None);
        let mut after = [c"-q", c"-"].map(Arg::new);
        assert!(parse(table, &mut after, flags).is_ok(), "{flags:?}");
        assert_eq!(
            (quiet.get(), file.get().map(Arg::as_c_str)),
            (1, Some(c"-"))
        );

        file.set(None);
        quiet.set(0);
        let mut before = [c"-", c"-q"].map(Arg::new);
        let (_, left) = parse(table, &mut before, ParseFlags::FORCE_GNU).unwrap();
        assert_eq!(strings(left), [c"-q", c"-"]);
        assert_eq!(
            (quiet.get(), file.get().map(Arg::as_c_str)),
            (1, Some(c"-"))
        );

        let mut two = [c"a", c"-"].map(Arg::new);
        let mut parser = Parser::new(table, &mut two, flags);
        let unknown = (Some(Status::UnknownOption), None, Some(c"-"), None);
        assert_eq!(steps(&mut parser, table)[1], unknown, "{flags:?}");
    }
    assert_eq!(help.get(), 0);
}

/// An unknown option ends its bundle wherever it is in it: the next argument parses from its
/// start, where the C reads it from as far in as the bundle had gone, past its end here.
#[test]
fn an_unknown_option_ends_its_bundle() {
    for bundle in [c"-xd", c"-dx", c"-ddxd"] {
        let mut args = [Arg::new(bundle), Arg::new(c"y")];
        let (debug, file) = (Cell::new(0), Cell::new(None));
        let specs = [
            Spec::new(Target::Bool(Some(&debug))).with_alias(b'd'),
            Spec::new(Target::Arg(Some(&file))),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
        let steps = steps(&mut parser, table);
        let unknown = steps
            .iter()
            .position(|s| s.0 == Some(Status::UnknownOption))
            .unwrap();
        assert_eq!(steps[unknown].2, Some(bundle));
        assert_eq!(
            steps[unknown + 1],
            (Some(Status::Ok), Some(1), Some(c"y"), None)
        );
        assert_eq!(file.get().map(Arg::as_c_str), Some(c"y"), "{bundle:?}");
    }
}

/// A literal `--` parsed before the first bare argument makes every later argument literal, and
/// a GNU-style parse moves none of them, where the C moves those that look like options ahead
/// of the others.
#[test]
fn nothing_after_a_literal_is_reordered() {
    let (debug, first, second) = (Cell::new(0), Cell::new(None), Cell::new(None));
    let specs = [
        Spec::new(Target::Bool(Some(&debug))).with_alias(b'd'),
        Spec::new(Target::Literal),
        Spec::new(Target::Arg(Some(&first))),
        Spec::new(Target::Arg(Some(&second))),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"--", c"a", c"-d"].map(Arg::new);
    let (done, args) = parse(table, &mut args, ParseFlags::FORCE_GNU).unwrap();
    assert_eq!(done.as_ref().status(), Some(Status::Done));
    assert_eq!(strings(args), [c"--", c"a", c"-d"]);
    assert_eq!(first.get().map(Arg::as_c_str), Some(c"a"));
    assert_eq!(second.get().map(Arg::as_c_str), Some(c"-d"));
    assert_eq!(debug.get(), 0);
}

/// A GNU-style parse whose last argument is an option with an optional value parses the bare
/// arguments before it, where the C skips them.
#[test]
fn a_last_optional_value_loses_no_argument() {
    for last in [c"-S", c"--gpg-sign"] {
        let mut args = [Arg::new(c"file"), Arg::new(last)];
        let (sign, file) = (Cell::new(None), Cell::new(None));
        let specs = [
            Spec::new(Target::Value(Some(&sign)))
                .with_name(c"gpg-sign")
                .with_alias(b'S')
                .with_usage(Usage::VALUE_OPTIONAL),
            Spec::new(Target::Arg(Some(&file))).with_usage(Usage::REQUIRED),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let (_, args) = parse(table, &mut args, ParseFlags::FORCE_GNU).unwrap();
        assert_eq!(strings(args), [c"file", last]);
        assert_eq!(file.get().map(Arg::as_c_str), Some(c"file"));
        assert!(sign.get().is_none());
    }
}

/// A whole parse fails at the first argument that does, unknown or missing its value, and once
/// done reports the first requirement not given, a choice's by its first spec; it succeeds when
/// one option of a required choice is given.
#[test]
fn a_parse_fails_at_the_first_failure_or_requirement() {
    let mode = Cell::new(0);
    // The status, failing spec and argument of a parse of `line` against `cmd_cat_file.c`'s
    // table, or `None` once it succeeds.
    let parse_line = |line: &[&CStr]| {
        let mut args: Vec<_> = line.iter().copied().map(Arg::new).collect();
        let (kind, object) = (Cell::new(None), Cell::new(None));
        let specs = [
            Spec::new(Target::Value(None)).with_alias(b'c'),
            Spec::new(Target::Switch(Some(&mode)))
                .with_alias(b't')
                .with_switch_value(1)
                .with_usage(Usage::REQUIRED),
            Spec::new(Target::Switch(Some(&mode)))
                .with_alias(b's')
                .with_switch_value(2)
                .with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(Some(&kind))).with_usage(Usage::CHOICE),
            Spec::new(Target::Arg(Some(&object))).with_usage(Usage::REQUIRED),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let o = parse(table, &mut args, ParseFlags::FORCE_GNU).err()?;
        let o = o.as_ref();
        let arg = o.arg().map(|a| a.to_bytes().to_vec());
        Some((o.status(), o.spec().and_then(|s| table.position(s)), arg))
    };
    let missing = Some(Status::MissingArgument);
    assert_eq!(parse_line(&[]), Some((missing, Some(1), None)));
    assert_eq!(parse_line(&[c"-t"]), Some((missing, Some(4), None)));
    assert_eq!(parse_line(&[c"blob"]), Some((missing, Some(4), None)));
    assert_eq!(
        parse_line(&[c"-x", c"-t"]),
        Some((Some(Status::UnknownOption), None, Some(b"-x".to_vec())))
    );
    assert_eq!(
        parse_line(&[c"-t", c"-c"]),
        Some((Some(Status::MissingValue), Some(0), Some(b"-c".to_vec())))
    );
    for line in [&[c"-s", c"obj"][..], &[c"blob", c"obj"], &[c"obj", c"-t"]] {
        assert_eq!(parse_line(line), None, "{line:?}");
    }
    // The last parse's `-t`.
    assert_eq!(mode.get(), 1);
}

/// A bundle gives a whole parse one spec per option, more than it has arguments, with no bound,
/// where the C keeps them in a stack array of one entry per argument and writes past it.
#[test]
fn a_parse_keeps_every_spec_a_bundle_gives() {
    let flags: Vec<Cell<c_int>> = (0..26).map(|_| Cell::new(0)).collect();
    let mut specs: Vec<Spec<'_>> = (b'a'..=b'z')
        .zip(&flags)
        .map(|(alias, flag)| {
            Spec::new(Target::Switch(Some(flag)))
                .with_alias(alias)
                .with_switch_value(1)
                .with_usage(Usage::REQUIRED)
        })
        .collect();
    specs.push(Spec::default());
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [Arg::new(c"-abcdefghijklmnopqrstuvwxyz")];
    let (done, _) = parse(table, &mut args, ParseFlags::DEFAULT).unwrap();
    assert_eq!(done.as_ref().status(), Some(Status::Done));
    assert!(flags.iter().all(|f| f.get() == 1));
    let mut args = [Arg::new(c"-abcdefghijklmnopqrstuvwxy")];
    let missing = parse(table, &mut args, ParseFlags::DEFAULT).unwrap_err();
    assert_eq!(
        missing.as_ref().spec().and_then(|s| table.position(s)),
        Some(25)
    );
}

/// An option that stops parsing ends a whole parse there, done, its requirements unchecked and
/// the arguments after it unread.
#[test]
fn an_option_that_stops_parsing_ends_the_parse() {
    let (help, debug) = (Cell::new(0), Cell::new(0));
    let specs = [
        Spec::new(Target::Switch(Some(&help)))
            .with_name(c"help")
            .with_switch_value(1)
            .with_usage(Usage::STOP_PARSING),
        Spec::new(Target::Bool(Some(&debug))).with_alias(b'd'),
        Spec::new(Target::Arg(None)).with_usage(Usage::REQUIRED),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut args = [c"--help", c"--nope", c"-d"].map(Arg::new);
    let (done, args) = parse(table, &mut args, ParseFlags::DEFAULT).unwrap();
    let d = done.as_ref();
    assert_eq!(d.status(), Some(Status::Done));
    assert_eq!(d.spec().and_then(|s| table.position(s)), Some(0));
    assert_eq!(d.arg(), Some(c"--help"));
    assert_eq!(strings(args), [c"--help", c"--nope", c"-d"]);
    assert_eq!((help.get(), debug.get()), (1, 0));
}

/// A callback sees every outcome in order, failures included, and the parse stops at the code it
/// breaks with; one that never breaks sees the parse through.
#[test]
fn for_each_calls_back_until_it_breaks() {
    /// A table of one value and the remaining arguments, its lifetime the parse's.
    fn specs<'p>() -> [Spec<'p>; 3] {
        [
            Spec::new(Target::Value(None)).with_alias(b'c'),
            Spec::new(Target::Args(None)),
            Spec::default(),
        ]
    }
    let line = [c"-ca=1", c"--nope", c"-c", c"b=2", c"file"];
    for break_at in [1, 2, 3, 4] {
        let mut args = line.map(Arg::new);
        let specs = specs();
        let table = SpecTable::new(&specs).unwrap();
        let mut seen = Vec::new();
        let mut f = callback(|opt| {
            seen.push((opt.status(), opt.value()));
            if seen.len() == break_at {
                ControlFlow::Break(NonZero::new(-4).unwrap())
            } else {
                ControlFlow::Continue(())
            }
        });
        let flow = for_each(
            table,
            &mut args,
            ParseFlags::FORCE_GNU,
            ForeachCallbackFn::new(&mut f),
        );
        let all = [
            (Some(Status::Ok), Some(c"a=1")),
            (Some(Status::UnknownOption), None),
            (Some(Status::Ok), Some(c"b=2")),
            (Some(Status::Ok), None),
        ];
        assert_eq!(seen, all[..break_at.min(4)], "{break_at}");
        assert_eq!(flow, ControlFlow::Break(NonZero::new(-4).unwrap()));
    }
    let mut args = line.map(Arg::new);
    let specs = specs();
    let table = SpecTable::new(&specs).unwrap();
    let mut calls = 0;
    let mut f = callback(|_| {
        calls += 1;
        ControlFlow::Continue(())
    });
    let flow = for_each(
        table,
        &mut args,
        ParseFlags::FORCE_GNU,
        ForeachCallbackFn::new(&mut f),
    );
    assert_eq!((flow, calls), (ControlFlow::Continue(()), 4));
}

/// Options of specs without a variable parse as those with one do, writing nothing, and a parse
/// of no arguments succeeds, handing back none.
#[test]
fn specs_without_variables_and_no_arguments_parse() {
    let mut args = [
        c"--debug",
        c"--no-debug",
        c"--mode",
        c"--out=x",
        c"--out",
        c"y",
        c"-dm",
    ]
    .map(Arg::new);
    let mut none: [Arg<'_>; 0] = [];
    let specs = [
        Spec::new(Target::Bool(None))
            .with_name(c"debug")
            .with_alias(b'd'),
        Spec::new(Target::Switch(None))
            .with_name(c"mode")
            .with_alias(b'm'),
        Spec::new(Target::Value(None)).with_name(c"out"),
        Spec::new(Target::Literal).with_alias(b'l'),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).unwrap();
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let ok = Some(Status::Ok);
    let found: Vec<_> = steps(&mut parser, table)
        .into_iter()
        .map(|(status, spec, _, value)| (status, spec, value))
        .collect();
    assert_eq!(
        found,
        [
            (ok, Some(0), None),
            (ok, Some(0), None),
            (ok, Some(1), None),
            (ok, Some(2), Some(c"x")),
            (ok, Some(2), Some(c"y")),
            (ok, Some(0), None),
            (ok, Some(1), None),
            (Some(Status::Done), None, None),
        ]
    );
    let (done, args) = parse(table, &mut none, ParseFlags::FORCE_GNU).unwrap();
    assert_eq!(done.as_ref().status(), Some(Status::Done));
    assert!(args.is_empty());
}
