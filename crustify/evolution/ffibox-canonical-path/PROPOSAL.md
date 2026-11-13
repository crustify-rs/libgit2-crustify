# ffibox is declared by its canonical path

Status: `open`.

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

To be drafted: the path dependency names ffibox's checkout by its canonical path (no symlink on
it), so that compile-fail snapshots normalize it.

## Validation

`2ea24a0db`: the five snapshots pass on plain.

## Review

Pending.
