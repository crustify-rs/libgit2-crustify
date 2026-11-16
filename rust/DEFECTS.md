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

### util: `git__substrdup` with `n > PTRDIFF_MAX`
- C: `n + 1` wraps to 0 for `SIZE_MAX` (then `memcpy` of `SIZE_MAX` bytes);
  any `n` above `PTRDIFF_MAX` cannot describe readable memory.
- Rust: the export checks `n` before forming any slice and fails with the
  out-of-memory error, NULL. (In general, `export::bytes` refuses a length
  above `isize::MAX` with a panic, which the barrier turns into an internal
  error, rather than form an invalid slice.)

### str: `git_buf_tostr` of a `git_buf` that holds an out-of-memory string
- C: `git_buf_sanitize` writes `ptr[0] = '\0'` (into `git_str__oom`) and the
  `git_str` stays out of memory. (Not a C defect; recorded because the first
  port got it wrong, review finding F-1.)
- Rust: the same; a `git_buf` with `git_str__initstr` and `reserved > 0`
  becomes an empty string with `asize = 0`.

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

## net, sortedcache, process (WP u4)

### net: NULL fields dereferenced
- C: `src/util/net.c`: `git_net_url_is_ipv6` (`strchr(url->host)`),
  `git_net_url_is_default_port` (`strcmp(url->port, …)` when the scheme has
  a default port), `matches_pattern` (`git__strlcmp(url->host|port, …)`,
  `strlen(url->host)`) read NULL fields.
- Trigger: a URL without a host (`git_net_url_parse(&u, "relative")`) or
  without a port (`u.port = NULL`), then those calls.
- Rust: a NULL host has no `:` (`is_ipv6` false); a NULL port is not the
  default; patterns see NULL as `""` (so only `*`, `.` / `*.` with an empty
  domain, match a missing host; a pattern with a port never matches a
  missing port).

### net: `git_net_url_apply_redirect` with a location without scheme or host
- C: `strcmp(original->scheme, tmp.scheme)` and
  `git__strcasecmp(original->host, tmp.host)` with `tmp.scheme` / `tmp.host`
  NULL, after parsing a relative location that does not start with `/`
  (e.g. `"foo/bar"`).
- Rust: a NULL scheme or host differs from the original's: the redirect is
  refused with the C's message, the NULL shown as glibc's printf shows it
  (`cannot redirect from 'http' to '(null)'`).

### net: `remove_service_suffix` with a NULL path or query
- C: `strlen(url->path)` with a NULL path (a redirect to a URL without a
  path, e.g. `"http:"`); `git__strcmp(url->query, …)` with a NULL query
  when the suffix has a query (`"/info/refs?service=…"`) and the path ends
  with the suffix's path part; with an empty path and an empty suffix,
  `url->path[1] = '\0'` writes past the string.
- Rust: a NULL path leaves the URL unchanged; a NULL query does not match
  the suffix's query; the truncation never writes past the string.

### sortedcache: none found
The port keeps C's semantics; its hash map is a Rust map (see
DIVERGENCES.md).

### process: the child calls `exit(0)` when `chdir` or `execve` fails
- C: `src/util/unix/process.c`, `git_process_start`: after a failed
  `chdir` / `execve` the forked child calls `exit(0)`, which is not
  async-signal-safe: it runs the parent's `atexit` handlers and flushes
  the parent's stdio buffers a second time (duplicated output; in a
  multi-threaded parent it may deadlock on a lock held at the fork).
- Trigger: `git_process_start` of a program `execve` rejects
  (`ENOEXEC`), or with a `cwd` that does not exist, while stdout has
  unflushed output.
- Rust: `_exit(0)` (same status, no handlers, no flush). Between `fork`
  and `execve` the child only makes async-signal-safe system calls on
  data prepared before the fork.

### process: `waitpid(0)` after a failed start
- C: on a failed `read_status`, `waitpid(process->pid, …)` with
  `process->pid` still 0: it waits for *any* child of the process group
  (it may reap an unrelated child, or block on another child).
