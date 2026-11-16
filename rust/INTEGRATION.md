# L-wave integration log

The integration branch is `integ` (`/work/wt/integ`, build dir
`/work/builds/integ`). "Green" means: every offline clar suite passes
(`runtests.sh`), `cargo test --features link-c` passes, `cargo +nightly miri
test` passes (with `-Zmiri-strict-provenance`), `cargo clippy --all-targets`
(with and without `link-c`) and `cargo fmt --check` are clean.

## Status (latest first)

- **`1172dfc`/`dec005c`** (2026-11-16): merged on `integ`: l-repo, l-odb, l-config,
  l-index, l-pack, l-commit, l-revwalk, l-refs, each with its review fixes
  as of the coordinator's messages (l-config `de91683`, l-index `6aacd37`,
  l-pack `1822f62`, l-commit `df73db3`, l-repo `2b5638b`, l-revwalk
  `06cc18b`, l-odb `686da6b`/`32eaa1a`, l-refs `f58892b`). Not merged: l-obj
  (not finished). Fully green commits (clar 436/436, cargo test link-c,
  Miri, clippy, fmt): `63818f6`, `4162edd`, `af822f1`, `076ca1b`,
  `1172dfc` (= `04e2c73` + comments). `dec005c` (constants re-exported
  from their owners) and `2d2cd18` (log): clar, cargo test, clippy, fmt
  green; Miri not re-run. Not Miri-green: `b1f31de`..`8324cc3` (l-commit's
  Miri-only test did not compile; fixed in `4394660`/`fb7b34f`). PENDING between merged WPs: all resolved but
  `refs_api::Repository` (integration notes I-6). The other PENDING markers
  are PENDING(l-obj) (unmerged).

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

## 5. l-pack (merge `c1de8ce`, resolution `b9b6032`)

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

Re-merges: `wp/l-index` review fixes (`076ca1b`, clean); `wp/l-pack`
review fixes incl. the `GitMutex::try_lock` fix (clean).

## 6. l-commit (merge `e52ac9a`; resolution `b1f31de`; re-merge of its review fixes `e66486d`; `d0eeeed`)

