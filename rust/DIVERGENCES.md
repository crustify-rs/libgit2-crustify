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
  collision detection); SHA-256 is OpenSSL's `SHA256_*`, as the C's, with
  `SHA256_CTX` held in place. Digests, the collision error and the "safe"
  digest written on a collision are the C's (rust/difftest/hash_diff.c).
  `git_hash_sha1_*`/`git_hash_sha256_*` are not exported: nothing outside
  `hash.c` called them.
- **zstream: inflating.** Inflate streams use zlib-rs (vendored in
  `rust/vendor/zlib-rs` with a fix that keeps zlib's error messages), not
  the system zlib; deflate streams still use the system zlib, so compressed
  bytes are unchanged. Return codes and messages are zlib's
  (rust/difftest/zstream_diff.c: valid, truncated and corrupted streams).
  zlib-rs allocates its state with Rust's allocator where zlib used
  `malloc`; neither used `git__allocator`. zlib (1.2.9+) rejected a stream
  that was copied or moved after `inflateInit` (`inflateStateCheck`: its
  state points back to the stream) with `Z_STREAM_ERROR`; zlib-rs keeps no
  such pointer and accepts it, so freeing both copies of a copied inflate
  `git_zstream` frees its state twice. No C that stays copies or moves an
  initialized `git_zstream`; `git_zstream_init`'s contract forbids it.
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
- **process: the safe `Process` restores `SIGPIPE`'s default disposition
  in the child** (as std's `Command`: a Rust host ignores `SIGPIPE`, and
  `execve` keeps an ignored disposition). The C ABI keeps the inherited
  disposition, as C did.
- **process: `git_process_read` / `_read_err`** write into the caller's
  buffer through a raw pointer (C passes uninitialized buffers).
- **sortedcache: the safe API's guards** take a `std::sync::RwLock` gate
  before the cache's `pthread_rwlock_t` (re-locking on one thread
  deadlocks or panics in std instead of being POSIX-undefined), and
  panic if `pthread_rwlock_*lock` fails (the C ignores positive errors).
  `SortedCache::lock_and_load` returns the file's contents without a
  copy (`FileContents`). A panic in a cached `T`'s `Drop` aborts (it
  runs in the C-ABI free callback).
- **sortedcache: `git_sortedcache_entry` / `lookup_index` never sort
  without a comparison function** (as `git_vector_sort`), so readers of
  a cache without one never write; the configuration (`item_path_offset`,
  the comparison) is read without the lock, outside the lock-protected
  state.

## Filesystem layer (WP u3: posix, unix/map, unix/realpath, futils, fs_path, filebuf)

- **posix: `p_open`** is variadic: a C shim in `rust_shims.c` reads the mode
  (with `O_CREAT`, as C) and calls the Rust (`git__p_open`). A NULL path
  fails with `EFAULT` (C passed it to `open`, which does the same).
- **posix: `p_fsync__cnt`** is an `AtomicUsize` that the Rust's `fsync`
  increments atomically; C's inline `p_fsync` (in the header, for the C
  that stays) still increments it non-atomically, as before.
- **posix: `p_write` of more than `isize::MAX` bytes** fails with `EINVAL`
  before any system call (the Rust makes a slice of the bytes, and no
  buffer can be that large); C called `write(2)`. `p_read`, `read_raw`,
  `git_futils_readbuffer_fd` and the other `git__is_ssizet`/`git__is_sizet`
  checks are the C's (always true on LP64: a huge length fails later, as in
  C, e.g. `readbuffer_fd(…, SIZE_MAX)` with the out-of-memory error).
