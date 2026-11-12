Filed by the assistant from d04070734 and 9aa6520a5, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `HashId::size` returns `u32` while the hash-size constants are `usize`

- **finding**: `ergonomics/hash-id-size-is-u32`
- **batch**: `wave-169/batch-4/run-1` of `libgit2/deps/reftable/basics.c`
- **item**: `fn::hash_size`
- **affected commit**: `bbaafcd09`
- **source commits**: `d04070734` (file), `9aa6520a5` (fix)

## Finding

`HashId::size` returned `u32`, while `HASH_SIZE_SHA1`, `HASH_SIZE_SHA256` and
`HASH_SIZE_MAX` are `usize`. Comparing an id's size with the constants, or cutting an id
with it, needed a cast.

## Evidence

As the finding states. The exact command to rerun: not recorded: the report was refused by
the host; filed from d04070734 and 9aa6520a5.

## Reproducer

None (no reproducer).

## Fix

In `9aa6520a5`: `HashId::size` returns `usize`, the unit of `HASH_SIZE_SHA1`,
`HASH_SIZE_SHA256` and `HASH_SIZE_MAX`. The equiv and unit tests follow.
