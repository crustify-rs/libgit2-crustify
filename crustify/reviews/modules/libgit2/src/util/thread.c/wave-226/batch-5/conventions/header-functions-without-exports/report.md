# Twelve ported `static inline` functions have no export, against `porting-symbols` § 4

class: nit
evidence: reading; the measurements in `measure/`
conditions: any target; on `955c56eb7` (the atomics), `d3d90f565` (`git_threads_global_init`) and the wave's tip `d3d90f565`

- **finding**: `conventions/header-functions-without-exports`
- **batch**: `wave-226/batch-5/run-1`, the review of `wave-226/batch-1` (`955c56eb7`) and
  `wave-226/batch-3` (`d3d90f565`)
- **items**: `thread.h`'s eleven: `fn::git_atomic32_get`, `_set`, `_inc`, `_dec` and `_add`,
  `fn::git_atomic64_get`, `_set` and `_add`, `fn::git_atomic__compare_and_swap`, `__swap` and
  `__load`; and `unix/pthread.h`'s `fn::git_threads_global_init`
- **where**: `src/libgit2/src/util/thread.rs`'s gateway (`mod ffi_export`), which exports
  none of them, and the two headers, which keep the bodies
- **tiers**: caught by this review batch, as its task message asks; `wave-226/batch-1` and
  `wave-226/batch-3` had it in scope and followed their task messages, which said the opposite
  of the spec
- **status**: not patched, at the assistant's instruction: whether the twelve are exported is
  the user's decision
- **revisit**: fixed by `wave-226/batch-6/run-1` (`2f2ffb73d`), the user's decision: the
  twelve are exported in the module's gateway, under `ffi-export-src-util-thread-c` until the
  switch, each checked against its raw binding. `wave-226/batch-7/run-1`, the module pass,
  found all twelve defined there and, in a rehearsal of the switch with the prototypes in
  place, called by the remaining C on `asan-ubsan` and `tsan` with its C tests at the
  baseline (that run's `switch-rehearsal.md`)

## Finding

`porting-symbols` § 4 ports a `static inline` function a header defines as the module's other
functions, and at the module's switch replaces the header's definition by its prototype, so
that the remaining C calls the Rust export. The `conventions`' Exports section (§1.4) lists
"the `static inline` functions its headers define" among a gateway's exports. The three port
batches' task messages said that such a function "stays in the header and needs no export, as
`porting-symbols` describes": that sentence of the spec is about a header's macros. So the
eleven atomics are native methods and functions with no export, `git_threads_global_init` is
`global_init()` with none, and at the switch the headers would keep all twelve bodies. Batch 1
filed the difference as the proposal `static-inline-functions-kept-in-their-headers`, which is
`open` and unreviewed. The 19 macros of batch 2, and `GIT_MEMORY_BARRIER` and the
`git_atomic_*` macros of batch 1, are macros: they stay in their headers by the spec, and are
not part of this finding.

## What each choice costs

**Export the twelve, as § 4 has it.** The gateway gains twelve exports over raw pointers: a
`git_atomic32 *`, a `git_atomic64 *` or a `void * volatile *`. Each export is checked against its
raw binding, which the equivalence tests already allowlist. At the switch, `thread.h`'s
`GIT_THREADS` bodies become prototypes, as does `unix/pthread.h`'s `git_threads_global_init`;
`win32/thread.h` already declares the latter as a function. The remaining C then calls into Rust
at every expansion. Measured on this tree:

- Reference sites in the C that remains, `src/` and `tests/` less `thread.[ch]`, `unix/pthread.h`
  and `win32/`: `git_atomic32_get` 14, `_set` 8, `_inc` 14, `_dec` 11, `_add` 2, the three
  `git_atomic64_*` 1 each; the macros over the pointer functions, `git_atomic_compare_and_swap`
  14, `git_atomic_swap` 16 and `git_atomic_load` 7; `git_atomic_ssize_get` 1 and `_add` 3; and,
  through `util.h`, `GIT_REFCOUNT_INC` 52 (21 files) and `GIT_REFCOUNT_DEC` 21 (15 files). Every
  reference count of the library goes through them. `git_threads_global_init` has one site,
  `libgit2.c`'s `init_fns`, which takes its address.
- In gen5's `plain` objects, `thread.c.o` left out, 254 atomic read-modify-write and exchange
  instructions sit inlined in 34 objects, led by `repository.c.o` (52), `refdb_reftable.c.o`
  (33), `attrcache.c.o` (19) and `submodule.c.o` (18) (`measure/count-sites.sh`,
  `measure/sites-plain.txt`). The count holds every such instruction the objects inline, so it
  is an upper bound for `thread.h`'s. The sequentially consistent loads compile to plain moves,
  and are not counted, though they would become calls too.
- An increment and a decrement through a call to another translation unit, with no LTO, as
  an export is called, took 24.2 to 24.9 ns a pair, against 15.4 to 15.7 ns inlined: about
  4.5 ns, or 57 to 59%, more per pair, single-threaded and uncontended (`measure/run-bench.sh`,
  clang 18 at `-O2`, on this shared host, three runs of 10⁸ pairs twice). Contended operations
  cost more, which the call adds to unchanged. Cross-language LTO, which would inline the export
  back, is § 4's own TODO.
- `git_threads_global_init` costs nothing measurable: `git_libgit2_init` calls it once. Its
  export returns `c_int` 0, where `global_init` returns `()`.

What the export buys is § 4's and `porting-workflow` § 1.1's principle, that each function has
one implementation. After the switch the C would run the Rust's operations, so no equivalence
test is needed to hold the two forms together.

**Keep the bodies, as the batches did.** Nothing changes at the switch: no export, no header
edit, no call where C inlined an instruction. Each operation keeps two implementations, the
header's and the port's. Both are the same C11 sequentially consistent operation on the same
bytes, as the module documentation argues. The `equiv` tests run them on one object at once:
four threads raise and drop one count, two through each, and C's and Rust's compare-and-swap
race on one slot, plain, under ASan+UBSan and under TSan. A later change to one form must be
made in the other too, until `thread.h` leaves with the last C file that includes it. The
proposal `static-inline-functions-kept-in-their-headers` would make this the spec's rule for a
`static inline` function whose body reaches only its arguments and the language's operations.
`git_threads_global_init` would fall under it too: its body is `return 0`.

Either way, the module's switch must say which it took. Under the spec as it stands, the
module pass or the switch adds the twelve exports and edits both headers.
