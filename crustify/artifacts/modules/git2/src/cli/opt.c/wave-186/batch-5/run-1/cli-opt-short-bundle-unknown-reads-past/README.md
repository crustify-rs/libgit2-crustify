# `parse_short` reads the argument after an unknown bundled option past its end

Found by `wave-186/batch-5/run-1` while porting `parse_short`.

An unknown option at the start of a bundle (`-xd`) ends it: `parse_short` returns
`CLI_OPT_STATUS_UNKNOWN_OPTION` with `in_short` still 0, and the parser moves on. One later in a
bundle (`-dx`, `d` known) returns from the `spec == NULL` branch with `parser->idx` advanced but
`parser->in_short` left at the bundle's offset, so `cli_opt_parser_next` takes the next argument
for the rest of a bundle and reads it from `&arg[1 + in_short]`: from the wrong character, and
past its end when it is shorter. `cli_opt_parse` returns at the unknown option and `main.c` stops
at it, so the CLI's own callers never parse on; `cli_opt_foreach`, whose callback decides, does.

Run with the include path of `opt.c` in the variant's `build.ninja`, from the batch's worktree:

```bash
INC="-Ic/src/util -Ic/src/cli -Ic/include -Icrustify/builds/plain/gen_headers"
clang -g -O0 -fsanitize=address -D_GNU_SOURCE -DGIT_DEPRECATE_HARD -DNDEBUG $INC \
    main.c c/src/cli/opt.c -o repro
./repro   # ERROR: AddressSanitizer: heap-buffer-overflow, READ of size 1, opt.c:109 (spec_for_short)
```

The port (`take_short` in `src/git2/src/cli/opt.rs`) ends the bundle at an unknown option, as C
does for one at its start: `in_short` is reset and the next argument parses from its start, kept
by `unit_pub`'s `an_unknown_option_ends_its_bundle`. The equivalence tests give the C no unknown
option after a bundle's first.
