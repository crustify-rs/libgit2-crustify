//! UB tests for the adoption of `opt.h`'s layouts: `SpecRef`, `OptRef` and `ParserRef`, and
//! their exclusive handles, built by `CRefSeam::from_ptr` and `CMutSeam::from_ptr` from a
//! pointer, within the contract those seams state and the one each type's `# Adoption`
//! section adds, that the pointee is a valid layout for the `'p` the caller names. Each
//! adopted handle is then read through every getter, and the exclusive ones written, while
//! what it points at lives (`ASan`: use after free; Miri: dangling references).
//!
//! `SpecTable::from_ptr` adopts a whole table, as the exports adopt the CLI's C tables: from
//! its first spec, from one within it, and at its end, each read to that end and no further
//! (`ASan`: overflow; Miri: out-of-bounds reads).
//!
//! None calls C: the layouts are built in Rust storage, as C builds them in its own, and
//! adopted from their pointers, so Miri runs them all.

use core::cell::Cell;
use core::num::NonZero;
use std::ffi::CString;
use std::ptr;

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2_crustify::git2::src::cli::opt::{
    Arg, ArgsSlot, Kind, Opt, OptRef, ParseFlags, Parser, ParserMut, ParserRef, Spec, SpecMut,
    SpecRef, SpecTable, Status, Target, Usage,
};
use libgit2_crustify::sys::git2 as sys;

/// A null pointer adopts as no handle, whichever seam adopts it.
#[test]
fn null_adopts_as_none() {
    // SAFETY: `from_ptr` refuses a null pointer, which it is given here.
    let spec = unsafe { SpecRef::from_ptr(ptr::null_mut()) };
    assert!(spec.is_none());
    // SAFETY: as above.
    let opt = unsafe { OptRef::from_ptr(ptr::null_mut()) };
    assert!(opt.is_none());
    // SAFETY: as above.
    let parser = unsafe { ParserMut::from_ptr(ptr::null_mut()) };
    assert!(parser.is_none());
}

/// A spec adopted at the lifetime of its strings and its variable lends them for that
/// lifetime: every getter reads what was stored, and the variable is written through the
/// target the adopted handle lends. This is the regression test of
/// `contracts/ffi_import/adopted-handles-outlive-their-borrows`, whose reproducer adopted
/// the spec at `'static`, which the adoption's contract now excludes.
#[test]
fn an_adopted_spec_lends_its_strings_for_their_lifetime() {
    let name = CString::new("debug").unwrap();
    let help = CString::new("show debugging output").unwrap();
    let flag = Cell::new(0);
    let spec = Spec::new(Target::Bool(Some(&flag)))
        .with_name(&name)
        .with_alias(b'd')
        .with_usage(Usage::HIDDEN)
        .with_help(&help);
    let ptr = spec.as_ref().as_ptr().cast_mut();
    // SAFETY: `ptr` addresses `spec`, live and unwritten while the handle lives; its strings
    // and its variable live, unchanged but for the variable's `Cell`, for the `'p` the handle
    // is adopted at, which the borrows of `name`, `help` and `flag` bound.
    let adopted: SpecRef<'_, '_> = unsafe { SpecRef::from_ptr(ptr) }.unwrap();
    assert_eq!(adopted.kind(), Some(Kind::Bool));
    assert_eq!(adopted.name(), Some(name.as_c_str()));
    assert_eq!(adopted.alias().map(NonZero::get), Some(b'd'));
    assert_eq!(adopted.usage(), Usage::HIDDEN);
    assert_eq!(adopted.value_name(), None);
    assert_eq!(adopted.help(), Some(help.as_c_str()));
    let Some(Target::Bool(Some(variable))) = adopted.target() else {
        panic!("{:?}", adopted.target());
    };
    variable.set(1);
    assert_eq!(flag.get(), 1);
    assert_eq!(format!("{adopted:?}"), format!("{spec:?}"));
}

