# wave-186/batch-5/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-5`, on `crustify/batches/wave-186/batch-5`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-5/run-1/task.json` in it.
- Batch: `wave-186/batch-5` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- Your worklist is the last of `src/cli/opt.c`'s `port`: the eight symbols of waves 190 to
  193 (`parse_arg`, `parse_long`, `parse_short`, `spec_for_sort`, `sort_gnu_style`,
  `cli_opt_parser_next`, `cli_opt_foreach`, `cli_opt_parse`), on top of batch 4. After you,
  the symbols are reviewed, the module pass follows, and the module switches from C.
- `wave-186/batch-4` (`64431c07a`) ported the eleven symbols of waves 188 and 189:
  `Parser::new` natively, `OptRef::write_status` (`cli_opt_status_fprint`) over any
  `io::Write`, `ForeachCallbackFn` (a closure returning `ControlFlow<NonZero<c_int>>`, or a C
  function and context through the crate-private `unsafe fn from_c`, which your
  `cli_opt_foreach` export uses), the public `unsafe fn SpecTable::from_ptr`, and eight private
  helpers that wait for you under `dead_code` expectations (`find_long`, `find_short`,
  `find_arg`, `is_choice`, `skip_choice_args`, `is_given`, `check_required`,
  `write_spec_name`); its exports sit under `ffi-export-src-cli-opt-c`, off. Read its Notes
  entry in the dashboard.
- It found a C defect in one of your items: `cli_opt_parse`'s `alloca`'d `given_specs` holds
  `args_len + 1` entries, but a bundle of short options gives one spec per letter, so `-ab`
  alone writes past the buffer (ASan `dynamic-stack-buffer-overflow` at `opt.c:579`). Its
  reproducer and README are in its run dir,
  `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-4/run-1/cli-opt-parse-given-specs-overflow/`.
  Do not carry it over: `alloca` has the `std` route (`Vec`), which removes the bound. Your
  equivalence tests keep the C inside its bound. It also did not carry over three other C
  defects (a nameless bool spec's NULL `strcmp`, the bare `-`, `consume_choices` stepping
  before the table): after the switch `git2 -` reports an unknown option where C matched
  `--help`. Check what your items make of them.
- The wave took the working branch (`2f25988a3`) for `src/util/utf8.c`'s switch from C, the
  campaign's first: `c/` is the fork's without `utf8.c`, `build.json` is at version 7, and
  your worktree's `crustify/builds` links to the variants' new generation,
  `crustify/builds.gen1`, whose link holds the safe crate's Rust static library. Read the
  switch's Notes entry in the dashboard and its lesson in `crustify/LESSONS.md` before your
  first build.
- libc's `alloca` has the `std` route (`alloc::vec::Vec`): check it against the code and
  report it for its `confirmation`.
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
