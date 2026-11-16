# Foundation benchmarks: C reference vs Rust port

Machine: shared 144-core x86-64 host under heavy load (load average 420-470
throughout), so wall times are noisy; C and Rust were interleaved (run.sh) and
pinned to one CPU. clang -O2 driver, Rust `--release` (opt-level 3).

## Workloads (`./run.sh 7 5`: 5 alternating rounds of 7 iterations each)

| workload | C median ms | Rust median ms | Rust/C median | Rust/C min |
|---|---:|---:|---:|---:|
| str_append (puts/putc/put) | 530.1 | 631.4 | 1.19 | 1.22 |
| str_printf | 457.5 | 465.6 | 1.02 | 0.99 |
| str_join (joinpath, join, join_n, self-join) | 461.3 | 644.6 | 1.40 | 0.86 |
| vector_insert (1e6 x 25) | 355.6 | 382.2 | 1.07 | 1.16 |
| vector_sort_random (250k strings) | 434.6 | 411.2 | 0.95 | 0.53 |
| vector_sort_nearly | 400.5 | 428.5 | 1.07 | 1.26 |
| vector_bsearch (1e6) | 280.2 | 188.0 | 0.67 | 0.88 |
| vector_insert_sorted (40k) | 360.8 | 340.4 | 0.94 | 0.87 |

The vector workloads are at parity within the noise (min and median ratios
straddle 1). The string workloads are slower.

## Per call (`str_ops.c`, best of 3 x 5 runs of 5M calls, ns)

| call | C | Rust | Rust/C |
|---|---:|---:|---:|
| git_str_putc | 6.1 | 6.1 | 1.00 |
| git_str_put (4 bytes) | 11.9 | 15.2 | 1.27 |
| git_str_puts | 24.2 | 30.6 | 1.27 |
| git_str_sets | 22.0 | 27.8 | 1.26 |
| git_str_joinpath | 52.9 | 57.1 | 1.08 |
| git_str_joinpath(&s, s.ptr, x) | 53.2 | 70.1 | 1.32 |
| git_str_join_n (3 segments) | 159.7 | 200.6 | 1.26 |

Before the hot-path work (first port) these were 1.6-2.4x for str_append /
str_join and join_n 300 ns. Remaining gap: a few nanoseconds per call in the
export prologue and the C/Rust split of join_n (C shim collects the varargs,
Rust joins); not closed. See the WP report.

# WP u1: oid, pool, pqueue, utf8, varint, parse (C reference vs Rust port)

Machine as above (load 270-580 during the runs); both builds' programs
pinned to CPU 2 and interleaved.

## Public API: `src/bench_oid.c` (master's suite, `compare.sh /work/build-c /work/builds/u1 oid`)

424k ids of git.git's pack; 3 rounds x 3 repetitions, medians.

| workload | C ms | Rust ms | Rust/C |
|---|---:|---:|---:|
| from_string | 76.9 | 25.5 | 0.33 |
| from_prefix | 43.5 | 18.3 | 0.42 |
| fmt | 23.6 | 8.9 | 0.38 |
| tostr | 48.4 | 34.5 | 0.71 |
| pathfmt | 19.0 | 15.2 | 0.80 |
| sort_cmp (qsort with `git_oid_cmp`) | 68.7 | 48.3 | 0.70 |
| equal_ncmp_streq | 63.7 | 43.5 | 0.68 |
| shorten (20 x 12000 ids) | 14.2 | 26.9 | 1.90 |

Check values identical. `shorten` is the one regression: the trie walk has
C's algorithm and node layout (32-byte nodes, tails borrowed from the
caller's strings) but costs ~110 ns per id against C's ~60 ns; nothing
inside libgit2 calls `git_oid_shorten_*`. (A first version that copied
the ids was 2.9x, from page faults and an extra indirection.)

## Internal API, per call: `micro/u1_bench.c` (`micro/run_u1.sh 5`)

Best of 5 runs of N calls, median over 5 interleaved rounds, ns per call.

| call | C | Rust | Rust/C |
|---|---:|---:|---:|
| git_oid_from_string | 193.0 | 74.1 | 0.38 |
| git_oid_from_prefix (7) | 32.2 | 21.4 | 0.66 |
| git_oid_fmt | 55.7 | 25.1 | 0.45 |
| git_oid_tostr_s | 69.9 | 45.0 | 0.64 |
| git_oid_tostr (8) | 20.0 | 17.2 | 0.86 |
| git_oid_pathfmt | 54.6 | 39.2 | 0.72 |
| git_oid_cmp | 15.8 | 7.5 | 0.48 |
| git_oid_equal | 17.9 | 6.8 | 0.38 |
| git_oid_ncmp (7) | 10.3 | 9.4 | 0.91 |
| git_oid_streq | 55.0 | 40.4 | 0.74 |
| git_oid_is_zero | 4.5 | 4.8 | 1.07 |
| git_oid_shorten_add (4096 ids repeated) | 141.7 | 99.4 | 0.70 |
| git_pool_malloc | 25.6 | 25.7 | 1.01 |
| git_pool_strndup | 31.8 | 30.6 | 0.96 |
| git_pqueue_insert + pop (1000 queued) | 86.6 | 84.1 | 0.97 |
| git_utf8_char_length (61 bytes) | 107.6 | 32.9 | 0.31 |
| git_utf8_iterate (3-byte char) | 14.4 | 16.8 | 1.17 |
| git_decode_varint | 19.7 | 18.1 | 0.92 |
| git_encode_varint | 24.4 | 26.7 | 1.09 |
| git_parse_advance_line | 26.9 | 19.6 | 0.73 |
| git_parse_advance_ws + digit | 95.5 | 86.7 | 0.91 |

Within noise (±10%) for the pool, pqueue, varint and `is_zero`;
`git_utf8_iterate` is ~2 ns slower per call (export prologue). The
hash maps are not benchmarked against C here: C's maps are inline in the
C that stays; the Rust map runs the same algorithm (`tests/u1_hashmap_c.rs`
checks identical states and iteration order).
