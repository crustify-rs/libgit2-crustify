# Reproducer

`bsan_tls_growth.rs` reads one `TlsKey` on one thread, 2000 lookups per chunk, and prints the
cost per lookup of each chunk. Copy it to the crate's `examples/` and run, from the
workspace, `crustify/scripts/cargo-variant.sh bsan run --example bsan_tls_growth -- 2000 8`.
`run.txt` is its output on `d63c80a1f` over `crustify/builds.gen8`. The cost per lookup grows
linearly with the lookups already made, from 4.3 ms to 77.7 ms.
