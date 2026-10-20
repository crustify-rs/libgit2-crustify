// A string's borrowed run cannot outlive the string whose bytes it reads.
use libgit2::libgit2::raw::OdbDataStrFree;

fn main() {
    let s = OdbDataStrFree::dup(c"odb").unwrap();
    let run = s.as_handles();
    drop(s);
    let _ = run.elem(0);
}
