//! `src/cli/opt.c`: the spec, outcome and parser layouts through the safe API alone, under
//! the sanitizers and Miri. [`Parser::new`], the port of `cli_opt_parser_init`, stores the table
//! and the arguments it is given in the parser's C layout; these tests set parsers up over the
//! shapes C's spec tables take and read back, through the getters, the pointers it stored, with
//! borrowed handles aliased, reborrowed and copied. The status message reads a choice's later
//! options from the table it is given, never past it, and a callback's closure keeps the strings
//! the outcomes lend past the outcomes. The parser, [`ParserMut::parse_next`], [`parse`] and
//! [`for_each`], reads heap copies of its arguments within their bounds, in any state its setters
//! can give it, and what a parse takes from them outlives it.
//!
//! [`ParserMut::parse_next`]: libgit2_crustify::git2::src::cli::opt::ParserMut::parse_next

use core::cell::Cell;
use core::ffi::CStr;
use core::num::NonZero;
use core::ops::ControlFlow;
use std::ffi::CString;

use ffibox::CZeroable;
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, ForeachCallbackFn, Kind, Opt, OptRef, ParseFlags, Parser, Spec, SpecTable,
    Status, Target, Usage, for_each, parse,
};

/// A parser set up over an empty argument array: its stored pointer is never read, and the
/// getters lend an empty slice.
#[test]
fn a_parser_over_no_arguments() {
    let specs = [Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let mut args: [Arg<'_>; 0] = [];
    let mut parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    assert!(parser.as_ref().args().is_empty());
    assert!(parser.as_mut().args_mut().is_empty());
    assert_eq!(parser.as_ref().specs().map(|t| t.len()), Some(0));
}

/// A parser stores the table and the arguments as given, clears every position, and records
/// the flags; the getters read each, as many times and through as many copies of the shared
/// handle as asked.
#[test]
fn a_parser_reads_back_what_it_stored() {
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

/// `FORCE_GNU` sets the parser up to reorder its arguments whatever the environment,
/// in the bit beside `in_literal`, which it leaves clear.
#[test]
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

/// The arguments a parser stored reorder through the exclusive handle, and the strings taken from
/// them before stay valid: a variable, an outcome and a slot keep what they took.
#[test]
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

/// A spec read through a parser's table writes its variable through the `Cell` it was given,
/// and the caller reads the write back through its own borrow.
#[test]
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

/// A required choice that the table's last spec continues, or that a spec of another table
/// starts, is read no further than the table given: its last option before the end, or the spec
/// alone.
#[test]
fn a_choice_is_read_within_its_table() {
    let specs = [
        Spec::new(Target::Switch(None)).with_alias(b'a'),
        Spec::new(Target::Switch(None))
            .with_alias(b'b')
            .with_usage(Usage::CHOICE),
        Spec::zeroed(),
        Spec::new(Target::Switch(None))
            .with_alias(b'z')
            .with_usage(Usage::CHOICE),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let one = [
        Spec::new(Target::Switch(None)).with_alias(b'q'),
        Spec::zeroed(),
    ];
    let alone = SpecTable::new(&one).expect("ended");
    let mut opt = Opt::zeroed();
    opt.as_mut().set_status(Status::MissingArgument);
    for (at, message) in [
        (0, &b"either '-a' or '-b' is required.\n"[..]),
        (1, b"argument '-b' is required.\n"),
    ] {
        opt.as_mut().set_spec(table.get(at));
        let mut out = Vec::new();
        opt.as_ref().write_status(&mut out, None, table).unwrap();
        assert_eq!(out, message);
        let mut out = Vec::new();
        let mut other = Opt::zeroed();
        other.as_mut().set_status(Status::MissingArgument);
        other.as_mut().set_spec(alone.get(0));
        other.as_ref().write_status(&mut out, None, alone).unwrap();
        assert_eq!(out, b"argument '-q' is required.\n");
    }
}

/// What a callback's closure keeps of the outcomes of `lent`, each built and dropped in turn:
/// the arguments they lend for `'p`.
fn keep_through_a_callback<'p>(lent: &[&'p CStr]) -> Vec<&'p [u8]> {
    let mut kept = Vec::new();
    let mut keep = |opt: OptRef<'_, 'p>| {
        kept.extend(opt.arg().map(CStr::to_bytes));
        ControlFlow::Continue(())
    };
    {
        let mut callback = ForeachCallbackFn::new(&mut keep);
        for &arg in lent {
            let mut opt = Opt::zeroed();
            opt.as_mut().set_arg(Some(arg));
            let _ = callback.call(opt.as_ref());
        }
    }
    kept
}

/// A callback's closure keeps the strings each outcome lends, which live as long as the
/// arguments do, after the outcome that lent them is gone.
#[test]
fn a_callback_keeps_what_outcomes_lend() {
    let mut args = [Arg::new(c"--value"), Arg::new(c"file")];
    let specs = [Spec::new(Target::Arg(None)), Spec::zeroed()];
    let table = SpecTable::new(&specs).expect("ended");
    let parser = Parser::new(table, &mut args, ParseFlags::DEFAULT);
    let lent: Vec<&CStr> = parser
        .as_ref()
        .args()
        .iter()
        .map(|a| a.as_c_str())
        .collect();
    assert_eq!(keep_through_a_callback(&lent), [&b"--value"[..], b"file"]);
}

/// Heap copies of `line`, each its own allocation, as the sanitizers and Miri bound them.
fn heap(line: &[&str]) -> Vec<CString> {
    line.iter()
        .map(|a| CString::new(*a).expect("no NUL"))
        .collect()
}

/// A GNU-style parse over heap strings reorders its array in place, takes values from within
/// arguments and from the ones after them, reads each bundle to its end and no further, and
/// what the variables, the outcomes and the slot keep stays readable once the parse is over and
/// the array handed back.
#[test]
fn a_parse_keeps_what_it_takes_from_heap_arguments() {
    let owned = heap(&[
        "a", "-dvofile", "b", "--out", "x", "-v", "--out=y", "c", "d",
    ]);
    let mut args: Vec<_> = owned.iter().map(|a| Arg::new(a)).collect();
    let (debug, verbose, out, first) =
        (Cell::new(0), Cell::new(0), Cell::new(None), Cell::new(None));
    let rest = ArgsSlot::new();
    let specs = [
        Spec::new(Target::Bool(Some(&debug))).with_alias(b'd'),
        Spec::new(Target::Accumulator(Some(&verbose))).with_alias(b'v'),
        Spec::new(Target::Value(Some(&out)))
            .with_name(c"out")
            .with_alias(b'o'),
        Spec::new(Target::Arg(Some(&first))),
        Spec::new(Target::Args(Some(&rest))),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let (done, args) = parse(table, &mut args, ParseFlags::FORCE_GNU).expect("parsed");
    let order: Vec<_> = args.iter().map(|a| a.as_c_str().to_bytes()).collect();
    assert_eq!(
        order,
        [
            &b"-dvofile"[..],
            b"--out",
            b"x",
            b"-v",
            b"--out=y",
            b"a",
            b"b",
            b"c",
            b"d"
        ]
    );
    assert_eq!(done.as_ref().args_len(), 3);
    assert_eq!((debug.get(), verbose.get()), (1, 2));
    assert_eq!(out.get().map(Arg::as_c_str), Some(c"y"));
    assert_eq!(first.get().map(Arg::as_c_str), Some(c"a"));
    let tail: Vec<_> = rest
        .get_in(args)
        .expect("in the arguments")
        .iter()
        .map(|a| a.as_c_str())
        .collect();
    assert_eq!(tail, [c"b", c"c", c"d"]);
}

/// A parser in a state its setters can give it but no parse does, inside a bundle past its
/// argument's end, past its arguments, or with no table, reads no argument past its end.
#[test]
fn a_parser_in_any_state_reads_within_its_arguments() {
    let owned = heap(&["-d", "-", "x"]);
    let mut args: Vec<_> = owned.iter().map(|a| Arg::new(a)).collect();
    let specs = [
        Spec::new(Target::Value(None)).with_name(c"config-env"),
        Spec::new(Target::Bool(None)).with_alias(b'd'),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut parser = Parser::new(table, &mut args, ParseFlags::FORCE_GNU);
    for (idx, in_short) in [
        (0, 1),
        (0, 2),
        (0, usize::MAX),
        (1, 0),
        (2, 3),
        (3, 0),
        (usize::MAX, 0),
    ] {
        let mut p = parser.as_mut();
        p.set_idx(idx);
        p.set_in_short(in_short);
        let _ = p.parse_next();
        let _ = parser.as_mut().parse_next();
    }
    let mut none = Parser::default();
    let owned = heap(&["--x", "-", "y"]);
    let mut args: Vec<_> = owned.iter().map(|a| Arg::new(a)).collect();
    none.as_mut().set_args(&mut args);
    for _ in 0..4 {
        let _ = none.as_mut().parse_next();
    }
    assert_eq!(none.as_ref().idx(), 3);
}

/// `f`, as the callback of a parse over a table borrowed for `'p`: a closure of every outcome's
/// borrow, which may keep what the outcomes lend for `'p`.
fn callback<'p, F>(f: F) -> F
where
    F: FnMut(OptRef<'_, 'p>) -> ControlFlow<NonZero<core::ffi::c_int>>,
{
    f
}

/// A whole parse's callback keeps the values and arguments the outcomes lend, which outlive the
/// parse and the callback, and its break ends the parse at once.
#[test]
fn a_callback_keeps_what_a_whole_parse_lends() {
    let owned = heap(&["-ca", "-c", "b", "file", "-cc"]);
    let mut args: Vec<_> = owned.iter().map(|a| Arg::new(a)).collect();
    let specs = [
        Spec::new(Target::Value(None)).with_alias(b'c'),
        Spec::zeroed(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let mut kept: Vec<&CStr> = Vec::new();
    {
        let mut keep = callback(|opt| {
            kept.extend(opt.value());
            kept.extend(opt.arg());
            match opt.arg().map(CStr::to_bytes) {
                Some(b"file") => ControlFlow::Break(NonZero::new(1).expect("non-zero")),
                _ => ControlFlow::Continue(()),
            }
        });
        let flow = for_each(
            table,
            &mut args,
            ParseFlags::FORCE_GNU,
            ForeachCallbackFn::new(&mut keep),
        );
        assert_eq!(flow, ControlFlow::Break(NonZero::new(1).expect("non-zero")));
    }
    let kept: Vec<_> = kept.iter().map(|s| s.to_bytes()).collect();
    assert_eq!(
        kept,
        [&b"a"[..], b"-ca", b"b", b"-c", b"c", b"-cc", b"file"]
    );
}
