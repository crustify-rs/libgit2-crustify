# The packaging gate's command is worked out by hand

Status: `open`.

## Signal

A procedure I had to work out by hand, over three failed runs, that the gate names but does
not give.

## Evidence

`wave-186/batch-0/run-1` changed the raw bindings (`sys/git2/allowlist.txt`). Per
`testing-gates`' Regressions section, that means it packages the safe crate "built alone in
bundled mode … each crate the package depends on … patched to its local source". Run from
the worktree with `CRUSTIFY_BUILD_VARIANT=` and `--allow-dirty`:

1. `cargo package` failed: `no matching package named glibc-bindings found` (crates.io). The
   manifest's `[patch."https://github.com/crustify-rs/…"]` does not apply to the packaged
   manifest, whose dependencies are registry ones. The fix was
   `--config 'patch.crates-io.glibc-bindings.path="/target/glibc-crustify"'
   --config 'patch.crates-io.ffibox.path="/crustify-rs/ffibox"'`.
2. The bundled CMake configure then failed: `PCRE2 support was requested but not found`, and
   after that `system zlib was requested but not found`. The side campaigns' installs
   (`<repo>/../{openssl,pcre2,zlib}-bindings/crustify/builds/plain/install`) have to be on
   `CMAKE_PREFIX_PATH`, and colon-separated, since that is the environment variable's form.
   The `plain` variant's `configure` in `build.json` passes them semicolon-separated, as a
   `-D` value, and copying that form failed.

With both changes, the crate packaged 698 files and verified.

## Destination

A campaign artifact the assistant owns: a script beside `crustify/scripts/cargo-variant.sh`
(such as `crustify/scripts/package.sh`) that runs the gate with the patches derived from the
manifest's `[patch]` sections and the side installs from `build.json`'s `plain` prefix path.
Its use would then go in `testing-gates`' Regressions section, or in a lesson until the
wrappers are published.

## Change

Not made: neither file is the translator's. The command that passed:

```bash
S=$(realpath -m crustify/builds/plain/../../../..)
I() { echo "$S/$1-bindings/crustify/builds/plain/install"; }
CRUSTIFY_BUILD_VARIANT= OPENSSL_ROOT_DIR=$(I openssl) \
CMAKE_PREFIX_PATH="$(I openssl):$(I pcre2):$(I zlib)" \
PKG_CONFIG_PATH="$(I pcre2)/lib/pkgconfig:$(I zlib)/lib/pkgconfig" \
cargo package --allow-dirty \
  --config 'patch.crates-io.glibc-bindings.path="/target/glibc-crustify"' \
  --config 'patch.crates-io.ffibox.path="/crustify-rs/ffibox"'
```

## Validation

The command above packaged and verified the tree of `wave-186/batch-0`. The two earlier
forms failed as quoted.

## Review

None yet.
