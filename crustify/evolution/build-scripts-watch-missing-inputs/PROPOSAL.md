# Build scripts watch inputs that do not exist, so every cargo run rebuilds the crate

Status: `open`, proposed by `modules/libgit2/src/util/utf8.c/wave-232/batch-1/run-1`; no change
made.

## Signal

A failed gate run that recurs, and measured waste. The wave gate's Miri stage of `end-gates`
builds the test binaries once (`miri-build`), then runs one `cargo miri test` per binary side
by side, all in the snapshot's `target/miri`. Every one of those runs compiles `libc-sys`,
`libgit2-crustify` and `glibc-bindings` again. Seven builds of the same crates in one target
dir at once race. One of them then fails with `error: crate `libc_sys` required to be
available in rlib format, but was not found in this form` after its tests pass.

- `wave-229`'s review of `diff_parse.c` met it on `ub_safe` (the dashboard's Notes entry
  `libgit2/src/libgit2/diff_parse.c` wave 229: review of wave 228). Rerun alone, the binary
  passed.
- This run met it on `lib` at wave 232's tip `d48feaced`: `miri-lib` passed 441 and ignored
  461, then exited 1 on that error. The other six binaries passed. Each of the seven per-binary
  logs shows the same three `Compiling` lines after `miri-build` had built them.

## Evidence

Cargo says why each crate is dirty, in an `end-gates` snapshot whose `target/miri` was just
built:

```bash
cd <tree>/crustify/tmp/gates-<label>
CARGO_TARGET_DIR=$PWD/target/miri cargo +nightly-2026-10-08 miri test -p libgit2-crustify \
  --test unit_pub --no-run -v 2>&1 | grep Dirty
```

```text
Dirty libc-sys v0.1.0 (/target/glibc-crustify/libc-sys): the file `/target/glibc-crustify/libc-sys/src/static_fns.c` is missing
Dirty libgit2-crustify v0.1.0 (…/gates-tip): the file `sys/libgit2/external.txt` is missing
Dirty glibc-bindings v0.1.0 (/target/glibc-crustify): the dependency `libc-sys` was rebuilt
```

The campaign's `build.rs` prints `cargo:rerun-if-changed` for every name of `SYS_INPUTS`,
`external.txt` included, which `sys/libgit2/` does not have (it has no types of another link
unit to blocklist). glibc's wrapper (`/target/glibc-crustify/libc-sys/build.rs`, `24da93f`)
does the same for `external.txt` and for `src/static_fns.c`, which its build writes to
`OUT_DIR` when the source tree has none. Cargo treats a watched path that does not exist as
changed, so each build script reruns on every invocation, and each crate after it rebuilds.
The cost is paid in every gate, by every `cargo` command run on the tree, not only under Miri.

## Destination

- The safe crate's `build.rs` (the campaign's, written as `bindgen`'s Build modes section
  describes, binder-owned) and glibc's wrapper's `libc-sys/build.rs`, in its side campaign: a
  build script watches only the inputs that exist. An optional input that is missing is
  watched by its directory instead, so that adding it reruns the script.
- `bindgen`'s Build modes section, which is the binder's to evolve: one sentence saying so,
  since every link unit's and every wrapper's build script is written from it.

`end-gates` needs no change once the builds are fresh: its per-binary runs then only run.

## Change

None made: neither file is the reviewer's, and the gate passes when the failing binary is
rerun alone, as this run did.

## Validation

Not made. Before landing, the reproducer above prints no `Dirty` line on a second run, and
`end-gates --stages miri` shows no `Compiling` line in its per-binary logs.

## Review

Pending.
