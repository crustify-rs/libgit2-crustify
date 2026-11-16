# Benchmark results

All numbers: medians over all samples (3 rounds × 3 repetitions = 9 per
build, after 1 warm-up run per round), per timed run, in milliseconds. See
README.md for what each workload does. Ratio = second build / first build.

## Baseline: C vs C (noise level)

The C reference build against itself, to show how much two runs of the same
library differ on this machine:

```
LABEL_A=C1 LABEL_B=C2 rust/bench/compare.sh /work/build-c /work/build-c
```

- Date: 2026-11-16, 05:55 to 06:32 UTC (37 minutes); defaults
  (`BENCH_ROUNDS=3 BENCH_REPS=3 BENCH_WARMUP=1`, per-workload interleaving),
  pinned to CPU 2 (`taskset -c 2`).
- Machine: 4 × Intel Xeon E7-8870 v3 (144 hardware threads), Linux
  6.14, clang 18.1.3; `/work/build-c` is RelWithDebInfo (`-O2 -g`). The
  machine was shared and oversubscribed throughout: load average 350–450,
  every CPU at ~100% utilisation (including CPU 2 and its hyperthread
  sibling 74). Pinning therefore does not give the benchmark a quiet CPU; it
  only keeps it from migrating. Absolute times are inflated by the
  contention and should not be compared with numbers from another machine
  or another time; only the ratios of an interleaved comparison are
  meaningful.
- Data: `/work/bench-data` as made by `prepare-data.sh` (git.git cloned
  2026-11-16).
- Raw data: `baseline-c-vs-c.tsv` (and `results/baseline-c-vs-c/`, not
  committed).

### Summary

| | wall time | CPU time |
|---|--:|--:|
| workloads | 117 | 117 |
| geometric mean of ratios | 1.004 | 0.999 |
| median \|ratio − 1\| | 1.8% | 1.6% |
| within ±5% | 102 | 103 |
| within ±10% | 110 | 111 |
| largest \|ratio − 1\| | 57.6% (refs/reflog_read_2000_reflogs) | 20.2% (index/read_git_x10) |
| check mismatches | 0 | |

How to read a C-vs-Rust comparison against this:

- On this machine a single workload's ratio within about ±5% is noise; a
  difference is credible when it exceeds ~10% and shows in both the wall
  and the CPU ratio (and, better, persists in a re-run with a filter on that
  workload). The geometric mean over many workloads of a component is much
  more stable than any single ratio (here 1.004 / 0.999 overall).
- The `refs` workloads (and others dominated by file-system calls: loose
  odb, init, config writes) have wall times about twice their CPU times in
  this run, i.e. they were waiting on the shared file system; their wall
  ratios are the noisiest (up to 1.58). Their CPU ratios are within ±5%
  except `lookup` (0.93), `name_to_id` (1.19) and `open_lookup_packed_x100`
  (1.11). Prefer CPU time for them, and re-run them in isolation before
  concluding anything.
- An earlier baseline run with whole-program interleaving
  (`BENCH_INTERLEAVE=program`) was much noisier: median |ratio − 1| 3.8%,
  68 of 117 within 5%, outliers from 0.45 to 2.24, because bursts of load
  on the machine lasting minutes hit one build's whole program. That is why
  `compare.sh` now interleaves per workload by default.
- All 117 check values were identical between the two runs and consistent
  across all repetitions and rounds.

### Table

