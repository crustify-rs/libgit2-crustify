This run is not a review batch: it reviews three changes to Crustify's spec, as the `evolution` skill's Review section describes, one verdict per change, and changes, commits and lands nothing. You run under the campaign's working branch of Crustify, `libgit2-crustify/devel` (`63b92be`). The changes are stacked, each forked from the one before, so each is reviewed as its own diff:

1. `git -C /opt/crustify diff 63b92be 6aa6dc4` (`libgit2-crustify/evolution/bindgen-llvm-pin`): the bindgen skill's libclang TODO answered. Proposal: `/target/libgit2-crustify/crustify/evolution/bsan-regenerates-other-bindings/PROPOSAL.md`; the campaign's implementation it generalizes is `git -C /target/libgit2-crustify show 47b4662c9` (build.rs `pin_llvm`, Cargo.toml metadata).
2. `git -C /opt/crustify diff 6aa6dc4 b3ebca7` (`…/ffibox-canonical-path`): the ffibox tool skill's path dependency. Proposal: `…/crustify/evolution/ffibox-canonical-path/PROPOSAL.md`; the campaign's fix `git -C /target/libgit2-crustify show 2ea24a0db`.
3. `git -C /opt/crustify diff b3ebca7 ce6445d` (`…/bindgen-modes-after-bundling`): the bindgen skill's Generation, Build modes, Extending and Publishing sections and the setup skill's Builds section. Proposal: `…/crustify/evolution/bindgen-modes-after-bundling/PROPOSAL.md` (items 1–8); the campaign's tree they describe is crustify/devel at `/target/libgit2-crustify` (`build.rs`, `src/sys.rs`, `sys/`, `Cargo.toml`), from `53786c97e` and `44e95b76d`.

Read the changes in `/opt/crustify.wt/spec-fixes` (read-only; it has the last branch checked out) or with git. Your own skills are the working branch's: judge the changes, don't follow them.

For each change, check, citing file and line:
- that the proposal's evidence shows the signal, and that the text answers it and agrees with what the campaign actually built (read the campaign code it describes; a rule the code contradicts, or a code behaviour the rule leaves unsaid, is a finding);
- placement and wording per `authoring`, no rule stated in the target's names (libgit2, git2, the CLI's file names, host paths other than the image's documented ones);
- consistency with the rest of the spec (bindgen's Layout, Build modes table, Landing; setup's Builds; testing-gates' Regressions; ffibox's Rust tree TODO), and that `python3 -I providers/claude/compose.py build` in a `git archive ce6445d` copy leaves `providers/claude/` unchanged;
- the `EVOLUTION.md` entry each adds: a recurrence signal a campaign would observe; and
- whether it weakens any check (change 3's shared-mode refusal and change 1's dependent regeneration left unpinned in particular).

Give a verdict per change, approved or rejected with the fixes each requires, briefly, in your final message.
