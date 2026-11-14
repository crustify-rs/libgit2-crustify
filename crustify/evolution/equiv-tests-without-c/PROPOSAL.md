# An equivalence test that calls no C is carried over at the switch

Status: `landed` as crustify `7da3744` (record `6fb272d`) on `libgit2-crustify/devel`, on 2026-11-14, once its
review approved it. Split from `module-switch-procedure` that day.

Goal: Equivalence. Its measure counts the inputs and states on which the Rust is compared
with the C it replaces; a comparison that still runs after the switch should not be lost.

## Signal

"Skills that contradict each other, or an agent prompt that leads its agent wrong": a defect
in the spec. `porting-workflow` retires a module's equivalence tests "which need the C", and
some of them do not need it.

## Evidence

**Retiring a module's equivalence tests by file loses those that run no C (item 6).** Met at
the second switch, `src/cli/opt.c`'s (`modules/git2/src/cli/opt.c/integration/run-1`, the
fork's `f57e1a616`, dashboard Notes entry `git2/src/cli/opt.c` integration).
`porting-workflow` retires them "which need the C", and keeps their behaviour in unit tests,
but an equivalence file also holds tests that only read the raw bindings' constants, which
stay with the header: two of `opt.c`'s did, and no unit test kept what they established.

- `usage_flags_match_the_c_constants` and `parse_flags_match_the_c_constants`, which
  compared `Usage`'s and `ParseFlags`' bits with `opt.h`'s `CLI_OPT_USAGE_*` and
  `CLI_OPT_PARSE_*` through the raw bindings, moved to `tests/unit_pub/src/cli/opt.rs`; the
  CLI's spec tables, which stay C, set those bits. The switch's step would read: before an
  equivalence file is retired, each of its tests that calls no C is carried over to the
  module's unit tests.

**The equivalence tests that call no C are marked already (item 12).** Met at the third
switch, `deps/llhttp/api.c`'s (`modules/libgit2/deps/llhttp/api.c/integration/run-1`). Item
6's step needs each retiring test's calls checked. A test that calls C carries
`#[cfg_attr(miri, ignore = "calls C")]` (the Miri gate's marker), so the tests that lack it
are the candidates, a mechanical check; here all 21 carry it, and none is carried over.

- `grep -c 'ignore = "calls C"'` over `tests/equiv/deps/llhttp/api.rs` gives 21, its
  `#[test]` count; the module pass's map (`wave-194/batch-7`'s `misc` report) names the unit
  tests that keep each.
- Item 12 again, at `deps/llhttp/http.c`'s switch
  (`modules/libgit2/deps/llhttp/http.c/integration/run-1`): all six of
  `tests/equiv/deps/llhttp/http.rs`'s tests carry the `calls C` marker; the module pass's
  `equiv-retirement.md` maps each to the unit tests that keep it.

## Destination

From the original's Destination list:

- `porting-workflow`, One language per module (item 6): an equivalence test that calls no C
  is carried over to the module's unit tests at the switch, not retired with the file.
- `porting-workflow`, One language per module (item 12): the carry-over candidates are the
  retiring tests without the Miri `calls C` marker.

Protected: none. `porting-workflow` is not protected.

## Change

Made in the skill, as the Review of the change below gives it: the rule speaks of the module's
equivalence tests that need no C, and the switch's first part lists the carry-over. The
candidates' sentence first drafted here, and item 12's bullet above, were dropped by review.
In the campaign, one switch carried two tests over by hand. What was drafted first:

`porting-workflow`'s One language per module section would add, where it retires the
equivalence tests:

> Before an equivalence file is retired, each of its tests that calls no C is carried over
> to the module's unit tests. Such a test compares the Rust with what stays after the
> switch, such as the constants of a header that remaining C still includes. The candidates
> are the retiring tests that lack the marker the Miri gate puts on a test that calls C.

No script moves with this proposal. The check is one `grep` for the marker beside a count of
the file's tests.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| before a module's equivalence tests are retired at its switch, in the Rust side's commit, each that needs no C is carried over to its unit tests, where what it compares with stays | `libgit2-crustify`, `equiv-tests-without-c` | crustify `7da3744` | retiring a module's equivalence tests by file lost those that run no C, and one switch carried two over by hand | a review or test-adequacy finding of behaviour only a retired equivalence test checked, or a switch that carries tests over by hand without the rule | watched |

## Validation

Copied from the original:

- `src/cli/opt.c`'s switch: "Items 5 to 9 are its evidence, and are not made:
  `porting-workflow` is the translator's to evolve, but item 6 wants review with the rest of
  this proposal."
- `deps/llhttp/api.c`'s switch: "Items 10, 12 and 13 are evidence, and are not made: they
  want review with the rest of this proposal."

The step was applied by hand at three switches: two tests carried over at the second, none
of 21 at the third, none of six at the fourth.

## Review

None yet.

## Review of the change

`review-1` (Opus 5.5, `$0.99`), on `11a7f05`: **rejected**, two fixes, made as `7da3744`; it weakens no check.
(1) The candidates were told by a reason string the skills never fix: this campaign's tests
say `calls libgit2` 1370 times and `calls C` 236, and a test that reads a C global calls no C
and still needs it. The sentence is removed and the rule reads "needs no C". (2) The rule
left out the equivalence tests that sit inline in a module's file, and named no point of the
switch: it now speaks of the module's equivalence tests, and the Rust side's part of the
switch lists the retirement, the carry-over first. Listed apart: a carried test that holds
unsafe code cannot go to `unit_pub`; such tests should not be written into equivalence files
at all, which is `testing`'s, a protected path.

`review-2` (Opus 5.5, `$0.28`), on `7da3744`: **approved**.
