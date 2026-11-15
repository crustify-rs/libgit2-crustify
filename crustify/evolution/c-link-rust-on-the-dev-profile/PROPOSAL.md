# The Rust library the remaining C links is built unoptimised, and no event measures what that costs

Status: `open`.

Goal: Performance, the emitted Rust's measure, "the emitted Rust's time and memory against the
C it replaces, on the target's own workloads", which `GOALS.md` marks "Not taken yet" with a
TODO: "which workloads, and at which event, is not specified".

## Signal

"A TODO met again and again", with a measured cost. From a module's switch on, the remaining C
reaches the module through the Rust static library each variant's build makes with
`crustify/scripts/rust-staticlib.sh`. The script builds it "on the dev profile, the campaign's
tests' own" (`opt-level` 0, debug assertions and overflow checks on). The workspace sets no
`[profile]`. The variants compile the C at `RelWithDebInfo` (`-O2 -g -DNDEBUG`), so from the
switch on, a C consumer runs an `-O0` Rust in place of `-O2` C. No skill says whether a switch
measures what that costs, nor on which build. A ported routine on a hot path then slows every
C caller, and no gate sees it, since the gates measure only correctness.

## Evidence

- `wave-221/batch-3/run-1`, `src/util/hash/sha1dc/ubc_check.c`'s module pass, measured
  `ubc_check` at about 3.3 times C's `-O2` cost per call in that library (its task message for
  the switch, `modules/libgit2/src/util/hash/sha1dc/ubc_check.c/integration/run-1/task.md`).
- `modules/libgit2/src/util/hash/sha1dc/ubc_check.c/integration/run-1`, the switch (commit
  `f42cb56c4`, the fork's `e717996da`), measured the cost end to end for a C consumer on
  `plain`. A C program (`clang -O2`) linked each generation's `libgit2.so` and hashed one
  256 MiB blob with `git_object_id_from_buffer` (`git_odb_hash` is deprecated, and
  `-DDEPRECATE_HARD=ON` leaves it out). That is SHA-1 with collision detection (`sha1.c`),
  which calls `ubc_check` once per 64-byte block. The program ran 18 times on each
  generation, interleaved, timing thread CPU time on a host whose load average was about 68
  from another campaign. `crustify/builds.gen6` (C's `ubc_check`, `-O2`): minimum 1.544 s,
  median 1.751 s. `crustify/builds.gen7` (the Rust's, dev profile): minimum 2.039 s, median
  2.219 s. That is 1.32 times the minimum and 1.27 times the median, about 166 MiB/s against
  126 MiB/s. Both generations give the same id. `util_tests -ssha1` takes about 25 ms of CPU on
  both, too short to show the difference.
- How it fits: if `ubc_check` costs 3.3 times as much and the whole hash 1.3 times as much,
  then `ubc_check` took about 13% of the C hash.
- `modules/libgit2/src/util/thread.c/integration/run-1`, the switch of `src/util/thread.c`
  (commit `d63c80a1f`, the fork's `cc1e40f3e`), measured on `plain`, `crustify/builds.gen7`
  beside `crustify/builds.gen8`, both linking the dev-profile library. From this switch on,
  every reference count of the remaining C (`GIT_REFCOUNT_INC` and `GIT_REFCOUNT_DEC`) calls the
  module's exports where C inlined one `lock`-prefixed instruction. An increment-and-decrement
  pair of one `git_atomic32` (`wave-226/batch-6/run-1/bench/pair.c`, six alternating rounds of
  five runs of 5 × 10⁷ pairs) cost a median of 15.47 ns on gen7 and 102.29 ns on gen8, 6.6
  times as much. Batch 6 measured the same crate's library on the release profile at 21 to
  27 ns. Whole-library workloads, run through the variant's `git2` CLI, five interleaved
  rounds: a `file://` clone of the fork's `fork/main` (16458 commits, 134989 objects) took a
  median of 41.11 s wall and 78.40 s CPU on gen7, and 42.24 s and 81.24 s on gen8, 1.027 and
  1.036 times. `git2 blame src/util/thread.h` in the clone took 4.60 s and 4.77 s, 1.035
  times. The load average was 136 to 168 on 144 cores. A second switch on a hot path met the
  signal. Its cost to whole workloads, about 3 to 4%, is smaller than `ubc_check`'s to hashing.
  It falls on every operation that takes a reference.

## Destination

The spec: `porting-workflow`'s Builds section and `testing-gates`' switch, for where a switch
measures its cost. `rust-staticlib.sh` and `build.json` belong to the campaign, and the
assistant owns them, as it owns the variants' build settings.

## Change

Not made: a build setting is the assistant's, and the measurement event is the spec's.

1. A switch whose module's exports are on a hot path of the remaining C measures one target
   workload on `plain`, the previous generation beside the new, and records the ratio in its
   Notes. Which workloads count would follow the TODO's answer.
2. The library a shipped build's C links is built optimised. One option is a profile of the
   workspace that `rust-staticlib.sh` takes for shipping builds, with `opt-level` 2 or 3 and
   debug assertions off. The sanitizer and coverage variants would keep the dev profile, which
   the tests use. Whether `plain` changes too is the assistant's call: an optimised `plain`
   would hide nothing from the gates, but it would no longer be the build the Rust tests use.

## Validation

To be run: rebuild gen7's `plain` with the library on an optimised profile, in a scratch
generation, and rerun the measurement above. The expectation is a ratio near 1, or below it.

## Record

| a switch measures its exports' cost to the remaining C, and a shipped build's C links an optimised Rust library | `libgit2-crustify`, `c-link-rust-on-the-dev-profile` | — | the C that remains ran a dev-profile Rust in place of `-O2` C, 1.3 times as slow on SHA-1 hashing, and no event measured it | a switch or ship point that finds a C workload slower over the Rust library, unmeasured until then | watched |

## Review

Pending.
