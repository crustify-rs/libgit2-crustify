# ffimap takes its `-sys` inventory from extern crates alone, so in-crate raw bindings have none

Status: `open`. No change is made yet.

Goal: Safety, the unsafe surface's measures; Attribution.

## Signal

A tool that reports what the source contradicts: a campaign whose raw bindings are a module of its crate gets no `fn.ffibox.sys` or `fn.ffibox.wrapper` counters, as though it declared and wrapped no C function.

## Evidence

Found by `ffimap-wrapper-without-alias`'s review-1 on 2026-11-14, reading the driver at ffimap `2a2aa1e`: `count_sys_types` walks `tcx.crates(())`, the extern crates, for those `--sys-crates` names. Since 2026-11-13 this campaign's raw bindings are `crate::sys`, under `sys/<link unit>/`, a module of `libgit2-crustify`, which `--sys-crates` cannot name. glibc-crustify's are a crate, `libc-sys`, and are counted (`fn.ffibox.sys.total` 23). Not checked here: which of this campaign's dashboard cells read a `sys` inventory, and what they show since the bindings moved.

## Destination

A tool: ffimap's driver and `--sys-crates`, or the `ffimap` tool skill's account of how a campaign with in-crate bindings names them.

## Change

Not made. The inventory would also take a module of the scanned crate, named by its path.

## Validation

None yet.

## Review

None yet.
