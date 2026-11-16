# The foundation: a guide for later work packages

`src/util` ports libgit2's util layer and provides what every other module
builds on. This is the short version; each module's rustdoc has the rest.

| You need | Use | Module |
|---|---|---|
| a status / error | `Result<T>`, `ErrorCode`, `git_error_set!`, `errors::set_str`, `errors::oom` | `errors` |
| an argument check with the C's message | `git_assert_arg!`, `git_assert!` | `errors` |
| the safe API's error | `Error` (code, class, message) | `errors` |
| memory C may free | `alloc::malloc/free/strdup…`, `GitAlloc`, `GitCStr` | `alloc` |
| `git_str` | `str::GitStr` | `str` |
| `git_vector` | `vector::GitVector` | `vector` |
| `git_array_t(T)` | `array::GitArray<T>` | `array` |
| `git_mutex`, `git_rwlock`, `git_atomic32`, `git_refcount` | `thread::{GitMutex, GitRwLock, Atomic32, GitRefcount}` | `thread` |
| a global init / shutdown hook | `runtime::shutdown_register` | `runtime` |
| strtol, prefix/suffix compares, bsearch, hash, ctype | `util::*`, `ctype_compat::*`, `integer::*` | `util` |
| timsort (what `git_vector_sort` uses) | `tsort::tsort` | `tsort` |
| an export | `export::{barrier, status, bytes, cstr}` | `export` |
| a layout test | `crate::c_layout_test!` | `layout` |
| test setup | `test_support::{init, exclusive, with_alloc_limit}` | `test_support` |
| `p_*` system calls (`errno` kept for `GIT_ERROR_OS`), `git_map` | `posix::{open, read, write, lstat, …, mmap}` | `posix` |
| paths (`git_fs_path_*`), directory iteration | `fs_path::{dirname_r, join_unrooted, prettify, direach, GitFsPathDiriter, …}` | `fs_path` |
| whole files, `mkdir -p`, `rm -r`, file stamps | `futils::{readbuffer, writebuffer, mkdir, rmdir_r, Filestamp, …}` | `futils` |
| lock files (`git_filebuf`) | `filebuf::GitFilebuf` (C-embedded), `filebuf::Filebuf` (Rust) | `filebuf` |
| SHA-1 (collision-detecting) / SHA-256; `git_hash_ctx` embedded in a mirrored struct | `hash::{Hasher, HashAlgorithm, hash_buf, hash_vec, Digest}`; `hash::HashCtx` | `hash` |
| zlib streams; `git_zstream` embedded in a mirrored struct | `zstream::{ZStream, inflate_buf, deflate_buf}`; `zstream::GitZstream` | `zstream` |
| `git_regexp` (PCRE2) | `regexp::Regexp` | `regexp` |
| `wildmatch` | `wildmatch::wildmatch` | `wildmatch` |
| `git_date_parse`, RFC 2822 dates | `date::{parse, offset_parse, rfc2822_fmt}`, `date::{Clock, UtcClock}` | `date` |
| `git_rand_next` | `rand::{next, seed}` | `rand` |

## Errors

C's model is kept: a failure is a non-zero `int`, and the message and class
live in the thread's error state. Internal functions return
`errors::Result<T>` (`= Result<T, ErrorCode>`); the `ErrorCode` is `Copy`, and
the message is already set when you get it.

```rust
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::{git_assert_arg, git_error_set};

fn lookup(name: &[u8]) -> Result<u32> {
    git_assert_arg!(!name.is_empty(), "name");          // "invalid argument: 'name'"
    if name == b"x" {
        // git_error_set(GIT_ERROR_REFERENCE, "reference '%s' not found", name)
        let _ = git_error_set!(ErrorClass::REFERENCE, "reference '{}' not found", Bytes(name));
        return Err(ErrorCode::NOT_FOUND);
    }
    let size = crate::util::integer::alloc_add(name.len(), 1)?;  // GIT_ERROR_CHECK_ALLOC_ADD
    Ok(size as u32)
}
```

- Messages must be the C's byte for byte. Write the Rust format so that it
  prints what the C's printf format printed (`%d` → `{}`, `%08x` → `{:08x}`,
  `%.*s` → `{}` of the sliced bytes). Byte strings (paths) go through
  `errors::Bytes`, which inserts them unchanged even when they are not
  UTF-8 (inside `git_error_set!` only).
- `ErrorClass::OS` appends `": "` and `strerror(errno)` and resets `errno`,
  as C does: capture nothing yourself.
