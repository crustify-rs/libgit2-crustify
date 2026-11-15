# The `_free` macros' anchors sit on empty `impl Drop` blocks, which make the locks stricter than std's

class: nit
evidence: failing compilation (E0597) of `src/main.rs` here; reading
conditions: any target and features; reproduces on the wave's tip `d3d90f565`, whose three impls are `8aa231a69`'s unchanged

- **finding**: `conventions/free-anchors-on-empty-drop-impls`
- **batch**: `wave-226/batch-5/run-1`, the review of `wave-226/batch-2` (`8aa231a69`), with
  batches 1 and 3
- **items**: `mac::git_mutex_free`, `mac::git_cond_free`, `mac::git_rwlock_free`, and with them
  `mac::git_mutex` (`Mutex<T>`), `mac::git_cond` (`Condvar`) and `mac::git_rwlock`
  (`RwLock<T>`)
- **where**: `src/libgit2/src/util/thread.rs`, the `# Locks` section's types
- **tiers**: caught by this review batch; `wave-226/batch-2`'s own gates had it in scope and
  passed it, and so did the review of its proposal `std-symbol-alias-stand-ins`, which listed
  the cost as not required

## Finding

Batch 2 put each `_free` macro's anchor on a `Drop::drop` it wrote for that alone:
`impl<T: ?Sized> Drop for Mutex<T> { fn drop(&mut self) {} }`, and the same for `RwLock<T>`
and `Condvar`. The standard types the three wrap have no `Drop` of their own: dropping one
releases nothing, as the empty body says. An explicit `Drop` still changes what the type
promises:

- **Drop check.** A type with a `Drop` impl may use its borrowed data when it is dropped, so
  the borrow checker requires everything `T` borrows to strictly outlive the lock. A
  `Mutex<&'a T>` declared before what it borrows does not compile, where a
  `std::sync::Mutex<&'a T>` does. `#[may_dangle]`, which would lift this, is unstable.
- **Moving out.** A type with a `Drop` impl cannot be taken apart, so `Mutex` and `RwLock`
  could not get `into_inner`, which their standard types have, without unsafe code. The batch
  gave them none.

Neither is what `git_mutex_free` means, and nothing reads the anchor on a trait method a
consumer never names. The `conventions` place a macro's annotation on the item that stands in
for it (§3.1). The release of a type here is the type's own drop glue, so the type is that
item. A Rust item may carry several anchors, as `Atomic64Ref::get` carries
`fn::git_atomic64_get` and `mac::git_atomic_ssize_get`. The evolution review of
`std-symbol-alias-stand-ins` rejected the rule that put these anchors on `Drop`, partly because
it contradicts §3.1. It named this cost, and putting the anchor on the type as its fix.

## Evidence

`src/main.rs` here declares a `Mutex<&i32>` and an `RwLock<&i32>` before the `i32` they borrow.
From the repository root, on the tip `d3d90f565`:

```text
CARGO_TARGET_DIR=crustify/tmp/b5/target-repro crustify/scripts/cargo-variant.sh plain \
  run -q --manifest-path crustify/reviews/modules/libgit2/src/util/thread.c/wave-226/batch-5/conventions/free-anchors-on-empty-drop-impls/Cargo.toml
```

It fails to build, exit status 101 (`run-affected.log`):

```text
error[E0597]: `value` does not live long enough
  --> src/main.rs:12:24
   |
11 |     let value = 7;
   |         ----- binding `value` declared here
12 |     mutex = Mutex::new(&value);
   |                        ^^^^^^ borrowed value does not live long enough
...
17 | }
   | -
   | |
   | `value` dropped here while still borrowed
   | borrow might be used here, when `mutex` is dropped and runs the `Drop` code for type `libgit2_crustify::libgit2::src::util::thread::Mutex`
   |
   = note: values in a scope are dropped in the opposite order they are defined
```

The same program over `std::sync::Mutex` and `std::sync::RwLock` compiles and runs.

## Fix

In this batch's second commit: the three empty `impl Drop` blocks are gone. `mac::git_mutex_free`,
`mac::git_cond_free` and `mac::git_rwlock_free` are anchored on `Mutex`, `Condvar` and `RwLock`
beside their type macros, each with its alias. Each type's documentation already said that
dropping it destroys it, as the `_free` macro does, and now also carries the sentence the `Drop`
doc held (the borrow checker sees that no guard or waiter holds it, which POSIX asks of
`pthread_*_destroy`'s caller). `Mutex` and `RwLock` gain `into_inner`, as their standard types
have it, which hands back the data or the poisoned lock's data. The reproducer then prints
`a lock over a borrow drops after what it borrows` and exits 0 (`run-fixed.log`).

The API change: three `Drop` impls removed, a change `ffimap api` counts on the types, and two
methods added. No consumer drops a lock by calling `Drop::drop`, which Rust forbids, so removing
it breaks nothing. The proposal `std-symbol-alias-stand-ins`, still open, gains this occurrence:
its rule must say that the type carries its release's anchor, not its `Drop`.

## Tests

`tests/unit_pub/src/util/thread/sync.rs`: `a_lock_over_a_borrow_drops_after_what_it_borrows`
is the reproducer's scenario, which did not compile before the fix.
`locks_hand_back_their_data` takes a `Mutex` and an `RwLock` apart, and a poisoned `Mutex`.
