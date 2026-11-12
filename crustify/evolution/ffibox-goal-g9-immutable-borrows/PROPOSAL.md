# ffibox goal G9 and references over immutable C bytes

Status: `landed`, as ffibox `efe1be9` on `devel`. The user approved it on 2026-11-12 and
refined it: the hazard is a reference that something else can alias while it lives, not
any reference.

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
over foreign memory that anything may write or free while the reference lives (C, another
handle, another thread), and no `&mut` over memory anything else may access.
Immutable runs lent under a shared handle are then the one stated exception, and every
other site stays a handle or a `CSlice`.

The alternative is to keep G9 absolute and move both cases to `CSlice` and a `CStr`
handle. That would touch the campaign's 8 `&[u8]` sites and every `&CStr` getter, and it
would cost consumers copies or handles where a reference is sound.

The wording covers single-threaded writes too, such as a later call through the same
handle or a re-entering callback, and writes by C through its own pointers, not only by
another Rust reference: a `&T` asserts that no one writes the memory while it lives. The
README's mental model and `borrows.md`'s convention follow; `lib.rs`'s rule, about wrapped
objects, stands.

## Validation

None needed beyond reading: a wording change. If the alternative is chosen, ffibox's suite
plus a migration note.
