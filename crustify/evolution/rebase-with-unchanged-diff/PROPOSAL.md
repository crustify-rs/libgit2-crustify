# A rebased commit whose diff is unchanged needs no new review

Status: `open`: the change is made on crustify's
`libgit2-crustify/evolution/rebase-with-unchanged-diff` (`6051f8c`), awaiting its review. It changes
`skills/crustify/evolution/**`, a protected path, and a rule of the review itself, so it lands
only with the user's approval.

Goal: Cost: a review's cost buys nothing where it reads the diff another review approved.

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
> again, unless it is the reviewed commit rebased with its diff unchanged, as `git patch-id`
> shows, onto a tip whose changes since the review touch none of its files.

The second condition keeps the case where another landing changed the text around the
change, which an unchanged diff does not show.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a reviewed commit rebased with its diff unchanged, onto a tip whose changes since the review touch none of its files, lands without a new review | `libgit2-crustify`, `rebase-with-unchanged-diff` | crustify `6051f8c` | seven approved changes on one tip: each landing rebased the rest, which the rule sent back to review with identical diffs | a landed change that contradicts a rule another change landed between its review and its landing | watched; weakens a check: a rebased commit lands unread where it was reviewed again |

## Validation

None is possible by rerun: the rule changes when a review is asked for. The five landings of
2026-11-14 are the case, made under the user's approval.

## Review

None yet.
