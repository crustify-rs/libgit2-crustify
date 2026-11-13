# `sort_gnu_style` reorders the arguments after a literal `--`

Found by `wave-186/batch-5/run-1` while porting `sort_gnu_style`.

A GNU-style parser sorts once, at the first bare argument it meets. When a literal `--` comes
before every bare argument, `parse_long` sets `in_literal` and `needs_sort` is still set, so the
first bare argument after the `--` sorts the rest: `sort_gnu_style` stops only at a `--` it
scans, not at one already parsed, and moves every later argument that looks like one of the
table's options ahead of the others. Those are literal (`opt.h`: "everything after this is
literal"), so the bare argument specs receive them in another order: `-- a -d` sets the first
argument to `-d` and the second to `a`. The CLI's `blame`, `init`, `clone` and `index-pack` take
a literal `--`: `git2 init -- dir -q` takes `-q`, `init`'s `--quiet`, as its directory and
reports `dir` an unknown option, where `dir` is the directory and `-q` the extra argument.

```bash
clang -g -O0 -D_GNU_SOURCE -DGIT_DEPRECATE_HARD -DNDEBUG $INC main.c c/src/cli/opt.c -o repro
./repro   # status 0 d 0 first -d second a
```

(`$INC` as in `../cli-opt-short-bundle-unknown-reads-past/README.md`.) The port
(`reorder_gnu_style`) reorders nothing once the parser is literal, still clearing `needs_sort` as
C does, kept by `unit_pub`'s `nothing_after_a_literal_is_reordered`.
