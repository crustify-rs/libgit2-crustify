"""Planner run 8's decomposition edits to modules.json.

Wavefront 88793d7 schedules every entity under its C kind: the macros of the
closure's files, the function pointers written inline and the typedefs of their
own are units now, beside the structs, unions, enums, functions, globals and the
function pointers a typedef declares. The decomposition's Bundling rule places
each of them; run 7's modules.json placed none, since none was a unit of the
schedules it was checked against. This script places them, from the committed
modules.json (1aa1ee98e), and renames the `callbacks` counters
`function_pointers` (counters8.py recounts them).

- The shared headers' API macros, function pointers written inline and typedefs
  of their own that no module owned go to the module the rule names, each split
  recorded in the header's file entry (`EDITS`): a macro to the module whose
  logic it implements, as the functions it calls and the types it accesses show,
  a constant (or a family of flags) to the module that owns the type holding its
  values or, holding none, whose functions consume it; a function pointer
  written inline with the struct or function whose declaration holds it; a
  typedef of its own to the module whose functions use it most.
- The headers whose entities are all macros or typedefs of their own, which no
  module listed and so the closure left out, are bundled: with a TU when one
  carries their name or implements what they declare, else split as above, and
  a header implementing what no TU does is a header module of its own
  (`NEW_MODULES`).
- A file entry whose module now owns every entity of the header becomes a whole
  entry.

Not API, so neither owned nor translated (decomposition, Bundling): the
attribute, linkage and configuration macros in `NOT_API` (compose/not_api.json).
Out of scope, as before: the deprecated git_url_resolve_cb.

Input: git 1aa1ee98e:crustify/artifacts/modules.json.
Output: crustify/artifacts/modules.json."""
import json, os, subprocess, collections
R = '/target/libgit2-bindings/'
HERE = os.path.dirname(os.path.abspath(__file__))
M = json.loads(subprocess.run(['git', '-C', R, 'show', '1aa1ee98e:crustify/artifacts/modules.json'],
                              capture_output=True, check=True).stdout)
