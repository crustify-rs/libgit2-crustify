# Review reports the host refuses to let a reviewer write

Status: `open`

## Signal

A rule an agent cannot follow: the `reviewing` skill's Reports section, and the reviewer
role's step 3, have a review batch write each finding's `report.md` under its reviews dir.
The host that runs reviewers as subagents refuses that write, with "Subagents should
return findings as text, not write report files. Include this content in your final
response instead." Two consecutive review batches met it, so their findings exist only in
commit messages and final reports, and `crustify/reviews/` holds none of them: the
reviews dir that `revisit` reruns, and the dashboard's report counts cite, is empty for
them.

## Evidence

- `wave-176/batch-2/run-1` of `src/util/allocators/failalloc.c`: its fix commit
  `ab7d4915c` says "The harness kept the run from writing report.md files under
  crustify/reviews/, so this message records each finding instead"; the dashboard's note
  on that batch records the same.
- `wave-176/batch-3/run-1`, the module pass of the same module: the Write of
  `crustify/reviews/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-3/misc/failalloc-foreign-storage-guarantee-unstated/report.md`
  was refused with the message above. The task message told the run not to work around
  the refusal, so the finding is carried in the fix commit's message and the run's final
  report. The run's coordinator then sanctioned writing the reports through the shell,
  and withdrew that within minutes: the harness's safety classifier judged the route a
  way around its guard, and the user is to settle it through their settings. A second
  attempt through the file tool, for one of batch 2's reports, was refused the same way.
  None of the three reports is filed.

## Destination

The spec: the `reviewing` skill's Reports and Landing sections, the reviewer role, and the
assistant's review steps.

## Change

Either the host's settings let reviewer subagents write under `crustify/reviews/`, which
only the user can decide, or the spec routes the reports around the reviewer:

Where the host refuses a reviewer's report writes, the reviewer hands each finding's
report, in the format the Reports section gives, in its final message, one section per
`<objective>/<slug>`, and the assistant files them verbatim under the batch's reviews dir
as the batch's first commit before promoting the wave, with the reviewer's run id in its
subject. A reproducer, which is code rather than a report, is still written by the
reviewer. The Landing section then says that a review batch that could file no report
lands its Rust-tree commit alone, and that its reports commit follows from the assistant.

## Validation

Not yet validated: it needs the assistant to file batch 3's report, and batch 2's two
findings from `ab7d4915c`'s message, and the next review batch to run under the changed
skill.
