This run is not a review batch: it reviews one change to Crustify again, after review-1
rejected the commit it read, as the `evolution` skill's Review section describes, and
changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/evolution-record-and-lessons/review-2/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- Review-1's report, with its nine required fixes: the longest `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/evolution-record-and-lessons/review-1/stream.jsonl`.
  The proposal records it, with the user's two choices since, its corrected evidence, its
  drafted record row and its validation:
  `/target/libgit2-crustify/crustify/evolution/evolution-record-and-lessons/PROPOSAL.md`.
- The fix: `git -C /crustify-rs/crustify diff 74f960c d3eace2`; the whole change:
  `git -C /crustify-rs/crustify diff 5933a5d d3eace2`, readable in
  `/crustify-rs/crustify-wt/evolution-record-and-lessons` (read-only). It adds a skill,
  `skills/crustify/evolution/record/SKILL.md`.
- The dry index the validation cites: `/target/libgit2-crustify/crustify/evolution/INDEX.md`.

The change touches protected paths and changes what checks the rest: the user reads your
verdict before it lands. Since review-1 the user chose two things, which the fix follows:
the index of open signals is a file of the campaign's repository, not rows of
`EVOLUTION.md`; and the watched rows are read at a period the campaign adjusts, by default
every fifth module, rather than at each module's end, which the drafted row marks as
weakening a check.

Check that each of review-1's nine fixes is resolved under those choices (cite the line);
that each rule can now be followed as written by the run it falls to, a stage agent, a
reviewer and the assistant; that the split is clean (every reference to the old sections
resolves, the part is in the plugin's manifest, both skills are within `authoring`'s size);
that the protected paths are complete for what judges the work and matchable against a
diff, and the blocking-defect exception cannot be used to loosen a check; that no text of
Crustify still describes the old protocol; the proposal's recounted evidence against the
artifacts; the drafted row's recurrence signal; that
`python3 -I providers/claude/compose.py build` in a `git archive d3eace2` copy leaves
`providers/claude/` unchanged; and what the change weakens. Verdict, approved or rejected
with the fixes required, in your final message, with what you would word differently
listed apart. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, and remove any
build directory you create from outside it.
