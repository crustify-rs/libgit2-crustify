# An anchor's alias that equals its item's name

Status: `open`.

## Signal

A rule an agent had to work out by hand that a skill should have stated: an annotation the
conventions require does not compile.

## Evidence

`wave-186/batch-1/run-1` (`git2/src/cli/opt.c`) ported the static function
`support_gnu_style`. The conventions' Naming section keeps a C name that already reads as a
Rust one (`snake_case`, no library prefix to drop), and their Anchors section puts
`#[doc(alias = "support_gnu_style")]` beside the `Replaces` anchor. On Rust 1.100.0 rustdoc's
check, which `cargo build` runs on every item, rejects it:

```text
error: `#[doc(alias = "support_gnu_style"]` is the same as the item's name
   --> src/git2/src/cli/opt.rs:102:15
```

The batch renamed the port `uses_gnu_style`, which a Rust caller reads as the predicate it
is, and kept the alias, so that a search of the docs for the C name still finds it. The
anchor alone would have recorded the translation, since a search by the item's own name
finds the item without an alias, but `ffimap` counts a wrapper by its `doc(alias)`
(`fn.ffibox.wrapper.*`) and `ffimap api` keys items by their anchors; neither was checked
for an anchor without its alias.

Any C function, global or macro whose name is already a valid, unprefixed Rust name meets
it: the CLI's static helpers (`opt.c`'s `spec_included`, `consume_choices`,
`validate_required`, `parse_long`, `parse_short`, `parse_arg`, `sort_gnu_style`, and the
`cmd_*.c` helpers) among them.

## Destination

The spec: the conventions' Anchors section (`rust/conventions`), which the translator and
binder do not evolve.

## Change

Add, after the paragraph on the alias:

> rustdoc rejects an alias equal to its item's name. A stand-in whose Rust name would equal
> its C name is therefore named otherwise, by the Naming section's rules, the idiomatic Rust
> term preferred, such as a predicate's `is_`, `has_` or `uses_` form, so that the alias, and
> the search it serves, is kept on every annotated item.

## Validation

`wave-186/batch-1/run-1` builds with the rename and the alias kept; with the alias equal to
the name the library fails to build (above).
