# wave-194/batch-7/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-7`, on `crustify/batches/wave-194/batch-7`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-7/run-1/task.json` in it.
- Batch: `wave-194/batch-7` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- This is `deps/llhttp/api.c`'s module pass: the module's `port` is whole on this wave's
  branch (waves 194 to 196 folded), in `src/libgit2/deps/llhttp/api.rs`: the callback table
  `Settings` kept in its C layout (`wrap`, until wave 1165), 60 symbols, 13 of them ported
  onto accessors of `src/libgit2/deps/llhttp/llhttp.rs`, and 31 exports under
  `ffi-export-deps-llhttp-api-c`, off. Twelve units of `llhttp.c` are bound ahead in
  `llhttp.rs`, `llhttp.c`'s own module, which its own run ports and passes later: read them,
  and report on `api.c`'s module. Its landings were reviewed in the wave: the types by batch
  5 (`f58d38aa9`), the symbols by batch 6 (`2f0cf4265`), which widened the contracts of
  `set_settings_ptr` and `set_data` to "for as long as the parser holds it". Review the
  module whole, across its batches: its unsafe regions, its exports and its tests.
- The module switches from C at its integration, right after you land: its export feature
  is deleted, `api.c` leaves `c/`, and the C that remains, `llhttp.c`'s generated parser
  (the 23 dispatchers) and libgit2's HTTP client (`src/libgit2/transports/httpparser.c`,
  `httpparser.h`'s macros, `httpclient.c`), calls the 31 exports. Review them for what that C
  relies on. `deps/llhttp/http.c`'s run starts after the switch. The module's equivalence
  tests call 19 `api.c` functions no other C calls, so they retire at the switch: say which
  behaviour they established is kept by which unit test, and which is not.
- The module has no C tests tied to it by name; libgit2's online and offline HTTP tests
  exercise it through the client. Say what checks it after the switch.
- Your gates are the wave's and the module's end, without BSan, and with the export feature
  on as well: check and lint the crate with it, and run the workspace's tests with the
  exports replacing C's `api.c.o` in a scratch copy of the archive, without changing `c/` or
  the shared builds, as batches 4 and 6 did.
- The wave's tree is past `src/util/utf8.c`'s switch from C: `crustify/builds` in your
  worktree links to the variants' new generation, `crustify/builds.gen1`. Read the switch's
  Notes entry in the dashboard and its lesson in `crustify/LESSONS.md` before your first
  build. `crustify/scripts/c-tests.sh` runs the target's remaining C tests over the link
  with the Rust static library.
- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- Your gates are the wave's and the module's end, without BSan: the workspace's tests on
  plain and under ASan/UBSan and TSan, and Miri per test binary, on the wave's tip before any
  change and again on your landed tree where your fix could affect them.
- The dashboard's usage cells are the assistant's; fill your landing cells only.
- Five batches of four modules run at once on this host (`wave-186/batch-0` and `batch-1`,
  `wave-194/batch-0`, `wave-200/batch-0`, `wave-229/batch-0`), and a side campaign's runs may
  join them. Cap your builds and tests at 24 jobs (`CARGO_BUILD_JOBS=24`,
  `RUST_TEST_THREADS=24`, and the same for C builds). Keep every scratch file, log and scratch
  build in your worktree's `crustify/tmp/`, and delete scratch builds you no longer need
  before you end: the disk is 92% full. End only processes of your own worktree.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- The tree changed on 2026-11-13: the safe crate is `libgit2-crustify` (lib
  `libgit2_crustify`), the raw bindings are `sys/<link-unit>/` reached as
  `crate::sys::<link-unit>`, stable is Rust 1.100.0, and each variant's C is the static
  archive `crustify/builds/<variant>/crustify/libcrustify.a`. `crustify/builds` in your
  worktree is a link to the base checkout's, which you never rebuild.
- Known failures that are not yours: under the nightly variants (asan-ubsan), 8 `compile_fail`
  snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
  The plain suite passes at your base (2768 passed, 1 ignored).
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`, as `wave-225/batch-0` did.
- BSan is no regression gate of yours (the settings' `sanitizers` answer, from 2026-11-13, a
  departure from `testing-gates`' Regressions section the user asked for): never run
  `cargo bsan` over the workspace, a whole test binary or as a batch, wave or module-end
  gate; that runs once before each ship point. You may run it on small things: the tests
  your own work adds or changes, a reproducer and the regression test it becomes, each test
  named and under a `timeout`.
