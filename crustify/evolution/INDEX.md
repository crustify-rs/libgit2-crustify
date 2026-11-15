# Index of proposals

One row per proposal of this campaign, as the `evolution` skill's Proposals section describes:
its slug, its signal in a line, its status, the runs its proposal cites as having met the
signal, and the row of the record, Crustify's `EVOLUTION.md`, it matches, if any. Written by
the assistant alone, as each batch's proposals land.

| slug | signal | status | met by | record |
|---|---|---|---|---|
| `bindgen-modes-after-bundling` | The bindgen skill's build modes, as the bundling migration met them | `landed` | — | — |
| `binding-c-defects` | How a binding keeps a C defect out of its safe API | `landed` | — | — |
| `bsan-borrow-history-grows` | BorrowSanitizer's cost per access to one allocation grows with its borrow history | `superseded` | `wave-225/batch-1` | `bsan-sized-tests` |
| `bsan-cost-in-the-fixture` | `testing-gates` sizes a test BSan cannot finish by its inputs, without first finding what its cost grows with | `open` | `fixes/gates-green/batch-0` | — |
| `bsan-regenerates-other-bindings` | BSan builds regenerate different bindings, so campaign mode fails under BSan | `landed` | — | — |
| `bsan-sized-tests` | A test BSan cannot finish is sized for it, not skipped | `landed` | `wave-225/batch-1` | — |
| `build-scripts-watch-missing-inputs` | Build scripts watch inputs that do not exist, so every cargo run rebuilds the crate | `open` | `wave-232/batch-1` | — |
| `bundled-pcre2-shadows-the-side-campaign` | pcre2-sys's vendored PCRE2 shadows the side campaign's for libgit2's C | `landed` | — | — |
| `c-link-unused-rust` | The C library's link keeps Rust that the C never calls | `open` | `module-switch-procedure`'s switches | — |
| `callback-context-in-an-argument` | A callback whose context is reached through an argument has no sound C form | `landed` | `wave-194/batch-1`, `wave-194/batch-2` | — |
| `callback-reenters-its-caller` | A callback can re-enter the function that calls it, through the object it is handed | `landed` | `wave-194/batch-1` | — |
| `carried-over-test-cases` | The test cases a `port/unexport` carries over, in the schedule | `landed` | — | — |
| `compile-fail-doctest-codes-unchecked-on-stable` | A compile-fail doctest's error code is checked only on nightly | `open` | `wave-186/batch-2`, `fixes/gates-green/batch-0` | — |
| `config-branches-of-one-result` | A macro whose configured branches compute one result is ported once | `landed` | `wave-225/batch-0`, `wave-225/batch-1` | — |
| `config-branches-rust-cfg` | A macro whose configured branches differ, under conditions Rust's `cfg` sees, has no route | `open` | `wave-219/batch-1` | `config-branches-of-one-result` |
| `contracts-unrefusable-c-arguments` | Which of C's arguments a gateway refuses, and which it trusts | `landed` | `wave-177/batch-1` | — |
| `coverage-c-scope-matches-no-path` | The coverage report's C scope names the paths the archive records | `landed` | `wave-169/batch-1`, `wave-169/batch-2` | — |
| `coverage-c-scope-unfiltered` | The coverage report's C figures ignore their scope and move with the test binaries | `landed` | `wave-181/batch-0` | — |
| `coverage-report-relative-rows` | The coverage report checks its Rust scope against paths llvm-cov prints relative | `open` | `wave-225/batch-0`, `wave-219/batch-0` | — |
| `coverage-report-unit-suite` | The coverage report has no unit suite, which each batch merges by hand | `open` | `wave-194/batch-0`, `wave-194/batch-1`, `wave-194/batch-2` | — |
| `dashboard-lacks-module-passes` | Give the dashboard the template's `Module passes` table | `landed` | `wave-169/batch-5`, `wave-176/batch-3`, `wave-182/batch-1` | — |
| `dashboard-module-passes` | The dashboard has no `Module passes` tables for a module pass to fill | `landed` | `wave-169/batch-3`, `wave-169/batch-4`, `wave-176/batch-2`, `wave-182/batch-1` | — |
| `dashboard-tables-follow-the-template` | A template change that adds a dashboard table reaches the dashboard before batches fill it | `landed` | `wave-181/batch-0`, `wave-182/batch-0`, `wave-183/batch-0`, `wave-184/batch-0` | — |
| `doc-alias-equal-to-the-name` | An anchor's alias that equals its item's name | `landed` | `wave-186/batch-1`, `wave-186/batch-4`, `wave-186/batch-5` | — |
| `elementwise-releasers` | Releasers that free a container's elements with it | `landed` | — | — |
| `end-gates-lint-stage` | `end-gates` runs the format and lint gates over the committed tree | `landed` | `wave-35/batch-0`, `wave-49/batch-0`, `wave-197/batch-0`, `wave-197/batch-1` | — |
| `end-gates-runner` | A runner for a wave's and a module's end gates | `landed` | `wave-169/batch-5`, `wave-181/batch-0`, `wave-181/batch-1` | — |
| `end-gates-skip-names` | `end-gates` cannot skip a test whose name is a prefix of another's, and checks a variant stage's skip against one binary | `landed` | `gates-slow-list`'s review-1 | — |
| `enum-integer-form-crate-private` | A closed enum's integer-form conversions are crate-private | `landed` | `wave-186/batch-0` | — |
| `equiv-c-side-meets-the-c-preconditions` | The C side of an equivalence test meets the C's preconditions | `landed` | `wave-169/batch-5` | — |
| `equiv-macro-without-effect` | A macro whose expansion computes nothing has no equivalence test to run | `open` | `wave-219/batch-0`, `wave-219/batch-1` | `equiv-unreachable-macro-sites` |
| `equiv-tests-without-c` | An equivalence test that calls no C is carried over at the switch | `landed` | `module-switch-procedure`'s switches | — |
| `equiv-unreachable-macro-sites` | A macro whose expansions no binding reaches is checked against its body | `landed` | `wave-200/batch-0`, `wave-200/batch-1`, `wave-225/batch-0`, `wave-225/batch-1` | — |
| `evolution-protocol-research` | The evolution protocol against existing practice | `deferred` | — | — |
| `evolution-record-and-lessons` | The record holds every signal; lessons are reviewed entries; protected paths | `landed` | — | its own row |
| `export-adoptions-and-the-safe-boundary` | What an `ffi_import` region may expose to its module's `ffi_export` | `open` | `wave-186/batch-4` | — |
| `export-link-check` | A port batch runs its module's tests with its exports linked in place of its C | `open` | `wave-186/batch-5`, `wave-186/batch-6`, `wave-186/batch-7`, `wave-194/batch-4`, `wave-194/batch-7`, `wave-197/batch-0` and more | — |
| `export-panic-without-a-failure-outcome` | An export whose failure needs an outcome a panic cannot give | `open` | `wave-186/batch-5` | — |
| `export-signature-check` | How a gateway checks an export against its raw binding | `landed` | `wave-229/batch-0` | — |
| `exports-without-a-header-declaration` | An export whose C declaration no header holds has no raw binding to be checked against | `open` | `wave-194/batch-4`, `wave-197/batch-0` | — |
| `ffibox-adoption-of-lifetime-cells` | Adopting a handle over a layout type with a lifetime vouches for nothing about that lifetime | `landed` | `wave-186/batch-3` | — |
| `ffibox-allocator-api` | `Box` and `Vec` over a C allocator: ffibox adopts the standard `Allocator` | `landed` | — | — |
| `ffibox-canonical-path` | ffibox is declared by its canonical path | `landed` | — | — |
| `ffibox-cclosure-failed-during-invocation` | `CClosure::failed()` reads the panic slot an invocation holds mutably | `landed` | — | — |
| `ffibox-goal-g9-immutable-borrows` | ffibox goal G9 and references over immutable C bytes | `landed` | — | — |
| `ffibox-lifetime-ctypes` | `define_ctype!` for layout types that carry a lifetime | `landed` | — | — |
| `ffibox-usage-docs` | ffibox documentation that led agents to hand-write what ffibox covers | `landed` | — | — |
| `ffibox-vtables` | Function-pointer tables a library calls into, implemented by a Rust trait | `landed` | — | — |
| `ffimap-api-namespaces-share-a-path` | `ffimap api` reads a type and a function of one path as one item | `open` | `wave-194/batch-4` | — |
| `ffimap-driver-builds-under-a-workspace` | ffimap's driver does not build under a Cargo workspace, and its suite then skips half its tests | `landed` | the reviews of `doc-alias-equal-to-the-name` and `ffimap-wrapper-without-alias` | — |
| `ffimap-scanner-records-the-enclosing-head` | ffimap records the enclosing repository's HEAD as its own version | `landed` | `ffimap-sys-inventory-in-crate`'s review-2 | — |
| `ffimap-seams-into-raw` | The seam list names `from_raw` but not its inverse `into_raw` | `landed` | — | — |
| `ffimap-sys-inventory-in-crate` | ffimap takes its `-sys` inventory from extern crates alone, so in-crate raw bindings have none | `landed` | — | — |
| `ffimap-wrapper-without-alias` | ffimap does not count a wrapper that keeps its C function's name, which carries no alias | `deferred` | `wave-169/batch-2` | — |
| `fileref-prints-bytes` | libc's wrapper writes text, not the bytes C streams carry | `landed` | `wave-186/batch-4`, `wave-186/batch-6` | — |
| `gate-findings-outside-the-workset` | A gate's true positive is the batch's finding, wherever it lies | `landed` | `wave-49/batch-1`, `wave-229/batch-1` | — |
| `gates-slow-list` | End gates run every stage less a slow list; full runs periodically | `landed` | — | — |
| `gateway-feature-across-a-multiline-attribute` | build.rs reads a gateway as switched when a multi-line attribute follows its `cfg` | `landed` | `wave-194/batch-4`, `wave-197/batch-0`, `wave-197/batch-1` | — |
| `goal-fitness-functions` | Each goal names its fitness | `landed` | `wave-186/batch-1`, `wave-186/batch-3`, `wave-194/batch-5`, `wave-197/batch-0`, `wave-232/batch-1` | — |
| `header-macros-after-their-expansions` | A header macro that implements logic is scheduled after the symbols it expands to | `landed` | `wave-169/batch-1`, `wave-169/batch-2` | — |
| `hidden-lifecycle-primitives` | Lifecycle primitives the target's shared library hides | `declined` | `wave-3/batch-0` | — |
| `integration-merge-dashboard-sums` | Recomputing the dashboard's `Σ` rows in an integration merge | `landed` | — | — |
| `integration-merge-helpers` | The additive conflicts of an integration merge, settled by scripts | `open` | — | — |
| `internal-headers-in-the-bindings` | Where the bindings find an internal header's include path and definitions | `landed` | `wave-12/batch-2`, `wave-169/batch-1`, `wave-169/batch-2`, `wave-176/batch-1`, `wave-181/batch-0`, `wave-182/batch-0` and more | — |
| `internal-symbols-unlinkable` | The variants must let the Rust link the internal symbols it binds | `landed` | `wave-169/batch-0`, `wave-176/batch-0` | — |
| `io-write-over-write-all` | An `io::Write` over the libc wrapper's `write_all` must implement `write_all` itself | `deferred` | `wave-186/batch-6` | — |
| `later-stage-c-changes` | What a `port/unexport` changes in `c/` | `landed` | `wave-232/batch-0` | — |
| `macro-incidental-double-evaluation` | A macro that evaluates an operand twice only as a hazard is a function | `landed` | `wave-200/batch-0`, `wave-200/batch-1` | — |
| `merge-left-conflict-markers` | A merge into a wave branch is checked before it is committed | `landed` | `wave-169/batch-2` | — |
| `module-end-gate-runner` | A script for the wave's and the module's end gates | `landed` | `wave-176/batch-3`, `wave-177/batch-1` | — |
| `module-switch-procedure` | A module's switch: variant generations, the C's Rust library, and the C test gate | `superseded` | `wave-186/batch-7`, `wave-194/batch-7` | — |
| `obligation-spans-what-the-holder-keeps` | An `unsafe` setter's obligation lasts as long as its object holds what it stored | `landed` | `wave-194/batch-5`, `wave-194/batch-6` | — |
| `package-gate-procedure` | The packaging gate's procedure, for a crate whose dependencies are unpublished | `open` | `wave-229/batch-0`, `wave-219/batch-0` | — |
| `packaging-gate-procedure` | The packaging gate's command is worked out by hand | `superseded` | `wave-186/batch-0` | — |
| `port-coverage-figures` | Coverage figures that mean something in a port | `open` | `wave-225/batch-0` | — |
| `port-onto-an-existing-accessor` | A function that is one field's read or write is ported onto that field's accessor | `landed` | `wave-194/batch-0`, `wave-194/batch-3`, `wave-194/batch-4` | — |
| `rebase-with-unchanged-diff` | A rebased commit whose diff is unchanged is sent back to review | `landed` | — | — |
| `reftable-error-wrapped-ahead-of-basics` | Wrap `enum reftable_error` ahead of its first consumer, `deps/reftable/basics.c` | `landed` | `wave-169/batch-2`, `wave-169/batch-4` | — |
| `reproducer-repeats-the-workspace-patches` | A reproducer package repeats the affected workspace's `[patch]` tables | `open` | `wave-4/batch-1`, `wave-186/batch-6` | — |
| `reproducers-of-deleted-feature` | Nothing says what a switch does with a review reproducer that names its deleted export feature | `open` | `module-switch-procedure`'s switches | — |
| `review-reports-refused-by-the-host` | Review reports the host refuses to let a reviewer write | `landed` | `wave-0/batch-0`, `wave-176/batch-2`, `wave-176/batch-3` | — |
| `reviewed-evolution` | Every change is reviewed, recorded and watched | `landed` | — | — |
| `rust-library-per-variant` | Campaign mode builds the Rust library the remaining C links, once per variant | `open` | `module-switch-procedure`'s switches | — |
| `scratch-shared-between-agents` | Scratch a host shares between concurrent agents | `landed` | `wave-12/batch-2`, `wave-169/batch-1`, `wave-169/batch-2`, `wave-176/batch-1` | — |
| `soundness-patterns` | The UB patterns the campaigns met recur across modules, and no skill lists them | `landed` | — | — |
| `spec-consistency-2026-11-12` | Consistency fixes to the spec, made in another workspace | `landed` | — | — |
| `spec-language` | The spec's language; the goals rewritten under it | `landed` | — | its own row |
| `stage-points-count-staying-tests` | The C tests that stay, in a `port/unexport`'s release point | `landed` | — | — |
| `std-macro-expansions-outside-regions` | The standard library's own macro expansions count as unsafe code outside the regions | `open` | `wave-194/batch-0`, `wave-194/batch-1`, `wave-194/batch-3` | — |
| `switch-archive-check` | A switch checks its static archive by the removed unit's symbols, not by member names | `open` | `module-switch-procedure`'s switches | — |
| `switch-c-tests-baseline` | A switch's C tests run per variant by a tracked script and are compared with the baseline | `open` | `module-switch-procedure`'s switches | — |
| `switch-gate-beyond-c-tests` | A switch whose exports no baseline C test reaches is gated by the C's own programs, old build beside new | `open` | `module-switch-procedure`'s switches | — |
| `switch-order-in-testing-gates` | `testing-gates` restates a switch's parts in an order `porting-workflow` contradicts | `landed` | `switch-step-order`'s review-1 | — |
| `switch-step-order` | A switch's Rust side lands before its builds, and removes every mention of the export feature | `landed` | `module-switch-procedure`'s switches | — |
| `symbol-alias-anchors` | Where a symbol alias's anchor goes | `landed` | `wave-177/batch-0` | — |
| `sys-bindings-layout` | The raw bindings sit under `sys/<link-unit>/`, as the public `sys` module | `landed` | — | — |
| `test-only-exports-and-the-harness` | Exports only the C tests reach, and the test harness's own references | `landed` | — | — |
| `type-erased-field-tagged-by-a-sibling` | A type-erased field whose type a sibling tag selects is one tagged enum with its tag | `landed` | `wave-186/batch-2` | — |
| `unexported-lifecycle-primitives` | Lifecycle primitives the library does not export | `declined` | `wave-3/batch-0`, `wave-176/batch-0` | — |
| `variant-generations` | The build variants are built in generations, not rebuilt in place | `open` | `module-switch-procedure`'s switches | — |
| `wavefront-enum-constant-uses` | wavefront records no use of an enum constant | `landed` | `wave-186/batch-1` | — |
| `wavefront-test-tables` | wavefront's suite runs without CodeQL by default | `landed` | — | — |
| `wavefront-two-prototypes-one-signature` | wavefront composes a function's definition and a differently typed prototype into one signature | `open` | `wave-194/batch-6` | — |
| `wrap-ahead-claims` | An item wrapped ahead is held by one batch across the campaign | `landed` | — | — |
| `wrap-ahead-field-users` | A wrap batch ahead of its consumer lists the fields its consumers access | `open` | `wave-194/batch-0` | — |
