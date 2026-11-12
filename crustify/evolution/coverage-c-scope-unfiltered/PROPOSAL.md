# The coverage report's C figures ignore their scope and move with the test binaries

Status: `open`

## Signal

A tool that reports what the source contradicts: `crustify/scripts/coverage/report.py`'s C
figures cannot be reproduced and do not depend on the scope they are given.

## Evidence

wave 12 batch 3 (`src/util/alloc.c` review), run 1, on the static campaign link (`d21c78c70`):

- `run.sh "$PWD" libgit2 <dir>`, then `report.py <dir> "C=$PWD/c/src/libgit2:$PWD/c/src/util:$PWD/c/include:$PWD/c/deps,Rust=<dir>/tree/src" --c crustify/builds/coverage/crustify/libgit2.a`,
  as the dashboard's notes give it. On the fixed tree, C `TOTAL` was `112018/190263` lines
  (58.88%). On the wave tip `1dc9911d7`, measured the same way, it was `111979/196575`
  (56.97%). One C archive, one C scope, and the denominator moves by 6,312 lines between two
  Rust trees that differ by 77 lines of `alloc.rs` and its tests.
- With `MERGED=1`, a scope over the base checkout's `c/`
  (`/target/libgit2-crustify/c/...`) and one over the worktree's `c/` print identical C
  figures, with or without `@<compilation dir>`. The source filter selects nothing, and the
  report falls back to every file the objects map.
- The dashboard's Coverage stats record 45.90% for the same total (batches 1 and 2 of wave
  12, measured "with the coverage variant's static archive as the C object"). Neither run
  reproduces it. The Rust figures do reproduce, to 0.01 pp.

## Destination

A campaign artifact: `crustify/scripts/coverage/report.py` and `run.sh`, which setup owns,
and possibly the `testing-builds` skill's Coverage section, if objects must be named
differently for a static link.

## Change

Not made, since the cause is not settled. What to check: whether the static archive's
coverage mapping records its sources relative to `crustify/builds/coverage`, so that the
absolute filters never match. Also whether each test binary's own copy of the archive
members it links adds C mapping regions to the union, so that the C denominator follows the
binaries. A fix makes the C scope select by a path that matches what the objects record,
and takes the C denominator from the archive alone.

## Validation

None yet. The two figure sets above, under `crustify/tmp/coverage` and
`crustify/tmp/coverage-base` of the batch's worktree, are the before and after to
reproduce.
