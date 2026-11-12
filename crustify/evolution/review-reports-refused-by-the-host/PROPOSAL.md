# Review reports the host refuses to let a reviewer write

Status: `landed`, as crustify `dcaaadd` on `devel`, the change's option 1: the `claude`
provider's `spawn.py` launches each stage run as a process of its own, and the assistants'
prompts end with how they launch through it. The interim is done too: the assistant filed
the refused reports of the foundational review batches, from their commits, under their
reviews dirs.

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
- Planner run 12 (`crustify/planner/run-12`) met it too, for a plan correction the
  `reviewing` skill's Plan corrections section files as
  `crustify/reviews/planner/modules/if-still-in-c-for-test-only-exports-alone/report.md`:
  the Write was refused with the same message, so the report went to the assistant in the
  run's final message. The refusal reaches every stage agent run as a subagent that files
  reports, not reviewers alone.

## Destination

The `claude` provider (`providers/claude/`), as the `evolution` skill's Destinations section
has placed a host's refusal since Crustify `36a7154`: the spec's rule, that a reviewer files
each finding's `report.md`, stays as it is.

## Change

The provider prevents the refusal with what Claude Code offers, in this order of preference:

1. it launches each stage run as a process of its own (`claude -p --agent crustify:<role>`),
   whose agent is a main agent rather than a subagent, so the host's subagent guard on
   report files does not apply; or
2. the settings it installs allow the stage agents' writes under the campaign's
   `crustify/reviews/` and `crustify/artifacts/` paths, if Claude Code's permission rules
   reach that guard.

Until one lands, the interim the Destinations section gives applies: the reviewer hands
each report, in the Reports section's format, in its final message, one section per
`<objective>/<slug>`, and the assistant files them verbatim under the batch's reviews dir.
The reports of `wave-176/batch-2` and `batch-3`, and of the later review batches whose
commits carry them, are filed that way from their commit messages.

## Validation

Not yet validated: option 1 or 2 is validated by the next review batch writing its reports
itself; the interim by the assistant filing the earlier batches' reports from their commit
messages.

Validated on 2026-11-12: a general-purpose subagent's Write of
`crustify/tmp/probe-subagent/wave-0/batch-0/misc/probe/report.md` was refused with the message
above, a `claude -p` process's Write of the same shape of path succeeded, and a
`crustify:reviewer` run launched through `spawn.py` wrote its `report.md`, recorded its
`session_id` and `exit_code` in its run dir, and had its transcript copied into
`$CRUSTIFY_HOME/transcripts/`.
