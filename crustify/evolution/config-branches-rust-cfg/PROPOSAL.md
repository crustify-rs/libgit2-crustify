# A macro whose configured branches differ, under conditions Rust's `cfg` sees, has no route

Status: `open`.

Goal: Equivalence: the Rust keeps what the C computes on every configuration the C does.
Its measure is the differences found after the comparison ended, here by a review. Also
Consistency: two runs that meet such a macro should port it the same way.

## Signal

The recurrence signal of a watched row of the record, `config-branches-of-one-result`
(crustify `769d102`): "a task message or plan correction that settles again whether a
configuration-dependent macro is ported or shimmed". The landed rule covers a macro whose
branches compute one result, which it ports once. It covers a macro whose branches differ
only where the condition is one "the Rust does not see" (`binding-symbols`' shim). It gives
no route for branches that differ under a condition Rust's own `cfg` tests, such as the
target's OS or pointer width. The translator ported such a macro once, with this build's
branch. Its review's task message asked for that judgment "above all", and the review had to
work out the route by hand.

## Evidence

`wave-219/batch-0` (`src/util/cc-compat.h`, `f570d2a4e`) ported `PRIdZ`, `PRIuZ` and `PRIxZ`
as `&str` constants holding this build's `"zd"`, `"zu"` and `"zx"`, for every target. Its
docs said the header's other branches, `"I64d"` / `"Id"` and the rest under `_MSC_VER` or
`__MINGW32__`, "format the same values". Its module pass, `wave-219/batch-1`, filed
`misc/size-formats-ported-with-one-branch`, as its review reads the spec:

- `porting-symbols`' Macros section ports a configuration-dependent macro once "where every
  configuration's expansion computes the same result for every input". The strings differ,
  so that sentence does not apply. Its next sentence, "Only one whose branches differ in what
  they compute keeps the shim `binding-symbols` gives it", then sends the macro to a shim.
- `binding-symbols`' Macros section gives the shim only to a macro whose configuration "the
  Rust does not see". `cfg(windows)` and `cfg(target_pointer_width = "64")` see these
  conditions. The constants are not callable either, so `crustify_<NAME>` does not fit them.

Neither section says what such a macro becomes. The review reimplemented every branch, one
item per branch under the `cfg` that stands for its condition, which reproduces the header's
expansion on every Windows and non-Windows target. It checked this with
`clang --target=<t> -E` against a constant assertion per target.

## Destination

The spec: `porting-symbols`' Macros section (`skills/rust/porting/symbols/SKILL.md`), not a
protected path. The reviewer does not evolve `porting-symbols`, so the change is proposed.

## Change

In the Macros section's second paragraph, after "and its documentation names the branches
the build does not take.", insert:

> One whose branches compute different results under conditions Rust's own `cfg` tests,
> such as the target's operating system or pointer width, is ported with every branch, each
> under the `cfg` that stands for its condition, so that it computes on each target what the
> C computes there.

The paragraph's last sentence, "Only one whose branches differ in what they compute keeps the
shim", then reads "Only one whose branches differ under a configuration the Rust does not see
keeps the shim".

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro whose configured branches differ under conditions Rust's `cfg` tests is ported with every branch under its `cfg`; only one whose configuration Rust does not see keeps the shim | `libgit2-crustify`, `config-branches-rust-cfg` | — | a port took one branch of a macro whose branches differ by target OS, and its review worked out by hand that the rule gave no route | a review finding of a macro ported with one branch of several that differ, or a task message or plan correction that settles how such a macro is ported | — |

The row `config-branches-of-one-result` is `recurred` by this proposal's evidence.

## Validation

Not run as a rerun of the batch. The review's fix is the change applied to the item its
evidence came from: on `wave-219/batch-1`'s tree, the constants match the header's
preprocessed expansion for `x86_64`/`i686` `pc-windows-msvc` and `pc-windows-gnu`, and for
`x86_64-unknown-linux-gnu`. On `f570d2a4e`, `x86_64-pc-windows-gnu` fails the same check.

## Review

None yet.
