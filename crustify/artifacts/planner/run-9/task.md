# Planner run 9

Run by the assistant in its own session, at the user's request ("you adjust the schedule
yourself and fix the stale records"), in place of a planner agent: a departure from the
assistant's role, which spawns planner runs rather than planning.

- Repository: `/target/libgit2-bindings`, on `crustify/planner/run-9`.
- Settings: `crustify/CRUSTIFY.md`.

## Instructions

- Bring the plan to wavefront 7c244c2, which makes a unit of every macro a published
  header defines, whether or not the closure expands it, and leaves include guards out of
  the API: re-extract the changed T1 table and re-run run 8's composition over it.
- Give the started waves' items, and their batches' items, their release points afresh,
  which run 8 left as run 7 computed them. Nothing else of the started waves changes.

The composition and what it changed are in `compose/README.md`.
