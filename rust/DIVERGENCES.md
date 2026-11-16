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

- **Infallible Rust allocations on rare paths.** These allocate with Rust's
  allocator (an allocation failure aborts, where C did not allocate or
  failed with -1): `errors::last()` / `Error::last()` (the safe API's copy
  of the message); in the `git_str` exports, the copy of a data argument
  that overlaps the string's own allocation outside its contents
  (pathological: C reads freed memory there, see DEFECTS.md), and the copy
  of in-buffer data for the encoders, `splice`, `join3`, `puts_escaped`,
  `common_prefix`, `crlf_to_lf`/`lf_to_crlf`. `git_error_save`,
  `errors::save`, and `git_str_join_n` (including more than 8 segments)
  fail with the out-of-memory error instead.
- **Callbacks may not read the array they sort.** `git_vector_sort`,
  `git_vector_insert_sorted`, `git_vector_uniq`, `git_vector_bsearch*` and
  `git__tsort*` hold a reference to the array across the comparison, `on_dup`
  and free callbacks, which therefore must not access the vector (C would
  allow reads). No caller in libgit2 does.
- **In-buffer data must be initialized.** `git_str_set(buf, buf->ptr, len)`
  with `len > size` (C's in-place case) requires the bytes up to `len` to
  have been written: the Rust reads contents as bytes, where C reads
  indeterminate `char`s.
- **Lock API (Rust side).** The guards of `GitMutex`/`GitRwLock` are `!Send`;
  `GitMutex::raw_lock` and `GitCond::wait` are `unsafe` (POSIX rules: no
  move or relock while held; one mutex per condition variable).
## net, sortedcache, process (WP u4)

- **net: inputs are read up to their first NUL** (as C reads its strings);
  the safe API (`Url`) takes byte slices and does the same.
- **net: `git_net_url_dup` and `git_net_url_joinpath` with `out == in`**
  are handled as C behaves (dup copies each string onto itself, leaking
  the originals; joinpath disposes `out` first, so it joins onto an empty
  URL), without aliased Rust references.
- **net: `git_net_url_fmt` / `fmt_path` append with the foundation's
  fast path** when the piece fits the buffer: same bytes, same `asize`.
- **sortedcache: the struct is opaque.** Its body moved from
  `sortedcache.h` into `sortedcache.c` (built only with `USE_RUST=OFF`);
  `refdb_fs.c` reads the path through `git_sortedcache_path` instead of
  `refcache->path` (3 lines). The cache is an `Arc` in Rust's allocator
  (`git_sortedcache_new` no longer fails gracefully on out-of-memory for
  the cache itself; the items, the vector and the pages still come from
  `git__allocator`, with the C's page sizes, and fail as C).
- **sortedcache: the key map** is a Rust `HashMap` (Rust's allocator, a
  `try_reserve` before each insert) instead of `git_hashmap_str`; lookups
  are the same. The item vector (`git_vector`, C's growth and sort), the
  arena (`git_pool`'s page sizes and zeroed 8-aligned items) are kept.
- **sortedcache: `git_sortedcache_copy` sorts the copy** before returning
  it; C left it unsorted until its first sorted read or unlock, which
  sorts it the same way. This keeps reads under a read lock free of
  writes.
- **sortedcache: `git_sortedcache_entrycount` without the lock** reads an
  atomic copy of the count (C read the vector's length racily).
- **process: `git_process` is a Rust object** (`Box`, Rust's allocator);
  its arguments and environment are `CString`s (Rust's allocator, with
  `try_reserve` where C checked allocations; `CString`'s own allocation
  is infallible). The `execve` pointer arrays are built in the parent
  before `fork`.
- **process: `git_process_result_msg`** formats with Rust's formatting
  (same text).
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
