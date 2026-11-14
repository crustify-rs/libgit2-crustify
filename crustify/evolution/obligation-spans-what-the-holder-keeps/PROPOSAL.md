# An `unsafe` setter's obligation lasts as long as its object holds what it stored

Status: `open`: the change is crustify `1da6e44` on `libgit2-crustify/evolution/obligation-spans-what-the-holder-keeps`, made on
2026-11-14 at the user's direction after the triage of open proposals, awaiting its review.

Goal: Safety, its soundness measure.

## Signal

Review findings that recur across batches, in one objective and category (`evolution` §2):
two successive reviews of one module found the same defect in the contract of one `unsafe`
setter, an obligation whose duration ends before what safe code can still do with the value.

## Evidence

- `wave-194/batch-5/run-1`, `contracts/ffi_import/settings-ptr-admits-a-reason-that-dies-first`:
  `ParserMut::set_settings_ptr` asked for a table sound "for as long as the parser executes
  with it", and nothing of a reason its callbacks set, which the safe `ParserRef::reason` reads
  long after; fixed by asking that the reason live "while the parser holds it".
- `wave-194/batch-6/run-1`, `contracts/ffi_import/settings-ptr-bounds-a-table-by-later-safe-calls`:
  the same contract, and `set_data`'s, still bounded the table and the context by when the
  parser "executes", which the safe dispatchers make at any time; a caller who frees the table
  after its last parse reaches a use after free from safe code (ASan); fixed by "for as long as
  the parser holds it: until … replaced … or cleared, or else for as long as the parser lives".

In both, the code was right and the obligation's span was wrong: it was stated over the calls
during which C uses the value, while the safe API, not the `unsafe` caller, decides when those
calls happen.

## Destination

The spec: `binding`'s Safe boundary section, or `binding-symbols`' Globals and Function
pointers, which no reviewer evolves, and `reviewing`'s Contracts objective (§3.2), which this
role evolves, as a case to try.

## Change

In `binding` § Safe boundary, after the paragraph on `unsafe fn`s:

> The obligation of an `unsafe fn` that stores a value C keeps, such as a pointer a later
> call follows, lasts as long as C holds it, in the object or in any copy C makes of it: until
> a setter replaces it or a call clears it, the contract naming which, or else as long as its
> holders live. It is never bounded by the calls that use the value, since safe code makes
> those after the `unsafe` call has returned.

The case first drafted for `reviewing` § 3.2 is left out: that section already sends a
reviewer to `binding`'s Safe boundary, and the `soundness` skill's `hidden-dependency` pattern
gives the case.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| an `unsafe` function that stores a value its object keeps has an obligation lasting as long as the object holds it, never bounded by the calls that use it | `libgit2-crustify`, `obligation-spans-what-the-holder-keeps` | crustify `1da6e44` | an `unsafe` setter's obligation was stated as "while it executes", which safe calls made after it extend: a use-after-free from safe code, twice in one module | a `ub` or `contracts` finding of the `hidden-dependency` pattern on an `unsafe` setter whose contract bounds its obligation by a span of calls, or by one holder where the C copies the value | watched |

## Validation

None run: both findings were fixed in place, and each regression test is in `ub_unsafe`.

## Review

None yet.

## Review

`review-1` (Opus 5.5, `$1.09`), on `a3552fb`: **rejected**, one fix required: the rule bounded the
obligation by the one object that holds the value, which contradicts `hidden-dependency` where
the C copies the value into another holder. Fixed as `1da6e44`: as long as C holds it, in the
object or in any copy. Evidence, goal, placement and row held; it weakens no check. Listed
apart: the second finding's value was stored by a callback the setter installs, which the
`soundness` skill's closing rule covers.
