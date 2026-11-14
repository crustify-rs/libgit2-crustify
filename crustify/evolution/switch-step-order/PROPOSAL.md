# A switch's Rust side lands before its builds, and removes every mention of the export feature

Status: `open`: the change is crustify `663ed2d` on `libgit2-crustify/evolution/switch-step-order`, made on 2026-11-14,
awaiting its review. Split from `module-switch-procedure` that day.

Goal: Incremental migration without breaking consumers. Its measure counts the landed trees
that failed a gate.

## Signal

"Skills that contradict each other, or an agent prompt that leads its agent wrong": a defect
in the spec. `porting-workflow`'s Builds section lists a switch's steps in an order that
cannot be run (item 10), and its words "its export feature is deleted" read as fewer edits
than the lints need (item 14).

## Evidence

**The switch's Rust side precedes the generation (item 10).** Met at the third switch,
`deps/llhttp/api.c`'s (`modules/libgit2/deps/llhttp/api.c/integration/run-1`, the fork's
`c54c0c180`, dashboard Notes entry `libgit2/deps/llhttp/api.c` integration). It is the first
switch that removes raw bindings: 23 functions without an export, with their allowlist
entries. Each variant's `build` runs `rust-staticlib.sh`, which builds the crate of the
checkout with `--features bindgen` in campaign mode, checking the committed bindings against
the variant's headers, and compiles the gateway only once the export feature is gone. So the
switch's Rust side, the feature deleted, the allowlist pruned and the bindings regenerated,
comes before `build-variants.sh`, not after the C's removal as the procedure's order (the C
change, then the variants, then the gates) suggests. The bindings are regenerated against
the previous generation, whose headers the switch does not change.

- The run deleted the feature, pruned `sys/libgit2/allowlist.txt` of the 23 functions and
  regenerated `bindings.rs` (`CRUSTIFY_BINDINGS_UPDATE=1 cargo-variant.sh plain build
  --features bindgen --lib`, against gen2), committed that, and only then committed the C
  change and ran `build-variants.sh`, whose `rust-staticlib.sh` then checked the committed
  bindings against gen3's headers on every variant.
- Item 10 again, at `deps/llhttp/http.c`'s switch
  (`modules/libgit2/deps/llhttp/http.c/integration/run-1`, the fork's `230c9dbdb`): the run
  deleted the feature, pruned the allowlist of `llhttp_message_needs_eof` and regenerated
  `bindings.rs` against gen4, committed that with the submodule at the fork's new commit, and
  only then ran `build-variants.sh`.

**The export feature is named outside the gateway's `cfg` (item 14).** Met at the fourth
switch, `deps/llhttp/http.c`'s. The switch deletes "the module's export feature", which
reads as the `Cargo.toml` entry and the gateway's `#[cfg(feature = …)]`. http.c's
`headers_code`, which only the gateway calls, also carried
`#[cfg_attr(not(any(test, feature = "ffi-export-deps-llhttp-http-c")), expect(dead_code))]`:
with the feature gone, the unknown `cfg` and the expectation left unmet fail clippy with
warnings denied. The module pass foresaw it, so the switch did not fail; nothing in the
procedure would have. The step is mechanical: every mention of the feature's name in the
crate (`src/`, `tests/`, `Cargo.toml`, `build.rs`) is removed or rewritten, a `cfg_attr` or
`cfg(any(…))` that names it as much as the gateway's attribute, so that only the run records
and the reviews' reproducers (item 7) still name it.

- `grep -rn ffi-export-deps-llhttp-http-c` over `src`, `tests`, `Cargo.toml` and `build.rs`
  found the `Cargo.toml` entry, the gateway's `cfg`, `headers_code`'s `cfg_attr` and the
  module's doc comment; after the switch it finds none there.

**The order within one variant's build (from item 2's evidence).** At the first switch
(`modules/libgit2/src/util/utf8.c/integration/run-1`, commits `012d1d34c` and the fork's
`1b37e3d08`), `crustify/scripts/rust-staticlib.sh` runs as the first step of each variant's
`build`, between `configure` and `cmake --build`: the Rust library needs the files
`configure` writes, and the C's link needs the Rust library.

## Destination

From the original's Destination list:

- `porting-workflow`, Builds: the order a switch's steps take, the C build's `configure`
  before the Rust library and its link after it.
- `porting-workflow`, Builds (item 10): the switch's Rust side, the bindings regenerated,
  lands before the generation is built, since every variant's build checks them.
- `porting-workflow`, Builds (item 14): deleting the export feature removes every mention of
  its name in the crate, not only the gateway's `cfg`.

Protected: none. `porting-workflow` (`skills/workflows/porting-workflow/SKILL.md`) is not
protected.

`testing-gates`' Regressions section, which is protected, repeats the steps in one sentence
("removes its translation unit …, rebuilds the variants from the C that remains and deletes
the module's export feature"). The original does not name it for these items. If its order
is read as binding, it would follow.

## Change

Made, in `porting-workflow`'s Builds section, as an ordered list of the step's four parts after
the paragraph that describes the switch. The sentence "The shared variants are rebuilt from the
C that remains once the integration lands" is removed: the list's third part states the
builds, and how the shared variants take them is `variant-generations`' to say. What was
drafted first:

`porting-workflow`'s Builds section would state a switch's steps in the order they run:

1. The Rust side first, as one commit. The module's export feature is deleted. The allowlist
   entries of the module's functions and globals that have no export are removed. The
   bindings are regenerated against the builds in use, whose headers the switch does not
   change.
2. Deleting the export feature removes or rewrites every mention of its name in the crate:
   its sources, its tests, its manifest and its build script. A `cfg_attr` or a
   `cfg(any(…))` that names the feature counts as much as the gateway's attribute. After
   the step, only run records and review reproducers still name it. The reason is that an
   unknown `cfg`, or a lint expectation left unmet, fails the lints with warnings denied.
3. The C side next: the module's translation unit and its tied C tests leave the fork, and
   the campaign's commit records the submodule at the fork's new commit.
4. The builds last. Each variant is configured, then the Rust library is built, then the C
   is built and linked. The reason is that each variant's build checks the committed
   bindings against its headers, so the bindings must be committed first.
5. The gates run over the new builds.

No script moves with this proposal.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a switch's parts run in order, the Rust side first, then the C side, the builds and the gates, and deleting the export feature removes every mention of its name in the crate | `libgit2-crustify`, `switch-step-order` | crustify `663ed2d` | `porting-workflow` listed a switch's steps in an order that cannot run, the builds checking bindings not yet committed, and "the export feature is deleted" read as fewer edits than the lints need | an integration run that reorders a switch's parts by hand, or whose lints fail on a mention of a deleted export feature | watched |

## Validation

Copied from the original:

- `deps/llhttp/api.c`'s switch: "Items 10, 12 and 13 are evidence, and are not made: they
  want review with the rest of this proposal."
- `deps/llhttp/http.c`'s switch: "Items 14, 15 and 17 are evidence, and are not made."

Two switches ran the order of step 1 before step 4 by hand, as the Evidence records. The
skill's text is not written.

## Review

None yet.
