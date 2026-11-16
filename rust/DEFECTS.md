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