- `git_assert_arg!(cond, "c expr")` / `git_assert!(cond, "c expr")` take the
  C expression's text (the message quotes it). A third argument is the
  value to return instead of `Err(ErrorCode::GENERIC)`, for exports.
- `errors::oom()` sets the out-of-memory error and returns `GENERIC`
  (`GIT_ERROR_CHECK_ALLOC`). `alloc::malloc` and the `GitAlloc` family set it
  themselves on failure.
- `errors::save()` / `restore()`, `clear()`, `exists()`,
  `set_after_callback(code, "action")` are C's.
- The safe API converts at its boundary: `Error::from(code)` takes the class
  and message from the thread state. `Error`, `ErrorCode` and `ErrorClass`
  are re-exported from the crate root.
- `ErrorCode::check(c_int)` / `check_zero` turn a C status into a `Result`.

## Memory

Memory that C may free, `realloc` or keep comes from `git__allocator`:

```rust
use crate::util::alloc::{self, GitAlloc, GitCStr, GIT_ALLOC};

let p = alloc::strdup(c"name");                    // char *, C frees with git__free
let mut v: Vec<*mut c_void, GitAlloc> = Vec::new_in(GIT_ALLOC);
v.try_reserve(n).map_err(|_| errors::oom())?;      // fallible where C checked
let s: Option<GitCStr> = unsafe { GitCStr::from_raw(p) }; // owned C string
```

- `GitAlloc` is ffibox's `CAllocator` over `git__allocator` (8-byte
  alignment). Use it for `Vec`/`Box` and as the policy of ffibox owners
  (`CBox<Foo, GitAlloc>`, `CVec<T, GitAlloc>`, `GitCStr = CStrBox<GitAlloc>`).
- Memory only Rust frees uses Rust's allocator. Where C checked an
  allocation, use `try_reserve` and return `errors::oom()`.
- Never call `git__allocator` across a replacement: tests replace it only
  under `test_support::exclusive()`.

## `git_str`

`GitStr` is the mirrored `git_str` (`{ptr, asize, size}`), with the C's
growth policy, so `asize` matches the C after the same calls. It drops like
`git_str_dispose`. Methods take `&[u8]` and return `Result`:

```rust
let mut path = GitStr::new();
path.puts(repo_dir)?;
path.joinpath_self(b"objects")?;                  // git_str_joinpath(&p, p.ptr, "objects")
path.printf(5, format_args!("/{:02x}", byte))?;   // git_str_printf(&p, "/%02x", byte): 5 = strlen
let bytes: &[u8] = path.as_bytes();
let c: Option<&CStr> = path.as_c_str();
```

- States: empty (`git_str__initstr` or NULL), owned, borrowed
  (`GitStr::borrowed(&[u8])`, or the `unsafe attach_notowned`), out of memory
  (every later operation fails with -1; `as_bytes()` is empty). A borrowed
  string is never written.
- `printf(format_len, args)` takes the length of the C format it replaces,
  so that the first reservation (`2 * format_len`) and thus `asize` match.
- `detach()` gives an owned `GitCStr`; `take_raw()` / `from_raw_parts` move
  the fields (for `git_buf`).
- A struct C sees that contains a `git_str` embeds a `GitStr` by value.

## `git_vector`

`GitVector` is the mirrored `git_vector`. Elements are raw pointers it does
not own. Growth (8, ×1.5) and sorting (`tsort`) are the C's.

```rust
let mut v = GitVector::with_capacity(0, Some(my_cmp))?;   // git_vector_init
v.insert(ptr)?;
unsafe { v.sort() };                     // calls the C comparison: unsafe
v.sort_by(|a, b| rust_cmp(a, b));        // a Rust comparator: safe
for e in v.iter() { … }                  // git_vector_foreach
```

Methods that call the stored C function pointer (`sort`, `insert_sorted`,
`bsearch`, `search`, `uniq`, `verify_sorted`) are `unsafe`: their contract
is that it is sound to call on the elements. The `_by` variants take Rust
closures.

## Threads and atomics

`GitMutex`, `GitRwLock`, `GitCond` are `pthread_*_t` with C's layout, `Sync`,
used through `&self` (`lock()` returns a guard). They have no `Drop`
(`destroy()` is `git_mutex_free`). `Atomic32`/`Atomic64` are `git_atomic32`/
`git_atomic64` (sequentially consistent); `GitRefcount` is `git_refcount`
(`inc`, `dec() -> bool` "free now", `own`, `owner`). A pointer C sets with
`git_atomic_compare_and_swap` is an `AtomicPtr<T>`
(`thread::compare_and_swap`). Types C never sees use `std::sync`.

## Exports

