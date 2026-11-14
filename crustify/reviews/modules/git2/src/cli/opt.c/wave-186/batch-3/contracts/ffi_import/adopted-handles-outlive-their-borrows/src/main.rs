//! A spec adopted through `SpecRef::from_ptr`, within the contract that method states
//! (ffibox's `CCell::ref_from_ptr`: a live, initialised `cli_opt_spec` that outlives the
//! handle's `'a`, which nothing writes meanwhile), lends its name for the `'p` the caller
//! picks, here `'static`. The getter's `SAFETY:` comment rests on "its handle's adoption"
//! vouching that the strings live for `'p`, which no stated contract asks. The name then
//! outlives its string: a use after free, which Miri and ASan report.

use std::ffi::{CStr, CString};

use ffibox::CRefSeam;
use libgit2::git2::src::cli::opt::{Spec, SpecRef, Target};

fn main() {
    let name: &'static CStr;
    {
        let owned = CString::new("debug").unwrap();
        let mut spec = Spec::new(Target::Literal);
        spec.as_mut().set_name(Some(&owned));
        let ptr = spec.as_ref().as_ptr().cast_mut();
        // SAFETY: `ptr` addresses `spec`, a live, initialised `cli_opt_spec`, which outlives
        // the handle: the handle is last used on the next line, before `spec` is dropped,
        // and nothing writes `spec` meanwhile. That is all `CRefSeam::from_ptr` asks.
        let adopted: SpecRef<'_, 'static> = unsafe { SpecRef::from_ptr(ptr) }.unwrap();
        name = adopted.name().unwrap();
    }
    // `spec` and `owned` are gone; `name` is a `&'static CStr` into the freed string.
    println!("{:?}", name.to_bytes());
}
