# ffibox documentation that led agents to hand-write what ffibox covers

Status: `landed`, as ffibox `6a43dc3`, `1b3308b` and `d0caf20` on `devel`, with the user's
approval of the usage audit's follow-up 3 (2026-11-12).

## Signal

An agent prompt or document that leads its agent wrong (`evolution` §2). The usage audit
of 2026-11-12 found four places where ffibox's documentation produced hand-written code or
a defect.

## Evidence and change

1. **Lifetime-carrying layout types.** `docs/hand-written.md` sent every such type to the
   hand-written layer. Agents carried that over to the types' policies: 21 `CDrop` and 6
   clone policies, beside 34 hand-written `CCell` impls.
   - `impl_cdrop!`, `impl_crefclone!` and `impl_cdispose!` take `Blob<'_>`, which
     implements the trait for every lifetime. `impl_cdupclone!` and `impl_cguarded!` do
     not.
   - `tests/lifetime_policies.rs` drives `CBox`, `CArc` and `CVal` over a type that
     borrows a local. Miri passes.
   - The docs, the macro rustdoc and a compiled example in `examples.md` say this. The
     macro change for the other two is part of `ffibox-lifetime-ctypes`.
2. **The `unsafe fn` adapter.** Agents avoided it because the old `crustify scan-unsafe`
   counted it as 4 smells.
   - A probe with the current ffimap (`9aa2ef1`, `--regions`) shows that an adapter in the
     module's unsafe region adds no Safe boundary site. It does add one private `unsafe fn`,
     one raw pointer in a private unsafe signature, and one unsafe block and line.
   - The docs now prefer the adapter, placed beside its policy.
3. **No migration note for `CVec` dropping its elements** (`936204b`). This was the
   `OdbDataStrVec` double free, fixed by `5d0e779f8`.
   - A new `CHANGELOG.md` covers it, beside `CDupClone<c_char>`'s new contract and the
     stateful-policy check.
   - `CVec`'s rustdoc and `ownership.md` warn against dropping the elements twice.
4. **Immutable counted byte runs.** `docs/borrows.md` was silent on them, and eight
   campaign sites returned `&'a [u8]`, each justified in its own SAFETY comment.
   - `&'a [u8]` is prescribed under three stated conditions: nothing writes the bytes while
     the shared handle lives, they live as long as the object does, and they are
     initialised. A NULL or empty run returns `&[]`.
   - Otherwise the run is a `CSlice<'a, u8>`. No primitive was added. The wording question
     this raises against goal G9 is `ffibox-goal-g9-immutable-borrows`.

## Validation

- ffibox's suite: 240 passed, including the compiled examples and new doctests.
- Miri on `tests/lifetime_policies.rs`.
- clippy, `fmt --check`, and `cargo doc -D warnings` are clean.
- Each new compile_fail was checked to fail for the reason it states.
- `src/` changes are doc comments only.
