This run is not a review batch: it reviews one change to Crustify, and changes, commits and lands nothing. You run under Crustify's `devel` (`e2a51c6`), whose reviewer role does not yet describe such a review, so this message gives its checks; nothing here relaxes your role.

- The change: crustify `libgit2-crustify/evolution/reviewed-evolution` at commit `6d533ec`, i.e. `git -C /opt/crustify diff e2a51c6 6d533ec`. It is two commits: the user's draft `ae35f81` and the fixes `6d533ec`. Read it with git, or in the worktree `/opt/crustify.wt/reviewed-evolution` (read-only). Your own skills are devel's; judge the change, don't follow it.
- The proposal, with the first review's verdict and its five required fixes: `/target/libgit2-crustify/crustify/evolution/reviewed-evolution/PROPOSAL.md`. The first review's run: `/target/libgit2-crustify/crustify/artifacts/evolution/reviewed-evolution/review-1/`.

Check, citing file and line:
1. Each of the five required fixes and each advisory of review-1: resolved, partly, or not.
2. The checks the change itself names for a review: the proposal's evidence shows the signal, and the change answers it; it serves a goal of `GOALS.md`; it is placed and written as `authoring` describes, with no rule in the target's names; its validation was run and shows what the proposal says; and it weakens no check. Look hard for any rule it loosens.
3. Consistency between the `evolution` skill, the assistant and reviewer roles (`agents/`, and the generated `providers/claude/agents/`, which must match `compose.py build`), `task-schema`, `layout`, `accounting` and `authoring`.
4. `EVOLUTION.md`: does every 2026-11-12 landing on the devel branches of Crustify, ffibox (`/opt/venv/share/ffibox`), wavefront (`/crustify-rs/wavefront`) and ffimap (`/crustify-rs/ffimap`) have an entry? Does each entry follow the Record section's format? Is each recurrence signal one a campaign would observe? Sample at least ten entries against their proposals under `/target/libgit2-crustify/crustify/evolution/`.

Give your verdict, approved or rejected with why, in your final message. Note that this change edits the `evolution` skill, so only the user's approval lands it; your verdict is input to that.
