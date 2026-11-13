# wave-194/batch-3/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-3`, on `crustify/batches/wave-194/batch-3`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-3/run-1/task.json` in it.
- Batch: `wave-194/batch-3` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- Your worklist is the `symbol` route of `deps/llhttp/api.c`'s wave 194, `port`: 28 items,
  the three generator macros `HTTP_ERRNO_GEN`, `HTTP_METHOD_GEN` and `HTTP_STATUS_GEN`,
  `llhttp__debug`, `llhttp_execute`, the getters, `llhttp_pause`, `llhttp_resume`,
  `llhttp_resume_after_upgrade`, `llhttp_set_error_reason` and the `llhttp_set_lenient_*`
  setters, in `src/libgit2/deps/llhttp/api.rs`. Most are a few lines over a field `Parser`
  already has an accessor for: port them as the parser's native API, without duplicating
  what `llhttp.rs` gives, and keep the batch's tests proportionate to that. `wave-194/batch-4`
  ports the last 32 symbols (waves 195 and 196: `llhttp_init`, `llhttp_reset`,
  `llhttp_settings_init`, the three name functions, `llhttp_finish`, the two callback macros
  and the 23 `llhttp__on_*` dispatchers) once you have landed.
- `llhttp__debug` writes to `stderr` with `fprintf`. `dependencies.json` gives `fprintf` and
  `stderr` the `crustify-campaign` route and `abort` and `memset` the `std` route: check each
  against the code and report it for its `confirmation`. Whether the function is compiled at
  all on this build decides what you port; say what you found.
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
  - batch 2 (`97dea9da3`), in `api.c`'s own file, `src/libgit2/deps/llhttp/api.rs`: the
    callback table `llhttp_settings_s` as `Settings` with `SettingsRef` and `SettingsMut`,
    `wrap`, keeping its C layout; `Settings::new()` over C's `llhttp_settings_init`, which
    wave 195's port replaces; a getter and setter per slot over `Callback` and
    `DataCallback`; and `ParserMut::set_settings(Option<SettingsRef<'static>>)`, the getter
    staying the raw `settings_ptr`, since libgit2's own C callbacks read `parser->data` and
    are not safe with every parser.
  Read both batches' Notes entries in the dashboard, and
  `crustify/evolution/callback-context-in-an-argument` and `callback-reenters-its-caller`.
- libc's wrapper, `glibc-bindings`, is pinned at `1e60130` since `f1eced216`: `FileRef::print` and `write!` over a locked `fprintf`, and `unsafe fn
  stderr<'a>() -> FileRef<'a>`, whose caller vouches that the stream stays open and locked
  for the borrow.
- Packaging in bundled mode needs `glibc-bindings` and `ffibox` patched to their local
  sources and the OpenSSL, PCRE2 and zlib installs on the CMake and pkg-config paths (batch
  0's Notes entry gives the command).
- The wave took the working branch again (`300c14dfc`) for `src/util/utf8.c`'s switch from
  C, the campaign's first: `c/` is the fork's without `utf8.c`, `build.json` is at version 7,
  and your worktree's `crustify/builds` links to the variants' new generation,
  `crustify/builds.gen1`, whose link holds the safe crate's Rust static library. Read the
  switch's Notes entry in the dashboard and the lesson it recorded in `crustify/LESSONS.md`
  before your first build. `api.c` switches the same way at its integration: write each
  exported item's gateway under the module's export feature, off, as `utf8.rs` did
  (`680ac73bf`; the feature it declared was deleted at its switch, so yours is declared anew
  in `Cargo.toml`).
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