- Rust: waits for the child just forked.

### process: the status pipe leaks; closed descriptors stay named
- C: on success the read end of the status pipe is stored in
  `process->status` (a `git_process_result_status`) and never closed:
  one descriptor leaks per started process. On a failed start the parent
  ends of the stdio pipes are closed but stay in `child_in` / `child_out` /
  `child_err`, so a later `git_process_close` closes those numbers again
  (possibly another file's).
- Rust: the status pipe is closed once read; on failure the fields are
  reset to -1.

### process: `try_read_status` counts `-1` on `EINTR`
- C: `read_len += ret` also when `read` returned -1 with `EINTR`/`EAGAIN`,
  so the count goes back by one (or wraps to `SIZE_MAX` from 0, and the
  status buffer is then read uninitialized). The function name's length is
  trusted even when fewer bytes were read (`fn[fn_len] = '\0'` after
  uninitialized bytes).
- Trigger: a signal delivered while the parent waits for the child's
  status.
- Rust: retries without counting; terminates the name at the bytes read.

### net: `git_net_hostname_matches_cert` never matches a pattern byte >= 0x80
- C: `char c = git__tolower(*pattern++)` keeps a signed `char` (a byte
  >= 0x80 is negative), compared with `git__tolower(*hostname++)`, an
  `int` in 0..255 (`tolower((unsigned char)…)` on Unix): never equal.
  After a `*` the search for the next pattern byte compares two `char`s,
  but the match then resumes at that byte and fails the same way. A `*`
  still spans host bytes >= 0x80.
- Trigger: `("\x80", "\x80")` and `("a\xc3", "*\xc3")` are false;
  `("\xc3\x80b", "*b")` is true.
- Rust: kept as C (found by the random section of `net_diff.c`).

### process: `git_process_free` leaks the pipes after `git_process_wait`
- C: `git_process_free` closes the pipes only `if (process->pid)`;
  `git_process_wait` sets `pid = 0`, so `new`, `start`, `wait`, `free`
  leaks the three pipe descriptors unless the caller calls
  `git_process_close` first.
- Rust: the pipes are always closed on free (closing an already closed
  pipe, -1, does nothing); the safe `Process` closes them on drop.

### process: pipes are not close-on-exec (atomically, or at all)
- C: the capture pipes are made with `pipe` (never close-on-exec), and the
  status pipe gets `FD_CLOEXEC` with a separate `fcntl`. In a threaded
  parent, a program another thread spawns meanwhile inherits them: it
  holds our `in[1]` (our child never sees EOF on stdin after
  `git_process_close_in`), or the status pipe's write end
  (`git_process_start` blocks until that program exits). The child's own
  program also inherited the original pipe ends besides 0/1/2.
- Rust: all four pipes are made with `pipe2(O_CLOEXEC)`; the child's
  `dup2` onto 0/1/2 clears the flag (and `fcntl(F_SETFD, 0)` when the
  descriptor already is 0/1/2). The child's program gets 0/1/2 and what
  the parent had open without close-on-exec, no pipe ends.

### process: `git_process_new` leaks on a failed `cwd` copy
- C: returns -1 without freeing the process when `git__strdup(opts->cwd)`
  fails. Rust: frees it.
## hash, zstream, rand, regexp, wildmatch, date (WP u2)

### hash: sha1dc reads input blocks through misaligned `uint32_t` pointers
- C: `src/util/hash/sha1dc/sha1.c:391` (`SHA1DCUpdate`, with
  `SHA1DC_ALLOW_UNALIGNED_ACCESS`) casts the input to `const uint32_t *`
  at any alignment: undefined behaviour (UBSan reports it).
- Trigger: any `git_hash_update` with an input not 4-byte aligned.
- Rust: the `sha1collisiondetection` crate copies an unaligned block into
  its buffer first; same digests.

### hash: a NULL context is dereferenced
- C: `src/util/hash.c`, `git_hash_ctx_init`/`git_hash_init`/
  `git_hash_update`/`git_hash_final` read or write `ctx->algorithm` before
  the backends' `GIT_ASSERT_ARG(ctx)`.
- Rust: -1 with `invalid argument: 'ctx'` (class `GIT_ERROR_INVALID`).

### zstream: output that fills the buffer exactly is left without its NUL
- C: `src/util/zstream.c`, `zstream_buf` writes up to `asize - size` bytes
  (the NUL's byte included) and terminates only `if (out->size <
  out->asize)`; a `git_str` with `size == asize` has no NUL, and
  `git_str_cstr` and friends read past its allocation.
- Trigger: `git_zstream_deflatebuf(&s, "", 0)` (8 bytes into an 8-byte
  buffer), or inflating `zlib.compress(b"a" * 16)` (16 bytes into 16).
- Rust: the buffer is grown by one byte and terminated (`asize` then
  differs from the C's in that case only).

### date: signed `int` overflows
- C: `src/util/date.c`: `approxidate_alpha` computes `*num - 1`,
  `diff += 7*n`, `diff * 24 * 60 * 60`, `tm->tm_mon - *num`,
  `tm->tm_year -= *num` in `int`; `match_tz` `hour * 60 + min`;
  `git_date_rfc2822_fmt` `offset * 60` and `gmt.tm_year + 1900`. Each
  overflows (undefined behaviour) on large inputs.
- Trigger: `git_date_parse(&t, "2147483647 fridays ago")`,
  `"4294967296 days ago"`, `git_date_rfc2822_fmt(&s, 0, INT_MAX)`.
- Rust: two's complement wrapping, which is what the C build computes
  (rust/difftest/date_diff.c gives the same output for these inputs).

### date: an object-header timestamp may not start with 9
- C: `match_object_header_date` rejects `*date < '0' || '9' <= *date`, so
  `"@9 +0000"` is not read as a timestamp (it goes on to approxidate).
- Rust: kept as the C (the result is defined, and callers may depend on it).

### date: "N months ago" loops N/12 times
- C: `approxidate_alpha` adds 12 to the month (and takes a year) in a loop
  until it is not negative: `"2147483647 months ago"` iterates 178 million
  times.
- Rust: the same result, computed in one step.

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
## u1 (pool, pqueue, utf8, varint, strlist, hashmap, oid, oidarray, parse)

### pool: `alloc_size` overflows
- C: `src/util/pool.c`, `alloc_size`: `item_size * count` and
  `(count + 7) & ~7` wrap; `git_pool_strndup(pool, s, SIZE_MAX - 1)` passes
  the `n == SIZE_MAX` guard, `n + 1` rounds to 0, and `memcpy` copies
  `SIZE_MAX - 1` bytes into a 0-byte block.
- Trigger: `git_pool_malloc(pool_of_8_byte_items, SIZE_MAX / 4)`.
- Rust: checked arithmetic; NULL with the out-of-memory error.

### oid: `git_oid_allocfmt` leaks for an unknown type
- C: allocates `hex_size + 1` bytes before checking `hex_size`, and returns
  NULL without freeing them when it is 0.
- Rust: checks first; allocates nothing; NULL.

### oid: `git_oid_shorten_add` writes out of bounds
- C: when a leaf is split, the next digit of its stored tail is converted
  with `git__fromhex` unchecked; a non-hex byte (the tail of an id that
  had an invalid digit after the point where it became a leaf, or the NUL
  of an id shorter than 40 digits) gives -1, and `children[-1]` is
  written (heap overflow). The tail is also a pointer into the caller's
  string, read later.
- Trigger: add `"ab"`, then `"abcd"`; or add `"12x…"`, then `"12…"`.
- Rust: a tail is a bounded slice of the id (at most 40 bytes, borrowed as
  in C); a non-hex or missing tail digit fails the add with `unable to
  shorten OID - invalid hex value` (class INVALID), the trie as C would
  leave it before the bad write.

### oid: `git_oid_shorten_add` leaves a leaf behind a positive link
- C: `src/libgit2/oid.c`, `git_oid_shorten_add`: when the walk reaches a
  leaf it makes the parent's link positive, and converts the leaf only on
  the next iteration, after validating that iteration's digit. If that
  digit is not hex the add fails and the node keeps its `tail` pointer
  while its link says it is a normal node. Later walks through it read the
  pointer's bytes as `children[0..3]`: a digit 0..3 follows pointer bits
  as a node index (out of bounds); 4..f read 0 (zeroed memory).
- Trigger: add `"abcdef…01"`, then `"ax"`, then `"a0…"`.
- Rust: the failing add empties that node (C's view of it for digits
  4..f, also used for 0..3); the leaf's id is dropped, as in C. (Review
  u1-r1, U1-1: the first port read the union's tail as children here.)

### oid: `resize_trie` loses the trie on allocation failure
- C: `self->nodes = git__reallocarray(self->nodes, …)` overwrites the
  pointer with NULL (leak; the next add dereferences NULL).
- Rust: the out-of-memory error; the set is unchanged.

### strlist: `git_strlist_copy*` leak on allocation failure
- C: returns -1 from inside the loop without freeing the copies made so
  far or the array.
- Rust: frees them, then the out-of-memory error.

### hashmap: a set's `iterate` does not advance
- C: `src/util/hashmap.h`, `GIT_HASHSET_FUNCTIONS`' `name##_iterate`
  returns the entry at `*iter` without incrementing it, so repeated calls
  (and `name##_foreach`) return the first live key forever. No C caller
  iterates a set today; the C macro stays as it is.
- Rust: `GitHashMap::iterate_keys` advances past the key, as the map's
  `iterate` does.

### hashmap: shrinking assigns a failed `realloc`
- C: `__resize`'s shrink step assigns `git__reallocarray`'s result to
  `keys`/`vals` even when it is NULL (leak, then NULL dereference).
- Rust: keeps the larger arrays (still valid) when shrinking fails.

### parse: `git_parse_advance_chars` past the line
- C: subtracts `char_cnt` from `line_len` and `remain_len` unchecked; more
  than `line_len` wraps them and later reads run past the content.
- Rust: advances by at most `line_len`.

## repository, libgit2, settings (WP l-repo)

### repository: `git_repository_open_bare` and `git_repository_wrap_odb` leak on failure
- C: `open_bare` returns the error of `read_repository_format` without
  freeing the repository it allocated; `wrap_odb` leaks it when its
  `GIT_ASSERT(git_oid_type_is_valid(odb->options.oid_type))` fails.
- Trigger: a bare repository whose config has `repositoryformatversion = 2`.
- Rust: the repository is freed; the error is the C's.

### repository: `git_repository_odb__weakptr` leaks an odb whose backends fail
- C: the odb is owned by the repository (`GIT_REFCOUNT_OWN`) before
  `git_odb__set_caps` / `git_odb__add_default_backends`; on their failure
  `git_odb_free` decrements the count but frees only an unowned object.
- Rust: the owner is cleared first, so the odb is freed.

### repository: `repo_local_config` frees a weak pointer
- C: `git_repository_reinit_filesystem` calls `git_config_free` on the
  pointer from `git_repository_config__weakptr`, which holds no reference.
  The count ends one too low: a configuration the user holds (from
  `git_repository_config`) is freed under it when the repository releases
  its own.
- Rust: the weak pointer is not released.

### repository: `git_repository_commit_parents` frees commits with `git__free`
- C: on a merge-head failure the commits looked up so far are freed with
  `git__free`, not `git_commit_free`: cached objects are freed under the
  object cache (use after free, double free).
- Trigger: a `MERGE_HEAD` naming a missing commit after a valid one.
- Rust: they are released with `git_object_free`.

### repository: `git_repository__item_path` reads past its table
- C: `items[item]` is not bounds-checked; `item >= GIT_REPOSITORY_ITEM__LAST`
  reads beyond the static array.
- Rust: such an item fails with "invalid item directory" (class
  `INVALID`), -1.

### repository: `git_repository_set_namespace` with its own namespace
- C: frees `repo->namespace`, then `git__strdup`s the argument, which is
  that freed string.
- Trigger: `git_repository_set_namespace(repo, git_repository_get_namespace(repo))`.
- Rust: the argument is copied first; the namespace is unchanged.

### repository: `git_repository_initialbranch` with a valueless `init.defaultBranch`
- C: dereferences `entry->value`, NULL for `[init] defaultBranch` with no
  `=`.
- Rust: treats it as empty, so the default `master` is used.

### repository: NULL arguments the C dereferences
- C: most of `repository.c`'s functions without `GIT_ASSERT_ARG` crash on
  a NULL repository (`git_repository_config__weakptr`, `_message`,
  `_state_cleanup`, `_set_namespace`, `_head_detached`, …), and the
  `git_libgit2_opts` getters on a NULL out-pointer; `GIT_OPT_SET_SSL_CIPHERS`
  with NULL calls `strdup(NULL)`.
- Rust: they fail with `invalid argument: 'repo'` (or `'out'`), -1 (NULL
  for the string getters); `GIT_OPT_SET_SSL_CIPHERS` with NULL clears the
  setting.

### repository: `git_repository__extensions` leaks on allocation failure
- C: returns -1 with the strings copied so far and the vector leaked.
- Rust: frees them.
## odb, odb_loose, odb_mempack, delta, cache (WP l-odb)

### odb: `git_odb_foreach` calls a NULL `foreach` slot
- C: `src/libgit2/odb.c` (`git_odb_foreach`) calls `b->foreach` of every
  backend without checking it; a backend without one (the mempack backend
  has none, and it is optional in `git2/sys/odb_backend.h`) crashes.
- Trigger: `git_odb_foreach` on an odb with a mempack backend.
- Rust: backends without `foreach` are skipped.

### odb: `git_odb_write` reads the odb before checking it
- C: `git_odb_write` reads `db->options.oid_type` before
  `GIT_ASSERT_ARG(db)`; a NULL `db` crashes.
- Rust: -1 with `invalid argument: 'db'`.

### odb: `git_odb__add_default_backends` leaks a backend it fails to add
- C: when `add_backend_internal` fails for the new loose (or pack)
  backend, the backend is not freed.
- Trigger: an allocation failure in `add_backend_internal`.
- Rust: the backend is freed through its `free` slot; the result is -1, as C.

### odb: streams without a hash context or a slot
- C: `git_odb_stream_write` hashes into `stream->hash_ctx`, NULL for a read
  stream or a stream a backend returned with `GIT_PASSTHROUGH`;
  `git_odb_stream_read` / `git_odb_stream_finalize_write` call `read` /
  `finalize_write` unchecked (NULL for write-only / read-only streams).
- Rust: no hashing without a context; a missing slot is an error
  (`stream does not support reading` / `writing`, class `GIT_ERROR_ODB`).

### cache: the size limit of an out-of-range type
- C: `cache_should_store` indexes `git_cache__max_object_size[8]` with the
  object's type, out of bounds for a type outside 0..7.
- Trigger: a backend that returns such a type, with strict hash
  verification off (`GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION`).
- Rust: such objects are not cached.

### odb_loose: an empty objects directory
- C: `git_odb__backend_loose` reads `objects_dir[objects_dirlen - 1]` with
  `objects_dirlen == 0`.
- Rust: appends the `/` (the directory is `/`).

### odb_loose: a pack-like object whose body is shorter than its header says
- C: `read_loose_packlike` returns `hdr.size` bytes of a buffer whose tail
  past what was inflated is uninitialized.
- Trigger: a pack-like loose object (first byte not a zlib header) with a
  short deflated body, strict hash verification off.
- Rust: the tail is zero (as `read_loose_standard`'s zero padding).

### odb_loose: an id of another type than the backend's
- C: `object_file_name` grows the path for the backend's hex size, then
  `git_oid_pathfmt` writes the id's (64 digits for a SHA-256 id in a SHA-1
  backend: a heap overflow when the string has no slack).
- Rust: the path has the backend's hex size, the digits taken from the id.

### odb_mempack: `impl__write` leaks the object when the map insert fails
- C: on `git_odb_mempack_oidmap_put` failure the `memobject` is not freed.
- Rust: freed (the error is returned as C).

### delta: `git_delta_read_header_fromstream` adds a negative read status to the length
`delta.c:git_delta_read_header_fromstream` does `len += read` for any
non-zero `read` that is not `GIT_EBUFS`, so an error of
`git_packfile_stream_read` (-1, "error reading from the zlib stream") makes
`len` wrap, and `hdr_sz` then parses up to `SIZE_MAX` bytes past the
16-byte stack buffer (uninitialized and out-of-bounds reads). Input: a pack
whose delta object's zlib data is corrupt. The Rust returns -1 with the
stream's error. A `GIT_EBUFS` that never clears (a window that cannot be
opened) still loops forever, as in C: the stream gives no way to tell it from
a transient one.

