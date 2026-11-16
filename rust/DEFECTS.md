# Defects in the C, and what the Rust does instead

One entry per defect: component, C location, the defect, an input that
triggers it, and the Rust's defined behaviour.

## Foundation (src/util, buf, strarray)

### str: `git_str_splice` writes through a stale pointer when it grows
- C: `src/util/str.c`, `git_str_splice`: `splice_loc = buf->ptr + where` is
  computed before `ENSURE_SIZE`, which may `realloc` the buffer.
- Trigger: a splice whose result does not fit `asize`, e.g. `"hello"`
  (asize 8) spliced at 1 with 10 bytes inserted: use-after-free.
- Rust: the position is computed after growing; the result is the intended
  splice.

### str: data arguments that point into the buffer itself
- C: `git_str_put`, `git_str_set`, `git_str_splice`, the encoders, … copy
  from `data` after `ENSURE_SIZE`; if `data` lies within the buffer and the
  buffer is reallocated, they read freed memory.
- Trigger: `git_str_put(&buf, buf.ptr, buf.size)` when the result exceeds
  `asize`.
- Rust: such data is copied in place (`copy_within`) after growing, or
  copied out first; the result is the intended one (the buffer's old
  contents). (`git_str_join`'s `str_a` and `git_str_join_n`'s segments,
  which C handles explicitly, behave as in C.)

### str: `git_str_try_grow`: size rounding wraps to 0
- C: `new_size = (new_size + 7) & ~7` wraps to 0 for a target within 7 of
  `SIZE_MAX`; then `realloc(ptr, 0)`, `size = asize - 1 = SIZE_MAX` and
  `ptr[SIZE_MAX] = '\0'`.
- Trigger: `git_str_grow_by(&fresh, SIZE_MAX)` or `git_str_grow(&s, SIZE_MAX)`.
- Rust: treated as the overflow it is: out-of-memory error, and the string
  becomes out of memory (with `mark_oom`).

### str: `git_str_grow_by` overflow leaks the buffer
- C: on `size + additional` overflow, `buf->ptr = git_str__oom` without
  freeing the old buffer.
- Rust: the owned buffer is freed before the string becomes out of memory.

### str: out-of-memory paths free memory the string does not own
- C: `git_str_try_grow` (with `mark_oom`) frees `ptr` whenever it is not
  NULL or `git_str__initstr`, including a borrowed pointer of size 0
  (`GIT_STR_INIT_CONST(p, 0)`); `git_str_vprintf` frees `ptr` on a
  `vsnprintf` failure even when it is `git_str__initstr`.
- Trigger: allocation failure on such a string; an encoding error in
  `vsnprintf`.
- Rust: only an owned buffer (`asize > 0`) is ever freed.

### str: `git_str_join_n` sizes and copies an inner segment differently
- C: the first pass measures a segment that points into the buffer with
  `strlen`, the second copies `original_size - offset` bytes; with a NUL
  inside the buffer the second pass writes past the allocation.
- Rust: both passes use `original_size - offset`.

### str: `git_str_unquote` reads past the contents
- C: for `"\0"` (an octal escape cut short, `j == size - 2` but not
  `size - 3`) the error message reads `ptr[size]`, the NUL after the
  contents — beyond the memory of a borrowed string.
- Rust: reads that byte as 0, so the message is the C's (`truncated quoted
  character \0"` and a NUL byte); a borrowed string is copied first.

### str: `git_str_consume` and friends write into borrowed memory
- C: `git_str_consume`, `git_str_unquote`, `git_str_unescape` write through
  `ptr` even when the string borrows it (`GIT_STR_INIT_CONST` of a string
  literal: a write to read-only memory).
- Rust: never writes a borrowed buffer: `consume` moves the view;
  `unquote`/`unescape` copy into an owned buffer first.

### str: `git_str_common_prefix` with a NULL string after the first
- C: dereferences it. Rust: treats it as empty (the prefix becomes empty).

### errors: the message buffer's failure leaves a dangling error
- C: `errors.c` keeps the message in a `git_str`; when growing it fails,
  `git_str_try_grow` frees it and marks it out of memory, but `last` still
  points at `threadstate->error`, whose `message` is the freed buffer. A
  `vsnprintf` failure does the same, and every later message on the thread
  is then lost (the buffer stays out of memory).
- Trigger: allocation failure while setting a message; a format with an
  invalid multibyte sequence (`%ls`).
- Rust: the error becomes the static out-of-memory one; later errors are
  set normally.

### errors: `git_error_save` leaks its copy
- C: when `git__strdup` of the message fails, the `git_error` copy is not
  freed. Rust: it is freed.

### tsort: `binsearch` reads past the run
- C: `src/util/tsort.c`, `binsearch`: the scans over equal elements
  (`while (cmp(x, dst[i]) == 0) i++`) are unbounded; with an inconsistent
  comparator they read past the array.
- Rust: they stop at the run's end.

### util: `git__strncasecmp` with `sz == 0`
- C: the `do … while (--sz && …)` loop wraps `sz` past 0 and compares the
  whole strings.
- Rust: kept as C (the result is defined, and callers may depend on it).

### util: `git__substrdup` with `n == SIZE_MAX`
- C: `n + 1` wraps to 0, then `memcpy` of `SIZE_MAX` bytes.
- Rust: out-of-memory error, NULL.

### vector: `git_vector_set` at `SIZE_MAX`
- C: `position + 1` wraps to 0, the vector is not grown, and
  `contents[SIZE_MAX]` is written.
- Rust: out-of-memory error.

