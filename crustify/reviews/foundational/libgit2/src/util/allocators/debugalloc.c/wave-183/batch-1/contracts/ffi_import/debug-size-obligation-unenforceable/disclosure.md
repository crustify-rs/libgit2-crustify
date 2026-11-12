To: security@libgit2.com
Subject: debugalloc: unchecked size addition writes past a block for a crafted loose object

Hello libgit2 maintainers,

Under `-DDEBUG_STRICT_ALLOC=ON` (the configuration of several CI jobs in `main.yml` and
`nightly.yml`), reading a crafted loose object writes 8 bytes past a heap block.
`src/util/allocators/debugalloc.c` computes each block's size without an overflow check:

```c
static void *debugalloc__realloc(void *_ptr, size_t len, const char *file, int line)
{
	...
	size_t total = len + sizeof(size_t);   /* wraps for len > SIZE_MAX - 8 */
	...
	if ((newptr = malloc(total)) == NULL)
		return NULL;
	...
	memcpy(newptr, &len, sizeof(size_t));  /* line 53 */
```

`debugalloc__malloc` has the same addition. A loose object in the old pack-like format whose
header encodes the size `0xfffffffffffffff0` (bytes `b0 ff ff ff ff ff ff ff ff 0f`) passes the
`GIT_ADD_SIZET_OVERFLOW(&alloc_size, hdr.size, 1)` check in `read_loose_packlike`;
`git_str_try_grow` rounds the request to `SIZE_MAX - 7` and calls
`git__realloc(NULL, SIZE_MAX - 7)`. `total` wraps to 0, `malloc(0)` succeeds, and the length word
is written past the empty block. `git_odb_read` of that object under AddressSanitizer:

```text
ERROR: AddressSanitizer: heap-buffer-overflow
WRITE of size 8 thread T0
    #0 in debugalloc__realloc src/util/allocators/debugalloc.c:53:2
    #1 in git__realloc src/util/alloc.h:29:12
    #2 in git_str_try_grow src/util/str.c:86:12
    #4 in git_str_init src/util/str.c:31:2
    #5 in read_loose_packlike src/libgit2/odb_loose.c:259:3
    #9 in git_odb_read src/libgit2/odb.c:1170:10
located 0 bytes after 1-byte region, allocated by debugalloc.c:42:16
```

With the standard allocator the same read fails cleanly as out of memory. The impact is limited
to builds with the debug allocator, but any repository content such a build reads can trigger it,
and the 0-byte block is then handed out as one of `SIZE_MAX - 7` bytes.

A fix is to refuse the sizes that overflow, in both functions:

```c
	if (GIT_ADD_SIZET_OVERFLOW(&total, len, sizeof(size_t)))
		return NULL;
```

We can share the full reproducer on request.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness
auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and
independently reproduced.
