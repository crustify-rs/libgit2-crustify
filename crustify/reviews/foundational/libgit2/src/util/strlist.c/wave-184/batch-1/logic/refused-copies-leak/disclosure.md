To: libgit2 maintainers, as a public issue at https://github.com/libgit2/libgit2/issues
Subject: git_strlist_copy and git_strlist_copy_with_null leak the array and earlier copies on allocation failure

Hello,

This is a memory leak on an out-of-memory path, not a memory-safety bug, so I am filing it in
public.

In `src/util/strlist.c` (at `0551dfd4a`), both copies allocate the array, then duplicate each
string, checking each allocation with `GIT_ERROR_CHECK_ALLOC`:

```c
dup = git__calloc(len, sizeof(char *));
GIT_ERROR_CHECK_ALLOC(dup);

for (i = 0; i < len; i++) {
	dup[i] = git__strdup(in[i]);
	GIT_ERROR_CHECK_ALLOC(dup[i]);
}
```

`GIT_ERROR_CHECK_ALLOC` returns `-1` at once, so when `git__strdup` of string *i* fails, the
array and the *i* strings already copied are lost: `*out` is not written, and nothing else
holds them. `config_mem.c`'s backend reaches `git_strlist_copy` this way.

A reproducer, run through a Rust binding that called these two functions, installs a
counting allocator through `GIT_OPT_SET_ALLOCATOR` that refuses 5-byte requests, and copies
`["one", "four"]`:

```text
StrListFree::copy: 2 allocations left live by the refused copy
NullStrListFree::copy: 2 allocations left live by the refused copy
```

Releasing what was made before returning fixes it, for instance with
`git_strlist_free(dup, i)` (or `git_strlist_free_with_null(dup)`, since the array is zeroed)
before `return -1`. I can share the reproducer on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