Merge conflicts: `sys/{commit_graph,diff,path}.rs` (both sides declared
different functions of the same C modules): combined with one declaration
per C symbol (`path::is_valid` builds l-index's `ConstStr`);
`src/util/rust_shims.c` and `sys/pending.rs`: the union dropped closing
braces (fixed by hand). Before the resolution, 40 clar suites crashed
(SIGSEGV): l-commit read `git_index` at the old C offsets (integration
notes I-1).

Resolved (PENDING entries between l-commit and l-repo/l-odb/l-index,
deleted):

- `sys/pending.rs` `l_commit`: the opaque `git_repository`, `git_odb`,
  `git_odb_object`, `git_index`, `git_tree_cache` are now aliases of
  `GitRepository`, `GitOdb`, `GitOdbObject`, `GitIndex`, `GitTreeCache`;
  its `GitCachedObj` and `GitIndexEntry` mirrors are gone
  (`crate::cache::GitCachedObj`, `crate::index`'s entries).
- Fixed offsets `REPO_OID_TYPE`, `REPO_GRAFTS`, `REPO_SHALLOW_GRAFTS`
  (and the branch's later atomic-load helpers `repo_oid_type`,
  `repo_grafts`), `INDEX_OID_TYPE`, `INDEX_FLAGS`, `INDEX_TREE`,
  `INDEX_TREE_POOL`, `TREE_CACHE_ENTRY_COUNT`, `TREE_CACHE_OID`: replaced by
  `GitRepository::{oid_type_raw, grafts_ptr, shallow_grafts_ptr}` and
  `GitIndex::{oid_type, tree, tree_pool, ignore_case()}` /
  `GitTreeCache::{entry_count, oid}`; their layout asserts removed.
- C declarations of `git_odb_read`, `git_odb_object_{free,dup,data,size,id,type}`,
  `git_odb_write`, `git_odb__freshen`, `git_odb__get_commit_graph_file`,
  `git_repository_{open,free,odb__weakptr,head_tree,head,commit_parents,index}`,
  `git_index_{entrycount,get_byindex,has_conflicts,__set_ignore_case,write_tree,free}`,
  `git_tree_cache_{get,read_tree}`: removed; the call sites call the
  owning modules' Rust exports (`crate::odb::ffi::…` etc.); `repo_odb` uses
  `GitRepository::odb_weak`.
- `pending_repository::Repository` (stand-in for l-repo's): removed;
  `commit::api` / `tree::api` take `crate::Repository` (new crate-internal
  `Repository::raw_ptr`).
- l-index's `l_index` PENDING(l-commit): `git_tree__write_index`,
  `git_tree_walk`, `git_tree_entrycount`, `git_tree_entry_{byindex,filemode,
  filemode_raw,name,id}`, `TreewalkCb`: removed; `index::foreign` calls
  `crate::tree::ffi`.

Left (PENDING): l-commit → l-obj (object lookup/free/dup, `git_object__from_raw`,
type names, signatures, grafts), → l-refs (reference lookup/update/peel),
→ l-revwalk (`git_revwalk__commit_lookup`, `git_revwalk` offsets 0/8/56).
The l-obj/l-refs symbols are declared by several sections with different
opaque types (`#[allow(clashing_extern_declarations)]` until they land).

`build.rs`: without `link-c`, the system zlib and PCRE2 are linked
(`8324cc3`): the plain `cargo test` now links and passes the 20 test
binaries that need no libgit2 C; the lib test binary and 11 integration
tests (`config_api`, `delta_*`, `l_index_*`, `l_pack_*`, `odb_*`,
`util_fs`) still need `--features link-c` (they reach the C rest).

## 7. l-revwalk (merge + resolution `4394660`, `3613d75`, `fb7b34f`)

Merge conflicts: `sys/{diff,pathspec,revparse}.rs` (added by both sides):
combined, one declaration per C symbol, l-revwalk's names as aliases of
the existing mirrors (`git_diff_options`, `git_pathspec`); `sys/pending.rs`
rebuilt (the union lost braces again); `lib.rs`: duplicate re-exports.

Resolved (PENDING entries, deleted):

- l-revwalk → l-repo/l-odb/l-commit (`sys::pending::l_revwalk`): the opaque
  `git_repository`, `git_odb`, `git_commit`, `git_tree`, `git_tree_entry`
  are the Rust types; the declarations of `git_repository_{odb,open,free}`,
  `git_odb_free`, `git_commit_list_parse`, `git_commit_{tree,parent,
  parentcount}`, `git_tree_entry_{bypath,free,id}` go (calls go to the
  owners' Rust exports); the `c_stubs` test definitions of those (marked
  "remove at integration") are removed; `revwalk::api::Repository`
  (stand-in) is now `crate::Repository`; `revwalk/node.rs`'s mirrors of
  `git_commit_list_node` / `git_commit_list` are l-commit's
  `GitCommitListNode` / `GitCommitList`.
- l-commit → l-revwalk: the `git_revwalk` offsets `REVWALK_REPO` (0),
  `REVWALK_ODB` (8), `REVWALK_COMMIT_POOL` (56), the `field`/`field_ptr`
  helpers, the opaque `git_revwalk` and the `git_revwalk__commit_lookup`
  declaration: replaced by `GitRevwalk.prefix.{repo, odb, commit_pool}`
  and `crate::revwalk::ffi::git_revwalk__commit_lookup`; the
  `c_foreign_layout` test (no foreign offsets left) is removed. l-commit's
  Miri test of the quick parse, which faked a walk by byte offsets and
  defined `git_revwalk__commit_lookup` itself (now a duplicate symbol),
  runs over a real `GitRevwalk::with_repo` (made `pub(crate)`) and a zeroed
  `GitRepository`.

## 8. l-refs (merge `6d069e0`; re-merge of review fixes `18afb35`; resolution `1ea897c`, `f621775`, `04e2c73`)

Also re-merged on the way: `wp/l-odb` review fixes (`77c7e3e`;
`GitRefcount::own` became `unsafe`: the repository's owner contracts are
stated at each call, `163a783`).

Merge conflict: `sys/path.rs` (added by both): combined; l-refs'
byte-string `is_valid` is `is_valid_bytes` over the shared `ConstStr`.

Resolved (PENDING entries, deleted):

- l-refs → l-repo (top-level of `sys/pending.rs`): the opaque
  `git_repository` and `git_commit` are the Rust types; the declarations of
  `git_repository_{refdb__weakptr, refdb, __configmap_lookup,
  foreach_worktree, path, commondir, get_namespace, ident, is_bare,
  is_worktree, oid_type, open, init, free}` and `git_repository__fsync_gitdir`,
  and the fixed offset `REPO_REFDB_TYPE_OFFSET` (240), go: `RepoPtr`'s
  methods read `GitRepository` directly; `refs::rename` calls the
  repository's `git_repository_foreach_worktree` export.
- l-refs → l-commit: `git_commit_{parentcount, summary, committer}`
  declarations go (`crate::commit::ffi`).
- l-repo → l-refs (`l_repo.rs`): every `git_refdb_*` / `git_reference_*`
  declaration and `git_reference__enable_symbolic_ref_target_validation`
  go. `handles::{Refdb, Reference, RefIter}` are now typed owners of
  `GitRefdb`, `GitReference`, `refdb::git_reference_iterator` over l-refs'
  exports; `GitRepository._refdb` is `AtomicPtr<GitRefdb>`; the C ABI of
  `git_repository_refdb*`/`_set_refdb` is typed `*mut GitRefdb`. The
  publish/set_owned race tests now use the `grafts` slot.
- l-config → l-refs: `git_refdb_lookup` and the l_repo reference
  declarations used by `repository_head_symbolic_target` go (l-refs'
  exports).
- l-commit → l-refs: `git_reference_{lookup_resolved, target, free,
  __update_for_commit, peel}` declarations and the opaque `git_reference`
  go.
- l-revwalk → l-refs: `git_reference_{name_to_id, iterator_glob_new, next,
  name, free, iterator_free}` declarations and the opaque
  `git_reference`/`git_reference_iterator` go.
- l-refs → l-index: `sys::iterator::INDEX_ENTRY_PATH_OFFSET` (80) is
  `offset_of!(index::GitIndexEntry, path)`.
- `libgit2_init_once` calls `crate::libgit2::init` (no C declaration).

Left: `refs_api::Repository`, l-refs' `Sync` stand-in for l-repo's
`Repository` (integration notes I-6: an API decision is needed); the
safe `Repository::head()` still returns `repository::handles::Reference`,
not l-refs' safe `refs_api::Reference` (they are tied to the two
different repository types).

## 9. Cleanup (`dec005c`)

Duplicated constants between merged modules now come from their owners:
`GIT_REFERENCE_{DIRECT,SYMBOLIC}` (repository → refs), `GIT_OBJECT_{COMMIT,
TREE}` (pack → odb), `GIT_OBJECT_{ANY,COMMIT}` (revwalk → odb),
`GIT_REFDB_FILES` (repository → refdb), `GIT_REFS_DIR` /
`GIT_REFS_HEADS_DIR` (repository → refs).

## PENDING left (at `dec005c`)

68 `PENDING` mentions in the crate (from 36 at the start, before l-config
… l-refs added theirs). Between merged WPs only one is left:
`refs_api::Repository` (l-refs' `Sync` stand-in for l-repo's
`Repository`, integration notes I-6). The rest are PENDING(l-obj) (WP
l-obj not merged: object lookup/free/dup/peel/type names, signatures,
grafts, blob creation, tree lookup/free, `git_commit_lookup`/`_free`,
`git_tag_target_id`, l-commit's own copies of signature/header parsing,
l-odb's copy of the object type names, `repository::handles::Object`,
the `grafts` slots), plus `revwalk::api`'s note that
`git_revwalk_pathspec` has no safe counterpart (C pathspec, stays C).

## 10. Coordinator decisions on I-6 and I-2 (after master = `572eede`)

- **I-6:** `Repository` is `Send + Sync` (justified field by field in
  `repository/api.rs`). `index(&mut self) -> Index<'_>` and
  `reinit_filesystem(&mut self)` (submodule foreach reads the index and the
  submodule cache); the other `&self` methods reach only thread-safe state
  (odb, refdb, config, object cache, configmap cache, git-directory files).
  `refs_api::Repository` (l-refs' stand-in, the last PENDING between merged
  WPs) is gone: `refs_api` and its tests, including the multi-threaded
  `refs_stress`, use `crate::Repository` (`Repository::ptr` for the refs
  modules). `Repository::head()` returns `refs_api::Reference<'_>`.
  - Found while auditing for `Sync`: `refs::update_for_commit` (reached
    from `CommitBuilder::write` with `update_ref`) computed the reflog
    summary with `git_commit_summary`, which writes the lazily computed
    summary into the *cached, shared* commit object: a data race once two
    threads write the same commit. It now computes the summary without
    writing the object (`commit::summary_of`). The safe `Commit` caches
    per handle (`OnceCell`), which is fine.
- **I-2:** `Odb<'r>`, `Config<'r>`, `Index<'r>` (and `ConfigEntries`,
  `IndexEntries<'r>`, `IndexConflicts<'r>`) carry the lifetime of the
  repository they came from (`'static` when made alone); clones,
  `Config::{open_level, open_global, snapshot}` keep it. `RepoHandle` is
  gone (`Repository::{odb, config, config_snapshot, index}` return the
  types directly). `compile_fail` doctests in `repository/api.rs`: an odb
  clone, a config snapshot, a config moved to a `'static` thread, an index
  clone outliving the repository; using the repository while an index
  lives; sending an index. Each was checked to compile without its
  offending line.
- **Send/Sync:** `tests/api_send_sync.rs` asserts, at compile time, `Send`
  and `Sync` or their absence for every public type of the safe API
  (positive with trait bounds, negative with an ambiguity check that fails
  to compile if the type gains the trait).
