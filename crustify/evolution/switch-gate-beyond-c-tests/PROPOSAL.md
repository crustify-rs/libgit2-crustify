# A switch whose exports no baseline C test reaches is gated by the C's own programs, old build beside new

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Equivalence. Its measure counts the inputs and states on which the Rust is compared
with the C it replaces. Where no C test of the baseline calls a module's exports, the C test
gate compares nothing for that module.

## Signal

"One procedure repeated by hand: the same script written again, the same queries run in
sequence": a tool, and a rule the gates skill lacks. Two switches each had to build a gate of
their own, because the baseline's C tests do not reach their exports.

## Evidence

**A link unit no C test reaches has its switch gate in a module pass's run dir (item 5).**
Met at the second switch, `src/cli/opt.c`'s (`modules/git2/src/cli/opt.c/integration/run-1`,
the fork's `f57e1a616`, dashboard Notes entry `git2/src/cli/opt.c` integration), the first of
the `git2` link unit, the CLI. The CLI's exports are checked by running the relinked `git2`
beside the previous one (the `git2`-exports lesson), but the comparison is `cli-smoke.py`, a
scratch script recorded in `wave-186/batch-7/run-1`'s artifacts, which writes to a fixed
`crustify/tmp/cli` and whose scratch commits take the clock's time, so that two runs' outputs
differ in every line that prints a commit's date or id.

- The switch ran `cli-smoke.py` from the module pass's run dir, the stock `git2` being gen1's
  and the relinked one gen2's, on plain, ASan/UBSan and TSan: 62 of 66 the same on each, the
  4 others the module's recorded departures, and no sanitizer report. Gen2's outcomes equal
  the module pass's relinked ones on all 66, but for three (`cat-file -p HEAD`, `cat-file
  commit HEAD`, `blame f1`) that differ only in the scratch commit's date and id. A tracked
  script beside `c-tests.sh`, taking its output dir and the command lines' file, with
  `GIT_AUTHOR_DATE` and `GIT_COMMITTER_DATE` fixed, would make each run's record comparable
  with the last.

**A module whose exports only an online suite reaches has no C gate (item 11).** Met at the
third switch, `deps/llhttp/api.c`'s (`modules/libgit2/deps/llhttp/api.c/integration/run-1`,
the fork's `c54c0c180`, dashboard Notes entry `libgit2/deps/llhttp/api.c` integration).
`c-tests.sh` runs the offline suites the baseline records, and no offline C test drives
libgit2's HTTP client (the module pass's finding): `httpparser.c`, `httpclient.c` and
`llhttp.c`'s generated parser call the 31 exports only in a fetch, clone or push over HTTP,
whose suites (`online::*`) the baseline disables, since they need the network. libgit2's own
CI runs part of them offline, against servers on localhost named by `GITTEST_REMOTE_URL`
(`ci/test.sh`'s `auth_clone_and_push`, `-sonline::clone::push -sonline::push`).

- The run added `crustify/scripts/http-remote.py`, `git http-backend` behind a persistent
  HTTP/1.1 front that frames each response by its path (`Content-Length`, chunked in 1, 7,
  4096 and 65536-byte chunks, chunked with chunk extensions, or close-delimited), with an
  optional Basic challenge, and `crustify/scripts/http-gate.sh`, which runs a variant's
  `git2 clone` of two repositories through each framing, three clones that fail, and
  `libgit2_tests -sonline::push -sonline::clone::credentials` with `GITTEST_REMOTE_URL` at
  the guarded remote, into a `results.txt` free of times, ports and paths. Gen2's and gen3's
  results are compared with `diff` (the dashboard entry).
- Item 11 again, at `deps/llhttp/http.c`'s switch
  (`modules/libgit2/deps/llhttp/http.c/integration/run-1`, the fork's `230c9dbdb`), which ran
  item 11's scripts unchanged but for item 16: `http-gate.sh` over gen4's C and gen5's Rust,
  on plain, ASan+UBSan and TSan, gives the same `results.txt`, and on gen5's coverage variant
  the run calls all four exports.

Item 16, the gate's blindness to keep-alive, is recorded as a lesson of the campaign
(`http-gate-tallies-connections` in `LESSONS-draft.md`), since it is stated in the names of
libgit2's HTTP client. The facts of item 11 that hold for libgit2 alone are also kept there
(`http-exports-gated-through-a-local-remote`).

## Destination

From the original's Destination list:

- `testing-gates`, Regressions (item 5): a link unit that no C test reaches is checked at its
  switch by its relinked executables beside the previous generation's, by a tracked script
  beside `c-tests.sh`, and the result compared with the module pass's record.
- `testing-gates`, Regressions (item 11): a switch whose exports only the baseline's disabled
  online suites reach runs those the target's own CI runs against local servers, against a
  local remote, by the scripts beside `c-tests.sh`, on the new generation and the previous
  one, compared; `http-remote.py` and `http-gate.sh` generalized from libgit2's suites and
  variables into arguments.

Protected: all of it. `testing-gates` and every script placed beside it are under
`skills/verification/**`.

## Change

Not made. In the campaign only, `crustify/scripts/http-remote.py` and
`crustify/scripts/http-gate.sh` exist; the CLI's comparison script is still the scratch one
in a module pass's run dir.

The change would be:

1. `testing-gates`, Regressions, states a rule for a switch whose exports no C test of the
   baseline calls. The switch then runs the remaining C's own programs that do call the
   exports, on the previous builds and on the new builds, and compares the two results. The
   result holds no times, ports, paths or ids that differ between runs, so that two runs
   compare line by line. A difference is either a departure the module records or a failure
   of the gate.
2. When the exports belong to a link unit that is an executable, the programs are that
   executable's command lines, run in scratch directories with fixed dates. The result is
   also compared with the module pass's record.
3. When the exports are reached only by suites the baseline disables, the switch runs those
   of them that the target's own CI runs against local servers, against a local server.
4. A script for rule 2 is written beside `c-tests.sh`. It takes the two executables, an
   output dir and a file of command lines. It replaces `cli-smoke.py`, whose fixed output
   dir (`crustify/tmp/cli`) and unfixed commit dates it does not keep.
5. `http-remote.py` and `http-gate.sh` move beside `c-tests.sh` only once their libgit2
   names are arguments: the executable that clones (`git2 clone`), the test executable and
   its suites (`libgit2_tests -sonline::push -sonline::clone::credentials`), the environment
   variable that names the remote (`GITTEST_REMOTE_URL`), the credentials (`foo:bar`), the
   repositories served (`testrepo.git`, and a tree at the fixed commit `0551dfd4a`), and the
   list of framings. Whether a server for one protocol belongs beside a skill at all, or
   stays the campaign's, is open; the original asks for it to be generalized.

## Validation

Copied from the original:

- `src/cli/opt.c`'s switch: "Items 5 to 9 are its evidence, and are not made".
- `deps/llhttp/api.c`'s switch: item 11's two scripts are made, in the campaign only
  (`crustify/scripts/http-remote.py` and `http-gate.sh`), and validated by that run: gen2's C
  and gen3's Rust give the same `results.txt` on plain, ASan+UBSan and TSan, and on gen3's
  coverage variant the run calls 24 of the 31 exports. A mutant of an export was not built,
  so whether the comparison catches a departure rests on the coverage alone.
- `deps/llhttp/http.c`'s switch ran the scripts again, for a fourth module, and changed one:
  item 16's connection tally, in `http-remote.py`'s log and `http-gate.sh`'s `results.txt`,
  in the campaign only, validated by four identical runs over gen4 and gen5's equal ones.

## Review

None yet.