- **posix: `p_mmap` failure** leaves `out->data` NULL (and `len` 0); C left
  `MAP_FAILED` in `data`. Same code and message; no caller reads `data`
  after a failure.
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
- **fs_path / futils: arguments that alias the output `git_str`** are
  copied before the string is written. Where C's result did not depend on
  the aliasing (`git_fs_path_dirname_r(&b, b.ptr)`, `basename_r`,
  `normalize_slashes`, `fromurl`, `mktmp(&b, b.ptr)`, the `git_str_set`
  of the C idioms) the results are the C's. Where C asserted or read
  memory it had just overwritten, the Rust gives the result of the
  unaliased call instead:

  | call | C | Rust |
  |---|---|---|
  | `join_unrooted(&b, b.ptr, "/base")` | -1, `invalid argument: 'str_b < buf->ptr \|\| str_b >= buf->ptr + buf->size'` | 0, the join, root 5 |
  | `join_unrooted(&b, "rel", b.ptr)` | root = `strlen` of the result's start | root = the base's length |
  | `prettify(&b, b.ptr, base)` (relative `b`) | -1, the same assertion | the prettified join |
  | `apply_relative(&b, b.ptr + n)` | 1, the same assertion | the applied path |
  | `git__percent_decode(&b, b.ptr)` | 0, empty (cleared before read) | the decoded input |
  | `find_executable(&b, b.ptr)` | `GIT_ENOTFOUND` (cleared before use) | the lookup of the name |
  | `contains*(&dir, <ptr into dir>)` | false (the assertion) | the real check |

  No caller in the tree aliases these. A `git_str` that borrows its buffer is not written in place
  (`git_fs_path_resolve_relative` and `squash_slashes` copy it into an
  owned buffer first, or leave it), as in the foundation.
- **fs_path: `git_fs_path_walk_up`'s callback** gets the path truncated in
  place in the caller's buffer, as C; for a borrowed `git_str` it gets a
  NUL-terminated copy instead.
- **fs_path: `git_fs_path_str_is_valid_ext`** on a sized `git_str` reads
  exactly `size` bytes (C read `ptr[size]` too, before testing the length).
- **fs_path / futils: NULL path arguments** that C passed to the system
  (and crashed on, or got `EFAULT`) are read as `""` by the exports.
- **fs_path: `git_fs_path_set_error`** prints the action's bytes as they
  are (`errors::Bytes`).
- **futils: the mkdir path cache** (checkout's `cache_pool` and
  `cache_pathset`) is used through u1's `GitPool` and `GitHashSetStr`, the
  Rust ports of those C types.
- **Allocations where C had none** (Rust's allocator; an out-of-memory
  aborts instead of failing): one `CString` per entry in `rmdir_r`'s walk
  and in `cp_r` (the paths are rewritten during the walks), a copy of
  `mktmp`'s filename (it may lie in the output), and `CString`s of the
  `fs` API's paths. Directory iteration, `mkdir`, `filebuf` and
  `resolve_symlink` borrow their paths.
- **`libgit2_rs::fs` (the safe API) refuses paths with a NUL byte** (class
  `INVALID`, "path contains a NUL byte", or `false`), as `std` does; the
  exports read C strings up to their NUL, as C.
## u1 (pool, pqueue, utf8, varint, strlist, hashmap, oid, oidarray, parse)

- **oid: `git_oid_tostr_s`'s buffer** is a 65-byte Rust thread-local (no
  destructor), not a `git__malloc`'d `git_tlsdata` slot: it never fails
  (C returned NULL on allocation failure), is not freed by
  `git_libgit2_shutdown`, and stays valid until the thread's next call or
  its end. `git_oid_global_init` does nothing and returns 0.
- **oid: the shortener's memory** is Rust's allocator (C used
  `git__allocator`), so `cl_alloc_limit` does not make
  `git_oid_shorten_new`/`_add` fail. As in C, it keeps pointers into the
  caller's id strings (the safe `OidShortener<'a>` borrows them for `'a`).
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

## Pack (WP l-pack: pack, mwindow, odb_pack)

- **pack: `struct git_pack_file` is a shared prefix** (`pack.h` under
  `GIT_RUST`): `mwf`, `refcount`, `has_cache` (an `unsigned int`; C's was a
  1-bit bitfield), `idx_cache`, and `pack_name`, now a `const char *` (it
  was a flexible array). The rest is Rust's; `git_pack_cache` and its
  entries are not C types any more. `git_mwindow` is opaque.
- **pack: an unknown object id type** given to `git_packfile_alloc` (or the
  pack backends' options) fails with `invalid oid type` (class
  `GIT_ERROR_INVALID`); C went on with a hash size of 0.
