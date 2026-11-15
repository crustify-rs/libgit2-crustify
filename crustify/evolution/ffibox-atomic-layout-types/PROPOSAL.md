# Layout types whose state is written through shared access

Status: `open`.

## Signal

One representation written by hand (`evolution` §2), and a rule that forces a hand-written
layer `define_ctype!` could cover. `define_ctype!` lays a type out as its C struct alone,
`#[repr(transparent)] struct Foo(ffi::foo, PhantomData<…>)`. A value Rust holds hands out
its handles from `&self`, so they carry a pointer that may only be read through, as
`CRefSeam::as_ptr`'s documentation says. That is right for every C type whose writes go
through an exclusive handle. It is wrong for one whose state its holders change through
shared access, as an atomic counter's is: no write through a shared handle is sound over
a value Rust holds, and a `static` of such a type may be placed in read-only memory.

## Evidence

- `wave-226/batch-0/run-1`, `src/util/thread.c`, wraps `git_atomic32` and `git_atomic64`
  (`thread.h`): one `volatile int` or `int64_t` that libgit2's threads change only with
  `__atomic` builtins. Examples are every `git_refcount`, the runtime's `init_count`, and
  the object cache's `git_cache__current_storage`. The port batches that follow port
  `git_atomic32_inc` and its siblings over them, and the runtime's globals become Rust
  `static`s. Every write to these types goes through shared access.
- With `define_ctype!`, `Atomic32::new(0).as_ref().val().fetch_add(1, SeqCst)`, through
  `AtomicI32::from_ptr` on the projected field, is a write through a pointer derived from a
  shared borrow of a type with no `UnsafeCell`. That write is UB. `reproducer/`, beside this
  proposal, does it over ffibox `568bcb5`, and `cargo +nightly-2026-10-08 miri run` stops it:
  "Undefined Behavior: … only grants SharedReadOnly permission", the tag created by
  `define_ctype!`'s `as_ref`. Under Tree Borrows
  (`MIRIFLAGS=-Zmiri-tree-borrows`) the same run stops too: "write access through <509> …
  is forbidden … the conflicting tag <487> has state Frozen" (`wave-226/batch-4/run-1`, its
  review). The Reference counts the bytes a shared reference points at, outside an
  `UnsafeCell`, as immutable, so the write is UB in either model. So
  the run hand-wrote both layout types over `UnsafeCell<ffi::git_atomic32>`. It wrote one
  local `macro_rules!`, `atomic_type!` in `src/libgit2/src/util/thread.rs`, of about 140
  lines, which restates `define_ctype!`'s expansion: the layout type, both handles over
  `CBorrowedPtr`, `unsafe impl CCell`, the seams, and `as_ref` / `as_mut`. The only change
  is the `UnsafeCell`.
- `CCell::ref_from_ptr`'s contract (ffibox `568bcb5`) lets nothing write the object while a
  shared handle lives, except through a `Mut` of the same borrow, or under a lock that
  every read through `Ref` takes. An atomic takes no lock. Each of its reads is atomic,
  and so is each concurrent write C makes. So the port's export of `git_atomic32_inc`
  adopts `a` as an `Atomic32Ref` while other C threads increment the same count. That keeps
  the atomics' own soundness argument, but not the trait's literal text. The record's row
  for `ffibox-ref-contract-self-locking` widened that text for a self-locking object; an
  object of atomics is the lock-free case of the same rule.
- G9's handle rule is "no Rust reference over foreign memory that anything may write
  while the reference lives". Read literally, it forbids the `&'a AtomicI32` the run's
  `val` accessor lends. The hazard G9 excludes is a reference that asserts what is false:
  immutability, or exclusive access. A shared reference to a standard atomic asserts
  neither. It asserts liveness, alignment, and that every concurrent access is atomic,
  which is the atomic's contract. The standard library offers `AtomicI32::from_ptr` for
  exactly this interoperation.

## Goal

Crustify's Safety goal (soundness), and ffibox's G1 (mirror standard Rust: a C atomic
field's safe form is the standard atomic) and G12 (obligations local: the contract an
atomics wrapper relies on is stated by the trait it implements, not argued against it).
The fitness measure: the hand-written `unsafe impl CCell` count in the campaign's crate,
which this run raised by two. No `ub` or `contracts` finding should appear on a layout
type written through shared access.

## Destination

ffibox: `define_ctype!` (`src/macros.rs`), `CCell::ref_from_ptr`'s and `CRefSeam::as_ptr`'s
contracts (`src/traits.rs`), `docs/borrows.md`'s conventions, `docs/hand-written.md`'s
scope, and G9's handle row in `docs/goals.md`. Once that lands, `binding-types`' Field accessors
section gains the case's line: a field C changes through shared access, with atomic
operations alone, is lent by the shared handle as the standard atomic of its width, and its
layout type is declared `interior`.

## Change

1. `define_ctype!` takes an option, `interior`, for a C type whose state its holders write
   through shared access. It lays the type out as
   `#[repr(transparent)] struct Foo(UnsafeCell<ffi::foo>, PhantomData<…>)`. Its `as_ref`
   then hands out a pointer that may be written through, as `UnsafeCell::get`'s does, and a
   `static` of it is writable. All else is unchanged: `CCell`, the handles and the seams.
   The layout type stays `!Sync` until the binding asserts `Sync`.
2. `CCell::ref_from_ptr`'s contract admits a third kind of concurrent write. Beside writes
   through a `Mut` of the same borrow, and writes under a lock every read through `Ref`
   takes, it admits writes made atomically to bytes that every access through `Ref` makes
   atomically, of the same size. `CRefSeam::as_ptr`'s note extends to such a type: one
   declared `interior` may be written through `as_ptr` atomically, even when Rust holds
   the value.
3. G9's handle row and `docs/borrows.md`'s "No reference covers C memory anything may
   write" exempt a shared reference to a standard atomic over a field that only atomic
   accesses reach. Such a reference asserts no immutability and no exclusive access.

## Validation

Not made: this proposal is the binder's, and ffibox is outside its evolution scope. The
run's `atomic_type!` is the expansion that `interior` would emit. Its tests are evidence
for that expansion:

- `tests/unit_pub/src/util/thread.rs`, `tests/ub_safe/src/util/thread.rs` and
  `tests/ub_unsafe/src/util/thread.rs`;
- a `static` written from four threads; and
- shared handles and a lent `&AtomicI32` incremented from four threads at once.

They are clean under Miri and TSan. A change would add these as ffibox tests, with a
`compile_fail` case that a non-`interior` type still hands out a read-only pointer.

## Row

| change | from | landed as | signal | recurrence signal | status |
|---|---|---|---|---|---|
| `define_ctype!` takes `interior`, a layout over `UnsafeCell` for state written through shared access; `ref_from_ptr` admits atomic writes to bytes every `Ref` access reaches atomically; G9 exempts a reference to a standard atomic | `libgit2-crustify`, `ffibox-atomic-layout-types` | — | `git_atomic32` / `git_atomic64` were laid out by hand over `UnsafeCell`, since `define_ctype!`'s values hand out read-only pointers, and the adoption the atomics' port makes broke `ref_from_ptr`'s text | a hand-written `CCell` over `UnsafeCell`, or a `ub` finding of a write through a shared handle of a value Rust holds | — |

## Review

None yet.
