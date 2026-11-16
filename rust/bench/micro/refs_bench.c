/* Per-call cost of the refs functions (refs.c, refdb.c, refdb_fs.c,
 * reflog.c) through their internal C ABI, linked against the C reference
 * objects or against the Rust port (same symbols).
 *
 *   RUST_BUILD=/work/builds/l-refs ./run_refs.sh [ROUNDS] [FILTER]
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2.h"
#include "git2/sys/refs.h"
#include "refs.h"
#include "refdb.h"
#include <time.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e3 + t.tv_nsec / 1e6; }
static volatile size_t sink;

#define BENCH(name, n, body) do { double best = 1e18; int r_; size_t i; \
	if (only && !strstr(name, only)) break; \
	for (r_ = 0; r_ < 5; r_++) { double t = now(); for (i = 0; i < (size_t)(n); i++) { body; } t = now() - t; if (t < best) best = t; } \
	printf("bench %-26s %9.2f ns\n", name, best * 1e6 / (n)); } while (0)

static const char *names[] = {
	"refs/heads/master", "refs//heads///topic/x", "refs/tags/v2.45.0", "refs/remotes/origin/HEAD",
	"HEAD", "refs/heads/a..b", "refs/heads/feature/with/a/rather/long/name", "@",
};
#define NN (sizeof(names) / sizeof(names[0]))

int main(int argc, char **argv)
{
	const char *only = argc > 1 ? argv[1] : NULL;
	git_repository *repo;
	git_refdb *db;
	git_oid id;
	git_str buf = GIT_STR_INIT;
	char out[GIT_REFNAME_MAX];

	git_libgit2_init();
	git_oid_from_string(&id, "a65fedf39aefe402d3bb6e24df4d4f5fe4547750", GIT_OID_SHA1);

	BENCH("normalize_name", 2000000, { sink += git_reference__normalize_name(&buf, names[i % NN], GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL); git_error_clear(); });
	BENCH("normalize_name_buffer", 2000000, { sink += git_reference_normalize_name(out, sizeof(out), names[i % NN], GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL); git_error_clear(); });
	BENCH("name_is_valid", 2000000, { int v; git_reference_name_is_valid(&v, names[i % NN]); sink += v; git_error_clear(); });
	BENCH("is_per_worktree_ref", 10000000, { sink += git_reference__is_per_worktree_ref(names[i % NN]); });
	BENCH("shorthand", 10000000, { sink += *git_reference__shorthand(names[i % NN]); });
	BENCH("alloc_free_direct", 5000000, { git_reference *r = git_reference__alloc(names[i % NN], &id, NULL); sink += git_reference_name(r)[0]; git_reference_free(r); });
	BENCH("alloc_free_symbolic", 5000000, { git_reference *r = git_reference__alloc_symbolic("HEAD", names[i % NN]); sink += git_reference_symbolic_target(r)[0]; git_reference_free(r); });

	if (git_repository_open(&repo, "/work/bench-data/refs.git") < 0 ||
	    git_repository_refdb(&db, repo) < 0) {
		fprintf(stderr, "no refs.git\n");
		return 1;
	}
	BENCH("refdb_lookup_loose", 20000, { git_reference *r; if (!git_refdb_lookup(&r, db, "refs/heads/main")) { sink += r->type; git_reference_free(r); } });
	BENCH("refdb_lookup_packed", 20000, { git_reference *r; if (!git_refdb_lookup(&r, db, "refs/tags/v2.45.0")) { sink += r->type; git_reference_free(r); } });
	BENCH("refdb_exists_packed", 20000, { int e; git_refdb_exists(&e, db, "refs/tags/v2.45.0"); sink += e; });
	BENCH("refdb_resolve_head", 20000, { git_reference *r; if (!git_refdb_resolve(&r, db, "HEAD", -1)) { sink += r->type; git_reference_free(r); } });
	BENCH("refdb_lookup_missing", 20000, { git_reference *r; sink += git_refdb_lookup(&r, db, "refs/heads/nope"); git_error_clear(); });
	git_refdb_free(db);
	git_repository_free(repo);
	git_str_dispose(&buf);
	fprintf(stderr, "sink %zu\n", (size_t)sink);
	return 0;
}
