To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: StrList and NullStrList lack the slice traits their stand-ins offer

Hi Marius,

This is an ergonomics gap in the crate's safe API, not a soundness bug. It can be handled in
public.

In `src/libgit2/src/util/strlist.rs` (commit aba03725e), `StrList` and `NullStrList` stand in
for a `Vec` of owned strings: they dereference to `&[AllocStr]` and `&mut [AllocStr]`, as
ffibox's `CVec` of owners does. `CVec` also implements `AsRef<[T]>`, `AsMut<[T]>`,
`IntoIterator` for `&` and `&mut`, `PartialEq`, `Eq`, `PartialEq<[T]>` and `Hash`, and ffibox
documents its owners as behaving like their std counterparts. The two lists implement none of
these, so a consumer cannot write `for s in &list`, compare two lists with `==`, use one as a
`HashMap` key, or pass one where `impl AsRef<[AllocStr]>` is taken, without reslicing it
first:

```rust
let a = StrListFree::copy(&[c"x"]).unwrap();
let b = a.clone();
assert!(a == b);        // error[E0369]: binary operation `==` cannot be applied
for s in &a {}          // error[E0277]: `&StrList` is not an iterator
```

The fix implements those traits on both lists, element-wise as `CVec` does, with a unit test.
It is on the review branch (crustify/waves/wave-184). I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
