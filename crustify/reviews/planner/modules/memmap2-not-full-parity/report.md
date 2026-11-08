# `memmap2` was recorded as a full-parity equivalent of src/util/unix/map.c

## What the earlier plan decided

`modules.json`, module `src/util/unix/map.c` (libgit2, index 50), `objective.rust_equivalent`:

```json
"std": null,
"crates": [{"crate": "memmap2", "feature_parity": "full", "license": "MIT OR Apache-2.0", "notes": ""}]
```

Under `port`, a `full` crate makes the module `wrap`.

## Evidence

The module owns four functions (src/util/unix/map.c, and `git__page_size`,
`git__mmap_alignment` selected from src/util/posix.h): `p_mmap`, `p_munmap`,
`git__page_size` and `git__mmap_alignment`. The last two are consumed elsewhere: pool.c
sizes its pages by `git__page_size` (`c/src/util/pool.c:30`), and indexer.c aligns the
offsets it maps the pack at by `git__mmap_alignment` (`c/src/libgit2/indexer.c:729, 758`).

memmap2 (0.9.8) maps files shared, private or read-only at any offset, which covers
`p_mmap`/`p_munmap`, but its page-size query is private:

```sh
grep -n "fn page_size" ~/.cargo/registry/src/*/memmap2-0.9.8/src/unix.rs   # 490: fn page_size() -> usize {   (no pub)
grep -n "^int" c/src/util/unix/map.c
grep -rn "git__page_size\|git__mmap_alignment" c/src --include=*.c
```

The standard library has no page-size query either.

## Edit

`memmap2` is `partial`, with what it lacks in its notes; the module's objective under
`port` is `port`. Downstream: `git_map`, `p_mmap`, `p_munmap`, `git__page_size` and
`git__mmap_alignment` are `port` items of its waves; `git_map`'s layout users in other
modules (mwindow.c, indexer.c, odb_loose.c, odb_pack.c, refdb_fs.c, commit_graph.c, midx.c, diff_file.c, deps/reftable/system.c, ...) make it `wrap` where the order places it and `port` after
their integration.
