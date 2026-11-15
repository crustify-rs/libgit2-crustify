# The coverage report checks its Rust scope against paths llvm-cov prints relative

Status: `open`.

## Signal

A tool's defect that did not block the run: a documented invocation fails, and is worked
around by hand.

## Evidence

`wave-225/batch-0/run-1`, on `git archive 50c93dd23` and on its own tree, after
`crustify/scripts/coverage/run.sh <repo> libgit2-crustify <out>`:

```text
$ python3 crustify/scripts/coverage/report.py <out> "C=@c,Rust=<out>/tree/src" \
    --c crustify/builds/coverage/crustify/libcrustify.a
scope ['<out>/tree/src'] reports 48 files outside it, such as deps/reftable/basics.rs:
its filters match no source
```

`llvm-cov report` prints each file relative to the files' common prefix, here
`<out>/tree/src/libgit2/`, since `src/lib.rs` and `src/sys.rs` hold no counted region.
`report.py`'s stray check resolves each row against its own working directory, so every row
reads as outside the scope, though the filter matched. Run from `<out>/tree/src/libgit2`,
the same command passes and reports the figures the batch recorded.

`wave-219/batch-0/run-1` (`src/util/cc-compat.h`), an occurrence: its task message names the
workaround. Run from `<out>/tree/src/libgit2`, with `"C=@c,Rust=<out>/tree/src"`, the report
passes on `git archive f7f0eaaa0` and on the batch's tree.

## Destination

A campaign artifact the assistant owns: `crustify/scripts/coverage/report.py`.

## Change

Resolve each row against the common prefix of the files `llvm-cov` reports, or read the
file list from `llvm-cov export -summary-only`, whose `filename` fields are absolute,
rather than against the working directory.

## Validation

The two runs above: from the worktree's root, the check fails on both trees; from
`<out>/tree/src/libgit2`, it passes on both. A fix is validated by the first form passing.
