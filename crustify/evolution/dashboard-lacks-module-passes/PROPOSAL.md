# Give the dashboard the template's `Module passes` table

Status: `open`

## Signal

Skills that contradict a campaign artifact: a rule an agent had to work around.

## Evidence

- `accounting`'s Landing cells section places a module pass's rows "in `Module passes`",
  and `templates/translate/dashboard.template.md` has that table (`## Module passes`,
  `### Passes`: module, wave, batch, types, symbols, +LoC, $, tokens, wall), with its own
  review column set for both routes (`+contracts import`, `+ergonomics`, `+logic`, ...).
- `crustify/artifacts/dashboard.md` keeps an older layout: no `Module passes` table, and
  review columns without `ergonomics` or `contracts` (batches 3 and 4 of wave 169 counted
  ergonomics under `+misc` and contracts under `+internal`). The settings ask that its
  format be preserved exactly.
- `wave-169/batch-5`, the first module pass this campaign lands, reviewed both routes of
  `deps/reftable/basics.c`. With no table for it, its rows went into the Symbols route's
  tables, all its findings being symbol-route items, and a Note says so.

## Destination

A campaign artifact the assistant owns: `crustify/artifacts/dashboard.md`. Migrating it to
the template's current tables is a format change the settings leave to the user.

## Change

Add the template's `Module passes` section, move `wave-169/batch-5`'s row there, and adopt
the template's review columns, mapping the earlier `+internal` cells to `+contracts import`
and recounting the `ergonomics` findings counted under `+misc` from their commit messages.

## Validation

None: a format change, checked by reading the template beside the dashboard.

## Further evidence: the integration of `deps/reftable/basics.c`

`foundational/libgit2/deps/reftable/basics.c/integration/run-1` merged `crustify/devel`
(`1043d550f`), whose dashboard had gained the `Module passes` section with
`src/util/allocators/failalloc.c`'s module pass (`wave-176/batch-3`) in it. The merge keeps
every row where its side put it, so the merged dashboard holds both layouts: `wave-176/batch-3`
in `Passes` and `Passes: PoC + reports`, and `wave-169/batch-5` in the Symbols route's
`Batches` and `Batches — review: PoC + reports`, under the older review columns. What is left
of the change is the move of `wave-169/batch-5`'s two rows, with its review columns mapped,
and the Symbols `Σ review` cells that count it recomputed.

## Further evidence: the integration of `src/util/allocators/win32_leakcheck.c`

`foundational/libgit2/src/util/allocators/win32_leakcheck.c/integration/run-1` merged
`crustify/devel` (`27407ede6`) into the module's branch (`d2b4d3e8b`). Its module pass,
`wave-182/batch-1`, was planned before the section reached the module's base, so it too
recorded its rows as `review` rows of the Symbols tables, beside `wave-169/batch-5`'s, and
filed the same signal as `crustify/evolution/dashboard-module-passes/`, which this merge
brings beside this proposal. The two are one change: the move now covers both passes' rows
(`wave-169/batch-5` and `wave-182/batch-1`), and `dashboard-module-passes` can be closed as
a duplicate of this one.
