This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-wrapper-without-alias/review-1/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, which is on `devel` (`9aa2ef1`); the
campaign's working branch of ffimap is `libgit2-crustify/devel` (`c3203c7`).

- The proposal, with its evidence, validation and drafted row: `/target/libgit2-crustify/crustify/evolution/ffimap-wrapper-without-alias/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffimap diff c3203c7 2a2aa1e`.

It changes the driver, `src/ffimap/driver/src/main.rs` (`count_sys_types` and a new
`anchors_fn`), `docs/schema.md`, the `cells` fixture and `tests/test_cells.py`. Read ffimap's
`docs/goals.md` first: G1, G7, G9, G10 and G13 above all. Run the suite on a `git archive 2a2aa1e`
copy, `PYTHONPATH=src python3 -m unittest discover -s tests` (145 tests; the driver builds on
its pinned nightly, which is installed), and show that the changed test fails on the old
driver. Judge above all: whether `anchors_fn` reads the same form as `api.py`'s `anchors`,
for every case the fixture or you can think of (a doc attribute in place of a doc comment,
several doc fragments, indentation, a raw identifier); whether it can count a function that is
no wrapper, a port's export above all; whether the row's `test` is right; and whether the
validation on real code is enough, the proposal saying what it did not run. The user chose to
fix this now and knows the anchor's form then lives in two parsers: do not reopen that.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, or contradict another rule. The rest is listed
apart, as not required. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`, removed from
outside it before you end.
