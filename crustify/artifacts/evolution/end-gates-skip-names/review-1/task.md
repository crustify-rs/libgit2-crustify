This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/end-gates-skip-names/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`1b0d20d`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/end-gates-skip-names/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 1b0d20d 01064ed`.

It changes `end-gates`, the script beside the `testing-gates` skill, and its tests: code, and
a check, on a protected path, so the user reads your verdict before it lands. Run
`python3 -m unittest test_end_gates` in a `git archive 01064ed` copy of
`skills/verification/testing/gates` (10 tests, about half a minute), show that the two new
tests fail on the old script, and read the code: `check_skips`, `workspace_counts`,
`skip_args` and `per_binary`. Judge above all whether `--exact` can make a run skip more or
fewer tests than its skip file names, Miri's `compile_fail` skip included, and whether the
row's `test` is right, its tests checking all of the recurrence signal. The campaign's slow
list is `/target/libgit2-crustify/crustify/scripts/slow-list.txt`. No campaign gate is to be
run.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, or contradict another rule. The rest is listed
apart, as not required. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, removed from
outside it before you end.
