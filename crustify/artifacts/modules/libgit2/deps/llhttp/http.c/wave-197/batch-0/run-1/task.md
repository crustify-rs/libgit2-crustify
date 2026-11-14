# wave-197/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/http.c/wave-197/batch-0`, on `crustify/batches/wave-197/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/http.c/wave-197/batch-0/run-1/task.json` in it.
- Batch: `wave-197/batch-0` of the module `libgit2/deps/llhttp/http.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-197`. Name it `refs/heads/crustify/waves/wave-197` in every git command.

## Instructions

- Your worklist is `deps/llhttp/http.c`'s whole `port`: the five symbols of waves 197, 198
  and 199, folded into this batch on wave 197's branch, each wave building on the one
  before: `llhttp__before_headers_complete` and `llhttp_message_needs_eof`, then
  `llhttp__after_headers_complete` and `llhttp_should_keep_alive`, then
  `llhttp__after_message_complete`. The module's file is scaffolded at
  `src/libgit2/deps/llhttp/http.rs`. After you, the module pass follows, and the module
  switches from C.
- `deps/llhttp/api.c` is ported and switched (`21c5c5f47`): `src/libgit2/deps/llhttp/api.rs`
  holds its native API on the parser handles and 31 exports, and
  `src/libgit2/deps/llhttp/llhttp.rs` the parser struct, enums, callback types and maps
  bound ahead of `llhttp.c` (`Parser`, `ParserRef`, `ParserMut`, `Errno`, `Method`, `Status`,
  `ParserType`). Your one cut dependency, `llhttp__internal_s`, is that `Parser`: if your
  items read or write a field it has no accessor for (batch 0 left six of the generated
  parser's own fields without one, and the flag words raw), report it rather than reach the
  raw struct, unless the skills let the module's own import region read it.
- The C that remains calls your items: `llhttp.c`'s generated parser calls the three
  `llhttp__*` hooks, which no header declares (`api.c`'s 23 dispatchers were in the same
  case: their exports are checked against a function-pointer type copied from `llhttp.c`'s
  own prototypes, `crustify/evolution/exports-without-a-header-declaration`), and
  `llhttp.h` declares `llhttp_message_needs_eof` and `llhttp_should_keep_alive`. Write each
  exported item's gateway under the module's export feature, off, declared anew in
  `Cargo.toml` (the three before it were deleted at their switches), and run the module's
  tests with the exports replacing C's `http.c.o` in a scratch copy of the archive, linking
  every test binary, `equiv` included, with only your own equivalence file retired, as
  `api.c`'s module pass learned (`crustify/evolution/export-link-check`).
- `crustify/scripts/http-gate.sh` and `http-remote.py`, which api.c's switch added, drive
  libgit2's HTTP client over localhost through the generated parser, which calls your hooks
  on every response: use them on your scratch link, and say which of your five the run
  reaches.
- This batch runs alone on the campaign's side; another campaign shares the host. Cap your
  builds and tests at 48 jobs (`CARGO_BUILD_JOBS=48`, `RUST_TEST_THREADS=48`, and the same for
  C builds). Keep every scratch file, log and scratch build in your worktree's
  `crustify/tmp/`, and delete scratch builds you no longer need before you end. End only
  processes of your own worktree. A wait loop over `pgrep -f` matches its own command line
  and never ends: wait on a PID.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- Three modules have switched from C: `src/util/utf8.c`, `src/cli/opt.c` and
  `deps/llhttp/api.c`. `c/` is the fork's without them (`c54c0c180` on its local
  `fork/main`), `build.json` is at version 7, and your worktree's `crustify/builds` links to
  the variants' current generation, `crustify/builds.gen3`, whose link holds the safe crate's
  Rust static library; you never rebuild it. Read the three switches' Notes entries in the
  dashboard and `crustify/LESSONS.md` before your first build.
- Known failures that are not yours: under the nightly variants (asan-ubsan, tsan), 9
  `compile_fail` snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail
  doctest (E0624). The plain suite passes at your base.
- libc's wrapper is pinned at `24da93f`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`.
- Packaging in bundled mode: the procedure is `crustify/evolution/package-gate-procedure`,
  with the workspace's Git-source patch for ffibox, as opt.c's switch found.
- BSan is no regression gate of yours (the settings' `sanitizers` answer, from 2026-11-13, a
  departure from `testing-gates`' Regressions section the user asked for): never run
  `cargo bsan` over the workspace, a whole test binary or as a batch, wave or module-end
  gate; that runs once before each ship point. You may run it on small things: the tests
  your own work adds or changes, a reproducer and the regression test it becomes, each test
  named and under a `timeout`.
