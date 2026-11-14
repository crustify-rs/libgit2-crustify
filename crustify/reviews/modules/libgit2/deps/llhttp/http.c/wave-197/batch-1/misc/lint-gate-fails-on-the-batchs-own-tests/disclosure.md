To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: http.c's port landed tests that fail the lint gate

Hi Marius,

This is a test-suite defect, with no safety impact, so it can be handled in public.

The tree `deps/llhttp/http.c`'s port landed on crustify/waves/wave-197 (`a81dc3103`) fails
`clippy --workspace --all-targets` with warnings denied, the gate every landing passes, in
two of the batch's own test files, though its notes record the gate as passing:

```text
error: this function has too many lines (102/100)
   --> tests/unit_pub/deps/llhttp/http.rs:104:1
error: very complex type used. Consider factoring parts into `type` definitions
   --> tests/unit_pub/deps/llhttp/http.rs:106:16
error: this `unsafe` block contains 2 unsafe operations, expected only one
  --> tests/equiv/deps/llhttp/http.rs:65:9
```

The first two are pedantic lints the workspace denies through `-D warnings`; the third,
`multiple_unsafe_ops_per_block`, is denied outright: one block both read and wrote the raw
parser. The command, run from the commit's tree, is
`crustify/scripts/cargo-variant.sh plain clippy --workspace --all-targets -- -D warnings`,
and gives the same three errors with `--features ffi-export-deps-llhttp-http-c`.

The fix names the test's case tuple with an alias and moves its loop into a helper shared
with a new test, and splits the equivalence fixture's block into a read and a write, each
justified. The tests assert what they did, and the workspace now lints clean with the export
feature off and on. It is on crustify/waves/wave-197, with the report under
`crustify/reviews/modules/libgit2/deps/llhttp/http.c/wave-197/batch-1/misc/`. I can send the
exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
