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

# WP u2: hash, zstream, date, wildmatch (C reference vs Rust port)

Machine: the same shared host, load average 300-470. C and Rust interleaved
and pinned (micro: `BENCH=codec_bench micro/run.sh 5 7`, medians of 7
rounds' medians; suite: `compare.sh /work/build-c /work/builds/u2 codec`).

## Internal API (`micro/codec_bench.c`)

| workload | C median ms | Rust median ms | Rust/C median | Rust/C min |
|---|---:|---:|---:|---:|
| sha1_64m (SHA-1 with collision detection) | 643.6 | 592.4 | 0.92 | 0.94 |
| sha256_64m | 396.8 | 432.4 | 1.09 | 1.10 |
| sha1_100b_x200k | 239.8 | 225.8 | 0.94 | 0.97 |
| sha256_100b_x200k | 189.3 | 201.5 | 1.07 | 1.06 |
| sha1_1k_x200k | 1748.6 | 1664.6 | 0.95 | 0.94 |
| sha256_1k_x200k | 1188.6 | 1285.2 | 1.08 | 1.10 |
| inflate_text_16m (12 MiB of C sources) | 71.9 | 36.2 | 0.50 | 0.50 |
| inflate_stream_16m (64 KiB output chunks) | 64.1 | 35.0 | 0.55 | 0.52 |
| inflate_small_x4000 (100 B-8 KiB) | 128.2 | 94.7 | 0.74 | 0.73 |
| deflate_text_4m (system zlib in both) | 179.1 | 188.8 | 1.05 | 0.96 |
| date_parse_x100k | 676.1 | 685.1 | 1.01 | 1.04 |
| date_approx_x20k | 587.0 | 566.6 | 0.97 | 0.99 |
| wildmatch_x1m | 234.7 | 228.5 | 0.97 | 0.97 |

SHA-256 is OpenSSL's in the C; the Rust's own implementation (AVX2
schedule, BMI rounds) is 6-10% slower; the `sha2` crate was 2x slower. A
same-process, finely interleaved comparison on 64 KiB inputs gives 1.09-1.10.

## Public API (`src/bench_codec.c`; needs `codec` in `BENCH_PROGRAMS`)

| workload | C ms | Rust ms | ratio | minrat |
|---|---:|---:|---:|---:|
| codec/mempack_hash_64m | 851.2 | 848.5 | 1.00 | 0.86 |
| codec/mempack_hash_small_x100k | 2420.0 | 2494.5 | 1.03 | 0.94 |
| codec/repository_hashfile_16m | 178.2 | 172.6 | 0.97 | 0.98 |
| codec/pathspec_match | 161.8 | 176.9 | 1.09 | 1.10 |

`pathspec_match` is mostly wildmatch on short paths that fail early: the
Rust pays ~20 ns per call more (two `strlen`s for its slices, entry cost),
while long scans are as fast or faster (memchr).