### delta: `git_delta_create_from_index` with a NULL index
C dereferences the index (it returns early only for an empty target);
`git_delta_index_init` returns a NULL index for an empty source, so
`git_delta_create_from_index(…, NULL, trg, n>0, …)` crashes. The Rust fails
with `invalid argument: 'index'` (-1, NULL output).

### delta: the output buffer's size wraps for deltas above ~1.4 GiB
`git_delta_create_from_index` grows its buffer with `bufsize * 3 / 2` in
`unsigned int`; past 2^32/3 bytes this wraps and `git__realloc` shrinks the
buffer, which the next opcode overflows. The Rust fails with the
out-of-memory error instead.
## config (WP l-config: config, config_file, config_parse, config_list, config_cache, config_mem, config_snapshot)

### config_file: `onbranch:` leaves the result unset for a `HEAD` outside `refs/heads/`
- C: `src/libgit2/config_file.c`, `conditional_match_onbranch`: when `HEAD`
  is symbolic but its target does not start with `refs/heads/`, it returns
  0 without writing `*matches`; `parse_conditional_include`'s `matches` is
  uninitialized, so the include is read or not at random.
- Trigger: `HEAD` → `refs/remotes/origin/main` and an
  `[includeIf "onbranch:*"]` section.
- Rust: no match (the include is not read).

