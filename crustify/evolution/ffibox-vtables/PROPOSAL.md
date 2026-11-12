# Function-pointer tables a library calls into, implemented by a Rust trait

Status: `open`, proposed after the ffibox usage audit of 2026-11-12. The user approved
filing it; it awaits their decision on the design at a module's end. It answers the TODO
in the `ffibox` tool skill's Binding symbols section: "ffibox has no primitive yet for a
struct of function pointers a library calls into, bound to a trait the Rust side
implements: a binding hand-writes it until it does".

## Signal

One representation hand-written in several places, and a TODO met again and again
(`evolution` §2).

## Evidence

Every C struct of function pointers that libgit2 calls into, and that the Rust side fills,
is hand-written. Counts of `extern "C" fn` per file at `fd0120c5e` include each table's
trampolines:

| table | file | `extern "C" fn` | Rust trait |
|---|---|---|---|
| `git_config_backend`, `git_config_iterator`, `git_config_backend_entry` | `src/libgit2/src/libgit2/config.rs` | 36 | `CustomBackend` (:4534) |
| `git_odb_backend`, `git_odb_stream`, `git_odb_writepack` | `src/libgit2/src/libgit2/odb.rs` | 23 | — |
| `git_allocator` | `src/libgit2/src/util/alloc.rs` | 10 | `alloc::Alloc` (:448) |
| reftable's allocator hooks | `src/libgit2/deps/reftable/basics.rs` | 3 | `basics::Alloc` (:1551) |
| `git_writestream` | `src/libgit2/src/libgit2/filter.rs` | 3 | — |

Each hand-writes the same parts:
- a `#[repr(C)]` struct with the C table at offset zero followed by the Rust state;
- one `unsafe extern "C" fn` per slot, which casts the table pointer back to the enclosing
  struct and calls the trait method behind the panic barrier;
- the `free` slot that reclaims the `Box`; and
- an adoption into an owner.

Every part carries its own SAFETY argument. The hand-writing has already produced defects:
- the two allocator modules disagreed on the panic policy (`basics.rs` aborted where
  `alloc.rs` returned NULL; fixed in `fixes/ffibox-audit`);
- `config::BackendFree` panicked on a NULL `free` slot where `odb::OdbBackendFree` skips it
  (same run);
- `config.rs`'s `Dispatch` state (:4701) guards re-entry by hand.

Ported modules stay on this path: the port campaign's `ffi_export` region still hands C
backends, streams and allocators that Rust implements.

## Destination

ffibox: a new module for C tables implemented in Rust. Its rustdoc, README and lib.rs rows,
a section in `docs/hand-written.md` or `docs/ownership.md`, and the `ffibox` tool skill's
Binding symbols TODO, which is replaced with the rule.

## Change

A primitive declared once per C table:

```rust
ffibox::vtable! {
    /// libgit2's config backend, implemented by `B`.
    pub struct BackendTable<B: CustomBackend>: ffi::git_config_backend {
        version = ffi::GIT_CONFIG_BACKEND_VERSION,
        get     => fn get(&self, key: &CStr) -> Result<Entry, Error>    on panic GIT_EUSER,
        set     => fn set(&mut self, key: &CStr, value: &CStr) -> …     on panic GIT_EUSER,
        free    => drop,
    }
}
```

The primitive:
- lays out `#[repr(C)] { table, state: B }` with the table at offset zero, checked at
  compile time;
- generates one trampoline per slot, recovering `&B` or `&mut B` from the table pointer
  under the aliasing rule the slot declares;
- puts every trampoline behind `callback::barrier` with the slot's declared panic result;
- makes `free` reclaim the box; and
- hands back an owner (`CBox<Table, TableFree>`) for adoption.

The status conversions stay the binding's, through the slot signatures it writes. ffibox
supplies only the layout, recovery, barrier and release. Re-entry, where C calls back into
the same backend during a call, is declared per table: shared `&self` slots, or a guard
that ffibox supplies and that returns the declared error.

Before the macro is designed, a design note should settle two questions:
- whether the primitive is a macro or a generic `CTable<T: Table>` with an `unsafe` trait
  per C table;
- how slots that hand out owned C memory (`git_config_backend_entry`, iterators) compose
  with the existing owners.

## Validation

ffibox's suite and Miri, over a toy table in its tests, checking that:
- a panicking slot yields its declared result;
- `free` reclaims once;
- re-entry follows the declared rule; and
- a table with the state at the wrong offset is rejected at compile time.

Then port one campaign table to it, `basics.rs`'s reftable hooks as the smallest, with its
tests unchanged. Then compare the scan (`ffimap`) of the module before and after:
`code.unsafe_block` and `fn.unsafe.priv` should fall.

## Reviews

Not landed yet: on ffibox's `libgit2-crustify/evolution/ffibox-vtables`.

- First change (`d0f612e`, `23e0a9b`, `454172f`): `CTable<T, S>` and `impl_cvtable!`, its
  validation ported `filter.rs`'s `git_writestream`, since reftable's allocator hooks are no
  table (three context-free hooks, no state to recover). An independent soundness review found
  three paths to UB from safe Rust, each with a Miri reproducer: `as_ref`/`as_mut` handles
  ignoring `S`'s thread-safety (F1), a deferred release dropping `S` on another thread (F2),
  `#[target_feature]` accepted on a trampoline (F3); and three hazards: a wrong `@` marking
  compiling (F4), `CTable::new` letting safe code choose slots (F5), `S` borrowing past
  `into_c` (F6).
- Fixes (`041fc6c`): F1 `S: Send + Sync` on the handles with `unsafe` `_local` forms; F2 a
  stated obligation; F3 `#[doc]` only; F4 `unsafe impl` input; F5 `CVtable::base()` and
  `unsafe with_base`; F6 `S: 'static` on `into_c`. Re-review: those closed, but the macro
  matched the caller's `unsafe` as a literal and dropped it, hiding the obligation from
  `forbid(unsafe_code)` and clippy (N1, reproduced); the contract licenses NULL slots C calls
  unconditionally (N2); the `Install` token can be kept (N3); the handles' bound is stricter
  than `define_ctype!`'s `!Send` handles need (N4); and F2 should be a bound, as `CPayload` and
  `CClosure` make theirs. A second round of fixes is under way.
