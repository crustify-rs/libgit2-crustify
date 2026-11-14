# ffimap records the enclosing repository's HEAD as its own version

Status: `open`: the change is ffimap `a73eb55` on `libgit2-crustify/evolution/ffimap-scanner-records-the-enclosing-head`, made on 2026-11-14,
awaiting its review.

Goal: Attribution: every figure names the revision that produced it.

## Signal

A tool that reports what the source contradicts.

## Evidence

Found by `ffimap-wrapper-without-alias`'s review-1 on 2026-11-14: run from a copy that is no git repository, nested inside another repository, `scanner.ffimap` records that repository's HEAD. Its scans from `crustify/tmp/` of this campaign recorded `2111b0af`, libgit2-crustify's HEAD, as ffimap's revision.

A second way, met on 2026-11-14 (`ffimap-sys-inventory-in-crate`'s review-2): a scan from a
checkout with uncommitted changes records the commit below them. `scan-new2.json` of that
proposal records `8cd7a26` for a tree that became `a0797e6`.

## Destination

A tool: ffimap's scanner version, `scan.py`.

## Change

In `measure.py`, `own_revision`: the commit counts only where the repository's top is ffimap's
own, two levels above the package, and carries `+dirty` with tracked changes not committed.
`scanner` falls back to the package's version otherwise, as before. Three tests in
`tests/test_scan.py`; `docs/analysis.md` states the forms.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| ffimap records the enclosing repository's HEAD as its own version: fixed | `libgit2-crustify`, `ffimap-scanner-records-the-enclosing-head` | ffimap `a73eb55` | a scan from a copy nested in another repository recorded that repository's HEAD as ffimap's, and a scan from a checkout with uncommitted changes the commit below them | test | |

## Validation

ffimap's suite passes in its own checkout with the change, the new tests among them; the
workspace test fails without the manifest's table.

## Review

None yet.