### config: `get_entry` hands out an uninitialized entry when a backend fails
- C: `get_entry` sets `*out = &be->entry` for any result but
  `GIT_ENOTFOUND`, with `be` uninitialized when the backend failed; with
  `GET_NO_ERRORS` (`git_config__lookup_entry(..., true)`,
  `git_config__get_*_force`) the error is cleared and the caller reads and
  frees that pointer.
- Trigger: a file backend whose refresh fails (e.g. an include that cannot
  be parsed after the file changed), then `git_config__get_bool_force`.
- Rust: `*out` is NULL; the forced getters return their fallback.

### config_file, config: a pattern matched against a value-less variable
- C: `write_on_variable` (`git_config_set_multivar`,
  `git_config_delete_multivar`) and `multivar_iter_next`
  (`git_config_multivar_iterator_new`, `git_config_get_multivar_foreach`)
  call `git_regexp_match(preg, value)` with `value` NULL for a bare
  variable (`[a]\n\tb\n`): `strlen(NULL)`, a crash.
- Trigger: `[a]\n\tb\n` and `git_config_set_multivar(cfg, "a.b", ".*", "x")`
  (the differential test avoids these writes for that reason).
- Rust: a missing value is matched as `""` (so `^$` matches it).

### config: NULL slots of a C backend or iterator are called
- C: `config.c` calls `backend->get`, `->iterator`, `->snapshot`, …,
  and `iter->next`/`->free` without checking them; a user's backend that
  leaves one NULL crashes.
