# A gate's true positive is the batch's finding, wherever it lies

Status: `open`: the change is crustify `ecfa4fb` on
`libgit2-crustify/evolution/gate-findings-outside-the-workset`, approved by its review. It touches a protected path, so it
lands only with the user's approval.

Goal: Safety, its soundness measure, and Verification: a defect a gate proves is filed among
the reports.

## Signal

A rule an agent had to work out by hand that a skill should state. `reviewing`'s Gates
section says only that "without `fix`, each true positive a gate finds is filed as a UB
report"; its Worklists section has a review "report only on" its workset. A gate runs over
the whole workspace, so a true positive it finds may lie outside the workset, and the two
rules then leave it to no one: neither says whether a review under `fix` files it, patches
it, or leaves it.

## Evidence

`wave-229/batch-1/run-1`, the module pass of `src/util/utf8.c`: its module's end gates, TSan
over the workspace on the wave's tip `680ac73bf`, failed the `lib` binary with a data race in
`src/libgit2/filter.c`, far from the module. It reproduced alone, from safe code
(`crustify/reviews/modules/libgit2/src/util/utf8.c/wave-229/batch-1/ub_safe/first-filter-loads-race-on-builtin-filter-flags`,
exit 66 five runs of five), and `filter.rs`'s module docs had named it as open since the
tree was homed. An earlier review of the same race (`legacy-wave-49/batch-1`'s
`misc/matched-path-workdir-tests-race-the-first-filter-load`) had filed it as a test defect
of its fixture, and a later batch (`wave-332`) serialized another fixture around it: each
left the defect to the module that owned it, which no batch revisited. The pass filed it,
patched it (`runtime_pinned` initialises the built-in filters) and turned the reproducer
into a regression test, reasoning that the gate was its own.

## Destination

The spec: `reviewing`'s Gates section, which the reviewer's role evolves.

## Change

crustify `1423c7f`: the section's sentence becomes "Each true positive a gate finds is a
finding of the batch, filed as a UB report and, under `fix`, patched, whether or not the
workset holds the code it reaches: the gate is the batch's, and a report it leaves to no one
is lost."; and its `EVOLUTION.md` entry.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`1423c7f`, which also edited `EVOLUTION.md`; on 2026-11-14 it was rebased onto the working
branch as `9f42026`, without that edit, since a change's row is drafted in its proposal.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a gate's true positive in the reviewed codebase is the batch's finding, patched under `fix`, wherever it lies | `libgit2-crustify`, `gate-findings-outside-the-workset` | crustify `ecfa4fb` | the reviewing skill filed a gate's true positive only without `fix`, and its workset rule left one outside the workset to no one | a review whose gate found a true positive it neither filed nor patched, or left to a later batch, as outside its workset | watched |

## Validation

The run above, which followed the rule before it was written: the finding is filed with
its reproducer, the fix makes the reproducer exit 0 five runs of five, and the regression
test passes under TSan with the fix and fails with the race without it.

## Review

`review-1` (Opus 5.5, `$1.08`), on `9f42026`: **rejected**, one fix required: the sentence had a
review under `fix` patch a gate's report inside a tool's code, which the Reports section makes a
proposal. Fixed as `ecfa4fb`: "each true positive a gate finds in the reviewed codebase". Its
evidence, placement, validation and row held, and it weakens no check. Listed apart, not
required: the BSan sentence after it is now a special case; "filed as a UB report" does not
fit a plain test's or a lint's failure; without `fix` a known defect is filed again at each
end; a batch without `ub` is still told to patch.

`review-2` (Opus 5.5, `$0.20`), on `ecfa4fb`: **approved**: the fix is made as review-1 asked and
nothing else changed.
