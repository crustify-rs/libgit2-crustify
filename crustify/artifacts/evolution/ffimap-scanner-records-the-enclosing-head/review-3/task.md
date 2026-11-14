This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-scanner-records-the-enclosing-head/review-3/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, on `devel` (`a0797e6`); the campaign's
working branch of ffimap is `libgit2-crustify/devel` (`b5c043c`).

This is a third, narrow review. Review-2 found `1075740`'s code right and able to land
unchanged, with one fix to the proposal's text, which is made (`/target/libgit2-crustify/crustify/evolution/ffimap-scanner-records-the-enclosing-head/PROPOSAL.md`, whose Review section
holds its verdict). The working branch has since taken `b5c043c`, which adds two classes to
`tests/test_scan.py`, the file this change adds one to. `50a800e` is `1075740` rebased onto it,
the conflict in that file settled by keeping all three classes.

Check only that `50a800e` is `1075740` but for that file's merge
(`git -C /crustify-rs/ffimap range-diff a0797e6..1075740 b5c043c..50a800e`), that the proposal no
longer claims what `--no-optional-locks` does not do, and that the suite passes on a
`git archive 50a800e` copy under `/tmp` (155 tests, none skipped). Build or run nothing in
`/crustify-rs/ffimap`. Verdict, approved or rejected, in a few lines, in your final message.
Remove your scratch: `cd /` first, then `rm -rf` it.
