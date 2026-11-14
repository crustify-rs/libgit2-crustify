For: libgit2-crustify's issue tracker (https://github.com/crustify-rs/libgit2-crustify/issues),
as a public issue. The defect is in the crate's own Rust exports of the CLI's option parser,
not in libgit2; it is undefined behaviour under Miri's Stacked Borrows model, with no
miscompilation or security impact we know of.

Title: `cli_opt_parse`'s export stores a `char **` for C derived from a borrow that ends first

### Summary

The Rust exports of the CLI's `src/cli/opt.c` (`src/git2/src/cli/opt.rs`, under the
`ffi-export-src-cli-opt-c` feature) adopt the `argv` C passes as a `&mut [Arg]`. A
`CLI_OPT_TYPE_ARGS` spec's variable, a `char **` that C walks after the parse, was stored from a
shared reborrow of that slice:

```rust
slot.set(parser.as_ref().args().get(idx..).unwrap_or_default());
```

The parse's own `slice::from_raw_parts_mut` over the array, and the next export's adoption of
the same `argv`, as `common.c`'s `cli_opt_foreach` does for `hash-object -w`, are unique
reborrows, which end that pointer's access before C reads through it.

### Reproduction

`cli_opt_parse` of `hash-object a.txt -w b.txt`, `cli_opt_foreach` of the same `argv + 1`, then
the walk of the variable, each export called through its C declaration, under Miri:

```text
error: Undefined Behavior: attempting a read access using <8387> at alloc1467[0x10], but that tag does not exist in the borrow stack for this location
help: <8387> was created by a SharedReadOnly retag at offsets [0x10..0x20]
help: <8387> was later invalidated at offsets [0x8..0x20] by a Unique retag
```

Tree Borrows accepts the run. The reproducer, a small Cargo package, is available on request.

### Fix

Fixed in `wave-186/batch-7/run-1`: the exports store C's own pointer plus the position's index,
as the C does, and the parser keeps C's `argv` pointer. A patch is available.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
