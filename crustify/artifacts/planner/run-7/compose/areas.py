"""The subject areas of libgit2's C test suites: for each test file, the modules
whose own results or effects its assertions check, the judgement the
decomposition's Bundling rule asks for. libgit2 names each clar suite for what
it tests (tests/libgit2/refs/branches/ tests branch.c, tests/util/str/ str.c),
so the area is read from the suite's path and its tests' code.

A test case is tied to an area module that its own code, or a test helper it
calls, reaches: the call is the operation whose result the test asserts. Every
other module a test calls (the repository it opens, the objects it looks up,
the oids, strings and paths it compares, the errors clar's macros read) sets up
or tears down its fixtures, or observes the result, and is no subject of it.

`ALWAYS` names the area modules whose results a file's assertions check
although its tests reach them only through the module they call: config/read.c
reads through git_config_* what config_file.c and config_parse.c produce. Such
a module is a subject of every test case of the file.

A test case that reaches none of its file's area modules, and uses no module's
internals, tests its suite's subject through its fixtures (a lookup, an option,
a callback): it is tied to the area modules its file's other test cases are
tied to by their results, or to every area module of its file when none is. A
win32-only test case, whose body the configured build compiles empty, is tied
the same way: it tests its suite's subject on the builds that compile it. A
file whose every test case the build compiles empty (a deprecated API's, or a
win32-only suite) tests nothing here, and is tied to none.

The most specific entry wins: a file, then its directories, deepest first."""

L = 'src/libgit2/'
U = 'src/util/'

