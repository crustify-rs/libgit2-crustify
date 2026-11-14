# Each goal names its fitness

Status: `open`: the change is made on Crustify's `libgit2-crustify/evolution/goal-fitness-functions`
(`bca1d35`), at the user's direction on 2026-11-14, and awaits its review. `GOALS.md` changes
only with the user's approval: the user asked for this change, and reads the result before
it lands.

## Signal

The user's direction (`evolution` §2: the user's corrections), from the pilot of five `port`
modules: its results were reported goal by goal with no measure named for any goal, and the
figures that would have served were scattered or missing.

## Evidence

- Soundness: the pilot's reviews proved 6 memory-safety or soundness defects with
  reproducers (`opt.c`'s adopted handles and dead borrow, `api.c`'s freed reason and freed
  table, glibc-crustify's `stderr` and lock-less stream), and three defects escaped the tier
  that first read the code (two link breaks at `api.c`'s switch, the dead borrow after
  `opt.c`'s symbols review). Nothing records where a defect was caught: the count was made
  by hand from the runs' reports, and the dashboard's review tables file contracts findings
  under `+misc` and `+internal` (83 of 170 reports sit in `misc`).
- Verification: `wave-232/batch-1` seeded two faults in `utf8.rs`'s class table; the whole
  public unit suite passed both at full line coverage. Coverage alone said the code was
  checked.
- Performance: cost per item was reported (`$2.90` over 108 items) without what was redone:
  `alloc.c`'s and `failalloc.c`'s batches run twice, `opt.c`'s integration merged twice. The
  emitted Rust's own performance is measured nowhere.
- Determinism: three batches renamed items around the alias rule, each its own way
  (`doc-alias-equal-to-the-name`), and `port-onto-an-existing-accessor`,
  `macro-incidental-double-evaluation` and `config-branches-of-one-result` each record a
  pattern a batch chose where the skills gave none. The goal named "Determinism and
  attribution" said only that facts can be reproduced and changes traced.
- Incremental migration: `wave-186/batch-1` met two dependencies the plan had placed in its
  own wave, and waited (`$13.19` for two items); `wave-197/batch-0` landed a tree that
  failed clippy. Neither is counted anywhere.

## Destination

`GOALS.md`, at Crustify's root, and its entry in `EVOLUTION.md`. Nothing else names a goal
by the two names that change.

## Change

Crustify `bca1d35`:

- a paragraph after the goals' opening: each goal names its fitness, read from what the
  runs produce anyway; a measure worse work could improve is paired with the one that
  catches it; a measure no campaign takes yet says so;
- one or two fitness lines under each goal;
- **Fidelity** is named **Equivalence**, its text unchanged;
- **Determinism and attribution** becomes two goals: **Determinism**, that one kind of item
  takes one pattern and a departure states its reason, and **Attribution**, with the former
  goal's text. The goals are twelve.

Not in this change, and proposed to follow it: a per-module table of these measures in the
dashboard template; the `evolution` skill's Record reading a watched change against the
measure it named; and a ledger of the defects reproducers prove, with the tier that caught
each, which the soundness measure is computed from.

A goal on dependency order, which the user asked about, is not added: the order is how
goal 8 is met, and `scheduling` already states it. Its measure is goal 8's last clause,
the batches that met a dependency not yet landed.

## Validation

No failure prompted it, so there is nothing to rerun. The measures are stated from figures
the pilot's runs produced, cited above; which artifact holds each, and which none holds, is
what the follow-up settles.

## Review

None yet.
