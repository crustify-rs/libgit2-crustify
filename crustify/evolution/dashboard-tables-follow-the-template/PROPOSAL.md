# A template change that adds a dashboard table reaches the dashboard before batches fill it

Status: `open`

## Signal

A rule an agent had to work out by hand that a skill should state. Crustify `87e3816`
made every batch fill its row of the `Batches: API stability` table. The skill
`accounting` and the template `templates/translate/dashboard.template.md` name that table.
The campaign's `crustify/artifacts/dashboard.md` at the wave base `69ff068e8` has no such
table, in either route's section.

## Evidence

`foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-0/run-1` had to add
the table to both the `Types` and `Symbols` sections, copied from the template, before it
could fill its row. Three other batches launched beside it from the same base
(`wave-181/batch-0`, `wave-182/batch-0`, `wave-184/batch-0`). Each lands on its own wave
branch and fills the same row. If each adds the table too, the copies conflict when the
waves are promoted and the modules integrated. The assistant then has to resolve which
heading, model line and `Σ` rows survive. If a batch does not add the table, its row has
nowhere to go.

## Destination

The spec: `accounting`, The dashboard (section 1). The assistant applies it when a
Crustify change is adopted, as `evolution`'s Adoption section describes.

## Change

In `accounting` section 1, after "keeps the template's format exactly":

> When an adopted Crustify change adds a table or a column to the template, the assistant
> adds it to the dashboard on the working branch before it launches the next batch. A batch
> fills the rows of tables that are already in place, and never adds a table.

## Validation

No rerun is needed: the change only orders a step the assistant already takes. This run
added the table as the template lays it out, with its row and `Σ` rows, and says so in
its dashboard note, so that the integration can keep one copy.
