Filed by the assistant from d04070734 and 9aa6520a5, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `Error`'s `Display` is not `reftable_error_str`'s text for most codes

- **finding**: `equiv/error-display-is-not-reftable-error-str`
- **batch**: `wave-169/batch-4/run-1` of `libgit2/deps/reftable/basics.c`
- **items**: `enm::reftable_error`, `fn::reftable_error_str`
- **affected commit**: `bbaafcd09`
- **source commits**: `d04070734` (file), `9aa6520a5` (fix)

## Finding

`Error`'s `Display` was documented as "the text reftable_error_str gives the code". That
holds only for `Format`, `Api` and `OutOfMemory`.

## Evidence

The reproducer (cargo run, plain variant) prints both texts for codes -14..=-1 and fails:

```
-2: C "I/O error", Rust "reftable error code -2"
-1: C "general error", Rust "reftable error code -1"
Display diverges from reftable_error_str for
  [-14, -12, -11, -10, -9, -8, -7, -5, -4, -2, -1]
```

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report: `cargo run`, plain variant. The command's
environment in full: not recorded: the report was refused by the host; filed from d04070734
and 9aa6520a5.

## Fix

In `9aa6520a5`: `Display`'s doc now says it gives `reftable_error_str`'s text for `Format`,
`Api` and `OutOfMemory`, and prints any other code by its number. The equivalence test
`error_display_is_reftable_error_str_for_the_named_codes` compares the three named codes
with C and asserts the documented divergence for the rest. It declares `reftable_error_str`
locally, since wave 216 binds it. unit_pub already asserts `Other(-2)`'s text.
