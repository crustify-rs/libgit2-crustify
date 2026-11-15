# A switch whose C side changes a header makes it before the bindings are regenerated

Status: `landed`.

Goal: Incremental migration without breaking consumers. Its measure counts the landed trees
that failed a gate.

## Signal

"Skills that contradict each other, or an agent prompt that leads its agent wrong": a defect
in the spec. `porting-workflow`'s Builds section orders a switch's parts as the Rust side,
"the bindings … regenerated against the builds in use, whose headers the switch does not
change", then the C side. `bindgen`'s Generation section has the bindings follow "from the
headers at the `c/` revision", their source dirs "taken from the checkout's `c/`". A switch
whose C side changes a header the bindings read, as a `static inline` function's switch
does when it replaces the definition by its prototype (`porting-symbols`' Static inline
functions section), therefore changes the bindings by its C side: in the order the section
gives, the Rust side's regeneration reads the header unchanged and changes nothing, and
every variant's build of the new generation, which checks the committed bindings against
the checkout's headers, fails.

## Evidence

`src/util/integer.h`'s switch (`modules/libgit2/src/util/integer.h/integration/run-1`, the
campaign's fifth and its first of a header module, commit `b95941fa6` and the fork's
`d9a1d699b`). The header's six `GIT_INLINE` functions were bound through `static_fns.c`'s
wrappers, `git__is_<name>__extern`, and their raw bindings carried
`#[link_name = "git__is_<name>__extern"]`. The switch replaces the six definitions by
prototypes, so that the remaining C calls the module's exports.

- The run made the C side first: `c/src/util/integer.h` changed and committed on the fork's
  `fork/main` as `integrate src/util/integer.h`. Only then did
  `CRUSTIFY_BINDINGS_UPDATE=1 crustify/scripts/cargo-variant.sh plain build --features
  bindgen --lib`, against gen5 (the builds in use, whose generated headers the switch does
  not change), regenerate the bindings: `bindings.rs` lost the six `link_name` lines and
  `static_fns.c` its six wrappers. The Rust side's one commit records the submodule at
  `d9a1d699b` beside them.
- The module pass's rehearsal on `plain` (`wave-223/batch-1/run-1`'s
  `switch-rehearsal.md`) found the same diff only from a tree whose header was already
  changed.
- The earlier switches removed whole translation units, whose headers stayed as they were,
  so the order held for them: `deps/llhttp/api.c`'s and `deps/llhttp/http.c`'s
  regenerated against the previous generation after pruning the allowlist, as
  `switch-step-order` records.

The record's row of `switch-step-order` watches "an integration run whose builds precede its
Rust side, or whose lints fail on a mention of a deleted export feature": this is neither,
so the proposal is new.

## Destination

The spec: `porting-workflow`'s Builds section, the first two parts of a switch.

Protected: none. `porting-workflow` (`skills/workflows/porting-workflow/SKILL.md`) is not
protected.

## Change

In `porting-workflow`'s Builds section, the list of a switch's parts states one order for
every switch, the C side first:

> 1. The C side, committed on the fork: the module's TU and the C tests tied to it that reach
>    its internals leave it, and each of the module's `static inline` functions has its
>    definition in its header replaced by its prototype, as `porting-symbols`' Static inline
>    functions section describes.
> 2. The Rust side, in one commit, which records the submodule at the fork's new commit. […]
>    the bindings are regenerated from the checkout's `c/` at that commit, as `bindgen`'s
>    Generation section describes, against the builds in use, whose generated headers the
>    switch does not change. The C side comes first because the bindings follow from its
>    headers: a `static inline` definition replaced by its prototype changes them, and
>    bindings regenerated before it fail the check of every build made after it.

Parts 3 and 4, the builds and the gates, are unchanged. Made as crustify `c6fa095`, three
commits on `libgit2-crustify/evolution/switch-header-change-before-bindings` (the first,
`5319438`, added a condition inside the old part 1, which review-1 rejected), in
`skills/workflows/porting-workflow/SKILL.md` alone.

## Validation

Before, in the section's order: the bindings as regenerated before the C side, that is
`12dfd634d`'s `bindings.rs` and `static_fns.c` (the header unchanged, the regeneration has
nothing to change), put back in the switch's tree with `c/` at `d9a1d699b`, and
`cargo-variant.sh plain build --features bindgen --lib` run in campaign mode: the build
script fails, `sys/libgit2/bindings.rs does not match its bindgen input … rebuild with
CRUSTIFY_BINDINGS_UPDATE=1 and commit it` (`build.rs:1428`). Every variant's
`rust-staticlib.sh` runs the same check, so each of the generation's builds would fail so.

After, in the changed order: the C side committed first, then the regeneration, which gives
the rehearsal's diff exactly, and `build-variants.sh` built gen6's five variants from
`d9a1d699b`, each variant's `rust-staticlib.sh` passing the bindings check.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| every switch commits its C side on the fork first, then its Rust side in one commit that regenerates the bindings from that C and records the submodule at it | `libgit2-crustify`, `switch-header-change-before-bindings` | crustify `c6fa095` | `porting-workflow` regenerated a switch's bindings before its C side, so a header the switch changes left them stale and every variant's bindings check failed | a switch whose committed bindings differ from those its new generation's builds regenerate, or a variant build failing the bindings check after a switch | watched |

## Review

Review-1 (`crustify/artifacts/evolution/switch-header-change-before-bindings/review-1`, of
crustify `5319438`): rejected. The signal and evidence hold and no check is weakened, but the
change left a switch two orders, the list's Rust side then C side and a sentence inside it
the opposite where a header changes, while all five switches committed the C side first.
Required: one order for every switch, the condition dropped, and this row's change cell
reworded. Reworked by the assistant as crustify `7a6baf0`.

Review-2 (`review-2`, of `7a6baf0`): rejected for one fix. Review-1's three fixes are made;
the rework's "each of its headers' `static inline` definitions" tied the replacement to the
header, where `decomposition` and `porting-symbols` tie it to the function's own module.
Reworked as crustify `c6fa095`, the branch rebased onto the working branch; 

Review-3 (`review-3`, of `c6fa095`): approved. Every fix of the two reviews before is made, and
the change weakens no check. Landed on crustify's `libgit2-crustify/devel` as `c6fa095`,
recorded as `6efafd5`.
