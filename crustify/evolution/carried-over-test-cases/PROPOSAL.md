# The test cases a `port/unexport` carries over, in the schedule

Status: `open`

## Signal

One representation written by hand where the schema has no place for it. The scheduling
skill's Later stages section, 5.2 (Crustify `596b74e`), has each `port/unexport` carry
over the C test cases that still reference its export, and remove them from `c/`. The
testing skill's C tests carried over section asks that the cases none carries over be
listed before a test leaves. But `schedule.json`'s planned items have no field for the
cases. Planner run 11 of `crustify-rs/libgit2-crustify` had to record them outside the
schedule, in its composition (`crustify/artifacts/planner/run-11/compose/carryover.json`,
from `carryover11.py`), where the batch that runs the wave does not look.

## Evidence

- `crustify/artifacts/planner/run-11/task.md`, item 2: record "which C test cases it
  carries over, in the schedule if its schema has a place for them and in your
  composition otherwise".
- The schedule-schema skill's planned item: `name`, `defined_in`, `kind`, `field_users`,
  `typedefs`, `home`, `objective` and the three release points. Nothing names a test.
- Run 11's plan has 906 waves holding a `port/unexport`. 90 of them carry over at least
  one case, 1211 cases in all. One wave carries over 603 cases: wave 766,
  `src/libgit2/repository.c`, through `git_repository_open`, which `deps/clar`'s
  `cl_git_sandbox_init` calls. 11 waves remove an export the harness itself references.

## Destination

The spec: the schedule-schema skill's waves[*] section (the planner proposes; the schema is
not the planner's to evolve). It also needs a line in `templates/schedule.port.json`, and
one in the task-schema skill if a batch's items should carry the field too.

## Change

In `skills/artifacts/schedule-schema/SKILL.md`, add to the planned item's table:

> | `carries_over` | on a `port/unexport` item, the C test cases still referencing its export when its wave runs, in the plan's order, which the stage carries over and removes from `c/`: each `{path, tests}`, the test file and its test case names, with `harness` naming the functions of the test harness that reference the export, which the stage adapts; empty when none does; absent on every other objective |

After the release-point paragraph:

> The plan's order on disk decides which wave carries over a case several of them
> reference: the first by index that runs after the case's last chance to leave
> otherwise. A wave whose dependencies let it run earlier carries over what is still in
> `c/` when it runs, which its batch checks against the list.

Give the example `port/unexport` item in `templates/schedule.port.json` a
`carries_over: [{path: "tests/core.c", tests: ["test_core__open"]}]`.

## Validation

Run 11's `carryover11.py` computes the field as written above, per wave rather than per
item, from the final schedule. No case that stays references an export a
`port/unexport` removes. It was not written into `schedule.json`, whose schema forbids
the field.
