This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/gates-slow-list/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`4a0e778`), this checkout.

- The proposal, with its evidence and drafted row:
  `/target/libgit2-crustify/crustify/evolution/gates-slow-list/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 4a0e778 4557a8f`, readable in
  `/crustify-rs/crustify-wt/gates-slow-list` (read-only).
- `end-gates`, beside the `testing-gates` skill, whose `--help` documents the skip file the
  rule calls the slow list. The change does not touch it.

The rule is the user's, settled with them on 2026-11-14: a wave's and a module's end run
every stage over the workspace less a per-stage slow list; a full run without skips runs
periodically, by default every fifth module, started by the assistant with no agent, and at
each ship point; the slow list's changes go to the user at ship points. Do not reopen those
decisions. The change touches a protected path: the user reads your verdict before it lands.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, let a check be
weakened unseen, or contradict another rule. A wording you would sharpen, an edge case no
run is likely to meet, and a gap whose fix would add a rule or a definition are listed
apart, as not required, and do not make the verdict a rejection. Where a required fix can be
made by removing text rather than adding it, say so.

Check that the proposal's figures match
`/target/libgit2-crustify/crustify/tmp/assistant/gates-timing/` (compute each); that the
change says what the user settled and no more; that no other skill, agent or template still
states the old depths of a wave's or a module's end; that the row marks every way the change
weakens a check; and that `python3 -I providers/claude/compose.py build` in a
`git archive 4557a8f` copy leaves `providers/claude/` unchanged. No gate is to be run: the
proposal says the rule has no validation run yet, and judge whether that is acceptable for
what it changes. Verdict, approved or rejected with the fixes required, briefly, in your
final message. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, and remove any
build directory you create from outside it, not while it is your working directory.
