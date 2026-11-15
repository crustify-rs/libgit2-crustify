# Where an exported global's storage is defined while its export feature is off

Status: `open`.

## Signal

A rule an agent had to work out by hand that a skill should state. `porting-symbols`'
Globals section has a ported global's storage held by Rust "from its port, beside its
module's C, which keeps its own copy until the switch; from then on the Rust's is the only
one", and a global the remaining C accesses "is exported, as a function is". The
conventions' Exports and Export features sections put every export in the module's
`ffi_export` gateway, compiled only under the export feature, which stays off until the
switch, and put every `#[unsafe(…)]` attribute in a region. For a function, the native
function and its export are two items, so the native one compiles with the feature off and
the export calls it. A global's export is its storage: the symbol C reads is the static
itself. With the feature off, the native API needs the storage, which the gateway cannot
hold; with it on, a second static in the gateway would make two Rust copies, against "the
Rust's is the only one"; and `#[unsafe(no_mangle)]` or `#[unsafe(export_name)]` on the
native static, under a `cfg_attr`, would put unsafe code outside the regions. No skill says
which of these a port takes, and the crate held no exported global to follow: the
switched modules' exports are all functions.

## Evidence

`wave-221/batch-1/run-1` (`src/util/hash/sha1dc/ubc_check.c`), whose table `sha1_dvs`
`sha1.c`'s C reads (`sha1_process`) and keeps reading through the export once the module
switches, since `sha1.c` stays C past the campaign's first ship point. The batch defined
the table's data once, as a private `const DVS`, and its storage under the two exclusive
configurations:

```rust
#[cfg(not(feature = "ffi-export-src-util-hash-sha1dc-ubc-check-c"))]
pub static SHA1_DVS: [DvInfo; DV_COUNT] = DVS;

#[cfg(feature = "ffi-export-src-util-hash-sha1dc-ubc-check-c")]
pub use self::ffi_export::sha1_dvs as SHA1_DVS;

// in `mod ffi_export`:
#[unsafe(no_mangle)]
pub static sha1_dvs: [DvInfo; DV_COUNT] = DVS;
```

so that each build holds one Rust copy, which the native API names by its Rust name in
both, and the remaining C reads that copy once the feature is on. With the feature on, the
test binaries link it in place of the archive's `ubc_check.c.o`: `nm` shows one `sha1_dvs`,
`R`, and none of the TU's `static const` bit constants, and a gateway `unit_priv` test
checks that `&raw const sha1_dvs` and `&raw const SHA1_DVS` are one address.
`non_upper_case_globals`, which the workspace denies, exempts a `#[no_mangle]` static, and
`build.rs`'s `exported_name` reads the `pub static sha1_dvs` line as an export. At the
switch, deleting the feature removes the `cfg(not(…))` static and the `cfg` of the
re-export, and the storage stays the gateway's until the global's `port/unexport` moves it
back to the native code.

The batch's module pass (`wave-221/batch-3/run-1`) reviewed the pattern, as the Change below
waits for. It holds where it was meant to: with the feature on, the relinked `libgit2_tests` and
`util_tests` hold one `sha1_dvs`, `R`, which `sha1.c` reads through every offline C suite (3442
passed, the `plain` baseline). It missed one thing: the documentation. The native static's
description and its `#[doc(alias = "sha1_dvs")]` are on the `cfg(not(feature))` static alone,
and the gateway's static cannot take the alias, rustdoc rejecting one equal to its item's name,
so with the feature on `SHA1_DVS` lost both, and would have lost them for good at the switch
(`conventions/sha1-dvs-alias-missing-with-the-export-feature`). The fix put the alias and the
description on the `cfg(feature)` re-export, which rustdoc shows before the gateway's text. So
the paragraph the Change drafts should say that the re-export carries the native item's alias
and description, and the gateway's static the anchor, so that the switch, deleting the
`cfg(not(…))` static, keeps all three.

## Goal

Consistency (the same case written the same way in every module) and Interoperability
(one copy of each global, which the remaining C and the Rust share). Fitness: a review
finding of an exported global held twice, or of its export outside the gateway, which this
rule prevents.

## Destination

The spec: `porting-symbols`' Globals section, which the translator's role evolves; the
conventions' Exports section names it.

## Change

Not made yet, pending review of the pattern on this batch's module pass. One paragraph in
`porting-symbols`' Globals section, after "is exported, as a function is": the global's
data is one constant; while its export feature is off, the native static holds it under
`cfg(not(feature))`; under the feature, the gateway's `#[unsafe(no_mangle)]` static, under
the C name, is the storage, and the module re-exports it under the native name, so that
each build has one copy and the API's path does not change; the switch removes the native
static and the re-export's `cfg`; its `port/unexport` moves the storage back to the native
code. And a sentence in `porting-workflow`'s Builds section's first step, naming the
re-export among the mentions of the feature the switch rewrites.

## Record

The row for `EVOLUTION.md`, copied in when the change lands.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| an exported global's one storage is the native static while its export feature is off and the gateway's `no_mangle` static, re-exported under the native name, once it is on | `libgit2-crustify`, `exported-global-storage-before-the-switch` | — | the first exported global of a port campaign had to work out where its storage is defined while its gateway compiles only under the export feature | an exported global held in two Rust copies, or exported from outside its gateway | watched |

## Validation

The batch above: `clippy --workspace --all-targets -D warnings` with the feature off and on;
the module's tests plain, under ASan+UBSan, TSan and Miri with the feature off, and its
gateway's tests with it on, under Miri too; with the feature on, `sha1.c`'s own C
(`SHA1DCInit`, `SHA1DCUpdate`, `SHA1DCFinal`, called from a scratch test) still detects the
collision of `shattered-1.pdf` through the exported table and check, and does not once one
bit of the file changes.

## Review

Pending.
