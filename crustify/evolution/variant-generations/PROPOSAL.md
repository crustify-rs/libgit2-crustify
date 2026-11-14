# The build variants are built in generations, not rebuilt in place

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Incremental migration without breaking consumers. Every landing builds and passes its
gates, and a batch still running keeps the build it started against.

## Signal

"Skills that contradict each other, or an agent prompt that leads its agent wrong": a defect
in the spec. A switch that follows `porting-workflow`'s Builds section as written breaks the
builds of the batches still running (item 1). Items 8 and 17 are the row "one procedure
repeated by hand": each switch divides the job count and picks a smoke test by hand.

## Evidence

Four switches met these items: `modules/libgit2/src/util/utf8.c/integration/run-1` (the
first), `modules/git2/src/cli/opt.c/integration/run-1`,
`modules/libgit2/deps/llhttp/api.c/integration/run-1` and
`modules/libgit2/deps/llhttp/http.c/integration/run-1`. The first run's commits are
`012d1d34c` (the switch) and the fork's `1b37e3d08`; its dashboard Notes entry is
`libgit2/src/util/utf8.c` integration. The later switches' records are their commits on each
module's branch, the fork's `f57e1a616`, `c54c0c180` and `230c9dbdb`, and their dashboard
Notes entries (`git2/src/cli/opt.c` integration, `libgit2/deps/llhttp/api.c` integration,
`libgit2/deps/llhttp/http.c` integration).

**The variants cannot be rebuilt in place (item 1).** `porting-workflow`'s Builds section
rebuilds the shared variants "from the C that remains once the integration lands", and
`testing-builds` has them prebuilt and immutable. Each variant's `configure` takes the C from
the base checkout (`cmake -S ../../../c`), whose `c/` the batches still running hold with the
module as C: three agents were building and testing against `crustify/builds` while this
switch ran. A build configured from the integration worktree's `c/` instead is bound to a
tree that is purged once the module integrates, and its build files then name sources that
no longer exist.

- Where the first run went: the variants were rebuilt as `crustify/builds.gen1`, a directory
  beside `crustify/builds`, which the integration worktree's `crustify/builds` links to, each
  configured from `crustify/builds.gen1/c`, a detached worktree of the fork at the commit the
  generation was built from (`crustify/scripts/build-variants.sh`). The side campaigns'
  installs, reached as `$PWD/../../../..`, resolve the same from a sibling of `builds`.
- `src/cli/opt.c`'s switch: `build-variants.sh` built `crustify/builds.gen2` from
  `f57e1a616`, five variants in about 15 minutes, each archive holding 211 objects where
  gen1's held 212, `opt.c.o` gone. The script ran unchanged.
- `deps/llhttp/api.c`'s switch: `build-variants.sh` built `crustify/builds.gen3` from
  `c54c0c180`, five variants in about 7 minutes at 12 jobs each (item 8's split again, for
  the 64 the run was capped at), each archive holding 210 objects where gen2's held 211,
  `api.c.o` gone. The script ran unchanged.
