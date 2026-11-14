# Map of `module-switch-procedure`'s 17 items

Source: `crustify/evolution/module-switch-procedure/PROPOSAL.md`. Drafts:
`crustify/tmp/assistant/split/<slug>/PROPOSAL.md` and `LESSONS-draft.md`.

| item | what it is | taken by | kind |
|---|---|---|---|
| 1 | the variants cannot be rebuilt in place | `variant-generations` | proposal |
| 2 | campaign mode cannot build the Rust library the C links | `rust-library-per-variant` | proposal |
| 3 | BorrowSanitizer has no `rustc` subcommand | `rust-library-per-variant` | proposal |
| 4 | the C test gate was scratch | `switch-c-tests-baseline` | proposal |
| 5 | a link unit no C test reaches | `switch-gate-beyond-c-tests` | proposal |
| 6 | equivalence tests that run no C are lost | `equiv-tests-without-c` | proposal |
| 7 | a reproducer names the deleted export feature | `reproducers-of-deleted-feature` | proposal |
| 8 | `build-variants.sh` takes no job count | `variant-generations` | proposal |
| 9 | the C library's link pulls Rust it never calls | `c-link-unused-rust` | proposal |
| 10 | the switch's Rust side precedes the generation | `switch-step-order` (whole); one sentence repeated in `variant-generations` and `rust-library-per-variant` | proposal |
| 11 | a module only an online suite reaches | `switch-gate-beyond-c-tests` (the rule); `http-exports-gated-through-a-local-remote` (libgit2's facts) | proposal and lesson |
| 12 | the tests that call no C are marked already | `equiv-tests-without-c` | proposal |
| 13 | a CMake file names the removed source in a property | `cmake-property-naming-a-removed-source` | lesson |
| 14 | the export feature is named outside the gateway's `cfg` | `switch-step-order` | proposal |
| 15 | an archive's member names are not unique | `switch-archive-check` | proposal |
| 16 | the HTTP gate did not see keep-alive | `http-gate-tallies-connections` | lesson |
| 17 | the variants' smoke test is gone | `variant-generations` | proposal |

Nine proposals and three lessons.

## Where the grouping departs from the suggested one

- Item 9 is a proposal of its own, not with items 2 and 3. Items 2 and 3 are made in the
  campaign and ran at four switches. Item 9 is "not tried", has two candidate fixes, and one
  of its destinations is the fork's own CMake file. It can be reviewed, or declined, alone.
- Items 4, 5 with 11, and 15 are three proposals, not one. Item 4 moves one script
  (`c-tests.sh`). Items 5 and 11 are one rule (exports no baseline C test reaches) with
  other scripts. Item 15 changes a check and moves no script.
- Item 10 is not split. `switch-step-order` takes it whole. Its sentences on the generation
  (each variant's build checks the committed bindings; the bindings are regenerated against
  the previous generation) are repeated in `variant-generations`' Evidence with a pointer.

## Text of the original placed in more than one draft

- The "Items 1 to 4" bullets of the second, third and fourth switches mix facts of several
  items. Each sentence is copied to every draft it bears on: build times and object counts
  to `variant-generations`; object counts and the symbols each link unit defines to
  `switch-archive-check`; the symbols and the CLI's link to `rust-library-per-variant`;
  "`c-tests.sh` ran unchanged" to `switch-c-tests-baseline`; the 907 dynamic symbols and
  6747 Rust functions to `c-link-unused-rust`; the smoke tests to `variant-generations`.
- Validation's first paragraph (gen1) is copied to `variant-generations`,
  `rust-library-per-variant` and `switch-c-tests-baseline`.
- Item 11's evidence is copied to the proposal and to the lesson.

## Text of the original placed by judgement

- Destination, "`porting-workflow`, Builds: the order a switch's steps take, the C build's
  `configure` before the Rust library and its link after it". It carries no item number and
  follows from item 2. It is placed in `switch-step-order`, with item 2's sentence on where
  `rust-staticlib.sh` runs copied there.
- Destination, "The `libgit2` CMake lesson (item 13): a property naming the removed file is
  left". The draft is a new entry that points at the existing lesson. It could instead be an
  edit of that entry, which would make that entry `provisional` again.
- Evidence, the second switch's sentences on the CLI's link unit (`SelectRust.cmake`, the
  crate's default features). They hold for libgit2 alone. They are kept as evidence in
  `rust-library-per-variant`, and are neither a rule there nor a lesson.
- Evidence, item 1: "The side campaigns' installs, reached as `$PWD/../../../..`, resolve the
  same from a sibling of `builds`." Kept in `variant-generations`; the rule it implies (a
  generation sits at the same depth as the shared builds) is not written into the Change.

## Text of the original not placed

- The Signal's opening sentence, "Procedures worked out by hand that the skills should
  state, …, and two gaps where following the skills as written fails". No row of the
  Signals table reads "a procedure worked out by hand that a skill should state". Each
  draft names the nearest row; `c-link-unused-rust` and `reproducers-of-deleted-feature` say
  that no row fits exactly.
- The Change section's reason: "the change spans `setup`, `bindgen`, `testing-builds` and
  `testing-gates`, which are not the translator's to evolve, and wants its own review". Each
  draft lists its protected paths instead. `setup` and `bindgen` are not on the protected
  list given for this split, though the original calls them not the translator's.
- Validation, "`porting-workflow` is the translator's to evolve, but item 6 wants review
  with the rest of this proposal", and "they want review with the rest of this proposal"
  (items 10, 12, 13). Copied where the items went, but "the rest of this proposal" no longer
  exists once the split is filed. `equiv-tests-without-c` and `switch-step-order` touch only
  `porting-workflow`, which is not protected.
- The first switch's note in the Destination preamble that the scripts sit "beside the
  skills that name them". It does not say which skill `rust-staticlib.sh` sits beside.
- "Review: Pending." and "Status: `open`." of the original, and its row in
  `crustify/evolution/INDEX.md` (runs `wave-186/batch-7`, `wave-194/batch-7`). The split
  needs the original marked `superseded` by the nine slugs and nine index rows; not done.

## Gaps noticed while splitting

- The original never records how gen4 was built. Evidence names gen1, gen2, gen3 and gen5,
  and uses gen4 only as the previous generation of the fourth switch.
- The evolution skill's Proposals section asks each proposal for "the row the record takes
  when the change lands". The section list given for these drafts has no Record section, so
  none is written. Each draft needs one before it lands.
- `http-gate-tallies-connections` holds a general idea in libgit2's names: a comparison gate
  shows a departure only if its lines depend on what each export returns. If a second gate
  meets it, it is a rule for `testing-gates`, not a lesson.
- The lesson headings are slugs, as asked. The entries now in `crustify/LESSONS.md` use
  sentence headings and `**Scope.**`, not `Scope:`.
