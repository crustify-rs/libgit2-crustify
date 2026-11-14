# `testing-gates` restates a switch's parts in an order `porting-workflow` contradicts

Status: `open`. No change is made yet. Its destination is a protected path.

Goal: Consistency; and Verification, whose gates a run must be able to carry out as written.

## Signal

Skills that contradict each other. `testing-gates`' Regressions section says a module's switch
"removes its translation unit …, rebuilds the variants from the C that remains and deletes the
module's export feature". Read as a sequence, that is the C change, then the variants, then
the feature, the order `module-switch-procedure`'s item 10 found cannot run.
`porting-workflow`'s Builds section, once `switch-step-order` lands, gives the parts in the
order that runs: both sides, then the builds, then the gates.

## Evidence

`switch-step-order`'s review-1, on 2026-11-14, which read both switches' transcripts
(`modules/libgit2/deps/llhttp/api.c/integration/run-1` and
`modules/libgit2/deps/llhttp/http.c/integration/run-1`): each regenerated the bindings and
deleted the feature before the variants were built.

## Destination

The spec: `testing-gates`' Regressions section, `skills/verification/testing/gates/SKILL.md`,
a protected path.

## Change

Not made. By removal: the section drops its restated list and keeps "a module's switch, as
`porting-workflow`'s Builds section describes", with the gates over the variants built from
the C that remains.

## Validation

None yet.

## Review

None yet.
