# Two raw FFI crates were listed as safe bindings in dependencies.json

## What the earlier plan decided

`dependencies.json` listed, among the crates that put a safe Rust API over a library
(`rust_equivalent.bindings`), two crates whose own notes say they have none:

```json
{"crate": "libc", "sys": null, "feature_parity": "partial", "license": "MIT OR Apache-2.0",
 "notes": "raw FFI: every entity the campaign consumes is declared, but as unsafe extern functions over raw pointers, with no safe API; the crate the workspace already depends on"}
```

under `libc`, and under `libpcre2-8`:

```json
{"crate": "pcre2-sys", "sys": null, "feature_parity": "partial", "license": "Unlicense OR MIT",
 "notes": "raw FFI: declares every function the campaign consumes as unsafe extern functions over raw pointers, with no safe API; the crate the workspace already depends on"}
```

## Evidence

The dependencies schema, section 2: "`bindings` [lists] the crates that put a safe Rust API
over the C library itself ... A crate that wraps the library but exposes raw pointers or
`unsafe` to its users is not safe bindings; a `-sys` crate alone is listed as no binding."
Both crates are exactly that: `libc` declares glibc's functions as `unsafe extern "C"`
items, and `pcre2-sys` is the `-sys` crate the `pcre2` entry already names as its `sys`.

```sh
cd /target/libgit2-bindings
git show HEAD~1:crustify/artifacts/dependencies.json | python3 -c '
import json,sys
for l in json.load(sys.stdin)["libraries"]:
    print(l["name"], [(c["crate"], c["sys"]) for c in l["rust_equivalent"]["bindings"]])'
grep -n "pub fn qsort" ~/.cargo/registry/src/*/libc-0.2.191/src/unix/mod.rs            # 697, in an extern "C" block
grep -n "pub fn pcre2_compile_8" ~/.cargo/registry/src/*/pcre2-sys-0.2.10/src/bindings.rs  # 601, likewise
```

## Edit

`dependencies.json`: the two entries are removed from `bindings`. `libc` keeps `nix` and
`rustix`; `libpcre2-8` keeps `pcre2`, whose `sys` still names `pcre2-sys`. Nothing
downstream moves: no module, wave or wrapper depends on these lists, which inform the
user's choice of adopting a crate.

## Questions for the user

None.
