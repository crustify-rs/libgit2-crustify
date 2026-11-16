# Porting libgit2's core to Rust: design and conventions

This document is binding for every work package (WP) of the port. Read it in
full before you write code, and read every report in `/work/reviews/` on the
components ported before yours.

## 1. What is ported, and the boundary

The port replaces, on Linux x86-64 with the configuration in §9, these C
sources (each with its header):

- `src/util`: everything portable: `alloc`, `allocators/{stdalloc,failalloc,debugalloc}`,
  `errors`, `runtime`, `thread`, `util`, `str`, `vector`, `pool`, `tsort`,
  `pqueue`, `utf8`, `varint`, `strlist`, `hash` (+ `hash/collisiondetect`,
  `hash/sha1dc/*`, `hash/openssl` replaced by Rust crates), `zstream`, `rand`,
  `regexp`, `wildmatch`, `date`, `net`, `posix`, `futils`, `fs_path`,
  `filebuf`, `sortedcache`, `unix/map`, `unix/realpath`, `unix/process`.
  The header-only facilities (`array.h`, `hashmap*.h`, `integer.h`,
  `bitvec.h`, `ctype_compat.h`, …) keep their headers for the C that stays,
  and get Rust equivalents.
- `src/libgit2`: `oid`, `oidarray`, `object`, `object_api`, `odb`,
  `odb_loose`, `odb_pack`, `odb_mempack`, `pack`, `mwindow`, `delta`,
  `commit`, `commit_list`, `tree`, `tree-cache`, `blob`, `tag`, `signature`,
  `refs`, `refdb`, `refdb_fs`, `reflog`, `config`, `config_file`,
  `config_parse`, `config_list`, `config_cache`, `config_mem`,
  `config_snapshot`, `repository`, `index`, `revwalk`, `buf`, `parse`,
  `strarray`, `hashsig`, `libgit2`, `settings`, and, because they are
  inseparable from those, `cache`, `index_map` and `grafts`.

Everything else (attr, filter, diff, merge, checkout, iterator, submodule,
worktree, transports, streams, commit_graph, midx, indexer, pack-objects,
refdb_reftable, …) stays in C. Its transitive closure is all of libgit2, so
this is the boundary. Rust calls it through `extern "C"` declarations in
`crate::sys` (§4.6), and it calls the Rust through the same symbols it
called before.

`win32/*` and the hash backends of other platforms stay in C. On a platform
other than Unix, CMake's `USE_RUST` is OFF and the C is built as before
(`cmake/RustPort.cmake`). The Rust implements the Unix branch of every
`#ifdef`; a `GIT_WIN32` branch is not ported.

## 2. Layout of the tree and the build

```
rust/
  Cargo.toml                 workspace, lints
  ported_sources.txt         C sources that leave the build (one per line)
  libgit2.exports            the exact dynamic symbol list of the C build
  PORTING.md                 this file
  DEFECTS.md                 C defects found, and what the Rust does instead
  DIVERGENCES.md             every intended difference from the C
  libgit2-rs/
    build.rs                 links the C rest for `--features link-c`
    src/lib.rs
    src/util/<name>.rs       one module per src/util/<name>.c
    src/<name>.rs            one module per src/libgit2/<name>.c
                             (`tree-cache` → `tree_cache`)
    src/sys/<cmodule>.rs     extern "C" declarations of C that stays C
    tests/*.rs               integration tests
```

- CMake (`cmake/RustPort.cmake`) builds the crate as a static library with
  cargo and whole-archives it into `libgit2.so`; the tests and the CLI link it
  too. `libgit2.so` is linked with a version script made from
  `rust/libgit2.exports`, so it exports exactly the C build's ABI.
- A C file leaves the build when its path is listed in
  `rust/ported_sources.txt`. List it only when the Rust defines every
  function it defined (public and internal) that anything still references.
- `git2_c_rest` (`<build>/libgit2_c_rest.a`) is all the C that is still C.
  `cargo test --features link-c` with `LIBGIT2_BUILD_DIR=<build>` links it.

Configure a build with `/work/tools/configure.sh <src> <build>`, then
`ninja -C <build>`. Run the clar suites in parallel with
`/work/tools/runtests.sh <build> [suite-prefix ...]`. The C reference build is
`/work/build-c` (never modify it); all its offline suites pass, so every
failure in yours is yours.

## 3. Work packages, worktrees and merging

