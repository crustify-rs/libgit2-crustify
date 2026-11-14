class: nit
evidence: test timeout under BorrowSanitizer (`aliasing-model only` instrument; no report, the tests never finish)
conditions: the `bsan` variant (`bsan 0.1.0`, debug test builds, `BSAN_OPTIONS=gc_hang_threshold_ms=120000`), `cfg(bsan)`; reproduces on `3ed770062`

# The `indexer.c` tests' fixture is built a byte at a time, which BorrowSanitizer cannot finish

**Where.** The fixture of four tests, each skipped by name at every module's end since
`failalloc.c`'s integration (dashboard Notes), so that BSan checked none of their paths:

- `src/libgit2/src/libgit2/indexer.rs`, `unit_tests::blobs()`, used by
  `libgit2_calls_the_closure_with_its_progress`, `a_break_cancels_the_indexing_with_git_euser`
  and `a_panic_cancels_then_resumes_after_the_walk`;
- `tests/equiv/src/libgit2/indexer.rs`, `git_pack()`, used by
  `a_progress_callback_sees_and_cancels_as_a_c_callback`.

**What is wrong.** Each builds its base blob, 4096 lines of `line <i>\n` (39,850 bytes), with
`(0..4096).flat_map(|i| format!(…).into_bytes()).collect()`, which pushes it into one `Vec` a
byte at a time. BorrowSanitizer's cost per access to one allocation grows with the borrows of
it already made (`crustify/evolution/bsan-borrow-history-grows`), so the build's cost grows
faster than quadratically. The code under test, libgit2's indexer, pack, zstream and SHA-1,
all still C, is not what stalls: no pack file is open and no `git` child runs while a test
hangs. A probe test, run alone under BSan, timed each part (logs `probe*.log` of this run):

| step | input | BSan |
|---|---|---:|
| base pushed a byte at a time | 128 lines, 1,042 B | 1.8 s |
| | 256 lines, 2,194 B | 9.5 s |
| | 512 lines, 4,498 B | 45.1 s |
| base collected line by line | 4096 lines, 39,850 B | 3.3 s |
| `git pack-objects` | 4096 lines | 0.63 s |
| indexing, 4096-byte pieces (3 appends) | 9,630-byte pack | 0.02 s |
| indexing, 97-byte pieces (100 appends) | 9,630-byte pack | 0.55 s |

At that growth, the 39,850 bytes pushed a byte at a time take hours, in each of the four
tests.

**Reproduce.**

```
RUST_TEST_THREADS=1 timeout 1500 crustify/scripts/cargo-variant.sh bsan test --lib -- --exact \
  libgit2::src::libgit2::indexer::unit_tests::a_break_cancels_the_indexing_with_git_euser
```

on `3ed770062`, and likewise for the other two `lib` tests and for `--test equiv`'s
`src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`: in this run each
was killed by its `timeout` at 1,500 s (`rc=124`), never past its `test … ` line.

**Fix.** Both fixtures push the base a line at a time, `base.push_str(&format!(…))` for
each of the 4096 lines into one `String`, and give the same bytes. How the lines are joined
matters: `collect::<String>()`, which pushes the same way, built it in 3.3 s, but
`collect::<Vec<String>>().concat()`, which clippy's `format_collect` would suggest over it, ran
each of the four tests past a 600 s `timeout`. `concat` copies each piece through a
`split_at_mut` of what remains of the buffer, which likely nests each borrow in the last
rather than deriving it from the buffer; that cause was not confirmed. Nothing is sized: the inputs are unchanged in every build, BSan included, so BSan
now runs each test whole, every path of the code it tests reached as in the other variants.
Alone under BSan, by exact name and under `timeout 600`, each passes:
`libgit2_calls_the_closure_with_its_progress` in 5.37 s,
`a_break_cancels_the_indexing_with_git_euser` in 4.83 s,
`a_panic_cancels_then_resumes_after_the_walk` in 4.81 s and `equiv`'s
`a_progress_callback_sees_and_cancels_as_a_c_callback` in 10.55 s (libtest's times; logs
`bsan-final-*.log`). The push carries `#[expect(clippy::format_push_string)]`, since the
`write!` the lint asks for pushes each line in pieces. The four names are
off `crustify/scripts/slow-list.txt`.

**Tiers.** Caught by the module ends' BSan gate, which skipped the four by name from then on.
The batches that wrote them run BSan only on what they expect to trigger it, so the module
end was the first BSan run of either fixture: no escape.
