# A switch's C tests run per variant by a tracked script and are compared with the baseline

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Equivalence. Its first measure is the target's own tests passing over the Rust at each
switch, against their recorded baseline.

## Signal

"One procedure repeated by hand: the same script written again, the same queries run in
sequence": a tool. Every switch runs the baseline's suites on every variant and compares the
result, and the only runner was an untracked scratch script.

## Evidence

**The C test gate was scratch (item 4).** Met at the campaign's first switch from C,
`modules/libgit2/src/util/utf8.c/integration/run-1` (commits `012d1d34c`, the switch, and the
fork's `1b37e3d08`; dashboard Notes entry `libgit2/src/util/utf8.c` integration). The
baseline's per-suite runner, `crustify/tmp/scripts/c-suites.sh` in the base checkout, is
untracked and writes its results into the variant's dir; a switch's gate compares with that
baseline on every variant, at every switch.

- Where the run went: `crustify/scripts/c-tests.sh` runs `build.json`'s `test` command over
  a copy of the variant's `CTestTestfile.cmake` tree, and every suite alone as the baseline
  did, into an output dir of its own, and compares the tally with `c-baseline.json`'s record.
- `src/cli/opt.c`'s switch (`modules/git2/src/cli/opt.c/integration/run-1`, over
  `crustify/builds.gen2`): `c-tests.sh` ran unchanged.
- `deps/llhttp/api.c`'s switch (`modules/libgit2/deps/llhttp/api.c/integration/run-1`, over
  `crustify/builds.gen3`): `c-tests.sh` ran unchanged.
- `deps/llhttp/http.c`'s switch (`modules/libgit2/deps/llhttp/http.c/integration/run-1`,
  over `crustify/builds.gen5`): `c-tests.sh` ran unchanged.
- From item 11: `c-tests.sh` runs the offline suites the baseline records. The suites the
  baseline disables (`online::*`) are the proposal `switch-gate-beyond-c-tests`'s.

## Destination

From the original's Destination list:

- `testing-gates`, Regressions: a switch's C tests run by `c-tests.sh` per variant the
  baseline records, compared with it.

Protected: all of it. `testing-gates` (`skills/verification/testing/gates/SKILL.md`) and a
script placed beside it are under `skills/verification/**`.

## Change

Not made. In the campaign only, `crustify/scripts/c-tests.sh` exists.

The change would be:

1. `testing-gates`, Regressions, states how a switch runs the target's remaining C tests.
   They run once per variant that the baseline records. Each run writes nothing into the
   variant's dir. Each run's tally is compared with the baseline's record of that variant:
   the case counts, and each suite that exits non-zero. A difference fails the gate and is
   named.
2. `c-tests.sh` moves beside the `testing-gates` skill. It keeps its arguments: the variant
   dir, the output dir and a job count. These libgit2-specific names become arguments or are
   read from `build.json` and the baseline: the test executables (`libgit2_tests`,
   `util_tests`), the ctest targets its `test` command selects (`offline`, `util`), and the
   prefix of the suites the baseline leaves out (`online::`). The way a test executable
   lists its suites (`-l`) and runs one (`-s<suite>`) is also the target's.

## Validation

Copied from the original:

- First switch: `build-variants.sh` built the five variants of `crustify/builds.gen1` from
  the fork's `1b37e3d08`, each smoke-tested (`util_tests -sutf8`); the workspace's gates ran
  over them and `c-tests.sh` compared four variants with the baseline, as the dashboard entry
  records. A later switch validates the generalized scripts by running them.
- `src/cli/opt.c`'s switch ran `build-variants.sh`, `rust-staticlib.sh` and `c-tests.sh`
  unchanged, for a second link unit, and its gates over `crustify/builds.gen2` are its
  dashboard entry's.
- `deps/llhttp/api.c`'s switch ran the three scripts unchanged again, for a third module.
  Its gates over `crustify/builds.gen3` are its dashboard entry's.
- `deps/llhttp/http.c`'s switch ran the scripts again, for a fourth module.

The script with its names turned into arguments has not been run.

## Review

None yet.
