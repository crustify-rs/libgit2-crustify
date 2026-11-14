# ffimap records the enclosing repository's HEAD as its own version

Status: `open`. No change is made yet.

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

Not made. The version is read from ffimap's own checkout or package, and is absent, not another repository's, where neither gives one.

## Validation

None yet.

## Review

None yet.
