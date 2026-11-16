# Benchmarks of the ported core

C programs that use libgit2's public API, built unchanged against the C
reference build and against the Rust port (same C ABI), so the two can be
compared on the same work. One program per component (`src/bench_<component>.c`),
a shared harness (`src/bench.h`), and scripts:

| file | purpose |
|---|---|
| `prepare-data.sh` | creates the datasets under `$BENCH_DATA` (idempotent) |
| `run.sh <builddir> [filter]` | builds against `<builddir>`, runs, writes TSV + JSON |
| `compare.sh <builddir-C> <builddir-Rust> [filter]` | runs both, interleaved, prints the comparison table |
| `report.py` | TSV to JSON, and the aggregation/table of `compare.sh` |
| `RESULTS.md` | recorded numbers (C-vs-C baseline, later C-vs-Rust) |

## Running

```sh
rust/bench/prepare-data.sh                     # once; needs $BENCH_DATA/git.git
rust/bench/run.sh /work/build-c                # whole suite against one build
rust/bench/run.sh /work/builds/foo odb,revwalk/master_time
rust/bench/compare.sh /work/build-c /work/builds/foo           # C vs Rust
LABEL_A=C1 LABEL_B=C2 rust/bench/compare.sh /work/build-c /work/build-c   # noise level
```

Each program is compiled with
`$CC $BENCH_CFLAGS -I<builddir>/include -I<src>/include -L<builddir> -lgit2 -Wl,-rpath,<builddir>`
into `rust/bench/build/<builddir path>/` (rebuilt when the source, `bench.h` or
`libgit2.so` is newer). The build directory only needs `libgit2.so` (and
optionally `include/`, the generated headers); nothing internal is linked.

**Filter**: comma-separated patterns, matched against `component/workload`
(and against `component` alone). A pattern without glob characters matches
as a substring (`odb` = `*odb*`); with `*?[` it is an `fnmatch(3)` pattern
(`'revwalk/cold_*'`).

Environment:

| variable | default | meaning |
|---|---|---|
| `BENCH_DATA` | `/work/bench-data` | datasets |
| `BENCH_CPU` | `2` | CPU list for `taskset -c`; `none` to not pin |
| `BENCH_REPS` | 10 (`run.sh`), 3 per round (`compare.sh`) | timed repetitions |
| `BENCH_WARMUP` | 2 (`run.sh`), 1 per round (`compare.sh`) | untimed warm-up runs |
| `BENCH_ROUNDS` | 3 | `compare.sh`: rounds per build, interleaved |
| `BENCH_OUT` | `rust/bench/results/<kind>-<timestamp>` | output directory |
| `BENCH_TMP` | `$TMPDIR` or `/tmp` | parent of the scratch directory |
| `CC`, `BENCH_CFLAGS` | `clang`, `-O2 -g -std=c11 -Wall -Wextra ...` | compiler |

A whole `run.sh` takes about 15 minutes with the defaults on the reference
machine; `compare.sh` with the defaults about 40 (both under heavy machine
load; see RESULTS.md).

## Method

For each workload the harness runs `prep()` once (untimed: collecting ids,
loading inputs), then `BENCH_WARMUP` + `BENCH_REPS` runs; `setup()` and
`teardown()` around each run are untimed (opening a fresh repository/odb,
copying an index or config file to a scratch directory, deleting it). The
timed `run()` is measured with `CLOCK_MONOTONIC`. Reported: the median and
the minimum wall time, and every sample.

`run()` returns a 64-bit **check value** computed from the results of the
work (counts, object ids, sizes, sampled content checksums, file contents
written). It must be the same in every repetition (otherwise the harness
prints `INCONSISTENT` and exits 1) and the same for both builds:
`compare.sh` flags `CHECK-MISMATCH` when they differ, `INCONSISTENT(<build>)`
when a build's checks differ across repetitions or rounds, and
`MISSING(<build>)` when a workload produced no result (crash); it exits 1 if
anything is flagged. Content reads are additionally verified by libgit2
itself (strict hash verification is on by default, so every object read is
re-hashed). Check values do not depend on scratch paths, process ids or
times.

Output of a program: one TSV line per workload,
`component workload reps median_ns min_ns ops check samples_ns`
(`ops` = operations per run, for per-operation figures); `#` lines are
comments. `run.sh` concatenates them into `results.tsv` and writes
`results.json`. `compare.sh` keeps the raw outputs
(`<label>/round<r>-<NN>-<program>.out`) and writes `compare.tsv`,
`compare.json` and `table.txt`; the medians and minima it reports are over
all samples of all rounds; `ratio` = B median / A median (Rust / C: below 1
means the port is faster), `minrat` the same for minima.

