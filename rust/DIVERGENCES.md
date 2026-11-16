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

## Filesystem layer (WP u3: posix, unix/map, unix/realpath, futils, fs_path, filebuf)

- **posix: `p_open`** is variadic: a C shim in `rust_shims.c` reads the mode
  (with `O_CREAT`, as C) and calls the Rust (`git__p_open`). A NULL path
  fails with `EFAULT` (C passed it to `open`, which does the same).
- **posix: `p_fsync__cnt`** is an `AtomicUsize` that the Rust's `fsync`
  increments atomically; C's inline `p_fsync` (in the header, for the C
  that stays) still increments it non-atomically, as before.
- **posix: `p_write` / `p_read` of more than `isize::MAX` bytes** fail with
  `EINVAL` before any system call (no buffer can be that large).
- **filebuf: the write buffer is zeroed** (`git__calloc`; C used
  `git__malloc`), so that its bytes are always initialized for the Rust
  (`git_filebuf_reserve` hands out bytes that are flushed as they are). No
  visible effect but the cost of the zeroing.
- **filebuf: `git_filebuf_printf`** formats once in its C shim (into an
  8 KiB stack buffer, or a `git__malloc`'d one for longer messages) and
  hands the bytes to the Rust, which places them as C did (in the buffer if
  they fit, after a flush if they then fit, else through
  `git_filebuf_write`). C formatted into the filebuf's buffer, up to twice,
  and allocated only for messages longer than the buffer: under an
  allocation limit, a message longer than 8 KiB may fail (`BUFERR_MEM`)
  where C did not, and the error state is checked after formatting (with
  the same result).
- **filebuf: `resolve_symlink`** reads the link into a stack buffer
  (`GIT_PATH_MAX + 1`; C grew a `git_str` to that size), and the APPEND copy
  and `git_futils_cp` use a heap buffer of `GIT_BUFSIZE_FILEIO` (C: the
  stack), with an out-of-memory error if it cannot be had.
- **futils: `git_futils_readbuffer_fd_full`** reads into a Rust buffer and
  copies the result into the `git_str` (which grows as the C's did).
- **futils: the mkdir path cache** (`cache_pool` + `cache_pathset` of
  `struct git_futils_mkdir_options`, which `checkout.c` owns) is read and
  extended through out-of-line copies of `hashmap_str.h`'s inline
  functions (`git_hashset_str__contains`, `git_hashset_str__add`, shims in
  `rust_shims.c`), so that the set stays exactly the C's.
- **fs_path / futils: arguments that alias the output `git_str`**
  (`git_fs_path_dirname_r(&b, b.ptr)`, `git_fs_path_prettify(&b, b.ptr,
  …)`, …) are copied before the string is written; the results are the
  C's. A `git_str` that borrows its buffer is not written in place
  (`git_fs_path_resolve_relative` and `squash_slashes` copy it into an
  owned buffer first, or leave it), as in the foundation.
- **fs_path: `git_fs_path_walk_up`'s callback** gets the path truncated in
  place in the caller's buffer, as C; for a borrowed `git_str` it gets a
  NUL-terminated copy instead.
- **fs_path: `git_fs_path_str_is_valid_ext`** on a sized `git_str` reads
  exactly `size` bytes (C read `ptr[size]` too, before testing the length).
- **fs_path / futils: NULL path arguments** that C passed to the system
  (and crashed on, or got `EFAULT`) are read as `""` by the exports.
