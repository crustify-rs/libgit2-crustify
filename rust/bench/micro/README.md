# util micro-benchmarks (C vs Rust port)

`util_bench.c` exercises libgit2's internal util layer through its C ABI and is
linked twice: once against the C reference objects, once against the Rust port.
Because both expose the same symbols, the same benchmark object is used for both.

Workloads (each prints `bench <name> median_ms <x> min_ms <x> max_ms <x>`):

| name                 | what                                                             |
|----------------------|------------------------------------------------------------------|
| str_append           | `git_str_puts/putc/put` into one big git_str + many fresh ones   |
| str_printf           | `git_str_printf(&b, "%s/%d/%08x", ...)`                          |
| str_join             | `git_str_joinpath`, `git_str_join`, `git_str_join_n(..,3,..)`, and `git_str_joinpath(&b, b.ptr, x)` |
| vector_insert        | `git_vector_insert` of 1e6 pointers, x25                         |
| vector_sort_random   | `git_vector_sort` (git__strcmp_cb) of 250k random strings        |
| vector_sort_nearly   | same, on sorted input with 1% random swaps                       |
| vector_bsearch       | `git_vector_bsearch` on a sorted 1e6-string vector (hits+misses) |
| vector_insert_sorted | `git_vector_insert_sorted` of 40k strings                        |

Input data comes from a fixed-seed PRNG. Untimed setup (e.g. refilling the
vector before sorting) runs before each timed iteration, and one warm-up run
comes first.

## Build

    ./build.sh

This writes `out/util_bench_c` and `out/util_bench_rust` (clang -O2). You can
override `C_BUILD` (default `/work/build-c`), `RUST_BUILD` (default
`/work/builds/foundation`), and `CC`.

- C: the internal symbols are not exported from libgit2.so, so the bench
  links the object files and libraries from the `util_tests` link edge in
  `$C_BUILD/build.ninja`, minus `tests/` (clar) objects.
- Rust: `util_bench.o -Wl,--whole-archive libgit2_c_rest.a -Wl,--no-whole-archive
  rust-target/release/liblibgit2_rs.a <rust-link-libs.txt> -ldl -lm -lpthread`.

## Run

    ./run.sh [ITERS=7] [ROUNDS=5] [FILTER]

For each workload, the C and the Rust binaries are run alternately ROUNDS
times (ITERS timed iterations each, after a warm-up), so that load on the
machine affects both alike. The table shows the median and the minimum of the
rounds' medians, and their ratios (Rust/C). It pins to CPU 2 with taskset when
available; `BENCH_CPU=N` picks another CPU, `BENCH_CPU=` turns pinning off.
Either binary can be run directly: `out/util_bench_c [ITERS] [FILTER]`.

Results: see `RESULTS.md`.

Results: see `RESULTS.md`.

## Other benchmark programs

`build.sh` and `run.sh` take `BENCH=<name>` (default `util_bench`) to build
and run `<name>.c` the same way:

- `u4_bench.c` (WP u4): URL parsing / formatting / patterns (`net.c`), the
  sorted cache on `refs.git`'s 54k-line `packed-refs` (`sortedcache.c`),
  and process spawning and pipes (`unix/process.c`).
  `BENCH=u4_bench ./build.sh && BENCH=u4_bench ./run.sh 7 5`.