- `deps/llhttp/http.c`'s switch: `build-variants.sh` built `crustify/builds.gen5` from
  `230c9dbdb`, five variants in about 6 minutes at 12 jobs each (item 8's split, for the
  run's cap of 64), each archive holding 209 objects where gen4's held 210.
- From item 10: each variant's `build` runs `rust-staticlib.sh`, which checks the committed
  bindings against the variant's headers, so a generation is built only after the switch's
  Rust side is committed. The bindings are regenerated against the previous generation, whose
  headers the switch does not change. The proposal `switch-step-order` takes item 10 whole.

**`build-variants.sh` takes no job count (item 8).** It builds every variant side by side,
each `cmake --build` and `cargo` at the environment's job count, so a host cap holds only if
the caller divides it by hand.

- `src/cli/opt.c`'s switch split the host's cap of 48 by hand, `CARGO_BUILD_JOBS=10` and
  `CMAKE_BUILD_PARALLEL_LEVEL=10` for the five variants.
- The two llhttp switches ran 12 jobs each for a cap of 64, as above.

**The variants' smoke test is gone (item 17).** The earlier switches smoke-tested each
variant with `util_tests -sutf8`, which has found no suite since `git_utf8_char_length`'s
`port/unexport` (wave 232) carried its C test over, gen4 included. Nothing names the smoke
test, so each switch picks one; a suite of a module that stays C the longest, or
`build.json`'s own, would keep it from leaving with a module.

- First switch: each of gen1's five variants was smoke-tested with `util_tests -sutf8`.
- `deps/llhttp/api.c`'s switch: each variant passed its smoke test (`util_tests -sutf8`).
- `deps/llhttp/http.c`'s switch: each variant passed `util_tests -sstr` and `git2 --version`.

## Destination

From the original's Destination list:

- `setup`, Builds, and `testing-builds`, Builds: the variants are built in generations, each
  from a worktree of the fork of its own, and a checkout's `crustify/builds` links to one; a
  switch builds the next generation, and the assistant moves the base checkout to it once the
  switch lands, so that the batches in flight keep theirs. `build-variants.sh` beside `setup`.
- `build-variants.sh` (item 8): a job count, divided among the variants it builds.
- `setup`, Builds, or `build.json` (item 17): the variants' smoke test is named once, from C
  that stays.

Protected: `testing-builds` (`skills/verification/testing/builds/SKILL.md`, under
`skills/verification/**`). Not protected: `setup`, the script beside it, and the
`build.json` template.

Not in the original's list, but the same sentence: `porting-workflow`'s Builds section ("The
shared variants are rebuilt from the C that remains once the integration lands") and
`testing-gates`' Regressions section ("rebuilds the variants from the C that remains") say
what `setup` says. `testing-gates` is protected.

## Change

Not made. In the campaign only, `crustify/scripts/build-variants.sh` exists and built four
generations.

The change would be:

1. `setup`, Builds, and `testing-builds`, Builds, state that the variants are built in
   generations. A generation is a directory beside the shared builds. It holds a detached
   worktree of the fork at the commit it is built from, and one build dir per variant, each
   configured from that worktree. A checkout's builds path is a link to one generation. A
   generation is built once and never rebuilt.
2. When a switch removes C, it builds the next generation. Once the switch lands, the
   assistant moves the base checkout's link to the new generation. The batches in flight keep
   the generation they started against.
3. `build-variants.sh` moves beside the `setup` skill. Its header and body, as read today,
   name the generation dir and the variants as arguments and take everything else from
   `build.json`; no libgit2 name needs to become an argument. It gains one argument, a job
   count, which it divides among the variants it builds and passes to each build.
4. `setup`, Builds, or the `build.json` template, names each variant's smoke test once. The
   test is taken from C that stays C the longest, so that no module's switch removes it.

## Validation

Copied from the original:

- First switch: `build-variants.sh` built the five variants of `crustify/builds.gen1` from
  the fork's `1b37e3d08`, each smoke-tested (`util_tests -sutf8`); the workspace's gates ran
  over them and `c-tests.sh` compared four variants with the baseline, as the dashboard entry
  records. A later switch validates the generalized scripts by running them.
- `src/cli/opt.c`'s switch ran `build-variants.sh`, `rust-staticlib.sh` and `c-tests.sh`
  unchanged, for a second link unit, and its gates over `crustify/builds.gen2` are its
  dashboard entry's. Item 8 is its evidence, and is not made.
- `deps/llhttp/api.c`'s switch ran the three scripts unchanged again, for a third module. Its
  gates over `crustify/builds.gen3` are its dashboard entry's.
- `deps/llhttp/http.c`'s switch ran the scripts again, for a fourth module. Item 17 is
  evidence, and is not made.

## Review

None yet.