- **pack: locking.** The index is opened once under the pack's lock and is
  immutable after: lookups, `git_pack_foreach_entry` and
  `git_pack_foreach_entry_offset` read it without the lock (C held it for
  each lookup, and held it across `foreach_entry_offset`'s callbacks, so a
  callback that used the pack deadlocked). `pack_window_open` takes only
  the file's lock when the file is open (C took the pack's too).
- **pack: a prefix longer than the hash** (`git_pack_entry_find` or a
  backend slot called directly with `len` over 40 for SHA-1): C compares
  the extra digits with the bytes after the id in the index (the next
  entry's); the Rust compares the full id (the odb clamps `len` anyway).
- **pack: a window smaller than the hash size** (`GIT_OPT_SET_MWINDOW_SIZE`
  below 40 or so): C read a `REF_DELTA`'s base id past the window's end
  (within its page); the Rust returns `GIT_EBUFS` there.
- **pack: the delta base cache** is a Rust map of reference-counted
  entries. Eviction is C's (every entry not in use goes); C's `last_usage`,
  which nothing read, is not kept. It and the packs live in Rust's
  allocator, so `cl_alloc_limit` does not make `git_packfile_alloc` fail.
- **pack: `git_packfile_free`** deregisters the file from the window list
  even when its descriptor is closed (C only when it was open), so no
  freed file can stay in the global list.
- **mwindow: `git_mwindow__mutex`** is statically initialized; neither
  `git_mwindow_global_init` nor the shutdown re-initializes or destroys it.
- **odb_pack: the backend's state** (its packs, the multi-pack-index, the
  last pack an object was found in) is behind a mutex, held for a lookup
  or a read but never across `foreach`'s callback; `foreach` iterates a
  snapshot, so a pack that a refresh from its callback adds is not
  visited (see DEFECTS.md).
- **odb_pack: tables refused.** A slot of a pack backend called while its
  release is deferred (C freed it: a use after free) returns -1, `the pack
  backend is being freed` (class `GIT_ERROR_ODB`; `exists` returns 0). A
  writepack's `append`/`commit` called re-entrantly (from the indexer's
  progress callback) or concurrently returns -1, `the writepack is in use`
  (class `GIT_ERROR_INDEXER`). A panic in a slot returns -1 with
  `GIT_ERROR_INTERNAL`.
## repository, libgit2, settings (WP l-repo)

- **`git_libgit2_opts` is a C shim** (`src/util/rust_shims.c`): it reads each
  option's arguments from the `va_list` exactly as the C did (for
  `GIT_OPT_SET_SEARCH_PATH` the path only after a valid level) and calls
  `git_libgit2__opts`, an internal Rust export (not in `libgit2.exports`).
- **The registered extensions** (`GIT_OPT_SET_EXTENSIONS`) live in a Rust
  `Vec` under a mutex (C: an unsynchronized `git_vector` of `git__strdup`
  copies), in Rust's allocator: setting them cannot fail with -1 on
  allocation failure, and concurrent calls are safe. Order, duplicates
  and the `!` rejection are the C's.
- **The features and build information** (`git_libgit2_features`,
  `_feature_backend`, `_buildinfo`, the version) come from the build's
  `git2_features.h` and `include/git2/version.h`, read by `build.rs` into
  `cfg`s and constants; without `LIBGIT2_BUILD_DIR` (Miri) the reference
  configuration is assumed.
- **Infallible Rust allocations on rare paths**: copies of paths and
  config values made while opening (`CString`/`Vec` conversions of
  environment variables, `core.worktree`, ceiling directories, gitlink
  contents) and the safe API's conversions abort on out-of-memory where C
  returned -1. The `git_repository` itself, its strings, the extensions
  array and every `git_str` use `git__allocator` and fail as C.
- **`GIT_OPT_*` with a NULL out-pointer** fails with `invalid argument: 'out'`
  instead of crashing (see DEFECTS.md).
- **The safe `Repository` is `Send`, not `Sync`.** Its setters that replace
  paths (`set_workdir`, `set_bare`, `set_namespace`, `set_ident`) take
  `&mut self`. The odb, config and index it returns are the odb, config
  and index modules' own `Odb`, `Config`, `Index`, each with its own
  reference, wrapped in a `RepoHandle<'_, T>` that borrows the repository
  and is neither `Send` nor `Sync` (the C objects point back to it); the
  refdb and references are still thin types of `repository::handles`
  (PENDING(l-refs)).
