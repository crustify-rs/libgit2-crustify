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

### The cause of the scope miss (wave-181/batch-0)

`foundational/libgit2/src/util/allocators/stdalloc.c/wave-181/batch-0/run-1` read the
coverage variant's archive alone with `llvm-cov report -instr-profile <suite>.profdata
crustify/builds/coverage/crustify/libgit2.a`, the nightly's `llvm-cov`, on both its trees.
`llvm-cov export -summary-only` names the archive's sources as
`/target/libgit2-bindings/c/src/...`: the variant was configured from the campaign's earlier
checkout (`CMAKE_HOME_DIRECTORY:INTERNAL=/target/libgit2-bindings/c` in its `CMakeCache.txt`),
and that is the path its coverage mapping records. A scope over `$PWD/c/...` therefore
selects nothing, and `llvm-cov` falls back to every file. With the archive as the only
object:

- no filter gives 86,833 lines on both trees, 45.92% on the wave base `69ff068e8` and 45.85%
  on the batch's tree; and
- the filter `/target/libgit2-bindings/c/src/ /target/libgit2-bindings/c/include/
  /target/libgit2-bindings/c/deps/` gives 86,770 lines (45.88% on the tree). That is the
  denominator the dashboard's `Overview test counts` note records for the earlier
  `libgit2.so` measurement.

So the denominator moves only when the test binaries are passed as objects beside the
archive, since each binary maps its own copy of the members it links. The change this
supports: `report.py` reads the C figure from the archive alone, and maps the recorded
source root to the checkout's `c/` (`-path-equivalence=<CMAKE_HOME_DIRECTORY>,$PWD/c`) or
filters by the recorded root, so that a C scope selects what it names.

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
