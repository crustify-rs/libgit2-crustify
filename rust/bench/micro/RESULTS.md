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

## Per call (`str_ops.c`, best of 4 x 5 runs of 5M calls, ns)

After review round 1 (F-11): each hot export is a short fast path (a
`memmove` into the allocation when no growth is needed, as C's) with the
barrier and the error paths outlined; `git_str_join`/`git_str_join_n` have
panic-free fast paths.

| call | C | Rust | Rust/C | (round 0) |
|---|---:|---:|---:|---:|
| git_str_putc | 6.50 | 4.99 | 0.77 | 1.00 |
| git_str_put (4 bytes) | 11.45 | 10.53 | 0.92 | 1.27 |
| git_str_puts | 24.81 | 24.63 | 0.99 | 1.27 |
| git_str_sets | 22.02 | 19.96 | 0.91 | 1.26 |
| git_str_joinpath | 52.38 | 47.81 | 0.91 | 1.08 |
| git_str_joinpath(&s, s.ptr, x) | 50.93 | 49.29 | 0.97 | 1.32 |
| git_str_join_n (3 segments) | 157.90 | 149.98 | 0.95 | 1.26 |

Build and run: `../../difftest/link.sh str_ops.c out`, then
`taskset -c N out/str_ops_c` and `out/str_ops_rust`, alternately.

Before the hot-path work (first port) these were 1.6-2.4x for str_append /
str_join and join_n 300 ns. Remaining gap: a few nanoseconds per call in the
export prologue and the C/Rust split of join_n (C shim collects the varargs,
Rust joins); not closed. See the WP report.

# WP u4: net, sortedcache, process

`u4_bench.c` (`BENCH=u4_bench ./build.sh && BENCH=u4_bench ./run.sh 7 5`),
C reference `/work/build-c` vs `/work/builds/u4`; 5 alternating rounds of 7
iterations, pinned to CPU 2, load average 400-570 (noisy). Times are the
median and minimum over the rounds' medians, in ms per iteration.

| workload | what | C med | C min | Rust med | Rust min | Rust/C med | Rust/C min |
|---|---|---:|---:|---:|---:|---:|---:|
| url_parse | 400k `git_net_url_parse` + dispose, 8 URLs (IPv6, credentials, %-escapes) | 385.6 | 380.4 | 379.3 | 376.9 | 0.98 | 0.99 |
| url_parse_scp | 400k `parse_standard_or_scp` of SCP paths | 239.8 | 231.0 | 225.0 | 203.1 | 0.94 | 0.88 |
| url_fmt | 1M `git_net_url_fmt` into a reused `git_str` | 206.2 | 203.6 | 195.9 | 184.5 | 0.95 | 0.91 |
| url_join_redirect | 200k joinpath + apply_redirect with a service suffix | 358.8 | 166.6 | 327.7 | 154.0 | 0.91 | 0.92 |
| url_pattern | 1M `matches_pattern_list` (10 no_proxy entries) + `hostname_matches_cert` | 629.6 | 550.4 | 476.7 | 472.0 | 0.76 | 0.86 |
| sortedcache_load | 5x new + lockandload + 54k upserts + wunlock (sort) + free, refs.git packed-refs | 700.5 | 689.7 | 264.6 | 257.8 | 0.38 | 0.37 |
| sortedcache_lookup | 1M lookups, 125k lookup_index, 62k misses, a walk of 54k entries | 2127.4 | 2081.8 | 1617.8 | 1601.0 | 0.76 | 0.77 |
| process_spawn | 200x new + start + wait of `/bin/true` | 1237.2 | 1178.2 | 1023.6 | 963.9 | 0.83 | 0.82 |
| process_pipe | 20x `cat` with 1 MiB written and read back in 16 KiB pieces | 225.4 | 209.5 | 208.5 | 201.4 | 0.93 | 0.96 |

(url_pattern was rerun alone after the last change; the others are from one
full run.) The sorted cache's key map is a Rust `HashMap` with an Fx-style
hash and a murmur finalizer (without the finalizer, keys with long common
prefixes collided and the cache was 12x slower). `git_net_url_fmt` uses the
foundation's `git_str` fast path for its pieces.
