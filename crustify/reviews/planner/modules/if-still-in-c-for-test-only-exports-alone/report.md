# `if_still_in_c` recorded for the test-only exports alone

## What the earlier plan decided

Planner run 11 (`b7cb28473`) computed, per `port/unexport` wave, the test cases "the wave
carries over if they are still in `c/`" only for the exports planned for the C tests alone
(`crustify/artifacts/planner/run-11/compose/carryover11.py`):

    for it in w['items']:
        k = uk(it)
        if it['objective'] == 'port/unexport' and k in TESTONLY:
            maybe |= set(ST.tests_left(k)[0]) - cases

The assistant split that record per item into each `todo` `port/unexport` item's
`carries_over` (`9237f3a18`), and `5d23cc8e9` kept it when the end of the C suite moved
cases out of the later waves' `cases`. So 1,698 of the 1,783 `port/unexport` items of
`crustify/devel` (`6ae0bd6f9`) have an empty `if_still_in_c`, among them every export a
remaining C module still references. For example `git_error_last`'s, in wave 1762
(`src/util/errors.c`), which depends on waves 293, 796, 1352, 1742 and 1760 alone:

    "carries_over": {"cases": [... 24 cases ...], "if_still_in_c": [], "harness": ["deps/clar/main.c:main"]}

## The evidence against it

The schedule schema (crustify `911b82e`, unchanged in substance since), `carries_over`:

> `if_still_in_c`: the cases, shaped as `cases`, that reference the export but that the
> plan's order removes before the wave, by a module's switch or another wave's carry-over
> it does not depend on: the stage carries over those still in `c/` when it runs

and: "A wave whose dependencies let it run earlier carries over what is still in `c/` when
it runs, which its batch checks against the record." The definition holds for every
`port/unexport` item, not only the exports the tests alone hold. Any stage wave runs once
its own `depends_on.scheduled` have run. So a case that references its export, and that
leaves `c/` at a switch or a carry-over the wave does not depend on, may still be in `c/`
when it runs. The stage must then carry it over, or the case no longer links. Run 11's
composition predates the schema's field: it recorded carry-overs in
`compose/carryover.json`, and its proposal `carried-over-test-cases` asked for the field.
It computed the set only for the case it was then reasoning about, the exports freed by
their own module's switch alone.

On `crustify/devel`, recomputing over HEAD's own waves and fates with the whole definition
(`crustify/artifacts/planner/run-12/compose/carryover12.py`, which with `IF_STILL_FULL=0`
reproduces every one of HEAD's 1,783 `carries_over` exactly):

    C=crustify/artifacts/planner/run-12/compose
    IF_STILL_FULL=0 python3 -I $C/carryover12.py --check-head   # differing: 0
    IF_STILL_FULL=1 python3 -I $C/carryover12.py --check-head   # differing: 520

520 items of 354 waves lack 25,783 entries. `git_error_last`'s wave 1762 depends neither
on wave 766 (`src/libgit2/repository.c`), which carries over 603 cases that reference it,
nor on the switches of the modules whose internals 1,355 more of them reach. All 1,958
are cases it carries over if it runs first.

## The edit and what it moves

`carryover12.py` gives every `todo` `port/unexport` item the schema's whole set, in the
plan's order on disk. That set is the cases referencing its export that leave `c/` before
its wave where the wave does not depend, through `depends_on.scheduled`, on the wave of
that departure. A case leaves at the switch of a module whose internals it reaches (a
switch's wave being its module's last translation wave), at another wave's carry-over, or
at the end of the C suite. For an export planned for the C tests alone it keeps run 11's
set too. `cases` and `harness` are unchanged. So are the waves after the end of the C
suite, whose `cases` and `harness` stay empty. Their `if_still_in_c` now names the cases
the suite's end removes before them, where they do not depend on it, which `5d23cc8e9` had
dropped.

No wave or item moves. On run 12's schedule, 523 items of 355 waves gain 25,918 entries
(26,605 in all). The largest is `git_error_last`'s, now in wave 1784, with 1,958.

## Question for the user

None.