Interleaving: for each program, `compare.sh` runs round 1 as A then B,
round 2 as B then A, and so on, so slow drifts of the machine affect both
builds alike.

Caches: "cold" workloads open a new repository/odb in `setup()`, so
libgit2's object cache, odb cache and pack delta-base cache start empty; the
OS page cache is warm after the warm-up runs (the datasets fit in memory),
so no workload measures disk I/O of the data. Writes go to `$BENCH_TMP`.
The configuration search paths (system, global, XDG) are set to an empty
directory, so the user's configuration does not affect the results.

## Data (`prepare-data.sh`)

All derived deterministically from `git.git` (a bare clone of git/git).

| dataset | contents |
|---|---|
| `git.git` | bare clone of git/git: one 330 MB pack, 424,578 objects, 86,289 commits, 1,027 refs (packed) |
| `git-wd` | non-bare clone of git.git (shared objects) checked out at v2.47.0: real index of 4,524 entries with a TREE extension; `t/a/b/.../h` for discovery |
| `bigindex` | synthetic index of 100,000 entries (100 dirs × 10 subdirs × 100 files, real blob ids), TREE extension |
| `loose.git` | 34,101 loose objects (all objects of v2.40.0..v2.47.0, unpacked; 870 MB inflated) |
| `refs.git` | objects via alternates to git.git; `packed-refs` with 53,027 refs (git.git's 1,027 incl. peeled tags, 50,000 synthetic tags, 2,000 branches), 5,000 loose branches (2,000 of them shadowing packed ones), each with a 10-entry reflog; `refs/heads/main` (loose) with a 10,000-entry reflog |
| `config/big.config` | ~22,000 entries: 2,000 `remote` sections (url, 2 × fetch, pushurl with comment), 2,000 `branch` sections, a 2,000-value multivar with quoting and escapes, 8 `include.path` files of 500 entries each |
| `config/small.config` | a typical 11-entry repository config |
| `packs/v2.46.0-v2.47.0.pack` | self-contained pack of 5,501 objects (deltified), made with `pack.threads=1` so it is reproducible |

## Workloads

Counts are per timed run.

### oid (`bench_oid`) — the 424,578 ids of git.git's pack, shuffled
- `from_string`: `git_oid_from_string` of every hex id.
- `from_prefix`: `git_oid_from_prefix` with lengths 7–40.
- `fmt`, `tostr`, `pathfmt`: `git_oid_fmt`; `git_oid_tostr` with every buffer size 1–41 and `git_oid_tostr_s`; `git_oid_pathfmt`.
- `sort_cmp`: `qsort` of 100,000 ids with `git_oid_cmp`.
- `equal_ncmp_streq`: `git_oid_equal`, `git_oid_ncmp` (lengths 1–40), `git_oid_streq`, `git_oid_strcmp`, `git_oid_is_zero` over neighbours.
- `shorten`: `git_oid_shorten_add` of 20 sets of 12,000 ids (the trie holds at most 32,767 nodes), check = sum of returned lengths.

### odb (`bench_odb`) — fresh `git_odb_open` per run
- loose (`loose.git`): `loose_read` (2,000 random objects < 32 KiB: inflate + hash verification), `loose_read_header` (10,000 headers), `loose_exists` (10,000 present + 2,000 missing; a miss refreshes the backends), `loose_foreach` (enumerate the 34,101 objects), `loose_write` (500 new blobs of 64 B–64 KiB into an empty odb: deflate, hash, temp file + rename), `loose_write_existing` (the same 500 again: exists/freshen path).
- pack (`git.git`): `pack_read_random` (2,000 random objects, mostly deep deltas), `pack_read_sequential` (2,000 consecutive objects in pack-offset order, trees and blobs, benefits from the delta-base cache), `pack_read_header` (20,000 random), `pack_exists` (50,000 present + 2,000 missing), `pack_exists_prefix` (20,000 7-hex-digit prefixes, `git_odb_exists_prefix`), `pack_read_prefix` (1,000 `git_odb_read_prefix`), `pack_foreach` (enumerate 424,578 ids).
- mempack: `mempack_write` (20,000 blobs into a `git_mempack_new` backend), `mempack_read` (read + header + exists of the 20,000, twice), `mempack_reset`.

### delta (`bench_delta`) — delta application through the public API
- `read_deltified_random`: 1,500 random ofs-delta objects of git.git (each read applies its whole chain).
- `read_deep_chains`: 300 objects with chains of depth ≥ 45 (mean 46.8).
- `read_header_deltified`: 20,000 `git_odb_read_header` of deltified objects (`git_delta_read_header` on the chain).
- `indexer_resolve`: `git_indexer` on `packs/v2.46.0-v2.47.0.pack` (64 KiB appends, commit): inflates every object, resolves every delta, hashes, writes .idx/.pack. Check = pack name + object/delta counts.

The internal `git_delta_apply`/`git_delta_create_from_index` are not exported from `libgit2.so`, so they are measured only through these paths.

### objects (`bench_objects`) — git.git
- `commit_lookup_all`: `git_commit_lookup` + every accessor (author, committer, message, summary, tree id, parent ids) of all 86,289 commits, cold repository.
- `cache_hit`: 5,000 cached commits looked up 40 times each with `git_commit_lookup` and `git_object_lookup(ANY)` (object cache hit path).
- `tree_lookup_iterate`: root trees of 300 commits (every 20th first-parent commit of v2.47.0) and, recursively, all their subtrees; every entry's name, mode, id.
- `tree_walk`: `git_tree_walk` (pre and post order) over 20 root trees.
- `tree_entry_bypath`: `git_tree_entry_bypath` for the 4,524 paths of v2.47.0, 10 times, plus `git_tree_entry_byname` at the top level (trees cached; the default 4 KiB tree cache limit is raised for this workload only).
- `blob_lookup`: the 4,524 blobs of v2.47.0, content checksum and `git_blob_is_binary`.
- `tag_lookup_all`: all 1,019 annotated tags: name, message, target, tagger.
- `signature_parse`: `git_signature_from_buffer` + `git_signature_dup` on 50,000 real author/committer lines.
- `signature_new`: 50,000 `git_signature_new` (trimming, validation).
- `commit_create_buffer`: 20,000 `git_commit_create_buffer` (serialisation of a commit).

### revwalk (`bench_revwalk`) — git.git (83k commits from master)
- cold (new repository per run; commits read from the pack during the walk): `cold_master_unsorted`, `cold_master_time`, `cold_master_topo`, `cold_all_refs` (`push_glob("refs/*")`, 86k commits).
- warm (every commit already in the odb cache; measures the walker itself): `master_unsorted`, `master_time`, `master_topo`, `master_topo_time`, `master_time_reverse`, `master_first_parent`, `all_refs`, `range_2.30_2.47` (`push_range`), `range_2.30_2.47_topo`, `hide_ref_time` (master minus v2.45.0), `release_ranges_x17` (17 walkers `v2.N.0..v2.N+1.0`), `reset_reuse_x17` (one walker, `git_revwalk_reset` between 17 push/hide walks).

Check = number of commits and a hash of the order.

### refs (`bench_refs`) — refs.git unless noted
- `open_lookup_packed_x100`: open the repository and look up one packed ref, 100 times (fresh refdb each time).
- `lookup`, `name_to_id`: 20,000 lookups, a quarter each loose, packed synthetic tag, packed real tag, missing.
- `dwim`: 5,000 `git_reference_dwim` of short names.
- `iterate_all_names` (`git_reference_next_name`), `iterate_all_refs` (`git_reference_next`): all 56,028 refs, loose and packed merged.
- `iterate_glob` (`refs/heads/loose/b1*`), `foreach_glob_tags` (`git_reference_foreach_glob("refs/tags/*")`, 51k).
- scratch repository with git.git's objects: `create_loose` (2,000 new branches), `delete_loose` (delete them), `delete_packed` (delete 20 packed refs from a copy of refs.git's 53k-line packed-refs, each rewriting it), `update_with_reflog` (1,000 `git_reference_set_target` with `core.logAllRefUpdates`, each appending to the reflog).
- reflog: `reflog_read_10k_entries_x5` (read and walk the 10,000-entry reflog 5 times), `reflog_read_2000_reflogs` (2,000 small reflogs), `reflog_append_write` (read the 10,000-entry reflog, append 200 entries, write, re-read).

