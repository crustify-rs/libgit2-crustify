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
