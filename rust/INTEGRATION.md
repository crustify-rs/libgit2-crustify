# L-wave integration log

The integration branch is `integ` (`/work/wt/integ`, build dir
`/work/builds/integ`). "Green" means: every offline clar suite passes
(`runtests.sh`), `cargo test --features link-c` passes, `cargo +nightly miri
test` passes (with `-Zmiri-strict-provenance`), `cargo clippy --all-targets`
(with and without `link-c`) and `cargo fmt --check` are clean.

## 1. l-repo + l-odb (already merged on master at `c7f5662`)

State found: 3 clar suites failing (`odb::backend::mempack`,
`odb::backend::loose`, `refs::iterator`), all from
`git_repository_wrap_odb`: l-repo read `odb->options.oid_type` at byte 60
(the old C layout of `struct git_odb`); l-odb moved it to byte 20 (only `rc`
and `options` stay C-visible).

Resolved (PENDING entries between l-repo and l-odb, deleted):

- `repository/handles.rs` `Odb` (stand-in owner over `git_odb_*` C
  symbols, with `oid_type_raw` reading the fixed offset 60,
  `ODB_OID_TYPE_OFFSET`): removed. The repository holds `crate::odb::GitOdb`
  (`_odb: AtomicPtr<GitOdb>`), creates it with `GitOdb::new`, and calls
  `set_caps`, `add_default_backends`, `exists_ext`, `oid_type` directly. The
  safe `Repository::odb`/`set_odb`/`wrap_odb` take and return the odb WP's
  safe `crate::Odb` (new crate-internal `Odb::{from_raw, new_ref, as_raw,
  into_raw, inner}`). New `GitRepository::odb() -> Result<&GitOdb>`.
- `sys/pending/l_repo.rs` PENDING(l-odb): `git_odb_new_ext`, `git_odb_free`,
  `git_odb_add_disk_alternate`, `git_odb__set_caps`,
  `git_odb__add_default_backends`, `git_odb_exists`, `git_cache_init`,
  `git_cache_clear`, `git_cache_dispose`, `git_cache_set_max_object_size`,
  and the globals `git_cache__enabled`, `git_cache__max_storage`,
  `git_cache__current_storage`, `git_odb__strict_hash_verification`,
  `git_odb__packed_priority`, `git_odb__loose_priority`: removed;
  `settings.rs` uses `crate::cache::*` / `crate::odb::*` (and
  `cache::set_max_object_size`).
- `git_repository.objects`: was an opaque `[u64; 13]` initialized through
  `git_cache_init`; now `crate::cache::GitCache` by value (same layout, the
  layout test checks it), written in place at allocation, cleared with
  `GitCache::clear`, disposed with `GitCache::dispose`.
- `odb.rs` PENDING(l-repo) `REPOSITORY_OBJECTS_OFFSET` (the repository's
  cache read at byte 32 of the owner): now `GitRepository::objects()`.
- `sys/pending.rs` PENDING(l-repo) `git_repository__configmap_lookup` (for
  `GIT_ODB_CAP_FROM_OWNER`) → `GitRepository::configmap_lookup`;
  `git_repository__fsync_gitdir` → `repository::fsync_gitdir()` (the Rust
  static). `GIT_CONFIGMAP_FSYNCOBJECTFILES` moved to `repository` (layout
  test asserts it).
- Duplicated constants: l-repo's `GIT_ODB_CAP_FROM_OWNER`,
  `GIT_OBJECT_{ANY,COMMIT,TREE}` now come from `crate::odb`.
- C ABI unchanged (`git_repository_odb`, `_odb__weakptr`, `_set_odb`,
  `_wrap_odb` take `git_odb *` typed as `*mut GitOdb`).