Each module has `pub mod ffi` with `#[unsafe(no_mangle)] pub unsafe extern
"C" fn git_…`. The shape:

```rust
/// `git_foo_bar`.
///
/// # Safety
///
/// `out` is writable; `name` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_foo_bar(out: *mut GitStr, name: *const c_char) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: the caller passes a valid, unaliased git_str and string.
        let (out, name) = unsafe { (&mut *out, cstr_bytes(name)) };
        status(foo::bar(out, name))
    })
}
```

- `export::barrier` catches panics (ffibox's `callback::barrier`): the
  export returns -1 / NULL / nothing, with class `GIT_ERROR_INTERNAL`.
- `status(Result<()>)` → 0 or the code; `bytes(ptr, len)`, `cstr(ptr)`,
  `cstr_bytes(ptr)`, `write_opt(out, v)` are the justified conversions.
- Variadic functions cannot be defined in Rust: add a shim to
  `src/util/rust_shims.c` (compiled only with `USE_RUST`) that formats with
  `vsnprintf` or collects the arguments and calls a `git_…__` Rust export.
- Arguments C may alias with the object (a `git_str`'s own buffer passed as
  data) must be handled: see `str::ffi::seg`.

## Layout tests

Every mirrored or shared-prefix type gets a layout test. At the end of the
module, one entry per type:

```rust
crate::c_layout_test! {
    headers: ["vector.h"],
    types: [
        GitVector => "git_vector" { alloc_size as "_alloc_size", cmp as "_cmp", contents, length, flags },
        Other => "struct git_other" { a, b },
    ],
    asserts: ["GIT_VECTOR_SORTED == 1"],     // optional raw C checks (bitfields, enums)
}
```

It writes `_Static_assert`s of `sizeof`, `_Alignof`, and each field's
`offsetof` and size from Rust's `size_of`/`offset_of!`, and compiles them
with clang using the include paths and `-D` flags of `LIBGIT2_BUILD_DIR`'s
`build.ninja` (`common.h` is included first). It runs with
`--features link-c`; it is ignored otherwise and under Miri.

## Tests

- `let _g = test_support::init();` at the start of every test that touches
  the library: it initializes once per process (the C `git_libgit2_init` with
  `link-c`, the Rust subsystems otherwise) and keeps allocator-replacing
  tests out while held.
- `let g = test_support::exclusive(); test_support::with_alloc_limit(&g, n,
  || …)` runs code under `cl_alloc_limit(n)`.
- Keep testable logic in safe Rust apart from C calls, so Miri runs it;
  mark tests that reach C `#[cfg_attr(miri, ignore)]`.
- `rust/difftest/run.sh` links a C driver against the C reference and the
  Rust build and compares outputs; add drivers there for parsers/encoders.

```sh
LIBGIT2_BUILD_DIR=/work/builds/<wp> cargo test --features link-c
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test
```

## Object ids, maps, pool, pqueue, parse context (WP u1)

| You need | Use | Module |
|---|---|---|
| `git_oid`, `git_oid_t` | `Oid`, `OidType` (crate root) | `crate::oid` |
| `git_oid_shorten` | `OidShortener` | `crate::oid` |
| `git_oidarray`, `git_array_oid_t` | `OidArray`, `GitArray<Oid>`, `oidarray::{add, remove}` | `crate::oidarray` |
| a khash map/set (`hashmap.h`) | `GitHashMap<K, V, X = ()>`, `GitHashSet<K>` | `hashmap` |
| `hashmap_str.h` keys | `CStrKey<'a>`; `GitHashMapStr`, `GitHashSetStr` | `hashmap` |
| `hashmap_oid.h` keys | `&'a Oid`, `Oid`, `OidPtr`; `OidMap<'a, V>`, `OidSet`, `OidPtrMap` | `crate::hashmap_oid` |
| exports of a C-named map (`git_pack_oidmap_*`) | `git_hashmap_exports!` | `hashmap` |
| `git_pool` | `pool::GitPool` | `pool` |
| `git_pqueue` | `pqueue::GitPqueue` | `pqueue` |
| `git_parse_ctx` | `crate::parse::GitParseCtx<'a>` | `crate::parse` |
| utf8 / varint / strlist | `utf8::{iterate, char_length, valid_buf_length}`, `varint::{encode, decode}`, `strlist::*` | |

**`Oid`** is the mirrored `git_oid` (33 bytes, align 1): a `*const git_oid`
from C is a `*const Oid`, and C structs that embed a `git_oid` embed an
`Oid`. It is `Copy`; `==`, `Ord` and `Hash` are over the whole struct (C's
`git_oid_cmp`/`git_oid_equal`). Any byte pattern is valid, so `kind()` is
`Option<OidType>`; size-dependent functions treat an unknown type as C does.

```rust
let id = Oid::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391", OidType::Sha1)?; // git_oid_from_string
let p  = Oid::from_prefix(b"e69de", OidType::Sha1)?;   // git_oid_from_prefix
let r  = Oid::from_bytes(raw, OidType::Sha256);        // git_oid_from_raw (None if short)
id.hex();          // HexOid on the stack: as_str()/as_bytes()/as_c_str() (git_oid_fmt)
id.hex_path()?;    // "e6/9de2…" (git_oid_pathfmt)
id.hex_prefix(8);  // git_oid_tostr(buf, 8, id): 7 digits
id.nfmt(&mut out)?;            // git_oid_nfmt
id.fmt_substr(&mut out, start); // git_oid_fmt_substr
id.ncmp(&p, 5); id.cmp_raw(&p); id.strcmp(b"e69d"); id.is_zero(); id.prefix(7);
id.hashcode();     // git_hashmap_oid_hashcode
```

Errors are `Result<T>` with `ErrorCode` and C's messages (`unable to parse
OID - too short` …); `?` into `Result<_, Error>` gives the rich error.

**`GitHashMap<K, V, X>`** is `GIT_HASHMAP_STRUCT` (with `X` a field C puts
before the members, e.g. `git_index_entrymap`'s `ignore_case`: use
`GitHashMap<_, _, c_uint>` — a `GitHashMap` *after* a `u32` field would be
at offset 8, not 4). It runs khash exactly (tested against the C macros:
`tests/u1_hashmap_c.rs`), so iteration order is the C's, and C's inline
functions and the Rust can work on the same map (arrays from
`git__allocator`). Keys implement `HashKey`; `*_with(hash, eq)` variants
take runtime hash/equality (the index's icase map). `put` replaces key and
value of an equal key (as C) and returns the old pair; `remove` returns the
pair; `iterate(&mut it)` is C's cursor (removal between calls is fine);
`retain` is C's iterate-and-remove loop.

Maps embedded in C-visible structs, found by grepping the C that stays:
- `git_pack_file.idx_cache` (`git_pack_oidmap`, pack.h): `indexer.c` calls
  `git_pack_oidmap_{contains,put,iterate,dispose}`, which `pack.c` defines
  extern: the pack WP must export them (`git_hashmap_exports!`) over a
  `GitHashMap<OidPtr, *mut git_pack_entry>` at the same offset.
- `git_mwindow__pack_cache` (`git_mwindow_packmap`, mwindow.h): a global;
  `tests/libgit2/pack/sharing.c` calls `git_mwindow_packmap_{iterate,size}`,
  defined extern in `mwindow.c`: export them, and the global, with the
  layout of `GitHashMap<CStrKey<'static>, *mut git_pack_file>`.
- `git_hashset_str` (hashmap_str.h): `checkout.c` owns one and passes it to
  `git_futils_mkdir_relative` (`git_mkdir_options.cache_pathset`), whose
  ported Rust then adds to it, while `checkout.c` disposes it with the C
  inline: use `GitHashSetStr<'static>` on the pointer.
- `git_sortedcache.map` (`git_hashmap_str`): only `sortedcache.c` and its
  ported users touch it.
- `git_cache.map` (`git_cache_oidmap`, in `git_repository` by value),
  `git_revwalk.commits`, `git_treebuilder.map`, `git_index.entries_map`:
  only their (ported) modules touch the maps, but the structs are seen by C,
  so keep the layout (`OidPtrMap`/`OidMap` match `git_cache_oidmap`).
- Module-local maps (`odb_mempack`, `grafts`, `config_list`): free to use
  any `GitHashMap` instantiation; use one (not `std::HashMap`) wherever the
  iteration order is observable (`git_grafts_oids` → shallow roots).

**`GitPool`** is the mirrored `git_pool` (C's paging: 4056-byte pages,
8-byte rounding, big requests on their own page). `malloc`, `mallocz`,
`strndup`, `strdup`, `strcat` return `NonNull` pointers valid until
`clear`/drop. **`GitPqueue`** is `#[repr(transparent)]` over `GitVector`:
`insert`/`pop` (`unsafe`: they call the C comparison) or `insert_by`/
`pop_by` with a Rust closure; `as_vector()` for `git_pqueue_get` & co.
**`GitParseCtx<'a>`** is the mirrored `git_parse_ctx` over a `&'a [u8]`:
`line()`, `rest()`, `advance_line/chars/expected/ws/nl/digit/oid`,
`peek(skip_ws)`, `contains`.
