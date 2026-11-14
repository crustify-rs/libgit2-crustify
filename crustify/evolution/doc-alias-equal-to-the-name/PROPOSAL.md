# An anchor's alias that equals its item's name

Status: `landed` as crustify `1b0d20d` and ffimap `c3203c7`, each on its `libgit2-crustify/devel`
(record: crustify `0504357`), on 2026-11-14. ffimap's `devel` took the ffimap half on 2026-11-14, with the user's approval (`c3203c7`, a
fast-forward, not pushed), so the shared install runs it.

Goal: Idiomatic Rust: an item keeps the name the Naming section gives it.

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

`wave-186/batch-4/run-1` met it seven times over, porting `opt.c`'s static helpers
`spec_for_long`, `spec_for_short`, `spec_for_arg`, `spec_is_choice`, `consume_choices`,
`spec_included` and `validate_required`, each refused as above. They are private, so the
batch renamed them for what they do (`find_long`, `find_short`, `find_arg`, `is_choice`,
`skip_choice_args`, `is_given`, `check_required`) and kept each anchor and alias. A batch that
ports a module's helpers meets it as a rule of the module's whole route, not of one item.

`wave-186/batch-5/run-1` met it five times more, for `parse_long`, `parse_short`, `parse_arg`,
`spec_for_sort` and `sort_gnu_style`, each private, renamed `take_long`, `take_short`,
`take_bare`, `find_for_sort` and `reorder_gnu_style` with their anchors and aliases kept: every
static function of `opt.c` that the route ported has needed it.

## The second campaign

glibc-crustify's `wave-4/batch-0/run-1` met it on a public item: the global `stderr`, bound
as `pub fn stderr()`, whose alias rustc rejects the same way. It kept the name and dropped
the alias, and `ffimap api` then listed the item under `anchors.without_alias`.

## A third occurrence, in this campaign

`wave-169/batch-2` (`2032628fd`) landed two wrappers that keep their C function's name and
carry no alias, `binsearch` and `parse_names` in `src/libgit2/deps/reftable/basics.rs`, each
`Wraps fn::<name>`: glibc-crustify's way out, taken here before either proposal was filed.
`ffimap api` lists exactly these two under `without_alias` today. The ffimap half's review
found them.

## Decision

The two campaigns took opposite ways out: this one renamed its thirteen private helpers and
kept each alias, glibc-crustify kept the name and dropped the alias. The user chose, on
2026-11-14: the alias is dropped, for such an item alone. A rename changes the product to
suit a documentation tool, and suits a public item badly; a mangled alias would not match
the search the alias serves. The thirteen helpers already renamed stay as they are.

## Destination

The spec: `conventions`' Anchors section. And a tool: `ffimap api`'s
`anchors.without_alias`.

## Change

In `conventions`, after "…a label in the alias would no longer match that search.":

> An item whose own name is that path, a name of the source the Rust keeps, carries no alias:
> rustdoc rejects an alias equal to its item's name, and the name already answers the search.
> Its anchor is unchanged.

In ffimap, `api`'s extractor leaves out of `without_alias` an anchor whose path is its item's
own name, `docs/api.md` says so, and the `api` fixture holds such an item, `same_name`, which
the old check reports.

Not made, and known: the driver's `fn.ffibox.wrapper` counters read a wrapper from its alias
alone (`docs/schema.md`), so a function that wraps a C function and keeps its name, with no
alias, is not counted, and the share of C functions with a wrapper reads lower by each one.
This campaign has two such functions already, `binsearch` and `parse_names` above, so its
counters may read low by two; whether they do depends on `--sys-crates` reaching bindings that
are a module of the crate, which was not run. Counting it needs the driver to read
the anchor, and is filed apart, as `ffimap-wrapper-without-alias`.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| an item whose own name is its anchor's path carries no alias, and `ffimap api` does not report it as lacking one | `libgit2-crustify`, `doc-alias-equal-to-the-name` | crustify `1b0d20d`; ffimap `c3203c7` | rustdoc rejects an alias equal to its item's name: two campaigns met it, on sixteen items, and chose opposite ways out | a batch that renames an item, or mangles an alias, because its alias would equal its name; or `ffimap api` listing such an item under `without_alias` | watched |

## Validation

`wave-186/batch-1/run-1` builds with the rename and the alias kept; with the alias equal to
the name the library fails to build (above).

The ffimap change: its whole suite passes (145 tests), and the fixture's `same_name` fails the
old check.

## Review

`review-1-crustify` (Opus 5.5, `$1.05`), on `a5ae2ea`: **approved**, no fix required; it weakens no check.
It landed as `1b0d20d`, that commit rebased, its file the same in the commit and its parent. Its row
is recorded once the ffimap half lands. Listed apart: `conventions` § 3.2's "the same anchor
and alias" could say "where it carries one"; ffimap's `docs/schema.md` still calls the alias
the one the conventions require.

`review-1-ffimap` (Opus 5.5, `$1.56`), on ffimap `c3203c7`: **rejected**, for this proposal's text
alone: it said no campaign had a wrapper that keeps its C name without an alias, and this
campaign has two. Corrected above; the code, its tests and its fit with ffimap's goals held,
and it weakens no check. On this campaign's rustdoc output, `without_alias` goes from those two
entries to none, the 1227 item records unchanged.

`review-2-ffimap` (Opus 5.5, `$0.29`), on ffimap `c3203c7`: **approved**.
