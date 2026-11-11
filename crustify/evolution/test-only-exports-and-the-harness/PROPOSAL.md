# Exports only the C tests reach, and the test harness's own references

Status: `open`, with a question for the user (below)

## Signal

A gap between skills that an agent had to fill by hand. It is a case of the defect that
the plan correction `crustify/reviews/planner/modules/test-only-referrers-unexported/`
corrected in run 6, under the rule Crustify `596b74e` introduced:

- The conventions' Exports section exports what "a C test or executable that stays"
  references. The porting workflow's Tests follow their modules section keeps a test that
  uses only a module's public API after the module's integration, "until their
  `port/unexport` carries them over".
- The scheduling skill's Later stages section, 5.2, leaves the C tests out of an export's
  referrers. It says that "a function or global that remaining C never references has no
  export, and no `port/unexport`". Its Later stages section also says that "a stage whose
  release point the item's own port already passes is not planned".

Take a function whose last C referrer, other than the tests, is its own module or a
module ordered before it, and which a C test that stays references. Read literally, it
gets no `port/unexport`. Then either it is exported for the test and never removed, which
keeps it for good because of a test, or it is not exported and the test fails to link at
the module's integration.

The C test harness, which every C test links, is the second gap. Here it is `deps/clar`:
`main.c`'s `main` and the `clar_libgit2*.c` setup it calls. No skill says what happens
when a `port/unexport` removes an export the harness itself calls. The harness is no test
case, so 5.2's carry-over does not reach it.

## Evidence

Planner run 11 of `crustify-rs/libgit2-crustify`, in
`crustify/artifacts/planner/run-11/compose/` (`stages11.py`, `carryover11.py`):

- 181 exports have a `closure` point their own port passes, yet C test cases still
  reference them once their module is integrated. Run 11 plans their `port/unexport`
  freed by that integration alone, in 56 waves.
- Of these, 28 waves carry over no case in the plan's order: the referencing cases leave
  at the integration of another module, which the order puts first and the dependencies
  do not. Those cases are recorded as `if_still_in_c`.
- The harness's own code references 14 exports: `git_libgit2_init`,
  `git_libgit2_shutdown` and `git_error_last` from `main`; `git_libgit2_opts`, the
  `git_fs_path_*` and `git_str_*` calls of the sandbox setup; `git_trace_set`/`git_trace`
  for the optional trace hook; and `git_time_monotonic` for the perf timer. 11 waves
  remove one of them.
- Wave 1966 (`src/libgit2/libgit2.c`) removes `git_libgit2_init` and
  `git_libgit2_shutdown`. In the plan's order, 43 cases are still in `c/` then, 19 of them
  for good: they test the foundational modules, or they reference only the two exports
  the foundational C keeps (`git_error_set`, `git_error_set_oom`). Without those exports,
  the harness cannot initialise the library for them.

## Destination

The spec: the scheduling skill's Later stages section, 5.2, which the planner evolves,
and the wavefront skill's Scheduling › Later stages section. Approval is needed first,
since the `evolution` answer has no `unattended` part.

## Change

In `skills/planning/scheduling/SKILL.md`, 5.2, after "removes them from `c/`":

> An export whose release point its own port passes, but which C test cases or the test
> harness still reference once its module is integrated, is exported for them at that
> integration. Its `port/unexport` is planned in a wave freed by that integration alone,
> and carries them over. A case has left by then when it reaches the internals of the
> item's module or of a module ordered before it. The harness's own references, those of
> the code every C test links rather than of a test case, are adapted by the
> `port/unexport` that removes the export: an optional hook is dropped, and [the user's
> answer below].

In `skills/tools/wavefront/SKILL.md`, Scheduling › Later stages, after "with none left,
the stage is not planned": "unless C tests still reference the export after the item's
module is integrated, as the `scheduling` skill's Later stages section describes".

## Question for the user

What happens when a `port/unexport` removes an export the harness cannot run without? In
this campaign that is `git_libgit2_init`/`shutdown`, `git_error_last` and the sandbox
setup's `git_libgit2_opts`. Options:

1. The wave carries over every case still in `c/`, so the C suite ends there. Here that is
   43 cases at wave 1966, including the 19 that would otherwise stay for good.
2. The harness's lifecycle calls go through a narrow export kept until the C suite ends.
   That is an exception to "no export for a test", which the user asked for.
3. The cases that would stay for good, those tied only to foundational modules, are
   carried over with their modules' Rust tests at that wave. The other cases follow
   option 1.

Run 11 records the harness's references per wave (`harness` in `carryover.json`) and
decides none of these.

## Validation

`stages11.py` and `carryover11.py` implement the first paragraph of the change. No case
that stays references an export a `port/unexport` removes. No `port/unexport` is kept for
good because of a test (the only two kept are kept by the foundational modules' C).
