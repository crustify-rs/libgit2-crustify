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
