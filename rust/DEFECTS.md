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

## Pack (WP l-pack: pack, mwindow, odb_pack)

### pack: `git_pack_foreach_entry` lists wrong ids for SHA-256 packs
- C: `src/libgit2/pack.c`, `git_pack_foreach_entry`: the ids in pack order
  are `&index[5 * (current - off)]`, a stride of 20 bytes: right for SHA-1
  only.
- Trigger: `git_odb_foreach` (or the commit-graph writer) over a SHA-256
  pack (`tests/resources/testrepo_256.git`): every id but the first is
  made of the bytes of two neighbouring ids.
- Rust: the stride is the hash size; the ids are the pack's.

### pack: a cyclic `REF_DELTA` chain loops forever
- C: `git_packfile_resolve_header` follows bases in a `while` loop, and
  `pack_dependency_chain` pushes one element per base until it runs out of
  memory, with no check that the chain ends.
- Trigger: a pack whose `REF_DELTA` names itself (or a cycle of objects)
  as its base: `git_odb_read` / `git_odb_read_header` of it never returns.
- Rust: both walks check for a revisited offset (Brent's algorithm, O(1)
  memory) and fail with `invalid pack file - delta chain loops` (class
  `GIT_ERROR_ODB`, -1). Valid packs never revisit an object.

