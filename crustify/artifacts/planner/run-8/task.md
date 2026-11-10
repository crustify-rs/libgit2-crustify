# Planner run 8

- Repository: `/target/libgit2-bindings`, checked out on `crustify/planner/run-8`.
- Settings: `/target/libgit2-bindings/crustify/CRUSTIFY.md`.
- Task: `crustify/artifacts/planner/run-8/task.json`.

## Instructions

This run brings the plan up to wavefront's schema v6 and Crustify's granular item kinds.
Since run 7, wavefront has moved from 2a95e3c to 88793d7: it tells unions from structs,
schedules every entity under its C kind (`struct`, `union`, `enum`, `function`, `global`,
`macro`, `function_pointer`, `typedef`), folds the typedefs that name a struct, union, enum
or function pointer into that item's `typedefs`, and gives function pointers and typedefs
of their own `unreferenced_after`. Crustify's skills changed with it (crustify b47995e).
The assistant re-extracted the CodeQL T1/T2 tables with wavefront 88793d7 from the
unchanged database: `t2/function_pointer_uses.csv` and
`t2/function_pointer_signature_types.csv` are new, and `t1/types.csv`, `t1/functions.csv`,
`t2/layout_uses.csv` and three other T2 tables changed. The T3 tables and the ownership
store were not touched.

What follows an earlier schema is updated as your workflow's first step describes, among
it:

- the configuration's module items, whose `type` and `symbol` kinds wavefront 88793d7
  refuses, take their C kinds;
- `schedule.json`'s items take their C kinds and `typedefs`, the function pointers and
  typedefs of their own become items where the scheduling skill places them, and the
  release points and later-stage items are re-composed from the new tables; and
- `modules.json`'s `callbacks` counters become `function_pointers`.

Departure, at the user's request: there is no backward-compatibility layer, so the items
of the waves that have started, the 88 `done` waves and wave 12, which is `wip` with no
batch running, also take their new `kind` (a `callback` item becomes `function_pointer`)
and `typedefs`, in place. Nothing else of those waves changes: not their items' set, names,
`home` or `field_users`, their batches, commits, progress or indices. This overrides the
rule that a started wave changes only in its progress, for this migration alone.

The `ty::` anchors in the landed Rust are not yours: the assistant retags them after this
run. In your commit message, give the items per kind before and after, and the counts of
the later-stage items per objective.
