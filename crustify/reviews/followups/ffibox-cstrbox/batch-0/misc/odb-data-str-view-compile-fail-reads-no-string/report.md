# `odb_data_str_view_outlives_owner` no longer uses the string after the drop

- **item**: `crustify/rust/libgit2/tests/ub_safe/compile_fail/odb_data_str_view_outlives_owner.rs`
- **affected commit**: `ac7dd20e5`
- **class**: misc, a weakened compile-fail case
- **fixed**: yes, in this batch's second commit

## Finding

The case shows that a borrowed view of an `OdbDataStr` cannot outlive the string.
Before `ac7dd20e5`, the use after `drop(s)` was `view.to_bytes()`. That read the freed
string, so the program the compiler must reject was a use after free. The commit
rewrote it as:

```rust
let run = s.as_handles();
drop(s);
let _ = run.len();
```

`CSlice::len` returns the length stored in the view (`ffibox/src/refs.rs`, `CSlice`),
so it touches no string memory. The case still fails to compile (`E0505`), and it
would still catch a `CSlice` that lost its lifetime. But the illegal path it shows no
longer reaches the freed string. If the borrow checker accepted it, nothing would go
wrong. A compile-fail case under `ub_safe` exists to show that a UB path is rejected.

The other two `odb_data_str_*` cases still read the string after their illegal step,
through `data_str_bytes`, and test what they are named for:
`odb_data_str_vec_element_outlives_owner` and
`odb_data_str_vec_aliased_reborrow`.

## Fix

The use is now `run.elem(0)`, which copies the first byte out of the string, so the
rejected program reads freed memory. The comment names what the run reads, and the
`.stderr` is updated: it has the same error, labels and line numbers, with the new
expression. From `crustify/rust`,
`scripts/cargo-variant.sh plain test -p libgit2 --test ub_safe compile_fail` passes.