L, U, X, N, F = 'src/libgit2/', 'src/util/', 'deps/xdiff/', 'deps/ntlmclient/', 'deps/reftable/'
WHOLE = None
# module name -> [(section, path, included names or WHOLE, excluded names)]
EDITS = {
    U + 'errors.c': [
        ('internal_headers', U + 'assert_safe.h', WHOLE, []),
        ('internal_headers', U + 'git2_util.h', ['GIT_ADD_SIZET_OVERFLOW', 'GIT_ERROR_CHECK_ALLOC', 'GIT_ERROR_CHECK_ALLOC_ADD',
                                               'GIT_ERROR_CHECK_ALLOC_ADD3', 'GIT_ERROR_CHECK_ALLOC_MULTIPLY',
                                               'GIT_ERROR_CHECK_ALLOC_STR', 'GIT_ERROR_CHECK_ERROR',
                                               'GIT_MULTIPLY_SIZET_OVERFLOW'], []),
        ('internal_headers', X + 'git-xdiff.h', ['XDL_BUG'], []),
    ],
    U + 'futils.c': [('internal_headers', U + 'git2_util.h', ['GIT_BUFSIZE_DEFAULT', 'GIT_BUFSIZE_FILEIO'], [])],
    L + 'filter.c': [
        ('internal_headers', U + 'git2_util.h', ['GIT_BUFSIZE_FILTERIO'], []),
        ('api_headers', 'include/git2/types.h', ['int (*)(git_writestream *)', 'int (*)(git_writestream *, const char *, size_t)',
                                                'void (*)(git_writestream *)'], []),
    ],
    U + 'hash.c': [('internal_headers', U + 'hash/sha.h', WHOLE, [])],
    U + 'thread.c': [('internal_headers', U + 'unix/pthread.h', WHOLE, [])],
    L + 'transports/auth_ntlmclient.c': [('internal_headers', L + 'transports/auth_ntlm.h', WHOLE, [])],
    L + 'transports/auth.c': [('internal_headers', L + 'transports/auth_negotiate.h', ['git_http_auth_negotiate'], [])],
    L + 'libgit2.c': [('api_headers', 'include/git2/version.h', WHOLE, [])],
    F + 'system.c': [('internal_headers', F + 'reftable-system.h', WHOLE, [])],
    F + 'block.c': [
        ('internal_headers', F + 'constants.h', ['MAX_RESTARTS'], []),
        ('internal_headers', F + 'reftable-constants.h', WHOLE, []),
    ],
    F + 'writer.c': [('internal_headers', F + 'constants.h', ['DEFAULT_BLOCK_SIZE'], [])],
    F + 'stack.c': [('internal_headers', F + 'constants.h', ['DEFAULT_GEOMETRIC_FACTOR'], [])],
    X + 'xdiffi.c': [('internal_headers', X + 'xdiff.h', [
        'XDF_DIFF_ALG', 'XDF_DIFF_ALGORITHM_MASK', 'XDF_HISTOGRAM_DIFF', 'XDF_IGNORE_BLANK_LINES', 'XDF_IGNORE_CR_AT_EOL',
        'XDF_IGNORE_WHITESPACE', 'XDF_IGNORE_WHITESPACE_AT_EOL', 'XDF_IGNORE_WHITESPACE_CHANGE', 'XDF_INDENT_HEURISTIC',
        'XDF_NEED_MINIMAL', 'XDF_PATIENCE_DIFF', 'XDF_WHITESPACE_FLAGS'], [])],
    X + 'xemit.c': [('internal_headers', X + 'xdiff.h', [
        'XDL_EMIT_FUNCCONTEXT', 'XDL_EMIT_FUNCNAMES', 'XDL_EMIT_NO_HUNK_HDR',
        'int (*)(void *, long, long, long, long, const char *, long)', 'int (*)(void *, mmbuffer_t *, int)'], [])],
    X + 'xmerge.c': [('internal_headers', X + 'xdiff.h', [
        'DEFAULT_CONFLICT_MARKER_SIZE', 'XDL_MERGE_DIFF3', 'XDL_MERGE_EAGER', 'XDL_MERGE_FAVOR_OURS', 'XDL_MERGE_FAVOR_THEIRS',
        'XDL_MERGE_FAVOR_UNION', 'XDL_MERGE_MINIMAL', 'XDL_MERGE_ZEALOUS', 'XDL_MERGE_ZEALOUS_ALNUM',
        'XDL_MERGE_ZEALOUS_DIFF3'], [])],
    U + 'regexp.c': [('internal_headers', X + 'git-xdiff.h', ['xdl_regex_t', 'xdl_regmatch_t'], [])],
    U + 'alloc.c': [('internal_headers', X + 'git-xdiff.h', ['xdl_calloc', 'xdl_free', 'xdl_malloc', 'xdl_realloc'], [])],
    L + 'buf.c': [('api_headers', 'include/git2/buffer.h', ['GIT_BUF_INIT'], [])],
    U + 'fs_path.c': [('api_headers', 'include/git2/common.h', ['GIT_PATH_LIST_SEPARATOR', 'GIT_PATH_MAX'], [])],
    L + 'transports/git.c': [('api_headers', 'include/git2/net.h', ['GIT_DEFAULT_PORT'], [])],
    L + 'odb_loose.c': [('api_headers', 'include/git2/odb_backend.h',
                         ['GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT', 'GIT_ODB_BACKEND_LOOSE_OPTIONS_VERSION'], [])],
    L + 'odb_pack.c': [('api_headers', 'include/git2/odb_backend.h',
                        ['GIT_ODB_BACKEND_PACK_OPTIONS_INIT', 'GIT_ODB_BACKEND_PACK_OPTIONS_VERSION'], [])],
    L + 'odb.c': [
        ('api_headers', 'include/git2/odb_backend.h', 'FP', []),
        ('api_headers', 'include/git2/sys/odb_backend.h', ['GIT_ODB_BACKEND_INIT', 'GIT_ODB_BACKEND_VERSION', 'FP'], []),
    ],
    L + 'refdb.c': [('api_headers', 'include/git2/sys/refdb_backend.h',
                     ['GIT_REFDB_BACKEND_INIT', 'GIT_REFDB_BACKEND_VERSION', 'FP'], [])],
    L + 'stream.h': [('api_headers', 'include/git2/sys/stream.h', ['GIT_STREAM_VERSION'], [])],
    L + 'streams/registry.c': [('api_headers', 'include/git2/sys/stream.h', ['FP'], [])],
    U + 'date.c': [('api_headers', 'include/git2/types.h', ['git_time_t'], [])],
    L + 'blob.c': [('api_headers', 'include/git2/types.h', ['git_object_size_t'], [])],
    L + 'diff.c': [('api_headers', 'include/git2/types.h', ['git_off_t'], [])],
    L + 'config.c': [('internal_headers', L + 'config_backend.h', ['int (*)(const git_config_entry *, void *)'], [])],
    L + 'oid.c': [('internal_headers', L + 'hashmap_oid.h', ['GIT_HASHMAP_OID_FUNCTIONS', 'GIT_HASHMAP_OID_PROTOTYPES',
                                                            'GIT_HASHMAP_OID_SETUP', 'GIT_HASHMAP_OID_STRUCT'], [])],
    L + 'repository.c': [('internal_headers', L + 'repo_template.h', WHOLE, [])],
    L + 'diff_driver.c': [('internal_headers', L + 'userdiff.h', WHOLE, [])],
    N + 'crypt_openssl.c': [('internal_headers', N + 'crypt.h', [], ['ntlm_des_block'])],
    N + 'ntlm.c': [('internal_headers', N + 'crypt.h', ['ntlm_des_block'], [])],
    N + 'unicode_builtin.c': [('internal_headers', N + 'unicode.h', WHOLE, [])],
}
NEW_MODULES = [  # header modules: headers implementing what no TU does
    {'name': U + 'hashmap.h', 'section': 'internal_headers', 'std': {
        'library': ['std::collections::HashMap', 'std::collections::HashSet'], 'feature_parity': 'partial',
        'notes': "the generic open-addressing map and set the macros mint per key and value type (khash's), "
                 "with the hash and equality functions each instance names: HashMap<K, V, S> and HashSet<K, S> "
                 "with a BuildHasher cover them, but not the C instances' layout, their iterator as a bare bucket "
                 "index (git_hashmap_iter_t) that survives insertions, or a per-instance equality function other "
                 "than Eq"},
        'crates': [{'crate': 'hashbrown', 'feature_parity': 'partial', 'license': 'MIT OR Apache-2.0',
                    'notes': "the SwissTable map the standard library's HashMap is built on; its raw table API "
                             "gives bucket indices and custom hash and equality closures (HashTable), closer to the "
                             "instances' find, put and remove by key, but its buckets move on growth as khash's do "
                             "and it has no khash layout either"}]},
    {'name': U + 'cc-compat.h', 'section': 'internal_headers', 'std': {
        'library': ['core::fmt', 'core::ffi::VaList'], 'feature_parity': 'partial',
        'notes': "the size_t printf formats (PRIuZ, PRIxZ, PRIXZ, PRIdZ) are core::fmt's {} and {:x}; GIT_UNUSED "
                 "is `let _ = x;`; va_copy is VaList::clone, which is unstable (c_variadic)"},
        'crates': []},
    {'name': U + 'ctype_compat.h', 'section': 'internal_headers', 'std': {
        'library': ['u8::is_ascii_alphabetic', 'u8::is_ascii_digit', 'u8::is_ascii_alphanumeric', 'u8::is_ascii_hexdigit',
                    'u8::is_ascii_whitespace', 'u8::is_ascii_graphic', 'u8::to_ascii_uppercase', 'u8::to_ascii_lowercase'],
        'feature_parity': 'partial',
        'notes': "the u8 ASCII predicates and case maps are the C locale's; the macros call <ctype.h>, which follows "
                 "the process locale (libgit2 runs in the C locale unless its caller sets another), and isspace counts "
                 "the vertical tab, which is_ascii_whitespace does not; isprint is is_ascii_graphic or b' '"},
        'crates': []},
    {'name': U + 'strnlen.h', 'section': 'internal_headers', 'std': {
        'library': ['core::ffi::CStr::from_bytes_until_nul', 'core::slice::<[u8]>::iter().position'],
        'feature_parity': 'full', 'notes': ''},
        'crates': [{'crate': 'memchr', 'feature_parity': 'full', 'license': 'Unlicense OR MIT',
                    'notes': "memchr(0, &buf[..n]) is p_strnlen's bounded scan for the terminating NUL, vectorized"}]},
    {'name': X + 'xmacros.h', 'section': 'internal_headers', 'std': {
        'library': ['core::cmp::min', 'core::cmp::max', 'i64::abs', 'u8::is_ascii_whitespace', 'u8::is_ascii_digit',
                    'alloc::vec::Vec::try_reserve', 'u32::from_le_bytes', 'u32::to_le_bytes'],
        'feature_parity': 'full', 'notes': ''},
        'crates': []},
    {'name': N + 'compat.h', 'section': 'internal_headers', 'std': {
        'library': ['core::cmp::min'], 'feature_parity': 'full', 'notes': ''},
        'crates': []},
]
NOT_API = json.load(open(os.path.join(HERE, 'not_api.json')))

