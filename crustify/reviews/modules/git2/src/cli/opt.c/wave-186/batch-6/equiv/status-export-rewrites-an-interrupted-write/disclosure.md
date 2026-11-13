For: libgit2-crustify's issue tracker (https://github.com/crustify-rs/libgit2-crustify/issues),
as a public issue. The defect is in the crate's own Rust export, not in libgit2, and has no
security impact we know of: a status message written twice to standard error.

Title: `cli_opt_status_fprint`'s export writes a piece again after `EINTR`, and reports success

### Summary

The Rust export of the CLI's `cli_opt_status_fprint` (`src/git2/src/cli/opt.rs`, under the
`ffi-export-src-cli-opt-c` feature) writes the message's pieces with `io::Write::write_all` over
a C stream. Its writer left `write_all` to the trait's default, which retries a write that fails
with `ErrorKind::Interrupted`:

```rust
impl io::Write for FileWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        /* one fprintf(stream, "%s", text) of the whole piece */
    }
}
```

When the `write(2)` under a piece fails with `EINTR` after part of it went out, C's `fprintf`
returns -1 and `cli_opt_status_fprint` returns that failure; the export writes the whole piece
again, finishes the message and returns 0.

### Reproduction

On a `fopencookie` stream, unbuffered, whose first write takes three bytes and fails with
`EINTR`, for an unknown option `--nope` and the command `git2`:

```text
C:      returned -1, errno 4, wrote "git"
export: returned 0, errno 4, wrote "gitgit2: unknown option: --nope\n"
```

The reproducer, a small Cargo package that runs both, is available on request.

### Fix

Fixed in `wave-186/batch-6/run-1`: the writer writes through glibc-bindings'
`FileRef::write_all`, one `fwrite` per piece, and implements `write_all` as that one call, so
that a failure ends the message as it ends C's. A patch is available.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
