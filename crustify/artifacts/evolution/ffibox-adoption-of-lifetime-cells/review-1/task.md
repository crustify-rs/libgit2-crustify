This run is not a review batch: it reviews one change to ffibox, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffibox-adoption-of-lifetime-cells/review-1/task.json`.
You run under ffibox's `libgit2-crustify/devel` (`9c22a73`), this checkout, `/crustify-rs/ffibox`.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/ffibox-adoption-of-lifetime-cells/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffibox diff 9c22a73 45c3ed1`.

It changes `src/traits.rs` (`CCell::ref_from_ptr`'s `# Safety`), `src/macros.rs` (`define_ctype!`'s
`phantom` paragraph) and `docs/borrows.md`: documentation alone, no signature. The proposal
also names the owners' adoptions (`CBox::from_c`, `from_raw`, `CSlice::from_raw_parts`); the
change leaves them, since their contracts already ask for a live `T`, the Rust type. Judge
whether that holds.

Read ffibox's `docs/goals.md` first: the change is judged against it, G9, G10 and G12 above
all. Check that the rule agrees with Crustify's `soundness` skill
(`/crustify-rs/crustify/skills/rust/soundness/SKILL.md`). `cargo +1.100.0 doc --no-deps` and
`cargo +1.100.0 test --doc` pass on the stack's tip (58 doctests); run them again on this
commit only if you doubt it, in a `git archive` copy under
`/target/libgit2-crustify/crustify/tmp/`, removed afterwards.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Where a required fix can be made by removing text rather than adding
it, say so. Verdict, approved or rejected with the fixes required, briefly, in your final
message.
