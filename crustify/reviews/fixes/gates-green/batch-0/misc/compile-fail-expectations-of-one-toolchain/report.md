class: nit
evidence: failing trybuild test (`ub_safe`'s `compile_fail`: 9 `mismatch`)
conditions: the sanitizer variants' toolchain, `nightly-2026-10-08` (`asan-ubsan`, `tsan`), against expectations recorded on the pinned stable, 1.100.0; reproduces on `3ed770062`

# Nine `compile_fail` expectations hold on the pinned stable only

**Where.** `tests/ub_safe/compile_fail/`, on `3ed770062`:

- the std-source notes: `checkout_notify_callback_not_send`,
  `checkout_perfdata_callback_not_send`, `checkout_progress_callback_not_send`,
  `progress_callback_not_send`, `diff_parse_options_no_raw_conversion` and
  `odb_options_no_raw_conversion`;
- the closure lifetimes: `checkout_notification_outlives_call`,
  `checkout_perfdata_outlives_call` and `checkout_progress_path_outlives_call`.

**What is wrong.** The plain variant runs the suite on the pinned stable and the sanitizer
variants on the pinned nightly; the expectations were recorded on stable, and nine read
differently on the nightly, so the suite's one test fails under both sanitizer variants. Two
causes:

- The first six reach a bound or an associated function of the standard library
  (`thread::Scope::spawn`'s `Send`, `From::from`). The nightly has `rust-src`, which `-Zbuild-std`
  needs for TSan, so its notes quote the library's source; stable has none, so its notes
  stop at the path. The rejection is the same on both.
- The last three pass `new` a closure that takes its argument at an outer lifetime `'k`.
  Stable reports E0521 (`kept` escapes) and two "implementation of `FnMut` is not general
  enough"; the nightly reports E0277 ("expected an `FnMut(CheckoutNotification<'_>)` closure,
  found ... `fn(CheckoutNotification<'k>)`"). Both reject the closure at `new`'s higher-ranked
  bound, for the reason each case names.

**Reproduce.**

```
crustify/scripts/cargo-variant.sh plain test --test ub_safe -- --exact compile_fail       # ok
crustify/scripts/cargo-variant.sh asan-ubsan test --test ub_safe -- --exact compile_fail  # FAILED, 9 mismatch
```

on `3ed770062`; logs of this run: 198 `ok` plain, 188 `ok` and 9 `mismatch` under ASan+UBSan.

**Fix.** The six reach the bound through a local function in the case itself:
`fn send<T: Send>(_: T) {}` and `send(callback)` in place of the scoped thread, and
`fn from_raw<T: From<sys::git_…>>(raw) -> T` in place of `DiffParseOptions::from(raw)`. Each
proves the same property, `!Send` or no `From` of the raw type, now stated as the trait bound
it is, and reads alike on both toolchains (`rustc` output compared byte for byte against
each variant's built crate, then trybuild).

The three cannot read alike without losing what they prove: a closure annotated `'_` is
higher-ranked and fails in its own body whatever `new` accepts, so it no longer tests `new`.
Each is written once under `compile_fail/toolchain/` and linked from `stable/` and `nightly/`,
which hold each toolchain's full expectation; `compile_fail` checks the directory of the
channel `rustc -vV` reports for the compiler trybuild's cargo runs (`RUSTC`, else `rustc`, as
cargo resolves it). Every expectation is still checked in full on each toolchain. The
directory's `README.md` says how a case is kept toolchain-neutral and how the two sets are
recorded.

**Tiers.** Caught by a wave's end gates under the sanitizer variants, recorded since as one
of "the known nightly failures". The batches that wrote each case ran it on stable alone, as
their gates have them: an escape. That a batch's own `compile_fail` cases are not run on the
nightly that the sanitizer variants use is the gap that let them land.
