# wave-197/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/http.c/wave-197/batch-1`, on `crustify/batches/wave-197/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/http.c/wave-197/batch-1/run-1/task.json` in it.
- Batch: `wave-197/batch-1` of the module `libgit2/deps/llhttp/http.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-197`. Name it `refs/heads/crustify/waves/wave-197` in every git command.

## Instructions

- This is `deps/llhttp/http.c`'s module pass and its only review: its run landed one
  translation batch, `wave-197/batch-0` (`a81dc3103`), the module's whole `port` (waves 197
  to 199 folded): five methods on the parser handles in `src/libgit2/deps/llhttp/http.rs`
  (`message_needs_eof`, `should_keep_alive`, `before_headers_complete`,
  `after_headers_complete`, `after_message_complete`) and four exports under
  `ffi-export-deps-llhttp-http-c`, off: the three hooks `llhttp.c`'s generated parser calls,
  checked against a `Hook` type copied from `llhttp.c`'s prototypes, no header declaring
  them, and `llhttp_should_keep_alive`. `llhttp_message_needs_eof` has no export.
- The module switches from C at its integration, right after you land: its export feature
  is deleted, `deps/llhttp/http.c` leaves `c/`, and `llhttp.c`'s generated parser calls the
  hooks on every response. Review the exports for what that C relies on, thoroughly: the
  hooks' return codes drive the parser's state machine. The batch kept one upstream quirk
  (`HTTP/2.0` treated as an earlier version for keep-alive) and returns
  `Result<Body, InvalidTransferEncoding>` from `after_headers_complete`.
- Your gates include the export feature on: check and lint the crate with it, run the
  workspace's tests with the exports replacing llhttp's `http.c.o` in a scratch copy of the
  archive (it holds two `http.c.o` members, the transport's first: the batch used `ar dN 2`
  and checked with `nm`), every test binary linked, `equiv` included with only the module's
  own equivalence file retired, and run `crustify/scripts/http-gate.sh` over that link.
  Say which behaviour the module's equivalence tests established is kept by which unit test
  once they retire, and add what is not.
- The batch filed `crustify/evolution/gateway-feature-across-a-multiline-attribute`, a defect
  of `build.rs` with a reproducer.
- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- Your gates are the wave's and the module's end, without BSan: the workspace's tests on
  plain and under ASan/UBSan and TSan, and Miri per test binary, on the wave's tip before any
  change and again on your landed tree where your fix could affect them.
- The dashboard's usage cells are the assistant's; fill your landing cells only.
- One other batch of the campaign runs beside you, and another campaign shares the host. Cap your
  builds and tests at 32 jobs (`CARGO_BUILD_JOBS=32`, `RUST_TEST_THREADS=32`, and the same for
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
