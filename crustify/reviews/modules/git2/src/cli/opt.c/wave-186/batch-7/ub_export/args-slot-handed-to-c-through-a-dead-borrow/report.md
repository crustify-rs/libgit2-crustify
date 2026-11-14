# The `char **` an `Args` spec stores for C is derived from a borrow that ends with the call

- **finding**: `ub_export/args-slot-handed-to-c-through-a-dead-borrow`
- **batch**: `wave-186/batch-7/run-1`, the module pass of `git2/src/cli/opt.c`.
- **item**: `fn::cli_opt_parse`, and `fn::cli_opt_parser_next` and `fn::cli_opt_foreach`: their
  exports in `src/git2/src/cli/opt.rs`'s `ffi_export` (`wave-186/batch-5`, `c62dee857`), over
  `take_bare` (`fn::parse_arg`), which stores an `Args` spec's position, and `Parser::into_args`.
- **affected commit**: `961150dd9`, the wave's tip; since `c62dee857`.
- **class**: undefined behaviour reached from C through an export, within `opt.h`'s contract, as
  Miri's default aliasing model, Stacked Borrows, defines it: the campaign's Miri gate.

## Finding

A `CLI_OPT_TYPE_ARGS` spec's variable is a `char **`, which C reads after the parse:
`cmd_hash_object.c` walks `filenames` to `argv`'s `NULL`. The C parse stores `&args[idx]`, its
caller's own pointer plus an index. The export adopted `argv + 1` as a `&mut [Arg]`, and
`take_bare` stored the position from a shared reborrow of it,
`ArgsSlot::set(parser.as_ref().args().get(idx..))`: a pointer that grants access only while no
later borrow of the array takes it away. Two do, before C reads it:

- `parse` itself, which hands its array back through `Parser::into_args`, a
  `slice::from_raw_parts_mut` over it: a unique reborrow, which invalidates the stored pointer
  before `cli_opt_parse` returns; and
- any later export over the same `argv`, whose adoption is a unique reborrow of the array:
  `cmd_hash_object.c -w` opens the repository, and `common.c`'s `parse_common_options` parses
  `argv + 1` again through `cli_opt_foreach`, before the files are walked.

C then reads `argv` through a pointer whose access Rust ended. Under Tree Borrows the same run is
accepted, since neither reborrow writes the array here; Stacked Borrows, Miri's default and the
model of the campaign's Miri gate, reports it. Once the module switches, the gateway's tests run
under that gate, and the C that walks the variable is the CLI's.

`cli_opt_parser_init` had the same shape for the parser's own `args`: it stored the adopted
slice's pointer, not C's, so that a position `cli_opt_parser_next` stored was derived from a
borrow too. `main.c`'s parser never reaches its `Args` spec, since it stops at the command.

## Evidence

`src/main.rs` here runs `cmd_hash_object.c`'s sequence, calling each export through its raw
binding as the C does, within `opt.h`'s contract: `cli_opt_parse` of `hash-object a.txt -w
b.txt` against `-w` and a required `Args` spec, then `cli_opt_foreach` of the same `argv + 1`
against `-c`, then the walk of the `char **` to `argv`'s `NULL`. Under Miri, which resolves the
raw bindings to the exports, from the repository root:

```text
CARGO_TARGET_DIR=crustify/tmp/target-miri-repro cargo +nightly-2026-10-08 miri run -q \
  --manifest-path crustify/reviews/modules/git2/src/cli/opt.c/wave-186/batch-7/ub_export/args-slot-handed-to-c-through-a-dead-borrow/Cargo.toml
```

On `961150dd9` it exits 1 (`run-affected.log`):

```text
error: Undefined Behavior: attempting a read access using <8387> at alloc1467[0x10], but that tag does not exist in the borrow stack for this location
  --> src/main.rs:78:29
78 |         let file = unsafe { at.read() };
help: <8387> was created by a SharedReadOnly retag at offsets [0x10..0x20]
  --> src/main.rs:53:9
53 |         sys::cli_opt_parse(opt.as_mut_ptr(), specs.as_ptr().cast(), args, len, sys::CLI_OPT_PARSE_GNU)
help: <8387> was later invalidated at offsets [0x8..0x20] by a Unique retag
  --> src/main.rs:53:9
```

The tag is the one `take_bare` stored, through `ArgsSlot::set`, and the unique retag
`Parser::into_args`'s, as the gateway test that found it showed with the crate's own frames. With `MIRIFLAGS=-Zmiri-tree-borrows` the run exits 0
(`run-affected-tree-borrows.log`).

## Fix

The gateway hands C its own pointers. `rebase_args_slots`, run by `cli_opt_parse`,
`cli_opt_parser_next` and `cli_opt_foreach` once the parse step is done, rewrites each position
stored in an `Args` spec's variable of the table, when it lies within the array C passed, as C's
pointer plus its index, the `&args[idx]` C stores; a position outside it, written by an earlier
parse of another array, is left alone. `cli_opt_parser_init` stores C's `args` in the parser, as
C does, over the adopted slice's, so that `cli_opt_parser_next` rebases onto it. The native API
does not change: its positions are never read through, and `ArgsSlot::get_in` compares
addresses alone. The reproducer exits 0 on the fix (`run-fixed.log`).

It becomes the gateway's test
`ffi_export::unit_priv::the_files_a_parse_stores_outlive_a_later_parse_of_its_argv`, which runs
the same sequence through the exports and walks the files. Under Miri it fails on the old
gateway, with the same report, and passes on the fix. It compiles with the export feature,
as the gateway does, until the switch deletes the feature; from then on it runs in `lib`, and in
the Miri gate.

API stability: no public item changes.
