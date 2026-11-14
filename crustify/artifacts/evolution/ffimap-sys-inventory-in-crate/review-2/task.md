This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-sys-inventory-in-crate/review-2/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, which is on `devel` (`9aa2ef1`); the
campaign's working branch of ffimap is `libgit2-crustify/devel` (`c3203c7`).

- The proposal, with its evidence, validation and drafted row: `/target/libgit2-crustify/crustify/evolution/ffimap-sys-inventory-in-crate/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffimap diff c3203c7 a0797e6`.

This is a second review, and not a narrow one: review-1 rejected `8cd7a26` with four fixes, and
answering its second, the bindings' own code counted as the crate's, widened the change. Its
verdict is in the proposal's Review section. `a0797e6` replaces `8cd7a26`
(`git -C /crustify-rs/ffimap diff 8cd7a26 a0797e6`):

- a named module's definitions now leave every walk but the inventory's: `in_sys_module` is
  tested at the head of nine walks over `hir_crate_items(()).definitions()` and four over
  `hir_body_owners()`, `named_defs`' two among them, and resolves the named modules once;
- the fixture module is `src/raw.rs`, under the feature `raw`, written as bindgen writes
  bindings, with derives, a `Default` impl and an `unsafe` block, and has two tests;
- `docs/analysis.md` says what a named and an unnamed module are;
- the proposal's evidence, validation and row are corrected, and the `ffimap` skill's
  paragraph is crustify `3dcc464`, reviewed apart.

Read ffimap's `docs/goals.md` first. Run the suite on a `git archive a0797e6` copy that is NOT
under a Cargo workspace (under `/tmp`): 147 tests, none skipped. Show the two new tests fail
on `8cd7a26`'s driver. Judge above all:
- whether any walk over the crate is left unfiltered, or any counter reaches a named module's
  items another way (through a type's impls, a trait's implementors, a caller's body that
  names a binding, which must still count as the crate's use of it);
- whether filtering can drop something that is the crate's: an item the crate writes inside
  the named module, a re-export, a wrapper whose body sits beside the bindings;
- whether a named crate, and a scan that names no module, are unchanged: the 21 older driver
  tests, and glibc-crustify with `--ffibox --sys-crates libc_sys` against the old driver;
- whether the numbers on libgit2-crustify are believable against 2026-11-10's
  (`/target/libgit2-crustify/crustify/artifacts/ffimap.json`): the scans are
  `/target/libgit2-crustify/crustify/tmp/assistant/ffimap-scan/scan.json` (old driver) and
  `scan-new2.json` (this commit). You need not rescan libgit2;
- whether the row and the proposal now say what the scans show.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, or contradict another rule. The rest is listed
apart, as not required. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Remove your scratch before you end.
