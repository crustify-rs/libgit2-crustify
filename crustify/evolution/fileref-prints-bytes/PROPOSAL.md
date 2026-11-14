# libc's wrapper writes text, not the bytes C streams carry

Status: `landed`, as glibc-crustify `24da93f` on its `crustify/devel` (its second advance,
`libio/iofwrite.c`): `FileRef::write_all(&self, &[u8]) -> io::Result<()>`, a byte-exact `fwrite`
under the stream's lock, a short write being an error, rather than the `print_c_str` drafted
below. The campaign pins it since `ac1f36519`, and `wave-186/batch-6/run-1` moved the export's
`FileWriter` onto it: the departure, and its documentation, are gone.

## Signal

A route the plan recommends that holds for an entity's state but not for its values: an
entity needed through a wrapper that lacks part of it.

## Evidence

`wave-186/batch-4/run-1` (`git2/src/cli/opt.c`) ported `cli_opt_status_fprint`, which writes
an argument parse's error to the `FILE *` the CLI's C hands it (`stderr`, from `main.c` and
`opt_usage.c`). `_IO_FILE` and `fprintf` take the `crustify-campaign` route, which holds: the
CLI's remaining C writes the same stream, so the export writes through glibc-bindings'
`FileRef` (pinned at `1e60130`), whose only write is `FileRef::print(&str)` (and `write_fmt`),
one locked `fprintf(stream, "%s", text)`.

The message holds the user's argument, `unknown option: <arg>`, and the specs' names, as C
strings, which C writes as their bytes. An argument need not be UTF-8: `argv` holds whatever
the shell passed. `&str` cannot carry it, so the export's `FileWriter` (an `io::Write` over
`FileRef`, in `src/git2/src/cli/opt.rs`) writes such a piece through
`String::from_utf8_lossy`: `--caf\xe9` comes out as `--caf\u{fffd}`, where C writes the byte.
The inline test `ffi_import::equiv_priv::the_export_writes_what_c_writes` asserts both sides of
that departure. The native API, `OptRef::write_status(&mut impl io::Write, …)`, writes the
bytes exactly; only the export departs, and only for bytes that are not UTF-8.

`opt_usage.c` (`cli_opt_usage_fprint`, `cli_opt_help_fprint`) and `progress.c`, which the
plan routes the same way, print the same strings and will meet the same gap.

## Destination

The glibc-crustify side campaign: `fprintf.c`'s wrapper, `FileRef`. As the `dependencies`
skill's Side campaigns section has it, the parent reports the entity its wrapper lacks and the
side campaign wraps it.

## Change

Not made: the side campaign's. A byte-exact write beside `print`, such as
`FileRef::print_c_str(&self, text: &CStr) -> io::Result<usize>`, the same locked
`fprintf(stream, "%s", text)` over a string that is already NUL-terminated, with `print`'s
errors and lock. `FileWriter` then writes each piece through it, with a `CString` of the piece
(which holds no NUL: the pieces are C strings and literals), and its departure goes. It is
needed before `opt.c` switches from C, at its integration.

## Validation

None yet: the change is the side campaign's. The equivalence test above turns into an exact
comparison once the export writes through it.

## Review

None in this campaign: the change was the side campaign's, reviewed there.

## Answer

`wave-186/batch-6/run-1` (the review of the module's `symbol` route) writes each piece of the
export's message through `FileRef::write_all`, the command's bytes included, which the export
had turned to text with `to_string_lossy`. `FileWriter` overrides `io::Write::write_all` with
that one call: the trait's own retries a write that failed with `ErrorKind::Interrupted` from the
start of what is left, so an `EINTR` part way through a piece would have it written twice and
the message reported written, where C's `fprintf` fails (the batch's finding
`equiv/status-export-rewrites-an-interrupted-write`, which the `print`-based writer had too).
Its `write`, which no caller uses, fails whenever the `fwrite` does, though part of the bytes may
be out: the trait documents a failed `write` as writing nothing, which no `io::Write` over
`write_all` can keep, since `write_all` does not report `fwrite`'s count. The inline test
`the_export_writes_what_c_writes` now compares a non-UTF-8 argument and command byte for byte,
and all sixteen of the module's `equiv` tests pass with the exports linked in place of C's
`opt.c`.
