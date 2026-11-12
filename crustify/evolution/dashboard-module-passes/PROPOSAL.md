# The dashboard has no `Module passes` tables for a module pass to fill

Status: `open`

## Signal

A rule an agent works around. `accounting`'s Landing cells section puts a module pass's
row, "which spans both routes, in `Module passes`", and the translate template
(`templates/translate/dashboard.template.md`, Crustify `8e23c1c`) has that section, with
its `Passes` and `Passes: PoC + reports` tables. The campaign's dashboard,
`crustify/artifacts/dashboard.md`, predates it: it has no `Module passes` section, and the
settings ask that its format be preserved exactly.

## Evidence

`foundational/libgit2/src/util/allocators/win32_leakcheck.c/wave-182/batch-1/run-1` is the
campaign's first batch whose `task.json` sets `module_pass` (the earlier reviews of
wave 169's and wave 176's foundational modules, `wave-169/batch-3`, `wave-169/batch-4` and
`wave-176/batch-2`, set neither `module_pass` nor `wave_gates`). With no `Module passes`
table to fill, it recorded its row where wave 176's review recorded its own: in the
`Symbols` route's `Batches`, `Batches: tests`, `Batches: safety audit`,
`Batches: API stability` and `Batches — review: PoC + reports` tables, as a `review` row.
The module's one item is a symbol, so nothing of the pass is lost there; a pass over a
module with types and symbols would have to split its row or pick a route.

## Destination

A campaign artifact the assistant owns: the dashboard. Its `Module passes` section is added
from the template, with a Notes entry, and the module passes recorded in the route tables
since (this one) are moved there, or left with a note saying where they are.

## Validation

None needed: a format change, whose evidence is the missing section and the row placed
without it.
