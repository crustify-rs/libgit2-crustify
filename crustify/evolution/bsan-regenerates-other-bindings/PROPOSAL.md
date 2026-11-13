# BSan builds regenerate different bindings, so campaign mode fails under BSan

Status: `open`. Blocks the BSan gates of every module's end, the first being `src/cli/opt.c`'s.

## Signal

A gate that cannot run (`evolution` §2): `crustify/scripts/cargo-variant.sh bsan test --workspace
--no-run` fails in the raw bindings' build script, "bindings.rs does not match", on the working
branch before and after the bundling migration (`928aa1ba9`, `e11d5b3ae`), the two
regenerations byte-identical. `cargo bsan` forces `LIBCLANG_PATH` to BorrowSanitizer's own LLVM,
whose libclang adds 27 doc-comment lines on macro constants; setting `LIBCLANG_PATH` does not
override it. With `CRUSTIFY_BINDINGS_UPDATE=1` all six test executables build and link.

This is the `bindgen` skill's open TODO, "the libclang that generates the bindings is not pinned
yet, though a regenerated binding must match the committed one byte for byte", met.

## Destination

The spec (`bindgen` Generation: the libclang is pinned, and campaign mode generates with it
whatever toolchain drives the build) and the campaign's build script.

## Change

To be drafted and validated: the libclang is recorded once (with the bindgen version), and the
build script loads that one before bindgen runs, so that a BSan, Miri or coverage build
regenerates the committed bytes; not a comparison that ignores doc comments, which would weaken
the check.

## Validation

Pending: `cargo-variant.sh bsan test --workspace --no-run` builds without
`CRUSTIFY_BINDINGS_UPDATE`, and plain still reproduces the committed bindings.

## Review

Pending.
