# `cli_opt_status_fprint`'s export writes an interrupted piece again, and reports success

- **finding**: `equiv/status-export-rewrites-an-interrupted-write`
- **batch**: `wave-186/batch-6/run-1`, the review of `git2/src/cli/opt.c`'s `symbol` route
  (batches 1, 4 and 5).
- **item**: `fn::cli_opt_status_fprint`: its export in `src/git2/src/cli/opt.rs`'s `ffi_export`,
  which writes `OptRef::write_status`'s pieces through `FileWriter`, an `io::Write` over the C
  stream (`wave-186/batch-4`, `64431c07a`).
- **affected commit**: `17401d831`, the wave's tip; since `64431c07a`.
- **class**: an equivalence defect of an export, which the CLI's remaining C (`main.c`'s and
  `opt_usage.c`'s `cli_opt_status_fprint(stderr, …)`) reaches once the module switches from C.

## Finding

C's `cli_opt_status_fprint` writes its message in a few `fprintf` calls and returns at the first
that fails. `fprintf` does not retry a write that a signal interrupts: when the `write` under it
fails with `EINTR` part way through, `fprintf` returns -1, with `errno` `EINTR`, and the bytes
that went out stay out.

The export wrote each piece with `io::Write::write_all`, which `FileWriter` did not implement: the
trait's own `write_all` calls `write` again, from the start of what is left, whenever it fails
with `io::ErrorKind::Interrupted`, the kind of `EINTR`. `FileWriter::write` was one `fprintf` of
the whole piece (`FileRef::print`), which reports an `EINTR` failure as that kind, so the piece
was written a second time, whole, after the part that had gone out, and the message went on to
its end: the export returned 0 where C returns -1, and the stream held the interrupted piece's
first bytes twice. The task's move onto `FileRef::write_all`, a `write` over it alone, keeps the
defect, since `write_all` reports `EINTR` as the same kind.

## Evidence

`src/main.rs` here compiles C's `opt.c` under other names (`build.rs`, `c_cli_opt_*`) beside the
safe crate's exports, which its export feature links in place of the variant's `opt.c`, and
calls each `cli_opt_status_fprint`, the export through its raw binding as C calls it, on a
`fopencookie` stream, unbuffered, whose first write takes three bytes and fails with `EINTR`.
From the repository root:

```text
CARGO_TARGET_DIR=crustify/tmp/target-repro crustify/scripts/cargo-variant.sh plain run -q \
  --manifest-path crustify/reviews/modules/git2/src/cli/opt.c/wave-186/batch-6/equiv/status-export-rewrites-an-interrupted-write/Cargo.toml
```

On `17401d831` it exits 1 (`run-affected.log`):

```text
C:      returned -1, errno 4, wrote "git"
export: returned 0, errno 4, wrote "gitgit2: unknown option: --nope\n"
the export departs from C
```

## Fix

`FileWriter` writes through `FileRef::write_all` (glibc-crustify `24da93f`, pinned by
`ac1f36519`), one `fwrite` of the bytes as they are, and implements `io::Write::write_all` as that
one call, so that a failure ends the message unretried, as it ends C's. Its `write`, which the
export does not call, fails whenever the `fwrite` fails, though part of the bytes may be out:
`write_all` does not report `fwrite`'s count, so no `Ok(n)` can tell how many, and the type's
documentation says so. The same change answers `crustify/evolution/fileref-prints-bytes`: the
argument, the names and the command are written as their bytes, the command through the private
`OptRef::write_status_bytes`, where the export had passed it through `to_string_lossy`.

The reproducer exits 0 on the fix (`run-fixed.log`): both return -1, with `errno` `EINTR`, having
written `git`. It becomes the inline equivalence test
`ffi_import::equiv_priv::an_interrupted_write_fails_both_and_is_written_once`, which compares
C's `cli_opt_status_fprint` with `write_status_bytes` through `FileWriter`, as the export calls
them, with and without a command. It fails on the old `FileWriter`, and on one whose `write` calls
`FileRef::write_all` without overriding `write_all`, and passes on the fix.

API stability: no public item changes.