- Rust: the call fails with `the config backend does not implement '<slot>'`
  (class `GIT_ERROR_CONFIG`, -1); a NULL `free` is skipped.

### config: a NULL foreach callback
- C: `git_config_foreach_match` and `git_config_get_multivar_foreach`
  call `cb` unchecked (`git_config_backend_foreach_match` asserts it):
  crash at the first entry.
- Rust: `invalid argument: 'cb'` at the first entry (an empty config
  still succeeds, as in C).

### config_list, config_snapshot: an entry without a backend type
- C: `git_config_list_dup_entry` interns `entry->backend_type`
  unconditionally: a user backend's entry with `backend_type` NULL makes
  `git_config_list_add_string` hash NULL (crash), e.g. in a snapshot.
- Rust: NULL is kept NULL.

### config_mem: a `-c` value without `=`
- C: `parse_values` computes `strchr(value, '=') - value` with a NULL
  `strchr` result (undefined pointer arithmetic); in practice the name is
  the whole string (`git__strndup` stops at its NUL) and the value NULL.
- Rust: that result, without the arithmetic.

### config: `git_config_set_writeorder` does not reorder the writers
- C: it updates each backend's `write_order` and calls
  `git_vector_sort(&config->writers)`, which does nothing because the
  writers vector is flagged sorted (`git_vector_insert_sorted` keeps the
  flag); later inserts binary-search the unsorted vector.
