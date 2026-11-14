# `testing-gates` restates a switch's parts in an order `porting-workflow` contradicts

Status: `landed` as crustify `6868367` (record `57005de`) on `libgit2-crustify/devel`, with the user's
approval on 2026-11-14, which its protected path needs.

Goal: Maintainability: each rule has one place. Its measure is the rules stated in more than
one skill.

## Signal

Skills that contradict each other. `testing-gates`' Regressions section says a module's switch
"removes its translation unit …, rebuilds the variants from the C that remains and deletes the
module's export feature". Read as a sequence, that is the C change, then the variants, then
the feature, the order `module-switch-procedure`'s item 10 found cannot run.
`porting-workflow`'s Builds section, since `switch-step-order` landed (`10500c9`), gives the parts in the
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

By removal, in `testing-gates`' Regressions section: "A module's switch removes its translation
unit, and the C tests tied to it that reach its internals, from `c/`, rebuilds the variants
from the C that remains and deletes the module's export feature, as `porting-workflow`'s
Builds section describes" becomes "A module's switch runs as `porting-workflow`'s Builds
section describes", and its gates run "over the variants built from the C that remains".

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `testing-gates` no longer restates a switch's parts, which `porting-workflow` states in the order they run | `libgit2-crustify`, `switch-order-in-testing-gates` | crustify `6868367` | `testing-gates` listed the parts with the feature deleted last, an order that cannot run, beside `porting-workflow`'s | a run that follows a switch's order from `testing-gates`, or a review finding of two skills stating a switch's parts differently | watched |

## Validation

None is needed: no run failed by this text, both switches having run in the order that works,
and the effect of removing a restated list is not in doubt.

## Review

`review-1` (Opus 5.5, `$0.98`), on `6868367`: **approved**, no fix required; it weakens no check. No run
loses a rule it needs, the three removed parts being in `porting-workflow`'s Builds section in
the order that works; the gates of a switch mean what they did; and "the variants built from
the C that remains" agrees with `setup` and `testing-builds`. Its notes on this proposal's
goal, validation and signal are taken above.
