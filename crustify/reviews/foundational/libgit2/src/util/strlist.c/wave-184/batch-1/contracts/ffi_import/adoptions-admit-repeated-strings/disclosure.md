To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: StrList::from_raw_parts and NullStrList::from_raw admit one string twice

Hi Marius,

This is an unsound safety contract in the crate's unsafe API. Only a caller of these unsafe
functions can reach it, so it can be handled in public.

In `src/libgit2/src/util/strlist.rs` (commit aba03725e), `StrList::from_raw_parts` asks for
an array of libgit2's allocator "whose first `len` elements are initialised, each NULL or a
NUL-terminated string allocated by the same allocator", and for the caller to "transfer
unique ownership of the array and of those strings". `NullStrList::from_raw` reads the same.
Neither asks the elements to be distinct. An array that holds one string twice honours every
word, and the list's release (`git_strlist_free` / `git_strlist_free_with_null`) then frees
that string twice:

```rust
let (array, len) = StrListFree::copy(&[c"a", c"b"]).unwrap().into_raw_parts();
// slot 1 now holds slot 0's string; its own string was released
let list = unsafe { StrList::from_raw_parts(array, len) }.unwrap();
drop(list);
```

```text
==ERROR: AddressSanitizer: attempting double-free on 0x70345344db30 in thread T0:
    #1 git__free src/util/alloc.h:39:2
    #2 git_strlist_free src/util/strlist.c:92:3
    #3 <StrListFree as CLenDrop>::c_drop_len strlist.rs:178:22
```

The fix adds "no two of them the same string" to both obligations, and a UB test adopts
equal but distinct strings. Both are on the review branch (crustify/waves/wave-184). The
reproducer is under
`crustify/reviews/foundational/libgit2/src/util/strlist.c/wave-184/batch-1/contracts/ffi_import/`.
I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
