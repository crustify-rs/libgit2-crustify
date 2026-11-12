Filed by the assistant from d04070734 and 9aa6520a5, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `ReftableArrayFree::grow` gives installed-mode packaging as its reason for being a port

- **finding**: `misc/ported-grow-cites-installed-mode`
- **batch**: `wave-169/batch-4/run-1` of `libgit2/deps/reftable/basics.c`
- **item**: `fn::reftable_alloc_grow`
- **affected commit**: `bbaafcd09`
- **source commits**: `d04070734` (file), `9aa6520a5` (fix)

## Finding

`ReftableArrayFree::grow` (Replaces `fn::reftable_alloc_grow`) gave as its reason that the
generated wrapper "includes internal headers, which a build from the installed headers does
not have". binding-symbols permits the port for a different reason: the function is short,
its whole body is in basics.h, and it does not depend on the build's configuration.

## Evidence

The spec packages in bundled mode, and installed-mode packaging already fails since
wave-12/batch-2.

The exact command to rerun: not recorded: the report was refused by the host; filed from
d04070734 and 9aa6520a5.

## Reproducer

None (no reproducer).

## Fix

In `9aa6520a5`: `ReftableArrayFree::grow`'s doc gives binding-symbols' reason for the port:
a short static inline function, its whole body in basics.h, and independent of the build's
configuration.
