# integer.h's switch, rehearsed in scratch

What `wave-223/batch-1/run-1` ran under `crustify/tmp/`, after its fix
(`conventions/static-inline-functions-without-exports`), so that the module's integration
knows what its switch needs and what it gave. Nothing here was committed to `c/`.

1. A copy of the tree with the fix, its `c/` a git checkout of the fork's `230c9dbdb`, in
   which `c/src/util/integer.h`'s six `GIT_INLINE` definitions are replaced by these
   prototypes and the macros are left as they are:

   ```c
   extern int git__is_sizet(int64_t p);
   extern int git__is_ssizet(size_t p);
   extern int git__is_uint16(size_t p);
   extern int git__is_uint32(size_t p);
   extern int git__is_ulong(int64_t p);
   extern int git__is_int(int64_t p);
   ```

   `integer_h.rs`'s gateway lost its `#[cfg(feature = "ffi-export-src-util-integer-h")]`, as
   the switch deletes the feature.
2. The `plain` variant's `configure` and `build` from `crustify/artifacts/build.json`
   (version 7), in a build dir of its own, the side campaigns' installs by absolute path,
   `rust-staticlib.sh` run with `CRUSTIFY_BINDINGS_UPDATE=1`. The bindings it regenerated
   differ from the committed ones by the six `#[link_name = "git__is_<name>__extern"]`
   lines of `sys/libgit2/bindings.rs`, and `sys/libgit2/static_fns.c` loses its six
   `git__is_*__extern` wrappers: the functions are no longer `static inline`, so the raw
   bindings reach the exports. The switch commits both.
3. `nm libgit2.so` defines `git__is_int`, `git__is_sizet`, `git__is_ssizet`,
   `git__is_uint16`, `git__is_uint32` and `git__is_ulong` as local text symbols from the
   Rust library, and no `*__extern` wrapper.
4. `crustify/scripts/c-tests.sh` over that build: `3442` passed, `0` failed, `55` skipped
   of `3497`, no non-zero suite, which matches the baseline's `plain` record; `ctest`
   exited 0. A first run, with `c/` a `git archive` and no `.git`, failed
   `perf::merge::m1` and skipped `blame::simple::trivial_libgit2` and its cache-pressure
   twin: they open the checkout at `tests/resources/../..` as a repository.

The other variants were not built, and neither was the packaging gate.
