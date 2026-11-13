# `Box` and `Vec` over a C allocator: ffibox adopts the standard `Allocator`

Status: `landed`, as ffibox `f944c17` … `4fa703c` on ffibox's `libgit2-crustify/devel` (2026-11-13), once review-2 approved it.
`libgit2-crustify/evolution/allocator-api`, forked from this campaign's working branch of
ffibox, `libgit2-crustify/devel`, at `7af1232`: `f944c17`, `758eaaf`, `155b9df`, `bd7ad37`,
`e69a71c`, and `4fa703c` with the first review's fixes. Not landed.

## Signal

The user's direction, with a goal no decision has cited yet (`evolution` §2: the user's
corrections; a goal no decision cites), and a TODO met again and again:

- Rust 1.100.0 (2026-11-09) stabilized the core of `allocator_api`: `core::alloc::Allocator`,
  `AllocError`, `Box::{new_in, from_raw_in, from_non_null_in, into_raw_with_allocator,
  into_non_null_with_allocator, allocator}`, `Vec::{new_in, with_capacity_in,
  from_raw_parts_in, into_raw_parts_with_allocator, …}`. The user decided ffibox adopts it,
  raising its minimum Rust to 1.100 rather than gating it behind a feature.
- ffibox's goals name it: G17 "It uses the standard `Allocator` interface where a primitive
  needs a non-global allocator", and G1 "each primitive converts to and from its standard
  counterpart (`Box`, `Vec`, …)". Until 1.100 neither could be met on stable: no primitive
  implemented `Allocator`, and no owner converted to a `Box` or `Vec` over C's allocator.

## Evidence

- The `ffibox` tool skill's Porting types › Structs TODO
  (`skills/tools/ffibox/SKILL.md:111`): "ffibox's owners hold only `CCell` layout types, so
  an owner over C's allocator of a struct whose layout is native, between its `port/layout`
  and its `port/memory`, is hand-written until ffibox has one". `Box<Foo, CAllocator<P>>`
  is that owner.
- Each of the campaign's C allocation families is written twice by hand, once as a box
  policy and once as a buffer policy, at `crustify/devel` `fde60bca2`:
  - `src/libgit2/src/util/alloc.rs`: `Free` (`CDrop<T>` :769, `CAlloc<T>` :784,
    `CAllocZeroed<T>` :804) beside `ArrayFree` (`CLenDrop` :864, `CLenAlloc` :882,
    `CLenClone` :901);
  - `src/libgit2/deps/reftable/basics.rs`: `ReftableFree` (:1075, :1090, :1107) beside
    `ReftableArrayFree` (:1155, :1168, :1182);
  - `src/libgit2/src/util/allocators/failalloc.rs`: `FailAlloc` (:145, :163) beside
    `FailArrayAlloc` (:239, :257).
- Growth of a C-allocated run is hand-written over `CVec`: `ArrayFree::resize`
  (`alloc.rs:1004`), `resize_bytes` (:1031), and reftable's `resize` / `grow` / `grow_with`
  (`basics.rs:1240`, :1297, :1323), which is what `Vec<T, CAllocator<P>>` does through the
  family's `realloc`.

## Destination

ffibox: a new `allocator` module, a provided method on `CLenAlloc`, a policy macro, the
conversions, the docs (`docs/ownership.md`, `docs/outbound.md`, the README, the crate docs,
the CHANGELOG) and the manifest. The `ffibox` tool skill's TODO above is Crustify's, for the
assistant to replace once this lands: between a struct's `port/layout` and its
`port/memory`, its owner is `Box<Foo, CAllocator<P>>` over the C family's policy.

## Change

1. **MSRV 1.100** (`f944c17`): `rust-version = "1.100"`, stated in the README; the
   manifest drops `homepage`, which cargo 1.100 reports as redundant with `repository`;
   `vtable.rs`'s entry counter uses `AtomicUsize::try_update`, which its comment said was
   newer than the minimum. ffibox has no CI files, and `docs/goals.md` states no version
   number (G17 "with a declared minimum version"), so neither changes.
