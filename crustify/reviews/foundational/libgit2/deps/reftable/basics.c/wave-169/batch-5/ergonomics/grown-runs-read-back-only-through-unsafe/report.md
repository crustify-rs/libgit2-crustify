Filed by the assistant from 7db163587 and 81dede2b1, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# A grown run's elements are read back only through `unsafe`

- **finding**: `ergonomics/grown-runs-read-back-only-through-unsafe`
- **batch**: `wave-169/batch-5/run-1`, the module pass of `libgit2/deps/reftable/basics.c`,
  over wave 169's batches 0 to 4 as the wave tip `9aa6520a5` holds them
- **item**: `mac::REFTABLE_ALLOC_GROW`
- **affected commit**: `9aa6520a5`
- **source commits**: `7db163587` (file), `81dede2b1` (fix)

## Finding

`REFTABLE_ALLOC_GROW`'s forms, `ReftableArrayFree::grow` and `grow_or_clear`, and `resize`,
take and leave a `ReftableVec<MaybeUninit<T>>`, as the C leaves the new elements unwritten.
A consumer can write the elements safely but reads back what it wrote only with `unsafe`
`assume_init`, and the run never becomes a `ReftableVec<T>` without unsafe. ffibox's G2 asks
that a consumer write no unsafe.

## Evidence

The module's own tests show it: equiv's `grow_matches_reftable_alloc_grow` and
`grow_or_clear_keeps_elements_as_reftable_alloc_grow_does`, and unit_priv's allocation
sweep, read kept elements through unsafe, and neither ub_safe nor unit_pub can read a grown
element at all. The C callers' pattern, writing slot nr and counting it, is
`Vec::resize_with`'s in Rust.

The exact command to rerun: not recorded: the report was refused by the host; filed from
7db163587 and 81dede2b1.

## Reproducer

None (no reproducer).

## Fix

In `81dede2b1`: `ReftableArrayFree::grow_with(run: &mut ReftableVec<T>, min_len, fill)`
grows an initialised run as `grow` does and fills each new element with `fill()`, as
`Vec::resize_with` does: the elements keep their values, a failure leaves the run as it was,
and a panic in `fill` leaves it holding what was filled before, through a private `Refill`
guard. `grow`, `grow_or_clear` and `resize` keep the C's uninitialised form. Tests: unit_pub
`grow_with_keeps_the_elements_and_fills_the_new_ones` and ub_safe
`grown_runs_hold_each_owner_once_whatever_the_fill_does` (ASan, and LSan for an owner lost).
