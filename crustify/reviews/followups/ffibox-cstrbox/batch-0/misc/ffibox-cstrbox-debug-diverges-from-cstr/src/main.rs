//! ffibox's `CStrBox` `Debug` against `CStr`'s, which it claims to match, through
//! libgit2's `OdbDataStr` (`CStrBox<OdbDataStrFree>`) and the `OdbDataStrVec` that
//! formats its elements as `CStr`s.
#![forbid(unsafe_code)]

use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataStrVec};

fn main() {
    let srcs = [
        c"refs/heads/main",
        c"q\"t\\",
        c"caf\xe9",
        c"h\xc3\xa9llo",
        c"Jos\xc3\xa9 \xe2\x9c\x93",
    ];
    let mut differ = 0;
    for src in srcs {
        let s = OdbDataStrFree::dup(src).unwrap();
        let got = format!("{s:?}");
        // What the previous `CStrBox` printed: `debug_tuple("CStrBox").field(&CStr)`.
        let want = format!("CStrBox({src:?})");
        let mark = if got == want { "same" } else { "DIFFERS" };
        println!("{mark:>7}  CStr: {want}\n         ffibox: {got}");
        differ += usize::from(got != want);
    }
    let v = OdbDataStrVec::dup_all(&srcs).unwrap();
    println!("OdbDataStrVec: {v:?}");
    println!("first element: {:?}", v.as_slice()[3]);
    assert!(differ == 0, "{differ} of {} strings format unlike CStr", srcs.len());
}
