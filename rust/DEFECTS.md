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
