# `cli_opt_parse` writes past its `given_specs`

Found by `wave-186/batch-4/run-1` while comparing `check_required` (`validate_required`'s
port) with C's `cli_opt_parse`; the item is `wave-186/batch-5`'s.

`cli_opt_parse` sizes `given_specs` as `alloca(sizeof(const cli_opt_spec *) * (args_len + 1))`,
one entry per argument and the `NULL` that ends it, but stores one entry per outcome
`cli_opt_parser_next` returns, and a bundle of short options (`-ab`) returns one outcome per
option. `-ab` alone stores two specs and the `NULL` in two entries. The CLI's commands reach it:
`git2 cat-file -tsep <object>` stores four specs and the `NULL` for two arguments, on the stack.

Run with the `plain` variant's include path (`build.ninja`'s `INCLUDES` for `opt.c`), from the
batch's worktree:

```bash
clang -g -O0 -fsanitize=address -D_GNU_SOURCE -DGIT_DEPRECATE_HARD <INCLUDES> main.c c/src/cli/opt.c -o repro
./repro   # ERROR: AddressSanitizer: dynamic-stack-buffer-overflow, WRITE of size 8, opt.c:579
```

Under `plain`, a longer bundle overwrote the parser on `cli_opt_parse`'s stack, and the next
`cli_opt_parser_next` faulted in `strncmp` (`opt.c:473`). A port that keeps the given specs in a
`Vec`, the `alloca` entry's recommended route, has no bound to overflow; batch 5's equivalence
tests of `cli_opt_parse` keep bundles out of the C side, as this batch's do.
