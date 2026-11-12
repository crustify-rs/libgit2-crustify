# Releasers that free a container's elements with it

Status: `open`

## Signal

A rule the binder had to work out by hand that a skill and a tool's documentation should
have stated, and a documented ffibox pattern that is unsound as written.

ffibox's `docs/ownership.md`, in Choosing an owner, maps a counted array "freed with its
elements by one routine (`foo_array_free(arr, n)`)" to "the bare pointer or `Foo`, with that
routine as the `CLenDrop`". Followed literally, that owner is `CVec<*mut c_char, P>` with
`P::c_drop_len` calling the routine:

- a raw pointer is `CPlainElem`, so the `CVec` dereferences mutably to `&mut [*mut c_char]`,
  and safe code can store any pointer, or NULL, in a slot the routine then frees; and
- `CLenDrop`'s contract lets `c_drop_len` touch at most `byte_len` bytes of the buffer and
  must hold for every buffer a `CVec` can carry, so a public policy whose release reads and
  frees its elements is also reached by `CVec::from_raw_parts` of any element type, under a
  contract that says nothing of what the elements hold.

`binding-symbols`' Lifecycle strategies section says an item with a lifecycle role "becomes
a generic policy or strategy that calls the primitive", but not how a primitive that releases
a container's elements as well as the container keeps that sound.

## Evidence

- `crustify-rs/libgit2-crustify`, wave 184 batch 0 (`src/util/strlist.c`, foundational,
  run 1): `git_strlist_free(strings, len)` frees `len` strings and then the array, and
  `git_strlist_free_with_null(strings)` every string before the NULL terminator and then the
  array. Both are lifecycle roles the task asks to bind as strategies that call them.
- The batch bound them as `StrListFree` / `NullStrListFree` (`src/libgit2/src/util/strlist.rs`):
  each a `CLenDrop` policy with a private field, so no code outside the module makes a value
  of it, held in a private `CVec<ManuallyDrop<AllocStr>, P>` behind the owners `StrList` and
  `NullStrList`. The elements are the strings' own owners (`AllocStr`, `CStrBox` over
  `git__free`), which the `CVec` never drops, and the owners dereference to `&[AllocStr]` /
  `&mut [AllocStr]`: a slot can only ever hold another string of the allocator the routine
  frees. Its compile-fail doctests show the policies cannot be built outside the module.
- `OdbDataStrVec` (`src/libgit2/src/libgit2/odb.rs`, an earlier wrap campaign) met the same
  shape for `git_strarray`'s strings by dropping each element through its owner and then
  the array, so it never calls the elementwise routine at all.

## Destination

- ffibox (a tool's documentation, possibly a primitive): `docs/ownership.md`'s row, and
  `CLenDrop`'s rustdoc.
- The spec: `skills/rust/binding/symbols/SKILL.md`, Lifecycle strategies (the binder's to
  evolve), once the ffibox side is settled, since the rule names what ffibox offers.

## Change

ffibox, `docs/ownership.md`, the "freed with its elements by one routine" row: the element
type is the elements' own owner (`CStrBox<P>`, `CBox<Foo, P>`) wrapped in `ManuallyDrop`, so
that the `CVec` drops none of them and the routine frees each; the `CVec` is private to a
newtype owner that gives the elements out as `&[Owner]` / `&mut [Owner]`; and the policy
carries a private field, so that its `c_drop_len` only ever receives the buffers that owner
builds. Add to `CLenDrop`'s safety section that a policy whose release reads the buffer's
contents must not be constructible where a `CVec` of another element type could be built
over it. A primitive for the shape, a `CVec` whose policy releases its elements (say, an
element marker that `CLenDrop` impls can require), would remove the hand-written newtype; it
is ffibox's to judge against its goals.

`binding-symbols`, Lifecycle strategies, after "A family's allocator, releaser and cloner
form one policy.":

> A releaser that frees a container's elements with the container is held by an owner
> whose elements are the elements' own owners, which the container never drops itself, and
> whose policy no other code can instantiate, so that the releaser only ever receives what
> that owner built and its elements only ever hold what the releaser can free.

## Validation

Not rerun. The batch's tests exercise the shape under ASan/UBSan and Miri: elements
replaced, swapped and sorted between lists and other owners, lists cloned, adopted from C
and released in every order (`tests/ub_safe/src/util/strlist.rs`,
`tests/ub_unsafe/src/util/strlist.rs`), and the release counts checked against a counting
allocator (`strlist.rs`'s `lists_request_and_release_what_strlist_c_does`).

Reviewed in wave 184 batch 1 (the module pass), against every element operation the owners
expose: the mutable views (assignment, `swap`, `mem::replace`, `sort`, `fill`, iteration)
only ever exchange one `AllocStr` for another, so a slot holds a non-NULL string of
`git__free`'s family whatever safe code does; `try_clone` reads the elements as `&CStr`;
`into_raw_parts` / `into_raw` give the array up whole; and no value of either policy reaches
code outside the module. One gap, in the adoptions rather than the shape: `from_raw_parts` and
`from_raw` did not ask the adopted elements to be distinct strings, and a release that frees
each element then frees a repeated one twice (finding
`contracts/ffi_import/adoptions-admit-repeated-strings`, fixed in the contracts). The
rewording of `docs/ownership.md`'s row and `CLenDrop`'s rustdoc proposed above should carry
that obligation for any adoption of such a container.
