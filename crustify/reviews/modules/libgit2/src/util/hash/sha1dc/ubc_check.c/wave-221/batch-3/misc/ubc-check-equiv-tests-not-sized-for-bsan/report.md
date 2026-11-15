class: nit
evidence: the wave's BSan gate timed out (`end-gates`, `bsan-equiv: timed out, 3600.0 s`), the three tests below the only ones of the binary that never finished
conditions: the `bsan` variant, `cfg(bsan)`; `tests/equiv/src/util/hash/sha1dc/ubc_check.rs` at `e816c88bb`, the tip of `crustify/waves/wave-221`

# `ubc_check.c`'s equivalence tests do not finish under BSan, and fail the wave's gate

**Where.** `tests/equiv/src/util/hash/sha1dc/ubc_check.rs` on `e816c88bb`, the tests
`wave-221/batch-1` added:

- `ubc_check_matches_c_on_seeded_blocks`: the C and the Rust over 2,097,152 seeded blocks;
- `ubc_check_matches_c_on_each_one_bit_change_of_a_selecting_block`: a search over about
  46,000 seeded blocks for one selecting each vector, then each witness's 2,560 one-bit
  changes, 81,920 blocks;
- `ubc_check_matches_c_on_shattered_1_pdf`: all 6,600 blocks of the file.

**What is wrong.** `testing-gates`' Regressions section runs BSan over the workspace at a wave's
and a module's end, one invocation per test binary under a `timeout`, and has a test BSan
cannot finish within it "sized for BSan: in the configuration BSan's build sets, it runs fewer
inputs, chosen to keep reaching every path it checks". These three run the same inputs under
every build. The wave's end gate on the tip, `end-gates` with the slow list skipped and
`--timeout 3600`, timed the `equiv` binary out after an hour: of the 460 tests that started, the
three above alone had not finished, each reported "running for over 60 seconds" and none with a
result. Every other stage passed, as every end gate had at `fa1c04754`, the tip before the
module's base, so this is the module's.

Nothing a consumer can observe; hence `nit`, with no reproducer beyond the gate.

**Check.**

```
crustify/scripts/cargo-variant.sh bsan test --test equiv -- --exact \
  src::util::hash::sha1dc::ubc_check::ubc_check_matches_c_on_seeded_blocks
```

does not finish within an hour at `e816c88bb` (the gate's log: `crustify/tmp/logs/tip/bsan-equiv.log`
in this batch's worktree, not kept).

**Fix.** Under `cfg(bsan)` the tests take fewer inputs, as other modules' `equiv` tests do
(`tests/equiv/src/libgit2/oid.rs`, `path.rs`): the edge blocks and 1,024 seeded ones, whose
union and count assertions apply only to the full run; the one-bit changes of the first
selecting block, in words 35 to 64, which the conditions read; and the first eight blocks of
`shattered-1.pdf`, the near-collision pair among them. Each call runs every condition of the C
and the Rust, so the sized tests reach every path but the guarded clears of the vectors they do
not select. Each run alone under BSan, by exact name: `0.65 s`, `11.13 s` and `0.08 s` of test
time (28 s, 38 s and 22 s with cargo), passing. Every other build runs them whole; on `plain` they
pass as before. These tests retire with the C at the module's switch; the `unit_pub` tests this
batch adds keep what they establish, and are sized for BSan and Miri the same way.

**Tiers.** Caught by this module pass's wave gates (`wave-221/batch-3`). `wave-221/batch-1`'s
own gates had the tests in scope, and ran no BSan on them, as a batch runs it only on the tests
it expects to trigger it.
