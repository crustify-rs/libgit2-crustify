# A closed enum's integer-form conversions are crate-private

Status: `open`: the change is crustify `3d1691a` on
`libgit2-crustify/evolution/enum-integer-form-crate-private`, awaiting its review.

## Signal

I had to work out a rule by hand that a skill should have stated, and two skills read
together leave it open.

## Evidence

`wave-186/batch-0/run-1` ported `src/cli/opt.h`'s closed enums `cli_opt_type_t` and
`cli_opt_status_t` (`Kind`, `Status` in `src/git2/src/cli/opt.rs`). Both are scheduled a
`port/layout` (wave 2404), which per `porting-types`' Enums section "removes that form, its
C layout: its `repr`, its discriminants and its conversions". `porting` says a later stage
changes the code that reaches the item's C form, "never the safe API its consumers use". The
precedents the tree holds are bindings, whose integer form is permanent: `tree.rs`'s
`FileMode` and `WalkMode` have public `from_raw` / `raw`, `TryFrom` / `From` and a public
`Unnamed*` error type. Copied into a port, these would be public API that the enum's
`port/layout` removes, and `ffimap api` would report it `changed`.

The batch made the conversions `pub(crate) const fn from_raw(u32) -> Option<Self>` and
`raw(self) -> u32`. No code calls them yet, because the gateways that will (`cli_opt_parse`'s
export) are later batches of the module. With warnings denied, `dead_code` then fails the
build, so each conversion carries
`#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`, and an inline `equiv_priv` test
compares them with `sys::git2`'s constants.

## Destination

The spec: `porting-types`' Enums section, which the translator evolves.

## Change

crustify `3d1691a`. After "its `repr`, its discriminants and its conversions", the section
now says that the conversions are crate-private, reached only by the gateways and regions,
since the stage keeps the safe API. It also says that a conversion no crate code calls yet
carries a `dead_code` expectation that holds outside tests, and that an `equiv_priv` test
compares it with the C's constants. The change drafts its `EVOLUTION.md` entry.

## Validation

No failure prompted the change, and its effect is not in doubt: this batch's `opt.rs` is
the case. It builds and lints with warnings denied, both with and without `cfg(test)`
(`clippy --all-targets`), and its `equiv_priv` tests pass under plain, ASan+UBSan and Miri.
`ffimap api` reports the enums `new`, with no public conversion for a later stage to remove.

## Review

None yet.
