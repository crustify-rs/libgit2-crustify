# Adopting a handle over a layout type with a lifetime vouches for nothing about that lifetime

Status: `open`: no change made. The destination is ffibox, which a reviewer proposes changes
to and does not evolve.

## Signal

A contract that a caller can honour and still reach undefined behaviour (`reviewing`'s
`contracts` objective), stated once in ffibox and inherited by every layout type with a
lifetime: `define_ctype!`'s 3 uses with `phantom =` and the crate's 34 hand-written
`unsafe impl CCell for X<'_>`.

## Evidence

- `CCell::ref_from_ptr` (`src/traits.rs:907`), which `CRefSeam::from_ptr`,
  `CMutSeam::from_ptr`, their `void *` twins and `CCell::mut_from_ptr` forward to, asks only
  that "`ptr` must address a live, initialised `Self::C` (or be null) that outlives `'a`, and
  nothing may write it while the handle lives except through a `Mut` derived from the same
  borrow". `'a` is the handle's borrow. For `Self = Foo<'p>`, the getters of `FooRef<'a, 'p>`
  lend what the object points at for `'p`, and nothing in the contract says `'p` must hold.
  The caller names `'p`, `'static` included.
- `wave-186/batch-3`'s finding
  `contracts/ffi_import/adopted-handles-outlive-their-borrows`
  (`crustify/reviews/modules/git2/src/cli/opt.c/wave-186/batch-3/`): `SpecRef::<'_, 'static>::from_ptr`
  on a live `cli_opt_spec` keeps the stated contract word for word. `name()` then lends a
  `&'static CStr` that outlives its `CString`. Miri reports a dangling reference
  (`run-affected.log` there). opt.rs's getters justify their reads by "its handle's adoption",
  which no stated contract defines. The batch fixed it on its side, with an `# Adoption`
  section on each of `Spec`, `Opt` and `Parser` stating what an adopter vouches beyond the
  seams' contract.
- The same holds wherever a lifetime cell is adopted from a pointer: `Reference<'r>` (`refs.rs`),
  `Object<'r>`, `Blob<'r>` and the rest of `ffibox-lifetime-ctypes`' count. `Holder<'p>`'s
  example in `impl_cdrop!`'s rustdoc (`src/macros.rs`) writes the obligation into the
  `SAFETY:` comment of its `CBox::from_c` call ("an owned holder whose parent outlives
  `'p`"), but no `# Safety` section asks for it.

## Destination

ffibox: the `# Safety` sections of `CCell::ref_from_ptr`, `mut_from_ptr`, `ref_from_void_ptr`
and `mut_from_void_ptr` (`src/traits.rs`), which the seams' `# Safety` sections cite, and of
the other adoptions of a `Self` value from a pointer (`CSlice::from_raw_parts`, `CBox::from_c`
and the owners' `from_raw`). Also the `define_ctype!` rustdoc's `phantom` paragraph, and
`docs/borrows.md`'s inbound-seam paragraph.

## Change

Add one clause to each of those contracts: the pointee is a valid `Self`, not only an
initialised `Self::C`. Every borrow the lifetimes of `Self` name lives for them, and every
invariant the type's safe operations rely on holds, as the type's own documentation states
it. Say in `define_ctype!`'s `phantom` paragraph that a type whose layout points at memory
it borrows documents that invariant, for its adopters, in an `# Adoption` section, as
opt.rs's `Spec`, `Opt` and `Parser` now do. The wording only makes explicit what the
existing `SAFETY:` examples already assume. No signature changes.

## Validation

Not made. The reproducer above fails under Miri at `fe6b85bfa`. With opt.rs's `# Adoption`
sections, the batch's `tests/ub_unsafe/src/cli/opt.rs` adopts each handle within the
extended contract, and Miri and ASan pass it.

## Review

None yet.
