# Recomputing the dashboard's `Σ` rows in an integration merge

Status: `open`

## Signal

A rule worked out by hand that a skill should state. `versioning`'s Integration section
says the dashboard's rows conflict additively and both sides are kept, and `accounting`'s
Landing cells section says a conflicting rebase keeps every row and measures the landing's
cells again "where the gates its rebase reruns cover them". Neither says how an
integration merge settles the cells that are not rows: the `Σ` rows of the batch tables
and the `Coverage stats` table, which both sides rewrote from different trees. The task
message had to add "recompute the `Σ` rows from the merged rows", and the run still had to
decide three cases alone.

## Evidence

`foundational/libgit2/src/util/allocators/failalloc.c/integration/run-1`, merging
`crustify/devel` (`69ff068e8`, alloc.c wave 12 integrated) into the module branch
(`018f7da5e`, failalloc.c wave 176). The dashboard conflicted in six hunks: `Coverage stats`,
the `Σ` rows of `Symbols: Batches`, `Batches: tests` and `Batches: safety audit`, and of
`Batches — review: PoC + reports`. The run had to settle:

1. **Additive columns** (units, fields, `+LoC`, test counts and their pp, safety-audit
   deltas, report counts): Σ_merged = Σ_ours + Σ_theirs − Σ_base, which equals summing the
   merged rows when both sides only add rows, and needs no parsing of each table.
2. **Usage `Σ` cells** (`$`, tokens, `$/unit`): the module branch's rows carry no usage yet,
   so its `Σ` usage cells equal the base's, and the working branch's are the sum over every
   merged row that carries usage. The run kept the working branch's, which the assistant
   refreshes once it fills the integrated batches' usage. Recomputing `$/unit` over the
   merged units instead would divide the working branch's cost by units whose cost is not
   in it yet.
3. **Coverage cells**: a `Σ` coverage cell is "the pass's last landing", which across two
   branches is the later landing by time: here wave 176 batch 1 (03:37:12) over wave 12
   batch 2 (03:36:58) for `first`, wave 176 batch 3 for `review`. `Coverage stats` is a
   measurement of a tree neither side measured: the run summed its counts arithmetically
   (they match the merged tree's test binaries exactly: 719 `ub_safe`, 102 `ub_unsafe`, 495
   `equiv`, 799 + 7 unit) and kept the later landing's percentages, unmeasured, since no
   gate of the merge measures coverage and `accounting` forbids rerunning a suite only to
   record a figure.

## Destination

The spec: `accounting`, Landing cells, which the binder does not own.

## Change

In `accounting` §2.1, after "the others stand as measured on the batch's base.":

> An integration merge, and any merge of one branch into another, keeps every row both
> sides added, and settles the rest so: an additive `Σ` cell is the sum over the merged
> rows, ours plus theirs less the merge base's; a usage `Σ` cell, its per-unit cells
> included, sums only the rows that carry usage, so that it is filled again with theirs;
> a coverage `Σ` cell is the later landing's; and `Coverage stats` takes the merged
> suites' counts, with the later landing's percentages until a gate measures the merged
> tree, which the merge commit's message says.

## Validation

Applied by hand in this run's merge commit; the recomputed sums were checked against both
sides' rows (for example `Σ first` symbols 415 + 6 + 14 = 435, PoC total 80 + 2 + 3 = 85),
and the counts in `Coverage stats` against the merged tree's `cargo test` totals.

## Further evidence: the Overview's Rust LoC line

The Overview's `Rust LoC` line is not covered by the change above. The integration of
`deps/reftable/basics.c` (`d8ae0c8f6`) took it from the later landing, as it did the
`Coverage stats` percentages; that equalled the sum only because failalloc.c's landings had
left the line at the base's figure. Here both sides moved it, by the code lines their
landings added, and taking either would drop the other's.
`foundational/libgit2/src/util/allocators/win32_leakcheck.c/integration/run-1` summed it as
an additive cell (ours + theirs − base: `17768` + `18425` − `17732` = `18461` non-test,
`95814` + `96914` − `95599` = `97129` tests) and took `C LoC` (`86833`, the coverage
archive's lines, which no side changed) from the side that filled it. The change should
name it: "the Overview's `Rust LoC` figures are additive; its `C LoC` is the later
landing's".