- **The C ABI reads the caller's `git_repository_init_options`** once into
  Rust values and writes the internal `GIT_REPOSITORY_INIT__*` bits back
  into its `flags` at the end (C or-ed them in as it went); the caller sees
  the same flags afterwards.

## odb, odb_loose, odb_mempack, delta, cache (WP l-odb)

- **odb: `git_odb` is a shared prefix** (`rc`, `options`; `odb.h` keeps
  the other fields only without `GIT_RUST`), allocated with
  `git__allocator`; the backends are a Rust `Vec` (with `try_reserve`)
  under a `std::sync::Mutex`, sorted with the C's timsort and comparison,
  so the order is C's. The odb's own cache is a `git_cache` in the Rust
  state; the repository's is `GitRepository::objects()` of the owner.
- **odb: `git_odb_write_multi_pack_index`** calls the backends on a copy
  of the list taken under the lock (C walked the list without the lock).
- **odb: object type names.** `odb.rs` has its own copy of
  `git_objects_table`'s names (`git_object_type2string`,
  `git_object_stringn2type`, `git_object_type_is_valid`) and of
  `git_object_id_from_buffer` without filters (all the odb needs), so that
  the odb and loose parsing run without object.c (l-obj); same results.
- **Rust backends and streams are `ffibox::CTable`s** in Rust's allocator
  (C: `git__calloc`), so `cl_alloc_limit` does not see the tables
  themselves; their buffers (object data, the fake stream's buffer, the
  hash contexts) still come from `git__allocator`. A slot refused because
  its table is being released returns -1 with `the object database backend
  is being freed` (class `GIT_ERROR_INTERNAL`); C used the freed memory.
- **cache: `git_cache_size`** takes the read lock (C read the map's size
  unlocked).
- **Safe API:** `OdbBackend` implementations return `Vec<u8>`, copied into
  `git__allocator` memory for C; paths with a NUL byte are refused
  (`GIT_EINVALID`) instead of being cut short.

