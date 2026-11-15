class: nit
evidence: the C contradicts the documentation (`c/src/util/hash/sha1dc/ubc_check.h:39-40`, `c/src/util/hash/sha1dc/sha1.c:1301-1707,1743`)
conditions: any; `src/libgit2/src/util/hash/sha1dc/ubc_check.rs` at `2492d2a9f`, the C at the fork's `230c9dbdb`

# `DvInfo`'s step documents every step in `0..80` as one `sha1.c` saved a state for

**Where.** `src/libgit2/src/util/hash/sha1dc/ubc_check.rs` on `2492d2a9f`:
`DvInfoRef::recompression_step`'s first sentence, "The step `sha1.c` recompresses from, in
`0..80`: the index of the state it saved at that step", and `DvInfo::new`, whose only word on
the step is that one outside `0..80` is refused "where `sha1.c` could not read the record".

**What is wrong.** `sha1.c` saves the state of a step only where `DOSTORESTATE<step>` is
defined, and `ubc_check.h`, this module's header, defines two: `DOSTORESTATE58` and
`DOSTORESTATE65`. `sha1_recompression_step` has a `case` only for those steps and
`default: abort();`, and `sha1_process` calls it with `sha1_dvs[i].testt` (`sha1.c:1743`). So
`new` accepts a step, such as 10, that `sha1.c` would read without going out of bounds but
then abort on, and the getter's text says a state was saved at it. The bound `new` keeps is
the one memory safety needs, the index into `ctx->states[80]`, and is right; what the
documentation leaves out is the narrower set of steps the C recompresses from, which the
batch's own analysis records (`crustify/wavefront/ownership-store.json`, `dv_info_t`'s
`_comment_user`: "`sha1_recompression_step` aborts on a step without `DOSTORESTATE`, 58 and 65
here") but which reached no reader of the API. `sha1.c`'s port, which keeps this type's API,
reads it from there.

Nothing a consumer can observe: no safe API hands a `DvInfo` to C, which reads only
`sha1_dvs`, and every record of the C table has step 58 or 65 (32 records and the terminator,
whose type ends the scan before its step is read). Hence `nit`, with no reproducer.

**Check.**

```
grep -n 'DOSTORESTATE' c/src/util/hash/sha1dc/ubc_check.h          # 39, 40: 58 and 65
sed -n 1700,1707p c/src/util/hash/sha1dc/sha1.c                     # default: abort();
grep -n 'sha1_recompression_step(sha1_dvs' c/src/util/hash/sha1dc/sha1.c   # 1743
```

**Fix.** `DvInfo::new`'s documentation adds that not every step in range is one `sha1.c`
recompresses from, names the two `ubc_check.h` saves, and says `sha1_recompression_step`
aborts on any other and that a table `sha1.c` reads holds those alone; the getter says the
state is saved only for 58 and 65. `new` keeps accepting `0..80`: the narrower set is a
configuration of `sha1.c` (its `DOSTORESTATE` macros), which its port may change, and refusing
by it would tie the type's invariant to that configuration with no safety gain. The API is
unchanged.

**Tiers.** Caught by this review batch (`wave-221/batch-2`), the first tier after the batch
that wrote the type (`wave-221/batch-0`), whose own gates had it in scope.
