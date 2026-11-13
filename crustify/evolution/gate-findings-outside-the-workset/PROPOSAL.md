# A gate's true positive is the batch's finding, wherever it lies

Status: `open`: made as crustify `1423c7f` on `libgit2-crustify/evolution/gate-findings-outside-the-workset`,
awaiting its review.

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

## Validation

The run above, which followed the rule before it was written: the finding is filed with
its reproducer, the fix makes the reproducer exit 0 five runs of five, and the regression
test passes under TSan with the fix and fails with the race without it.

## Review

Pending.