AREA = {
    # tests/libgit2
    'tests/libgit2/apply/': [L + 'apply.c'],
    'tests/libgit2/apply/fromfile.c': [L + 'apply.c', L + 'patch_parse.c'],
    'tests/libgit2/attr/': [L + 'attr.c', L + 'attr_file.c', L + 'attrcache.c'],
    'tests/libgit2/blame/': [L + 'blame.c', L + 'blame_git.c'],
    'tests/libgit2/checkout/': [L + 'checkout.c'],
    'tests/libgit2/cherrypick/': [L + 'cherrypick.c'],
    'tests/libgit2/clone/': [L + 'clone.c'],
    'tests/libgit2/commit/': [L + 'commit.c'],
    'tests/libgit2/commit/parse.c': [L + 'commit.c', L + 'signature.c'],
    'tests/libgit2/commit/signature.c': [L + 'signature.c'],
    'tests/libgit2/config/': [L + 'config.c', L + 'config_file.c', L + 'config_parse.c', L + 'config_list.c',
                              L + 'config_mem.c', L + 'config_snapshot.c', L + 'config_cache.c'],
    'tests/libgit2/core/buf.c': [L + 'buf.c'],
    'tests/libgit2/core/env.c': [L + 'sysdir.c'],
    'tests/libgit2/core/features.c': [L + 'libgit2.c'],
    'tests/libgit2/core/hashsig.c': [L + 'hashsig.c'],
    'tests/libgit2/core/oid.c': [L + 'oid.c'],
    'tests/libgit2/core/oidarray.c': [L + 'oidarray.c'],
    'tests/libgit2/core/opts.c': [L + 'settings.c', L + 'libgit2.c'],
    'tests/libgit2/core/pool.c': [U + 'pool.c'],
    'tests/libgit2/core/structinit.c': [],
    'tests/libgit2/core/useragent.c': [L + 'settings.c'],
    'tests/libgit2/core/version.c': [L + 'libgit2.c'],
    'tests/libgit2/date/': [U + 'date.c'],
    'tests/libgit2/delta/': [L + 'delta.c'],
    'tests/libgit2/describe/': [L + 'describe.c'],
    'tests/libgit2/diff/': [L + 'diff.c', L + 'diff_generate.c', L + 'diff_print.c', L + 'diff_tform.c',
                            L + 'diff_stats.c', L + 'diff_parse.c', L + 'diff_file.c', L + 'diff_driver.c',
                            L + 'diff_xdiff.c', L + 'patch.c', L + 'patch_generate.c', L + 'patch_parse.c'],
    'tests/libgit2/email/': [L + 'email.c'],
    'tests/libgit2/fetch/': [L + 'remote.c', L + 'fetch.c'],
    'tests/libgit2/fetchhead/': [L + 'fetchhead.c'],
    'tests/libgit2/filter/': [L + 'filter.c', L + 'crlf.c', L + 'ident.c', L + 'blob.c'],
    'tests/libgit2/grafts/': [L + 'grafts.c'],
    'tests/libgit2/grafts/shallow.c': [L + 'grafts.c', L + 'repository.c'],
    'tests/libgit2/graph/': [L + 'graph.c'],
    'tests/libgit2/graph/commitgraph.c': [L + 'commit_graph.c'],
    'tests/libgit2/ignore/': [L + 'ignore.c'],
    'tests/libgit2/ignore/status.c': [L + 'ignore.c', L + 'status.c'],
    'tests/libgit2/index/': [L + 'index.c', L + 'index_map.c', L + 'tree-cache.c'],
    'tests/libgit2/iterator/': [L + 'iterator.c'],
    'tests/libgit2/mailmap/': [L + 'mailmap.c'],
    'tests/libgit2/mailmap/blame.c': [L + 'mailmap.c', L + 'blame.c'],
    'tests/libgit2/merge/': [L + 'merge.c', L + 'merge_file.c', L + 'merge_driver.c', L + 'annotated_commit.c'],
    'tests/libgit2/message/': [L + 'trailer.c', L + 'message.c'],
    'tests/libgit2/network/cred.c': [L + 'transports/credential.c', L + 'transports/credential_helpers.c'],
    'tests/libgit2/network/fetchlocal.c': [L + 'remote.c', L + 'fetch.c', L + 'transports/local.c'],
    'tests/libgit2/network/refspecs.c': [L + 'refspec.c'],
    'tests/libgit2/network/remote/': [L + 'remote.c', L + 'refspec.c'],
    'tests/libgit2/network/remote/defaultbranch.c': [L + 'remote.c', L + 'clone.c'],
    'tests/libgit2/network/remote/local.c': [L + 'remote.c', L + 'transports/local.c'],
    'tests/libgit2/notes/': [L + 'notes.c'],
    'tests/libgit2/object/': [L + 'object.c', L + 'object_api.c'],
    'tests/libgit2/object/blob/': [L + 'blob.c'],
    'tests/libgit2/object/cache.c': [L + 'cache.c', L + 'object.c'],
    'tests/libgit2/object/commit/': [L + 'commit.c', L + 'object.c'],
    'tests/libgit2/object/message.c': [L + 'message.c'],
    'tests/libgit2/object/raw/': [L + 'oid.c', L + 'object.c'],
    'tests/libgit2/object/raw/write.c': [L + 'odb.c', L + 'odb_loose.c'],
    'tests/libgit2/object/tag/': [L + 'tag.c', L + 'object.c'],
    'tests/libgit2/object/tree/': [L + 'tree.c', L + 'object.c'],
    'tests/libgit2/odb/': [L + 'odb.c', L + 'odb_loose.c', L + 'odb_pack.c', L + 'odb_mempack.c'],
    'tests/libgit2/online/': [L + 'clone.c', L + 'remote.c', L + 'push.c', L + 'fetch.c', L + 'fetchhead.c'],
    'tests/libgit2/pack/': [L + 'pack.c', L + 'mwindow.c'],
    'tests/libgit2/pack/filelimit.c': [L + 'pack-objects.c', L + 'mwindow.c'],
    'tests/libgit2/pack/indexer.c': [L + 'indexer.c'],
    'tests/libgit2/pack/limits.c': [L + 'indexer.c'],
    'tests/libgit2/pack/midx.c': [L + 'midx.c'],
    'tests/libgit2/pack/packbuilder.c': [L + 'pack-objects.c'],
    'tests/libgit2/patch/': [L + 'patch.c', L + 'patch_parse.c', L + 'diff_print.c'],
    'tests/libgit2/path/': [L + 'path.c'],
    'tests/libgit2/perf/': [L + 'merge.c'],
    'tests/libgit2/rebase/': [L + 'rebase.c'],
    'tests/libgit2/refs/': [L + 'refs.c', L + 'refdb.c', L + 'refdb_fs.c'],
    'tests/libgit2/refs/branches/': [L + 'branch.c'],
    'tests/libgit2/refs/reflog/': [L + 'reflog.c'],
    'tests/libgit2/refs/reflog/messages.c': [L + 'reflog.c', L + 'refs.c'],
    'tests/libgit2/refs/revparse.c': [L + 'revparse.c'],
    'tests/libgit2/refs/tags/': [L + 'tag.c'],
    'tests/libgit2/refs/transactions.c': [L + 'transaction.c'],
    'tests/libgit2/remote/': [L + 'remote.c'],
    'tests/libgit2/repo/': [L + 'repository.c'],
    'tests/libgit2/repo/pathspec.c': [L + 'pathspec.c'],
    'tests/libgit2/reset/': [L + 'reset.c'],
    'tests/libgit2/revert/': [L + 'revert.c'],
    'tests/libgit2/revwalk/': [L + 'revwalk.c'],
    'tests/libgit2/revwalk/mergebase.c': [L + 'merge.c', L + 'graph.c'],
    'tests/libgit2/stash/': [L + 'stash.c'],
    'tests/libgit2/status/': [L + 'status.c'],
    'tests/libgit2/status/single.c': [L + 'object.c'],
    'tests/libgit2/status/submodules.c': [L + 'status.c', L + 'submodule.c'],
    'tests/libgit2/stream/': [L + 'streams/registry.c'],
    'tests/libgit2/stress/': [L + 'diff_tform.c', L + 'diff.c', L + 'diff_generate.c'],
    'tests/libgit2/submodule/': [L + 'submodule.c'],
    'tests/libgit2/threads/atomic.c': [U + 'thread.c'],
    'tests/libgit2/threads/basic.c': [L + 'libgit2.c', L + 'cache.c', U + 'errors.c', U + 'thread.c'],
    'tests/libgit2/threads/diff.c': [L + 'diff.c', L + 'diff_generate.c'],
    'tests/libgit2/threads/iterator.c': [L + 'iterator.c'],
    'tests/libgit2/threads/refdb.c': [L + 'refs.c', L + 'refdb.c', L + 'refdb_fs.c'],
    'tests/libgit2/threads/tlsdata.c': [U + 'thread.c'],
    'tests/libgit2/trace/': [L + 'trace.c'],
    'tests/libgit2/transport/': [L + 'transport.c'],
    'tests/libgit2/transports/smart/': [L + 'transports/smart_pkt.c'],
    'tests/libgit2/win32/': [],
    'tests/libgit2/win32/forbidden.c': [L + 'index.c', L + 'diff_generate.c', L + 'blame.c', L + 'path.c'],
    'tests/libgit2/worktree/': [L + 'worktree.c'],
    'tests/libgit2/worktree/config.c': [L + 'worktree.c', L + 'config.c', L + 'repository.c'],
    'tests/libgit2/worktree/merge.c': [L + 'worktree.c', L + 'merge.c'],
    'tests/libgit2/worktree/open.c': [L + 'worktree.c', L + 'repository.c'],
    'tests/libgit2/worktree/reflog.c': [L + 'worktree.c', L + 'reflog.c'],
    'tests/libgit2/worktree/refs.c': [L + 'worktree.c', L + 'refs.c', L + 'branch.c'],
    'tests/libgit2/worktree/repository.c': [L + 'worktree.c', L + 'repository.c'],
    'tests/libgit2/worktree/submodule.c': [L + 'worktree.c', L + 'submodule.c', L + 'repository.c'],
    # tests/util: one subject per file
    'tests/util/alloc.c': [U + 'alloc.c'],
    'tests/util/array.c': [U + 'array.h'],
    'tests/util/assert.c': [U + 'errors.c'],
    'tests/util/bitvec.c': [U + 'bitvec.h'],
    'tests/util/copy.c': [U + 'futils.c'],
    'tests/util/dirent.c': [U + 'fs_path.c'],
    'tests/util/encoding.c': [U + 'varint.c'],
    'tests/util/errors.c': [U + 'errors.c'],
    'tests/util/filebuf.c': [U + 'filebuf.c'],
    'tests/util/ftruncate.c': [U + 'posix.c'],
    'tests/util/futils.c': [U + 'futils.c'],
    'tests/util/gitstr.c': [U + 'str.c'],
    'tests/util/hashmap.c': [U + 'hashmap_str.h'],
    'tests/util/hex.c': [U + 'util.c'],
    'tests/util/hostname.c': [U + 'net.c'],
    'tests/util/iconv.c': [U + 'fs_path.c'],
    'tests/util/init.c': [L + 'libgit2.c', U + 'runtime.c'],
    'tests/util/integer.c': [U + 'integer.h'],
    'tests/util/link.c': [U + 'posix.c'],
    'tests/util/memmem.c': [U + 'util.c'],
    'tests/util/mkdir.c': [U + 'futils.c'],
    'tests/util/path/': [U + 'fs_path.c'],
    'tests/util/pool.c': [U + 'pool.c'],
    'tests/util/posix.c': [U + 'posix.c'],
    'tests/util/pqueue.c': [U + 'pqueue.c'],
    'tests/util/process/': [U + 'unix/process.c'],
    'tests/util/qsort.c': [U + 'util.c'],
    'tests/util/regexp.c': [U + 'regexp.c'],
    'tests/util/rmdir.c': [U + 'futils.c'],
    'tests/util/sha1.c': [U + 'hash.c', U + 'hash/collisiondetect.c', U + 'hash/sha1dc/sha1.c'],
    'tests/util/sha256.c': [U + 'hash.c', U + 'hash/openssl.c'],
    'tests/util/sortedcache.c': [U + 'sortedcache.c'],
    'tests/util/stat.c': [U + 'posix.c'],
    'tests/util/str/': [U + 'str.c'],
    'tests/util/string.c': [U + 'util.c'],
    'tests/util/strtol.c': [U + 'util.c'],
    'tests/util/url/': [U + 'net.c'],
    'tests/util/utf8.c': [U + 'utf8.c'],
    'tests/util/vector.c': [U + 'vector.c'],
    'tests/util/wildmatch.c': [U + 'wildmatch.c'],
    'tests/util/zstream.c': [U + 'zstream.c'],
}

