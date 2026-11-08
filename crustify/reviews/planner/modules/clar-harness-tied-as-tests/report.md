# The clar harness tied to modules as though it were their tests

## What the earlier plan decided

Planner runs 4 to 6 derived each module's `tests` with a script that took every file under
`tests/` and `deps/clar/` for a test (`crustify/tmp/planner/py/tests_gate.py`,
`istest=lambda f: f.startswith(('tests/','deps/clar/'))`). Six modules listed harness
files among their tests:

```json
// src/util/alloc.c (foundational)
{"path": "deps/clar/clar_libgit2_alloc.c", "items": {"included": [], "excluded": []}}
// src/util/errors.c
{"path": "deps/clar/clar_libgit2.c", "items": {"included": ["cl_git_report_failure"], "excluded": []}}
{"path": "deps/clar/main.c", "items": {"included": [], "excluded": []}}
// src/util/str.c
{"path": "deps/clar/clar_libgit2.c", "items": {"included": ["cl_fake_globalconfig", "cl_fake_homedir", "cl_getenv", "cl_git_path_url", "cl_git_remove_placeholders", "cl_git_sandbox_init", "cl_git_sandbox_path", "cl_sandbox_set_homedir", "cl_sandbox_set_search_path_defaults", "remove_placeholders_recurs"], "excluded": []}}
// src/libgit2/buf.c
{"path": "deps/clar/clar_libgit2.c", "items": {"included": [], "excluded": []}}
// src/libgit2/oid.c
{"path": "deps/clar/clar_libgit2.c", "items": {"included": ["cl_repo_commit_from_index"], "excluded": []}}
{"path": "deps/clar/clar_libgit2.h", "items": {"included": [], "excluded": []}}
// src/libgit2/signature.c
{"path": "deps/clar/clar_libgit2.c", "items": {"included": ["cl_repo_commit_from_index"], "excluded": []}}
```

Run 6 then read these ties when it planned the `port/unexport` stages
(`crustify/artifacts/planner/run-6/compose/stages.py`, `test_wait`): a call from
`clar_libgit2.c` counted as a test that leaves with the first of `str.c`, `buf.c`, `oid.c`
and `signature.c` to be integrated. Run 6 planned 22 `port/unexport` items that only
the harness's calls held back, after that first integration:

```text
git__getenv (wave 1702), git_buf_dispose (1713), git_error_last (1682),
git_fs_path_prettify (1851), git_fs_path_prettify_dir (1842), git_fs_path_to_dir (1857),
git_index_write_tree (2104), git_libgit2_init (2268), git_libgit2_shutdown (2268),
git_oid_equal (1829), git_oid_from_prefix (1812), git_oid_hexsize (1830), git_oid_type (1819),
git_signature_now (2053), git_str_copy_cstr (1637), git_str_detach (1668),
git_str_joinpath (1666), git_str_len (1665), git_str_putc (1663), git_str_puts (1667),
git_str_swap (1664), p_read (1897)
```

## The evidence against it

`deps/clar` is the harness, not a test. It defines no clar test case, and both test
executables compile all of it, beside every test file:

```sh
cd /target/libgit2-bindings/c
grep -c '^void test_' deps/clar/*.c                       # 0 in every file
sed -n 15,28p tests/libgit2/CMakeLists.txt                # SRC_CLAR: GLOB_RECURSE ${CLAR_PATH}/*.c
sed -n 15,45p tests/util/CMakeLists.txt
```

Every test reaches its fixtures through it (`cl_git_sandbox_init`, `cl_repo_commit_from_index`,
`cl_git_report_failure` behind `cl_git_pass`). The decomposition's Bundling ties a test to
the modules whose subject it is. The harness has no subject: it only sets up and tears
down the fixtures of every test. If it were tied to `oid.c`, the integration of `oid.c`
would remove `clar_libgit2.c`, which every remaining test links.

The harness stays as long as any C test does, and some C tests stay for good: those tied
to the foundational modules, such as `tests/util/alloc.c`, whose modules are never
integrated. Every export the harness reaches is therefore needed for good. It reaches 68
ported functions and globals:

```sh
cd /target/libgit2-bindings
python3 -I crustify/artifacts/planner/run-7/compose/testfacts.py
python3 -I crustify/artifacts/planner/run-7/compose/ties.py
python3 -I crustify/artifacts/planner/run-7/compose/stages7.py   # "test files keeping them": deps/clar/*
```

## The edit and what it moves

- `modules.json`: `deps/clar` is no module's test. `crustify/artifacts/planner/run-7/compose/testfacts.py`
  reads test functions under `tests/` only, and the `_comment` says why.
- `schedule.json`: the 22 items above are no longer planned, since the harness keeps their
  exports for good. In all, 70 exports are kept for good: 68 that `deps/clar` reaches
  (`git_error_set_oom` among them, which `alloc.c` and `strlist.c` also call),
  `git_error_set`, which only foundational C calls (`win32_leakcheck.c`), and
  `git_stream_register`, which the `__cleanup` of `tests/libgit2/stream/deprecated.c`
  calls. That suite tests a deprecated API, and the configured build compiles its test
  cases empty, so it is tied to no module. Their stage waves
  are re-planned with the rest, as `crustify/artifacts/planner/run-7/compose/write7.py`
  composes them.

## Questions for the user

None.
