# wave-194/batch-2/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-2`, on `crustify/batches/wave-194/batch-2`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-2/run-1/task.json` in it.
- Batch: `wave-194/batch-2` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- Your worklist is `deps/llhttp/api.c`'s one type, `llhttp_settings_s` (the callback table),
  `wrap`, in `api.c`'s own file, `src/libgit2/deps/llhttp/api.rs` (scaffolded at
  `2216b0cd0`): libgit2's HTTP client (`src/libgit2/transports/httpparser.c`) builds one in C
  and hands it to `llhttp_init`, so it keeps its C layout behind a safe API until its
  `port/layout` stage (wave 1165). With it bound, give `Parser`'s `settings` field the typed
  accessor batch 0 left raw, if the skills lead there. `api.c`'s 60 symbols follow you in two
  `port` batches, the first right after you land.
- libgit2's C callbacks read their context through `parser->data`
  (`callback-context-in-an-argument`): judge what a safe constructor of the table may accept.
- What has landed on this wave's branch (waves 195 and 196 are folded onto wave 194's), in
  `src/libgit2/deps/llhttp/llhttp.rs`, `llhttp.c`'s file, bound ahead of `api.c`:
  - batch 0 (`1ead111ee`): `Parser` (`llhttp__internal_s`) with `ParserRef` and `ParserMut`,
    its typedefs, and the enums `Errno`, `Method`, `Status` and `ParserType`; `data` and
    `settings` are raw addresses behind two `unsafe fn` setters until `llhttp_settings_s` is
    bound;
  - batch 1 (`2c69341c3`): `Parser::new()` and `ParserMut::reinit()`
    (`llhttp__internal_init`), `ParserMut::parse(&[u8]) -> Result<Parsed, ParseError>`
    (`llhttp__internal_execute`), which refuses a parser with no state and a callback that
    re-enters its own parse, the callback types `Callback` and `DataCallback` (`llhttp_cb`,
    `llhttp_data_cb`) with the `EventHandler` and `DataHandler` traits, and the X-macro maps
    on `Errno`, `Method` and `Status` (`MAP`, `map_name`, `map_string`, `constant_name`).
    It kept the names `init`, `reset` and `execute` free for `api.c`'s port.
  Read both batches' Notes entries in the dashboard, and
  `crustify/evolution/callback-context-in-an-argument` and `callback-reenters-its-caller`.
- libc's wrapper, `glibc-bindings`, is pinned at `1e60130` since the wave took the working
  branch (`f1eced216`): `FileRef::print` and `write!` over a locked `fprintf`, and `unsafe fn
  stderr<'a>() -> FileRef<'a>`, whose caller vouches that the stream stays open and locked
  for the borrow.
- Packaging in bundled mode needs `glibc-bindings` and `ffibox` patched to their local
  sources and the OpenSSL, PCRE2 and zlib installs on the CMake and pkg-config paths (batch
  0's Notes entry gives the command).
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
