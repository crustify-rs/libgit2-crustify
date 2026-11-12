# Lessons

Rules that hold for this campaign's target alone, each with the evidence it rests on and
its scope, as the `evolution` skill's Lessons section describes. Every run reads them
before its first step.

## Before `git_libgit2_init`, probe libgit2's allocator only through a release

**Rule.** Before `git_libgit2_init`, `git__allocator` is the failing family
(`src/util/allocators/failalloc.c`), and a test may reach it only through a release:
`git_odb_backend_data_free`, which is `git__free`, sets no error. Any allocation then
fails, and `git__malloc`, `git__realloc` and every error setter report through
`git_error_set_oom` or `set_error`, whose thread-state lookup reads TLS key 0 before
`git_error_global_init` has created libgit2's key: it may return another component's
slot and write `&oom_error` into it. The same holds for an allocator installed with
`set_allocator` before initialisation that fails a request.

**Evidence.** `git_odb_backend_data_alloc`'s finding in the ownership store (ASan
unknown-crash in `git_error_set_oom`, `errors.c:170`); `src/util/errors.c`'s
`threadstate_get`; the equivalence test of `wave-176/batch-0`
(`tests/equiv/src/util/allocators/failalloc.rs`), which compares the C release with the
Rust family's and leaves `malloc` and `realloc` uncompared for this reason. The family's
routines called directly, as `git_failalloc_malloc` through its raw binding, set no error
and are outside the rule: `wave-176/batch-1` compares them so in the test process.

**Scope.** The tests of `src/util/allocators/*.c`, `src/util/alloc.c` and
`src/util/errors.c`, and any test that runs libgit2 before its initialisation.
