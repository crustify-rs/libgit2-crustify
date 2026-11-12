Filed by the assistant from ec289165f and 418ca278b, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `Buf` is confined to the thread that made it, and its handle cannot be hashed

- **finding**: `ergonomics/buf-confined-to-its-thread`
- **batch**: `wave-169/batch-3/run-1` of `libgit2/deps/reftable/basics.c`, the review of the
  type route of batches 0 to 2
- **item**: `st::reftable_buf`
- **affected commit**: `bbaafcd09`
- **source commits**: `ec289165f` (file), `418ca278b` (fix)

## Finding

Buf is confined to the thread that made it, and its handle cannot be hashed. reftable's
buffer stands in for a `Vec<u8>` ("reftable_buf maps to Vec<u8>" in modules.json), and
ffibox's G1 asks for the standard traits of the counterpart, with each departure stated.
Two departures are neither stated nor needed:

1. `define_ctype!` withholds Send and Sync, and batches 0 (`85ad07673`) and 2 (`2032628fd`)
   wrote no proof. So `Buf`, `BufVal`, `ReftableBox<Buf>` and both handles stay on one
   thread. Nothing ties a buffer to its thread. Its allocation is reftable's, and the
   module's own Alloc contract requires every hook family to accept a cross-thread resize or
   release, which libgit2's and libc's allocators accept. The `ReftableStr` that
   `BufMut::detach` hands out, over the same allocation, already crosses threads (ub_safe's
   `strings_and_runs_are_released_on_other_threads`). Every operation a shared borrow
   reaches only reads.
2. `BufRef` is Eq and Ord by content (`reftable_buf_cmp`) but not Hash.

## Evidence

On `bbaafcd09` the reproducer does not compile:

```
error[E0277]: `*const ()` cannot be sent between threads safely
  --> src/main.rs:21:17
   = help: within `ffibox::refs::CVal<...::Buf, BufRelease>`, the trait `Send` is not
     implemented for `*const ()`
note: required because it appears within the type `...::basics::Buf`
error: could not compile `buf-confined-to-its-thread` ... due to 2 previous errors
```

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. In
crustify/reviews/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-3/ergonomics/buf-confined-to-its-thread,
run:

```
CRUSTIFY_BUILD_VARIANT=plain cargo run
```

It moves a filled `BufVal` to another thread, which grows it; two threads then read it at
once, and a third releases it. After the fix the reproducer exits 0.

## Fix

In `418ca278b`: in ffi_import, `Buf` and `BufMut<'_>` are Send, and `Buf`, `BufRef<'_>` and
`BufMut<'_>` are Sync, each impl with its proof. A buffer's allocation is reftable's, which
any thread may grow or release (the Alloc contract; libgit2's and libc's allocators). It is
allocated only once libgit2's runtime is pinned, so the thread it moves to reads the hooks
after libgit2 wrote them. A shared borrow reaches only reads. `BufVal` and
`ReftableBox<Buf>` follow from `Buf`. `BufRef` hashes its contents, which is what its Eq
compares. The module documentation states both.

Tests:

- ub_safe `buffers_grow_read_and_release_on_other_threads`, under TSan;
- unit_pub `buffers_hash_by_the_contents_they_compare`; and
- unit_pub `buffers_cross_threads_as_a_vec_does`, which checks the bounds at compile time.

The review found no other defect in the type route of batches 0 to 2. Every safety comment
holds. The ownership store's findings for `reftable_buf` and its lifecycle routines match
the C.
