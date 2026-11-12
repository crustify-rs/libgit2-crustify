# ffibox goal G9 and references over immutable C bytes

Status: `open`. Changing `GOALS`-level documents needs the user's approval
(`evolution` §8), so this waits for them.

## Signal

Two of a tool's documents that disagree (`evolution` §2). ffibox's `docs/goals.md`, G9,
excludes "aliasing a foreign object through a reference" by "handles: no Rust reference
over foreign memory". `docs/borrows.md` has long lent a NUL string an object owns as a
`&CStr` / `&[u8]` tied to the owner's borrow. Since `6a43dc3` (`ffibox-usage-docs`), it
also lends a counted run of immutable bytes as `&'a [u8]`, under stated conditions:
nothing writes the bytes while the shared handle lives, and they live with the object.

## Destination

ffibox `docs/goals.md`, G9's first row.

## Change

Reword the row so that the goal states the rule the primitives keep: no Rust reference
over foreign memory that C or another handle may write while the reference lives.
Immutable runs lent under a shared handle are then the one stated exception, and every
other site stays a handle or a `CSlice`.

The alternative is to keep G9 absolute and move both cases to `CSlice` and a `CStr`
handle. That would touch the campaign's 8 `&[u8]` sites and every `&CStr` getter, and it
would cost consumers copies or handles where a reference is sound.

## Validation

None needed beyond reading: a wording change. If the alternative is chosen, ffibox's suite
plus a migration note.
