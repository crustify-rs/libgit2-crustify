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

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`3d1691a`, which also edited `EVOLUTION.md`; on 2026-11-14 it was put on the working branch's
tip as `166d8fd`, the same diff without that edit, since a change's row is drafted in its
proposal. Review-1 read `166d8fd`; the commit is now `c461f16`.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| the integer-form conversions a port writes for a closed enum no wrapper gave a safe API are crate-private | `libgit2-crustify`, `enum-integer-form-crate-private` | crustify `c461f16` | `porting-types` has `port/layout` remove an enum's conversions while `porting` has later stages keep the safe API, and a port of enums ahead of their gateways meets `dead_code`: the batch worked out their visibility by hand | a closed enum whose first safe API a port wrote, with public `from_raw`/`raw` or `TryFrom`/`From` integer conversions, or its `port/layout` reporting a conversion `removed` or `changed` in its API stability row | watched |

## Validation

No failure prompted the change, and its effect is not in doubt: this batch's `opt.rs` is
the case. It builds and lints with warnings denied, both with and without `cfg(test)`
(`clippy --all-targets`), and its `equiv_priv` tests pass under plain, ASan+UBSan and Miri.
`ffimap api` reports the enums `new`, with no public conversion for a later stage to remove.

## Review

None yet.

`review-1` (session `aefe14bc-222f-4cc7-8b16-8ae56ed9e941`, Opus 5.5, `$1.99`, 7m03s), on `166d8fd`: **rejected**, on three fixes: the rule contradicted `porting` for an enum wrapped before its port, whose public conversions the port keeps; the `dead_code` expectation as stated fails the lint with the export feature on; and the row's recurrence signal fired on those wrapped enums. Made as `c461f16`, on `180fd67`: the rule covers the conversions a port writes for an enum no wrapper gave a safe API, the expectation's sentence is gone, and the row is scoped.
Its report is the longest `result` of `crustify/artifacts/evolution/enum-integer-form-crate-private/review-1/stream.jsonl`.

`review-2` (session `55481535-6db1-4da3-83c1-7369c44693b1`, Opus 5.5, `$0.40`, 1m20s), on `c461f16`: **approved**. Each required fix of
review-1 is resolved, the fix opened no contradiction and the row matches the change.
Not required: this proposal's Evidence still calls the wrapped enums' integer form permanent, which the schedule contradicts, and its Change section describes the first commit.
