# The coverage report has no unit suite, which each batch merges by hand

Status: `open`.

## Signal

One procedure repeated by hand (`evolution` §2). The dashboard's `Coverage stats` table and
every batch's `+unit tests` cell read the unit suite as `lib` and `unit_pub` together, but
`crustify/scripts/coverage/report.py` reports each profile directory under `prof/` alone,
the four-suite `TOTAL` and `ALL`, and nothing that merges `lib` with `unit_pub`.

## Evidence

`wave-194/batch-0/run-1`, `wave-194/batch-1/run-1` and `wave-194/batch-2/run-1` each record
"the unit column merging `lib` and `unit_pub`" in their Notes entries. `batch-2` did it by
linking both suites' profiles into a `prof/unit/` directory of each coverage run and
running `ONLY=unit report.py …`; that directory is then one more suite for any later
`report.py` run without `ONLY`, so `ALL` would count `lib` and `unit_pub` twice in its
profile list (harmless for a union, but a trap for anything that sums).

## Destination

A campaign artifact the assistant owns: `crustify/scripts/coverage/report.py` (and the
`Coverage stats` method it serves).

## Change

Report a `unit` suite, the merge of `lib` and `unit_pub` (and `unit_priv` where a tree has
it), beside the per-binary suites, so that the dashboard's four columns are read directly.

## Validation

None run. `batch-2`'s by-hand figures, `33.17`% / `87.78`% on `f1eced216` and `33.74`% /
`87.87`% on its tree, are what the change should reproduce.

## Review

None yet.
