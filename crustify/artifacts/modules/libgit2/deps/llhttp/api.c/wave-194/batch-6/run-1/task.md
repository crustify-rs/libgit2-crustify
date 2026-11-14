# wave-194/batch-6/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6`, on `crustify/batches/wave-194/batch-6`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6/run-1/task.json` in it.
- Batch: `wave-194/batch-6` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- This review batch takes the `symbol` route's landings of `deps/llhttp/api.c`'s folded
  waves (194 to 196): `wave-194/batch-1` (`2c69341c3`), which bound seven symbols of
  `llhttp.c` ahead of `api.c`, `wrap`, in `src/libgit2/deps/llhttp/llhttp.rs`
  (`Parser::new`, `ParserMut::reinit` and `parse`, the callback types `Callback` and
  `DataCallback` with `EventHandler` and `DataHandler`, and the X-macro maps on `Errno`,
  `Method` and `Status`); and `api.c`'s whole `port` of its 60 symbols: batch 3
  (`2a51ccb3f`, 28 symbols, 13 of them onto accessors `llhttp.rs` already had) and batch 4
  (`b3d6b9b37`, `init`, `reset`, `finish`, `Settings::new` natively, the three name
  functions, and the 23 `on_<slot>` dispatchers generated from one list), in
  `src/libgit2/deps/llhttp/api.rs`, with 31 exports under `ffi-export-deps-llhttp-api-c`,
  off. The types were reviewed apart (batch 5, `f58d38aa9`). The module pass follows you,
  then `api.c` switches from C.
- Depth, as `reviewing`'s Depth section gives it: batch 1's items are wrappers `llhttp.c`'s
  own waves port later, reviewed on their public API; `api.c`'s ports are for good; its
  exports serve C that is scheduled for porting (`llhttp.c`'s generated parser, which calls
  the 23 dispatchers, and libgit2's HTTP client, `src/libgit2/transports/httpparser.c` and
  `httpclient.c`): review them for what that C relies on. Points to judge: `parse` refusing
  a parser with no state and a callback that re-enters its own parse, with a panic
  (`crustify/evolution/callback-reenters-its-caller`); the dispatchers as safe public
  methods and the contracts of `set_settings_ptr` and `set_data` batch 4 tightened for them;
  the dispatchers' exports checked against a function-pointer type copied from `llhttp.c`'s
  prototypes, no header declaring them
  (`crustify/evolution/exports-without-a-header-declaration`); and the 13 items ported onto
  existing accessors, whose anchors sit in `llhttp.rs`
  (`crustify/evolution/port-onto-an-existing-accessor`).
- This batch carries no wave gates: the module pass does. Run the module's tests and the
  scoped gates of what you change, with the export feature off and on, and with the exports
  replacing C's `api.c.o` in a scratch copy of the archive, as batch 4 did.
- The wave's tree is past `src/util/utf8.c`'s switch from C: `crustify/builds` in your
  worktree links to the variants' new generation, `crustify/builds.gen1`. Read the switch's
  Notes entry in the dashboard and its lesson in `crustify/LESSONS.md` before your first
  build.
- No `objectives` are given, so every objective that applies runs, `fix` included. The
  dashboard's usage cells are the assistant's; fill your landing cells only.
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