Each WP is done in its own git worktree of `/work/libgit2` at `/work/wt/<wp>`
on branch `wp/<wp>`, with its own CMake build dir `/work/builds/<wp>`.
Commit to your branch as you go (small commits, clear messages). Do not
touch `master` and do not merge; the coordinator merges. Stay inside your
WP's files; where you must touch a shared file (`lib.rs`, `sys/*.rs`,
`ported_sources.txt`, `Cargo.toml`, a header another WP owns), keep the edit
minimal and say so in your final report.

When your module calls a module that another WP is porting concurrently,
call it through its C symbol, declared in `src/sys/pending.rs` with a
`// PENDING(<wp>)` comment. The integration step replaces each with the
Rust call once both have landed. Do not declare in `pending.rs` a function
that is already ported on `master`: call its Rust API.

## 4. Architecture

### 4.1 Three kinds of type

Every C type of a ported module is one of these. Decide per type, from what
the C that stays and the tests (`tests/libgit2`, `tests/util`, `src/cli`)
do with it. Grep them: field accesses (`->field`, `.field`), `sizeof`,
by-value embedding, allocation and freeing by C.

1. **Opaque.** C that stays only passes pointers to it around. The Rust type
   is native Rust (`Vec`, `String`, `Box`, `Arc`, `Mutex`, `HashMap`, …),
   with no `#[repr(C)]` beyond what the pointer needs. Remove the struct
   body from the internal header (leave the typedef) so that no C can come to
   depend on it.
2. **Shared prefix.** C reads or writes some fields, but never allocates,
   embeds or sizes the struct. The C-visible fields stay in the header, in
   order, as a `#[repr(C)]` mirror at offset 0 of the Rust allocation; the
   rest moves out of the header into Rust-native state after the prefix (or
   behind a pointer). Keep what C touches; move what it doesn't.
3. **Mirrored.** C allocates it, embeds it by value, sizes it, or it is in
   a public header (`include/git2/**`). Its Rust definition is an exact
   `#[repr(C)]` mirror of the C layout. Public-header types are never
   changed.

For kinds 2 and 3, a layout test asserts `size_of`, `align_of` and every
C-visible field's `offset_of!` against values the C compiler computes
(`tests/layout.rs` and its generated C; see the foundation's README in
`src/util/README.md` once it exists). C bitfields are an integer with masks;
document the bit positions (x86-64 SysV: first declared is the least
significant bit).

### 4.2 References to memory C can see

Follow ffibox's rule: **no Rust reference covers memory that anything else
may write or free while the reference lives**. Concretely:

- A `&mut T` or `&T` to a kind-2/3 object is fine for the extent of an
  exported call *only* if, during that extent, no C (including user
  callbacks) can reach the object, and no other thread can (per libgit2's
  threading rules). Otherwise reach it through a handle (`ffibox`'s
  `define_ctype!` layout types and `FooRef` / `FooMut`), or wrap the fields
  that are written while shared in `UnsafeCell` / atomics.
- Fields that the C writes concurrently (refcounts, lazily set pointers such
  as `repo->_odb`, which C sets with compare-and-swap) are atomics in Rust.
- Do not hold a Rust reference into an object across a call that hands
  that object's pointer to C, or that runs a user callback which may reach
  it; re-derive after.
- libgit2's objects are not thread-safe unless documented otherwise, but
  the repository's caches, odb, refdb and config *are* used from several
  threads at once (`tests/libgit2/threads`). Make such types `Sync` with
  real synchronisation, never by assertion.

### 4.3 Memory

- Memory that C may free, `realloc`, or keep past the call comes from
  libgit2's allocator (`git__allocator`, through the foundation's
  `GitAlloc`). This is what keeps `GIT_OPT_SET_ALLOCATOR` and the tests'
  failing allocators (`cl_alloc_limit`) meaningful.
- Memory that only Rust frees uses Rust's global allocator.
- An allocation failure that the C reports as an error (`GIT_ERROR_NOMEMORY`,
  return -1) is reported the same way from the Rust: use `try_reserve` /
  the fallible `GitAlloc` paths on every path where C checked the
  allocation. Where the Rust allocates in Rust's allocator and C would have
  failed gracefully, prefer fallible APIs; where that is unreasonable, list
  it in `DIVERGENCES.md`.
- Use `ffibox` for ownership C shares: `CBox`/`CArc`/`CVec`/`CStrBox` over
  C-owned memory, `RustAlloc`/`export_cdrop!` for Rust-owned objects that C
  frees, `CTable` + `impl_cvtable!` for tables of function pointers that C
  calls into (`git_odb_backend`, `git_refdb_backend`, `git_config_backend`,
  `git_writestream`, iterators), `CFn` / `CClosure` for callbacks.

### 4.4 Errors

