# wave-186/batch-4/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-4`, on `crustify/batches/wave-186/batch-4`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-4/run-1/task.json` in it.
- Batch: `wave-186/batch-4` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- Your worklist is the `symbol` route of `src/cli/opt.c`'s waves 188 and 189, `port`: eleven
  items, among them `spec_name_fprint` and `cli_opt_status_fprint`, which write formatted
  text to a `FILE *` the CLI's C hands them, `cli_opt_parser_init`, which batch 2 reached
  through C for `Parser::new` and you port natively, and the callback type
  `int (*)(cli_opt *, void *)`. `wave-186/batch-5` ports the last eight symbols (waves 190 to
  193: `parse_arg`, `parse_long`, `parse_short`, `spec_for_sort`, `sort_gnu_style`,
  `cli_opt_parser_next`, `cli_opt_foreach`, `cli_opt_parse`) once you have landed, on top of
  yours.
- What has landed on this wave's branch, in `src/git2/src/cli/opt.rs` (waves 187 to 193 are
  folded onto wave 186's branch):
  - batch 0 (`8a149ac08`): the enums, ported: `Kind` (`cli_opt_type_t`), `Status`
    (`cli_opt_status_t`), `Usage`, `ParseFlags`, with crate-private conversions to and from
    C's integers that wait for their first callers;
  - batch 1 (`29ec3ab87`): `Kind::is_option` (`spec_is_option_type`) and `uses_gnu_style`
    (`support_gnu_style`);
  - batch 2 (`fe6b85bfa`): the structs, `wrap`, keeping their C layouts for the CLI's C that
    still builds and reads them: `Spec<'p>`, `Opt<'p>`, `Parser<'p>` with their handles,
    `Target<'p>`, `Arg<'p>`, `ArgsSlot`, `SpecTable`, and `Parser::new` over C's
    `cli_opt_parser_init`;
  - batch 3 (`3829c7a06`): their review, which added the `# Adoption` contracts and `Spec`'s
    chainable builders (`with_name`, `with_alias`, ...).
  Read each batch's Notes entry in the dashboard before you write.
- libc's wrapper, `glibc-bindings`, is pinned at `1e60130` since the wave took the working
  branch (`4800c0ae2`): `File`, `FileRef<'a>`, `FileMut<'a>` (`libc::libio::genops`),
  `FileRef::print(&str) -> io::Result<usize>` and `write!` through `FileRef::write_fmt`
  (`libc::stdio_common::fprintf`), one locked `fprintf(stream, "%s", text)` per call, which
  refuses a NUL in the text and a stream without a lock, and `unsafe fn stderr<'a>() ->
  FileRef<'a>` (`libc::libio::stdio`). `_IO_FILE` and `fprintf` have the `crustify-campaign`
  route in `dependencies.json`: check each entity's route against the code and report it
  for its `confirmation`, with the other libc entities your items use (`memset`, `strncmp`,
  `strchr`, `strcmp`, `strlen`, `assert`, `alloca`).
- `opt.c` is the first module of the `git2` link unit (the CLI): raw bindings `sys/git2/`
  (`crate::sys::git2`). It switches from C at its integration, after its module pass: the
  CLI's C that remains then calls your exports, so write each exported item's gateway under
  the module's export feature, off, as `wave-229/batch-0` did for `src/util/utf8.c`
  (`ffi-export-src-util-utf8-c` on that wave's branch, `680ac73bf`, is the crate's first;
  yours is declared the same way in `Cargo.toml`). Packaging in bundled mode needs the
  procedure `crustify/evolution/packaging-gate-procedure` records.
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
