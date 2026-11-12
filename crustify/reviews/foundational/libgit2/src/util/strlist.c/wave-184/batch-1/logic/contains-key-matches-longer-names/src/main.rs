//! `contains_key` finds a variable's name at the start of a longer one: it calls
//! `git_strlist_contains_key`, which compares the bytes of `key` before its delimiter and
//! not the delimiter itself, so `FOO=1` is "contained" in `["FOOBAR=2"]`.
//!
//! libgit2's process runner (`src/util/unix/process.c`, `merge_env`) asks exactly this of
//! the caller's environment for each inherited `NAME=value` entry, to let the caller's entry
//! replace the inherited one (`src/util/process.h`: `git_process_new` "will add the given
//! environment variables (in `env`) to the current environment"). An environment variable is
//! named by the bytes before its first `=` (POSIX.1-2017, XBD 8.1), so an entry sets `FOO`
//! only when it begins with `FOO=`.
#![forbid(unsafe_code)]

use libgit2::libgit2::src::util::strlist::contains_key;

fn main() {
    let caller = [c"FOOBAR=2"];
    let found = contains_key(&caller, c"FOO=1", b'=');
    println!("contains_key([\"FOOBAR=2\"], \"FOO=1\", '=') = {found}");
    let empty_name = contains_key(&[c"HOME=/root"], c"=x", b'=');
    println!("contains_key([\"HOME=/root\"], \"=x\", '=') = {empty_name}");
    assert!(!found, "FOOBAR=2 sets FOOBAR, not FOO");
    assert!(!empty_name, "HOME=/root sets no variable of an empty name");
}
