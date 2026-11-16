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
