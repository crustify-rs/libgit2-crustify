// A string's borrowed run cannot outlive the string it views.
use libgit2::libgit2::raw::OdbDataStrFree;

fn main() {
    let s = OdbDataStrFree::dup(c"odb").unwrap();
    let run = s.as_handles();
    drop(s);
    let _ = run.len();
}
