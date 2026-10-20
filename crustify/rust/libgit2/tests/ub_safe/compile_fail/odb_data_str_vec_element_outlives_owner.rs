// An element borrowed from an `OdbDataStrVec` cannot outlive the run.
use libgit2::libgit2::raw::{OdbDataStrVec, data_str_bytes};

fn main() {
    let v = OdbDataStrVec::dup_all(&[c"a", c"b"]).unwrap();
    let first = &v.as_slice()[0];
    drop(v);
    let _ = data_str_bytes(first);
}
