Filed by the assistant from 801c9ddbd and 007596d40, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `Allocator::debug`'s size obligation cannot be honoured once the family is installed

- **finding**: `contracts/ffi_import/debug-size-obligation-unenforceable`
- **batch**: `wave-183/batch-1/run-1`, the module pass of
  `libgit2/src/util/allocators/debugalloc.c`, over batch 0 (`fd3e6b8e1`)
- **item**: `fn::git_debugalloc_init_allocator`, bound as `Allocator::debug`
- **affected commit**: `fd3e6b8e1`
- **source commits**: `801c9ddbd` (file), `007596d40` (fix)

## Finding

`Allocator::debug` handed out C's routines unchanged and stated debugalloc.c's unchecked
`len + sizeof(size_t)` as an obligation on calling its members, adding that "installing the
family vouches that nothing else asks it for such a size either". Once installed through
`set_allocator`, the caller is libgit2, which takes sizes from repository bytes, so no
installer can honour it.

## Evidence

A loose object in the old pack-like format whose header names 0xffff_ffff_ffff_fff0 bytes
makes `read_loose_packlike` (odb_loose.c:259) call `git_str_init(.., size + 1)`;
`git_str_try_grow` rounds it to 2^64 - 8 and calls `git__realloc(NULL, ..)`;
`debugalloc__realloc` wraps the block size to 0, `malloc(0)` succeeds, and
`memcpy(newptr, &len, 8)` (debugalloc.c:53) writes past it.

The reproducer exits 134 on `fd3e6b8e1` (`run-affected.log`, beside this report):
"AddressSanitizer: heap-buffer-overflow ... WRITE of size 8 ... #0 debugalloc__realloc
debugalloc.c:53:2 ... #5 read_loose_packlike odb_loose.c:259:3 ... #10 OdbRef::read
odb.rs:1425:27 ... located 0 bytes after 1-byte region".

The same overflow is upstream libgit2's under `-DDEBUG_STRICT_ALLOC=ON`, which its CI runs:
`disclosure.md`, beside this report, is addressed to security@libgit2.com.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `fd3e6b8e1`. Rerun from the
reproducer's dir:

```
CRUSTIFY_BUILD_VARIANT=asan-ubsan \
RUSTFLAGS="-Zsanitizer=address -Cforce-frame-pointers=yes" \
ASAN_OPTIONS=detect_leaks=0:abort_on_error=1 \
cargo +nightly-2026-10-08 run --target x86_64-unknown-linux-gnu
```

## Fix

In `007596d40`: `Allocator::debug` no longer hands out C's malloc and realloc. Its members
are two guards, in debugalloc.rs's ffi_import region, that refuse a request past isize::MAX
bytes with NULL, as a failed allocation, and otherwise call the routine
`git_debugalloc_init_allocator` writes, unchanged; its free is C's own. For the sizes C does
not wrap, glibc's malloc already refuses len + 8 past PTRDIFF_MAX, so only the wrapping
requests change outcome: they now fail as out of memory, as under the standard allocator.
The members' size obligation, and the installer's "vouching", are gone from the docs. The
reproducer exits 0 on this tree (`run-fixed.log`: "read failed: OdbError { .. class:
NoMemory .. }").

Regression tests: ub_unsafe `installed_debug_refuses_a_wrapping_object_size` installs the
value and reads four pack-like objects whose sizes wrap or exceed isize::MAX, each failing
with GIT_ERROR_NOMEMORY under ASan; unit_tests `guards_refuse_sizes_past_isize_max_before_c`
(Miri runs it) and `a_refused_resize_keeps_the_block_and_its_length_word` check the guards'
boundary. equiv's value test now checks C's free and two guards in place of C's
malloc/realloc (`debug_holds_cs_free_and_guards_over_any_value`), and its failure test adds
isize::MAX + 1 and usize::MAX - 8, which both families refuse. Its child process now gets
`allocator_may_return_null=1` through BSAN_OPTIONS too: the wave tip's BSan gate failed it
(equiv, 1 of 486), BorrowSanitizer aborting on a 0x8000000000000007-byte request
("allocation-size-too-big") that ASan and TSan were told to refuse.

The API is kept (ffimap api: `fn::git_debugalloc_init_allocator` and `st::git_allocator`
kept; 17 unattributed modules kept); the members' behaviour changed for the soundness fix,
as the API stability row says.
