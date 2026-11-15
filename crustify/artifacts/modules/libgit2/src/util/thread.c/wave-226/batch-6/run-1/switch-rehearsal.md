# thread.c's switch, rehearsed in scratch

What `wave-226/batch-6/run-1` ran under `crustify/tmp/rehearsal/`, after it added the exports of
`thread.h`'s eleven atomic operations and `unix/pthread.h`'s `git_threads_global_init`
(`conventions/header-functions-without-exports`), so that the module's integration knows what
its switch needs and what it gave. Nothing here was committed to `c/`, and the scratch was
deleted after the run.

1. A copy of the batch's tree, its `c/` a git clone of the fork at `230c9dbdb`, the wave
   branch's `c/`. In it, `src/util/thread.c` is removed. The twelve `GIT_INLINE` definitions
   are replaced by these prototypes, each where its definition was, its comment kept. In
   `src/util/thread.h`, inside `#ifdef GIT_THREADS`, after the include of
   `unix/pthread.h`:

   ```c
   extern void git_atomic32_set(git_atomic32 *a, int val);
   extern int git_atomic32_inc(git_atomic32 *a);
   extern int git_atomic32_add(git_atomic32 *a, int32_t addend);
   extern int git_atomic32_dec(git_atomic32 *a);
   extern int git_atomic32_get(git_atomic32 *a);

   extern void *git_atomic__compare_and_swap(
   	void * volatile *ptr, void *oldval, void *newval);

   extern volatile void *git_atomic__swap(
   	void * volatile *ptr, void *newval);

   extern volatile void *git_atomic__load(void * volatile *ptr);

   #ifdef GIT_ARCH_64
   extern int64_t git_atomic64_add(git_atomic64 *a, int64_t addend);
   extern void git_atomic64_set(git_atomic64 *a, int64_t val);
   extern int64_t git_atomic64_get(git_atomic64 *a);
   #endif
   ```

   and in `src/util/unix/pthread.h`, in place of
   `GIT_INLINE(int) git_threads_global_init(void) { return 0; }`:

   ```c
   extern int git_threads_global_init(void);
   ```

   The prototypes keep C's return types. `git_atomic__swap` and `git_atomic__load` return
   `volatile void *`, which their macros cast to `void *`. The bindings drop the qualifier,
   which changes no ABI. Everything else of both headers is left as it is: the macros, the
   types, `GIT_MEMORY_BARRIER`, and the `#else` branch, a build without threads. That branch
   keeps its plain `GIT_INLINE` bodies and its `#define git_threads_global_init git__noop`.
   No configuration the target is built in compiles it, so it is no remaining C, as
   `porting-symbols` §1 has it, and it holds no export. `win32/thread.h` already declares
   `git_threads_global_init` as a function. The edit is 14 insertions and 140 deletions in
   the two headers, and `thread.c`'s 140 lines go.

   The tied C tests stay: none reaches `thread.c`'s internals, which are its four
   `git_tlsdata_*` functions, public to the C and exported by the gateway since
   `wave-226/batch-3`.
2. The Rust side as the batch left it, but with the module's feature on: the scratch copy of
   `crustify/scripts/rust-staticlib.sh` builds with
   `--features bindgen,ffi-export-src-util-thread-c`. The switch deletes the feature, after
   which the stock script needs no change. The `plain` variant was then configured and built
   from `crustify/artifacts/build.json` (version 7), in a build dir of its own, with the side
   campaigns' installs given by absolute path and `rust-staticlib.sh` run with
   `CRUSTIFY_BINDINGS_UPDATE=1`. The bindings it regenerated differ from the batch's by the
   twelve `#[link_name = "<name>__extern"]` lines of `sys/libgit2/bindings.rs`, and
   `sys/libgit2/static_fns.c` loses the twelve `<name>__extern` wrappers. The functions are
   no longer `static inline`, so their raw bindings reach the exports, and the switch commits
   both files. The two `-Wincompatible-pointer-types-discards-qualifiers` warnings that the
   wrappers of `git_atomic__swap` and `git_atomic__load` raise go with them.
3. `crustify/libcrustify.a` holds 208 objects, where gen5's holds 209: `thread.c.o` is gone.
   `nm libgit2.so.1.9.0` defines the twelve and the four `git_tlsdata_*` as local text
   symbols from the Rust library, and no `*__extern` wrapper. `libgit2_tests` defines
   `git_atomic32_inc` and `git_threads_global_init` from its own link of the Rust library.
