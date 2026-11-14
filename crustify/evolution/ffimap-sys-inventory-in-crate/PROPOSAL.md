# ffimap takes its `-sys` inventory from extern crates alone, so in-crate raw bindings have none

Status: `open`: the change is ffimap `a0797e6` on `libgit2-crustify/evolution/`, made on 2026-11-14 at
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
`celled %` cell reads the inventory (`…raw_ptr_in_sys.of_which.wrapped` over `….used`), so
for those batches it is taken over PCRE2's types alone, 2 of 5, and its delta reads `+0.00 pp`
whatever the batch did with a libgit2 type.

A second defect came with the same move, and reaches further. The bindings' own code, bindgen's
derives, its `Default` impls and its declarations, is now code of the scanned crate, and
counts as the crate's wherever a counter does not skip a generated file: `ty.ffibox.sys.as`,
which the `C by value` cell reads, went from 111 on 2026-11-10 to 355, with or without the
module named, and `ty.impl_trait`, the `unsafe` counters and `fn.total` rose with it.
The scan is `crustify/tmp/assistant/ffimap-scan/scan.json`.

## Destination

A tool: ffimap's driver and `--sys-crates`, or the `ffimap` tool skill's account of how a campaign with in-crate bindings names them.

## Change

In the driver: an entry of `--sys-crates` written `crate::<path>` names a module of the scanned
crate (`sys_modules`). Its definitions, and those of the modules below it (`in_sys_module`,
which resolves the named modules once), are read as a named crate's items are:

- `count_sys_types` takes them into the inventory, through the same arms as a crate's; and
- every other walk over the crate's definitions and bodies, thirteen of them, `named_defs`
  among them, leaves them out, so that the bindings' own code is not the crate's and does not
  count as a use of the types it declares.

The CLI's help, `README.md`, `docs/analysis.md` and `docs/schema.md` state the form.
The `cells` fixture gains `src/raw.rs` under a feature, bindings as bindgen writes them, and
two tests.

Its other half is one paragraph of Crustify's `ffimap` tool skill, crustify `9885b23`: a
campaign whose raw bindings are a module of its crate names them to the scan.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `--sys-crates` names a module of the scanned crate as `crate::<path>`: its raw bindings join the `-sys` inventory and leave every other counter, and the `ffimap` skill has a scan name them | `libgit2-crustify`, `ffimap-sys-inventory-in-crate` | ffimap `a0797e6`; crustify `9885b23` | once this campaign's raw bindings became a module of its crate, its scans read PCRE2's inventory alone and counted the bindings' own code as the crate's, so `celled %` and `C by value` measured nothing of its batches | a scan whose `ty.ffibox.sys.total` or `fn.ffibox.sys.total` falls by an order of magnitude from the scan before it with no binding removed; a counter with a site inside a generated bindings file; or a campaign whose bindings are a module scanning without naming it | watched |

## Validation

- ffimap's suite, in its own checkout, outside any Cargo workspace: 147 tests pass. Two are new.
  One expects, with `crate::raw` named, one more C type behind a pointer, two more functions
  and one more wrapper, and no other inventory or wrapper counter moved, `used` included. The
  other expects that the module's code counts as the crate's while unnamed, that named, no
  counter but the inventory's has a site in it, and that every counter then reads what the
  crate reads without the module. The 21 other driver tests are unchanged.
- On real code, libgit2-crustify at `6d3c7873c`, the campaign's options:

  | | 2026-11-10, bindings a crate | bindings a module, old driver | new driver, `crate::sys` named |
  |---|---|---|---|
  | `ty.ffibox.sys.total` | 120 | 21 | 128 |
  | `…raw_ptr_in_sys.total` | 115 | 21 | 122 |
  | `…raw_ptr_in_sys.of_which.used` | 78 | 5 | 85 |
  | `…raw_ptr_in_sys.of_which.wrapped` | 70 | 2 | 77 |
  | `ty.ffibox.sys.as.total` | 111 | 355 | 122 |
  | `ty.ffibox.sys.as.by_val.total` | 111 | 219 | 119 |
  | `fn.ffibox.sys.total` | 723 | 222 | 785 |
  | `fn.ffibox.sys.of_which.used` | 452 | 6 | 509 |
  | `fn.ffibox.wrapper.total` | 511 | 0 | 590 |

  84 of 767 values move against the old driver: the inventory's, the wrapper counters', and the
  counters the bindings' own code had inflated. `celled %` reads 77 of 85, where 2026-11-10
  read 70 of 78. The scans are `crustify/tmp/assistant/ffimap-scan/scan.json` (old driver) and
  `scan-new2.json`; `scan-new.json` is review-1's commit, which read 106 used.

## Review

`review-1` (Opus 5.5, `$2.63`), on ffimap `8cd7a26`: **rejected**, four fixes required; it weakens no
check. A named crate was unchanged, the module walk took the right items, and the totals
checked against the bindings. The fixes, made as `a0797e6`: (1) the bindings' own declarations
counted as uses of their types, so `used` read 106 and `celled %` a wrong ratio; (2) the
bindings' own code counts as the crate's, a second signal, which the change now answers by
having a named module leave every other counter; (3) this proposal's evidence was wrong about
`C by value`, which does not read the inventory; (4) no run would name the module unless the
`ffimap` skill says so, which crustify `9885b23` adds. Its sturdier matching of the module is
taken too.

`review-1-crustify` (Opus 5.5, `$1.13`), on crustify `3dcc464`: **rejected**, one fix: the paragraph
named every in-crate bindings module, and in an audit a hand-written `mod ffi` so named would
leave the scan unseen. Made as `9885b23`: it is limited to bindings generated as `bindgen` describes,
says a named module leaves every other counter, and leaves a module with hand-written code
unnamed. The paragraph sits in a section that describes a scan a landing passes, a protected
path: the user reads it before it lands, after the ffimap half.
