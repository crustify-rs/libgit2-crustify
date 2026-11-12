Filed by the assistant from 7db163587 and 81dede2b1, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `parse_names` counts the array C returns by hand, reproducing `names_length`

- **finding**: `misc/parse-names-counts-its-result-by-hand`
- **batch**: `wave-169/batch-5/run-1`, the module pass of `libgit2/deps/reftable/basics.c`,
  over wave 169's batches 0 to 4 as the wave tip `9aa6520a5` holds them
- **items**: `fn::parse_names`, `fn::names_length`
- **affected commit**: `9aa6520a5`
- **source commits**: `7db163587` (file), `81dede2b1` (fix)

## Finding

`parse_names` counted the array C returns in Rust, with two raw-pointer unsafe blocks
(`out.add(count)`, `slot.read()`): `names_length`'s loop, reproduced beside its binding,
which the module links and calls in `NamesRef::len`. One call to `names_length` counts it.

## Evidence

As the finding states. The exact command to rerun: not recorded: the report was refused by
the host; filed from 7db163587 and 81dede2b1.

## Reproducer

None (no reproducer).

## Fix

In `81dede2b1`: `parse_names` counts C's array with one call to `names_length`.
