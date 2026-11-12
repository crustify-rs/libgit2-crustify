# `CClosure::failed()` reads the panic slot an invocation holds mutably

Status: `landed`, as ffibox `a2cef17` on `devel`: `failed()` reads an atomic `panicked` flag, set after the slot's one store, and `invoke_scoped` holds no reference to the slot across the closure's call. The fix also closes a data race the defect hid: a `Send` closure invoked on a C worker thread while another callback asks `failed()`. Both cases are tests that fail under Miri on the old code and pass under Stacked and Tree Borrows.

## Signal

A tool that reports what its source contradicts: a safe method of ffibox whose `SAFETY`
comment assumes a state safe code can break.

## Evidence

Found by the ffibox docs audit of 2026-11-12, at ffibox `ca1ff67` (`devel`):

- `CClosure::failed()` is safe and takes `&self`. It reads the panic slot through
  `panicked()` (`src/callback.rs:268–274`), whose `SAFETY` comment assumes no invocation is
  running. `Debug` (`:308`) calls it too.
- `invoke_scoped` (`:333`) holds a `&mut` to that slot across the closure's call (`:340`)
  and writes it at `:343` when the closure panics.
- A closure that reaches its own `CClosure` (leaked to `'static`, through a thread-local
  `&dyn Fn`) calls `failed()` mid-invocation, then panics. `cargo +nightly miri run`
  reports undefined behaviour at `callback.rs:343` under Stacked Borrows; it passes under
  Tree Borrows. Reproducer: the session scratchpad's `closure-reentry/src/main.rs`, safe
  code but for the one C call.

No campaign code calls `failed()`; `basics.rs`'s one `CClosure` is in a test.

## Destination

ffibox: `src/callback.rs`, with the reproducer as a Miri test.

## Change

Do not hold the `&mut` to the slot across the call: take it only after the call returns, to
store the payload; or have `failed()` refuse while an invocation runs.

## Validation

ffibox's suite, and the reproducer as a test passing under Miri with Stacked Borrows.
