class: nit
evidence: failing `cmake` configure of the fork with the lesson's rule applied to `deps/llhttp/llhttp.c`
conditions: the fork at `230c9dbdb`; CMake 3.28.3; the variants' `configure` options (OpenSSL, builtin SHA-1 and HTTP parser, no SSH), tests and CLI off

# The provisional lesson `cmake-property-naming-a-removed-source` takes in the switch that removes a directory's last source

**Where.** `crustify/LESSONS.md`, `cmake-property-naming-a-removed-source`, added on
2026-11-14 from `module-switch-procedure`'s item 13: its Scope, "the switches of
`deps/llhttp/*.c`", and its last sentence, "The dangling name stays until the last file the
property names goes".

**What is wrong.** The rule (leave a `CMakeLists.txt` that names the removed source only in
a source-file property unchanged) holds for `api.c`'s and `http.c`'s switches, as its
evidence shows: gen3 and gen5 configured and built, and gen5's `build.ninja` in every variant
still compiles `llhttp.c` with the property's flags. Its scope also takes in
`deps/llhttp/llhttp.c`'s switch, the directory's last C source. There the directory's
`file(GLOB SRC_LLHTTP "*.c" "*.h")` leaves `llhttp.h` alone, `add_library(llhttp OBJECT …)`
has nothing to compile, and CMake fails to generate. That switch must edit CMake, which the
rule says no switch in its scope does. The lesson it extends, "A libgit2 module's switch
removes its file from `c/` and changes no CMake file" (scope: every later module's switch),
has the same gap, at the switch that removes any directory's last source. That entry is not
provisional, so it is left for the next reconciliation of the lessons.

**Reproduce.**

```
d=$(mktemp -d) && git -C c archive 230c9dbdb | tar -x -C $d && cp -r $d $d-drop && rm $d-drop/deps/llhttp/llhttp.c
for s in $d $d-drop; do cmake -S $s -B $s-b -G Ninja -DUSE_HTTPS=OpenSSL -DUSE_SHA1=builtin \
  -DUSE_SHA256=HTTPS -DUSE_HTTP_PARSER=builtin -DUSE_SSH=OFF -DBUILD_TESTS=OFF -DBUILD_CLI=OFF \
  -DLIBGIT2_RUST_LIBRARY=/nonexistent.a; echo "rc=$?"; done
```

The copy as it is configures (`rc=0`); the copy without `llhttp.c` fails (`rc=1`):
`CMake Error: CMake can not determine linker language for target: llhttp`.

**Fix.** The entry's rule now says the dangling names stay while the directory keeps a C
source, and that the switch removing its last one is outside the rule and edits CMake. Its
scope is narrowed to switches after which the directory keeps a C source, `api.c`'s and
`http.c`'s here, and excludes `llhttp.c`'s. The evidence cites this check. Changed, the entry
stays `provisional`, for the next review batch. Its generality is as before: it refines a
libgit2-specific lesson (libgit2's CMake globs each directory's sources), and the general
gate is `switch-gate-beyond-c-tests`'s, not this.

**Tiers.** Caught by this review batch, the first to read the lesson; no earlier tier had it.