### pack: reading a corrupted delta's header hangs or reads uninitialized memory
- C: `git_packfile_resolve_header` reads a delta's result size with
  `git_delta_read_header_fromstream` (`delta.c`), whose loop retries
  `GIT_EBUFS` forever (the window cannot be opened, as when the data runs
  into the pack's trailer, or zlib needs input it never gets), and adds a
  failed read's -1 to its length, then parses its uninitialized 16-byte
  buffer as if it were `SIZE_MAX` bytes long.
- Trigger: `git_odb_read_header` of a delta whose deflated data is
  corrupted ("incorrect header check"): C returns 0 with a size made of
  stack garbage (`difftest/pack_diff.c`, case `t1/11/obj-ref-to`).
- Rust: the pack reader reads the header bytes itself, with the same reads,
  and parses them with `git_delta_read_header`; both cases fail with `error
  reading from the zlib stream` (class `GIT_ERROR_ZLIB`, -1).

### pack: a short pack file is checked against an uninitialized header
- C: `packfile_open_locked` reads the 12-byte header into an
  uninitialized `struct git_pack_header` and checks only for a failed read;
  the trailing checksum is likewise read with `p_pread` into an
  uninitialized buffer, checked only for `< 0`.
- Trigger: a `.pack` shorter than 12 bytes (or truncated under the
  index's checksum).
- Rust: both buffers start zeroed, so a short read fails the signature or
  checksum check: `invalid packfile '<name>'`, as for any bad header.

### pack: `git_packfile_unpack` leaks a delta's base on a corrupted delta
- C: when the base was not cached (too large, or already there) and the
  delta's data cannot be inflated, `obj->data = NULL` drops the base, which
  was the caller's to free.
- Trigger: a delta against a base over 1 MiB whose deflated data is
  corrupted.
- Rust: the base is freed. On failure the out-object is left empty (C left
  `obj->data` pointing at freed memory).

### pack: the delta base cache loops forever when every entry is in use
- C: `cache_add` runs `while (memory_used + len > memory_limit)
  free_lowest_entry(cache)`, and `free_lowest_entry` frees only unused
  entries: if all are in use, it never ends.
- Trigger: more threads than 16 MiB / 1 MiB unpacking deltas of the same
  pack at once, each holding a cached base of about 1 MiB.
- Rust: eviction stops when a pass frees nothing; the cache then exceeds
  its (soft) limit until the entries are released.

### mwindow: `git_mwindow_open` returns a pointer outside the mapping
- C: a new window is made for any offset and its containment is not
  checked: for an offset at or past the end of the file (within the last
  aligned window), or a small negative one, the pointer is outside the
  mapping and `*left` wraps around.
- Trigger: a corrupted index whose offset points past the end of the pack:
  `git_packfile_unpack_header` (which, unlike `pack_window_open`, does no
  bounds check) reads it: SIGSEGV (seen in `difftest/pack_diff.c`).
- Rust: such an offset fails with `invalid pack window offset <off> (file
  size <size>)` (class `GIT_ERROR_ODB`); `git_packfile_unpack_header` then
  returns `GIT_EBUFS`, as for a window it cannot open. Offsets so far past
  the end that the window cannot be mapped at all fail in `mmap`, as in C.

### mwindow: a failed `git_mwindow_open` leaves the cursor on a released window
- C: when the cursor's window does not hold the offset, its `inuse_cnt` is
  decremented, and if opening the new window then fails, `*cursor` still
  points at it: the caller's `git_mwindow_close` decrements it again
  (`size_t` underflow; that window can never be freed or closed again).
- Trigger: a cursor on a window, then a request for an offset whose window
  cannot be mapped.
- Rust: the cursor is cleared when its window is released.

### mwindow: `mapped` keeps the length of a window that could not be mapped
- C: `new_window_locked` adds the window's length to `ctl->mapped` before
  mapping it and does not subtract it when both `mmap` attempts fail.
- Trigger: an offset past the end of the pack (its "length" is negative,
  cast to `size_t`): `mapped` becomes huge and from then on every new
  window first closes every other one.
- Rust: the length is subtracted again.

### mwindow: a window size below 2 divides by zero
- C: `new_window_locked` aligns offsets to `git_mwindow__window_size / 2`.
- Trigger: `git_libgit2_opts(GIT_OPT_SET_MWINDOW_SIZE, 1)`, then any pack
  read: SIGFPE.
- Rust: the alignment is at least 1 byte; a window size of 0 then fails to
  map (`p_mmap`'s argument check).

### mwindow: the file limit closes descriptors of files that may be freed
- C: `git_mwindow_file_register` picks the least recently used files under
  `git_mwindow__mutex`, releases it, then locks each file and closes its
  descriptor: a file whose pack is freed meanwhile (on another thread) is
  locked and written after it is freed.
- Trigger: `GIT_OPT_SET_MWINDOW_FILE_LIMIT` and repositories opened and
  freed on several threads (`tests/libgit2/pack/threadsafety.c` runs this
  pattern).
- Rust: the descriptors are closed while the mutex is held (a file is
  deregistered under the mutex before it is freed), taking each file's
  lock with `try_lock` so that the lock order cannot deadlock. A file whose
  lock is held (a reader is opening a window of it) is left registered and
  open, and the next least recently used file is closed instead; if every
  candidate is busy, the (soft) file limit is exceeded until the next
  registration, as when C finds no candidate. (Round 1 left such a file
  open but deregistered, so its windows escaped the mapped limit: review
  L-PACK-2.)

### odb_pack: `foreach` iterates state its callback may free
- C: `pack_backend__foreach` iterates `backend->packs` and passes
  `backend->midx` to `git_midx_foreach_entry` while calling the callback,
  which may refresh the odb (a lookup of a missing object does):
  `git_vector_insert` may reallocate the vector, and a changed
  `multi-pack-index` is freed, under the iteration.
- Trigger: a `git_odb_foreach` callback that reads an object which is not
  there after a pack (or a new multi-pack-index) appeared.
- Rust: `foreach` iterates a snapshot (references to the packs and to the
  multi-pack-index), taken after its own refresh.

### odb_pack: `writepack` leaks when the indexer cannot be made
- C: `pack_backend__writepack` returns -1 without freeing the writepack
  when `git_indexer_new` fails.
- Rust: nothing leaks.
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

### odb: finalizing a stream whose backend belongs to no odb
- C: `git_odb_stream_finalize_write` does `if (git_odb__freshen(stream->backend->odb, out)) return 0;`;
  with `backend->odb == NULL` the freshen fails its `GIT_ASSERT_ARG(db)`
  and returns -1, which is true: C returns 0 without calling
  `finalize_write` (nothing is written) and leaves `invalid argument: 'db'` set.
- Trigger: a stream of a backend used directly (`backend->writestream`),
  never added to an odb (review l-odb-r1, L-ODB-2).
- Rust: no freshen without an odb; `finalize_write` runs and writes the object.

### odb: `git_odb_open_wstream` leaks the stream when the hash setup fails
- C: when `git__malloc` of the hash context, `git_hash_ctx_init` or the
  header fails after a backend made the stream, the stream is not freed
  and the error returned (the caller never sees it).
- Rust: the stream is freed through its `free` slot.

### odb_loose: an unknown `oid_type`
- C: `git_odb_backend_loose` takes any `opts.oid_type` (e.g. 3);
  `object_file_name` then sizes the path for a hex size of 0 and
  `git_oid_pathfmt` writes 41 bytes: a heap overflow.
- Rust: lookups and writes fail with `unknown oid type` (class
  `GIT_ERROR_INVALID`).

### odb_loose, odb_mempack: NULL data with a non-zero length
- C: the backends' `write` slots `memcpy` / deflate from NULL (a crash).
- Rust: -1 with `invalid argument: 'data'`. (`git_odb_write` itself fails
  earlier, as C, with `invalid object`.)

### cache: NULL objects
- C: `git_cached_obj_decref(NULL)` and `git_cache_store_raw/parsed(cache, NULL)`
  dereference NULL.
- Rust: they do nothing (decref) or return NULL (store).

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

### tree-cache: a 22-byte `TREE` record makes the C allocate 16 GiB
- C: `read_tree_internal` allocates (and zeroes) `children_count` pointers
  from the pool before reading any child, whatever is left of the data: a
  record declaring 2147483647 children asks for 16 GiB, and fails as
  corrupted only after (or out of memory).
- Trigger: `"\0-1 2147483647\n"` as a `TREE` extension.
- Rust: each child needs at least 6 bytes (`"\0-1 0\n"`), so a count
  above what is left / 6 is `corrupted TREE extension in index` before
  any allocation: C's result whenever its allocation succeeded.

### index: one index read from several threads races in C
- C: `tests/libgit2/threads/diff.c` diffs one repository index from 32
  threads: each calls `git_index_read(index, false)`, which writes the
  `on_disk` and `dirty` bitfields, and takes snapshots, which
  `git_vector_sort` the entries (a write when unsorted). Plain writes on
  shared memory from several threads: a data race (TSan reports it).
- Rust: the read paths take shared access only; the bitfields are an
  atomic word written only when a bit changes, the reference and readers
  counts atomics; a sort happens (exclusively) only when the entries are
  not sorted, which is the C's race again, now confined to that case.

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

### settings: `settings_global_shutdown` leaves the freed strings set
- C: frees `git__user_agent`, `git__user_agent_product` and
  `git__ssl_ciphers` at the last shutdown but leaves the pointers set.
- Trigger: `git_libgit2_init`, `GIT_OPT_SET_USER_AGENT`,
  `git_libgit2_shutdown`, `git_libgit2_init`, then `GIT_OPT_SET_USER_AGENT`
  (frees it again) or `GIT_OPT_GET_USER_AGENT` (reads freed memory).
- Rust: the pointers are reset to NULL (the defaults apply again).

### repository: `git_repository_head_for_worktree` returns a reference into a freed repository
- C: the reference is looked up in the worktree's repository, which is
  freed before returning; the reference keeps its refdb (refcounted), but
  the refdb's `repo` pointer dangles, so `git_reference_peel`,
  `git_reference_resolve` and the like on it use freed memory.
- Rust: the export returns the reference as C does (C callers only read
  its name, type and target); the Rust callers see it only while the
  worktree's repository is open.
## l-commit (commit, commit_list, tree)

### commit: a repeated `encoding` header leaks
- C: `commit.c`, `commit_parse`: each `encoding ` header assigns
  `commit->message_encoding = git__strndup(...)` without freeing the
  previous one.
- Trigger: a commit with two `encoding` headers.
- Rust: the earlier value is freed; the last one is kept, as in C.

### commit: a field with a newline in `git_commit_header_field`
- C: `git_commit__header_field`: `git__prefixcmp(buf, field)` can match
  across the line's end when `field` contains `\n`; `eol - buf` is then
  negative, and `git_str_put` is asked for `SIZE_MAX`-ish bytes.
- Trigger: `git_commit_header_field(&buf, commit, "tree\nparent")`.
- Rust: fails with the out-of-memory error (-1) without reading anything,
  which is what the C's `git_str_put` reports when it does not crash.

### commit: NULL signatures and missing fields are dereferenced
- C: `git_commit_create*` / `git_commit__create_buffer` pass `author` /
  `committer` to `git_signature__writebuf`, which dereferences them;
  `git_commit_time` / `_time_offset` dereference `committer`;
  `git_commit__header_field` and `git_commit_message` read `raw_header` /
  `raw_message`, which are NULL in a quick-parsed commit.
- Trigger: `git_commit_create(..., NULL, NULL, ...)`.
- Rust: `invalid argument: 'sig'` (-1) after the buffer is disposed;
  `INT64_MIN` / -1 (the getters' "invalid" values); a NULL header is
  empty (`GIT_ENOTFOUND`); a NULL message gives NULL.

### commit: allocation failures in signature parsing are ignored
- C: `signature.c`, `extract_trimmed` (`git_signature__parse`): a failed
  `git__substrdup` of the name or e-mail leaves it NULL, and the parse
  succeeds.
- Trigger: `cl_alloc_limit` during a commit parse.
- Rust (the commit parser's copy of the signature parser): the parse fails
  with the out-of-memory error.

### commit: `git_signature__writebuf` with an offset of `INT_MIN`
- C: `signature.c`, `git_signature__writebuf`: `offset = -offset` is a
  signed overflow for `INT_MIN`. The C build (clang) then divides the
  result as unsigned, and writes `-3579139408` (35791394 hours, 08 minutes).
- Trigger: `git_commit_create_buffer` with a signature whose
  `when.offset == INT_MIN` (`git_signature_new` accepts it).
- Rust: the unsigned magnitude (`unsigned_abs`), so the bytes are the C
  build's.

### commit: `git_commit_extract_signature` with an empty field never returns
- C: `git_commit__extract_signature` with `field == ""`: every line matches
  the empty prefix, and a line that does not continue with a space sets
  `buf = h` (the same position): an endless loop.
- Trigger: `git_commit_extract_signature(&sig, &data, repo, &id, "")`.
- Rust: an empty field is never found: `GIT_ENOTFOUND`, `this commit is not
  signed`.

### commit: `git_commit_amend` leaks the reference when the creation fails
- C: `git_commit_amend` frees the looked-up `update_ref` only
  `if (!error && update_ref)`.
- Rust: freed on every path.

### tree: NULL arguments of `git_tree_entry_bypath` and `git_tree_walk`
- C: neither asserts its arguments; NULL `entry_out`, `root`, `path`,
  `tree` or `callback` are dereferenced or called.
- Rust: `invalid argument: '<name>'` (-1) for each (`git_tree_walk` checks
  the mode first, as the C does).

### commit: `git_commit__extract_signature` leaks the object
- C: when appending to `signed_data` fails, it returns -1 without freeing
  the odb object (or clearing the outputs).
- Rust: the object is freed and both outputs are cleared, as on its other
  error paths.

### tree: a tree builder replacing a duplicate name leaks
- C: `tree.c`, `append_entry` (`git_treebuilder_new` from a tree with
  two entries of the same name; `git_tree__write_index`) puts the new
  entry into the map, which replaces the old one without freeing it.
- Trigger: `git_treebuilder_new` with a source tree parsed from a buffer
  that names the same file twice.
- Rust: the replaced entry is freed.

### tree: names longer than 65535 bytes
- C: `TREE_ENTRY_CHECK_NAMELEN` sets an error but does not return:
  `alloc_entry` stores the length truncated to 16 bits (a tree written from
  such an entry has a wrong name), and `tree_key_search` searches with the
  truncated length.
- Trigger: `git_treebuilder_insert` of a 70,000-byte name (with strict
  object validation off).
- Rust: the same (the behaviour is defined, and the result matches the C's
  byte for byte); recorded here because it is wrong.

### tree: an unknown object id type in a tree parse
- C: `git_tree__parse_raw` with an invalid `oid_type` reads 0 bytes per id
  and leaves each entry's `oid` uninitialized (`git_oid_from_raw` fails;
  its result is ignored).
- Rust: the ids are zero (the same error is set).

### tree: `git_tree_create_updated` leaks on allocation failure
- C: `GIT_ERROR_CHECK_ALLOC` of the stack's new levels returns without
  freeing the levels' builders and trees.
- Rust: everything is released on every error path.
## revwalk, hashsig (WP l-revwalk)

### hashsig: `hashsig_add_hashes` reads one byte past the input
- C: `src/libgit2/hashsig.c`, the whitespace-skipping loops
  (`for (; scan < end && git__isspace_nonlf(ch); ch = *scan) ++scan;` and
  the `'\r'` one) read `*scan` after `++scan` reached `end`. Triggered by
  any input that ends in non-LF whitespace (with `IGNORE_WHITESPACE`,
  `SMART_WHITESPACE`, or after a newline under `SMART_WHITESPACE`): with
  `git_hashsig_create`, an out-of-bounds read of the caller's buffer (a
  crash at a page end); with `git_hashsig_create_fromfile`, of the stack
  buffer (stale or past it at a full 4096-byte block). The byte decides
  `use_ignores` for the next block of a file, so a file whose 4096-byte
  block ends in such whitespace hashes from an indeterminate value.
- Rust: reads nothing past the input and uses 0 for that byte (so
  `use_ignores` becomes false under `SMART_WHITESPACE`). For buffers the
  result is the C's (the value is discarded); for files it is the C's
  whenever the C's is determined (the difftest skips the other case).

### hashsig: `git_hashsig_create` and `_create_fromfile` leak on a failed option check
- C: both return the `GIT_ASSERT` error of `hashsig_in_progress_init`
  (both whitespace options) without freeing the signature.
- Rust: same error, no leak.

### hashsig: NULL arguments
- C: `git_hashsig_create(NULL, …)`, `_create_fromfile` with a NULL path and
  `git_hashsig_compare` with a NULL signature dereference NULL.
- Rust: `invalid argument: '<name>'`, -1.

### revwalk: unchecked NULL walks and out-parameters
- C: `git_revwalk_push_range`, `git_revwalk_simplify_first_parent`,
  `git_revwalk__push_commit`, `__push_ref` (and `opts`) and
  `git_revwalk__commit_lookup` dereference a NULL `walk` (or `oid`,
  `opts`); `git_revwalk_new` writes a NULL `revwalk_out`.
- Rust: `invalid argument: '<name>'` and -1 (NULL for the lookup).

### revwalk: allocation failures that the C ignores
- C: `add_parents_to_list` ignores a failed `git_commit_list_insert_by_date`
  (the parent is silently dropped from the walk) and
  `mark_parents_uninteresting` a failed `git_commit_list_insert` (part of
  the history is not marked uninteresting and is shown); `limit_list` and
  `prepare_walk` leak their lists on errors; `git_revwalk_new` leaks the
  walk when `git_pqueue_init` or `git_pool_init` fails.
- Rust: every such failure is the out-of-memory error, returned from
  `git_revwalk_next`; nothing leaks.

### revwalk: a NULL parent slot
- C: `commit_list.c` stores the NULL of a failed
  `git_revwalk__commit_lookup` (out of memory) in `parents[i]` and goes
  on; the walk then dereferences it.
- Rust: the walk skips a NULL parent.
## References (WP l-refs: refs, refdb, refdb_fs, reflog)

### refdb_fs: the sorted lookup reads `packed-refs`' mapping after unlocking
- C: `packed_lookup` calls `packed_map_check` (which takes and releases
  `prlock`) and then binary-searches `backend->packed_refs_map.data`
  without the lock; a concurrent `packed_write` (`git_refdb_compress`, a
  delete of a packed reference) calls `packed_map_free`, which unmaps it.
- Trigger: one thread looking up references while another deletes a packed
  one (`tests/libgit2/threads/refdb.c` does both).
- Rust: the mapping is an `Arc`; a lookup holds it for the search, and it
  is unmapped when the last holder drops it.

### refdb_fs: the `packed-refs` traits are written by several threads
- C: `packed_set_peeling_mode` writes `backend->peeling_mode` and
  `backend->sorted` from `packed_reload` (under the cache's write lock) and
  from `packed_map_check` (under `prlock`), while lookups read them under
  neither: a data race.
- Rust: both are atomics (relaxed): each read sees one of the values a
  writer stored. Beyond the data race, C's trait and the mapping it
  describes can come from different files (a reload of a sorted file
  racing a lookup that mapped an unsorted one, whose binary search then
  misses a reference): the sorted lookup uses the `sorted` trait of the
  header of the very mapping it searches, and the shared value only when
  that file has no header (as C keeps the previous one).

### refdb_fs: an out-of-memory glob makes the iterator "succeed" without one
- C: `refdb_fs_backend__iterator` sets `error = GIT_ERROR_NOMEMORY` (the
  error class, 2) when `git_pool_strdup` of the glob fails, and returns it;
  `git_refdb_iterator` tests `< 0` and dereferences the unset `*out`.
- Rust: the out-of-memory error (-1).

### refdb_fs: `init` returns 1 when making `refs/heads` or `refs/tags` fails
- C: `(error = git_futils_mkdir(path.ptr, dmode, 0) < 0)` stores the
  comparison: a failure returns 1, which `git_repository_init` treats as
  success.
- Rust: kept (returns 1, with mkdir's error message set): the closest
  defined result.

### refdb_fs: `serialize_reflog_entry` reports out of memory as 1
- C: returns `git_str_oom(buf)` (true: 1); its callers test `< 0`, so an
  out-of-memory buffer is written to the reflog as if it had succeeded.
- Rust: the out-of-memory error; `reflog_write` returns 0 there as C does
  (its `error` is still 0 when it jumps to its cleanup), `reflog_append`
  fails.

### refdb_fs: `reflog_parse` reads an uninitialized `char`
- C: `char c;` is set only by `git_parse_peek`; on a line that ends right
  after the two object ids (`<old> <new>` and nothing else), the loop never
  peeks and `if (c == '\t')` reads it uninitialized.
- Rust: `c` starts at NUL: no message (the signature fails to parse there
  anyway, so the line is skipped).

### refdb_fs: the iterator finds its backend through `iter->db->backend`
- C: `refdb_fs_backend__iterator_next` takes `GIT_CONTAINER_OF(iter->parent.db->backend,
  refdb_fs_backend, parent)`: after `git_refdb_set_backend` replaced the
  backend during an iteration, that is another type (or freed memory).
- Rust: the iterator holds its own reference to the backend's state.

### refs: `git_reference__normalize_name` without a buffer and with validation disabled
- C: `git_str_sets(buf, current)` with `buf == NULL` (reached by
  `git_reference__name_is_valid(…, GIT_REFERENCE_FORMAT__VALIDATION_DISABLE)`)
  dereferences NULL.
- Rust: valid, nothing written.

### refs: `git_reference_dup` of a reference without a database
- C: `GIT_REFCOUNT_INC((*dest)->db)` with `db == NULL` (a reference from
  `git_reference__alloc` that no refdb returned) dereferences NULL.
- Rust: the copy has no database either.

### refs: `git_reference__realloc` leaks on failure
- C: when `git__realloc` fails, `*ptr_to_ref` is set to NULL and the old
  block is neither returned nor freed.
- Rust: frees it (with its symbolic target and its count on the refdb).

### refs: `git_reference_list` leaks the names on failure
- C: `git_vector_dispose(&ref_list)` frees the vector, not the names it
  holds.
- Rust: frees them.

### refdb: `git_refdb_set_backend(db, NULL)`
- C: `GIT_ERROR_CHECK_VERSION` accepts NULL, then `!backend->exists`
  dereferences it.
- Rust: "incomplete refdb backend implementation", `GIT_EINVALID`.

### refs: `git_reference__realloc` with the reference's own name
- C: `git__realloc(*ptr_to_ref, reflen)` may move (and free) the block,
  then `memcpy(rewrite->name, name, …)` reads `name`: if it points into the
  old block (`git_reference__realloc(&ref, ref->name)`), that is a read of
  freed memory.
- Rust: the name is copied before the reallocation; the result is the
  intended rename.

### refs: `git_reference__normalize_name` with a name inside the output buffer
- C: `git_str_clear(buf)` writes a NUL at `buf->ptr[0]` before the name is
  read, so a name that points at the buffer's contents is read as `""`
  (`GIT_EINVALIDSPEC`, "the given reference name '' is not valid"; `""`
  with validation disabled); a name further into the buffer is overwritten
  while it is read. No C caller in the tree passes such a name.
- Rust: the name is copied first; the result is the name's normalization.

### refs: `git_reference_dup` of a symbolic reference without a target
- C: `git_reference__alloc_symbolic(name, NULL)` fails its `target`
  assertion and `git_reference_dup` returns -1 (out of memory).
- Rust: the same (-1, with the assertion's and then the out-of-memory
  message). (Only a corrupt or hand-made reference has no target.)

## l-obj (object, object_api, blob, tag, signature, grafts)

### signature: `git_signature__parse` ignores a failed name or e-mail copy
- C: `extract_trimmed`'s `git__substrdup` result is not checked; on
  allocation failure the parse (and so `git_signature_from_buffer`) returns
  0 with `name` or `email` NULL, which callers (`commit.c`, `refdb_fs.c`,
  the tag parser) then dereference.
- Rust: -1 with the out-of-memory error.

### signature: timezone arithmetic overflows `int`
- C: `hours * 60 + mins` and the negation of the result, for a timezone
  that `git__strntol32` parses to a large negative number (`+-2147483648`:
  `hours` = -21474836), overflow a signed `int`.
- Rust: wrapping arithmetic (the value the C build happens to compute).

### signature: `git_signature__writebuf` of an offset of `INT_MIN`
- C: `offset = -offset` overflows; the reference build then prints the
  magnitude 2^31 (`-3579139408`).
- Rust: the magnitude, as that build prints (difftest `obj_diff.c`).

### signature: leaks and NULL dereferences on failure
- C: `git_signature_new` leaks the signature (and the name) when a copy
  fails; `git_signature_dup` leaks on a failed copy and crashes on a NULL
  `name`/`email`; `git_signature_default_from_env` frees the signatures it
  made with `git__free` on failure (their strings leak);
  `git_signature__equal` crashes on a NULL string; `current_time`
  dereferences NULL when `gmtime_r` fails.
- Rust: frees everything; copies a NULL string as NULL; a NULL string
  equals only NULL; an offset of 0 when `gmtime_r` fails.

### signature: `GIT_*_DATE` without a timezone
- C: `user_from_env` passes `offset` uninitialized to `git_signature_new`
  when `git_date_offset_parse` takes the approximate parser (which does not
  set the offset).
- Rust: offset 0.

### object: `git_object__from_raw` leaks on an id failure
- C: returns without freeing the object when `git_object_id_from_buffer`
  fails (an unknown oid type).
- Rust: frees it.

### object: a cached object with unknown flags
- C: `git_object_lookup_prefix`'s `GIT_ASSERT` path returns without giving
  back the cache's reference.
- Rust: gives it back, then the same error.

### object: NULL arguments C dereferences
- C: `git_object_lookup` reads `repo->oid_type` before
  `git_object_lookup_prefix` checks `repo`; `git_object_dup(NULL)`,
  `git_tag_tagger(NULL)`, `git_tag_create*` with a NULL `oid`,
  `git_grafts_size(NULL)`, `git_grafts_parse(NULL, …)`,
  `git_blob_create_from_stream_commit(…, NULL)` crash; `git_tag_target` of
  a tag parsed from raw data (no repository) dereferences NULL.
- Rust: `invalid argument: '<name>'` (-1) for the arguments, NULL from
  `git_tag_tagger`, 0 from `git_grafts_size`; `git_tag_target` of a tag
  without a repository fails with `invalid argument: 'repo'`, and with a
  NULL out-parameter with `invalid argument: 'object_out'` (C's
  `git_object_lookup` checks). `git_tag_delete(repo, NULL)` (C crashes)
  fails as the name `refs/tags/` does: -12, `the given reference name
  'refs/tags/' is not valid`.

### blob: a NULL buffer with a non-zero length
- C: `git_blob_create_from_buffer(…, NULL, n)`, `git_blob_data_is_binary(NULL,
  n)` and a blob stream's `write(s, NULL, n)` (n > 0) dereference NULL.
- Rust: -1, `invalid argument: 'buffer'` (`'str'` for
  `git_blob_data_is_binary`); nothing is written. (Review l-obj-r1, LO-3:
  the first port silently made the empty blob.)

### blob: the write stream after `close`
- C: `write` (even of 0 bytes) or `git_blob_create_from_stream_commit` after
  the stream's `close` crash in `flush_buffer` (`filebuf.c`).
- Rust: -1, `unrecoverable internal error: 'file->write'`; no temporary file
  is left.

### signature: `git_signature__parse` with `buffer_end` before `*buffer_out`
- C: `memchr` (or `memrchr`) with a length that wrapped: undefined behaviour.
- Rust: the range is empty: C's parse error for an empty buffer ("no
  newline given" with an ender, "malformed e-mail" without).

### tag: leaks of a failed parse or listing
- C: `git_tag_create_from_buffer` returns without freeing what `tag_parse`
  allocated before it failed; `git_tag_list_match` disposes the vector but
  not the names collected when the iteration fails (it still returns 0).
- Rust: frees them (the listing still returns 0, with an empty array).

### grafts: `git_grafts_open` leaks on a failed path copy
- C: `GIT_ERROR_CHECK_ALLOC(grafts->path)` returns without freeing the
  grafts.
- Rust: frees them.

### object: a NULL repository in `git_object_short_id`, `git_object__is_valid` and `git_object__from_odb_object`
- C: `git_object__short_id` reads `obj->repo->oid_type` of an object parsed
  by `git_object__from_raw` (NULL `repo`); `git_object__is_valid` and
  `git_object__from_odb_object` dereference a NULL `repo` argument: a crash.
- Rust: `short_id` fails with the assertion `obj->repo`; the other two
  report `invalid argument: 'repo'` (the Rust before dereferenced NULL too).

## Tests: `rebase::merge::copy_notes_{specified,disabled}_in_config` leak their config

- **C location:** `tests/libgit2/rebase/merge.c`, both tests take `git_repository_config(&config, repo)` and never free it.
- **Why the C build did not show it:** `repo_local_config` in the C `repository.c` frees a config reference it does not own (see the l-repo entry above), which leaves the repository's config one count low; that cancels the tests' missing free. The Rust fixes that defect, so LeakSanitizer reports the tests' 47 blocks.
- **Change:** the two tests now call `git_config_free(config)` (the only edit to a test file in the port).

## Exports (cross-cutting)

### NULL object, string or out-parameter arguments that the C dereferences unchecked
- C: exports without `GIT_ASSERT_ARG` for a pointer they dereference
  (e.g. `git_config_snapshot(out, NULL)`, `git_config_set_string(NULL, …)`,
  `git_oid_cmp(NULL, b)`, `git_repository_odb(NULL, repo)`): a NULL
  argument crashes.
- Trigger: any such export called with NULL for that argument.
- Rust: an export parameter typed `Option<&T>` / `Option<&mut T>` that the C
  dereferences unchecked fails as `GIT_ASSERT_ARG` would (`invalid
  argument: '<name>'`, -1 / NULL / nothing, per the return type); a NULL
  `Out<'_, T>` out-parameter is not written; a NULL `CStrArg` string reads
  as the code reading it decides (absent, or empty where the C would read
  it as a string). Before, such a NULL was a NULL reference or a write
  through NULL in the Rust as well (undefined behaviour).

### strarray: `git_strarray_copy(arr, arr)` empties the array
- C: `src/util/strarray.c`, `git_strarray_copy`: `memset(tgt, 0, …)` runs
  before `src` is read, so with `tgt == src` the copy sees `count == 0`.
- Trigger: `git_strarray_copy(&a, &a)`.
- Rust: as the C (a defined result: the array is emptied, 0 is returned,
  and its strings are neither freed nor copied — they stay with whoever
  else holds them, leaked if nobody does).

### config: the file backend reads its repository after the repository is freed
- C: `src/libgit2/config_file.c`: `config_file_open` keeps `repo` in the
  backend without a reference, and every refresh (`config_file_refresh`,
  on any read or write of a config holding the backend) evaluates
  `includeIf.gitdir:` / `onbranch:` conditions through it
  (`conditional_match_gitdir`, `conditional_match_onbranch`).
- Trigger: a config that outlives the repository it was opened with:
  `git_repository_config(&cfg, repo)` then `git_repository_free(repo)`
  then `git_config_get_string_buf(&b, cfg, "x")` after the file changed,
  or `git_config_add_file_ondisk(cfg, path, level, repo, 0)` with `repo`
  freed first; the file has an `includeIf.gitdir:` or `onbranch:` section.
  (`git_repository_free`'s documentation calls the use of spawned objects
  afterwards undefined; `git_config_add_file_ondisk`'s does not.)
- Rust: the backend keeps the pointer as C does, but reaches it only while
  the repository is registered live (`repository::live`: registered at
  allocation, unregistered first in `git_repository_free`, which waits for
  in-flight uses); afterwards the conditions are evaluated as with no
  repository (they do not match), as when `repo` is NULL.
