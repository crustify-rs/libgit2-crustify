# Intended differences between the Rust port and the C

One entry per difference: component, what differs, why.

## Foundation (src/util, buf, strarray)

- **errors: where the thread state lives.** C kept each thread's error
  state in a `git_tlsdata` slot allocated with `git__allocator`. The port
  keeps it in a Rust `thread_local!` in Rust's allocator. Consequences:
  error messages are recorded even while `git__allocator` fails (under
  `cl_alloc_limit`, C would keep the previous error); a thread's state is
  freed when the thread exits even after `git_libgit2_shutdown` (C leaked
  other threads' states once the key was deleted); "thread-local data
  initialization failure" is only returned while the thread's locals are
  being destroyed. `git_error_last` returns the same `git_error` contents
  with the same lifetime (until the thread's next error call).
- **errors: formatting first.** `git_error_set`'s shim formats the message
  (with `vsnprintf`) before touching the error state; C cleared the buffer
  first, so an argument pointing at the current message (`"%s",
  git_error_last()->message`) printed as empty in C and prints the message
  in the port. Messages over 511 bytes are formatted in a `malloc`'d buffer
  of the C library, not `git__allocator`.
- **errors: messages set from Rust** use Rust's formatting
  (`git_error_set!`); callers write formats whose output is the C's. Byte
  strings go in unchanged through `errors::Bytes`.
- **errors: panics.** A panic in Rust code reached from C returns the
  export's failure value with class `GIT_ERROR_INTERNAL` and the message
  `unrecoverable internal error: a Rust panic was caught` (C has no such
  path).
- **ctype: the C locale.** `git__isspace`, `git__tolower` and the other
  classes of `ctype_compat.h` are the C locale's (ASCII), whatever
  `setlocale` was called with (libgit2 never calls it).
- **str: borrowed strings are not written** (see DEFECTS.md): `consume` of a
  borrowed string moves its pointer; `unquote` and `unescape` copy it into
  an owned buffer first.
- **str: Rust formatting into a `git_str`.** `GitStr::printf(format_len,
  args)` reproduces `git_str_printf`'s growth (and so `asize`) when given the
  length of the C format it replaces; `write!` into a `GitStr` grows piece
  by piece.
- **str: `git_str_join_n` with more than 16 segments** allocates the
  argument array with `git__mallocarray` (it may fail with the
  out-of-memory error where C did not allocate).
- **alloc: `git__allocator`'s storage** is three atomic pointers with
  `git_allocator`'s layout; Rust reads and replaces it atomically. `GitAlloc`
  vouches for 8-byte alignment (what the tests' and debug allocators give).
- **runtime: the registry** of shutdown callbacks is a `Vec` under a
  `std::sync::Mutex`, and the init lock a `std::sync::Mutex`; the order,
  the limit (32) and the error are C's. A NULL shutdown callback is ignored
  at registration (C stored it and skipped it at shutdown).
- **util: `git__hexdump`** builds the text in Rust and prints it with one
  `printf("%s")` (same output through the same `stdio` stream).
- **vector: `git_vector_dup`** with a NULL comparison function on a source
  that has one leaves the copy unsorted, as C (it compares the argument).

## hash, zstream, rand, regexp, wildmatch, date (WP u2)

- **hash: the context.** `git_hash_ctx` is, under `GIT_RUST`, an opaque
  `uint64_t opaque[GIT_HASH_CTX_WORDS]` (2400 bytes; the C's union was 2408)
  holding the algorithm and the Rust computation. No C that stays reads its
  fields; it embeds it by value as before. `git_hash_ctx_cleanup` does
  nothing (as both C backends of this configuration).
- **hash: the backends.** SHA-1 is the `sha1collisiondetection` crate (a
  Rust port of sha1dc, same defaults: safe hash, unavoidable bit conditions,
  collision detection); SHA-256 is a Rust implementation
  (`util/hash/sha256.rs`, with an AVX2/BMI path chosen at run time) instead
  of OpenSSL's `SHA256_*`. Digests, the collision error and the "safe"
  digest written on a collision are the C's (rust/difftest/hash_diff.c).
  `git_hash_sha1_*`/`git_hash_sha256_*` are not exported: nothing outside
  `hash.c` called them.
- **zstream: inflating.** Inflate streams use zlib-rs (vendored in
  `rust/vendor/zlib-rs` with a fix that keeps zlib's error messages), not
  the system zlib; deflate streams still use the system zlib, so compressed
  bytes are unchanged. Return codes and messages are zlib's
  (rust/difftest/zstream_diff.c: valid, truncated and corrupted streams).
  zlib-rs allocates its state with Rust's allocator where zlib used
  `malloc`; neither used `git__allocator`.
- **rand: the state** is a `std::sync::Mutex` instead of a `git_mutex`, so
  no shutdown callback is registered to free it (one fewer of the
  runtime's 32 slots). Same sequences for the same seed. Under Miri (tests
  only) the seed is a constant.
- **regexp:** a `git_regexp` is still the `pcre2_code *` (now owned by a
  Rust `Regexp` until handed to C). Semantics are PCRE2's own.
- **date:** the parser reaches the C library's `time`, `gmtime_r`,
  `localtime_r` and `mktime` through a `Clock` (the C's calls, in the same
  order); the safe API also offers a pure-Rust UTC clock with a fixed
  "now".
- **CMake:** with `USE_RUST`, configuring fails unless `USE_SHA1=builtin`,
  `USE_SHA256=OpenSSL`, `USE_REGEX=pcre2` and zlib compression (the Rust
  implements these only). `git2_features.h` gains `GIT_RUST`.
