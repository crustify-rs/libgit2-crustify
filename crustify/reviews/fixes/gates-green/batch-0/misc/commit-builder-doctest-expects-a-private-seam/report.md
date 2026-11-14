class: nit
evidence: failing doctest (rustdoc on the pinned nightly: `Some expected error codes were not found: ["E0624"]`)
conditions: the sanitizer variants' toolchain, `nightly-2026-10-08` (`asan-ubsan`, `tsan`); the pinned stable, 1.100.0, does not check a doctest's error code; reproduces on `3ed770062`

# `CommitBuilder`'s compile-fail doctest expects a private seam that is public and unsafe

**Where.** `src/libgit2/src/libgit2/commit.rs`, the first doctest on `CommitBuilder` (line
1433 on `3ed770062`), and the sentence before it: "there is no public way to make one".

**What is wrong.** The doctest calls `CommitBuilderMut::from_ptr` inside `unsafe` and
expects E0624 (a private associated function). No such private function exists:
`ffibox::define_ctype!` implements `ffibox::CMutSeam` for every exclusive handle, a trait
documented as "the raw seam of an exclusive handle, callable from any crate", whose unsafe
`from_ptr` is the only `from_ptr` there is. The snippet fails only because it does not import
the trait (E0599), so it proves nothing about the type; with `use ffibox::CMutSeam;` it
compiles. The prose claims more than holds: there is a public way, the unsafe seam; what
holds is that safe code has none, which `tests/ub_safe/compile_fail/commit_builder_handles.rs`
already shows (E0133, with the trait in scope).

**Reproduce.**

```
crustify/scripts/cargo-variant.sh asan-ubsan test --doc -p libgit2-crustify -- CommitBuilder
```

on `3ed770062`: `CommitBuilder (line 1433) - compile fail ... FAILED`, rustdoc reporting
`error[E0599]: no associated function or constant named from_ptr found for struct
CommitBuilderMut<'a>` and `help: trait CMutSeam which provides from_ptr is implemented but
not in scope`. The same under `tsan`. `plain` passes it, since stable rustdoc checks only that
the snippet fails.

**Fix.** The sentence says what holds, "safe code has no way to make one, since the
handle's raw seam is unsafe", and the doctest imports `ffibox::CMutSeam` and calls
`from_ptr` outside `unsafe`, expecting E0133. Both toolchains report E0133, and the same
snippet with the call in `unsafe { }` compiles on both (checked by `rustc` against each
variant's built crate), so E0133 is the one thing rejecting it.

**Tiers.** Caught by a wave's end gates under the sanitizer variants, recorded since as one
of "the known nightly failures". The batch that wrote it and every review since had it in
scope and passed it on stable, which does not check the code
(`crustify/evolution/compile-fail-doctest-codes-unchecked-on-stable`): an escape.