| component | workload | C1 median ms | C2 median ms | ratio | ratio of min | C1 CPU ms | C2 CPU ms | CPU ratio | check |
|---|---|--:|--:|--:|--:|--:|--:|--:|---|
| oid | from_string | 71.300 | 69.051 | 0.968 | 0.924 | 70.496 | 68.711 | 0.975 | `ede5b72707a1981f` |
| oid | from_prefix | 40.656 | 40.824 | 1.004 | 1.058 | 40.663 | 40.713 | 1.001 | `dc31df4b1b390459` |
| oid | fmt | 18.012 | 18.504 | 1.027 | 1.094 | 18.016 | 18.509 | 1.027 | `426cda47b77674dd` |
| oid | tostr | 47.878 | 47.952 | 1.002 | 0.940 | 47.885 | 47.750 | 0.997 | `b9ba3214af5be548` |
| oid | pathfmt | 17.966 | 18.169 | 1.011 | 0.999 | 17.984 | 18.126 | 1.008 | `3f269283823fb96e` |
| oid | sort_cmp | 58.494 | 57.969 | 0.991 | 1.050 | 57.614 | 57.962 | 1.006 | `1ed33f9209a526f0` |
| oid | equal_ncmp_streq | 63.042 | 64.878 | 1.029 | 0.976 | 62.114 | 64.318 | 1.035 | `000000000010331d` |
| oid | shorten | 15.365 | 15.244 | 0.992 | 0.899 | 15.314 | 15.233 | 0.995 | `000000000018279e` |
| odb | loose_read | 450.203 | 440.148 | 0.978 | 0.875 | 448.987 | 438.878 | 0.977 | `dcb34697acc6bf9c` |
| odb | loose_read_header | 333.432 | 328.477 | 0.985 | 0.992 | 332.775 | 328.366 | 0.987 | `aab2c86c2631f430` |
| odb | loose_exists | 200.296 | 174.888 | 0.873 | 1.016 | 200.308 | 174.876 | 0.873 | `0000000000002710` |
| odb | loose_foreach | 31.235 | 32.606 | 1.044 | 1.038 | 31.240 | 32.578 | 1.043 | `000085350053a4d5` |
| odb | loose_write | 386.347 | 402.720 | 1.042 | 1.056 | 368.100 | 369.238 | 1.003 | `47d99484212157be` |
| odb | loose_write_existing | 48.302 | 50.436 | 1.044 | 1.055 | 44.290 | 48.472 | 1.094 | `47d99484212157be` |
| odb | pack_read_random | 946.867 | 946.676 | 1.000 | 0.986 | 946.869 | 942.107 | 0.995 | `932f2a35271653a5` |
| odb | pack_read_sequential | 939.447 | 943.472 | 1.004 | 0.994 | 938.091 | 938.921 | 1.001 | `1a3e2a9d80ff4b04` |
| odb | pack_read_header | 178.341 | 181.291 | 1.017 | 1.021 | 178.355 | 179.943 | 1.009 | `40230ebf4c3d01c6` |
| odb | pack_exists | 183.191 | 188.578 | 1.029 | 1.037 | 183.202 | 188.341 | 1.028 | `000000000000c350` |
| odb | pack_exists_prefix | 99.587 | 101.336 | 1.018 | 1.445 | 99.596 | 101.089 | 1.015 | `0f2216f2705aeafa` |
| odb | pack_read_prefix | 478.541 | 493.516 | 1.031 | 1.019 | 477.083 | 483.784 | 1.014 | `c065415bfc3441ac` |
| odb | pack_foreach | 207.161 | 208.588 | 1.007 | 0.963 | 205.890 | 208.601 | 1.013 | `00067a82008c3edb` |
| odb | mempack_write | 267.229 | 266.821 | 0.998 | 0.997 | 267.209 | 266.544 | 0.998 | `c5e27b2a23c35ff2` |
| odb | mempack_read | 504.603 | 497.950 | 0.987 | 0.957 | 504.151 | 497.976 | 0.988 | `8e1d7b69486a307d` |
| odb | mempack_reset | 5.893 | 5.766 | 0.978 | 1.018 | 5.900 | 5.772 | 0.978 | `0000000000000000` |
| delta | read_deltified_random | 823.768 | 822.177 | 0.998 | 0.993 | 823.679 | 821.996 | 0.998 | `74fa7a61c2b4dac2` |
| delta | read_deep_chains | 255.150 | 256.001 | 1.003 | 0.987 | 255.009 | 255.823 | 1.003 | `46b909175810c375` |
| delta | read_header_deltified | 208.446 | 209.396 | 1.005 | 0.982 | 208.399 | 209.410 | 1.005 | `e1ef0f00e537805c` |
| delta | indexer_resolve | 1738.265 | 1757.044 | 1.011 | 0.983 | 1711.736 | 1721.576 | 1.006 | `3eee8855b4b539e5` |
| objects | commit_lookup_all | 2817.109 | 2888.149 | 1.025 | 1.055 | 2815.005 | 2883.972 | 1.024 | `efcdd6b75776bdbf` |
| objects | cache_hit | 185.736 | 180.376 | 0.971 | 0.922 | 185.750 | 180.368 | 0.971 | `00000000000710c0` |
| objects | tree_lookup_iterate | 795.696 | 794.713 | 0.999 | 1.016 | 795.698 | 794.717 | 0.999 | `dcae1b8c400101c9` |
| objects | tree_walk | 70.290 | 68.228 | 0.971 | 0.972 | 70.283 | 68.237 | 0.971 | `3fef9f660091be70` |
| objects | tree_entry_bypath | 69.807 | 71.715 | 1.027 | 0.996 | 69.825 | 71.525 | 1.024 | `000000000058acd2` |
| objects | blob_lookup | 915.255 | 909.320 | 0.994 | 0.844 | 914.547 | 908.853 | 0.994 | `0aad68ee175dabe7` |
| objects | tag_lookup_all | 30.407 | 30.047 | 0.988 | 0.991 | 29.825 | 29.610 | 0.993 | `8906a379569d74c3` |
| objects | signature_parse | 39.943 | 39.943 | 1.000 | 1.012 | 39.417 | 39.949 | 1.013 | `db888788c5b793d2` |
| objects | signature_new | 13.911 | 14.831 | 1.066 | 0.980 | 13.915 | 14.835 | 1.066 | `000000000012d838` |
| objects | commit_create_buffer | 87.392 | 91.046 | 1.042 | 1.024 | 87.028 | 91.054 | 1.046 | `ab90de869972f0a1` |
| revwalk | cold_master_unsorted | 2614.872 | 2681.751 | 1.026 | 1.131 | 2613.497 | 2638.194 | 1.009 | `014420d921298666` |
| revwalk | cold_master_time | 2712.728 | 2563.929 | 0.945 | 0.888 | 2705.349 | 2551.979 | 0.943 | `0144202ad2a4857e` |
| revwalk | cold_master_topo | 2770.586 | 2725.815 | 0.984 | 0.978 | 2737.307 | 2725.755 | 0.996 | `0144206c439ea9a2` |
| revwalk | cold_all_refs | 2806.994 | 2788.717 | 0.993 | 1.008 | 2744.381 | 2786.205 | 1.015 | `0151114e150f2fe9` |
| revwalk | master_unsorted | 347.870 | 334.498 | 0.962 | 0.886 | 344.459 | 334.519 | 0.971 | `014420d921298666` |
| revwalk | master_time | 392.890 | 402.876 | 1.025 | 1.117 | 392.631 | 386.844 | 0.985 | `0144202ad2a4857e` |
| revwalk | master_topo | 386.304 | 383.305 | 0.992 | 1.041 | 386.325 | 382.032 | 0.989 | `0144206c439ea9a2` |
| revwalk | master_topo_time | 398.427 | 399.011 | 1.001 | 1.154 | 396.584 | 396.851 | 1.001 | `014420e5e3a97a3a` |
| revwalk | master_time_reverse | 424.251 | 429.891 | 1.013 | 0.982 | 424.135 | 429.891 | 1.014 | `0144208408aab15a` |
| revwalk | master_first_parent | 102.310 | 101.111 | 0.988 | 0.841 | 102.318 | 101.120 | 0.988 | `005fc2899a391c41` |
| revwalk | all_refs | 363.549 | 362.322 | 0.997 | 0.964 | 361.170 | 362.341 | 1.003 | `0151114e150f2fe9` |
| revwalk | range_2.30_2.47 | 211.761 | 210.341 | 0.993 | 0.844 | 209.611 | 208.312 | 0.994 | `0034f47edfa49331` |
| revwalk | range_2.30_2.47_topo | 217.257 | 208.453 | 0.959 | 1.169 | 195.006 | 208.437 | 1.069 | `0034f4cf32284efd` |
| revwalk | hide_ref_time | 106.085 | 105.323 | 0.993 | 1.021 | 106.093 | 105.331 | 0.993 | `0026cabfd40b71ba` |
| revwalk | release_ranges_x17 | 801.025 | 804.572 | 1.004 | 1.049 | 801.023 | 804.609 | 1.004 | `4f08de45cb2354ad` |
| revwalk | reset_reuse_x17 | 397.315 | 398.111 | 1.002 | 0.970 | 393.867 | 392.472 | 0.996 | `c5b51e0847efe4d6` |
| refs | open_lookup_packed_x100 | 79.534 | 86.063 | 1.082 | 1.362 | 38.806 | 42.864 | 1.105 | `b9cf1d5157631e45` |
| refs | lookup | 931.771 | 870.214 | 0.934 | 1.137 | 465.908 | 435.161 | 0.934 | `31716cd583e78139` |
| refs | name_to_id | 714.319 | 849.615 | 1.189 | 1.303 | 357.321 | 426.281 | 1.193 | `31716cd583e78139` |
| refs | dwim | 607.558 | 610.865 | 1.005 | 1.149 | 304.526 | 304.851 | 1.001 | `9913a7ad42c249ab` |
| refs | iterate_all_names | 765.908 | 752.696 | 0.983 | 0.741 | 384.433 | 375.945 | 0.978 | `00dadcb21b71a80b` |
| refs | iterate_all_refs | 780.872 | 798.675 | 1.023 | 1.130 | 390.443 | 398.602 | 1.021 | `0000dadc006d22c1` |
| refs | iterate_glob | 504.313 | 491.036 | 0.974 | 0.755 | 252.345 | 244.008 | 0.967 | `0003e8e483afb8e5` |
| refs | foreach_glob_tags | 352.096 | 352.367 | 1.001 | 0.874 | 174.915 | 175.377 | 1.003 | `0000000000111245` |
| refs | create_loose | 1871.309 | 1772.615 | 0.947 | 1.080 | 841.951 | 823.667 | 0.978 | `01bd1d70b70d3405` |
| refs | delete_loose | 3006.543 | 3443.719 | 1.145 | 0.939 | 1400.661 | 1339.891 | 0.957 | `000000e484222325` |
| refs | delete_packed | 2699.880 | 2751.176 | 1.019 | 0.955 | 1308.069 | 1290.897 | 0.987 | `00c33cee3f5d1022` |
| refs | update_with_reflog | 1798.739 | 1827.919 | 1.016 | 1.026 | 911.328 | 872.536 | 0.957 | `00000000000003e9` |
| refs | reflog_read_10k_entries_x5 | 256.394 | 258.455 | 1.008 | 0.958 | 130.828 | 130.283 | 0.996 | `48c5f2068c76c350` |
| refs | reflog_read_2000_reflogs | 115.899 | 182.672 | 1.576 | 1.005 | 108.716 | 113.952 | 1.048 | `1eec3e174d574e20` |
| refs | reflog_append_write | 150.281 | 133.470 | 0.888 | 0.972 | 78.445 | 76.209 | 0.971 | `0edd26c939c427d8` |
| config | open_big_x20 | 1153.923 | 1171.425 | 1.015 | 0.863 | 1153.829 | 1170.712 | 1.015 | `00000000000001a4` |
| config | open_small_x2000 | 142.776 | 135.675 | 0.950 | 1.035 | 142.741 | 135.684 | 0.951 | `00000000000007d0` |
| config | get_string_snapshot | 17.091 | 16.321 | 0.955 | 0.675 | 16.954 | 16.318 | 0.962 | `aed84d7e83584bcb` |
| config | get_bool_int_snapshot | 36.811 | 36.624 | 0.995 | 1.735 | 36.818 | 36.632 | 0.995 | `00000000009c6710` |
| config | get_entry_live | 300.566 | 304.571 | 1.013 | 0.933 | 300.585 | 304.588 | 1.013 | `bbaf2f90f46de508` |
| config | iterate_all | 68.764 | 66.313 | 0.964 | 0.914 | 68.635 | 66.321 | 0.966 | `009c60d2e9f6a3de` |
| config | iterate_glob | 43.168 | 43.220 | 1.001 | 1.012 | 42.678 | 42.439 | 0.994 | `0007d028c047db3c` |
| config | multivar_iterate | 129.296 | 133.080 | 1.029 | 1.430 | 129.270 | 128.870 | 0.997 | `947612e8a04524c0` |
| config | multivar_foreach | 601.952 | 628.164 | 1.044 | 1.039 | 601.878 | 625.913 | 1.040 | `60838e0580ec52b0` |
| config | snapshot_x5 | 369.329 | 359.406 | 0.973 | 1.240 | 369.327 | 359.427 | 0.973 | `0000000000000032` |
| config | set_big_x10 | 561.312 | 556.572 | 0.992 | 0.742 | 558.666 | 556.601 | 0.996 | `01e7fd293ee4635b` |
| config | set_small_x400 | 504.026 | 515.588 | 1.023 | 0.901 | 493.057 | 500.550 | 1.015 | `78045b8684cd59d8` |
| config | delete_big_x10 | 700.505 | 697.966 | 0.996 | 1.068 | 699.077 | 695.047 | 0.994 | `1834afdf1fc617cb` |
| config | repository_config_x500 | 137.921 | 135.079 | 0.979 | 0.785 | 137.931 | 135.088 | 0.979 | `00000000000001f4` |
| config | parse_bool_int | 21.529 | 21.805 | 1.013 | 1.283 | 21.533 | 21.810 | 1.013 | `00002985e2162010` |
| index | read_100k | 223.706 | 236.671 | 1.058 | 1.102 | 219.432 | 234.341 | 1.068 | `b9d98eb2eb00ac6b` |
| index | read_git_x10 | 75.167 | 90.329 | 1.202 | 0.856 | 75.173 | 90.337 | 1.202 | `ec385a8e65333760` |
| index | write_100k | 229.856 | 225.818 | 0.982 | 1.085 | 217.817 | 216.685 | 0.995 | `22f0e2201dace099` |
| index | write_git | 12.159 | 12.346 | 1.015 | 1.098 | 12.065 | 12.147 | 1.007 | `ef04e49736db21fe` |
| index | find_bypath_100k | 275.510 | 264.206 | 0.959 | 0.987 | 269.974 | 262.825 | 0.974 | `000000012ac8478c` |
| index | find_prefix | 12.031 | 12.114 | 1.007 | 0.751 | 11.971 | 12.119 | 1.012 | `000000001dc5c3e0` |
| index | add_remove_100k | 1126.989 | 1115.734 | 0.990 | 1.000 | 1121.771 | 1115.749 | 0.995 | `7a7d25999ea53dbc` |
| index | remove_directory | 1006.851 | 1002.001 | 0.995 | 0.969 | 1005.854 | 1001.263 | 0.995 | `ea9aa18304d66022` |
| index | conflicts | 1270.253 | 1245.891 | 0.981 | 0.994 | 1270.276 | 1232.191 | 0.970 | `453a5b2cb4a9e763` |
| index | read_tree_x10 | 111.601 | 107.569 | 0.964 | 0.682 | 111.612 | 107.093 | 0.960 | `ec385a8e65333760` |
| index | write_tree_git | 49.290 | 53.714 | 1.090 | 1.306 | 49.298 | 53.422 | 1.084 | `154f5a592fc11533` |
| repository | open_bare_x1000 | 223.605 | 216.408 | 0.968 | 1.167 | 223.618 | 213.635 | 0.955 | `0000000000006590` |
| repository | open_workdir_x1000 | 261.929 | 252.020 | 0.962 | 0.808 | 261.943 | 249.225 | 0.951 | `0000000000007148` |
| repository | open_search_x1000 | 435.116 | 432.404 | 0.994 | 1.119 | 407.699 | 422.282 | 1.036 | `0000000000007148` |
| repository | discover_x1000 | 172.171 | 171.005 | 0.993 | 1.007 | 171.110 | 170.792 | 0.998 | `0000000000007148` |
| repository | init_x50 | 309.148 | 303.712 | 0.982 | 0.874 | 283.359 | 278.254 | 0.982 | `0000000000000096` |
| repository | init_ext_x50 | 351.621 | 338.052 | 0.961 | 0.897 | 304.235 | 294.062 | 0.967 | `0000000000000190` |
| repository | head_packed | 240.581 | 225.193 | 0.936 | 0.988 | 240.595 | 225.206 | 0.936 | `ffb0ab840e58baf5` |
| repository | head_loose | 309.713 | 296.735 | 0.958 | 0.956 | 309.510 | 277.072 | 0.895 | `35ce7bea46827b25` |
| repository | revparse_head | 163.659 | 170.320 | 1.041 | 1.037 | 163.156 | 165.931 | 1.017 | `000000000005d048` |
| hashsig | create_files_normal | 399.411 | 403.847 | 1.011 | 1.078 | 390.295 | 399.190 | 1.023 | `000000000006bc6a` |
| hashsig | create_files_ignore_ws | 478.294 | 458.922 | 0.959 | 0.945 | 471.899 | 458.236 | 0.971 | `0000000000070a88` |
| hashsig | create_files_smart_ws | 432.621 | 442.067 | 1.022 | 1.116 | 432.605 | 441.703 | 1.021 | `000000000006bc6a` |
| hashsig | create_1MiB_x16 | 109.965 | 111.847 | 1.017 | 0.920 | 109.974 | 111.855 | 1.017 | `0025b4876b34e41a` |
| hashsig | create_16MiB_x2 | 219.760 | 214.112 | 0.974 | 1.005 | 219.657 | 213.717 | 0.973 | `0000000000000063` |
| hashsig | compare | 135.448 | 135.977 | 1.004 | 1.069 | 133.739 | 135.890 | 1.016 | `00000000000bb1f3` |
| hashsig | create_fromfile | 228.075 | 237.519 | 1.041 | 1.005 | 227.891 | 234.996 | 1.031 | `00000000000178bd` |
| util | buf_commit_header_field | 9.109 | 9.461 | 1.039 | 1.041 | 9.115 | 9.389 | 1.030 | `f16092feacd4f299` |
| util | buf_reuse | 7.836 | 6.490 | 0.828 | 0.944 | 7.701 | 6.496 | 0.843 | `8539fcb895796325` |
| util | buf_object_short_id | 11.761 | 11.794 | 1.003 | 1.009 | 11.751 | 11.761 | 1.001 | `07f52911ac5af635` |
| util | strarray_reference_list_56k | 422.149 | 424.814 | 1.006 | 1.046 | 415.026 | 416.244 | 1.003 | `0000012c0800dadc` |
| util | strarray_tag_list_x5 | 63.764 | 63.284 | 0.992 | 0.978 | 63.620 | 62.605 | 0.984 | `0000003b74ad0444` |
| util | strarray_tag_list_match_x20 | 250.329 | 247.310 | 0.988 | 0.984 | 250.344 | 247.285 | 0.988 | `00000000000000c8` |

## C vs Rust

Not yet recorded: the port build does not exist yet. Run
`rust/bench/compare.sh /work/build-c <port-build>` and add the table here,
with the date and the load average from `results/<run>/info.txt`.
