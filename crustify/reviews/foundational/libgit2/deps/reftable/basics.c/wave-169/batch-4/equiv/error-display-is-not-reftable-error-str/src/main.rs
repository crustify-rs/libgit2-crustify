//! `Error`'s `Display` is documented as "the text `reftable_error_str` gives the code". It is
//! for `Format`, `Api` and `OutOfMemory` alone: every other code, which `Error::Other`
//! carries, prints as "reftable error code <n>", where C names it.
//!
//! `reftable_error_str` (`deps/reftable/error.c`) is not allowlisted yet; the variant's static
//! archive, which campaign mode links, defines it, so it is declared here.

use core::ffi::{CStr, c_char, c_int};

use libgit2::libgit2::deps::reftable::basics::Error;

unsafe extern "C" {
    fn reftable_error_str(err: c_int) -> *const c_char;
}

fn main() {
    let mut diverging = Vec::new();
    for code in -14..=-1 {
        // SAFETY: `reftable_error_str` takes any code and returns a static string, or one in
        // its static buffer, read before the next call.
        let c = unsafe { CStr::from_ptr(reftable_error_str(code)) }
            .to_str()
            .unwrap()
            .to_owned();
        let rust = Error::from_code(code).to_string();
        println!("{code:>3}: C {c:?}, Rust {rust:?}");
        if c != rust {
            diverging.push(code);
        }
    }
    assert!(
        diverging.is_empty(),
        "Display diverges from reftable_error_str for {diverging:?}"
    );
}
