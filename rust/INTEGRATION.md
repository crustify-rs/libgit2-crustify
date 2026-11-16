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

Green: `63818f6` (clar 436/436, cargo test link-c, Miri, clippy, fmt).

## 2. l-config (merge `08c61a6`)

Merge conflict: `sys/sysdir.rs` (added by both l-repo and l-config):
combined into one module (template dir, get/set, find_global/xdg/system,
expand_homedir). `merge_wp.sh`'s union of `sys/pending.rs` dropped a
closing brace (fixed by hand).

Resolved (PENDING entries between l-config and l-repo, deleted):

- l-config's `sys/pending.rs` PENDING(l-repo): the opaque
  `git_repository` (now `crate::repository::GitRepository`),
  `GIT_REPOSITORY_ITEM_GITDIR`, `GIT_CONFIGMAP_CACHE_MAX`,
  `GIT_REPOSITORY_CONFIGMAP_CACHE_OFFSET` + `repository_configmap_cache`
  (fixed offset 272) → `GitRepository::configmap_cache()`;
  `git_repository_config__weakptr` / `repository_config_weakptr` →
  `GitRepository::config_weak()`; `git_repository__item_path` /
  `repository_gitdir` → `GitRepository::item_path(…, ITEM_GITDIR)`;
  `git_repository_refdb` (in `repository_head_symbolic_target`) →
  `GitRepository::refdb_weak()`. `config_cache`'s
  `repository_configmap_lookup` / `configmap_lookup_cache_clear` and
  `config::rename_section` now take `&GitRepository` (safe).
- l-repo's `sys/pending/l_repo.rs` PENDING(l-config): every `git_config_*`
  declaration (new, free, open_ondisk, open_default, open_level,
  add_file_ondisk, set_writeorder, snapshot, get_bool/int32/entry,
  __lookup_entry, entry_free, get_multivar_foreach, foreach_match,
  set_int32/bool/string, delete_entry, __update_entry, __get_path,
  __get_string_buf, parse_bool, __find_system/global/xdg,
  __global_location, `git_repository__configmap_lookup`,
  `_lookup_cache_clear`) and the mirrored `ConfigEntryRaw`: removed.
- `repository/handles.rs` `Config`, `ConfigRef`, `ConfigEntry`,
  `ConfigEntryView` (stand-ins over the C symbols): removed. The repository
  holds `_config: AtomicPtr<GitConfig>`; `config_weak()` returns
  `&GitConfig`; owners are l-config's safe `crate::Config` (new
  crate-internal `Config::{from_box, from_raw, new_ref, as_raw, into_raw,
  inner}`); entries are `config::EntryBox` / `EntryView`. New
  crate-internal `GitConfig::{with_value, get_bool, get_int32, get_path,
  get_string_buf, set_bool, set_int32, set_int64, foreach_match_with,
  multivar_foreach_with}` and `config::{find_global, find_xdg,
  find_system}`, which the C exports now share. The safe
  `Repository::config`/`config_snapshot`/`set_config` use `crate::Config`.
- Duplicated constants: `GIT_CONFIG_LEVEL_*` in `repository` re-export
  `crate::config`'s.
- C ABI unchanged (`git_repository_config*`, `_set_config` typed
  `*mut GitConfig`).

Left (PENDING(l-refs)): `sys/pending.rs` `git_refdb_lookup` and
`repository_head_symbolic_target` (config_file's `onbranch:` condition;
uses l_repo's `git_reference_*` declarations, which the duplicates in the
l-config section were folded into).

Re-merge of `wp/l-config` review r1 fixes (`de91683`): merge `4162edd`.
Conflict in `git_config_set_int64`'s export: kept the integration's
version (formats on the stack through `GitConfig::set_int64`).

Green: `bbdbe25` (Miri unit tests; its doctest run picked up the re-merged
sources and is superseded), `4162edd` (clar 436/436, cargo test link-c,
clippy, fmt; Miri: see below).

## 3. l-index (merge `9fc5715`)

