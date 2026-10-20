// An `OdbDataStrVec`'s shared and mutable views cannot coexist.
use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataStrVec, data_str_bytes};

fn main() {
    let mut v = OdbDataStrVec::dup_all(&[c"a"]).unwrap();
    let shared = v.as_slice();
    v.as_mut_slice()[0] = OdbDataStrFree::dup(c"b").unwrap();
    let _ = data_str_bytes(&shared[0]);
}
