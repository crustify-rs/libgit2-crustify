# A compile-fail doctest's error code is checked only on nightly

Status: `open`: no change made. The destination is the `testing` skill, which the binder
does not evolve.

## Signal

A check the gates appear to run and do not. A wrong expectation passed a batch's gates.

## Evidence

`wave-186/batch-2/run-1` added seven `compile_fail` doctests to `src/git2/src/cli/opt.rs`.
One was first written `compile_fail,E0499`, for assigning to an argument array a `Parser`
borrows. The plain gate, on the pinned stable (Rust 1.100.0), passed it. Rewritten as
`compile_fail,E0000`, it passed again: stable rustdoc checks only that the snippet fails to
compile, never which error it fails with. Compiled with `rustc` against the built rlib, the
snippet reports `E0506` (cannot assign to `args[_]` because it is borrowed), which the batch
now expects. The codes of all seven were confirmed that way. Only a nightly rustdoc checks the
code, and the campaign runs nightly only in its sanitizer and Miri variants (Miri skips
doctests). There, the known `CommitBuilder` doctest (`src/libgit2/src/libgit2/commit.rs`,
`E0624`) is the one recorded failure. So a wrong code a batch writes is caught, if at all, at a
later gate, as a known nightly failure.

## Destination

The spec: `testing`'s UB tests section, beside "When the type system should reject a
program, the test is a compile-fail doctest or a `trybuild` case". It could say that a
doctest's error code is verified on the pinned nightly (`cargo +<nightly> test --doc`), or
each snippet compiled alone, since the stable gate does not check it. A `trybuild` case's
`.stderr` is checked on stable and needs no such step.

## Change

Not made: `testing` is not the binder's to evolve.

## Validation

The two stable runs above: `E0499` and `E0000` both passed `cargo test --doc` on 1.100.0, and
`rustc` reported `E0506`.

## Review

None yet.
