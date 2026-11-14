# Nothing says what a switch does with a review reproducer that names its deleted export feature

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Attribution. Every fact a decision rests on can be reproduced; a reproducer that no
longer resolves on the current tree must say where it does.

## Signal

Nearest row: "skills that contradict each other, or an agent prompt that leads its agent
wrong", a defect in the spec. Here the skills do not contradict each other; they are silent,
and two switches each had to decide.

## Evidence

**A review's reproducer that enables the deleted export feature no longer resolves (item
7).** Met at the second switch, `src/cli/opt.c`'s
(`modules/git2/src/cli/opt.c/integration/run-1`, the fork's `f57e1a616`, dashboard Notes
entry `git2/src/cli/opt.c` integration). Two reproducers under
`crustify/reviews/modules/git2/src/cli/opt.c/wave-186/` name `ffi-export-src-cli-opt-c` in
their manifests; nothing says whether a switch updates them or leaves them reproducing at
the commit their report names.

- The two reproducers were left as they are, reproducing at the commit their reports name.
- Item 7 again, at `deps/llhttp/api.c`'s switch
  (`modules/libgit2/deps/llhttp/api.c/integration/run-1`):
  `wave-194/batch-7/equiv/execute-export-moves-an-empty-runs-error-pos`'s reproducer names
  `ffi-export-deps-llhttp-api-c`, and is left reproducing at the commit its report names.
- From item 14, at `deps/llhttp/http.c`'s switch: after every mention of the feature's name
  is removed from the crate, only the run records and the reviews' reproducers still name it.

## Destination

From the original's Destination list:

- `reviewing` (item 7): whether a switch updates the reproducers that name its export
  feature.

Protected: all of it. `reviewing` (`skills/verification/reviewing/SKILL.md`) is under
`skills/verification/**`.

## Change

Not made.

`reviewing`'s Reproducers section would state the rule the two switches followed: a
reproducer that enables a module's export feature is left unchanged at the module's switch,
and reproduces at the commit its report names. The reason is that a reproducer proves a
finding on the affected commit, and the report already names that commit.

The original asks only "whether" a switch updates them, so the other answer stays open: the
switch drops the feature from each reproducer's manifest, so that it builds on the current
tree. `reviewing`'s Revisit objective reruns earlier reports' reproducers on the current
tree, so the review of this proposal should say which answer that objective needs.

No script moves with this proposal.

## Validation

Copied from the original:

- `src/cli/opt.c`'s switch: "Items 5 to 9 are its evidence, and are not made".

Three reproducers were left unchanged at two switches, as the Evidence records.

## Review

None yet.
