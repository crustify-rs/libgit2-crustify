# Which of C's arguments a gateway refuses, and which it trusts

Status: `open`

## Signal

A rule a review had to work out by hand that a skill should state. `reviewing`'s Contracts
section (Crustify `57f14ff`) says an obligation stated on code C calls once a value is
handed to it is no contract, and that "what the obligation excludes is refused in Rust, in
front of the C, instead". It does not say what happens to an obligation Rust cannot check
at all: that a pointer C passes is live, of the right family, or a NUL-terminated string.
Read literally, every callback trampoline's reliance on C's pointer arguments would be a
finding no fix can close.

## Evidence

`foundational/libgit2/src/util/alloc.c/wave-177/batch-1/run-1`, the module pass of
`src/util/alloc.c`. The trampolines `Allocator::new` stores rely on three obligations of
`MallocFn` / `ReallocFn` / `FreeFn`: `file` is NULL or a C string, `ptr` is a live
allocation of the installed family, and `size` is non-zero when `ptr` is not NULL. The run
filed the third (`contracts/ffi_import/realloc-trampoline-leaves-zero-size-to-c`, fixed
by a guard in the trampoline: NULL, the block left live) and judged the other two no
finding: the first is what C's declaration (`const char *file`, libgit2's `__FILE__`)
states, and the second is vouched for by the `unsafe` installer (`set_allocator`'s
contract), which a Rust caller can keep. Nothing in the skill said so.

## Destination and change

The spec, `reviewing`'s Contracts section: an obligation on C's arguments is refused in
Rust where Rust can test it (a value, a range, a null, a length), and otherwise kept by
whoever hands the value to C: an `unsafe` installer whose Rust caller vouches for it, or
the C declaration's own contract, which a gateway trusts as an export does. Only an
obligation neither covers, and Rust can test, is a finding.

## Validation

By the evidence above; to validate where in doubt, rerun the module pass's contracts
review of `alloc.rs` with and without the clause and compare the findings.
