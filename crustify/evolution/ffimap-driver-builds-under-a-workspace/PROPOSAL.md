# ffimap's driver does not build under a directory that holds a Cargo workspace, and its suite then skips

Status: `open`. No change is made yet.

Goal: Verification: a gate result a run reports is one its tree reproduces.

## Signal

A tool that crashes, and a test suite that reports success when it ran half its tests.

## Evidence

Found by `ffimap-wrapper-without-alias`'s review-1 on 2026-11-14. The driver's `Cargo.toml` has no `[workspace]` table. Reproducer: extract `git archive 2a2aa1e` of ffimap into `crustify/tmp/x` of this campaign, which is under its Cargo workspace, and run `PYTHONPATH=src python3 -m unittest discover -s tests`: the driver fails to build, and the suite reports OK with 75 of 145 tests skipped. Two reviews of 2026-11-14 ran ffimap's suite in such a copy; `doc-alias-equal-to-the-name`'s review-1-ffimap reported 70 run and 75 skipped.

## Destination

A tool: ffimap's driver `Cargo.toml`, and its tests' skip condition.

## Change

Not made. An empty `[workspace]` table in the driver's manifest; and a driver that fails to build fails the driver suites instead of skipping them.

## Validation

None yet.

## Review

None yet.
