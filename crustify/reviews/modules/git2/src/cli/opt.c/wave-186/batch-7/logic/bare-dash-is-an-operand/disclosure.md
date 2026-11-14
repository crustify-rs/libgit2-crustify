For: libgit2-crustify's issue tracker (https://github.com/crustify-rs/libgit2-crustify/issues),
as a public issue. The defect is in the crate's own Rust port of the CLI's option parser, and
has no security impact we know of: a command-line argument refused.

Title: the ported option parser refuses the bare `-`, which `getopt` takes as an operand

### Summary

The Rust port of the CLI's `src/cli/opt.c` (`src/git2/src/cli/opt.rs`) parses every argument
that starts with `-` as a short option, the bare `-` included, and reports it as an unknown
option:

```rust
if in_short || (bytes.starts_with(b"-") && !in_literal) {
    return take_short(self);
}
```

POSIX `getopt` returns at `"-"` without consuming it, as the first operand; GNU `getopt_long`,
whose style `opt.h` names, and git's `parse-options` take it as a non-option argument, so that
`git init -` creates the directory `-`. libgit2's own C parser matches the bare `-` with the
first spec that has no alias and reads past the argument's end, so `git2 init -` prints the
usage; the port corrected that read, but refused the argument.

### Reproduction

Parsing against a table of a `-q` flag and one argument spec:

```text
["-"] ParseFlags(0): argument None, failure Some((Some(UnknownOption), Some("-")))
["-", "-q"] ParseFlags(2): argument None, failure Some((Some(UnknownOption), Some("-")))
```

The reproducer, a small Cargo package using the safe API alone, is available on request.

### Fix

Fixed in `wave-186/batch-7/run-1`: the bare `-` is a bare argument, in either style. A patch is
available.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
