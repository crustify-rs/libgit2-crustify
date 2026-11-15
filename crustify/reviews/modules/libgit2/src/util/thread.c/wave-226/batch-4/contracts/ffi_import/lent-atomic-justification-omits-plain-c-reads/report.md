# The lent atomic's justification says every racing access is atomic, which C's plain reads contradict

- **finding**: `contracts/ffi_import/lent-atomic-justification-omits-plain-c-reads`
- **batch**: `wave-226/batch-4/run-1`, the review of `wave-226/batch-0` (`3f7f3ab89`),
  `src/util/thread.c`'s types
- **items**: `fld::git_atomic32.val` and `fld::git_atomic64.val` (`Atomic32Ref::val`,
  `Atomic64Ref::val`)
- **affected commit**: `3f7f3ab89`
- **where**: `src/libgit2/src/util/thread.rs`, `mod ffi_import`, `atomic_type!`'s `val`: its
  second `// SAFETY:` comment and its doc comment
- **class**: `nit`
- **evidence**: reading: the C sources cited below; no reproducer, since the justification is
  imprecise and no input breaks what the code relies on
- **conditions**: any; `GIT_THREADS` with `GIT_BUILTIN_ATOMIC`, this build's
- **pattern**: `aliasing-at-the-boundary`
- **tiers**: caught by this review batch; `wave-226/batch-0`'s own gates had it in scope and
  passed it, as no gate can catch a comment

## Finding

`val` lends the field as `&'a AtomicI32` (or `AtomicI64`) through `from_ptr`, whose
contract forbids mixing conflicting atomic and non-atomic accesses without synchronisation.
Its `// SAFETY:` comment rests on this:

> Every access it may race with is atomic: the handle's other holders reach the value only
> through this, and C writes it only through `thread.h`'s atomic operations (the module
> documentation), of the same size.

and its doc comment says C "reads and writes it with sequentially consistent operations
(`SeqCst`) alone". C's writes are all atomic, but its reads are not. It reads some shared
values plainly:

- `t->cancelled.val` in `c/src/libgit2/transports/smart.c:42` and
  `smart_protocol.c:492`, `571`, `631`, `701`, `725` and `733`. `git_smart__cancel` sets the
  flag with `git_atomic32_set`, from whichever thread calls `git_remote_stop`.
- `git_cache__current_storage.val` in `c/src/libgit2/settings.c:181`, beside
  `cache_store`'s, `cache_evict_entries`'s and `clear_cache`'s `git_atomic_ssize_add`
  (`c/src/libgit2/cache.c:184`, `129`, `72`) on other threads.

The module documentation says so, and says the race is C's own. The comment the code rests
on, and the accessor's own documentation, say the opposite. That comment is what the port
batches of `git_atomic32_set` and its siblings build on: their exports adopt C's
`git_atomic32 *` and write through `val`, so a reader judging an export from the comment
would take a race that exists for granted as absent.

What the code relies on still holds. Every access Rust makes is atomic, and so is every write
C makes. A plain C read races with a concurrent write whoever makes it, Rust through `val`
or C through its own `__atomic_store_n`. That is a race of the C, which the binding neither
adds nor can remove. So the finding is an imprecise justification, a `nit`, and not a
contract that can be broken.

## Fix

In this batch's second commit, the comment says what holds. Every write `val` may race with
is atomic and of the same size, and so is every Rust read. C's plain reads of a shared value
(the module documentation) race with any write made on another thread, C's own atomic ones
as much as one made through `val`: a race of the C's, which lending the atomic neither adds
nor can remove. The doc comment says C writes the integer with sequentially consistent
operations alone, and reads it with them but for the plain reads the module documentation
names. The code does not change.
