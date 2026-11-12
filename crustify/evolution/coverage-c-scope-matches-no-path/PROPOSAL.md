# The coverage report's C scope names the paths the archive records

Status: `open`

## Signal

A tool reports what the source contradicts. Since the static campaign link (`d21c78c70`),
`crustify/scripts/coverage/report.py`'s C figure counts more than C. The scripts are a
campaign artifact the assistant owns.

## Evidence

`wave-169/batch-2` reported its C cells over a denominator of 190,798 lines (`59.03`%).
`wave-169/batch-1` measured the same tree. The coverage variant's static archive,
`crustify/builds/coverage/crustify/libgit2.a`, records its sources relative to the
directory it was configured in, as `libgit2-bindings/c/...`. They resolve to
`/target/libgit2-bindings/c/...`, not the `$PWD/c/...` paths the documented invocation
(dashboard Notes, "Coverage") passes as the C scope. `llvm-cov report` then matches no
source, and the scope falls back to every object: the archive and every test binary, whose
std, dependency and Rust code count as "C". The archive alone, with no scope, gives
libgit2's 86,833 lines, `45.89`% on batch 1's tree, close to the 86,770 lines `libgit2.so`
had before the static link. `-compilation-dir` does not repair the scope, and neither do
paths under `/target/libgit2-crustify/c`.

## Destination

`crustify/scripts/coverage/report.py` and the dashboard Notes' Coverage method (the
assistant's).

## Change

Read the C figure from the C objects alone, with no source filter. The archive holds only
the link unit's TUs. Alternatively, pass the scope in the archive's recorded form, and fail
when a named scope matches no file, rather than falling back to every object.

## Validation

`wave-169/batch-1` computed its cells both ways on the same profiles: the archive-only
figures reproduce the pre-static-link scale, and the filtered scope reproduces batch 2's
190,798.
