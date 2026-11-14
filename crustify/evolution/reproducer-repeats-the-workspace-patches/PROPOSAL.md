# A reproducer package repeats the affected workspace's `[patch]` tables

Status: `open`. Filed by the assistant on 2026-11-13 for `wave-186/batch-6`; the libc side
campaign's `wave-4/batch-1` filed the same signal there as
`reproducer-carries-the-workspace-patches`, so two runs have met it.

## Signal

A rule the how-to skills lack, met by two review batches (`evolution` §2).

## Evidence

- `wave-186/batch-6/run-1` (`961150dd9`): its first reproducer build failed with two versions
  of `ffibox`, because Cargo applies `[patch]` tables only in the workspace that declares
  them, and a reproducer is a standalone package with an empty `[workspace]`.
- glibc-crustify's `wave-4/batch-1/run-1` (`a5448ee`): without the workspace's patches its
  reproducer built ffibox `40621a2` rather than the `9c22a73` the workspace uses.

## Destination

The spec: `reviewing`'s Reproducers section.

## Change

Not made. As proposed: a reproducer's `Cargo.toml` repeats the `[patch]` tables of the
workspace whose crate it depends on, and its `Cargo.lock` is copied from it, so that it
builds the dependency graph the affected commit builds.

From glibc-crustify's `reproducer-carries-the-workspace-patches`, which this proposal
supersedes, the sentence for `reviewing` § 5: after "…depends on the affected crate through a
relative `path`", add ", and repeats the affected workspace's `[patch]` tables, which apply
only from the root of the workspace being built, so that it builds against the dependencies
the affected commit builds with".

## Validation

None yet.

## Review

None yet.
