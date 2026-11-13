# An `io::Write` over the libc wrapper's `write_all` must implement `write_all` itself

Status: `open`. Filed by the assistant on 2026-11-13 for `wave-186/batch-6`, which noticed it
after its two commits had landed.

## Signal

A review finding that a tool's documentation led to (`evolution` §2: an agent prompt, here a
wrapper's docs, that leads its reader wrong).

## Evidence

- `wave-186/batch-6/run-1` (`961150dd9`),
  `crustify/reviews/modules/git2/src/cli/opt.c/wave-186/batch-6/equiv/status-export-rewrites-an-interrupted-write`:
  the parent's `FileWriter`, an `io::Write` over a `FileRef`, used the trait's default
  `write_all`, which retries a write interrupted by `EINTR` from the start of what is left.
  The export wrote the interrupted piece twice and returned 0, where C wrote nothing more and
  returned -1.
- glibc-crustify's `src/libc/libio/iofwrite.rs` (`d796900`) presents `FileRef::write_all` as
  "the write an `io::Write` over a C stream makes": it does not report how many bytes `fwrite`
  wrote, so an `io::Write::write` built on it cannot honour "a failed write wrote nothing", and
  the default `write_all` over that `write` retries.

## Destination

A campaign artifact another campaign owns: glibc-crustify's `iofwrite.rs`, at its next
advance. Either its docs say that an `io::Write` over `write_all` must implement `write_all`
itself, or the wrapper reports `fwrite`'s count.

## Change

Not made.

## Validation

None yet.

## Review

None yet.
