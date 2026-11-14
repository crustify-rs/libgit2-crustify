# ffimap's driver does not build under a directory that holds a Cargo workspace, and its suite then skips

Status: `open`: the change is ffimap `5ac9ec5` on `libgit2-crustify/evolution/ffimap-driver-builds-under-a-workspace`, made on 2026-11-14,
awaiting its review.

Goal: Verification: a gate result a run reports is one its tree reproduces.

## Signal

A tool that crashes, and a test suite that reports success when it ran half its tests.

## Evidence

Found by `ffimap-wrapper-without-alias`'s review-1 on 2026-11-14. The driver's `Cargo.toml` has no `[workspace]` table. Reproducer: extract `git archive 2a2aa1e` of ffimap into `crustify/tmp/x` of this campaign, which is under its Cargo workspace, and run `PYTHONPATH=src python3 -m unittest discover -s tests`: the driver fails to build, and the suite reports OK with 75 of 145 tests skipped. Two reviews of 2026-11-14 ran ffimap's suite in such a copy; `doc-alias-equal-to-the-name`'s review-1-ffimap reported 70 run and 75 skipped.

## Destination

A tool: ffimap's driver `Cargo.toml`, and its tests' skip condition.

## Change

In `src/ffimap/driver/Cargo.toml`, an empty `[workspace]` table. In `measure.py`,
`driver_prerequisites`: `cargo`, `rustup` and the dated nightly present. The four driver
suites skip on that alone and then build the driver, so a build that fails raises at import
and fails the module. `tests/test_scan.py` gains a test that a copy of the driver below
another workspace resolves (`cargo metadata`), which fails without the table.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| ffimap's driver does not build under a directory that holds a Cargo workspace, and its suite then skips: fixed | `libgit2-crustify`, `ffimap-driver-builds-under-a-workspace` | ffimap `5ac9ec5` | under a directory holding a Cargo workspace the driver did not build, and ffimap's suite reported success with 75 of 145 tests skipped; two reviews ran it so | test | |

## Validation

ffimap's suite passes in its own checkout with the change, the new tests among them; the
workspace test fails without the manifest's table.

## Review

None yet.