- **delta: the index's block** is one `git__allocator` allocation, as in C
  (C frees it with `git__free`), but laid out by the Rust: 4-byte bucket
  heads and 16-byte entries (indices, not pointers), so it is smaller than
  the C's. `git_delta_index_size` still returns the size of the C's
  allocation (32 + 24 per entry + 8 per bucket), which `pack-objects.c`
  accounts with. The temporary bucket counts are a Rust `Vec`
  (`try_reserve`, out-of-memory error on failure, as C's `git__calloc`). The delta and applied
  buffers handed to C come from `git__allocator`, as before.
- **delta: a delta's NUL terminator.** `git_delta_apply` writes the NUL
  after the result once the result is complete (C wrote it first); the
  buffer C receives is the same.

## config (WP l-config)

- **config_list: shared, immutable lists.** A backend's entries are built
  once, then frozen in an `Arc` and never changed; an iterator shares the
  list where C duplicated it (`git_config_list_dup`) for every iteration,
  and a list's strings are interned once per file read. Entries are freed
  when the last reference (backend, iterator or entry handed out) goes, as
  with C's refcount. Not observable but in speed and memory.
- **config_list: memory.** Entry names, values and the lists are in Rust's
  allocator (C: `git__allocator`); `cl_alloc_limit` does not make reads
  fail, and allocation failures of the `Arc`s and iterator tables (small,
  fixed size) abort, where C reported -1. The variable-size buffers use
  `try_reserve` and fail with the out-of-memory error.
- **config: the backends of a config** are a copy-on-write list (an `Arc`
  behind a read-write lock, read in parallel): lookups and iterators take the list as it is when they
  start and keep its backends alive, and no lock is held while a backend
  runs, so a backend may re-enter the config. C read the live vectors (an
  iterator created before `git_config_add_backend` could see the new
  backend, by index).
- **config_file: the tree of files read** (stamps, checksums, includes) is
  rebuilt and swapped in after each read instead of being edited in place,
  and no lock is held while the includes are read (`onbranch:` reads
  `HEAD`, which may read the repository's config and so this backend
  again). As in C, a failed reload keeps the stamps and includes read so
  far and the previous entries.
- **config_file: concurrent writes.** Each slot takes `&self` (several
  threads read a repository's config); the lock state of
  `git_config_lock` is under a mutex. Concurrent writers to one backend
  were a race in C; here each write is a whole read-modify-write of the
  file under the file lock (`.lock`), the later writer failing with
  `GIT_ELOCKED` as in C.
- **config: tables.** The file, memory and snapshot backends and every
  iterator are ffibox `CTable`s: a slot called while the table is being
  released (C calling `free` from another thread during a slot) is refused
  with `the config backend is being released` (C: use after free). An
  iterator's `next` is exclusive: called while another call of the same
  iterator runs (re-entrantly, or from another thread) or while it is being
  released, it fails with `the config iterator is already in use or being
  released` (C would run it). A panic in a slot returns -1 with the
  barrier's internal error.
- **config: `git_config_backend.cfg`** is written when the backend is
  added, as C does; nothing reads it.
- **config: a memory backend opened twice** reads its text again into a
  new list; C appended the entries to the list a second time. A failed
  `open` keeps the entries read before the error, as C does.
- **config: `git_config_unlock` without `git_config_lock`** on a file
  backend fails as committing the C's zeroed `git_filebuf` fails (C wrote
  into the zeroed filebuf first, which could crash).
- **config: the safe API** (`Config`, `ConfigEntry`, `ConfigEntries`,
  `ConfigLevel`, `ConfigBackend`) copies entries out (`ConfigEntry` owns its
  strings) and rejects string arguments with an interior NUL; a Rust
  `ConfigBackend`'s entries without a backend type get `"rust"`.

## index, tree-cache, index_map (WP l-index)

- **`struct git_index`** (under `GIT_RUST` in `index.h`) keeps only the
  fields the C that stays touches (`rc`, `index_file_path`, `stamp`,
  `checksum`, `entries`, `oid_type`, the bitfields, `tree`, `tree_pool`,
  `names`, `reuc`), in the C's order; the path map, deferred deletions,
  readers count, unknown extensions and version are Rust-private, after
  them. The index itself is allocated by Rust's allocator (nothing in C
  frees it); entries, REUC and NAME records, snapshots' vectors and the
  tree cache keep `git__allocator` and the pool, as C.
- **`index_map.c`'s functions** are not exported: nothing outside
  `index.c` referenced them. The map keeps the C's khash, hash and
  equality (keys are entry pointers, lookup keys stack entries).
- **`git_index_read_index`** walks copies of both indexes' entry lists,
  sorted case-sensitively, instead of two C index iterators (which yielded
  exactly those, conflicts included); it takes the references and readers
  the iterators' snapshots took.
- **`index_insert`** looks the existing entry up again after the object
  check (`git_object__is_valid`, which can run code that changes the
  index); the C reused its pointers. The mode is still taken from the
  first lookup, as C.
- **Path checks** pass `git_path_str_is_valid` a sized `git_str` (the
  path's length) instead of `SIZE_MAX`; for paths without a NUL (all the C
  could pass) the result is the same. The safe API rejects a path with a
  NUL.
