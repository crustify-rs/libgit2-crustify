Review one change to Crustify, as your role's review of a change and the `evolution` skill's Review section describe.

- Your run's task: `/target/libgit2-crustify/crustify/artifacts/evolution/reviewed-evolution/review-1/task.json`.
- The change: crustify commit `ae35f81` on branch `review/evolution/reviewed-evolution` of `/opt/crustify` (a symlink of `/crustify-rs/crustify`). That branch is checked out in another workspace's worktree: read it with `git -C /opt/crustify show ae35f81` and `git -C /opt/crustify diff e2a51c6 ae35f81`, or in the detached copy at `/opt/crustify.wt/reviewed-evolution`. Do not check the branch out, and change or commit nothing in any repository.
- The proposal: `/target/libgit2-crustify/crustify/evolution/reviewed-evolution/PROPOSAL.md`.

This change rewrites the review rule itself, so read the `evolution` skill as it stands on `devel` (`/opt/crustify/skills/crustify/evolution/SKILL.md`) and as the change leaves it, and judge it against `GOALS.md`. Beyond the Review section's checks, say:
- whether it weakens any check (it is meant to strengthen them; look for any rule it loosens, such as the review's approval standing in for the user's under `unattended`);
- inconsistencies between the skill, the assistant and reviewer roles and `task-schema` (for example when `EVOLUTION.md` is written: as each change lands, or at a module's end);
- what the record should hold for changes that landed before it, on 2026-11-12: crustify `dcaaadd` and `e2a51c6`, ffibox `9c1a54c` and `a0c44b2`, wavefront `497794b`, `69a97a0` and `bf21e16` (their proposals are under `/target/libgit2-crustify/crustify/evolution/`);
- its cost: how much a review per change adds, and whether anything should be exempt.

Give your verdict, approved or rejected with why, and each point with the file and line it concerns, in your final message.
