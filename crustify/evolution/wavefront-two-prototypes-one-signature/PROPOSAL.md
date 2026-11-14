# wavefront composes a function's definition and a differently typed prototype into one signature

Status: `open` (filed by `wave-194/batch-6/run-1`; no change made).

## Signal

A tool that reports what the source contradicts (`evolution` §2): wavefront gives a function
twice as many parameters as it has, and its findings can never be complete.

## Evidence

api.c defines its 23 dispatchers as `int llhttp__on_url(llhttp_t* s, const char* p, const
char* endp)` (`c/deps/llhttp/api.c:344`), and `llhttp.c`, which calls them, declares each for
itself with the other spelling of the same types, `int llhttp__on_url(llhttp__internal_t* s,
const unsigned char* p, const unsigned char* endp);` (`c/deps/llhttp/llhttp.c:573`). From the
campaign's repository root, with wavefront `bf21e16`:

```
wavefront $PWD/c --config ../crustify/wavefront/configs/wavefront-config.json \
    query functions --name llhttp__on_url
```

prints `"type": "int llhttp__on_url(llhttp__internal_t *, llhttp_t *, const char *, const
unsigned char *, const char *, const unsigned char *)"` and six `ptr_args`, two at each
position 0, 1 and 2, one per declaration. `findings` offers one slot per name, `s`, `p` and
`endp`, which `--update` accepts; after it, `_analysis` reads `"submitted": true, "pending":
["ptr_args.s.ptr", "ptr_args.p.ptr", "ptr_args.endp.ptr"]`, one of each pair, which no
submission can fill. The 22 other dispatchers read the same
(`crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-6/wavefront/ownership/dispatchers-finish-reset-and-names-without-findings`).
The two spellings are one type each (`llhttp_t` is a typedef of `llhttp__internal_t`, and
`char` and `unsigned char` pointers are passed alike), so the function has three parameters.

## Destination

wavefront: the composition of a function's record from its declarations (`docs/artifacts/
syms.md`'s `type` and `ptr_args`), within its goals, a syntactic fact of the build.

## Change

Not made. A function's signature is composed from its definition where it has one, and
otherwise from one declaration; a declaration whose parameters differ from it, by position,
in their spelled types only is recorded as a declaration (`declared_in` already lists
`llhttp.c`), not as more parameters, and `_analysis.pending` counts a parameter by position.
A regression fixture: a function defined in one TU with `typedef struct s s_t;` parameters and
declared in another with `struct s *`, whose record has one parameter.

## Validation

None run.

## Review

None yet.
