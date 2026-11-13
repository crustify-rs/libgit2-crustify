//! `src/cli/opt.c`: the enums and structs of `opt.h`, `Kind::is_option`, the port of
//! `spec_is_option_type`, and `uses_gnu_style`, the port of `support_gnu_style`, checked
//! against their contract through Rust assertions alone. The CLI has no C tests to carry
//! over; these cover the flag sets' operations over the combinations the CLI's spec tables
//! use, the closed enums' C values, which the CLI's remaining C relies on, the parsing
//! style each combination of flags selects, and the accessors of the spec, outcome and parser
//! layouts, built in Rust storage.
//!
//! None reaches C. `uses_gnu_style` reads `POSIXLY_CORRECT`, which a test does not set in its
//! own process: setting the environment is unsafe while other threads may read it, as the
//! harness's do. The test that depends on it runs its body in a child process of its own
//! binary, whose environment the parent sets, and Miri, which spawns no process, ignores it;
//! it runs all the others.

use core::cell::Cell;
use core::num::NonZero;
use std::process::Command;

use ffibox::CZeroable;
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, Kind, Opt, ParseFlags, Parser, Spec, SpecTable, Status, Target, Usage,
    uses_gnu_style,
};

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
