#!/bin/bash
# Build the safe crate's Rust static library for a variant's own C to link (crustify's
# porting-workflow skill, Builds): once a module has switched from C, the remaining C, its
# library, CLI and tests, reaches the module through the crate's exports, and its CMake
# build links this library (`LIBGIT2_RUST_LIBRARY`).
#
#   rust-staticlib.sh <variant> <archive>
#
# Run from the variant's build dir, configured and not yet built, as the first step of its
# `build` command in crustify/artifacts/build.json; <archive> is the library written,
# relative to the build dir, as its configure's `-DLIBGIT2_RUST_LIBRARY` names it. The crate
# is the one of the checkout this script belongs to, built as a `staticlib` on the dev
# profile, the campaign's tests' own, in campaign mode against this build dir
# (`CRUSTIFY_BUILD_DIR`) with `CRUSTIFY_STATICLIB` set, so that it reads the build's headers
# and configuration and links none of its C, and with the `bindgen` feature, so that the
# committed bindings are checked against them. Its Rust is instrumented as
# crustify/scripts/cargo-variant.sh instruments <variant>'s, so that one runtime serves the C
# and the Rust of every C consumer. Cargo's target dir is the build's own, `crustify/cargo`;
# the archive is replaced atomically, so that a link never reads a partial one.
set -euo pipefail
usage='usage: rust-staticlib.sh <variant> <archive>'
variant=${1:?$usage}
archive=${2:?$usage}
[ $# -eq 2 ] || { echo "$usage" >&2; exit 2; }
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
host=x86_64-unknown-linux-gnu
[ -f CMakeCache.txt ] && [ -f build.ninja ] || {
  echo "rust-staticlib.sh: $PWD is not a configured CMake build with the Ninja generator" >&2
  exit 2
}
build=$(pwd -P)
case $archive in /*) ;; *) archive=$build/$archive ;; esac

export CRUSTIFY_BUILD_DIR=$build CRUSTIFY_STATICLIB=1
export CARGO_TARGET_DIR=$build/crustify/cargo
crate=(-p libgit2-crustify --lib --locked --features bindgen)
bundled=
case $variant in
  plain)
    out=$CARGO_TARGET_DIR/debug
    "$here/crustify/scripts/cargo-variant.sh" "$variant" rustc "${crate[@]}" --crate-type staticlib ;;
  coverage|asan-ubsan|tsan)
    out=$CARGO_TARGET_DIR/$host/debug
    "$here/crustify/scripts/cargo-variant.sh" "$variant" rustc "${crate[@]}" --crate-type staticlib ;;
  bsan)
    # `cargo bsan` drives `build`, `run` and `test` only, and a `build` cannot change a
    # crate's type. It sets its rustc driver up and runs the cargo CARGO names, so a stand-in
    # for that cargo turns its `build` into `rustc --crate-type staticlib`, every other call,
    # its `metadata` among them, passing through. The /opt shim of `cargo bsan` sets CARGO
    # itself, so the real cargo-bsan, the ELF on PATH, is run directly, as the shim runs it.
    out=$CARGO_TARGET_DIR/bsan/$host/debug
    real_cargo=$(type -P cargo)
    real_bsan=
    for c in $(type -ap cargo-bsan); do
      [ "$(head -c4 "$c")" = $'\x7fELF' ] && { real_bsan=$c; break; }
    done
    [ -n "$real_bsan" ] || { echo "rust-staticlib.sh: no cargo-bsan executable on PATH" >&2; exit 1; }
    mkdir -p "$CARGO_TARGET_DIR"
    shim=$CARGO_TARGET_DIR/cargo-rustc-staticlib
    printf '%s\n' '#!/bin/bash' \
      "[ \"\${1:-}\" = build ] || exec '$real_cargo' \"\$@\"" \
      'shift' \
      "exec '$real_cargo' rustc \"\$@\" --crate-type staticlib" >"$shim"
    chmod +x "$shim"
    side=$(realpath -m "$build/../../../..")
    export PKG_CONFIG_PATH="$side/pcre2-bindings/crustify/builds/$variant/install/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export CRUSTIFY_BUILD_VARIANT=$variant
    (cd "$here" && RUSTUP_TOOLCHAIN=bsan CARGO=$shim "$real_bsan" bsan build "${crate[@]}")
    # The driver bundles BorrowSanitizer's C runtime into the library; the C's own links
    # link it (the variant's CMAKE_EXE_LINKER_FLAGS), so that each process holds it once,
    # and a shared library none: its members are removed from the copy written.
    bundled=$(rustc +bsan --print sysroot)/lib/libclang_rt.bsan-x86_64.a
    [ -f "$bundled" ] || { echo "rust-staticlib.sh: no BorrowSanitizer runtime at $bundled" >&2; exit 1; } ;;
  *)
    echo "rust-staticlib.sh: unknown variant: $variant" >&2; exit 2 ;;
esac

lib=$out/liblibgit2_crustify.a
[ -f "$lib" ] || { echo "rust-staticlib.sh: cargo wrote no $lib" >&2; exit 1; }
mkdir -p "$(dirname "$archive")"
tmp=$archive.tmp.$$
trap 'rm -f "$tmp"' EXIT
cp "$lib" "$tmp"
if [ -n "$bundled" ]; then
  mapfile -t members < <(ar t "$bundled" | sort -u)
  while ar t "$tmp" | grep -qxF -f <(printf '%s\n' "${members[@]}"); do
    ar d "$tmp" "${members[@]}"
  done
  ranlib "$tmp"
fi
mv -f "$tmp" "$archive"
trap - EXIT
echo "rust-staticlib.sh: $archive is the $variant Rust of $here"
