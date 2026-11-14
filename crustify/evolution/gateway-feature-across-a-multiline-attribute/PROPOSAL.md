# build.rs reads a gateway as switched when a multi-line attribute follows its `cfg`

Status: `open`.

## Signal

A defect, worked around. `build.rs`'s `gateway_lines` finds each `mod ffi_export` of `src/`
and the export feature gating it, which `gateways` and `export_dynamic_symbols` read: a
gateway with no feature is a switched module's, which the `system` feature refuses to link
an install beside, and whose `#[unsafe(no_mangle)]` items every binary exports dynamically;
a gated one counts only when its feature is on. It takes the feature from a `#[cfg(feature =
"ffi-export-…")]` line and keeps it across the attribute lines after it, but resets it on
any line that neither starts with `#[` nor `//`. The second line of a multi-line attribute,
such as the region's own `#[expect(\n    unsafe_code,\n    reason = …\n)]`, the toolchain's
form for a region that takes the lint back, is such a line, so a gateway written as the
conventions have it, its `cfg` first, is read as having no feature: as a switched module's,
whether or not the feature is on.

## Evidence

- The reproducer below, from `wave-197/batch-0/run-1`, transcribes `gateway_lines` line for
  line. On a gateway written `cfg`
  then a multi-line `expect`, it reads no feature (`[None]`); with the `cfg` right above the
  `mod`, it reads `DEPS_LLHTTP_HTTP_C`. On `src/libgit2/deps/llhttp/api.rs` at `b3d6b9b37`
  (`wave-194/batch-4/run-1`, before api.c's switch), whose gateway was gated so, it reads
  `[None]`: from that batch to the switch, a `system` build would have refused, naming
  `api.rs` as switched, and every build in shared mode would have passed
  `-Wl,--export-dynamic-symbol=` for api.c's 31 exports with the feature off, whose names
  the C defined.
- `wave-197/batch-0/run-1` (`deps/llhttp/http.c`) wrote its gateway with the `cfg` right
  above `mod ffi_export`, after the `expect`, with a comment saying why.

## Destination

The campaign's `build.rs` (`gateway_lines`), which the `bindgen` skill's Build modes
section specifies: a gateway's feature is the `cfg` among the attributes of its `mod`,
wherever it stands among them.

## Change

Not made: `build.rs` is the bindings', and not a translator's to change. In
`gateway_lines`, track an attribute until its brackets close, so that the lines inside a
multi-line attribute neither set nor reset the gate:

```rust
// in the loop, before the `cfg` match:
if attr_depth > 0 {
    attr_depth += braces_of(t, '[', ']');
    continue;
}
if t.starts_with("#[") && braces_of(t, '[', ']') > 0 {
    attr_depth = braces_of(t, '[', ']');
    continue;
}
```

with `braces_of` counting the opening less the closing brackets, as `braces` counts braces.
A regression test of `gateway_lines` over both orders, the reproducer's two cases, keeps it.

## Reproducer

```python
"""build.rs's gateway_lines, line for line: the feature it reads for each `mod ffi_export`."""
def gateways(text):
    gate = None; out = []
    for line in text.splitlines():
        t = line.strip()
        if t.startswith('#[cfg(feature = "ffi-export-') and t.endswith('")]'):
            gate = t[len('#[cfg(feature = "ffi-export-'):-3].upper().replace('-', '_')
        elif t.startswith('mod ffi_export') or t.startswith('pub mod ffi_export'):
            out.append(gate); gate = None
        elif not t.startswith('#[') and not t.startswith('//') and t:
            gate = None
    return out
cfg_first = '#[cfg(feature = "ffi-export-deps-llhttp-http-c")]\n#[expect(\n    unsafe_code,\n    reason = "x"\n)]\nmod ffi_export {\n}\n'
cfg_last = '#[expect(\n    unsafe_code,\n    reason = "x"\n)]\n#[cfg(feature = "ffi-export-deps-llhttp-http-c")]\nmod ffi_export {\n}\n'
assert gateways(cfg_first) == [None]                    # the defect
assert gateways(cfg_last) == ['DEPS_LLHTTP_HTTP_C']
```

## Validation

None yet: the reproducer above shows the defect; the change's test is its two cases.

## Review

None yet.
