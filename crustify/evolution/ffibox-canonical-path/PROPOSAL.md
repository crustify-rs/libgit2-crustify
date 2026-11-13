# ffibox is declared by its canonical path

Status: `landed`, as crustify `7e35c25` on `libgit2-crustify/devel` (2026-11-13), once review-2 approved it.

## Signal

A rule whose result is worse than its omission (`evolution` §2): five `ub_safe` compile-fail
snapshots failed on the working branch (`928aa1ba9`) because rustc printed
`/opt/venv/share/ffibox/src/...:<line>:1` where the snapshots name `$FFIBOX/src/...`. trybuild
canonicalizes a path dependency's directory before normalizing it, while rustc prints the path
as declared; on this host `/opt/venv/share/ffibox` is a symlink to `/crustify-rs/ffibox`, so the
two never matched. The bundling migration's `2ea24a0db` declares ffibox (and its `[patch]`) by
`/crustify-rs/ffibox`, and the five pass as blessed.

The `ffibox` tool skill (line 38) names `ffibox = { path = "/opt/venv/share/ffibox" }`.

## Destination

The spec: the `ffibox` tool skill.

## Change

crustify `b3ebca7` and `7e35c25`: the `ffibox` skill's Rust tree section names the
checkout's canonical path, `/crustify-rs/ffibox` in Crustify's image, with no symbolic link on
it, and a `[patch]` of another ffibox source names the same; the Dockerfile and the image README
say the same, keeping `/opt/venv/share/<tool>` as a symlink for older manifests.

## Validation

`2ea24a0db`: the five snapshots pass on plain.

## Review

review-1 (`crustify/artifacts/evolution/bindgen-modes-after-bundling/review-1`), of crustify `b3ebca7`: **rejected**: the skill still gave
`/opt/venv/share/ffibox`, which the image makes a symlink, as the path to name, and the
Dockerfile and the image README called it the manifest's path. The mechanism held (trybuild
canonicalizes path dependencies; the five snapshots pass with `$FFIBOX`). Fixed in `7e35c25`:
all three name `/crustify-rs/ffibox`. 

review-2 (`crustify/artifacts/evolution/bindgen-modes-after-bundling/review-2`), of `7e35c25`: **approved**; every review-1 fix resolved (the skill's example, the Dockerfile and the image README name `/crustify-rs/ffibox`, a real directory in the image; the campaign's `Cargo.toml` and `[patch]` agree); documentation only, no weakening.
