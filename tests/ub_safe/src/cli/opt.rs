//! `src/cli/opt.c`: the spec, outcome and parser layouts through the safe API alone, under
//! the sanitizers. [`Parser::new`] is C's `cli_opt_parser_init`, which stores the table and
//! the arguments it is given; these tests set parsers up over the shapes C's spec tables take
//! and read back, through the getters, the pointers C stored, with borrowed handles aliased,
//! reborrowed and copied. Miri cannot call C, so the tests that set a parser up stop there.

use core::cell::Cell;
use core::num::NonZero;

use ffibox::CZeroable;
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, Kind, Opt, ParseFlags, Parser, Spec, SpecTable, Target, Usage,
};

/// A parser C set up over an empty argument array: its stored pointer is never read, and the
/// getters lend an empty slice.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_parser_over_no_arguments() {
    let specs = [Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args: [Arg<'_>; 0] = [];
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    assert!(parser.as_ref().args().is_empty());
    assert!(parser.as_mut().args_mut().is_empty());
    assert_eq!(parser.as_ref().specs().map(|t| t.len()), Some(0));
}

/// C stores the table and the arguments as given, clears every position, and records the
/// flags; the getters read each, as many times and through as many copies of the shared
/// handle as asked.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn a_parser_reads_back_what_c_stored() {
    let debug = Cell::new(0);
    let output = Cell::new(None);
    let rest = ArgsSlot::new();
    let specs = [
        Spec::new(Target::Bool(Some(&debug))).with_name(c"debug"),
        Spec::new(Target::Value(Some(&output))).with_alias(b'o'),
        Spec::new(Target::Args(Some(&rest))).with_usage(Usage::REQUIRED),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"-o"), Arg::new(c"out"), Arg::new(c"rest")];
    let parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let (a, b) = (parser.as_ref(), parser.as_ref());
    let stored = a.specs().expect("a table");
    assert_eq!(stored.len(), 3);
    for (i, spec) in b.specs().expect("a table").iter().enumerate() {
        assert_eq!(stored.position(spec), Some(i));
    }
    assert_eq!(stored.get(0).and_then(|s| s.name()), Some(c"debug"));
    assert_eq!(
        stored.get(1).and_then(|s| s.alias()).map(NonZero::get),
        Some(b'o')
    );
    assert_eq!(stored.get(2).map(|s| s.usage()), Some(Usage::REQUIRED));
    let read: Vec<_> = a.args().iter().map(|x| x.as_c_str()).collect();
    assert_eq!(read, [c"-o", c"out", c"rest"]);
    assert_eq!(b.args_len(), 3);
    assert_eq!(
        (a.idx(), a.arg_idx(), a.in_args(), a.in_short()),
        (0, 0, 0, 0)
    );
    assert!(!a.needs_sort());
    assert!(!b.in_literal());
    assert_eq!(a.flags(), ParseFlags::DEFAULT);
}

/// `FORCE_GNU` makes C set the parser up to reorder its arguments whatever the environment,
/// in the bit beside `in_literal`, which it leaves clear.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn force_gnu_needs_a_sort() {
    let specs = [Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"file")];
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    assert!(parser.as_ref().needs_sort());
    assert!(!parser.as_ref().in_literal());
    assert_eq!(parser.as_ref().flags(), ParseFlags::FORCE_GNU);
    parser.as_mut().set_in_literal(true);
    assert!(parser.as_ref().needs_sort());
    parser.as_mut().set_needs_sort(false);
    assert!(parser.as_ref().in_literal());
}

/// The arguments C stored reorder through the exclusive handle, and the strings taken from
/// them before stay valid: a variable, an outcome and a slot keep what they took.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reordering_keeps_the_strings_taken() {
    let value = Cell::new(None);
    let rest = ArgsSlot::new();
    let specs = [
        Spec::new(Target::Value(Some(&value))),
        Spec::new(Target::Args(Some(&rest))),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"a"), Arg::new(c"--b"), Arg::new(c"c")];
    let mut opt = Opt::zeroed();
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    let first = parser.as_ref().args()[0];
    value.set(Some(first));
    opt.as_mut().set_arg(Some(first.as_c_str()));
    opt.as_mut().set_spec(table.get(0));
    let mut m = parser.as_mut();
    let moved = m.args_mut();
    moved.rotate_right(1);
    rest.set(&moved[1..]);
    let order: Vec<_> = parser
        .as_ref()
        .args()
        .iter()
        .map(|x| x.as_c_str())
        .collect();
    assert_eq!(order, [c"c", c"a", c"--b"]);
    assert_eq!(value.get().map(Arg::as_c_str), Some(c"a"));
    assert_eq!(opt.as_ref().arg(), Some(c"a"));
    assert_eq!(
        opt.as_ref().spec().and_then(|s| s.kind()),
        Some(Kind::Value)
    );
    let tail = rest
        .get_in(parser.as_ref().args())
        .expect("in the parser's arguments");
    let tail: Vec<_> = tail.iter().map(|x| x.as_c_str()).collect();
    assert_eq!(tail, [c"a", c"--b"]);
}

/// A spec read through a parser's table writes its variable through the `Cell` C was given,
/// and the caller reads the write back through its own borrow.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn variables_are_written_through_the_table() {
    let count = Cell::new(1);
    let specs = [
        Spec::new(Target::Accumulator(Some(&count))).with_switch_value(2),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"-v")];
    let parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let spec = parser
        .as_ref()
        .specs()
        .and_then(|t| t.get(0))
        .expect("one spec");
    let Some(Target::Accumulator(Some(v))) = spec.target() else {
        panic!("{:?}", spec.target());
    };
    v.set(v.get() + spec.switch_value());
    assert_eq!(count.get(), 3);
}

/// A parser built zeroed and filled through its setters reads as C would have set it up.
#[test]
fn a_parser_filled_in_rust_reads_back() {
    let specs = [Spec::new(Target::Literal), Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args = [Arg::new(c"--"), Arg::new(c"-rf")];
    let mut parser = Parser::zeroed();
    parser.as_mut().set_specs(Some(table));
    parser.as_mut().set_args(&mut args);
    let r = parser.as_ref();
    assert_eq!(r.specs().map(|t| t.len()), Some(1));
    assert_eq!(r.args().len(), 2);
}
