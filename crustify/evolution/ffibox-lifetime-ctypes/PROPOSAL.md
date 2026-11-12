# `define_ctype!` for layout types that carry a lifetime

Status: `open`, proposed after the ffibox usage audit of 2026-11-12. The user approved
filing it; it awaits their decision on the design at a module's end.

## Signal

One representation hand-written in several places (`evolution` §2). `define_ctype!` declares
a layout type, its `Ref` / `Mut` handles, its `CCell` impl and their seams in one line, but
only for a type without generic parameters. ffibox's `docs/hand-written.md` sends every
layout type that carries a lifetime to the hand-written layer. libgit2 has many of these:
an object borrowing its repository, an options struct borrowing its strings, a trailer
borrowing its message.

## Evidence

- `src/` of `crustify-rs/libgit2-crustify` at `fd0120c5e` holds 34 hand-written
  `unsafe impl CCell for X<'_>`. In 20 of them the layout type is a
  `pub struct X<'r>(ffi::…, PhantomData<…>)`: `Reference<'r>` (`refs.rs:164`),
  `Object<'r>` (`object.rs:231`), `Blob<'r>` (`blob.rs:457`), `AnnotatedCommit<'r>`,
  `Transaction<'r>`, `OidShortener<'s>`, `InitOptions<'o>`, `MessageTrailer<'t>`, and three
  each in `tree.rs` and `commit.rs`. The audit counted 36 such types, beside 49
  `define_ctype!` uses.
- Each one restates `define_ctype!`'s expansion by hand, about 70 lines:
  - the `#[repr(transparent)]` layout type;
  - its `Ref<'a, 'r>` / `Mut<'a, 'r>` over `CBorrowedPtr`, with the `PhantomData` that makes
    `Mut` invariant;
  - `unsafe impl CCell`, `CRefSeam` and `CMutSeam`; and
  - the `Debug` impls.

  `refs.rs:154–230` is a typical one.
- Every hand-written copy is a site where `CCell`'s layout promises (`Ref` / `Mut`
  transparent over `CBorrowedPtr`, `Mut` invariant, no `Drop`) can go wrong. The type
  reviews checked them one by one. The audit found agents carried the same rule over to
  the lifecycle policies of these types, writing 21 `CDrop` and 6 clone policies by hand.
  The docs half of that is the `ffibox-usage-docs` evolution change.
- Ported modules keep handing out handles to C, so the pattern continues past the wrap
  era.

## Destination

ffibox: `define_ctype!` (`src/macros.rs`), its rustdoc, the README's and lib.rs's tables,
and `docs/hand-written.md`'s scope sentence. The `ffibox` tool skill's Binding types
section needs no change, since it already points at `define_ctype!`.

## Change

Let `define_ctype!` take lifetime parameters on the layout type, carried into its handles
after the borrow's own lifetime:

```rust
define_ctype!(
    /// A reference, borrowing its repository.
    Reference<'r>, ReferenceRef, ReferenceMut, ffi::git_reference,
    phantom = PhantomData<(*const (), &'r ())>
);
// expands to Reference<'r>, ReferenceRef<'a, 'r>, ReferenceMut<'a, 'r> with 'r: 'a,
// CCell for Reference<'r>, the seams and Debug, as the hand-written ones are.
```

- Lifetimes only: a type parameter stays hand-written, since its layout promise depends
  on the parameter.
- The phantom marker is the caller's, since variance and auto traits depend on it. This
  includes the `*const ()` many sites use to opt out of `Send`/`Sync`.
- `compile_fail` tests pin the handles' variance, matching the existing tests of
  `define_ctype!`. The macro's size assertions apply unchanged.

Migration is optional and per module: a hand-written type and a macro one are the same
layout. In a port campaign most of these types' modules are ported, so the campaign should
not sweep them. New wrappers and ported modules' exported handles use the macro.

## Validation

ffibox's suite, with tests that build a lifetime-carrying type through the macro and
check that:
- its handles' variance and invariance match the hand-written `Reference` triple;
- `compile_fail` cases cover a `Mut` outliving its borrow; and
- the handles keep the same size.

Then rebuild one campaign module with a type moved to the macro, such as
`transaction.rs`'s `Transaction<'r>`, with its tests unchanged.
