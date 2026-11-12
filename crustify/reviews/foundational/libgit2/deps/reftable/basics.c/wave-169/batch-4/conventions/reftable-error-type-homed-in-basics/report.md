Filed by the assistant from d04070734 and 9aa6520a5, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# basics.rs homes an unanchored stand-in for `enm::reftable_error`, which wave 216 holds

- **finding**: `conventions/reftable-error-type-homed-in-basics`
- **batch**: `wave-169/batch-4/run-1` of `libgit2/deps/reftable/basics.c`
- **items**: `enm::reftable_error`, `fn::reftable_error_str`
- **affected commit**: `bbaafcd09`
- **source commits**: `d04070734` (file), `9aa6520a5` (fix commit of the batch; this finding
  is not patched in it)

## Finding

basics.rs defines `pub enum Error`, an unanchored stand-in for `enm::reftable_error` with
four variants. Its `Display` restates three of `reftable_error_str`'s strings
(deps/reftable/error.c). schedule.json holds both items in wave 216, homed in
src/libgit2/deps/reftable/error.rs, and wave 169 records no dependency on them. Every later
reftable module will return `basics::Error`, and wave 216 would bind a second error type for
the same convention.

## Evidence

As the finding states: basics.rs's `pub enum Error`, deps/reftable/error.c, and
schedule.json's wave 216. The exact commands to rerun: not recorded: the report was refused
by the host; filed from d04070734 and 9aa6520a5.

## Reproducer

None (no reproducer).

## Fix

Not patched: the fix moves or completes an item another wave holds
(`enm::reftable_error`, which wave 216 holds). Proposed to the plan's owner in
crustify/evolution/reftable-error-wrapped-ahead-of-basics/.
