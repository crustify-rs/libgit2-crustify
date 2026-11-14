This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-driver-builds-under-a-workspace/review-1/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, on `devel` (`a0797e6`), which is also
the campaign's working branch of ffimap.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/ffimap-driver-builds-under-a-workspace/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffimap diff a0797e6 5ac9ec5`.

It changes the driver's manifest, `measure.py` (a new `driver_prerequisites`), the skip
condition of four test files and adds one test. Judge above all whether a host without the
toolchain still skips cleanly, whether a broken driver build now fails the suite loudly, and
whether the `[workspace]` table changes anything for the installed package (the manifest is
package data, and its `Cargo.lock`).

Read ffimap's `docs/goals.md` first. Run the suite on a `git archive 5ac9ec5` copy under `/tmp`,
and also on a copy under `/target/libgit2-crustify/crustify/tmp/`, which sits below a Cargo
workspace. Do not build or run anything in `/crustify-rs/ffimap` itself, the shared install,
which a long scan is using: read it only.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, or contradict another rule. The rest is listed
apart, as not required. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Remove your scratch, from outside it, before you end.