Merge conflicts: `sys/path.rs` and `sys/submodule.rs` (added by both
l-repo and l-index): combined (`validate_str_length` passes its `GitStr`
as l-index's `ConstStr` mirror of `git_str`); `sys/pending.rs` union lost a
brace again.

Resolved (PENDING entries between l-index and l-repo, deleted):

- l-index's `sys/pending.rs` `l_index` PENDING(l-repo):
  `git_repository__configmap_lookup`, `git_repository_workdir`,
  `git_repository_is_bare`, `git_repository_workdir_path`,
  `git_repository_open`, `git_repository_head`, `git_repository_free`,
  `git_repository_index__weakptr`. `index::foreign::Repo` now wraps
  `NonNull<GitRepository>` (`Repo::get() -> &GitRepository`) and calls
  `configmap_lookup`, `workdir()`, `is_bare()`, `workdir_path`,
  `index_weak`; `head_target_of` opens with `repository::open_owned` and
  reads `head_ref()` (the sub-repository is now also freed when HEAD
  fails; C leaks it).
- l-repo's `l_repo.rs` PENDING(l-index): `git_index_open_ext`,
  `git_index_free`, `git_index_set_caps`, `git_index__enforce_unsaved_safety`,
  and the `git_index_options` mirror `OidTypeOptions`: removed (settings
  uses `crate::index::git_index__enforce_unsaved_safety`).
- `repository/handles.rs` `Index` stand-in: removed. `_index:
  AtomicPtr<GitIndex>`; `index_weak()` opens with `index::ops::open_ext`
  and calls `ops::set_caps(…, FROM_OWNER)`; releases go through
  `IndexPtr::decref`. Safe `Repository::index`/`set_index` use
  `crate::index::Index` (new crate-internal `Index::{new_ref, as_raw}`,
  `IndexPtr::as_nonnull`). Duplicate `GIT_INDEX_CAPABILITY_FROM_OWNER`
  removed.
- C ABI unchanged (`git_repository_index*`, `_set_index` typed
  `*mut GitIndex`).

Green: `af822f1` (clar, cargo test link-c, clippy, fmt; Miri queued).

## 4. l-repo review r1 fixes (merge `8c47cab`)

Conflicts in `repository/{api,handles,open}.rs` (and a bad automatic merge
of `GitRepository::free` / the setters in `mod.rs`, rewritten by hand):
the setters, `cleanup`, `set_workdir`/`set_bare`/`set_objectformat` take
l-repo's `unsafe` contracts over the integration's Rust types; the safe
`Repository::{odb, config, config_snapshot, index}` return
`RepoHandle<'_, T>` (borrows the repository, `!Send`/`!Sync`, `Deref` to
`crate::{Odb, Config, Index}`); `set_odb`/`set_config`/`set_index`/
`wrap_odb` leave the safe API as on the branch. `repository::open_owned`
returns l-repo's `OwnedRepo`. The branch's publish/set_owned race tests
use the `refdb` slot (the only one still `AtomicPtr<c_void>`). Open
soundness gap of `RepoHandle` (a `Clone` of the inner type escapes the
lifetime): `/work/reviews/integration-notes.md` I-2.

## 5. l-pack (merge `e.g. below`)

`merge_wp.sh` union of `sys/pending.rs` resurrected the l-repo entries of
l-odb's section that step 1 had deleted (l-pack had edited the same
lines); rebuilt from integ's version minus what l-pack removed
(`git_odb_backend_pack`, `odb_backend_pack`, `git_packfile_stream_read`:
l-pack had already replaced the l-odb↔l-pack PENDING entries). The rest
merged cleanly (l-pack had merged master with l-odb and l-repo).

Resolved (PENDING between l-pack and l-repo, deleted): `l_repo.rs`
PENDING(l-pack) `git_mwindow_global_init` (init table now
`crate::mwindow::ffi::git_mwindow_global_init`), `git_mwindow__window_size`,
`__mapped_limit`, `__file_limit`, `git_disable_pack_keep_file_checks`
(settings uses `crate::mwindow::*` / `crate::pack::*`). `sys::indexer`'s
unused `Indexer::append` (dead now that `sys` is crate-private) removed;
`DEFAULT_MAX_OBJECT_SIZE` is Miri-only.