The C model is kept exactly: an error is a negative `int` code
(`GIT_ERROR` = -1, `GIT_ENOTFOUND`, …), and the message and class live in the
thread's error state (`git_error_last`). Rust functions return
`crate::util::errors::Result<T>`, whose error is a `Copy` code with the
message already set in the thread state, as `git_error_set` would. Messages
must be the C's byte for byte (tests compare them), including
`GIT_ASSERT_ARG`'s `invalid argument: '<expr>'` with the C expression text,
and `GIT_ERROR_CHECK_ALLOC`'s out-of-memory message. When the C overrides,
clears or preserves an error after a callee fails, do the same.

The safe Rust API (§5) converts the code and the thread's message into a
rich `Error` at its boundary.

Variadic C functions (`git_error_set`, `git_str_printf`, `git_str_join_n`, …)
cannot be defined in stable Rust. They stay as thin C shims in
`src/util/rust_shims.c`, which format with `vsnprintf` and call a Rust
function; the shim is the only C a ported module keeps.

### 4.5 Exported C functions

- Each module has `pub mod ffi` holding its exports:
  `#[unsafe(no_mangle)] pub unsafe extern "C" fn git_…`. Every one is
  `unsafe`, with a `# Safety` section that states C's contract in terms a
  caller can check (which pointers must be valid, for how long, what may
  alias, what may run concurrently).
- An export is a thin shim: check arguments as the C did (same errors),
  convert raw pointers to references/handles/slices in one justified place,
  call the safe Rust, write out-parameters, convert the result.
