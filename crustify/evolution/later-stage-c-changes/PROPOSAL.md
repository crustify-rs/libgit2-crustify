# What a `port/unexport` changes in `c/`

Status: `open`, the change made on crustify's
`libgit2-crustify/evolution/later-stage-c-changes` (`86d8b2f`), awaiting review.

## Signal

Skills that contradict each other, and a rule worked out by hand, met at the campaign's first
later stage, `src/util/utf8.c`'s `port/unexport` of `git_utf8_char_length`
(`modules/libgit2/src/util/utf8.c/wave-232/batch-0/run-1`):

1. **A later stage "changes no C", yet a `port/unexport` removes C test cases.**
   `porting-workflow`'s Later stages section has every later stage change no C, "since the C
   that held it back has left the build". `testing`'s C tests carried over section has a
   `port/unexport` carry over every C test case that still references the export it removes,
   "and remove the case from `c/`", and `porting-workflow`'s Tests follow their modules section
   says the same. The run's task message had to grant the C change as "a later stage's
   exception", and spell out by hand what a switch's procedure does: the commit on the fork's
   `main`, the superproject recording it, a new generation of the variants, and the C tests over
   it compared with an updated baseline.
2. **No skill says what becomes of the export's declaration.** `porting-symbols` removes "the
   export, with the raw binding and the allowlist entry that reach it", and `porting-workflow`
   keeps a module's headers "declaring the module's exports". Once a function has no export, its
   header's declaration declares nothing any build defines; the task message left the decision
   to the run, which took it from the conventions' Exports section (exports serve the target's
   remaining C alone) and removed it.
3. **No skill says whether C only an unbuilt configuration compiles holds an export back.** The
   function is still called under `GIT_WIN32`, which no variant defines: `fs_path.c`'s two
   sites, `fs_path.h`'s `static inline` and `tests/libgit2/refs/basic.c`'s
   `test_refs_basic__longpaths`. The plan counts no referrer, and `porting-workflow`'s Tests
   follow their modules section puts a configured-out test case out of scope, but says nothing of
   configured-out calls in a translation unit or a header.

## Evidence

The run's landing on `crustify/waves/wave-232`, its dashboard Notes entry
(`libgit2/src/util/utf8.c` wave 232 batch 0), and the fork's `9859ed5e7`
(`unexport git_utf8_char_length`), which removes `tests/util/utf8.c` and `utf8.h`'s
declaration. The variants' generation built from it, `crustify/builds.gen4`, defines
`git_utf8_iterate` and `git_utf8_valid_buf_length` and no `git_utf8_char_length` in
`libgit2.so`, `git2`, `libgit2_tests`, `util_tests` and the Rust library; `util_tests` lists
53 suites where gen3's listed 54. Schedule records for later `port/unexport` waves carry
`carries_over` cases, so every one of them changes `c/` the same way.

## Destination

The spec: `porting-workflow`'s Later stages section, and `porting-symbols`' Functions section,
which are the translator's to evolve. `testing-gates`' Regressions section, which says which
gates a landing that changes `c/` runs (the packaging gate and the C baseline), already covers
the gates once the stage is allowed its change; it is not changed.

## Change

On crustify's `libgit2-crustify/evolution/later-stage-c-changes`, `86d8b2f`:

- `porting-workflow`, Later stages: a stage changes no C but for what still names the export a
  `port/unexport` removes: the C test cases it carries over and removes from `c/`, and the
  export's declaration in its header. That change is a commit on the fork's `main`, recorded by
  the stage's commit as a switch's is, and the stage's C tests run over variants built from the
  C it leaves.
- `porting-symbols`, Functions: `port/unexport` also removes the export's declaration in its
  header; C that only a configuration the target is built without compiles is no remaining C,
  holds no export back, and a build under that configuration meets the function undeclared where
  it calls it.
- `EVOLUTION.md`: the entry, watched for a task message granting a later stage its C change by
  hand, or a `port/unexport` that leaves its function declared or holds an export for
  configured-out C.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`86d8b2f`, which also edited `EVOLUTION.md`; on 2026-11-14 it was put on the working branch's
tip as `0bec1f0`, the same diff without that edit, since a change's row is drafted in its
proposal. Review-1 read `0bec1f0`; the commit is now `180fd67`.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a `port/unexport` changes `c/`: it removes the C test cases it carries over and the export's header declaration, and C only an unbuilt configuration compiles holds no export back | `libgit2-crustify`, `later-stage-c-changes` | crustify `180fd67` | `porting-workflow` had every later stage change no C, against `testing`'s carried-over cases, and no skill said what becomes of an unexported function's declaration, or of its callers under a configuration the target is built without | a task message granting a later stage its C change by hand, or a `port/unexport` that leaves its function declared in a header or holds an export for configured-out C | watched |

## Validation

No failure prompted the change: the run followed the amended rule, as its task message allowed.
Under it, gen4's five variants configured and built from the fork's `9859ed5e7` and the C
tests over plain, ASan+UBSan, TSan and coverage matched the baseline less the one case carried
over (the dashboard entry). A `GIT_WIN32` build was not attempted: no variant or host here
builds for Windows.

## Review

Pending.

`review-1` (session `cb79c1a3-2051-4435-8b13-e197a710a5d4`, Opus 5.5, `$1.78`, 5m50s), on `0bec1f0`: **rejected**, on two contradictions, both fixed by removing text: the exception's list left out the test harness, which `scheduling` has a `port/unexport` adapt, and `versioning` said no batch changes the C. Made as `180fd67`, on `2c21af3`: the list is gone, and so is `versioning`'s clause. Not required, and the gap most likely to be met: batches of one wave that each commit on the fork's `main` from one base, which task messages settle until a rule does.
Its report is the longest `result` of `crustify/artifacts/evolution/later-stage-c-changes/review-1/stream.jsonl`.

`review-2` (session `e4fe76a9-38ba-4ce6-a315-049b905d534f`, Opus 5.5, `$0.33`, 49s), on `180fd67`: **approved**. Each required fix of
review-1 is resolved, the fix opened no contradiction and the row matches the change.
Not required: without its list, the exception also reads over configured-out callers, which `porting-symbols` leaves alone.
