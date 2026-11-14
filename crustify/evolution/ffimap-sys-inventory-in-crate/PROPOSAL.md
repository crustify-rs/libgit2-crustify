# ffimap takes its `-sys` inventory from extern crates alone, so in-crate raw bindings have none

Status: `open`. No change is made yet.

Goal: Safety, the unsafe surface's measures; Attribution.

## Signal

A tool that reports what the source contradicts: a campaign whose raw bindings are a module of its crate gets no `fn.ffibox.sys` or `fn.ffibox.wrapper` counters, as though it declared and wrapped no C function.

## Evidence

Found by `ffimap-wrapper-without-alias`'s review-1 on 2026-11-14, reading the driver at ffimap `2a2aa1e`: `count_sys_types` walks `tcx.crates(())`, the extern crates, for those `--sys-crates` names. Since 2026-11-13 this campaign's raw bindings are `crate::sys`, under `sys/<link unit>/`, a module of `libgit2-crustify`, which `--sys-crates` cannot name. glibc-crustify's are a crate, `libc-sys`, and are counted (`fn.ffibox.sys.total` 23). 
Measured on 2026-11-14, on `crustify/devel` at `6d3c7873c`, with the options of the last scan the
dashboard records (`--ffibox --sys-crates libgit2_sys,pcre2_sys`, the regions and seams):

| | 2026-11-10, bindings a crate | 2026-11-14, bindings a module |
|---|---|---|
| `ty.ffibox.sys.total` | 120 | 21 |
| `fn.ffibox.sys.total` | 723 | 222 |
| `fn.ffibox.wrapper.total` | not read | 0 |

What is left is PCRE2's, the one `-sys` crate that is still a crate. The batches since the
bindings moved scanned with `--sys-crates pcre2_sys` and record the same 21 and 222
(`wave-194`, `wave-197`, `wave-225`, `wave-229`, `wave-232`'s `ffimap.json`). The dashboard's
`C by value` and `celled %` cells read `ty.ffibox.sys`, so for those batches they are taken
over PCRE2's types alone: each shows `+0` and `+0.00 pp`, whatever the batch did with a
libgit2 type. The scan is `crustify/tmp/assistant/ffimap-scan/scan.json`.

## Destination

A tool: ffimap's driver and `--sys-crates`, or the `ffimap` tool skill's account of how a campaign with in-crate bindings names them.

## Change

Not made. The inventory would also take a module of the scanned crate, named by its path. Until then the two cells are not measures of this campaign's batches, and the dashboard should say so.

## Validation

None yet.

## Review

None yet.
