# ffibox has no C half for a function without context that Rust calls on other threads

Status: `open`.

## Signal

A representation ffibox cannot express, hand-written in a campaign: the C half of a callback
whose C type has no context argument, which Rust calls later, on a thread other than the one
that received it. ffibox's own source waits for this need: `CFn`'s comment
`TODO(CFn threads): a C function that the C API lets Rust call from other threads, once a
campaign needs it` (`src/callback.rs`, at ffibox `568bcb5`).

## Evidence

`wave-226/batch-3/run-1`, the port of `src/util/thread.c`'s thread-local storage (libgit2).
`git_tlsdata_init(git_tlsdata_key *key, void (*destroy_fn)(void *))` hands the port a
destructor, which the port's registry keeps with the key and calls on each thread's value at
that thread's exit, on that thread. The C type, `void (*)(void *)`, has no context argument:
its one argument is the value, which the destructor frees (`errors.c`'s `threadstate_free`,
`oid.c`'s `thread_str_free`). `CFn<'a, P, CTX>` takes a context and is neither `Send` nor
`Sync`, so the registry, a `static Mutex`, cannot hold one, nor can a thread other than the
one that built it call it. The batch hand-wrote the C half in `thread.rs`'s `ffi_import`:
`CDestructor(unsafe extern "C" fn(*mut c_void))`, `Copy`, with an `unsafe fn new` whose
contract states which values the function may be called with, on which threads and for how
long, and a safe `destroy` called only with a value C stored under the same key.

## Goal

ffibox's G5 (foreign patterns as Rust patterns) and G12 (unsafe encapsulated, obligations
local): its fitness measure, the hand-written representations campaigns keep beside
ffibox's primitives, falls by one per such callback.

## Destination

ffibox: `src/callback.rs` and `docs/outbound.md`'s "C hands Rust a callback".

## Change

A `CFn` without context, or `CFn`'s `TODO(CFn threads)` resolved: a C function of type `P`
with no context argument, `Copy`, `Send` and `Sync`, built through an `unsafe` constructor
whose contract names the threads and the time the function may be called for, and called
through an `unsafe fn call` as `CFn::call` is. Where the function's argument is a value it
frees, as here, the caller pairs the value with it, as binding-symbols' rule on a context
reached through an argument has it, so the call stays `unsafe` or behind the binding's own
pairing.

## Validation

Not made. Its validation: ffibox's suite and a test that calls one from a thread other than
the one that built it, under Miri and TSan; then `thread.rs`'s `CDestructor` rewritten over
it, its tests unchanged.

## Row

| a context-free C callback Rust calls on other threads is held by ffibox, not hand-written | `libgit2-crustify`, `ffibox-cfn-without-context-on-any-thread` | ffibox `<commit>` | `thread.c`'s TLS port hand-wrote `CDestructor`, since `CFn` takes a context and is neither `Send` nor `Sync` | a batch that hand-writes the C half of a callback ffibox's callback types could hold | watched |

## Review

Not reviewed yet.
