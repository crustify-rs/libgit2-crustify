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
