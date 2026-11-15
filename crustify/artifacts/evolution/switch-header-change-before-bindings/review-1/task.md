This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/switch-header-change-before-bindings/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`57005de`), which is checked out in this
worktree, `/crustify-rs/crustify.libgit2-crustify`; `/crustify-rs/crustify` is the component's own checkout, on `devel`.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/switch-header-change-before-bindings/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify.libgit2-crustify diff 57005de 5319438`, in `porting-workflow`'s Builds
  section, `skills/workflows/porting-workflow/SKILL.md`, which is no protected path.
- The run that made it carried out `src/util/integer.h`'s switch from C
  (`crustify/artifacts/modules/libgit2/src/util/integer.h/integration/run-1`, the
  campaign's `b95941fa6` and the fork's `d9a1d699b`).

Check that the new order agrees with `bindgen`'s Generation section, `versioning`'s
Integration section and `testing-gates`' paragraph on a module's switch, and that a switch
that changes no header still follows one order. Say whether it weakens a check. No gate or
rerun is to be run.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. The rest is listed apart, as not required. Verdict, approved or rejected with
the fixes required, briefly, in your final message. Create no scratch.
