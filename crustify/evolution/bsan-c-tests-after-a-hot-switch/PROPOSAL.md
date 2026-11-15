# A switch whose exports the C calls on every operation fails BSan's C-test comparison, and nothing says how to classify it

Status: `open`.

Goal: Safety, its soundness measure, and the gates' cost. A gate whose failure carries no
finding can neither block a switch for long nor be waived without a rule.

## Signal

A rule worked out by hand that a skill should have stated, and the recurrence of a landed
row. `testing-gates`' switch section has the remaining C tests run on every variant and
compared with the baseline, BSan's included. `bsan-sized-tests` (crustify `c18c8a8`) answers a
Rust test that BSan cannot finish by sizing it. Its recurrence signal is "a BSan binary timing
out in a full run on a test, or a test newly ignored or skipped under BSan for its cost". At
`src/util/thread.c`'s switch, BSan's C tests newly time out for the cost of the module's Rust,
and the target's own C tests cannot be sized. Nothing says whether such a difference blocks
the switch, how it is told apart from a UB finding, or how the `bsan` record of
`c-baseline.json` follows it.

## Evidence

`modules/libgit2/src/util/thread.c/integration/run-1`, the switch (`d63c80a1f`, the fork's
`cc1e40f3e`), `crustify/scripts/c-tests.sh` over `crustify/builds.gen8/bsan`, 16 suites side by
side, each under its 30-minute timeout:

- The tally: 3339 passed, 0 failed, 55 skipped of 3394, against the baseline's 3431, 0 and 55 of
  3486. 23 suites exit non-zero, against the baseline's 6. Of the 17 new ones, 14 time out
  (`blame::simple`, `clone::nonetwork`, `fetchhead::nonetwork`,
  `graph::reachable::from::any`, `index::crlf`, `merge::trees::renames`,
  `merge::workdir::dirty`, `network::fetchlocal`, `network::remote::local`, `perf::merge`,
  `rebase::merge`, `stash::apply`, `stash::save`, `util_tests`' `zstream`), and 3 abort on a
  BSan report (`commit::signature`, `diff::submodules`, `refs::revparse`). No case fails.
  Plain, ASan+UBSan, TSan and coverage match the baseline over the same generation.
- Every difference is the switch's. Over `crustify/builds.gen7/bsan`, the generation before,
  `index::crlf` passes its 10 cases in 4 s, where gen8's completes none in 30 minutes, nor
  later alone (see the switch's Notes entry). `commit::signature`, `refs::revparse` and
  `diff::submodules` pass in about 3 s each, with no report.
- The timeouts' cost is BSan's, in the module's thread-local storage. Sampled under gdb, the
  gen8 `index::crlf` process spends its time in `__bsan_retag` → `BorrowTracker::retag` →
  `LocationTree::perform_access`. These run under `tls::Table::position`, `c_keys::get` and
  `git_tlsdata_get`, which `errors.c`'s `threadstate_get` calls from `git_error_clear` on
  nearly every libgit2 call. Before the switch that lookup was `pthread_getspecific`, which
  BSan does not track. A thread's table is one allocation that lives as long as the thread,
  and BSan's cost per access to an allocation grows with its borrow history
  (`bsan-borrow-history-grows`, upstream). A reproducer through the public API, a `TlsKey`
  read with `with` 2000 times per chunk on one thread under `cargo bsan run`, costs 4.3 ms per
  lookup in the first chunk, then 12.6, 20.8, 29.7, 45.2, 57.5, 69.0 and 77.7 ms in the eighth.
  The cost grows linearly with the lookups already made.
- The three reports are "trying to access an allocation that has been freed", in C frames:
  `date.c:347`'s `switch (*end)` (twice), whose `end` glibc's uninstrumented `strtoul`
  writes, and `oid.c:242`'s `memcpy` in `git_commit_create`'s parent copy. ASan+UBSan runs
  the same suites over gen8 with no report. One reading, not verified, is that BSan reads a
  stale shadow provenance from a stack slot that only uninstrumented code wrote, left there
  by the Rust frames that now run on that stack (the dev-profile exports). By `testing-gates`,
  none is a UB finding: none reproduces through the Rust side.

## Destination

The spec: `testing-gates`' switch section, where the C tests are compared with the
baseline. Possibly `porting`, if a port's thread-local storage should be shaped so that a
hot lookup does not grow one allocation's borrow history. No such shape in safe Rust is
known.

## Change

Not made: the gates are not a translator's to change. The section would say:

1. Each suite a switch's BSan C tests run differently from the baseline is rerun alone over
   the previous generation. A difference that generation lacks is the switch's.
2. A switch's BSan report is a UB candidate only when it reproduces through the Rust side, as
   the section has it for Rust tests. One in C frames on a pointer that uninstrumented code
   wrote or returned, with ASan clean on the same suite, is recorded with its site and is
   not a finding.
3. A suite that newly times out is profiled before anything else. A cost in BSan's
   bookkeeping of a long-lived Rust allocation, shown by a reproducer, is the tool's. The
   switch records it, and the `bsan` record of `c-baseline.json` is re-recorded from the new
   generation with the user's approval, as a ship point's slow-list changes are put to the
   user.

## Validation

To be run: the rule over this switch's records gives the classification above, with the
14 timeouts the tool's and the 3 reports recorded, not findings. A full run at the first ship
point would show whether other suites follow.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a switch's BSan C-test differences are rerun on the previous generation, profiled, and classified as the switch's, the tool's or a UB candidate before the baseline follows | `libgit2-crustify`, `bsan-c-tests-after-a-hot-switch` | — | `thread.c`'s switch made 14 C suites time out and 3 abort under BSan, for the cost of Rust thread-local lookups the C makes on every call, and no rule classified them | a switch whose BSan C tests differ from the baseline with no case failing, left unclassified | — |

## Review

None yet.
