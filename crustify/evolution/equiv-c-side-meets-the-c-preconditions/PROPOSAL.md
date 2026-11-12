# The C side of an equivalence test meets the C's preconditions

Status: `open`

## Signal

A review finding that a gate's sanitizer caught in a test suite rather than in the code it
tests, and that the `testing` skill gives no rule against.

## Evidence

- `wave-169/batch-5` (the module pass of `deps/reftable/basics.c`), finding
  `contracts/ffi_import/equiv-c-side-races-the-first-init`, reproducer committed under
  `crustify/reviews/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-5/`.
- The wave's TSan gate on its tip `9aa6520a5` reported a data race in
  `tests/equiv/deps/reftable/basics.rs`: the suite's C side called `reftable_buf_add`, which
  reads reftable's allocator hooks, before anything pinned libgit2's runtime, while another
  test's first `git_libgit2_init` wrote the hooks (`git_reftable_global_init` ->
  `reftable_set_alloc`). The safe API it compares against pins the runtime before every
  such read; the C side skipped that precondition, and TSan halted the suite, so the rest of
  `equiv` never ran under TSan.

## Destination

The spec: `skills/verification/testing/SKILL.md`, its Equivalence tests section.

## Change

After "They use multi-call sequences and compare after each step; single calls are only a
baseline.", add:

> The C side of a test meets the preconditions the C sets its callers, as the wrapper meets
> them before its own calls: a library's global initialisation, which writes state its
> routines read without synchronisation, runs before the test's first call that reads it,
> since the harness runs other tests beside it on other threads.

## Validation

None run: a judgement. The batch's fix pins the runtime before the suite's first raw call,
after which the whole `equiv` suite runs clean under TSan.