### vector: `git_vector_dup` failure leaves an inconsistent vector
- C: sets `v->length = src->length` before allocating; on failure `v` has
  that length and NULL `contents`.
- Rust: length 0 (the other fields as C).

### vector: `git_vector_verify_sorted` without a comparison function
- C: calls NULL when the vector is flagged sorted and has two elements.
- Rust: returns 0.

### buf: `git_buf_dispose` frees `git_str__oom`
- C: frees any `ptr` other than `git_str__initstr`, including the
  out-of-memory sentinel (reachable through `git_buf_fromstr` of an
  out-of-memory string).
- Rust: never frees a sentinel.

### runtime: `git_runtime_init` with a NULL init function
- C: calls it. Rust: stops at the first NULL entry.

## Filesystem layer (WP u3: posix, unix/map, unix/realpath, futils, fs_path, filebuf)

### posix: `p_getcwd` writes one byte past the buffer
- C: `src/util/posix.c`, `p_getcwd` calls `git_fs_path_string_to_dir(buffer,
  size)` with the buffer's size, but `git_fs_path_string_to_dir` appends the
  slash when `strlen < size` and writes the NUL at `path[strlen + 1]`, i.e.
  at `buffer[size]` when the directory is `size - 1` bytes long.
- Trigger: `p_getcwd(buf, strlen(cwd) + 1)`.
- Rust: the slash is appended only when it and the NUL fit the buffer
  (`git_fs_path_string_to_dir` itself keeps the C's contract, which its
  test relies on: `size + 1` writable bytes).

### fs_path: `git_fs_path_diriter_init` with an empty path dereferences NULL
- C: `git_fs_path_diriter_init` `memset`s the iterator (so `path.ptr` is
  NULL), `git_str_puts(&path, "")` allocates nothing, and
  `path_trim_slashes` calls `git_fs_path_root(NULL)`.
- Trigger: `git_fs_path_diriter_init(&it, "", 0)` (or `git_fs_path_dirload`
  of `""`).
- Rust: fails with `could not open directory ''` (`GIT_ERROR_FILESYSTEM`),
  -1, as the C meant to.

### fs_path: `git_fs_path_diriter_next` on an iterator that was never opened
- C: `readdir(NULL)`.
- Trigger: `GIT_FS_PATH_DIRITER_INIT`, then `git_fs_path_diriter_next`.
- Rust: -1 with `unrecoverable internal error: 'diriter->dir'`.

### fs_path: `git_fs_path_dirload` leaks on failure
- C: on `GIT_ASSERT(name_len > prefix_len)` and on `GIT_ERROR_CHECK_ALLOC`
  it returns without `git_fs_path_diriter_free` (the `DIR` and the path
  leak); when `git_vector_insert` fails the copy of the name leaks.
- Rust: frees the iterator, and the copy, on every path.

### fs_path: `git_fs_path_direach` never returns `GIT_ENOTFOUND`
- C: it tests `errno == ENOENT` after `git_error_set(GIT_ERROR_OS, …)`,
  which reset `errno` to 0: a missing directory gives -1.
- Rust: the same (kept: callers may depend on -1).

### fs_path: `ensure_error_set` is dead code
- C: `git_fs_path_direach` and `git_fs_path_walk_up` set "filesystem callback
  returned %d" only when `git_error_last()` is NULL or has no message,
  which it never is since `git_error_last` returns the static "no error".
- Rust: does not set it either.

### fs_path: `git_fs_path_apply_relative` returns 1 on failure
- C: `return git_str_joinpath(…) || git_fs_path_resolve_relative(…);` is 0
  or 1; `filebuf.c`'s `resolve_symlink` tests `< 0`, so a failure (a symlink
  target with `..` above the root) is ignored and the loop goes on.
- Rust: the same (kept).

### fs_path: `git_fs_path_diriter_filename` of an entry of `/`
- C: the name starts at `parent_len + 1`, which skips its first byte when
  the parent ends with a slash (only the root keeps one).
- Rust: the same (kept).

### futils: `git_futils_readbuffer_fd_full` reads every block at the start
- C: `p_read(fd, buf->ptr, blocksize)` in the loop: for more than 10240
  bytes, each block overwrites the previous ones and the result (of the
  total size) ends with uninitialized bytes.
- Trigger: `git_futils_readbuffer_fd_full` of a pipe or file of more than
  10240 bytes (`git hash-object --stdin` in the CLI).
- Rust: the blocks are kept in order: the result is the input.

### filebuf: `git_filebuf_open_withsize` leaks on a late allocation failure
- C: when `git__malloc` of `z_buf` (or of the lock path) fails it returns
  -1 without cleaning up; the deflate stream is never ended (cleanup ends it
  only when `z_buf` is set) and the other buffers leak unless the caller
  calls `git_filebuf_cleanup`.
- Rust: ends the stream and cleans up (a later `git_filebuf_cleanup` is a
  no-op).

### filebuf: `git_filebuf_commit` leaks the descriptor when `fsync` fails
- C: `fd_is_open` is cleared before the `fsync`, so cleanup does not close.
- Rust: closes it.

### filebuf: a filebuf that was never opened
- C: `git_filebuf_write`, `_flush`, `_reserve` and `_printf` on
  `GIT_FILEBUF_INIT` call the NULL `write` function.
- Rust: -1 with `unrecoverable internal error: 'file->write'`.

### filebuf: `git_filebuf_printf` with `GIT_FILEBUF_DO_NOT_BUFFER`
- C: formats into `buffer + buf_pos` with `buffer` NULL and a size of
  `buf_size` (a write through NULL).
- Rust: the message is written directly, as `git_filebuf_write` would.
