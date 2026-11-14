This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/goal-fitness-functions/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`cc84106`), this checkout.

- The change: `git -C /crustify-rs/crustify diff cc84106 bca1d35` (`GOALS.md` and its row of
  `EVOLUTION.md`), readable in `/crustify-rs/crustify-wt/goal-fitness-functions` (read-only).
- Its proposal: `/target/libgit2-crustify/crustify/evolution/goal-fitness-functions/PROPOSAL.md`,
  whose evidence cites the campaign's runs: the dashboard and review reports are in
  `/target/libgit2-crustify` at `crustify/devel` (`crustify/artifacts/dashboard.md`,
  `crustify/reviews/`, `crustify/evolution/`).

`GOALS.md` is what every other change is judged against, and changes only with the user's
approval: the user asked for this one (fitness functions under each goal, Fidelity renamed
Equivalence, Determinism and attribution split in two, Determinism meaning that one kind of
item takes one good pattern with justified departures), and reads your verdict before it
lands. Beyond the Review section's checks, judge:

1. each fitness line: does it measure its goal, can a campaign take it from what its runs
   produce anyway (name the artifact that holds it today, or say that none does), and can
   worse work improve it, in which case is the measure that catches that named beside it;
2. the two measures marked as not taken yet, and any other that no artifact holds and is
   not marked;
3. the two renamed or new goals against the rest of Crustify: whether any skill, agent,
   template or tool document names a goal by its old name or number, and whether
   Determinism as now stated stands in a standing tension with Least constraint (goal 10)
   or Idiomatic Rust (goal 3) that the text should weigh, as `GOALS.md`'s opening asks of
   goals in tension;
4. the cited evidence, against the campaign's artifacts: check the figures the proposal
   gives (6 reproducer-proven defects, three escapes, 83 of 170 reports under `misc`, the
   two seeded faults of `wave-232/batch-1`, `$2.90` over 108 items, `wave-186/batch-1`'s
   `$13.19`), and say which you could not confirm;
5. whether the change weakens a check, and whether its `EVOLUTION.md` row names a recurrence
   signal a campaign would observe;
6. that `python3 -I providers/claude/compose.py build` in a `git archive bca1d35` copy
   leaves `providers/claude/` unchanged.

Verdict, approved or rejected with the fixes required, in your final message, with what you
would word differently listed apart from what must change. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create.
