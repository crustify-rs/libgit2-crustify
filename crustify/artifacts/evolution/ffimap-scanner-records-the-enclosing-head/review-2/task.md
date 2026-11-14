This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-scanner-records-the-enclosing-head/review-2/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, on `devel` (`a0797e6`).

This is a second review. Review-1 rejected `a73eb55` with two fixes; its verdict is in the
Review section of `/target/libgit2-crustify/crustify/evolution/ffimap-scanner-records-the-enclosing-head/PROPOSAL.md`. `1075740` replaces it (`git -C /crustify-rs/ffimap diff a0797e6 1075740`; from
what review-1 read, `diff a73eb55 1075740`):

- the dirty form is `<commit>+dirty.<12 hex of sha256 of git diff HEAD --binary>`, asked with
  `--no-optional-locks`;
- `measure.names_one_scanner` and `diff.diff`: `ffimap diff` reports the scanners unless they
  agree and each is a commit, bare or with that hash;
- `docs/schema.md` § 3.2, `docs/analysis.md`, `README.md` and the `scanner()` docstring say
  so; the tests moved above the `__main__` guard, with new ones in `test_scan.py` and
  `test_diff.py`;
- the proposal's Destination, Change, Validation and row are its own, and the row is
  `watched`.

Check that fix 1 is made in each of its four parts as review-1 asked, with your two-edits
probe now giving two different strings; that `ffimap diff`'s new rule breaks no use the
skills make of it (Crustify's `ffimap` tool skill, `/crustify-rs/crustify/skills/tools/ffimap/SKILL.md`,
compares scans of one install, which records a commit); and that nothing else changed. Run
the suite on a `git archive 1075740` copy under `/tmp`; build or run nothing in
`/crustify-rs/ffimap`. Do not raise again what review-1 listed as not required. Verdict,
approved or rejected with the fixes required, briefly, in your final message. Remove your
scratch, from outside it, before you end.