2. **A C allocation family as the standard `Allocator`** (`758eaaf`). A `CLenAlloc` policy
   already is a family's `malloc` and alignment, and its `CLenDrop` its `free`; `CLenAlloc`
   gains a provided `c_realloc_len` (`c_alloc_len`, a copy and `c_drop_len`), which a family
   with a `realloc` overrides. `impl_clenalloc!(P, malloc, align = N, realloc = r)` binds
   them beside `impl_clendrop!`. `CAllocator<P>` (`#[repr(transparent)]` over `P`) is then:
   - `unsafe impl<P: CLenAlloc + Copy> core::alloc::Allocator`: zero-sized requests never reach C
     (`Layout::dangling_ptr`; `deallocate` of size 0 does nothing; `grow` from 0 allocates,
     `shrink` to 0 frees); alignment beyond `P::ALIGN` is `AllocError`; `grow` / `shrink`
     go through `c_realloc_len`, the old block untouched on failure; `grow_zeroed` zeroes
     past the old size; no method unwinds (an abort guard around the policy's routines);
     every block is exactly the size requested, so a sized free gets it back; the
     policy is `Copy` (`4fa703c`), so the allocator has no drop glue, which std forbids to
     unwind: a bound the compiler checks, chosen over a contract on `CLenAlloc`, which
     would add an obligation to every existing impl; every family in use is a unit
     struct, and `CAllocator<P>` still takes any `P` as a policy of `CBox` / `CVec`;
   - a documented relaxation of the standard *currently allocated* and *fits*, grounded
     on ownership (`4fa703c`): a non-empty buffer of the family whose ownership has passed
     to the caller, C's included once C gave it up, is allocated by every `CAllocator<P>`
     until the caller frees, moves or returns it, and a layout fits it when its alignment is at most `P::ALIGN` and its size is non-zero
     and one `c_drop_len` accepts for it (at most the allocation, exactly it under a sized
     free). That is what makes `Vec::from_raw_parts_in(ptr, len, len, a)` sound for a
     buffer whose capacity C never recorded, under a free that ignores the size;
   - a storage-only policy for `CBox` (`CDrop<T: CCell>`, `CDrop<MaybeUninit<T>>`,
     `CAlloc`, `CAllocZeroed`, `CNew` for `CZeroable`), `CStrBox` (`CDrop<c_char>` /
     `CDupClone<c_char>` with `strlen + 1`) and `CVec` (forwarding `CLenDrop`, `CLenAlloc`,
     `CLenClone` to `P`), mirroring `RustAlloc`, so a migration swaps one for the other;
     zero-sized and over-aligned objects refused at compile time;
   - `try_box(self, value) -> Result<Box<T, Self>, AllocError>`: std's unstable
     `Box::try_new_in`, since G11 makes allocation failure a value.
   Zero-sized and `Copy` over a unit-struct policy (a `Box` over it is one pointer, checked
   in the tests); `Send` / `Sync` follow `P` (a compile-fail doc test for a thread-bound
   family).
3. **Conversions** (`155b9df`), where the owner's policy is the allocator, lossless and
   without copying:
   - `impl From<Box<T, CAllocator<P>>> for CBox<T, CAllocator<P>>` (`T: CCell`), and
     `unsafe fn CBox::into_box(self) -> Box<T, CAllocator<P>>`. Not `From`: the orphan rule
     refuses `impl From<CBox<..>> for Box<T, ..>` (`Box` is `#[fundamental]`, E0210); and
     `unsafe`, since a `Box` lends `&mut T`, through which safe code moves the bytes of an
     address-sensitive C object (`mem::swap`) that a `CBox` built by `new_uninit` +
     `assume_init`, or adopted by `from_raw`, never vouched could move, and copies or moves
     the `T` out (`*b`), which needs its bytes to be a valid `T` where C may have left a
     field unwritten or written a `bool` of 2 (`4fa703c`, from the first review's Miri
     probe). No ffibox type says either (`CZeroable` implies only the first), so the caller
     does (G4's documented case).
   - `impl From<CVec<T, CAllocator<P>>> for Vec<T, CAllocator<P>>` (`T: CSliceElem`, the
     elements a `CVec` already hands out as `&mut [T]`); a buffer of C objects stays behind
     handles.
   - `impl TryFrom<Vec<T, CAllocator<P>>> for CVec<T, CAllocator<P>>` (any `T`), its error
     the `Vec` unchanged, as std's `TryFrom<Vec<T>> for [T; N]`.

   The empty / zero-size rule, enforced by the safe API: zero-sized `T` and `T` aligned
   beyond `P::ALIGN` are refused at compile time on every path; an empty `Vec` (its
   dangling pointer) becomes the NULL `CVec`, its capacity freed if it had any; an empty
   non-NULL `CVec` (`malloc(0)`) is freed on the way to a `Vec`, which would leak it at
   capacity 0; a `Vec`'s spare capacity is given back through `realloc` before it becomes a
   `CVec`, which records only its length, so a sized free gets the buffer's size.
4. **Docs** (`bd7ad37`, `e69a71c`): `docs/ownership.md` gains "`Box` and `Vec` over a C
   allocator" (declaring a family; when the standard owners hold a value and when ffibox's
   do: C keeps aliasing it, a lifecycle routine rather than `free` releases it, a C object
   reached through handles, a counted object or a string while std's `Arc::new_in` and
   allocator-carrying `String` are unstable, a pinned object while `StaticAllocator` is;
   the crossing rules; the allocator's), rows in its owner and policy tables, a row in
   `docs/outbound.md`, the README's and the crate docs' pieces, and the CHANGELOG entry
   with the new minimum. `CAllocator` names `System` as its standard counterpart (G1).

Nothing existing changes behaviour: the provided `c_realloc_len` adds no obligation to an
existing `CLenAlloc` impl (its default is sound by `c_alloc_len`'s and `c_drop_len`'s
contracts); the obligation stated for an override binds only an impl that overrides it.

Not done, and why:

- `CArc` ↔ `Arc`, `CStrBox` ↔ `String` / `CString`: std's allocator constructors for them
  are not stable (`Arc::new_in`, `String` with an allocator).
- `CBox<T, RustAlloc>` ↔ `Box<T>` (`Global`): lossless for objects (same layout), but
  outside the task; `CVec<T, RustAlloc>` ↔ `Vec<T>` is not lossless (`RustAlloc` aligns
  buffers to 16, a `Vec` to `align_of::<T>()`).
- `Pin<Box<T, CAllocator<P>>>`: `Box::into_pin` / `pin_in` need the unstable
  `StaticAllocator`.
- A `CLenClone` for `CAllocator<P>` over a `P` without one: a copy of a non-NULL empty
  buffer has no allocation in a family that never allocates 0 bytes, so it is forwarded
  only where `P` has its own.

## Validation

At `4fa703c` (the Miri full-suite and downstream rows at `e69a71c`, which `4fa703c` changes
only in bounds, docs and tests), on the host's shared toolchains, `CARGO_BUILD_JOBS=32`,
`RUST_TEST_THREADS=32`.

| command | result |
|---|---|
| `cargo +1.100.0 test` (whole suite, doctests included) | 334 passed, 0 failed, 33 ignored; at `7af1232` 292 / 0 / 32: the new 33 tests in `tests/allocator.rs` and 9 doctests (8 `compile_fail`), 1 more ignored example |
| `cargo +nightly-2026-10-08 test` | 333 passed, 1 failed, 33 ignored; the failure is `shared::CArc::lock`'s `compile_fail` doctest, which fails the same way at `7af1232` (291 / 1 / 32) |
| `cargo +1.99.0 build` | refused: "ffibox@0.1.1 requires rustc 1.100" |
| `cargo +{1.100.0,nightly} test --no-default-features [--features alloc]` | the lib builds; the integration tests fail to build with the same 21 / 5 errors as at `7af1232` (existing tests need `RustAlloc` / `CPayload`); `tests/allocator.rs` is `#![cfg(feature = "alloc")]` and adds none |
| `cargo +1.100.0 clippy --all-targets -- -D warnings`; `clippy --lib -- -D warnings` with `--no-default-features` and with `--features alloc` | clean |
| `cargo +1.100.0 fmt --check`, `cargo +nightly-2026-10-08 fmt --check` | clean |
| `cargo +1.100.0 doc --no-deps` (default and `--no-default-features`), nightly `--all-features --cfg docsrs` | no warnings |
| `MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-08 miri test --test allocator`, and with `-Zmiri-tree-borrows` | clean under both models: 32 passed, 1 ignored (the abort-guard test, which spawns a process) |
| the same at `e69a71c` with `--test vtable --test rust_alloc --test cvec_empty --test smart_pointers` | clean: 30, 6, 8, 28, 21 passed under both models |
| `MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-08 miri test --tests` | 27 binaries, 250 passed, 0 failed, 2 ignored (the two that spawn cargo or open a file) |

`tests/allocator.rs` runs a stand-in C family of `extern "C"` routines over
`std::alloc::System` that counts every call and refuses, rather than performs, a `free` of
a pointer it never returned; each test checks on exit that nothing is live, no `malloc(0)`
was made, no dangling pointer was freed and no sized free got the wrong size. A control test
shows the detector catches a dangling free. Mutating the allocator to pass size 0 through
(`allocate` / `deallocate`) fails 4 tests; removing the conversions' empty-buffer branches
fails 2 (a dangling pointer handed to C; a leaked `malloc(0)` buffer). `4fa703c` adds: a
family whose `malloc` panics, run in a child process, must die of SIGABRT (removing the
`no_unwind` guard from `allocate` fails it: the child exits 101 after unwinding); a buffer
C allocated larger than the length it hands over, adopted by a `Vec` directly and through a
`CVec`; an emptied `Vec` under a sized free, which gives back its buffer with its size.

Downstream, the campaign at `crustify/devel` `fde60bca2` (already on Rust 1.100.0, so its
`rust-toolchain.toml` needed no edit), from a `git archive` under the ffibox worktree's
`target/downstream`, its `ffibox` dependency and `[patch]` pointed at the worktree, `c` and
`crustify/builds` linked to the campaign's, `git init`ed:
`crustify/scripts/cargo-variant.sh plain test --workspace --no-fail-fast`: 2,759 passed,
0 failed, 1 ignored (`blob::unit_tests::oversized_slices_are_counted_in_rust`, ignored by
design), every `ub_safe` `compile_fail` case matching. The same tree against ffibox at the
fork point `7af1232` (a second copy, its ffibox an archive of `7af1232`) gives the same
2,759 / 0 / 1, so the change makes no difference. The five `ub_safe` snapshot mismatches and
three `map_unwrap_or` lints expected on the older tree do not appear on `fde60bca2`, with or
without the change (its `53786c97e` took clippy 1.100's lints);
`cargo-variant.sh plain clippy --workspace --all-targets` is clean in both copies but for
the `package.homepage` manifest warning ffibox at `7af1232` raises and `f944c17` removes.
No campaign file was edited beyond the copy's two ffibox paths. The scratch copies and their
target dirs were deleted afterwards. Not rerun at `4fa703c`: the campaign names none of
`CAllocator`, its conversions or `impl_clenalloc!`, and `4fa703c` changes nothing else.

## Review

review-1 (`crustify/artifacts/evolution/ffibox-allocator-api/review-1`), of ffibox `e69a71c`:
**rejected** on four fixes; the design judged sound, its validation reproduced (1.100 suite, Miri
under Stacked and Tree Borrows, nine of ten mutations caught), nothing weakened. Required:
`into_box` must also require the object's bytes to be a valid `T` (a Miri probe built a `bool` of 2
through it); "currently allocated" rests on ownership passed to the caller, not on C's frees; the
policy's drop must not unwind; the abort guard needs a test (its removal passed the suite).
Answers: `into_box` cannot be safe for `CZeroable`; `try_update` belongs in the MSRV commit; keep
the relaxed contract, reworded; consolidate the campaign's free/array-free pairs into
`CAllocator<…>` later, one family at a time, keeping their alias names. Fixed in `4fa703c`.

review-2 (`crustify/artifacts/evolution/ffibox-allocator-api/review-2`), of `4fa703c`: **approved**; the four fixes resolved and sound (a probe showed a non-`Copy` policy refused as an `Allocator` through `Box`, `Vec` and `&A`; disarming the abort guard fails its test; the two added tests fail under their mutations), the record entry corrected, the reruns match, no weakening. Nits not taken: the SAFETY comment at `allocator.rs:294-295` omits "or back to C"; three long lines; a `compile_fail` doctest for the `Copy` bound.

## Record

The entry for Crustify's `EVOLUTION.md`, committed by the assistant when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| ffibox adopts the standard `Allocator`: `CAllocator<P>` over a C allocation family, `Box` / `Vec` over it, and their conversions to and from `CBox` / `CVec`; Rust 1.100 the minimum | `libgit2-crustify`, `ffibox-allocator-api` | ffibox `f944c17`, `758eaaf`, `155b9df`, `bd7ad37`, `e69a71c`, `4fa703c` | Rust 1.100 stabilized the allocator API; G17 names the standard `Allocator` and G1 conversions to `Box` / `Vec`, which no primitive met; the user asked to adopt it | an owner over a C family's storage hand-written after this landed (a box and a buffer policy written twice over one family, a hand-written realloc growth of a `CVec`, a native-layout struct in C's allocator without an owner), or a UB finding of a `Box` / `Vec` over `CAllocator`: one that hands C a dangling pointer, frees a C NULL or frees with a size the family's free refuses, a `Box` from `into_box` holding an invalid or address-sensitive `T`, or an allocator method or drop that unwinds | watched |

It weakens no check: no test, lint, gate or contract is relaxed. The `Allocator` docs relax
the standard *fits* for this allocator alone, which is a stated contract of new code, not a
loosened check; the review should judge it.