### config (`bench_config`)
- `open_big_x20`: `git_config_open_ondisk(big.config)` (parse + 8 includes) and two gets, 20 times. `open_small_x2000`: small.config 2,000 times.
- on a snapshot: `get_string_snapshot` (20,000 `git_config_get_string`, including keys from includes), `get_bool_int_snapshot` (20,000 × bool, int64 with unit suffix, missing key), `iterate_all` (all entries of the snapshot and of the live config), `iterate_glob` (regex `^remote\..*\.url$`), `multivar_iterate` (the 2,000-value multivar, with and without a value regex, twice), `multivar_foreach` (`git_config_get_multivar_foreach` on 20 keys; each call scans the whole config).
- live: `get_entry_live` (5,000 × `git_config_get_entry` + `git_config_get_string_buf`), `snapshot_x5` (`git_config_snapshot` of the 22k-entry config).
- writes on a scratch copy: `set_big_x10`, `set_small_x400` (set string/int64/bool and add multivar values; each set rewrites the file), `delete_big_x10`. Check = hash of the resulting file.
- `repository_config_x500`: open git-wd, `git_repository_config_snapshot`, one get, 500 times.
- `parse_bool_int`: 100,000 × `git_config_parse_bool`/`_int32`/`_int64`.

### index (`bench_index`)
- `read_100k` (`git_index_open` of bigindex), `read_git_x10` (git-wd's index, 10 times).
- `write_100k`, `write_git`: `git_index_write` of a scratch copy (sorting, extensions, trailing checksum). Check = file content.
- `find_bypath_100k`: `git_index_find` + `git_index_get_bypath` of all 100,000 paths in random order. `find_prefix`: 10,000 `git_index_find_prefix`.
- `add_remove_100k`: 10,000 new entries + 10,000 replacements with `git_index_add`, then 10,000 `git_index_remove`.
- `remove_directory`: 20 `git_index_remove_directory` (1,000 entries each).
- `conflicts`: 5,000 three-stage `git_index_conflict_add`, `_get` of each, the conflict iterator, 2,500 `_remove`, `_cleanup`.
- `read_tree_x10`: `git_index_read_tree` of v2.47.0's tree into an in-memory index, 10 times (same check as `read_git_x10`: the trees and the index agree).
- `write_tree_git`: `git_index_write_tree_to` of an index without tree cache (4,524 entries, all trees computed and found in git.git).

### repository (`bench_repository`)
- `open_bare_x1000`, `open_workdir_x1000`: `git_repository_open_ext(NO_SEARCH)` of git.git / git-wd. `open_search_x1000`: from 9 directories below the work tree. `discover_x1000`: `git_repository_discover` from there.
- `init_x50` (alternately bare/non-bare), `init_ext_x50` (`MKPATH`, initial head, description).
- `head_packed` (git.git: HEAD → packed `refs/heads/master`), `head_loose` (refs.git: loose `refs/heads/main`): 3,000 `git_repository_head` + `git_repository_head_detached`. `revparse_head`: 3,000 `git_revparse_single("HEAD")`.

### hashsig (`bench_hashsig`)
- `create_files_normal`, `create_files_ignore_ws`, `create_files_smart_ws`: `git_hashsig_create` of the 4,524 files of v2.47.0 (0 B to 964 KiB) with each option set (`ALLOW_SMALL_FILES` with ignore-whitespace).
- `create_1MiB_x16`, `create_16MiB_x2`: buffers of real source text, pairs compared.
- `compare`: `git_hashsig_compare` of each file's signature with its 20 neighbours (90,480 comparisons).
- `create_fromfile`: `git_hashsig_create_fromfile` of 1,000 files of git-wd.

### util (`bench_util`) — `git_buf` and `git_strarray` through the public API
- `buf_commit_header_field`: 20,000 `git_commit_header_field` into fresh `git_buf`s (+ `git_buf_dispose`). `buf_reuse`: the same into one reused `git_buf`.
- `buf_object_short_id`: 2,000 `git_object_short_id` (unique abbreviation via odb prefix lookups).
- `strarray_reference_list_56k`: `git_reference_list` of refs.git + `git_strarray_dispose`.
- `strarray_tag_list_x5`: `git_tag_list` of git.git. `strarray_tag_list_match_x20`: `git_tag_list_match("v2.4?.0")`.

## Adding a workload

Add a `bench_t` entry `{ component, name, prep, setup, run, teardown, ctx, ops }`
to the program's table; `run` must return a check value that depends on the
results and not on paths, pids or time. A new program needs its name in
`BENCH_PROGRAMS` in `lib.sh`. Keep a run between ~10 ms and ~1 s.