def entry(path, inc, exc):
    return {'path': path, 'items': {'included': sorted(inc or []), 'excluded': sorted(exc)}}
FPS = json.load(open(os.path.join(HERE, 'fp_owners.json')))   # fp8.py: inline function pointers by holder module
mods = {}
for lu in M['link_units']:
    for x in lu['modules']:
        mods.setdefault(x['name'], x)
lib = M['link_units'][0]
for name, edits in EDITS.items():
    x = mods[name]
    for sec, path, inc, exc in edits:
        if inc == 'FP' or (isinstance(inc, list) and 'FP' in inc):
            inc = [i for i in (inc if isinstance(inc, list) else []) if i != 'FP'] + FPS[name][path]
        lst = x[sec]
        old = [f for f in lst if f['path'] == path]
        if inc is WHOLE:
            new = entry(path, [], [])
        elif exc:                       # the module owns all of the header but these
            new = entry(path, [], exc)
        elif old and old[0]['items']['included']:
            new = entry(path, sorted(set(old[0]['items']['included']) | set(inc)), [])
        elif old:
            raise SystemExit(f'{name}: {path} is listed whole or with excluded')
        else:
            new = entry(path, inc, exc)
        if old:
            lst[lst.index(old[0])] = new
        else:
            lst.append(new)
