# Batch 4's 28 functions with pointers have no pointer findings

- **finding**: `wavefront/ownership/dispatchers-finish-reset-and-names-without-findings`
- **batch**: `wave-194/batch-6/run-1`, the `symbol` review of `deps/llhttp/api.c`'s folded
  waves
- **items**: `fn::llhttp_finish`, `fn::llhttp_reset`, `fn::llhttp_errno_name`,
  `fn::llhttp_method_name`, `fn::llhttp_status_name` and the 23 `fn::llhttp__on_<slot>`,
  ported by `wave-194/batch-4` (`b3d6b9b37`)
- **files**: `crustify/wavefront/ownership-store.json`

## Finding

The analysis of an item submits its pointer findings to the ownership store, and batches 1, 2
and 3 did so for theirs (`llhttp__internal_execute`, the two callback types,
`llhttp_settings_init`, the 25 functions of batch 3). Batch 4 submitted none: on
`b3d6b9b37`, `query functions --name llhttp_finish` (and each of the 27 others) prints
`"_analysis": {"submitted": false, …}`, every `ptr` slot `null`. The records are incomplete,
not wrong: each dispatcher's parser and run, `llhttp_finish`'s and `llhttp_reset`'s parser,
and the name functions' returned literal have no ownership, nullability or keepalive on
record, which the module's later stages and a module pass query.

Rerun, from the repository root:

```
wavefront $PWD/c --config ../crustify/wavefront/configs/wavefront-config.json \
    query functions --name llhttp__on_url
```

## Fix

The findings are submitted, one `query functions --name <fn> --update <fn>.json` each, the
documents written by `findings.py` beside this report
(`python3 -I findings.py <dir>`): the dispatchers' parser borrowed, mutable and not kept, a
span dispatcher's `p` the run's start, read `endp - p` bytes for the call, and `endp` its
end, never read; an event dispatcher's `p` and `endp` never read; `llhttp_finish`'s and
`llhttp_reset`'s parser borrowed and mutable, `llhttp_reset` given no lifecycle role, since it
reads the parser it re-initialises (`llhttp_init` is the `init` constructor); and the name
functions' return a static literal, never NULL. Each record now reads `"submitted": true`.

The 23 dispatchers' records still list `ptr_args.s.ptr`, `p.ptr` and `endp.ptr` once as
pending: wavefront composes api.c's definition (`llhttp_t *`, `const char *`) and `llhttp.c`'s
own prototype (`llhttp__internal_t *`, `const unsigned char *`) into one signature of six
parameters, two per name, and the store keys a parameter by its name, so one of each pair can
never be filled. That is a defect of wavefront, proposed as
`crustify/evolution/wavefront-two-prototypes-one-signature`, not of the findings.
