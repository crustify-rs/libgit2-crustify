# wave-194/batch-1/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-1`, on `crustify/batches/wave-194/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-1/run-1/task.json` in it.
- Batch: `wave-194/batch-1` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- This is the second wrap batch ahead of `deps/llhttp/api.c`'s `port` waves (194 to 196): the
  seven symbols of `deps/llhttp/llhttp.c` they use, as `wrap`, in `llhttp.c`'s file,
  `src/libgit2/deps/llhttp/llhttp.rs`: the callback types `llhttp_cb` and `llhttp_data_cb`,
  `llhttp__internal_init`, `llhttp__internal_execute`, and the X-macros `HTTP_ERRNO_MAP`,
  `HTTP_ALL_METHOD_MAP` and `HTTP_STATUS_MAP`. `llhttp.c`'s own waves port them later, keeping
  the safe API you give them. Your task's `module` is `deps/llhttp/api.c`, whose wave you
  land on; your items' homes are `llhttp.c`'s.
- `wave-194/batch-0` (`1ead111ee`) bound the types: `Parser` (`llhttp__internal_s`) with
  `ParserRef` and `ParserMut`, `Parser::zeroed()`, and the enums `Errno`, `Method`, `Status`
  and `ParserType` (185 variants over bindgen's constants). It left the real constructor,
  `llhttp__internal_init`, to you, kept `data` and `settings` as raw addresses behind two
  `unsafe fn` setters until `llhttp_settings_s` is bound (the next batch's, `api.c`'s own
  `wrap`), and recorded the `init` role of `llhttp__internal_init` in the ownership store.
  Read its Notes entry in the dashboard and `crustify/evolution/wrap-ahead-field-users`.
- The three `HTTP_*_MAP` macros are the X-macros the header's enums and `api.c`'s
  `llhttp_errno_name`, `llhttp_method_name` and `llhttp_status_name` expand; batch 0's enums
  already carry their constants. Bind what `api.c`'s `HTTP_ERRNO_GEN`, `HTTP_METHOD_GEN` and
  `HTTP_STATUS_GEN` ports need of them, as `binding-symbols`' Macros section leads, without
  duplicating the enums' tables.
- Packaging in bundled mode needs `glibc-bindings` and `ffibox` patched to their local
  sources and the OpenSSL, PCRE2 and zlib installs on the CMake and pkg-config paths
  (`crustify/evolution/packaging-gate-procedure` on wave 186's branch records the command;
  batch 0's Notes entry here gives the same).
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
