# `sort_gnu_style` skips the bare arguments before a last option whose value is optional

Found by `wave-186/batch-5/run-1` while porting `sort_gnu_style`.

When the option `sort_gnu_style` meets is a `CLI_OPT_TYPE_VALUE` that needs a value and is the
last argument, it sets `parser->idx` to it and returns, "so that we fail on it": the parse
resumes at the option, past the bare arguments between the old position and it. A required
value then fails, `CLI_OPT_STATUS_MISSING_VALUE`, and the parse stops. A value marked
`CLI_OPT_USAGE_VALUE_OPTIONAL` does not fail: the option parses, the parse ends, and the bare
arguments it skipped are never parsed, as if they were not given. `file -S` reports the required
`file` missing; were it optional, `file` would be dropped silently. The CLI's `commit` takes
`-S`/`--gpg-sign` so.

```bash
clang -g -O0 -D_GNU_SOURCE -DGIT_DEPRECATE_HARD -DNDEBUG $INC main.c c/src/cli/opt.c -o repro
./repro   # status 4 spec file file (null)
```

(`$INC` as in `../cli-opt-short-bundle-unknown-reads-past/README.md`.) The port
(`reorder_gnu_style`) leaves such an option in place, the last argument, so that the bare
arguments before it parse first; a required one still moves the parser to it, as C does. Kept by
`unit_pub`'s `a_last_optional_value_loses_no_argument`.
