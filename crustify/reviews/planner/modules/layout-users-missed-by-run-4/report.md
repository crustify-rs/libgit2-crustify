# Seven types ported where C of another module still uses their layout

## What the earlier plan decided

Planner run 4 decided each struct's and union's first objective from its own layout query
(`crustify/artifacts/planner/run-4/layout-uses/`) and from `t2/fa_with_root.csv`'s field
accesses. Run 5 kept that decision. It planned seven types `port` where the order places
them, as though no layout user of another module remained:

```json
// wave 568, src/util/sortedcache.c, sublayer 1
{"name": "git_sortedcache", "defined_in": "src/util/sortedcache.h", "kind": "type", "field_anchors": ["pool", "path", "rc", "lock", "map", "stamp", "items", "free_item_payload", "free_item", "item_path_offset"], "home": "src/libgit2/src/util/sortedcache.rs", "objective": "port"}
// wave 713, src/libgit2/transports/smart_pkt.c, sublayer 2
{"name": "git_pkt_err", "defined_in": "src/libgit2/transports/smart.h", "kind": "type", "field_anchors": ["type", "len", "error"], "home": "src/libgit2/src/libgit2/transports/smart_pkt.rs", "objective": "port"}
{"name": "git_pkt_ng", "defined_in": "src/libgit2/transports/smart.h", "kind": "type", "field_anchors": ["type", "msg", "ref"], "home": "src/libgit2/src/libgit2/transports/smart_pkt.rs", "objective": "port"}
{"name": "git_pkt_ok", "defined_in": "src/libgit2/transports/smart.h", "kind": "type", "field_anchors": ["type", "ref"], "home": "src/libgit2/src/libgit2/transports/smart_pkt.rs", "objective": "port"}
{"name": "git_pkt_unpack", "defined_in": "src/libgit2/transports/smart.h", "kind": "type", "field_anchors": ["type", "unpack_ok"], "home": "src/libgit2/src/libgit2/transports/smart_pkt.rs", "objective": "port"}
// wave 1535, src/util/hash/collisiondetect.c, sublayer 0
{"name": "git_hash_sha1_ctx", "defined_in": "src/util/hash/collisiondetect.h", "kind": "type", "field_anchors": ["c"], "home": "src/libgit2/src/util/hash/collisiondetect.rs", "objective": "port"}
// wave 1536, src/util/hash/openssl.c, sublayer 0
{"name": "git_hash_sha256_ctx", "defined_in": "src/util/hash/openssl.h", "kind": "type", "field_anchors": ["c"], "home": "src/libgit2/src/util/hash/openssl.rs", "objective": "port"}
```

## The evidence against it

Wavefront 2a95e3c records layout uses in its own T2 table, `t2/layout_uses.csv`. The
orchestrator extracted it on 2026-11-08 from the unchanged database: every other T1 and
T2 table is byte-identical to `crustify/tmp/old-tables-20261108/`. Wavefront closes each
type's layout users over by-value embedding, and the plan schedule names each type's last
users as its `opaque_after`:

```sh
cd /target/libgit2-bindings
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --with-enums --output /target/libgit2-bindings/crustify/tmp/planner-run-6/schedule-port.json
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  query types --name git_pkt_ng --layout-users
/opt/venv/bin/python crustify/artifacts/planner/run-6/compose/users.py     # every type's whole user list
python3 -I crustify/artifacts/planner/run-6/compose/stages.py              # the verdicts below
```

For each of the seven, `opaque_after.closure.modules` names modules other than the type's
own. The run-4 composition missed those users for two reasons.

1. **Field accesses through a cast or an access chain.** Run 4 credited each row of
   `fa_with_root.csv` to its `root_struct_name`, the struct at the root of the access
   chain, and not to its `struct_name`, the struct whose field is accessed. A packet is
   read through a cast from `git_pkt *`, so the accesses count for `git_pkt`, not for the
   packet type:

   - `src/libgit2/transports/smart_protocol.c:873`: `git__strdup(((git_pkt_ok *)pkt)->ref)`
     (`add_push_report_pkt`); `:883-884` `((git_pkt_ng *)pkt)->ref`, `->msg`; `:892`
     `((git_pkt_unpack *)pkt)->unpack_ok`; `:72` and `:1005` `((git_pkt_err *)pkt)->error`
     (`git_smart__store_refs`, `parse_report`).
   - `src/libgit2/refdb_fs.c:612`, `:618` and `:631`: `backend->refcache->path` in
     `packed_map_check`. `fa_with_root.csv` roots this chain at `refdb_fs_backend`.

   Run 4's own query found only the packet constructors' `sizeof`, in `smart_pkt.c` itself.

2. **By-value embedding through an anonymous union.** `src/util/hash.h:28-31` holds both
   contexts by value in `git_hash_ctx`, a type of `src/util/hash.c`:

   ```c
   typedef struct git_hash_ctx {
   	union {
   		git_hash_sha1_ctx sha1;
   		git_hash_sha256_ctx sha256;
   	} ctx;
   ```

   `t2/layout_uses.csv` records both rows as `embed` uses by `git_hash_ctx` (lines 30
   and 31). Run 4's query has no row for either, because it does not see the member
   through the anonymous union. So the 147 layout users `git_hash_ctx` has in other modules
   never counted toward the two contexts. Those users include `git_filebuf`,
   `git_indexer`, `git_indexwriter` and `config_file_backend`, which embed it in turn.

The other 579 structs and unions agree with run 4's decision: 339 are `port` where the
order places them and 240 are wrap followed by a later port.

## The edit and what it moves

Each of the seven is `wrap` where the order places it. It is ported in a `port/layout`
item once the modules its `opaque_after` names have been integrated, as the scheduling
spec's Later stages section describes:

| type | first stage | `port/layout` | waits for the integration of |
|---|---|---|---|
| `git_hash_sha1_ctx` | wave 1570 (was 1535) | wave 1728 | `apply.c`, `cherrypick.c`, `commit_graph.c`, `midx.c`, `odb_loose.c`, `rebase.c`, `refdb_fs.c`, `revert.c`, `transports/smart_protocol.c` |
| `git_hash_sha256_ctx` | wave 1571 (was 1536) | wave 1730 | the same nine |
| `git_sortedcache` | wave 1572 (was 568) | wave 1967 | `refdb_fs.c` |
| `git_pkt_err`, `_ng`, `_ok`, `_unpack` | wave 1573 (was 713) | wave 2083 | `transports/smart_protocol.c` |

Waves 568, 713, 1535 and 1536 are removed, and waves 1570-1573 replace them with the same
items: the seven types become `wrap`, and their other items stay `port`. The waves that
depended on the removed waves now depend on their replacements. No module, no module
order and no other item moves.

## Questions for the user

None.
