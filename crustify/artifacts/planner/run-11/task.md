# Planner run 11

- Repository: `/target/libgit2-crustify`, checked out on `crustify/planner/run-11`, forked
  from `crustify/devel` at `89786221a` (planner run 10, `c202c9f36`, promoted, plus its
  usage and a proposal's status).
- Settings: `crustify/CRUSTIFY.md`.
- Your run dir: `crustify/artifacts/planner/run-11/`, holding this message and `task.json`.

## Instructions

Crustify changed after run 10, at the user's request (`043a8ae`, `737aebc`, `bdd0cab`,
`596b74e` on its `devel`); your role's workflow applies in full. In particular:

1. **`dependencies.json` to the new schema.** The `dependencies-schema` skill now maps every
   entity the campaign uses of a library (`c_entities`: `c_name`, `kind`, `header`,
   `used_by`, `matches`, `recommended`, `confirmation` left `null`), assesses each candidate
   in one free-text `quality_assessment`, and names the side campaign `crustify_campaign`
   and the uncovered route `uncovered` (`crustify-campaign`, `local` or `null`). Run 10
   already read the crates: carry its findings into `quality_assessment` and the entities'
   `matches`, reading a crate's API again only where an entity's match needs it. Take the
   entities from the external records of `schedule.json`'s modules. libc's `decision` is
   the user's: carry it into the new shape unchanged (`adopt: []`, `uncovered:
   "crustify-campaign"`, no overrides) and its `wrapper` record into `crustify_campaign`.
   The other decisions stay `null`; the user decides from your recommendations.
2. **C tests hold no export back.** The `scheduling` skill's Later stages, 5.2, now leaves
   the C tests out of a `port/unexport`'s referrers, as wavefront's release points do:
   the stage carries over the C test cases that still reference its export, as Rust tests
   reaching what is still C through its wrappers, and removes them from `c/`. Re-derive
   the release points and later-stage waves under it, in place of run 10's test hold.
   No export should be kept for good because of a test. Record, for each `port/unexport`
   wave, which C test cases it carries over, in the schedule if its schema has a place for
   them and in your composition otherwise, and file a proposal for the schema if it has
   none.
3. **Empty test cases are out of scope.** A test case whose body the configured build
   compiles empty (run 10 counted 42, such as `GIT_WIN32`-only bodies) is out of scope, as
   a configured-out TU is.

Keep what has started: waves 0–168, wave 12 (`wip`) included, change only as the
`schedule-schema` skill's Re-planning section allows. Keep the foundational modules' waves
first. Your commit's message reports how many `todo` waves and items each change moved,
added or removed.

The settings' `evolution` answer is `evolve`.
