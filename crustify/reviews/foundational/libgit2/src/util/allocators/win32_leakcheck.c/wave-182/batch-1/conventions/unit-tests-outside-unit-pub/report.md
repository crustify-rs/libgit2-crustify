Filed by the assistant from d2b4d3e8b and d2b4d3e8b, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# The module's public-interface unit tests sit in an inline `mod unit_tests`

- **finding**: `conventions/unit-tests-outside-unit-pub` (the commit names it
  "conventions (unit-tests-outside-unit-pub)")
- **batch**: `wave-182/batch-1/run-1`, the module pass of
  `libgit2/src/util/allocators/win32_leakcheck.c`, reviewing batch 0 (`cff595a09`) for every
  objective
- **item**: `fn::git_win32_leakcheck_init_allocator`
- **affected commit**: `cff595a09`
- **source commits**: `d2b4d3e8b` (the batch has one commit, which carries the finding and
  its fix)

## Finding

Batch 0 put the module's four unit tests in an inline `#[cfg(test)] mod unit_tests` in
ffi_import. The conventions' Testing section puts tests of the public interface in
tests/unit_pub and tests of internal routines in an inline `mod unit_priv`. Three of the
four (`this_build_has_no_leakcheck_allocator`, `the_stubs_error_code_class_is_corrected`,
`each_call_replaces_the_threads_error`) check only the public interface. The raw half of the
second repeats tests/equiv's `c_step`, except that the stub leaves its argument unwritten,
which `c_step` computed but never asserted.

## Evidence

Rerun on `cff595a09`:

```
sed -n 113,190p src/libgit2/src/util/allocators/win32_leakcheck.rs
ls tests/
```

(no unit_pub).

## Reproducer

None: a conventions finding needs no reproducer.

## Fix

In `d2b4d3e8b`: tests/unit_pub is added with wave 176's main.rs, src.rs and src/util.rs byte
for byte, and Cargo.toml gets the same `[[test]]` unit_pub entry as waves 169 and 176. The
three tests move there, using the public API alone. The inline module becomes `unit_priv`
and keeps `other_errors_pass_through_unchanged`, the test of the private `reported_error`.
`c_step` now asserts that the failing stub leaves the value unwritten.

crustify/LESSONS.md gains the rule that the tree's 34 legacy inline `mod unit_tests` modules
are no precedent for test placement.
