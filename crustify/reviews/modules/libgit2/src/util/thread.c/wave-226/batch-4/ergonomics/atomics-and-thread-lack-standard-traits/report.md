# The atomics and the thread lack `Debug`, and the atomics `From` their integer

- **finding**: `ergonomics/atomics-and-thread-lack-standard-traits`
- **batch**: `wave-226/batch-4/run-1`, the review of `wave-226/batch-0` (`3f7f3ab89`),
  `src/util/thread.c`'s types
- **items**: `st::git_atomic32` (`Atomic32`, `Atomic32Ref`, `Atomic32Mut`), `st::git_atomic64`
  (`Atomic64`, `Atomic64Ref`, `Atomic64Mut`, and `AtomicSsize`, `tdf::git_atomic_ssize`),
  `st::git_thread` (`Thread`, `ThreadRef`, `ThreadMut`)
- **affected commit**: `3f7f3ab89`
- **where**: `src/libgit2/src/util/thread.rs`, `mod ffi_import`
- **class**: `nit`
- **evidence**: reading; `format!("{:?}", Atomic32::new(1))` does not compile on `3f7f3ab89`
  (E0277, `Atomic32` doesn't implement `Debug`)
- **conditions**: any
- **tiers**: caught by this review batch; `wave-226/batch-0`'s own gates had it in scope and
  passed it

## Finding

`reviewing`'s `ergonomics` objective names the standard traits a type lacks. The wrap is the
first batch to give these types a safe API, which their `port/layout` stages keep (waves 1445,
1620 and 2308), so their traits are reviewed here.

- None of the nine types implements `Debug`. `AtomicI32` and `AtomicI64`, which the atomics
  lend and stand in for, print their integer. The types this module's other batches add do
  implement it (batch 2's locks), as do many of the crate's layout types and handles
  (`AllocatorRef`, `BufRef`, `ErrorRef` among them), and earlier reviews filed
  its absence on a handle (`legacy-wave-37/batch-0`'s `misc/entry-handles-lack-debug`). A
  ported struct that embeds a reference count, as every `git_refcount` holder will once its
  `port/layout` makes it native, cannot `#[derive(Debug)]` while `Atomic32` lacks it.
- `Atomic32` and `Atomic64` have no `From<i32>` / `From<i64>`, which `AtomicI32` and
  `AtomicI64` have, so `let count: Atomic32 = 1.into()` does not compile where
  `let count: AtomicI32 = 1.into()` does.

## Fix

In this batch's second commit, in `ffi_import`, as safe impls beside the handles:

- `atomic_type!` gives each shared handle a `Debug` that prints the integer through the
  standard atomic's own `Debug` (`fmt::Debug::fmt(self.val(), f)`), so that it prints as
  `AtomicI32` does. The value and the exclusive handle print as their shared handle. It also
  gives each value `From` its integer, as `new` builds it.
- `ThreadRef` prints `Thread { thread: <id> }` through its getter, and `Thread` and
  `ThreadMut` print as it.

Each reads only what the safe getters read, and the change adds no unsafe code. Nothing is
removed or changed, so the port batches of the module's symbols that build on these types
keep compiling.

## Tests

`tests/unit_pub/src/util/thread.rs`:
`atomics_print_and_convert_as_the_standard_atomics_do` prints a value, each handle and an
`AtomicSsize`, compares them with `AtomicI32`'s output, and builds values with `From` and
`into`. `a_thread_prints_its_id` prints a zeroed thread and its handles. Both pass plain and
under Miri.
