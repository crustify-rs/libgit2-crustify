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

## u1 (pool, pqueue, utf8, varint, strlist, hashmap, oid, oidarray, parse)

- **oid: `git_oid_tostr_s`'s buffer** is a 65-byte Rust thread-local (no
  destructor), not a `git__malloc`'d `git_tlsdata` slot: it never fails
  (C returned NULL on allocation failure), is not freed by
  `git_libgit2_shutdown`, and stays valid until the thread's next call or
  its end. `git_oid_global_init` does nothing and returns 0.
- **oid: the shortener copies the ids** (C kept pointers into the
  caller's strings, which its contract says must stay unchanged). A caller
  that reuses one buffer for every id got wrong lengths (41) from C and
  gets the right ones from Rust. The shortener lives in Rust's allocator,
  so `cl_alloc_limit` does not make `git_oid_shorten_new`/`_add` fail.
- **oid: `git_oid_cmp`'s magnitude** is the difference of the first
  differing bytes (what glibc's `memcmp` returns); C returned `memcmp`'s
  unspecified value. Only the sign is documented.
- **pool: the page size** comes from `sysconf(_SC_PAGESIZE)` directly (the
  Unix `git__page_size`, whose error it reproduces); a pool initialized
  before `git_pool_global_init` gets it lazily (C used 0, one page per
  allocation). `GitPool::init` (Rust) frees the pool's pages first; the
  export `git_pool_init` overwrites the struct without reading it, as C.
- **hashmap: `dispose`** leaves a leading field (`GitHashMap<K, V, X>`'s
  `extra`, `git_index_entrymap`'s `ignore_case`) as it is; C's `memset`
  zeroed it. Maps that hold owned Rust values drop them on `clear`,
  `dispose` and drop (C's maps hold pointers and free nothing).
- **utf8 / varint: the Rust functions take slices** and return `None`
  where the C would read past the end of the buffer (`git_utf8_iterate`
  with `str_len == 0`, `git_decode_varint` on a truncated varint). The
  exports read exactly what the C read.
