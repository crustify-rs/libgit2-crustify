# wave-186/batch-6/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-6`, on `crustify/batches/wave-186/batch-6`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-6/run-1/task.json` in it.
- Batch: `wave-186/batch-6` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- This review batch takes the `symbol` route's landings of `src/cli/opt.c`'s folded waves
  (186 to 193), the module's whole `port` of its symbols, in `src/git2/src/cli/opt.rs`:
  `wave-186/batch-1` (`29ec3ab87`, `Kind::is_option` and `uses_gnu_style`), batch 4
  (`64431c07a`, `Parser::new`, `OptRef::write_status`, `ForeachCallbackFn`,
  `SpecTable::from_ptr` and eight private helpers) and batch 5 (`c62dee857`,
  `ParserMut::parse_next`, `parse`, `for_each`, five private helpers and `Parser::into_args`),
  with five exports under `ffi-export-src-cli-opt-c`, off. The types were reviewed apart
  (batch 3). The module pass follows you, then the module switches from C: the CLI's C that
  remains calls the exports, so review them for what that C relies on (`c/src/cli/`), as
  `reviewing`'s Depth section gives a ported module's exports.
- A fix this batch is asked for: `OptRef::write_status`'s export writes through the libc
  wrapper's `FileRef::print`, which takes `&str`, so it replaces bytes that are not UTF-8
  (an argument such as `--caf\xe9`) with U+FFFD, where C wrote them as they are: the one
  equivalence test of the sixteen that fails with the exports linked in place of C's
  `opt.c`. The wave now pins the wrapper at `24da93f` (`17401d831`), which has
  `FileRef::write_all(&self, &[u8]) -> io::Result<()>`, a byte-exact write under the stream's
  lock (`libc::libio::iofwrite`), a short write being an error. Move the export's
  `FileWriter` onto it, so that the departure and its documentation go, and judge what its
  `io::Write::write` returns when `write_all` fails after part of the bytes went out. Record
  `crustify/evolution/fileref-prints-bytes` as answered.
- The batches did not carry over seven C defects, each documented and kept by a unit test:
  `cli_opt_parse`'s `alloca` overflow on bundled short options, the read past an argument
  after an unknown option inside a bundle, literal arguments reordered after `--`, bare
  arguments dropped before a last option with an optional value, the bare `-`, the nameless
  bool's `--no-`, and `consume_choices` stepping before the table. Their reproducers are in
  the run dirs of batches 4 and 5. Judge each as `reviewing`'s Equivalence objective judges a
  deliberate divergence, and what the CLI's remaining C sees of it after the switch
  (`git2 -` reports an unknown option where C matched `--help`). Batch 5's exports abort on
  a panic where the caller would read an unwritten outcome
  (`crustify/evolution/export-panic-without-a-failure-outcome`).
- This batch carries no wave gates: the module pass does. Run the module's tests and the
  scoped gates of what you change, with the export feature off and on, and with the exports
  replacing C's `opt.c` in a scratch link, as batch 5 did.
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