- Panics never cross into C. Wrap each export's body in the foundation's
  barrier (ffibox's `callback::barrier` underneath), which maps a panic to
  `GIT_ERROR` with class `GIT_ERROR_INTERNAL`.
- Export every function the C defined that anything outside the file
  references (including the tests): internal `git__…`/`git_…` functions too.
  Static C functions are not exported. `GIT_INLINE` functions in headers stay
  in the header for C callers; the Rust has its own equivalent.
- With `DEPRECATE_HARD`, deprecated functions are not exported.
- `rust/libgit2.exports` is the public ABI. Every symbol in it that your
  module's C defined must be exported by your Rust.

### 4.6 Calling the C that stays

`src/sys/<cmodule>.rs` declares the functions and types of C module
`<cmodule>` that the Rust calls (`unsafe extern "C" { … }`), and gives each a
safe wrapper whose `SAFETY` comment states why the call's contract holds.
Ported code calls the safe wrappers, never the raw declarations. Under Miri,
foreign calls are unsupported: keep logic that can be tested in safe Rust
apart from the calls, so that the logic is Miri-tested.

### 4.7 Unsafe code

- Minimise it. Safe Rust first; `unsafe` only at the C boundary (exports,
  `sys` wrappers, layout/handle accessors) and where a measured hot path
  needs it.
- Every `unsafe fn` has `# Safety`; every `unsafe {}` block has a
  `// SAFETY:` comment that says why each of its conditions holds *here*.
  Keep one unsafe operation per block where practical.
- Safe functions must not be able to cause UB, whatever their arguments.
  A safe function that relies on an invariant of a type gets it from the
  type's constructors, which are the only way to make one.
- No `static mut`. Globals that C reads or writes (e.g. `git__allocator`)
  are `#[unsafe(no_mangle)]` statics of a type with interior mutability
  whose accesses are justified.
- The workspace lints (`rust/Cargo.toml`) are on. `cargo clippy
  --all-targets` and `cargo fmt --check` are clean before you finish.

### 4.8 Performance

The Rust is as fast as the C or faster. Keep the C's algorithms and data
layouts where they matter (hot loops in parsing, hashing, delta, pack
lookup, index search), avoid extra copies and allocations, and check with a
benchmark when unsure. `memchr` is available. Report numbers (§7).

### 4.9 Equivalence and defects

Where the C is well defined, the Rust behaves the same: same return codes,
same error classes and messages, same output bytes, same files on disk.
Where the C has undefined behaviour or a defect (out-of-bounds read,
overflow, leak, use-after-free, wrong result), the Rust gives a defined
result as close to the C's as possible, and you add an entry to
`rust/DEFECTS.md`: component, C location, the defect, the input that
triggers it, what the Rust does. Every other intended difference goes in
`rust/DIVERGENCES.md`.

## 5. The safe Rust API

For each ported component the crate offers Rust consumers a safe, idiomatic
API with the capabilities of libgit2's public C API for that component
(`include/git2/<component>.h`): owners that release on `Drop`, `Clone` where
C refcounts or duplicates, lifetimes where C requires a parent to outlive a
child (a `Commit<'repo>`), iterators for C's iterators and `foreach`,
closures for callbacks, `Result<T, Error>` (with the code, class and message)
for status codes, `&[u8]`/`&Path`/`&str` for strings as appropriate (paths
are bytes, not UTF-8). No raw pointer and no `unsafe fn` in it. Its tests
compile under `#![forbid(unsafe_code)]`.

The C ABI and the Rust API are two faces of the same safe Rust: the exports
are shims over it, the API is a thin layer over it. Design both so that
neither changes when the rest of libgit2's C is ported later: the API must
not expose that something is C (no C types in signatures, no `sys` types).

API naming follows libgit2 (`Repository`, `Odb`, `OdbObject`, `Oid`,
`ObjectType`, `Object`, `Commit`, `Tree`, `TreeEntry`, `Blob`, `Tag`,
`Signature`, `Reference`, `Reflog`, `Config`, `ConfigEntry`, `Index`,
`IndexEntry`, `Revwalk`, `Error`, `ErrorCode`, `ErrorClass`), re-exported
from the crate root.

## 6. Tests

Every WP delivers:

1. **The clar suites.** All offline suites pass on your build
   (`runtests.sh` with no prefix) before you finish; while working, run the
   suites of your component.
2. **Rust unit/integration tests** of your modules' safe functions and of
   the exported C ABI (call the `extern "C"` functions from Rust with raw
   pointers, as C would), covering edge cases and the C's behaviour on them.
   Where the expected value is not obvious, take it from the C reference
   (write a small C program against `/work/build-c`, or read the clar test).
3. **Miri.** `cargo +nightly miri test` runs every test that does not reach
   C (mark the others `#[cfg_attr(miri, ignore)]`), with
   `MIRIFLAGS=-Zmiri-strict-provenance` where possible, clean.
4. **Sanitizers.** Your build passes the clar suites of your component under
   ASan+UBSan (`configure.sh … -DCMAKE_C_FLAGS="-fsanitize=address,undefined"
   -DRUST_SANITIZER=address`). TSan is run by the coordinator on the
   integrated tree; run `threads::` under it if your component is used
   there.
5. **Differential tests** for parsers and encoders: feed the same inputs to
   the C reference and to your Rust (e.g. via a C driver linked against each
   build) and compare. Keep drivers and corpora under `rust/difftest/`.

## 7. Benchmarks

Each WP adds benchmarks for its component under `rust/bench/`: a C program
using the public API (or the internal API for util), built against
`/work/build-c/libgit2.so` and against your build, run on representative
workloads (`/work/bench-data` holds large real repositories). Report
median-of-N wall time for C and Rust in your final report and in
`rust/bench/RESULTS.md`.

## 8. Review

Every component is reviewed by an agent that did not write it. Reports go
to `/work/reviews/<wp>-<round>.md`, listing each finding with its location,
its class (UB, divergence from the C, weak safety documentation, avoidable
unsafe, other), and a severity. Authors fix every finding or explain why
not, in the same file under "Response". Before you start a WP, read all
reports so far, avoid every defect they found, and check the code you have
already written for the same kind of defect.

## 9. Configuration

The reference configuration (`/work/tools/configure.sh`):
`-DDEPRECATE_HARD=ON -DBUILD_TESTS=ON -DBUILD_CLI=ON -DBUILD_EXAMPLES=ON
-DUSE_THREADS=ON -DUSE_NSEC=ON -DUSE_HTTPS=OpenSSL -DUSE_SHA1=builtin
-DUSE_SHA256=OpenSSL -DUSE_HTTP_PARSER=http-parser -DUSE_REGEX=pcre2
-DUSE_SSH=libssh2 -DUSE_AUTH_NTLM=builtin -DUSE_AUTH_NEGOTIATE=GSSAPI
-DUSE_COMPRESSION=zlib`, RelWithDebInfo, clang. So the Rust implements:
threads (pthreads), nanosecond mtimes (`st_mtim`), SHA1 with collision
detection (sha1dc), SHA256, PCRE2 regular expressions, zlib-compatible
compression; the features that live beyond the boundary (HTTPS, SSH, NTLM,
Negotiate, the HTTP parser) stay in C.

## 10. Your final report

When your WP is done, reply with: what you ported; the types and their kind
(§4.1); header changes; ffibox primitives used and any you added (in
`/work/ffibox`, on its own branch `wp/<wp>`, with tests); DEFECTS and
DIVERGENCES entries; test results (clar, cargo test, Miri, sanitizers);
benchmark numbers; the `pending.rs` entries you left; anything unfinished.
