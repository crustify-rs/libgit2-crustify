/* revwalk: history walks of git.git (~83k commits from master, ~86k from
 * all refs).  A fresh repository is opened in setup() (untimed) so neither
 * the object cache nor the walker's commit map is warm. */
#include "bench.h"

static git_repository *repo;

static void open_repo(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("git.git"))); }

static uint64_t drain(git_revwalk *w);

/* a repository whose odb cache already holds every commit: the walk then
 * measures the walker (commit list parsing, sorting, marking) */
static git_repository *warm_repo;
static void prep_warm(void *ctx)
{
	git_revwalk *w;
	(void)ctx;
	if (warm_repo) return;
	CHECK(git_repository_open(&warm_repo, bench_data("git.git")));
	CHECK(git_revwalk_new(&w, warm_repo));
	CHECK(git_revwalk_push_glob(w, "refs/*"));
	drain(w);
	git_revwalk_free(w);
}
static void open_warm(void *ctx) { (void)ctx; repo = warm_repo; }
static void close_warm(void *ctx) { (void)ctx; repo = NULL; }
static void close_repo(void *ctx) { (void)ctx; git_repository_free(repo); repo = NULL; }

static uint64_t drain(git_revwalk *w)
{
	uint64_t n = 0, h = FNV_INIT;
	git_oid id;
	int e;
	while ((e = git_revwalk_next(&id, w)) == 0) {
		n++;
		h = fnv(h, id.id, 4);
	}
	if (e != GIT_ITEROVER) CHECK(e);
	return (n << 40) ^ (h & 0xffffffffffULL);
}

typedef struct { unsigned sort; const char *push; const char *hide; int first_parent; } walk_t;

static uint64_t run_walk(void *ctx)
{
	walk_t *c = ctx;
	git_revwalk *w;
	uint64_t r;
	CHECK(git_revwalk_new(&w, repo));
	CHECK(git_revwalk_sorting(w, c->sort));
	if (c->first_parent) CHECK(git_revwalk_simplify_first_parent(w));
	if (strchr(c->push, '*')) CHECK(git_revwalk_push_glob(w, c->push));
	else if (!strncmp(c->push, "refs/", 5)) CHECK(git_revwalk_push_ref(w, c->push));
	else CHECK(git_revwalk_push_range(w, c->push));
	if (c->hide) CHECK(git_revwalk_hide_ref(w, c->hide));
	r = drain(w);
	git_revwalk_free(w);
	return r;
}

static walk_t w_none = { GIT_SORT_NONE, "refs/heads/master", NULL, 0 };
static walk_t w_time = { GIT_SORT_TIME, "refs/heads/master", NULL, 0 };
static walk_t w_topo = { GIT_SORT_TOPOLOGICAL, "refs/heads/master", NULL, 0 };
static walk_t w_topo_time = { GIT_SORT_TOPOLOGICAL | GIT_SORT_TIME, "refs/heads/master", NULL, 0 };
static walk_t w_reverse = { GIT_SORT_TIME | GIT_SORT_REVERSE, "refs/heads/master", NULL, 0 };
static walk_t w_first_parent = { GIT_SORT_NONE, "refs/heads/master", NULL, 1 };
static walk_t w_all = { GIT_SORT_NONE, "refs/*", NULL, 0 };
static walk_t w_range = { GIT_SORT_NONE, "v2.30.0..v2.47.0", NULL, 0 };
static walk_t w_range_topo = { GIT_SORT_TOPOLOGICAL, "v2.30.0..v2.47.0", NULL, 0 };
static walk_t w_hide_ref = { GIT_SORT_TIME, "refs/heads/master", "refs/tags/v2.45.0", 0 };

/* short ranges between consecutive releases v2.N.0..v2.N+1.0, one
 * walker each (git log A..B repeated) */
static uint64_t run_release_ranges(void *ctx)
{
	uint64_t h = 0;
	int v;
	(void)ctx;
	for (v = 30; v < 47; v++) {
		git_revwalk *w;
		char range[64];
		snprintf(range, sizeof(range), "v2.%d.0..v2.%d.0", v, v + 1);
		CHECK(git_revwalk_new(&w, repo));
		CHECK(git_revwalk_push_range(w, range));
		h = h * 31 + drain(w);
		git_revwalk_free(w);
	}
	return h;
}

/* reuse one walker: git_revwalk_reset between walks (warm commit map) */
static uint64_t run_reset_reuse(void *ctx)
{
	uint64_t h = 0;
	git_revwalk *w;
	int v;
	(void)ctx;
	CHECK(git_revwalk_new(&w, repo));
	for (v = 30; v < 47; v++) {
		char ref[64];
		snprintf(ref, sizeof(ref), "refs/tags/v2.%d.0", v);
		CHECK(git_revwalk_push_ref(w, ref));
		snprintf(ref, sizeof(ref), "refs/tags/v2.%d.0", v - 5);
		CHECK(git_revwalk_hide_ref(w, ref));
		h = h * 31 + drain(w);
		CHECK(git_revwalk_reset(w));
	}
	git_revwalk_free(w);
	return h;
}

static bench_t benches[] = {
	/* cold: commits are read from the pack during the walk */
	{ "revwalk", "cold_master_unsorted", NULL, open_repo, run_walk, close_repo, &w_none, 1 },
	{ "revwalk", "cold_master_time", NULL, open_repo, run_walk, close_repo, &w_time, 1 },
	{ "revwalk", "cold_master_topo", NULL, open_repo, run_walk, close_repo, &w_topo, 1 },
	{ "revwalk", "cold_all_refs", NULL, open_repo, run_walk, close_repo, &w_all, 1 },
	/* warm: every commit is in the odb cache */
	{ "revwalk", "master_unsorted", prep_warm, open_warm, run_walk, close_warm, &w_none, 1 },
	{ "revwalk", "master_time", prep_warm, open_warm, run_walk, close_warm, &w_time, 1 },
	{ "revwalk", "master_topo", prep_warm, open_warm, run_walk, close_warm, &w_topo, 1 },
	{ "revwalk", "master_topo_time", prep_warm, open_warm, run_walk, close_warm, &w_topo_time, 1 },
	{ "revwalk", "master_time_reverse", prep_warm, open_warm, run_walk, close_warm, &w_reverse, 1 },
	{ "revwalk", "master_first_parent", prep_warm, open_warm, run_walk, close_warm, &w_first_parent, 1 },
	{ "revwalk", "all_refs", prep_warm, open_warm, run_walk, close_warm, &w_all, 1 },
	{ "revwalk", "range_2.30_2.47", prep_warm, open_warm, run_walk, close_warm, &w_range, 1 },
	{ "revwalk", "range_2.30_2.47_topo", prep_warm, open_warm, run_walk, close_warm, &w_range_topo, 1 },
	{ "revwalk", "hide_ref_time", prep_warm, open_warm, run_walk, close_warm, &w_hide_ref, 1 },
	{ "revwalk", "release_ranges_x17", prep_warm, open_warm, run_release_ranges, close_warm, NULL, 17 },
	{ "revwalk", "reset_reuse_x17", prep_warm, open_warm, run_reset_reuse, close_warm, NULL, 17 },
};

BENCH_MAIN(benches)
