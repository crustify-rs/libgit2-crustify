# A rebased commit whose diff is unchanged needs no new review

Status: `open`: the change is made on crustify's
`libgit2-crustify/evolution/rebase-with-unchanged-diff` (`e64f119`), approved by review-3. It changes
`skills/crustify/evolution/**`, a protected path, and a rule of the review itself, so it lands
only with the user's approval.

Goal: Performance, the harness: the cost per item, reviews included. A review buys nothing
where it reads the diff another review approved against the same text.

## Signal

A cost outlier, and a rule an agent works around. The `evolution` skill's Review section has
"a commit added after the verdict, or one a rebase or cherry-pick makes" reviewed again. A
change lands only by a fast-forward, so each landing moves the tip and every other approved
change must be rebased, which that sentence sends back to review though nothing in it changed.

## Evidence

On 2026-11-14 seven changes were approved on one tip of `libgit2-crustify/devel`. Landing the
first rebased the rest:

- `type-erased-field-tagged-by-a-sibling` and `export-signature-check` were approved as
  `7b738b9` and `27f80ba`, restacked as `144259c` and `c59838e`, and again, with three more
  approved commits (`2c21af3`, `180fd67`, `c461f16`), as `3251f5e` to `80325f9` once the gates
  rule landed. `git patch-id` gave each rebased commit the id of the commit its review read.
- A first review of a one-sentence change cost `$1.3` to `$2.0` that day (five cost `$8.36`),
  so reviewing the five again would have cost about what their first reviews did, to read the
  same diffs.
- The user approved landing them as they were, a departure from the rule, asked for each time.

## Destination

The spec: the `evolution` skill's Review section.

## Change

> ... a commit added after the verdict, or one a rebase or cherry-pick makes, is reviewed
> again. A rebase of a change to the text of skills, agents or templates alone, no script or
> program among its files, is the exception, where each file the commit changes is the same
> in the rebased commit and its parent as in the reviewed commit and its parent: the review
> then read the same diff against the same text.

It covers text alone: a program's change, a tool's or a script's beside a skill, is validated by its suite on the combined tree,
which two changes to different files can still break. A cherry-pick is not covered.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a rebase of a reviewed change to the text of skills, agents or templates alone lands without a new review where each file it changes is the same in the rebased commit and its parent as in the reviewed commit and its parent | `libgit2-crustify`, `rebase-with-unchanged-diff` | crustify `e64f119` | seven approved changes on one tip: each landing rebased the rest, which the rule sent back to review with identical diffs | a landed change that contradicts a rule another change landed between its review and its landing | watched; weakens a check: a rebased text change lands unread where it was reviewed again, so two changes approved on one tip that contradict each other across files are no longer caught by the second's review |

## Validation

None is possible by rerun: the rule changes when a review is asked for. The five landings of
2026-11-14 are the case, made under the user's approval.

## Review

`review-1` (Opus 5.5, `$1.11`), on `6051f8c`: **rejected**, three fixes required, all made as
`e64f119`: `git patch-id` ignores whitespace, so the clause no longer names it and asks for the
same files; "changes since the review" had no clear start, so the clause compares each file in
the commit and its parent before and after the rebase; and the clause covered a tool's code,
which the evidence does not reach, so it is narrowed to the spec. It recomputed the patch-ids
of the five landings, which match, and marked the change as weakening a check. Listed apart,
not required: the saving is a narrow review's cost, about `$0.3`, not a first review's;
whether the user's approval of a protected change carries over its rebase, and who records
the check, are not said.

`review-2` (Opus 5.5, `$0.43`), on `602c028`: **rejected**, one fix: "the spec" also holds
programs, the providers' and the scripts beside skills, which review-1's third fix meant to
leave out. Made as `e64f119`: the text of skills, agents or templates alone. The first two fixes
held, and the five landings pass the clause.

`review-3` (Opus 5.5, `$0.23`), on `e64f119`: **approved**: the fix is made as review-2 asked and nothing
else changed. It is still marked as weakening a check.
