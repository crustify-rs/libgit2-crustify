This run is not a review batch: it reviews one change to Crustify again, after review-1
rejected its first commit, as the `evolution` skill's Review section describes, and changes,
commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/goal-fitness-functions/review-2/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`cc84106`), this checkout.

- Review-1's report, with its five required fixes and its optional wordings: the longest
  `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/goal-fitness-functions/review-1/stream.jsonl`.
  The proposal's Review and Change sections record it:
  `/target/libgit2-crustify/crustify/evolution/goal-fitness-functions/PROPOSAL.md`.
- The fix: `git -C /crustify-rs/crustify diff bca1d35 ac41d76`; the whole change:
  `git -C /crustify-rs/crustify diff cc84106 ac41d76` (`GOALS.md` and its row of
  `EVOLUTION.md`), readable in `/crustify-rs/crustify-wt/goal-fitness-functions` (read-only).

`GOALS.md` changes only with the user's approval; the user asked for this change and reads
your verdict before it lands. Since review-1, the user chose the name **Consistency** for
goal 11, and settled with the assistant that the order items land in is no goal of its own
but a measure under goal 8, said to serve Safety and Performance too: both are in the fix.

Check that each required fix of review-1 is resolved (cite the line), that the fix
introduces nothing that contradicts the rest of `GOALS.md` or of Crustify, that every
measure worse work could improve now names its pair and every measure no artifact holds is
marked, that goal 11's text weighs its tension with Least constraint and judges an item by
its meaning, that the proposal's corrected evidence holds against the campaign's artifacts
(`/target/libgit2-crustify` at `crustify/devel`), that
`python3 -I providers/claude/compose.py build` in a `git archive ac41d76` copy leaves
`providers/claude/` unchanged, and whether anything weakens a check. Read only as deeply as
the fix reaches; what review-1 approved need not be re-derived. Verdict, approved or
rejected with the fixes required, briefly, in your final message, with what you would word
differently listed apart. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, and
remove any build directory you create.
