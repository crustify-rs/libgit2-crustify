This run is not a review batch: it reviews one change to ffimap, a tool, as Crustify's
`evolution` skill's Review section describes, and changes, commits and lands nothing. Its
task is `/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-sys-inventory-in-crate/review-1/task.json`.
You run under ffimap's checkout, `/crustify-rs/ffimap`, which is on `devel` (`9aa2ef1`); the
campaign's working branch of ffimap is `libgit2-crustify/devel` (`c3203c7`).

- The proposal, with its evidence, validation and drafted row: `/target/libgit2-crustify/crustify/evolution/ffimap-sys-inventory-in-crate/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/ffimap diff c3203c7 8cd7a26`.

It changes the driver, `src/ffimap/driver/src/main.rs` (`sys_crates`, new `sys_modules` and
`in_sys_module`, and `count_sys_types`, whose match arms move into a closure unchanged), the
CLI's help, three documents, the `cells` fixture and `tests/test_cells.py`. Read ffimap's
`docs/goals.md` first: G1, G7, G8, G9, G10 and G13 above all.

Run the suite on a `git archive 8cd7a26` copy that is NOT under a Cargo workspace (a copy under
this campaign's `crustify/tmp/` makes the driver fail to build and the suite skip 75 tests:
`ffimap-driver-builds-under-a-workspace`; use a directory under `/tmp` or add an empty
`[workspace]` table to the copy's driver manifest), and show the new test fails on the old
driver. Judge above all:
- whether the refactor of `count_sys_types` changes anything for a named crate (the 21 other
  driver tests, and glibc-crustify, `--ffibox --sys-crates libc_sys`, should give what the old
  driver gives);
- whether `in_sys_module` takes exactly the module's bindings: items of nested modules, an
  item of an `extern` block, an `impl`'s methods, a re-export, and whether `def_path_str` is
  a sound way to match the module;
- whether the numbers on libgit2-crustify are believable: the scans are
  `/target/libgit2-crustify/crustify/tmp/assistant/ffimap-scan/scan.json` and `scan-new.json`,
  and the bindings `/target/libgit2-crustify/sys/*/bindings.rs`, included by `src/sys.rs`.
  You need not rescan libgit2, which takes a campaign build;
- whether any other counter should read the module and does not: the peel set already holds
  every local type.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, or contradict another rule. The rest is listed
apart, as not required. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Remove your scratch before you end.