- **Rust-allocator copies that abort on out-of-memory**: the exports copy
  path arguments that may alias the index's own memory (`to_vec`);
  `git_index__fill` copies the source vector's pointers;
  `git_index_read_index` copies the two entry lists (it sorts them in
  place, with the C's timsort); `canonicalize_directory_path` copies the
  path (C's `git__strdup`); the index and conflict iterators are `Box`es
  (C's `git__calloc` failed cleanly). The parser's version-4 paths, the
  path copies of `add_bypath` and the deferred-deletion and extension
  lists are fallible. `cl_alloc_limit` limits none of the Rust
  allocations.
- **Miri stand-ins.** Under Miri only (`cfg(miri)`, which cannot call C),
  path validation is `fs_path`'s port of the filesystem checks plus a
  `.git` component check, and pathspecs are `wildmatch`'s port: the
  index's own logic, callbacks and re-entrancy run under Miri. The real
  builds call `path.c` and `pathspec.c`.
- **`git_index_iterator`** stays mirrored (the tests read its `snap`);
  `git_index_conflict_iterator` is opaque under `GIT_RUST`.
- **The safe API** (`Index`, `IndexEntry`, `IndexTime`, `Stage`,
  `IndexEntries`, `IndexConflicts`, …) returns owned copies of entries;
  `Index` is a reference (`Clone` takes another), neither `Send` nor
  `Sync`. A panic in a callback closure stops the operation (`add_all`,
  `update_all` and `remove_all` alike) with an error of class
  `GIT_ERROR_INTERNAL`. `read_tree`/`write_tree`/`add_path`'s
  submodule cases need the `Tree` and `Repository` types of l-commit and
  l-repo, so the safe API has no `read_tree`/`write_tree` yet (the C ABI
  has them).
- **(review l-repo-r1) The safe repository API ties its children to the
  repository**: `odb()`, `config()`, `config_snapshot()`, `refdb()`,
  `index()` and `head()` return `Odb<'_>`, `Config<'_>`, `Refdb<'_>`,
  `Index<'_>`, `Reference<'_>`, which are neither `Send` nor `Sync` (in C an
  odb uses its owner's object cache, config backends keep the repository,
  a reference's refdb keeps it). C lets some of them outlive the
  repository; the safe API does not, until the owning modules' ports can
  say which may. `wrap_odb`, `set_odb`, `set_config` and `set_index`
  (re-parenting) are not in the safe API for the same reason (open
  integration item). Every `Repository` holds a `Libgit2` guard; the free
  `libgit2::init`/`shutdown` are crate-internal.
- **(review l-repo-r1) The Rust readers of the user agents copy them under
  a lock** the setters take (`settings::user_agent()` returns a `Vec<u8>`);
  C's readers (`git_settings__user_agent`, the transports) keep C's rule.
## l-commit (commit, commit_list, tree)

- **Copies of other work packages' parsers.** The commit parser carries its
  own ports of `git_signature__parse`, `git_signature__writebuf`,
  `git_object__parse_oid_header` and `git_object__write_oid_header`
  (l-obj's), with the same messages and results (differential test
  `rust/difftest/commit_tree_diff.c`), so that it is pure Rust (Miri) and
  calls no C per header line. The integration step replaces them with
  l-obj's Rust. The commit's signatures are freed as `git_signature_free`
  does.
- **Quick parse without a `git_commit`.** `git_commit_list_parse` parses
  the parents and the commit time straight into the walk's node, without
  the temporary `git_commit`, its parent array and its committer that the
  C allocated (and freed). Under `cl_alloc_limit` it therefore makes fewer
  allocations. More than two parents are collected in a Rust `Vec`
  (`try_reserve`).
- **`git_commit_create_v`** is a C shim (`src/util/rust_shims.c`). It
  asserts `tree` and `git_tree_owner(tree) == repo` before reading any
  argument, then reads the parents as the C's callback did (up to
  `parent_count`, stopping at the first NULL) into an array: on the stack
  for up to 8, grown on the heap beyond, which can fail with the
  out-of-memory error where the C (which read each parent as it validated
  it) could not. It then calls the Rust `git_commit__create_v`.
- **`git_treebuilder` is opaque.** Its body moved from `tree.h` into
  `tree.c` (for the C build); with the Rust it is a Rust struct (allocated
  with `git__allocator`). Its map keeps the C's hashing and iteration order
  (`git_treebuilder_filter` visits entries in the same order). Writing a
  builder sorts its entries with Rust's sort in a Rust `Vec` (`try_reserve`)
  rather than `git_vector_sort`; names are unique, so the order is the
  same.
- **`git_tree_create_updated`** detects duplicate paths after sorting them
  (the C did on each sorted insertion); the result and the error are the
  same.
- **The safe API's summary and body** (`Commit::summary`, `Commit::body`)
  are computed into the Rust value, never into the shared commit object
  (whose `summary` / `body` fields the C ABI keeps filling lazily, as in C).
  `Commit::parent`, `nth_gen_ancestor`, `tree` and `Tree::walk` /
  `get_path` of an object parsed from bytes (no repository) fail with
  `invalid argument: 'repo'`, where the C dereferences NULL.
- **Repository stand-in.** The safe API takes a
  `pending_repository::Repository` (PENDING(l-repo)), a minimal owner of a
  `git_repository *`, until l-repo's `Repository` replaces it.