- Trigger: two writable levels and `git_config_set_writeorder` with both,
  in an order other than by level.
- Rust: as C (same writer chosen; no libgit2 caller depends on the order:
  `repository.c` passes one level).

### config, config_file, config_snapshot: use before open, after a failed open, after free
- C: a file or snapshot backend's `get` before `open` (or after a failed
  `open`) dereferences its NULL list; an iterator of `git_config_iterator_new`
  keeps a bare `git_config *` and reads freed vectors once the config is
  freed.
- Rust: the backend has no entries (`GIT_ENOTFOUND`); the iterator holds
  the backends it iterates alive.

### config: `GIT_CONFIGMAP_STRING` with a NULL `str_match`
- C: `git_config_lookup_map_value` calls `strcasecmp(value, m->str_match)`
  unchecked: a user's map with a NULL `str_match` crashes.
- Rust: a NULL `str_match` is matched as `""`.

### config_parse: the variable callback is optional but its strings leak
- C: `git_config_parse` with a NULL `on_variable` leaks each variable's
  name and value. (No caller passes NULL.)
- Rust: nothing is allocated for the callback.
## index, tree-cache, index_map (WP l-index)

### index_map: a removal after a case change leaves a dangling key
- C: `git_index_entrymap_remove` removes the key *equal* to the entry.
  After `git_index__set_ignore_case` (through `git_index_set_caps`) turned
  a case-sensitive map holding `x/a` and `x/A` into a case-insensitive one
  (without rehashing), removing `x/A` can remove `x/a`'s key and leave
  `x/A`'s; the entry is then freed, and the next probe of that bucket
  chain (`git_index_get_bypath`, another removal) reads freed memory.
