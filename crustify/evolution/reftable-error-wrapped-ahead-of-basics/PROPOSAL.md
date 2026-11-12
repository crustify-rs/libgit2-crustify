# Wrap `enum reftable_error` ahead of its first consumer, `deps/reftable/basics.c`

Status: `open`

## Signal

A plan correction. A review finding, `wave-169/batch-4`'s
`conventions/reftable-error-type-homed-in-basics`, shows that the plan schedules a module
before an item it uses.

## Evidence

- `deps/reftable/basics.c` (foundational, wave 169) returns `REFTABLE_OUT_OF_MEMORY_ERROR`
  and `REFTABLE_FORMAT_ERROR` (`basics.c:128,213,224,230,238`). These are constants of
  `enum reftable_error` in `deps/reftable/reftable-error.h`. Wave 169's `depends_on` is
  `{"scheduled": [], "cut": []}`.
- `schedule.json` holds `enm::reftable_error` and `fn::reftable_error_str` in wave 217 (wave 216 before planner run 12)
  (`deps/reftable/error.c`, home `src/libgit2/deps/reftable/error.rs`). It holds
  `reftable_error_str` again in wave 1103.
- With nothing to bind against, wave-169/batch-2 defined `basics::Error`, an unanchored
  four-variant enum (`Format`, `Api`, `OutOfMemory`, `Other(c_int)`), in
  `src/libgit2/deps/reftable/basics.rs`. Its `Display` restates three of
  `reftable_error_str`'s strings. Every later reftable module will return it, and wave 217
  will bind the same enum in `error.rs` with a variant for each constant.

## Destination

The plan, `schedule.json` and the wavefront configuration, which the planner owns. Or the
assistant's task message for wave 217.

## Change

Either of these:

1. Schedule `enm::reftable_error` in a wrap batch ahead of wave 169's consumers, homed in
   `error.rs`, and have `basics.rs` re-export it in place of its own `Error`. `scheduling`'s
   Wrap batches section describes such a batch.
2. Tell wave 217's batch to make `basics::Error` its binding of `enm::reftable_error`: move
   it to `error.rs`, give it a variant per constant with the anchors, keep the `basics`
   re-export, and give `Display` `reftable_error_str`'s text.

Wavefront's dependency graph did not tie `basics.c` to the enum's constants. If that is
because enum-constant uses are dropped, it is a wavefront gap and gets a proposal of its
own once confirmed.

## Validation

None yet: this is a scheduling decision.
