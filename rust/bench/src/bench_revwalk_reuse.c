/* revwalk, the walker alone: one walker over git.git whose commit map is
 * already full and every node parsed (by a first walk in prep), reused
 * for every run. A run is 5 walks (each ends with the walker's own reset),
 * so the time is the walker's: queueing, sorting, marking, the hide
 * callback, and the reset of the map's ~86k nodes; no object is read. */
#include "bench.h"

static git_repository *repo;
static git_revwalk *walker;
static git_oid master, v2_30, v2_45;

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

static void prep(void *ctx)
{
	(void)ctx;
	if (walker) return;
	CHECK(git_repository_open(&repo, bench_data("git.git")));
	CHECK(git_reference_name_to_id(&master, repo, "refs/heads/master"));
	{
		git_object *o;
		CHECK(git_revparse_single(&o, repo, "v2.30.0^{commit}"));
		git_oid_cpy(&v2_30, git_object_id(o));
		git_object_free(o);
		CHECK(git_revparse_single(&o, repo, "v2.45.0^{commit}"));
		git_oid_cpy(&v2_45, git_object_id(o));
		git_object_free(o);
	}
	CHECK(git_revwalk_new(&walker, repo));
	CHECK(git_revwalk_push_glob(walker, "refs/*"));
	drain(walker);
}

/* hide the commits whose id starts with 0x00 or 0x01 */
static int hide_low(const git_oid *id, void *payload)
{
	(void)payload;
	return id->id[0] < 2;
}

typedef struct { unsigned sort; int first_parent; int hide; int cb; } mode_t_;

static uint64_t run(void *ctx)
{
	mode_t_ *m = ctx;
	uint64_t h = 0;
	int i;
	for (i = 0; i < 5; i++) {
		CHECK(git_revwalk_sorting(walker, m->sort));
		if (m->first_parent) CHECK(git_revwalk_simplify_first_parent(walker));
		CHECK(git_revwalk_add_hide_cb(walker, m->cb ? hide_low : NULL, NULL));
		CHECK(git_revwalk_push(walker, &master));
		if (m->hide == 1) CHECK(git_revwalk_hide(walker, &v2_30));
		if (m->hide == 2) CHECK(git_revwalk_hide(walker, &v2_45));
		h = h * 31 + drain(walker);
	}
	return h;
}

static mode_t_ m_none = { GIT_SORT_NONE, 0, 0, 0 };
static mode_t_ m_time = { GIT_SORT_TIME, 0, 0, 0 };
static mode_t_ m_topo = { GIT_SORT_TOPOLOGICAL, 0, 0, 0 };
static mode_t_ m_topo_time = { GIT_SORT_TOPOLOGICAL | GIT_SORT_TIME, 0, 0, 0 };
static mode_t_ m_time_reverse = { GIT_SORT_TIME | GIT_SORT_REVERSE, 0, 0, 0 };
static mode_t_ m_topo_reverse = { GIT_SORT_TOPOLOGICAL | GIT_SORT_REVERSE, 0, 0, 0 };
static mode_t_ m_first_parent = { GIT_SORT_NONE, 1, 0, 0 };
static mode_t_ m_hide = { GIT_SORT_TIME, 0, 1, 0 };
static mode_t_ m_hide_near = { GIT_SORT_NONE, 0, 2, 0 };
static mode_t_ m_hide_cb = { GIT_SORT_NONE, 0, 0, 1 };

static bench_t benches[] = {
	{ "revwalk", "reuse_unsorted_x5", prep, NULL, run, NULL, &m_none, 5 },
	{ "revwalk", "reuse_time_x5", prep, NULL, run, NULL, &m_time, 5 },
	{ "revwalk", "reuse_topo_x5", prep, NULL, run, NULL, &m_topo, 5 },
	{ "revwalk", "reuse_topo_time_x5", prep, NULL, run, NULL, &m_topo_time, 5 },
	{ "revwalk", "reuse_time_reverse_x5", prep, NULL, run, NULL, &m_time_reverse, 5 },
	{ "revwalk", "reuse_topo_reverse_x5", prep, NULL, run, NULL, &m_topo_reverse, 5 },
	{ "revwalk", "reuse_first_parent_x5", prep, NULL, run, NULL, &m_first_parent, 5 },
	{ "revwalk", "reuse_hide_2.30_time_x5", prep, NULL, run, NULL, &m_hide, 5 },
	{ "revwalk", "reuse_hide_2.45_x5", prep, NULL, run, NULL, &m_hide_near, 5 },
	{ "revwalk", "reuse_hide_cb_x5", prep, NULL, run, NULL, &m_hide_cb, 5 },
};

BENCH_MAIN(benches)
