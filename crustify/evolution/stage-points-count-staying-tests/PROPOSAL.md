# The C tests that stay, in a `port/unexport`'s release point

Status: `landed`, as crustify `737aebc` on `devel`, with the user's answer to its open question: a test case the configured build compiles empty is out of scope

## Signal

A rule an agent has to work out by hand, and a gap between two skills, met again: planner
runs 6, 7, 9 and 10 of `crustify-rs/libgit2-crustify` each re-derived by hand how the C
tests hold back a ported function's export, and run 6 got it wrong (the plan correction
`crustify/reviews/planner/modules/test-only-referrers-unexported/`: 176 exports only
remaining tests reached were planned with no `port/unexport`, so those tests would have
failed to link).

## Evidence

- The scheduling skill's Later stages section, 5.2, times a `port/unexport` "after its
  `unreferenced_after`: the integration of its last referrers, the remaining C that calls
  it", and the wavefront skill's Scheduling › Later stages section has a stage wait for
  "the last waves of the modules its point's `closure.modules` names". Wavefront's
  closure leaves the tests out (`docs/batching.md`, Release points: "a test the closure
  leaves out is no user"), so its `unreferenced_after` never names a test.
- The conventions' Exports section exports a function that "a C test or executable that
  stays" references, and the porting workflow's Tests follow their modules section has a
  test that is not tied to the module, or that is tied to it but stays, keep running after
  the module's integration: such a test still calls the export, so the export's
  `port/unexport` waits for it, unless it is tied to the module through its public API
  alone, which holds nothing back.
- Neither the scheduling skill nor the wavefront skill says so, nor how a test's
  departure is timed: run 10 had to derive it
  (`crustify/artifacts/planner/run-10/compose/stages10.py`), including two cases the
  skills leave open: a test tied to a module through its public API alone that references
  none of that module's exports (here 76 test cases, 42 of them
  with a body the configured build compiles empty), and a test that leaves with an export another staying test holds
  (verdicts that depend on each other, settled by iteration).

## Destination

The spec: the scheduling skill's Later stages section, 5.2 (mine to evolve as planner),
with a sentence in the wavefront skill's Scheduling › Later stages section.

## Change

In `skills/planning/scheduling/SKILL.md`, 5.2, after the first paragraph:

> Its referrers include the target's C tests that stay after its module's integration,
> which wavefront's release points leave out, as the [`porting-workflow`](../../workflows/porting-workflow/SKILL.md)'s
> Tests follow their modules section times their departure: a test not tied to the
> module, and one tied to it through its internals, holds the export until it leaves; one
> tied to it through its public API alone holds nothing back. A test that would leave with
> an export holds no other export past that one's `port/unexport`; one with no point to
> leave at, such as the harness every test links, keeps the exports it reaches for good.

In `skills/tools/wavefront/SKILL.md`, Scheduling › Later stages, after "with none left,
the stage is not planned": "A `port/unexport` also waits for the C tests that stay, which
the closure leaves out, as the scheduling skill's Later stages section describes."

Open question for the user: whether a test case that the configured build compiles empty
(its body under a configuration the target is built without, such as `GIT_WIN32`) is out
of scope, as a configured-out TU is, rather than a test that never leaves; run 10 counts
it as staying, which keeps the exports its file's fixtures and helpers reach.

## Validation

Run 10's composition implements the rule as written above
(`stages10.py`, `test_referrers10.py`: 0 violations over 1472 `port/unexport` items). No
other run has been made under it.
