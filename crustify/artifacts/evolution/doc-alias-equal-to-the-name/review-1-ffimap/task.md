This run is not a review batch: it reviews one change to ffimap, as Crustify's `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/doc-alias-equal-to-the-name/review-1-ffimap/task.json`.
You run under ffimap's working branch of the campaign, this checkout, `/crustify-rs/ffimap`.

- The proposal, with its evidence, the user's decision and the drafted row:
  `/target/libgit2-crustify/crustify/evolution/doc-alias-equal-to-the-name/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffimap diff 9aa2ef1 c3203c7`.

It changes `src/ffimap/api.py` (one condition), `docs/api.md`, the `api` fixture and its test.
Read ffimap's `docs/goals.md` first. Run the suite on a `git archive c3203c7` copy,
`PYTHONPATH=src python3 -m unittest discover -s tests` (145 tests; the fixture tests need the
driver's pinned nightly, which is installed), and show that the fixture's `same_name` fails
the old check. Its other half is one sentence of Crustify's `conventions`, `a5ae2ea`, reviewed
apart. ffimap has no working branch of the campaign yet: `libgit2-crustify/devel` is created
at `devel` (`9aa2ef1`), and this checkout is on `devel` at that commit.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Do not reopen the user's decision of 2026-11-14, which the proposal's
Decision section records: the alias is dropped where it would equal the item's name, and
nowhere else. Verdict, approved or rejected with the fixes required, briefly, in your final
message. Keep any scratch under `/target/libgit2-crustify/crustify/tmp/`, and remove it from
outside it before you end.
