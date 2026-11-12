Filed by the assistant from 1b6fb129c and ab780bd47, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `StrList` and `NullStrList` lack the slice traits ffibox's `CVec` gives a run of owners

- **finding**: `ergonomics/lists-lack-slice-traits`
- **batch**: `wave-184/batch-1/run-1`, the module pass of `libgit2/src/util/strlist.c`, over
  wave 184 batch 0 (`aba03725e`), its six items
- **items**: the batch's five (`fn::git_strlist_copy`, `fn::git_strlist_copy_with_null`,
  `fn::git_strlist_contains_key`, `fn::git_strlist_free`, `fn::git_strlist_free_with_null`);
  which of them this finding is on: not recorded: the report was refused by the host; filed
  from 1b6fb129c and ab780bd47
- **affected commit**: `aba03725e`
- **source commits**: `1b6fb129c` (file), `ab780bd47` (fix)

## Finding

`StrList` and `NullStrList` stand in for a Vec of owned strings, dereferencing to
`&[AllocStr]` / `&mut [AllocStr]` as ffibox's `CVec` of owners does, but lack the traits
`CVec` gives such a run: `AsRef<[T]>`, `AsMut<[T]>`, `IntoIterator` for `&` and `&mut`,
`PartialEq`, `Eq`, `PartialEq<[T]>` and `Hash`.

## Evidence

`a == b` fails with E0369 and `for s in &a` with E0277 (checked against `aba03725e`).
`disclosure.md` sits beside this report.

## Reproducer

None: an ergonomics finding needs none.

## Fix

In `ab780bd47`: `StrList` and `NullStrList` implement `AsRef<[AllocStr]>`,
`AsMut<[AllocStr]>`, `IntoIterator` for `&` and `&mut`, `PartialEq`, `Eq`,
`PartialEq<[AllocStr]>` and `Hash`, element-wise as ffibox's `CVec` does (unit test
`lists_compare_and_iterate_as_their_strings`).

API stability (ffimap api): 4 kept, 2 extended (the copies' lists gain the traits).
