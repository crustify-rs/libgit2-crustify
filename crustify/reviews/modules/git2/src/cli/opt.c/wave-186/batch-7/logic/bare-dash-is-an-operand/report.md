# The parser reports the bare `-` as an unknown option, where `getopt` takes it as an operand

- **finding**: `logic/bare-dash-is-an-operand`
- **batch**: `wave-186/batch-7/run-1`, the module pass of `git2/src/cli/opt.c`.
- **item**: `fn::cli_opt_parser_next`, `ParserMut::parse_next` in `src/git2/src/cli/opt.rs`, and
  through it `fn::cli_opt_parse` (`parse`) and `fn::cli_opt_foreach` (`for_each`), and their
  exports (`wave-186/batch-5`, `c62dee857`; the short-option lookup `find_short` is
  `wave-186/batch-4`'s, `64431c07a`).
- **affected commit**: `961150dd9`, the wave's tip; since `c62dee857`.
- **class**: a logic defect of the native API, which the CLI's remaining C reaches through the
  exports once the module switches from C.

## Finding

`parse_next` dispatched every argument that starts with `-`, and is not a literal, to the
short-option parser. For the bare `-`, `find_short` finds no option character, so the parse
reported `Status::UnknownOption`, and `parse` failed on it, in either style.

The specification the parser implements takes the bare `-` as an operand:

- POSIX `getopt` (XSH, `getopt`): "If, when `getopt()` is called, `argv[optind]` ... points to
  the string `"-"`, `getopt()` shall return -1 without changing `optind`": it is the first
  operand, not an option. POSIX's Utility Syntax Guideline 13 gives it its meaning, standard
  input, where a utility takes files.
- GNU `getopt_long`, which `opt.h` names as the style of `CLI_OPT_PARSE_GNU` ("Parse with GNU
  `getopt_long` style behavior, where options can be intermixed with arguments"), takes it as a
  non-option argument, which it permutes as it permutes any other.
- git's own `parse-options`, whose commands the CLI's mirror, takes it as a non-option argument
  (`parse_options_step`: `if (*arg != '-' || !arg[1])`): `git init -` creates a repository in
  the directory `-`, and `git blame -` looks the path `-` up.

The port departed from all three. The C departs too, differently: `spec_for_short` compares the
empty option character with every spec's alias, so the bare `-` selects the table's first spec
without one, and `parse_short` and `spec_for_sort` then read the byte after the argument's end.
Batch 4 corrected that read, by matching no spec, and the bare `-` became unknown; batch 6 left
the choice to this pass. Nothing in the module's documentation, its tests or a command of the
CLI gives the bare `-` an option's meaning, and its departure from the cited specifications was
never weighed.

## Evidence

`src/main.rs` here parses, through the safe API alone (`#![forbid(unsafe_code)]`), `-`, `-q -`
and `- -q` against a table of a `-q` bool and one argument spec, in the default style and
`FORCE_GNU`, and asserts what `getopt` gives: the argument spec takes `-`, and `-q` is set where
given. From the repository root:

```text
CARGO_TARGET_DIR=crustify/tmp/target-repro crustify/scripts/cargo-variant.sh plain run -q \
  --manifest-path crustify/reviews/modules/git2/src/cli/opt.c/wave-186/batch-7/logic/bare-dash-is-an-operand/Cargo.toml
```

On `961150dd9` it exits 1 (`run-affected.log`):

```text
["-"] ParseFlags(0): argument None, failure Some((Some(UnknownOption), Some("-")))
["-"] ParseFlags(2): argument None, failure Some((Some(UnknownOption), Some("-")))
["-q", "-"] ParseFlags(0): argument None, failure Some((Some(UnknownOption), Some("-")))
["-", "-q"] ParseFlags(2): argument None, failure Some((Some(UnknownOption), Some("-")))
the bare `-` is not taken as an operand
```

What the CLI shows today, from the plain variant's `git2`: `git2 init -` and
`git2 hash-object -` print their usage and exit 0, the C having taken `-` as `--help`; on the
affected commit the exports, which `parse_next` serves, would report `unknown option: -`, as the
reproducer's failures show.

## Fix

`parse_next` takes the bare `-` as a bare argument, before the short-option branch: it takes the
next argument spec, or is unknown once those are taken, as any bare argument is, and a GNU-style
parse keeps it among the bare arguments it moves options ahead of, as `find_for_sort` already
selected no spec for it. The documentation of `parse_next`, of the module, and the C defect it
lists say so. The reproducer exits 0 on the fix (`run-fixed.log`).

It becomes the `unit_pub` test `cli::opt::a_bare_dash_is_a_bare_argument`, which replaces
`a_bare_dash_is_unknown`, the test that asserted the departure: `-` alone, after an option and
before one GNU style moves, in both styles, and unknown once the argument spec is taken. The
gateway's test `a_bare_dash_reaches_the_command_as_an_argument` checks it through
`cli_opt_parse`. With the exports linked into the CLI, `git2 init -` creates the repository
`-/.git`, as `git init -` does, `git2 hash-object -` reports that it cannot find `-`, and
`git2 -` reports `'-' is not a git2 command`, exit 1. The stock C prints `unknown option: -`
there, exit 129, but reaches it by reading the bytes past the argument as further options;
git's own top level, which is not `getopt`, reports `unknown option: -`.

API stability: no item's signature changes; `parse_next`, `parse` and `for_each` accept the
bare `-` as an argument, which they refused, a behaviour change recorded in the batch's Notes.
