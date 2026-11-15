This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/std-symbol-alias-stand-ins/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`57005de`), which is checked out in this
worktree, `/crustify-rs/crustify.libgit2-crustify`; `/crustify-rs/crustify` is the component's own checkout, on `devel`.

- The proposal, with its evidence and drafted row, is on the campaign's wave branch, not yet
  on its working branch: read it with
  `git -C /target/libgit2-crustify show refs/heads/crustify/waves/wave-226:crustify/evolution/std-symbol-alias-stand-ins/PROPOSAL.md`
  (and `git -C /target/libgit2-crustify ls-tree -r refs/heads/crustify/waves/wave-226 crustify/evolution/std-symbol-alias-stand-ins` for its other files).
- The change, one commit: `git -C /crustify-rs/crustify.libgit2-crustify diff 57005de 06c62a2`, in `porting-symbols`' Macros
  section, `skills/rust/porting/symbols/SKILL.md`, which is no protected path.
- The run that made it, `wave-226/batch-2/run-1`, ported libgit2's `git_mutex`, `git_cond`,
  `git_rwlock` and `git_thread_*` macros as types over the standard library's, landed as
  `8aa231a69` on that wave branch (`src/libgit2/src/util/thread.rs`).

Check that the rule agrees with the conventions' anchors and with `porting-symbols`' and
`dependencies`' routes, and that it states nothing in the target's names. Say whether it
weakens a check. No gate or rerun is to be run.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. The rest is listed apart, as not required. Verdict, approved or rejected with
the fixes required, briefly, in your final message. Create no scratch.
