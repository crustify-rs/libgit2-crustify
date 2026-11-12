# Planner run 12

- Repository: your worktree, `/target/libgit2-crustify/crustify/.worktrees/planner/run-12`,
  checked out on `crustify/planner/run-12`, forked from `crustify/devel` at `6ae0bd6f9`. Its `c/`
  is the C submodule at its pinned commit. Its `crustify/wavefront/codeql` and
  `crustify/wavefront/.cache` link to the main checkout's.
- Settings: `crustify/CRUSTIFY.md`.
- Your run dir: `crustify/artifacts/planner/run-12/`, holding this message and `task.json`.
- Query wavefront as `wavefront --config $PWD/crustify/wavefront/configs/wavefront-config.json c …`
  from the worktree's root. The config path must be absolute, since a relative one
  resolves against `c/`.

## Why this run

wavefront `e529c26` ("Order a macro after what its replacement list references"), on its
`devel` with the user's approval, gives every macro the dependency edges its body
expansion references. Before it, no macro had any, contrary to wavefront's docs. That is
proposal `header-macros-after-their-expansions`. The campaign's CodeQL tables were
re-extracted under it, so `t2/macro_references.csv` exists.

Re-plan the `todo` waves under the new order, as the `schedule-schema` skill's
Re-planning section allows. wavefront's own comparison on this campaign's earlier
configuration found:
- 651 of 7,473 units moved: 397 within their module, as macros now follow their
  expansions and functions follow the macros they expand, and 254 because 11 modules
  changed layer, such as the xdiff modules one layer later, `src/util/hashmap.h` from
  layer 0 to 3, and `src/cli/opt_usage.c` earlier;
- module cuts went from 201 to 206 and item cuts from 1,314 to 1,340;
- there are new cycles, `poll` ↔ `p_poll`, via `CLI_COMMON_OPT_HELP` and
  `XDL_ALLOC_GROW`, and `xemit.c` ↔ `xprepare.c`, and the errors/str cycle grows from 12 to
  21 nodes, each cut by the existing rule; and
- `GIT_INLINE` becomes a unit no module owns.

Check each of these against the `decomposition`, `scheduling` and `dependencies` skills,
and say in your commit message what you did with each.

## Instructions

1. **Keep what has started.** Waves 0 to 185, every wave that is `wip` or `done`, keep
   their records but for progress. Every module that is `frozen` keeps its
   decomposition, as the `modules-schema` skill's Status section says. All 7
   foundational modules are integrated, and their landed items are not scheduled again.
2. **Wrap batches ahead.** Crustify `93674e1` and `66e2ae8` changed `scheduling`'s Wrap
   batches section: an item that another batch of `schedule.json` holds, on any branch, is
   neither wrapped again nor reached through raw bindings, and a module a wrap batch
   homes an item in is frozen when that batch launches. No such batch is running now, so
   this changes no record. Plan under it.
3. **Report the changes.** Your commit message reports how many `todo` waves and items
   moved, were added or were removed, and the new index of the wave holding the last item
   of each module that the settings' first ship point names (`ship` answer). The settings
   say "planned through wave 263", and the assistant updates that from your report.
4. **Wave 216.** Proposal `reftable-error-wrapped-ahead-of-basics` refers to "wave 216",
   the wave holding `enm::reftable_error` and `fn::reftable_error_str`. Give their new wave
   index in your commit message. The assistant updates the proposal.

The settings' `evolution` answer is `evolve, unattended: the whole campaign`.
