# Planner run 10

- Repository: `/target/libgit2-crustify` (also reachable as `/target/libgit2-bindings`, a
  symlink), checked out on `crustify/planner/run-10`, forked from `crustify/devel` at
  `6e3eebbba`.
- Settings: `crustify/CRUSTIFY.md`.
- Your run dir: `crustify/artifacts/planner/run-10/`, holding this message and `task.json`.

## Instructions

The user asked for this run after the assistant found the plan behind the Crustify
revisions since planner run 9 (`66def428e`, planned under Crustify `b4cb5bd` and wavefront
`7c244c2`; Crustify is now `6f1e04c`, wavefront `ca31018`, which adds docs alone). Run 9's
composition, and how it was produced, is in `crustify/artifacts/planner/run-9/compose/`
(start from its `README.md`). Your role's workflow applies in full; the user asked for
these two changes in particular:

1. **`dependencies.json` to the current schema.** It still follows the earlier one: no
   `recommendation`, `decision` a bare string (`"wrapper"` for libc), `rust_equivalent`
   records in the earlier shape (`ported`, `feature_parity`, …). Bring every library
   (libc, libcrypto, libssl, libpcre2-8, libz) to the `dependencies-schema` skill's fields:
   assess the crates for each area the campaign uses, as your workflow's Decomposition step
   says, and recommend a route for each library. The user decides from your
   recommendations once this run is promoted. libc's existing `decision` is the user's: carry
   it into the new `decision` shape (its wrapper, `crustify-rs/glibc-bindings`, is the
   remainder route) without changing what it decides, and keep its `wrapper` record and pin.
   Local, unpushed wrappers exist beside the repository (`/target/openssl-bindings`,
   `/target/pcre2-bindings`, `/target/zlib-bindings`); record what they cover where the
   Reuse section asks, but they decide nothing.
2. **Release points under the current C-test rule.** Run 9 counted every C test as a
   referrer that holds back the stages of what it calls, and made a test leave with the
   first of its modules to be integrated. The `porting-workflow` skill's Tests follow their
   modules section now has only the tests that reach a module's internals leave at its
   integration; those that use only its public API stay, check its exports, and do not
   hold back its `port/unexport`; a multi-module test leaves at the first point at which
   it would leave as a test of one of them alone. Re-derive the release points, and the
   later-stage waves they place, under that rule, and under the `scheduling` skill's
   current Later stages section (a stage the item's own port already passes is not
   planned; `port/layout` and `port/memory` are independent).

Keep what has started: waves 0–168, wave 12 (`wip`, `src/util/alloc.c`) included, change
only as the `schedule-schema` skill's Re-planning section allows. No module is frozen.
Your commit's message reports how many `todo` waves and items each change moved, added or
removed.

The settings' `evolution` answer is `evolve`.

## Added after launch

3. **The foundational modules first.** The user expects the seven foundational modules to
   run first, as the `decomposition` skill's Foundational modules section and the
   `scheduling` skill's Order section say ("the foundational modules depend on no other, so
   they run first"; the type-erased ones before the string ones that call them). Run 9's
   `todo` numbering interleaves them with the independent modules instead: wave 169 is
   `src/cli/opt.c`, and 30 waves of other modules come before `deps/reftable/basics.c`'s
   first (199), the last foundational wave being 290. Number the foundational modules'
   `todo` waves first, from 169, in that order, then the rest in topological order, and
   report the correction as a plan correction.
