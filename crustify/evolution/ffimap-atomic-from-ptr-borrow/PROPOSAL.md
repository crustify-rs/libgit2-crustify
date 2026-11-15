# ffimap's std table does not count `Atomic*::from_ptr` as a borrow

Status: `open`.

## Signal

A tool that reports what the source contradicts (`evolution` §2). `raw_ptr.casted_to.std.borrow`
counts a raw pointer turned into a std reference through `<*const T>::as_ref`,
`<*mut T>::as_mut`, `NonNull::as_ref` / `as_mut` and `&*p`, but not through the standard
atomics' `from_ptr` (`AtomicI32::from_ptr` and its siblings, stable since Rust 1.75), which
turns a `*mut T` into a `&'a Atomic…` the same way. The `ffimap` skill's Reviewing section
reads that counter first among the `ub` smells, as references into C memory, so a review
that starts from the scan misses every atomic lent over C memory.

## Evidence

- `wave-226/batch-4/run-1`, the review of `wave-226/batch-0` (`3f7f3ab89`), `src/util/thread.c`'s
  types. `src/libgit2/src/util/thread.rs`'s `atomic_type!` lends `git_atomic32.val` and
  `git_atomic64.val` as `&'a AtomicI32` / `&'a AtomicI64`, through `AtomicI32::from_ptr` and
  `AtomicI64::from_ptr` on the field projected from the handle's pointer, which addresses C
  memory once a port adopts a C object. The batch's scan, with the skill's `--sites` for `ub`
  (`crustify/tmp/ffimap-base.json` in the batch's worktree), lists
  `raw_ptr.casted_to.std.borrow` nowhere in `thread.rs`: the two invocations (lines 251 and
  301 of the reviewed commit) appear only under `raw_ptr.deref.wr`, for the projection.
- `reproducer/`, beside this proposal: a crate of two functions, one forming a `&i32`
  through `<*const i32>::as_ref`, one an `&AtomicI32` through `AtomicI32::from_ptr`.
  `reproducer/check.sh` scans it and expects both counted; under ffimap `50a800e` it prints
  `raw_ptr.casted_to.std.borrow = 1 sites: [{"file": "src/lib.rs", "count": 1, "lines":
  [13]}]` and exits 1, line 24's `from_ptr` uncounted.
- `std_conversion` in `src/ffimap/driver/src/main.rs` and `docs/analysis.md` §11.1 list the
  table: no atomic is in it. On the pinned nightly (`nightly-2026-10-08`) `AtomicI32` is the
  alias `Atomic<i32>`, and Miri's backtrace names the callee
  `std::sync::atomic::Atomic::<i32>::from_ptr`, so `callee_owners` would read its owner as
  `Atomic`; on a stable release before the generic `Atomic<T>`, as `AtomicI32`.

## Goal

Crustify's Safety goal, its measure the unsafe surface a deterministic scan counts on each
landing; ffimap's G2 (compiler-precise: the callee is matched by its definition, in the
table `analysis.md` lists) and G4 (measures of how the crate meets C). The fitness measure:
`raw_ptr.casted_to.std.borrow` on this campaign's crate, which rises by two at the batch's
tree with the change, and the `ub` findings on a lent atomic a review would otherwise reach
without the scan pointing it there.

## Destination

ffimap: `std_conversion` in `src/ffimap/driver/src/main.rs`, `docs/analysis.md` §11.1's table
and `docs/schema.md`'s `raw_ptr.casted_to` row, with a fixture in the canary suite (G9).

## Change

`std_conversion` maps `(owner, "from_ptr")` to `borrow` for every standard atomic, matched
by the callee's definition in `core::sync::atomic`: the generic `Atomic` and the named types
(`AtomicBool`, `AtomicI8` to `AtomicI64`, `AtomicU8` to `AtomicU64`, `AtomicIsize`,
`AtomicUsize`, `AtomicPtr`), so that the match holds on toolchains before and after the
generic type. `CStr::from_ptr`, which shares the method name, keeps its `string` row, since
the match is on (owner, method). §11.1's `borrow` line names the atomics' `from_ptr`. The
canary gains the reproducer's two functions, expecting `borrow = 2`, and one of
`AtomicPtr::from_ptr`. As G10 asks, the examples are rescanned and the commit states what
moved.

## Validation

Not made: ffimap is outside the reviewer's evolution scope, and the defect does not block
the batch, whose review read `thread.rs` from its source. The reproducer fails on `50a800e`
as stated above.

## Row

| change | from | landed as | signal | recurrence signal | status |
|---|---|---|---|---|---|
| ffimap's std table counts the standard atomics' `from_ptr` as `raw_ptr.casted_to.std.borrow` | `libgit2-crustify`, `ffimap-atomic-from-ptr-borrow` | — | a binding lent `&AtomicI32` over a C field through `AtomicI32::from_ptr`, and the scan's references-into-C-memory smell listed none of it | a std constructor that forms a reference from a raw pointer and that the scan's `casted_to.std` arms do not count | — |

## Review

None yet.
