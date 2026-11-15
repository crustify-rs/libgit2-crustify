# What stands in for a symbol alias reached through the standard library

Status: `open`, Crustify `06c62a2` on `libgit2-crustify/evolution/std-symbol-alias-stand-ins`,
awaiting review.

## Signal

A rule an agent had to work out by hand that a skill should state. `porting-symbols`' Macros
section ports a symbol alias of an out-of-tree library's entity "as that entity's route reaches
it ...: through the standard library, it is a native function". `binding-symbols`' Macros
section puts a symbol alias's anchor on "the safe wrapper of the symbol it names", and the
record's row for `symbol-alias-anchors` on that symbol's stand-ins. An entity reached through the
standard library has no wrapper and no stand-in in the crate. No item of the standard library
can carry an anchor. And an alias of a type, or of the routine that releases one, is not a
function. So nothing said which item carries such a macro's anchor, nor what shape that item
takes.

## Evidence

`modules/libgit2/src/util/thread.c/wave-226/batch-2/run-1` ported 19 macros of
`src/util/unix/pthread.h`. Each names a POSIX entity. `git_mutex` is `pthread_mutex_t`, and
`git_mutex_init`, `_lock`, `_unlock` and `_free` are `pthread_mutex_init`, `_lock`, `_unlock` and
`_destroy`. The same holds for `git_cond`, `git_rwlock` and their operations, and
`git_thread_create` and `git_thread_join` are `pthread_create` and `pthread_join`. The header
keeps them for the C that remains, so their Rust forms are linked to no C, and they reach
`std::sync::Mutex`, `Condvar`, `RwLock` and `std::thread`. The run had to decide where 19
anchors go. A `pub type Mutex<T> = std::sync::Mutex<T>` could carry `mac::git_mutex`'s anchor,
but nothing could carry `git_mutex_lock`'s or `git_mutex_free`'s. So it wrote
`src/libgit2/src/util/thread.rs`'s `Mutex`, `Condvar` and `RwLock`, a newtype each over the
standard type, and `JoinHandle`, over `std::thread::JoinHandle`. Each operation macro is a method
(`Mutex::new`, `lock`, `MutexGuard::unlock`, `Condvar::wait`, `notify_one`, `RwLock::read`,
`write`, the guards' `unlock`), `git_thread_create` is the free function `spawn`, and each
`_free` is the type's `Drop::drop`, which `#[doc(alias)]` accepts on a trait impl's method.
`ffimap api` over the worklist keys all 19 as `new`, each with its stand-in.

## Goal

Consistency. The section already gives this case a pattern, "a native function", which does
not fit an alias of a type or of a release, so a run either leaves such a macro unanchored, as a
re-export of a standard item would, or shapes its stand-in its own way. Two such shapes cost a
consumer, who meets a different API for each header's locks, and a reviewer. The fitness
measure is the conventions findings per item; the change expects none on such a macro's anchor.
This is the case's first occurrence, so the proposal corrects the pattern the section gives
rather than adding one.

## Occurrences

- `wave-226/batch-5/run-1`, the review of the batch above
  (`conventions/free-anchors-on-empty-drop-impls`), against the Change's last clause, "an alias
  of the operation that releases it that type's `Drop`". The three empty `impl Drop` blocks that
  carried `mac::git_mutex_free`, `mac::git_cond_free` and `mac::git_rwlock_free` made
  `Mutex<&'a T>` and `RwLock<&'a T>` stricter under the drop check than the standard locks:
  a lock declared before what it borrows fails with `E0597`, where `std::sync::Mutex` compiles.
  They also kept `into_inner` from being written without unsafe code. The review moved the
  three anchors onto the types, beside their type macros' anchors, and removed the impls. The
  rule's release clause should read "the type itself, whose drop releases it", which is also
  the `conventions`' "item that stands in for it" (§3.1), as this proposal's review asks.

## Destination

The spec, `porting-symbols`' Macros section, which the translator evolves.

## Change

Crustify `06c62a2`: the section's last sentence becomes "through the standard library, it is a
native item of its module, which carries the alias's anchor, since no item of the standard
library can. An alias of a type is a type of the module over the standard one, an alias of an
operation on it that type's method, and an alias of the operation that releases it that type's
`Drop`."

## Validation

By the evidence that motivated it. The run above ported the 19 macros so. Its lints, with
warnings denied, accept the anchors on the three `Drop::drop` methods. Its `ffimap api` measure
finds every key of the worklist.

## Record

| change | campaign, proposal | landed as | signal | recurrence signal | status |
|---|---|---|---|---|---|
| a symbol alias of an out-of-tree entity reached through the standard library stands in as its module's type over the standard one, that type's method, or its `Drop` for the release, carrying the alias's anchor | `libgit2-crustify`, `std-symbol-alias-stand-ins` | crustify `06c62a2` | no skill said which item carries the anchor of a macro aliasing a C library's type or routine whose route is the standard library, whose items carry none | a review finding of such a macro's anchor missing or misplaced, or a port that re-exports or aliases a standard item for one | watched |

## Review

None yet.
