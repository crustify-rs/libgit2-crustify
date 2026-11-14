# The additive conflicts of an integration merge, settled by scripts

Status: `open`. Filed by the assistant on 2026-11-14, from the pilot of five `port` modules.

## Signal

One procedure repeated by hand (`evolution` §2: the same script written again). `versioning`
§4.5 has the assistant resolve an integration merge's additive conflicts itself, and names
them: the dashboard's rows, the raw bindings' allowlists, module declarations, export
features, a file two runs homed items in. Every merge of the pilot met some of them, and
`accounting` §2.1's rules for the dashboard's `Σ` cells cannot be applied by eye.

## Evidence

- Twelve merges of the working branch into a wave or module branch between 2026-11-13 and
  2026-11-14 (waves 186, 194 and 232 taking the working branch; the integrations of
  `compat.h`, `utf8.c`, `opt.c`, `api.c` and `http.c`; glibc-crustify's `fprintf.c`): the
  dashboard conflicted in all but two, the allowlist in two, module declarations in one, the
  export features in three, wavefront's ownership store in one.
- The first was settled by hand (glibc-crustify `42991d2`), seven hunks of `Σ` arithmetic.
  The scripts beside this proposal settled the rest: `dashmerge.py` (rows both sides added
  kept; a row both changed merged cell by cell, additive figures ours plus theirs less the
  base's, percentages the later landing's; prose kept from both), `storemerge.py` (the
  ownership store keyed by name and file), `modmerge.py`, `keepboth.py`, `cargomerge.py`,
  and the two drivers, `takedevel.sh` and `prepswitch.sh`, which also regenerate the
  bindings when the merged allowlists change them.
- Two mistakes of the assistant's the drivers did not prevent: a progress commit on the
  working branch while a module integrated (`opt.c`, 2026-11-14: its fast-forward was
  refused and the merge redone, `c80c8a527`), and a driver that went on after that failure.
  A driver should refuse to start while a module branch holds an integration merge the
  working branch has moved past, and stop at the first failed step.
- Derived cells the merge leaves stale: a share or a per-unit cost beside a count both sides
  changed (`ported symbols` read 2.55% beside 26), which later batches corrected by hand.

## Destination

The spec and its scripts: a script beside `versioning`, documented by its `--help`, as
`authoring` places one, with `accounting` §2.1 naming it; the dashboard merge is the part no
agent can do reliably without it.

## Change

Not made in Crustify. The scripts here are the campaign's, as they ran.

## Validation

Their merges are the working branch's history; no test suite covers them yet.

## Record

Drafted: recurrence signal, an integration merge whose dashboard carries a repeated heading,
a `Σ` cell that is not the sum of its rows, or a conflict marker; status `watched`.

## Review

None yet.
