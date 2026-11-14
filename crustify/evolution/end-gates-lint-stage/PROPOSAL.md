# `end-gates` runs the format and lint gates over the committed tree

Status: `landed` as crustify `888b582` (record `f307cc7`) on `libgit2-crustify/devel`, with the user's
approval on 2026-11-14, which its protected path needs, and which covered landing the
restacked commit without a new review; its 9 tests were rerun on it and pass.

Goal: Verification: a gate result a run reports is one its landed tree reproduces.

## Signal

A review finding that recurs: a landed tree that fails the lint gate every landing passes.
`testing-gates`' Regressions section has every batch format and lint the workspace "on the
tree it lands, with warnings denied", and `end-gates` runs a wave's and a module's end gates
over a snapshot of the committed tree, so that what they record is that tree's. Its stages
were the tests alone. Each batch ran `cargo fmt` and `cargo clippy` by hand, on its working
tree, and nothing tied the result it recorded to the commit it landed.

## Evidence

- `wave-197/batch-1/run-1`, `deps/llhttp/http.c`'s module pass,
  `misc/lint-gate-fails-on-the-batchs-own-tests`: the wave's tip `a81dc3103`, which
  `wave-197/batch-0` landed, fails `clippy --workspace --all-targets -- -D warnings` with the
  export feature off and on: `clippy::too_many_lines` and `clippy::type_complexity` in
  `tests/unit_pub/deps/llhttp/http.rs`, and the denied `multiple_unsafe_ops_per_block` in
  `tests/equiv/deps/llhttp/http.rs`. The batch's notes record "`clippy --workspace
  --all-targets` passes with warnings denied with the export feature off and on".
- `legacy-wave-35/batch-0`, `misc/memory-options-tests-drop-non-drop`: on the review wave's
  base, clippy printed three `drop_non_drop` warnings, which "kept `-D warnings` from
  passing".
- `legacy-wave-49/batch-0`, `conventions/undocumented-unsafe-blocks`: a landed block "also
  fails the workspace lint `clippy::undocumented_unsafe_blocks`".

## Destination

The spec's tool: `end-gates`, the script beside the `testing-gates` skill, and the skill's
paragraph that describes it.

## Change

- `end-gates` gains the stage `lint`, first among its default stages. Over the snapshot it
  runs `cargo +<nightly> fmt --all --check`, then `<runner> plain clippy --workspace
  --all-targets [--features <features>] -- -D warnings`, once with no feature and once for
  each `--lint-features` (repeatable), such as a port's unswitched export features. Each run
  is logged and summarised as the other stages' are (`lint-fmt`,
  `lint-clippy[-<features>]`). `--nightly` now names the formatter's toolchain too, since the
  workspace's `rustfmt.toml` sets unstable options (`toolchain`'s Formatting section).
- `testing-gates`' Regressions section: `end-gates` runs the format check and the lints once
  per export-feature set the batch checks, and a batch's own format and lint gates run the
  same way, over the commit it lands, so that the result it records is that tree's.
- `test_end_gates.py` gains `test_lint_checks_the_commit_once_per_feature_set`. A tiny
  workspace formats and lints clean with no feature, and its feature `extra` adds a
  `needless_return`. With an unformatted edit left uncommitted, `lint-fmt` and `lint-clippy`
  pass and `lint-clippy-extra` fails. Once the edit is committed, `lint-fmt` fails. Without
  the change, `--lint-features` is refused and `lint` would run as a variant.
- `EVOLUTION.md` gains the change's entry, recurrence signal "a review finding of a landed
  tree that fails the format or lint gate, run by `end-gates` or recorded as passing by its
  batch".

The change adds a check and weakens none.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`28a5e51`, which also edited `EVOLUTION.md`; on 2026-11-14 it was rebased as `888b582`, without that edit, since a change's row is drafted in its proposal. Its sentence of `testing-gates` was merged with the slow list's wording of `gates-slow-list`, which landed first; `test_end_gates` passes on it (9 tests).

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `end-gates` runs the format and lint gates over the committed tree, once per export-feature set | `libgit2-crustify`, `end-gates-lint-stage` | crustify `888b582` | three reviews found a landed tree failing the lint gate with warnings denied, which every landing passes and which `end-gates` did not run: `legacy-wave-35/batch-0`, `legacy-wave-49/batch-0` and `wave-197/batch-1`, whose batch recorded it as passing | a review finding of a landed tree that fails the format or lint gate, run by `end-gates` or recorded as passing by its batch | watched |

## Validation

- `python3 -I test_end_gates.py -v` in the change's `skills/verification/testing/gates/`:
  9 tests pass, the new one among them.
- The failing case, rerun under the change from `wave-197/batch-1`'s worktree:
  `end-gates . --logs crustify/tmp/logs-lint-base --stages lint --lint-features
  ffi-export-deps-llhttp-http-c --rev a81dc3103 --label lintbase --link crustify/builds`
  exits 1. `lint-fmt` passes in 3.6 s. `lint-clippy` and
  `lint-clippy-ffi-export-deps-llhttp-http-c` exit 101 on the finding's three errors and no
  other (`tests/unit_pub/deps/llhttp/http.rs:104` and `:106`,
  `tests/equiv/deps/llhttp/http.rs:65`).
- The same over the batch's fixed tree (`--working-tree`) exits 0: all three runs pass, in
  3.4, 48.9 and 34.6 s at 24 jobs.

## Review

`review-1` (Opus 5.5, `$1.10`), on `3b3316c`: **approved**; it weakens no check. `888b582` is
that commit restacked with its diff unchanged (the same `git patch-id`), after the change
beneath it was fixed. It ran the tests (9 pass; the new one fails on the old script) and read
the stage. Listed apart, not required: "once per export-feature set the batch checks" is
unclear at an end, where the script's own words are clearer; a package-qualified feature
crashes the log's name; a `lint` skip is accepted and ignored; the docstring says "committed
tree" though `--working-tree` lints the working tree.

On 2026-11-14 the approved `dcefd39` was rebased as `888b582`, its diff unchanged (`git patch-id
--verbatim`), once `gate-findings-outside-the-workset` landed beneath it.
