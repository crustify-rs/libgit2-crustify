This run is not a review batch: it reviews one change to Crustify a fourth time, after
review-3 rejected the commit it read on one fix, as the `evolution` skill's Review section
describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/evolution-record-and-lessons/review-4/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- Review-3's report, with its one required fix (F1): the longest `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/evolution-record-and-lessons/review-3/stream.jsonl`.
  The proposal: `/target/libgit2-crustify/crustify/evolution/evolution-record-and-lessons/PROPOSAL.md`.
- The fix, all this review reads: `git -C /crustify-rs/crustify diff 479c29f 122f1f2`, two
  template lines and one bullet of the `evolution` skill's Proposals section, readable in
  `/crustify-rs/crustify-wt/evolution-record-and-lessons` (read-only).

Review-3 approved everything else at `479c29f`: do not derive it again. Check that F1 is
resolved, that the reworded bullet changes no rule, that the diff touches nothing else, and
that `python3 -I providers/claude/compose.py build` in a `git archive 122f1f2` copy leaves
`providers/claude/` unchanged. The user's instruction of review-3 stands: require a fix only
for what would lead a run wrong, leave a signal unfiled, let a check be weakened unseen, or
contradict another rule, and list anything else apart, as not required. Verdict, approved or
rejected, in a few lines. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, and
remove any build directory you create from outside it, not while it is your working
directory.
