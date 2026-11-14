# Each goal names its fitness

Status: `open`: the change is made on Crustify's `libgit2-crustify/evolution/goal-fitness-functions`
(`bca1d35`, then `ac41d76` with review-1's fixes), at the user's direction on 2026-11-14;
review-1 rejected the first commit, and review-2 reads the second. `GOALS.md` changes
only with the user's approval: the user asked for this change, and reads the result before
it lands.

## Signal

The user's direction (`evolution` §2: the user's corrections), from the pilot of five `port`
modules: its results were reported goal by goal with no measure named for any goal, and the
figures that would have served were scattered or missing.

## Evidence

- Soundness: the pilot's reviews proved 6 memory-safety or soundness defects with
  reproducers (`opt.c`'s adopted handles, `wave-186/batch-3`, and dead borrow, `batch-7`;
  `api.c`'s freed reason, `wave-194/batch-5`, and freed table, `batch-6`; glibc-crustify's
  `stderr` and lock-less stream). One escaped the tier that first read the code: the dead
  borrow, which the module pass caught after the symbols review. Nothing records where a
  defect was caught: the count was made by hand from the runs' reports, and the dashboard's
  review tables file contracts findings under `+misc` and `+internal` (83 of the 219
  reports under `crustify/reviews/` sit in `misc`).
- Incremental migration and Verification: two link breaks at `api.c`'s switch escaped the
  batches' link checks and were caught by its symbols review and its module pass;
  `wave-186/batch-1` met two dependencies the plan had placed in its own wave, and waited
  (`$13.19` for two symbols); `wave-197/batch-0` reported clippy clean and landed a tree
  that failed it. None of the three is counted anywhere.
- Verification: `wave-232/batch-1` seeded two faults in `utf8.rs`'s class table. The whole
  public unit suite passed both; only the internal table test caught them, and the public
  suite is the one that outlives the internals.
- Performance: the pilot's cost per item was worked out by hand, from the CLI's totals of
  its 30 parent runs (`$312.73`) over the 108 items they landed (1, 1, 5, 28 and 73 by
  module): `$2.90`, a figure the dashboard does not hold, and one that leaves out what was
  redone: `alloc.c`'s and `failalloc.c`'s batches bound again, `opt.c` integrated twice
  (`6d823f4d7`, `c80c8a527`). The emitted Rust's own performance is measured nowhere.
- Consistency: three batches met the alias rule (`doc-alias-equal-to-the-name`) and took the
  same route, renaming the item and keeping the alias, with names each chose its own way;
  and `port-onto-an-existing-accessor`, `macro-incidental-double-evaluation` and
  `config-branches-of-one-result` each record a pattern a batch chose where the skills gave
  none. The goal named "Determinism and attribution" said only that facts can be
  reproduced and changes traced.

## Destination

`GOALS.md`, at Crustify's root, and its entry in `EVOLUTION.md`. Nothing else names a goal
by the two names that change.

## Change

Crustify `bca1d35` and `ac41d76`:

- a paragraph after the goals' opening: each goal names its fitness, read from what the
  runs produce anyway or from a rerun the reviews already make; a measure worse work could
  improve is paired with the one that catches it; a measure no artifact holds yet is marked
  *not taken yet*;
- fitness lines under each goal, each with its pair, and the measures not taken yet
  marked: the tier that caught a defect and the sites a review read, the inputs compared
  and the deliberate differences, the emitted Rust's performance, seeded faults, regions
  and states, the kinds of crossing, the order, and reshaped or twice-translated items;
- **Fidelity** is named **Equivalence**, its text unchanged;
- **Determinism and attribution**, goal 11, becomes **Attribution**, goal 12, with its text
  unchanged, and goal 11 is **Consistency**, the user's name for it: items that mean the
  same take the same pattern, a departure states its reason, and the goal weighs itself
  against Least constraint in its text. The goals are twelve;
- the order items land in is goal 8's second measure, stated as how that goal is met and
  Safety's and Performance's with it, rather than a goal of its own, as the user and the
  assistant settled.

Not in this change, and proposed to follow it: a per-module table of these measures in the
dashboard template; the `evolution` skill's Record reading a watched change against the
measure it named; and a ledger of the defects reproducers prove, with the tier that caught
each, which the soundness measure is computed from.

## Validation

No failure prompted it, so there is nothing to rerun. The measures are stated from figures
the pilot's runs produced, cited above; which artifact holds each, and which none holds, is
what the follow-up settles.

## Review

`review-1` (session `3bb4c02a-b4b2-424c-840a-6ce1c26a3c56`, Opus 5.5, `$1.88`, 7m25s), on `bca1d35`: **rejected**, its
direction approved; it weakens no check. Required, and made in `ac41d76`:

1. five measures worse work could improve named no pair (the unsafe surface, Idiomatic
   Rust, Maintainability, Least constraint, Incremental migration);
2. measures no artifact holds were not marked as not taken yet;
3. the new goal 11 stood in a standing tension with Least constraint and Idiomatic Rust
   that its text did not weigh, and judged a kind of item by its C shape, not its meaning;
4. the record's recurrence signal would have been met the day the change landed;
5. this proposal's evidence mixed two totals (83 of 170 reports), gave a cost per item the
   dashboard does not hold without saying how it was computed, counted two link breaks
   among the soundness escapes, and overstated wave 232's seeded faults. Corrected above.

Its optional wordings are taken too: unsafe code outside its regions is zero throughout,
the emitted Rust's performance names its pair, a rerun the reviews already make counts as
what the runs produce, and the record's row says which goal was goal 11.

`review-2`: pending, on `ac41d76`.