/// A spec adopted exclusively is written through the adopted handle, and the writes read
/// back through the value.
#[test]
fn an_adopted_spec_is_written_exclusively() {
    let value = Cell::new(None);
    let mut spec = Spec::zeroed();
    let ptr = spec.as_mut().as_mut_ptr();
    {
        // SAFETY: `ptr` addresses `spec`, which only this handle reaches while it lives; its
        // `NULL` strings and variable are valid for any `'p`.
        let mut adopted = unsafe { SpecMut::from_ptr(ptr) }.unwrap();
        adopted.set_target(Target::Value(Some(&value)));
        adopted.set_name(Some(c"output"));
        adopted.set_value_name(Some(c"file"));
        assert_eq!(adopted.as_ref().kind(), Some(Kind::Value));
    }
    let r = spec.as_ref();
    assert_eq!(r.name(), Some(c"output"));
    assert_eq!(r.value_name(), Some(c"file"));
    let Some(Target::Value(Some(v))) = r.target() else {
        panic!("{:?}", r.target());
    };
    v.set(Some(Arg::new(c"out.txt")));
    assert_eq!(value.get().map(Arg::as_c_str), Some(c"out.txt"));
}

/// An outcome adopted at its spec's and strings' lifetime lends its spec, which is then
/// adopted in turn through the table, and its strings.
#[test]
fn an_adopted_opt_lends_its_spec_and_strings() {
    let specs = [
        Spec::new(Target::Value(None)).with_name(c"name"),
        Spec::default(),
    ];
    let table = SpecTable::new(&specs).expect("ended");
    let arg = CString::new("--name=value").unwrap();
    let mut opt = Opt::zeroed();
    let mut m = opt.as_mut();
    m.set_status(Status::Ok);
    m.set_spec(table.get(0));
    m.set_arg(Some(&arg));
    m.set_value(Some(&arg.as_c_str()[7..]));
    let ptr = opt.as_ref().as_ptr().cast_mut();
    // SAFETY: `ptr` addresses `opt`, live and unwritten while the handle lives; its spec is
    // one of `specs`, a valid spec borrowed for the `'p` the handle is adopted at, and its
    // strings are `arg`'s, borrowed for it too.
    let adopted: OptRef<'_, '_> = unsafe { OptRef::from_ptr(ptr) }.unwrap();
    assert_eq!(adopted.status(), Some(Status::Ok));
    assert_eq!(adopted.spec().map(|s| table.position(s)), Some(Some(0)));
    assert_eq!(adopted.spec().and_then(|s| s.name()), Some(c"name"));
    assert_eq!(adopted.arg(), Some(arg.as_c_str()));
    assert_eq!(adopted.value(), Some(c"value"));
    assert_eq!(adopted.args_len(), 0);
}

/// A parser adopted exclusively over its table and arguments lends them, reorders its
/// arguments in place, and a slot finds its tail among them; the adopted shared handle reads
/// the same state back.
#[test]
fn an_adopted_parser_lends_its_table_and_arguments() {
    let rest = ArgsSlot::new();
    let specs = [Spec::new(Target::Args(Some(&rest))), Spec::default()];
    let table = SpecTable::new(&specs).expect("ended");
    let strings = [CString::new("a").unwrap(), CString::new("--b").unwrap()];
    let mut args = [Arg::new(&strings[0]), Arg::new(&strings[1])];
    let mut parser = Parser::zeroed();
    parser.as_mut().set_specs(Some(table));
    parser.as_mut().set_args(&mut args);
    parser.as_mut().set_flags(ParseFlags::GNU);
    let ptr = parser.as_mut().as_mut_ptr();
    {
        // SAFETY: `ptr` addresses `parser`, which only this handle reaches while it lives; its
        // table is `specs`, valid and ended, and its arguments `args`, which it borrows
        // exclusively, each one of `strings`, for the `'p` the handle is adopted at.
        let mut adopted = unsafe { ParserMut::from_ptr(ptr) }.unwrap();
        assert_eq!(adopted.as_ref().specs().map(|t| t.len()), Some(1));
        let moved = adopted.args_mut();
        moved.swap(0, 1);
        rest.set(&moved[1..]);
        adopted.set_idx(2);
    }
    let ptr = parser.as_ref().as_ptr().cast_mut();
    // SAFETY: as above, shared now: nothing writes `parser` while this handle lives.
    let adopted: ParserRef<'_, '_> = unsafe { ParserRef::from_ptr(ptr) }.unwrap();
    let order: Vec<_> = adopted.args().iter().map(|a| a.as_c_str()).collect();
    assert_eq!(order, [c"--b", c"a"]);
    assert_eq!(adopted.idx(), 2);
    assert_eq!(adopted.flags(), ParseFlags::GNU);
    let tail: Vec<_> = rest
        .get_in(adopted.args())
        .expect("in the parser's arguments")
        .iter()
        .map(|a| a.as_c_str())
        .collect();
    assert_eq!(tail, [c"a"]);
}

