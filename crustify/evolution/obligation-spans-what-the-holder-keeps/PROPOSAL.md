# An `unsafe` setter's obligation lasts as long as its object holds what it stored

Status: `open` (filed by `wave-194/batch-6/run-1`; no change made).

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

Not made. In `binding` § Safe boundary, after the paragraph on `unsafe fn`s:

> The obligation of an `unsafe` function that stores a value its object keeps, a pointer a
> later call follows, lasts for as long as the object holds the value: until a setter replaces
> it or a re-initialiser clears it, naming which, or else for as long as the object lives. It
> is never bounded by the calls that use the value, which safe code makes after the `unsafe`
> call has returned.

In `reviewing` § 3.2, beside the case of an obligation stated on code C calls: "an obligation
whose duration is a span of calls safe code can extend, such as \"while it executes\", rather
than what the object holds".

## Validation

None run: both findings were fixed in place, and each regression test is in `ub_unsafe`.

## Review

None yet.