# Area modules a file's tests check through another module's API, so that every
# test case of the file is tied to them, reached or not.
ALWAYS = {
    'tests/libgit2/fetchhead/nonetwork.c': [L + 'fetchhead.c'],
    'tests/libgit2/grafts/': [L + 'grafts.c'],
    'tests/libgit2/blame/simple.c': [L + 'blame_git.c'],
    'tests/libgit2/config/read.c': [L + 'config_file.c', L + 'config_parse.c'],
    'tests/libgit2/config/write.c': [L + 'config_file.c', L + 'config_parse.c'],
    'tests/libgit2/config/include.c': [L + 'config_file.c'],
    'tests/libgit2/config/conditionals.c': [L + 'config_file.c'],
    'tests/libgit2/config/multivar.c': [L + 'config_file.c'],
    'tests/libgit2/config/stress.c': [L + 'config_file.c', L + 'config_parse.c'],
    'tests/libgit2/config/snapshot.c': [L + 'config_snapshot.c'],
    'tests/libgit2/diff/drivers.c': [L + 'diff_driver.c'],
    'tests/libgit2/diff/userdiff.c': [L + 'diff_driver.c'],
    'tests/libgit2/filter/crlf.c': [L + 'crlf.c'],
    'tests/libgit2/filter/ident.c': [L + 'ident.c'],
    'tests/libgit2/object/commit/parse.c': [L + 'commit.c'],
    'tests/libgit2/object/tag/parse.c': [L + 'tag.c'],
    'tests/libgit2/object/tree/parse.c': [L + 'tree.c'],
    'tests/libgit2/odb/packed.c': [L + 'odb_pack.c'],
    'tests/libgit2/odb/packed256.c': [L + 'odb_pack.c'],
    'tests/libgit2/odb/packedone.c': [L + 'odb_pack.c'],
    'tests/libgit2/odb/packedone256.c': [L + 'odb_pack.c'],
    'tests/libgit2/online/push.c': [L + 'push.c'],
    'tests/libgit2/pack/sharing.c': [L + 'mwindow.c'],
    'tests/libgit2/pack/threadsafety.c': [L + 'pack.c', L + 'mwindow.c'],
    'tests/libgit2/refs/pack.c': [L + 'refdb_fs.c'],
    'tests/libgit2/threads/refdb.c': [L + 'refdb_fs.c'],
    'tests/libgit2/perf/merge.c': [L + 'merge.c'],
}
# tests/util files test one module each, through its own API or its macros
# (integer.h's overflow checks are compiler builtins, so no call shows them)
for _p, _ms in AREA.items():
    if _p.startswith('tests/util/') and _p.endswith('.c'):
        ALWAYS.setdefault(_p, list(_ms))


def area(path):
    """The area modules of a test file and those every test case of it checks."""
    best = None
    for p in AREA:
        if path == p or (p.endswith('/') and path.startswith(p)):
            if best is None or len(p) > len(best):
                best = p
    always = []
    for p, ms in ALWAYS.items():
        if path == p or (p.endswith('/') and path.startswith(p)):
            always += ms
    return (AREA[best] if best is not None else None), always
