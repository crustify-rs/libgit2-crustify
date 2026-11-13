# The raw bindings sit under `sys/<link-unit>/`, as the public `sys` module

Status: `landed`, as crustify `63b92be` on `libgit2-crustify/devel`, on 2026-11-12, once review-1 approved it.

## Signal

The user's correction (`evolution` §2), on 2026-11-12, before the campaign's raw bindings move
into the safe crate for its second link unit (`git2`, wave 186): the per-link-unit dir is to be
`sys/<link-unit>/`, "same for future crates", and the Rust module follows it, so that the
bindings are named after the ecosystem's `-sys` crates whose role they take.

## Destination

The spec: `bindgen` (Layout), `conventions` (Naming, Tree), `toolchain` (Generated code),
`testing`, `testing-builds` (Coverage's exclusions), `layout` (Tree) and the dashboard
template's `Rust LoC` definition.

## Change

`ffi/<link-unit>/` becomes `sys/<link-unit>/`, `src/ffi.rs` becomes `src/sys.rs`, the public
`ffi` module becomes `sys`, and a link unit's Rust imports `crate::sys::<link_unit>` as `sys`,
so that a raw binding is `sys::<name>`. The `ffi_import` region and the `ffi_export` gateway
and its `ffi-export-<tu>` features keep their names: they name what a region holds and what a
gateway does, not where the bindings sit. A record entry is appended to `EVOLUTION.md`.

## Validation

A rename with no failure behind it: no spec text, agent or template still names `ffi/<link-unit>/`,
`src/ffi.rs`, `crate::ffi` or the `ffi` module (grep over the worktree), and
`providers/claude/compose.py build` leaves the generated agents unchanged. The campaign's tree
takes the layout in its bundling migration (`crustify/migrations/bundled-sys`).

## Review

review-1 (`crustify/artifacts/evolution/sys-bindings-layout/review-1`), of `63b92be`: **approved**;
it weakens no check. No old name is left but those the record entry quotes and unrelated
`ffi` words (flate2's `ffi/c.rs`, `alloc::ffi::CString`, ffimap's `code.unsafe_call.ffi`); the
`ffi_import` / `ffi_export` names stay consistent; the grep and `compose.py build` rerun in a
`git archive` copy agree with the proposal. Advisories, with no failure behind them yet, so
not acted on:

- the coverage exclusion and the dashboard's `Rust LoC` say "under `sys/`", which a filter
  could apply unanchored, catching `src/<lu>/…/sys/` modules; "the repository root's `sys/`"
  would rule that out;
- a module with a child `mod sys;` cannot also import `crate::sys::<link_unit> as sys`
  (E0255), likelier than with `ffi`: the bundling migration meets it in the tests' `sys.rs`
  modules and reports its route;
- ffimap's `--sys-crates` and `ty.ffibox.sys.*` now share the word with the in-crate module.