4. The C objects of the build inline no atomic instruction. Gen5's `plain` objects inline 254
   atomic read-modify-write and exchange instructions in 34 objects, with `thread.c.o` left
   out, as the finding's `measure/count-sites.sh` counts them; the rehearsal's objects inline
   0. In their place, the build's objects hold 305 relocations to the twelve exports, in 36
   objects: 272 in `src/libgit2`, 11 in `src/util` and 22 in `tests/libgit2`. 304 of them
   are calls: `git_atomic32_inc` 82, `git_atomic__swap` 63, `git_atomic32_dec` 60,
   `git_atomic__load` 46, `git_atomic__compare_and_swap` 28, `git_atomic32_get` 11,
   `git_atomic32_set` 5, `git_atomic64_add` 4, `git_atomic64_get` 2, `git_atomic32_add` 2 and
   `git_atomic64_set` 1. The last is `libgit2.c`'s `init_fns`, which takes
   `git_threads_global_init`'s address (`R_X86_64_64`).
5. `crustify/scripts/c-tests.sh` over that build, 48 jobs: `3442` passed, `0` failed, `55`
   skipped of `3497`, with no non-zero suite. That matches the baseline's `plain` record, and
   `ctest` exited 0. Every suite that takes a reference count runs on the exports, and so do
   the suites of the module's tied tests: `threads::atomic` (13 cases), `threads::basic` (4),
   `threads::tlsdata` (2), and `object::cache` in its four configurations.
6. The cost of an increment-and-decrement pair of one `git_atomic32`, as `GIT_REFCOUNT_INC` and
   `GIT_REFCOUNT_DEC` make it, single-threaded and uncontended. `bench/pair.c`, beside this
   record and run by `bench/run.sh`, is built with
   the `plain` variant's C flags (`-O2 -g -DNDEBUG -std=c90 -fvisibility=hidden -fPIC`). It is
   built once against gen5's `c/`, where the C inlines the builtins, and once against the
   rehearsal's `c/`, where it calls the exports, linked with the rehearsal's `libcrustify.a`
   and Rust library. The two ran alternately, four rounds of five runs of 5 × 10⁷ pairs each,
   on the shared host at a load average of about 270 on 144 cores:

   | build | ns per pair |
   |---|---|
   | inlined, before the switch | 15.35 to 16.37 |
   | the exports, the variants' Rust library (dev profile, as `rust-staticlib.sh` builds it) | 101.42 to 102.75 |
   | the exports, the same crate's library on the release profile | 21.03 to 27.14 |

   The dev profile is what every variant's C links from the switch on, so the variants' C
   pays about 86 ns, 6.6 times the inlined cost, for every reference-count pair. Unoptimised,
   `git_atomic32_inc` calls `atomic32`, then `Atomic32Ref::from_ptr` and `Atomic32Ref::inc`,
   whose `val` and `fetch_add` are calls too. On the release profile the export is a null
   test and one `lock xadd`, and the pair costs 5.5 to 11.6 ns more than inlined. Its figures
   are bimodal, about 21.1 ns in two rounds and 26.9 ns in two, on this loaded host. The
   review measured a C call across translation units at 24.2 to 24.9 ns, in the same band.
   Cross-language LTO, `porting-symbols` §4's TODO, would inline the export back.

## What the switch takes from this

- the C commit on the fork: `thread.c` removed and the two headers edited as in step 1,
  with no CMake change (`crustify/LESSONS.md`, "A libgit2 module's switch removes its file from
  `c/` and changes no CMake file");
- the Rust commit: the feature deleted (the gateway's `cfg`, the `Cargo.toml` entry and
  its comment), the bindings regenerated from that `c/` as in step 2, and the equivalence
  tests retired. Once the bindings reach the exports, the gateway's `equiv_priv` and
  `tests/equiv/src/util/thread/atomics.rs` compare the exports with themselves, and
  `tests/equiv/src/util/thread/tlsdata.rs` compares C's keys with Rust's, which are then
  one implementation. Those that need no C are carried over first. The allowlist keeps
  the twelve: their bindings reach the exports, and the exports' signature checks read
  them.
- the gates: the module's end over the new generation, and the C tests as in step 5. The
  other variants, the coverage variant's figures and the packaging gate were not
  rehearsed.