for nm in NEW_MODULES:
    idx = max(m['index'] for m in lib['modules']) + 1
    lib['modules'].append({
        'index': idx, 'name': nm['name'], 'status': 'todo', 'foundational': False, 'depends_on': [],
        'translation_unit': None, 'internal_headers': [entry(nm['name'], [], [])], 'api_headers': [], 'tests': [],
        'objective': {'objective': 'port', 'rust_equivalent': {'std': nm['std'], 'crates': nm['crates']}},
        'counters': {'implementation': {'files': 1, 'loc': 0, 'structs': 0, 'functions': 0, 'global_variables': 0,
                                        'unions': 0, 'function_pointers': 0},
                     'api': {'files': 0, 'loc': 0, 'structs': 0, 'opaque_structs': 0, 'functions': 0,
                             'global_variables': 0, 'unions': 0, 'function_pointers': 0}}})
def rename(c):
    return {('function_pointers' if k == 'callbacks' else k): v for k, v in c.items()}
for lu in M['link_units']:
    for x in lu['modules']:
        x['counters'] = {b: rename(c) for b, c in x['counters'].items()}
        for d in x['depends_on']:
            d['counters'] = rename(d['counters'])
open(R + 'crustify/artifacts/modules.json', 'w').write(json.dumps(M, indent=2, ensure_ascii=False) + '\n')
print('modules', sum(len(lu['modules']) for lu in M['link_units']))
