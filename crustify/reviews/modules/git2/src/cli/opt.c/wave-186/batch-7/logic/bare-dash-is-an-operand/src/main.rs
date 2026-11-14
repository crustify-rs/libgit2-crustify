//! The bare `-` is an operand, as POSIX `getopt` and GNU `getopt_long` take it, which `opt.h`
//! names as the style of its parser: parsed against a table that takes bare arguments, it is the
//! argument a `Kind::Arg` spec stores, in either style, alone, after an option and before one
//! that GNU style moves ahead of it. Exits 1 at the first scenario the parser departs in.
#![forbid(unsafe_code)]

use core::cell::Cell;
use std::process::ExitCode;

use libgit2::git2::src::cli::opt::{Arg, ParseFlags, Spec, SpecTable, Target, parse};

fn main() -> ExitCode {
    let scenarios: [(&[&core::ffi::CStr], ParseFlags); 4] = [
        (&[c"-"], ParseFlags::DEFAULT),
        (&[c"-"], ParseFlags::FORCE_GNU),
        (&[c"-q", c"-"], ParseFlags::DEFAULT),
        (&[c"-", c"-q"], ParseFlags::FORCE_GNU),
    ];
    let mut departed = false;
    for (given, flags) in scenarios {
        let (quiet, file) = (Cell::new(0), Cell::new(None));
        let specs = [
            Spec::new(Target::Bool(Some(&quiet))).with_alias(b'q'),
            Spec::new(Target::Arg(Some(&file))),
            Spec::default(),
        ];
        let table = SpecTable::new(&specs).unwrap();
        let mut args: Vec<Arg<'_>> = given.iter().copied().map(Arg::new).collect();
        let outcome = parse(table, &mut args, flags);
        let taken = file.get().map(Arg::as_c_str);
        let quiet_given = given.contains(&c"-q");
        let ok = outcome.is_ok() && taken == Some(c"-") && (quiet.get() == 1) == quiet_given;
        let failed = outcome.err().map(|o| (o.as_ref().status(), o.as_ref().arg()));
        println!("{given:?} {flags:?}: argument {taken:?}, failure {failed:?}");
        if !ok {
            departed = true;
        }
    }
    if departed {
        println!("the bare `-` is not taken as an operand");
        ExitCode::FAILURE
    } else {
        println!("the bare `-` is an operand");
        ExitCode::SUCCESS
    }
}