- Trigger: add `x/a` then `x/A`, `git_index_set_caps(ix, IGNORE_CASE)`,
  `git_index_remove_directory(ix, "x", 0)`, `git_index_get_bypath(ix,
  "x/a", 0)`.
- Rust: removes the equal key as C, then also the entry's own key if it is
  still there; lookups never see a freed entry.

### index: an entry freed while the entries vector holds it
- C: `index_insert` frees the entry when `git_index_entrymap_put` fails
  after `git_vector_insert_sorted` put it in `index->entries`; so does
  `parse_index`. The vector keeps a dangling pointer.
- Trigger: an allocation failure growing the map.
- Rust: takes the entry out of the vector before freeing it.

### index: `git_index_read_index` frees entries an iterator still reads
- C: `git_index_read_iterator` frees the old entries it does not keep with
  `index_entry_free`, ignoring `readers`: a `git_index_iterator` (or a C
  iterator's snapshot) of the index then reads freed entries.
- Rust: the entries go to the deferred list while another reader is
  active, as `index_remove_entry` does.

### index: paths read past the index file's data
- C: `read_entry` looks for the NUL of a path of 0xFFF or more bytes with
  `memchr(path_ptr, 0, buffer_size)`, `buffer_size` counted from the
  entry's start, not the path's; takes the path with `strlen`; and in
  version 4 decodes the varint and `strlen`s the suffix with no bound.
  Only the `git_str` terminator of the file's buffer stops them, and a file
  that ends inside a varint makes `strlen` start past it.
- Trigger: a crafted index whose last entry's path or varint runs to the
  end of the data.
- Rust: every read stays within the data (its end counts as the NUL that
  ends the buffer); the result is C's: `invalid data in index - invalid
  entry`.

### tree-cache: recursion per level of the `TREE` extension
- C: `read_tree_internal` and `write_tree` recurse once per nesting level,
  so a crafted index with a deeply nested `TREE` extension (7 bytes per
  level) overflows the stack.
- Rust: an explicit stack; the same trees, errors and bytes.

### index: leaks on failures
- C: `git_index_read_tree` leaks the entries it made when the tree walk or
  the map fails; `git_index_read_iterator` leaks the copies it made when
  it fails; `add_repo_as_submodule` leaks the entry, the path and the
  opened repository on its errors; `read_conflict_names` the record on an
  allocation failure; `read_reuc` and `git_index__fill` an entry the
  vector could not take; a duplicate unknown extension's copy is leaked.
- Rust: frees them (the duplicate's error message is still set, as the
  C's `extension_ondup` set it while the read went on).

### index: entries read after a callback or a removal may have freed them
- C: `index_apply_to_all` copies `entry->path` after the user's callback,
  which may have removed (freed) the entry; `git_index_add_from_buffer`
  reads `entry->path` after `git_index__conflict_to_reuc`, which removes a
  source entry given with a stage.
- Rust: copies the path first.

### index: `git_index_extension_add` reads 4 bytes of any signature
- C: `memcpy(ext->header.signature, signature, 4)` reads past a shorter
  string (and stores what follows its NUL).
- Rust: reads up to the NUL (at most 4 bytes) and pads with NULs. Lookups
  and removals compare with `strncmp`, as C.

### index: NULL arguments dereferenced
- C: `git_index_read`, `git_index_write`, `git_index_read_tree`,
  `git_index_read_index`, `git_index_remove`, `git_index_remove_directory`,
  `git_index_find_prefix`, `git_index_caps`, `git_index_owner`,
  `git_index_name_add`, `git_index_reuc_find`, `git_index__conflict_to_reuc`,
  `git_index_snapshot_*`, `git_indexwriter_*`, `git_index_entry_stage` and
  `git_index_entry_is_conflict` dereference a NULL index (or path, or
  entry) without checking it.
- Rust: `invalid argument: 'index'` (or `'path'`, `'dir'`, `'prefix'`,
  `'tree'`) and C's failure value (-1, NULL, 0; `git_index_reuc_find` of a
  NULL path fails `git_vector_bsearch2`'s `'key'` check, as C).

### index: kept as the C behaves (defined, but surprising)
- `git_index_read` returns **1** (with `failed to read index: '…' no longer
  exists` set) when the file disappears between its existence check and
  its `stat` (`updated = git_futils_filestamp_check(...) < 0`); the stamp's
  "changed" result is otherwise ignored, only the checksum decides.
- `git_index_read_tree` and `git_index_read_index` install a
  case-sensitive path map (`GIT_INDEX_ENTRYMAP_INIT`) even in a
  case-insensitive index, so `git_index_get_bypath` is case-sensitive until
  the next `git_index__set_ignore_case`.
- Version 4 is written without `is_index_extended`, so the extended flags
  of an entry whose `flags` lack `GIT_INDEX_ENTRY_EXTENDED` are dropped.
- A `TREE` node with an entry count below -1 is read without an id and
  written with one.
- The index's `version` field may say 4 after a failed read of a version-4
  file (it is set from the header before the entries are parsed), and the
  entries read before a failure stay in the index.
