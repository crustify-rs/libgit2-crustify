# The safe Rust API of libgit2-rs

`libgit2-rs` is libgit2's core ported to Rust. Besides libgit2's C ABI (for
C programs and the C that stays C), it offers a safe Rust API over the same
code. This is a short guide to it; each item's rustdoc has the details, and
`rust/PORTING.md` §5 the rules the API follows.

```toml
[dependencies]
libgit2-rs = { path = "…/rust/libgit2-rs", features = ["link-c"] }
```

With `link-c` and `LIBGIT2_BUILD_DIR` naming a CMake build of libgit2 with
`USE_RUST=ON`, the build script links the rest of libgit2's C
(`libgit2_c_rest.a`) and the system libraries it needs; cargo passes the
directives on to your crate. There is no initialization call: the library
initializes itself on first use (`Libgit2::init()` does it eagerly, to see
an error early) and is never shut down by the safe API.

Everything is re-exported from the crate root (`libgit2_rs::Repository`,
…), plus the `libgit2_rs::opts` module for the global options. No item has
a raw pointer, an `unsafe fn` or a C type in its signature.

## Errors

Every fallible function returns `libgit2_rs::Result<T>` =
`Result<T, Error>`. An `Error` has libgit2's code, class and message:

```rust
use libgit2_rs::{ErrorClass, ErrorCode, Reference, Repository};

let repo = Repository::open("/path/to/repo")?;
match Reference::lookup(&repo, "refs/heads/nope") {
    Err(e) if e.code() == ErrorCode::NOT_FOUND => {
        assert_eq!(e.class(), ErrorClass::REFERENCE);
        println!("{e}"); // "reference 'refs/heads/nope' not found"
    }
    other => { other?; }
}
# Ok::<(), libgit2_rs::Error>(())
```

Backends and callbacks you write return `Error::new(code, class, message)`.

## Strings and paths

- File-system paths are `impl AsRef<Path>` in and `&Path`/`PathBuf` out.
- Paths inside a repository (index and tree entries), reference and
  configuration names, messages: `impl AsRef<[u8]>` in (`&str` works),
  `&[u8]` out, with `*_str() -> Option<&str>` where UTF-8 is common. Git
  does not require UTF-8, so neither does the API. A NUL byte inside is an
  error.

## Repositories, objects, references

```rust
use libgit2_rs::{Commit, Object, ObjectType, Repository, Tree};

let repo = Repository::open("/path/to/repo")?;     // or discover, init, new_in_memory
let head = repo.head()?;                             // a Reference<'_>
let commit = Commit::lookup(&repo, &head.target().unwrap())?;
println!("{} by {}", String::from_utf8_lossy(commit.summary()), commit.author());
let tree: Tree<'_> = commit.tree()?;
for entry in tree.iter() {
    println!("{:o} {}", entry.filemode(), String::from_utf8_lossy(entry.name()));
}
// Any object, then the kind you need:
let obj = Object::lookup(&repo, commit.id(), None)?;  // None: any type
let commit: Commit<'_> = obj.try_into().map_err(|_| unreachable!())?;
# Ok::<(), libgit2_rs::Error>(())
```

Objects borrow their repository (`Commit<'repo>`), so they cannot outlive
it. `ObjectType` is `Commit`, `Tree`, `Blob` or `Tag`; "any type" is
`None` where an `Option<ObjectType>` is taken. Signatures read from objects
are `SignatureRef<'_>` views; `Signature` is the owned kind you make
(`Signature::now`, `Signature::new(name, email, Time::new(secs, offset))`).

Creating objects:

```rust
use libgit2_rs::{CommitBuilder, Index, IndexEntry, ObjectType, Repository, Signature};

let repo = Repository::init("/tmp/new-repo", false)?;
let blob = repo.odb()?.write(b"hello\n", ObjectType::Blob)?;
let index = repo.index()?;
index.add(&IndexEntry::new("hello.txt", 0o100644, blob))?;
let tree = index.write_tree()?;
let me = Signature::now("Me", "me@example.com")?;
let id = CommitBuilder::new(tree, &me, &me, b"first\n")
    .update_ref(b"HEAD")
    .write(&repo)?;
# let _ = id;
# Ok::<(), libgit2_rs::Error>(())
```

References: `Reference::lookup`, `create`, `symbolic_create`, `resolve`,
`peel(Option<ObjectType>)`, `rename`, `delete`; `References::new`/`glob`
iterate; `Reflog` reads and writes reflogs.

History: `Revwalk::new(&repo)`, `push`/`hide` ids, refs, globs and ranges,
`sorting(Sort::TOPOLOGICAL | Sort::TIME)`, then iterate the ids.

## The object database, configuration, index

- `Odb` (a repository's, or `Odb::new`/`open` alone) reads, writes and
  streams objects; `Backend::loose`, `pack`, `mempack` and
  `Backend::custom(impl OdbBackend)` add backends.
- `Config` (a repository's, `Config::open_default`, or built from files,
  memory and `ConfigBackend`s) gets and sets typed values, iterates
  entries, takes snapshots.
- `Index` (a repository's, or `Index::new`/`open` alone): entries,
  conflicts, resolve-undo records, `add_all`/`update_all`/`remove_all`,
  `read_tree`/`write_tree`.

## Threads

What libgit2 lets several threads share is `Send + Sync`: `Repository`,
`Odb`, `Config`, `Reference`, ids, errors, options. The index is
single-threaded in C; the safe `Index` takes its own lock for every
operation, so it too is `Send + Sync`, and `Repository::index(&self)` can be
used while objects and references are in use, from several threads:

```rust
use libgit2_rs::Repository;

let repo = Repository::open("/path/to/repo")?;
let index = repo.index()?;
std::thread::scope(|s| {
    s.spawn(|| index.len());
    s.spawn(|| repo.head().map(|h| h.target()));
});
# Ok::<(), libgit2_rs::Error>(())
```

Iterators and streams are driven by one thread at a time (`Send`, not
`Sync`). `tests/api_send_sync.rs` states each type's traits.

## Callbacks and backends

Callbacks are closures; returning `false` (or a "stop" variant) stops the
operation, which then returns `Ok`. A panic in a closure stops the
operation and is resumed when it returns. Backend traits (`OdbBackend`,
`ConfigBackend`, `RefdbBackend`) have a default for every method, run on
whichever thread uses them (`Send + Sync`), and may panic, even in `Drop`.

## Options

```rust
use libgit2_rs::{ConfigLevel, opts};
opts::set_mwindow_mapped_limit(256 << 20);
opts::enable_strict_object_creation(false);
opts::set_search_path(ConfigLevel::Global, Some("/etc/my-git"))?;
# Ok::<(), libgit2_rs::Error>(())
```

## What is not here

The safe API covers what is ported (objects, odb, refs, config, index,
repository, revwalk). Diff, merge, checkout, status, remotes and the other
parts that stay C have no safe API yet; `/work/reviews/api-coverage.md`
lists, per C function, the Rust counterpart or why there is none.
