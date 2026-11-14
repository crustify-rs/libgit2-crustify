# ffimap records the enclosing repository's HEAD as its own version

Status: `open`: the change is ffimap `50a800e` on `libgit2-crustify/evolution/ffimap-scanner-records-the-enclosing-head`, made on 2026-11-14,
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

A tool: ffimap's scanner revision, `measure.py`, and `ffimap diff`, `diff.py`.

## Change

In `measure.py`, `own_revision`: the commit counts only where the repository's top is ffimap's
own, two levels above the package. With tracked changes not committed it is
`<commit>+dirty.<hash>`, the hash being of `git diff HEAD --binary`, so that two scanners
named alike are alike. `scanner` falls back to the package's version otherwise, as before.
`names_one_scanner` says which values name one scanner: a commit, bare or with that hash. In
`diff.py`, `ffimap diff` reports the scanners unless they agree and each names one, so that
two documents recorded by version alone are never passed as scanned alike. `docs/schema.md`
§ 3.2, `docs/analysis.md` and `README.md` state it.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| ffimap records its own checkout's commit, with the hash of its uncommitted changes, and `ffimap diff` never passes two scanners recorded by version alone as the same | `libgit2-crustify`, `` | ffimap `50a800e` | a scan from a copy nested in another repository recorded that repository's HEAD as ffimap's, and a scan from a checkout with uncommitted changes the commit below them | two scans compared as under one scanner that were not: a `scanner.ffimap` that is another repository's commit, or a diff that reports no scanner difference between scans of different ffimap sources | watched |

## Validation

ffimap's suite passes in its own checkout with the change (155 tests, on the working branch). The new ones: a
checkout of its own gives its commit; uncommitted changes give `+dirty.<hash>`, the same for
the same changes and another for others; a copy nested in another repository gives none; and
`ffimap diff` reports two scanners recorded as `0.1.0`, `unknown` or a bare `+dirty`, and
passes two equal commits.

## Review

None yet.

`review-1` (Opus 5.5, `$1.47`), on ffimap `a73eb55`: **rejected**, two fixes required; it weakens no check.
Every install kind attributes rightly, and the base reproduces the defect. (1) `+dirty` does not
identify a scanner: two different edits give the same string, and `ffimap diff` then reports
the scanners as the same; every copy that is no checkout of its own records the version alone.
The form must name the changes, `ffimap diff` must not report equal scanners where either is
no bare commit, the documents must say so, with a test. (2) This proposal's Validation and
Destination were another proposal's words. Not yet made.

Both fixes are made as ffimap `50a800e`: the dirty form hashes the changes, `ffimap diff` and the
three documents carry the rule, and this proposal's Destination and Validation are its own.
The row is `watched`: the tests check the forms, not every way a scan can be attributed.

`review-2` (Opus 5.5, `$0.98`), on ffimap `1075740`: the code is right and can land unchanged; one fix, to
this proposal's text, made above: it claimed `--no-optional-locks` keeps a scan from writing the
checkout's index, which `git diff` on this host's git does all the same. All four parts of
review-1's first fix are made, the new tests fail on the parent, and no skill uses `ffimap
diff`. Listed apart: the hash covers tracked changes only. The working branch had taken
`ffimap-driver-builds-under-a-workspace` meanwhile, which adds classes to the same test file,
so the commit is rebased as `50a800e`, both sets of classes kept, and reviewed once more.
