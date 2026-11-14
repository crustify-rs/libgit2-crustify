# A callback can re-enter the function that calls it, through the object it is handed

Status: `open`: the change is crustify `6ad1ea5` on `libgit2-crustify/evolution/callback-reenters-its-caller`, made on
2026-11-14 at the user's direction after the triage of open proposals, awaiting its review.

Goal: Safety, its soundness measure.

## Signal

A rule the how-to skills lack, which a binding had to work out by hand (`evolution` §2).
`binding-symbols`' Function pointers section has a trampoline refuse "a re-entrant call,
which would alias an `FnMut` closure": the hazard it names is the closure's state. It says
nothing of the function that runs the callback, whose own state the callback can reach when
C hands it the object the function is working on. Re-entering that function, or resetting
the object, from the callback is then undefined behaviour in C, reachable from safe Rust.

## Evidence

`wave-194/batch-1/run-1` (`deps/llhttp/llhttp.c`'s `llhttp__internal_execute`, bound as
`ParserMut::parse`). The parser's settings name callbacks C calls with the parser being
parsed, so a Rust handler receives that parser's exclusive handle, from which a safe API
offers `parse` and `reinit`. A nested `llhttp__internal_execute` on the same parser sets
`_span_pos0` into the nested input and moves `_index`; the outer run, resuming after the
callback, then hands a span callback `(state->_span_pos0, p)` across two allocations (a
wild length), and `llparse__match_sequence*` index their blobs with the nested `_index`.
`llhttp__internal_init` from a callback nulls `_span_pos0`, which a span-end state reads
back as its start. The batch refuses both with a panic, through a thread-local register of
the parsers each thread is executing, which the trampolines' barrier turns into the
callback's error (`tests/ub_unsafe/deps/llhttp/llhttp.rs`,
`a_handler_cannot_parse_the_parser_whose_parse_runs_it`,
`a_handler_cannot_initialise_the_parser_whose_parse_runs_it`).

## Destination

The spec: `binding-symbols`, Function pointers, beside the trampoline's re-entry rule; and
`analysis`' Findings, which should list it as something the analysis establishes.

## Change

Not made yet. In `binding-symbols` § Function pointers, after the paragraph on the
trampoline's re-entry refusal:

> A function that calls a callback with an object it is working on lends that object to
> the callback. The analysis establishes which of the object's routines C allows from
> within the callback; a routine it does not, the function itself or one that resets the
> state the function resumes from, is refused there by the binding, on the thread that
> runs the function, since the callback holds an exclusive handle the borrow checker
> cannot tell from any other.

In `analysis` § Findings, under states and transitions: "the calls a callback may make on
the object its caller lends it, and those that would corrupt the caller's state".

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a function that hands a callback the object it is working on lends it; the analysis establishes which of the object's routines C allows from the callback, and the binding refuses the others there | `libgit2-crustify`, `callback-reenters-its-caller` | crustify `6ad1ea5` | a callback could re-enter or reset the function running it through the object it was handed, C undefined behaviour reachable from safe Rust | a `ub` finding of the `callback-state` pattern where a callback calls a routine of the object its caller lent it | watched |

## Validation

None run: the rule is the one this batch followed; a rerun of the batch under it would
produce the same binding. The batch's two UB tests above, run under ASan+UBSan, Miri (they
stop at the foreign call) and BSan alone, are its evidence that the refusal holds.

## Review

None yet.
