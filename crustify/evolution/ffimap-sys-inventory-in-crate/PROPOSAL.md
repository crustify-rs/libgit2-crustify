# ffimap takes its `-sys` inventory from extern crates alone, so in-crate raw bindings have none

Status: `open`: the change is ffimap `8cd7a26` on `libgit2-crustify/evolution/`, made on 2026-11-14 at
the user's word, awaiting its review.

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

In the driver: an entry of `--sys-crates` written `crate::<path>` names a module of the scanned
crate (`sys_modules`). `count_sys_types` takes the module's definitions, and those of the
modules below it (`in_sys_module`), into the inventory through the same arms as a named
crate's items. The counters the inventory gates are emitted when a crate or a module is
named. The CLI's help, `README.md`, `docs/analysis.md` and `docs/schema.md` state the form,
and one difference: a named crate gives the items a dependent can reach, a named module every
item it defines.

The `cells` fixture gains `raw/bindings.rs`, a module of the crate with one struct and two
functions, and `test_a_module_of_the_crate_joins_the_inventory_when_named`.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `--sys-crates` names a module of the scanned crate as `crate::<path>`, whose raw bindings then join the `-sys` inventory | `libgit2-crustify`, `` | ffimap `8cd7a26` | once this campaign's raw bindings became a module of its crate, its scans read PCRE2's inventory alone, and two dashboard cells measured nothing of its batches | a scan whose `ty.ffibox.sys.total` or `fn.ffibox.sys.total` falls by an order of magnitude from the scan before it with no binding removed, or a campaign whose bindings are a module scanning without naming it | watched |

## Validation

- ffimap's suite, in its own checkout, outside any Cargo workspace: 146 tests pass. The new test
  expects one more C type, behind a pointer, two more functions and one more wrapper when
  `crate::raw` is named beside `cells-sys`, and the 21 other driver tests are unchanged, the
  module unnamed there.
- On real code, libgit2-crustify at `6d3c7873c`, the campaign's options with
  `--sys-crates crate::sys,pcre2_sys` against `libgit2_sys,pcre2_sys` on the old driver: 22 of
  767 values move, every one the inventory's or a wrapper counter's.

  | | old driver | new driver, `crate::sys` named | 2026-11-10 |
  |---|---|---|---|
  | `ty.ffibox.sys.total` | 21 | 128 | 120 |
  | `…raw_ptr_in_sys.of_which.used` | 5 | 106 | 78 |
  | `…raw_ptr_in_sys.of_which.wrapped` | 2 | 77 | 70 |
  | `fn.ffibox.sys.total` | 222 | 785 | 723 |
  | `fn.ffibox.sys.of_which.used` | 6 | 509 | 452 |
  | `fn.ffibox.wrapper.total` | 0 | 590 | not read |
  | `ty.ffibox.wrapper.fields.total` | 0 | 528 | not read |

  The scans are `crustify/tmp/assistant/ffimap-scan/scan.json` and `scan-new.json`.

## Review

None yet.
