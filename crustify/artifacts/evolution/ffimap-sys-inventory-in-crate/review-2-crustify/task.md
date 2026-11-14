This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-sys-inventory-in-crate/review-2-crustify/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`a49c1b1`), this checkout.

This is a second, narrow review. Review-1 rejected `3dcc464` with one fix: the paragraph named
every in-crate raw-bindings module, hand-written ones in an audit included. `9885b23` limits it to
bindings laid out and generated as `bindgen` describes, adds that a named module leaves
every other counter, and says a module holding hand-written code is left unnamed. Review-1's
verdict is at the end of `/target/libgit2-crustify/crustify/evolution/ffimap-sys-inventory-in-crate/PROPOSAL.md`. The change: `git -C /crustify-rs/crustify show 9885b23`; what
review-1 read: `show 3dcc464`.

Check only that the fix is made as review-1 asked and that nothing else changed. Do not
repeat review-1's other checks, and do not raise again what it listed as not required. The
paragraph is on a protected path: the user reads your verdict. Verdict, approved or rejected,
in a few lines, in your final message. Create no scratch.
