# A header module's `static inline` functions are exported, under a feature named for its header

Status: `open`.

Goal: Consistency. A `static inline` function of a header module is one kind of item with
any other module's, and both take one pattern, an export the remaining C calls after the
switch. Interoperability: the C that stays reaches the module through the boundary.
Fitness: the conventions findings reviews file, per item (Consistency's measure), here six
items of one module.

## Signal

Review findings, and an agent prompt that led its agent wrong. `wave-223/batch-1`, the
module pass of the header module `src/util/integer.h`, filed
`conventions/static-inline-functions-without-exports`. `wave-223/batch-0` ported the header's
six `static inline` functions with no export and no gateway, though C in 19 files calls
them. Its docs cite "porting's Macros and Static inline functions sections" as keeping the
header's items in the header.

## Evidence

- The batch's task message, written for the campaign's first header module with `static
  inline` functions, said "An item the remaining C still expands from the header needs no
  export". The batch read "expands" as covering the functions' bodies, which every caller
  inlines, as well as the macros.
- `porting-symbols`' Static inline functions section says what the switch does with such a
  function: "the header's definition is replaced by its prototype, and the remaining C calls
  the Rust module's export". It does not say that this holds where the module is the header
  alone and the header stays. The Macros section, the one the batch followed, says that a
  header macro "stays in its header for the remaining C, unexported". A header module holds
  both kinds, and the two sections read as one rule.
- The conventions' Export features section names a module's feature `ffi-export-<tu>`,
  "with `<tu>` its translation unit's path below the C root". A header module's
  `translation_unit` is `null` (`modules-schema`), so it has no name to give its feature.
  The review named it for the header, `ffi-export-src-util-integer-h`.
- Earlier header modules (`src/util/strnlen.h`, `deps/ntlmclient/compat.h`) held macros
  alone on this build, so none had met the case.

## Destination

The spec: `porting-symbols`' Static inline functions section, and the conventions' Export
features section.

## Change

In `porting-symbols`' Static inline functions section, after its second sentence, add:

> This holds for a header module too, the header alone, which stays while the remaining C
> includes it: its macros stay in it unexported, as Macros describes, and its `static
> inline` functions are exported, their definitions replaced by prototypes at the switch.

In the conventions' Export features section, after its first sentence, add:

> A header module, which has no translation unit, takes its header's path in its place.

## Validation

Not yet validated. The rerun is the item that exposed it, `wave-223/batch-0`'s six
`fn::git__is_*`, under the change and the same task message. It is expected to write the
gateway under `ffi-export-src-util-integer-h` and to document the header's prototypes, as
`wave-223/batch-1`'s fix did.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a header module's `static inline` functions are exported, though the header and its macros stay; a header module's export feature takes its header's path | `libgit2-crustify`, `header-module-exports` | — | a header module's `static inline` functions were ported with no export, read as kept with its macros, and its export feature had no name | a header module whose `static inline` functions the remaining C calls lands with no export, or names its export feature otherwise | — |

## Review

None yet.
