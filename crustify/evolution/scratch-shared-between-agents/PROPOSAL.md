# Scratch a host shares between concurrent agents

Status: `open`

## Signal

An agent prompt that leads its agent wrong. The batches of one wave run as subagents of
the assistant's session, and Claude Code hands each of them that session's scratchpad
directory (`/tmp/claude-0/<project>/<session id>/scratchpad`) as the place for temporary
files. `layout`'s Scratch section names `crustify/tmp/` for residual files, but only as the
place "in the repository" for them, so an agent that follows its host's guidance writes to
the shared directory instead, and siblings running at once clobber each other's files.

## Evidence

`foundational/libgit2/src/util/alloc.c/wave-12/batch-2/run-1` ran beside `wave-169/batch-1`,
`wave-169/batch-2` and `wave-176/batch-1`, all four handed the same scratchpad. This run
wrote its gate logs there under generic names (`miri.log`, `asan.log`, `tsan.log`), and
`wave-169/batch-1`'s agent wrote files of the same names: the Miri log this run read back
held `wave-169/batch-1`'s compile, and its `asan.log` was replaced within minutes. The run
caught it from the paths in the log, killed its own coverage runs, whose output dirs
(`cov-base`, `cov-new`, `base`) had names as generic, and reran everything under a
run-specific subdirectory. `wave-176/batch-1` had already used one (`b176/`). Nothing wrong
landed, but a gate result read from a sibling's log would have.

## Destination

The spec: `layout`, Scratch, which the binder does not own; and the role prompts' notes,
which `authoring` places.

## Change

In `layout`, Scratch, after its first sentence:

> An agent's scratch is its own: its worktree's `crustify/tmp/`, or, outside a worktree, a
> directory named after its batch id. A directory the host hands every agent of a session,
> such as a session scratchpad its subagents share, is never used under a name another
> agent could pick.

## Validation

None beyond this run: the rule removes the collision by construction, since each batch's
worktree is its own.
