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
