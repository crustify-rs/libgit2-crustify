//! `cmd_hash_object.c` as it runs once `opt.c` has switched: it parses `argv + 1` through
//! `cli_opt_parse`, its `Args` spec storing where the files start in its `char **filenames`, then,
//! for `-w`, has `common.c` read the configuration options of the same `argv + 1` through
//! `cli_opt_foreach`, and walks `filenames` to `argv`'s `NULL`. Each export is called through its
//! raw binding, as the C calls it, within `opt.h`'s contract; run under Miri, which reports the
//! walk's reads as undefined behaviour when the pointer the export stored has lost its access.
use core::cell::Cell;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;
use std::ffi::CString;

use libgit2::git2::src::cli::opt::{ArgsSlot, Spec, Target, Usage};
use libgit2::sys::git2 as sys;

/// `common.c`'s `parse_option`, reduced to reading what it is lent.
unsafe extern "C" fn parse_option(opt: *mut sys::cli_opt, _data: *mut c_void) -> c_int {
    // SAFETY: the outcome the parse lends for the call.
    let _ = unsafe { (*opt).value };
    0
}

fn main() {
    let (write, files) = (Cell::new(0), ArgsSlot::new());
    let specs = [
        Spec::new(Target::Switch(Some(&write)))
            .with_alias(b'w')
            .with_switch_value(1),
        Spec::new(Target::Args(Some(&files)))
            .with_value_name(c"file")
            .with_usage(Usage::REQUIRED),
        Spec::default(),
    ];
    let common = [
        Spec::new(Target::Value(None)).with_alias(b'c'),
        Spec::default(),
    ];
    let strings: Vec<CString> = ["hash-object", "a.txt", "-w", "b.txt"]
        .iter()
        .map(|s| CString::new(*s).unwrap())
        .collect();
    let mut argv: Vec<*mut c_char> = strings
        .iter()
        .map(|s| s.as_ptr().cast_mut())
        .chain([ptr::null_mut()])
        .collect();
    let args = argv[1..].as_mut_ptr();
    let len = strings.len() - 1;

    let mut opt = core::mem::MaybeUninit::<sys::cli_opt>::uninit();
    // SAFETY: `opt.h`'s contract: a writable outcome, a table ended by its zeroed spec whose
    // variables outlive the parse, and `argc - 1` C strings at `argv + 1`.
    let status = unsafe {
        sys::cli_opt_parse(opt.as_mut_ptr(), specs.as_ptr().cast(), args, len, sys::CLI_OPT_PARSE_GNU)
    };
    assert_eq!(status, sys::CLI_OPT_STATUS_DONE);
    assert_eq!(write.get(), 1);

    // SAFETY: as above, with a callback that reads the outcome it is lent.
    let rc = unsafe {
        sys::cli_opt_foreach(
            common.as_ptr().cast(),
            args,
            len,
            sys::CLI_OPT_PARSE_GNU,
            Some(parse_option),
            ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0);

    // `for (filename = filenames; *filename; filename++)`: an `ArgsSlot` is
    // `#[repr(transparent)]` over the `char **` C reads.
    // SAFETY: the variable is the `char **` the parse wrote.
    let mut at = unsafe { ptr::from_ref(&files).cast::<*const *mut c_char>().read() };
    let mut walked = Vec::new();
    loop {
        // SAFETY: `at` is within `argv`, which its `NULL` ends.
        let file = unsafe { at.read() };
        if file.is_null() {
            break;
        }
        // SAFETY: one of `argv`'s C strings.
        walked.push(unsafe { CStr::from_ptr(file) }.to_str().unwrap().to_owned());
        at = at.wrapping_add(1);
    }
    println!("filenames: {walked:?}");
    assert_eq!(walked, ["a.txt", "b.txt"]);
}