/// A table of C layouts with strings and a variable, adopted from its first spec, reads as the
/// table it is, ended by its zeroed spec, and its variable is written through it.
#[test]
fn a_table_adopted_from_its_start_reads_to_its_end() {
    let name = CString::new("output").unwrap();
    let count = Cell::new(0);
    let specs = [
        Spec::new(Target::Accumulator(Some(&count))).with_alias(b'v'),
        Spec::new(Target::Value(None)).with_name(&name),
        Spec::zeroed(),
        Spec::new(Target::Literal),
    ];
    // The array's pointer, whose provenance covers every spec: a pointer from one spec's
    // `as_ref()` covers that spec alone, and reading the next through it is undefined.
    let start = specs.as_ptr().cast::<sys::cli_opt_spec>();
    // SAFETY: `start` addresses `specs`, ended by its zeroed spec, live and unchanged but for the
    // variable's `Cell` while the table lives, for the `'p` the borrows of `name` and `count`
    // bound.
    let table = unsafe { SpecTable::from_ptr(start) }.expect("non-null");
    assert_eq!(table.len(), 2);
    assert_eq!(table.get(1).and_then(|s| s.name()), Some(name.as_c_str()));
    assert!(table.get(2).is_none());
    let Some(Target::Accumulator(Some(v))) = table.get(0).and_then(|s| s.target()) else {
        panic!("an accumulator");
    };
    v.set(v.get() + 3);
    assert_eq!(count.get(), 3);
    let positions: Vec<_> = table.iter().map(|s| table.position(s)).collect();
    assert_eq!(positions, [Some(0), Some(1)]);
}

/// A table adopted from a spec within an array starts there, and one adopted at the end is
/// empty; a null pointer is no table.
#[test]
fn a_table_adopted_within_an_array_starts_there() {
    let specs = [
        Spec::new(Target::Literal),
        Spec::new(Target::Arg(None)),
        Spec::new(Target::Args(None)),
        Spec::zeroed(),
    ];
    let base = specs.as_ptr().cast::<sys::cli_opt_spec>();
    // SAFETY: each pointer addresses an element of `specs`, which a zeroed spec ends, with the
    // array's provenance, live and unchanged while the tables live.
    let tail = unsafe { SpecTable::from_ptr(base.wrapping_add(1)) }.expect("non-null");
    let kinds: Vec<_> = tail.iter().map(|s| s.kind()).collect();
    assert_eq!(kinds, [Some(Kind::Arg), Some(Kind::Args)]);
    // SAFETY: as above.
    let end = unsafe { SpecTable::from_ptr(base.wrapping_add(3)) }.expect("non-null");
    assert!(end.is_empty());
    // SAFETY: `from_ptr` refuses a null pointer.
    assert!(unsafe { SpecTable::from_ptr(ptr::null()) }.is_none());
}
