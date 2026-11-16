/* index: bigindex (100,000 entries, with a TREE extension) and git-wd's real
 * index (~4,900 entries of git v2.47.0). */
#include "bench.h"

static git_index *idx;
static const char *tmpdir;
static char tmpidx[4096];

#define BIG "bigindex/.git/index"
#define WD "git-wd/.git/index"

static uint64_t index_check(git_index *ix)
{
	size_t n = git_index_entrycount(ix), i;
	uint64_t h = fnv_u64(FNV_INIT, n);
	for (i = 0; i < n; i += 997) {
		const git_index_entry *e = git_index_get_byindex(ix, i);
		h = fnv_str(h, e->path);
		h = fnv_oid(h, &e->id);
		h = fnv_u64(h, e->mode);
	}
	return h;
}

static uint64_t open_index(const char *rel)
{
	git_index *ix;
	uint64_t h;
	CHECK(git_index_open(&ix, bench_data(rel)));
	h = index_check(ix);
	git_index_free(ix);
	return h;
}

static uint64_t run_read_big(void *ctx) { (void)ctx; return open_index(BIG); }
static uint64_t run_read_wd(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 10; i++)
		h += open_index(WD);
	return h;
}

/* a private copy of an index file, opened */
static void setup_copy(const char *rel)
{
	tmpdir = bench_tmpdir("index");
	snprintf(tmpidx, sizeof(tmpidx), "%s/index", tmpdir);
	copy_file(bench_data(rel), tmpidx);
	CHECK(git_index_open(&idx, tmpidx));
}
static void setup_big(void *ctx) { (void)ctx; setup_copy(BIG); }
static void setup_wd(void *ctx) { (void)ctx; setup_copy(WD); }
static void teardown_copy(void *ctx) { (void)ctx; git_index_free(idx); idx = NULL; rm_rf(tmpdir); }

static uint64_t file_hash(void)
{
	size_t len;
	char *d = read_file(tmpidx, &len);
	uint64_t h = fnv_sample(FNV_INIT, d, len);
	/* the trailing checksum covers the whole content */
	h = fnv(h, d + len - 20, 20);
	free(d);
	return h;
}

static uint64_t run_write(void *ctx)
{
	(void)ctx;
	CHECK(git_index_write(idx));
	return file_hash();
}

/* the paths of bigindex, shuffled */
static char **paths;
static size_t npaths;

static void prep_paths(void *ctx)
{
	(void)ctx;
	if (!paths) {
		git_index *ix;
		size_t i;
		CHECK(git_index_open(&ix, bench_data(BIG)));
		npaths = git_index_entrycount(ix);
		paths = xmalloc(npaths * sizeof(*paths));
		for (i = 0; i < npaths; i++)
			paths[i] = xstrdup(git_index_get_byindex(ix, i)->path);
		git_index_free(ix);
		rng_seed(7);
		shuffle(paths, npaths, sizeof(*paths));
	}
	bench_ops = npaths * 2;
}

static uint64_t run_find(void *ctx)
{
	uint64_t h = 0;
	size_t i, pos;
	(void)ctx;
	for (i = 0; i < npaths; i++) {
		const git_index_entry *e;
		CHECK(git_index_find(&pos, idx, paths[i]));
		h += pos;
		e = git_index_get_bypath(idx, paths[i], 0);
		h += e->id.id[0];
	}
	return h;
}

static uint64_t run_find_prefix(void *ctx)
{
	uint64_t h = 0;
	size_t i, pos;
	(void)ctx;
	for (i = 0; i < 10000; i++) {
		char pfx[32];
		snprintf(pfx, sizeof(pfx), "src%03zu/mod%02zu/", i % 100, i % 10);
		CHECK(git_index_find_prefix(&pos, idx, pfx));
		h += pos;
	}
	return h;
}

/* add 10,000 new entries and replace 10,000, then remove 10,000 */
#define ADD_N 10000
static uint64_t run_add_remove(void *ctx)
{
	git_index_entry e;
	size_t i;
	(void)ctx;
	memset(&e, 0, sizeof(e));
	e.mode = GIT_FILEMODE_BLOB;
	for (i = 0; i < ADD_N; i++) {
		char path[128];
		snprintf(path, sizeof(path), "new%02zu/dir%zu/added_%05zu.txt", i % 37, i % 11, i);
		e.path = path;
		e.file_size = (uint32_t)i;
		e.id = git_index_get_byindex(idx, i * 7)->id;
		CHECK(git_index_add(idx, &e));
		e.path = paths[i];
		e.file_size = (uint32_t)i + 1;
		CHECK(git_index_add(idx, &e));
	}
	for (i = 0; i < ADD_N; i++)
		CHECK(git_index_remove(idx, paths[ADD_N + i], 0));
	return index_check(idx);
}

static uint64_t run_remove_dir(void *ctx)
{
	size_t i;
	(void)ctx;
	for (i = 0; i < 20; i++) {
		char dir[32];
		snprintf(dir, sizeof(dir), "src%03zu", i * 2);
		CHECK(git_index_remove_directory(idx, dir, 0));
	}
	return index_check(idx);
}

/* conflicts: add 5,000 three-stage conflicts, look them up, iterate,
 * remove half, clean up */
#define CONFLICT_N 5000
static uint64_t run_conflicts(void *ctx)
{
	git_index_entry anc, ours, theirs;
	const git_index_entry *a, *o, *t;
	git_index_conflict_iterator *it;
	uint64_t h = 0;
	size_t i;
	int err;
	(void)ctx;
	memset(&anc, 0, sizeof(anc));
	anc.mode = ours.mode = theirs.mode = GIT_FILEMODE_BLOB;
	ours = theirs = anc;
	for (i = 0; i < CONFLICT_N; i++) {
		anc.path = ours.path = theirs.path = paths[i];
		anc.id = git_index_get_byindex(idx, i)->id;
		ours.id = git_index_get_byindex(idx, i + 1)->id;
		theirs.id = git_index_get_byindex(idx, i + 2)->id;
		CHECK(git_index_conflict_add(idx, &anc, &ours, &theirs));
	}
	h += (uint64_t)git_index_has_conflicts(idx);
	for (i = 0; i < CONFLICT_N; i++) {
		CHECK(git_index_conflict_get(&a, &o, &t, idx, paths[i]));
		h += a->id.id[0] + o->id.id[1] + t->id.id[2];
	}
	CHECK(git_index_conflict_iterator_new(&it, idx));
	while ((err = git_index_conflict_next(&a, &o, &t, it)) == 0)
		h += strlen(a->path);
	if (err != GIT_ITEROVER) CHECK(err);
	git_index_conflict_iterator_free(it);
	for (i = 0; i < CONFLICT_N / 2; i++)
		CHECK(git_index_conflict_remove(idx, paths[i]));
	git_index_conflict_cleanup(idx);
	h += (uint64_t)git_index_has_conflicts(idx);
	return h ^ index_check(idx);
}

/* in-memory index from the v2.47.0 tree (git read-tree), and write-tree */
static git_repository *repo;
static git_tree *tree;

static void setup_tree(void *ctx)
{
	git_object *o;
	(void)ctx;
	CHECK(git_repository_open(&repo, bench_data("git.git")));
	CHECK(git_revparse_single(&o, repo, "v2.47.0^{tree}"));
	tree = (git_tree *)o;
	CHECK(git_index_new(&idx));
	/* warm the tree cache of the repository */
	CHECK(git_index_read_tree(idx, tree));
	git_index_free(idx);
	CHECK(git_index_new(&idx));
}

static void teardown_tree(void *ctx)
{
	(void)ctx;
	git_index_free(idx);
	git_tree_free(tree);
	git_repository_free(repo);
	idx = NULL; tree = NULL; repo = NULL;
}

static uint64_t run_read_tree(void *ctx)
{
	int i;
	uint64_t h = 0;
	(void)ctx;
	for (i = 0; i < 10; i++) {
		CHECK(git_index_read_tree(idx, tree));
		h += index_check(idx);
	}
	return h;
}

/* write-tree of git v2.47.0 from an index without a tree cache: computes
 * every tree and finds it present in git.git's odb */
static void setup_write_tree(void *ctx)
{
	git_index *src;
	size_t i, n;
	(void)ctx;
	CHECK(git_repository_open(&repo, bench_data("git.git")));
	CHECK(git_index_open(&src, bench_data(WD)));
	CHECK(git_index_new(&idx));
	n = git_index_entrycount(src);
	for (i = 0; i < n; i++)
		CHECK(git_index_add(idx, git_index_get_byindex(src, i)));
	git_index_free(src);
}

static void teardown_write_tree(void *ctx)
{
	(void)ctx;
	git_index_free(idx);
	git_repository_free(repo);
	idx = NULL; repo = NULL;
}

static uint64_t run_write_tree(void *ctx)
{
	git_oid id;
	(void)ctx;
	CHECK(git_index_write_tree_to(&id, idx, repo));
	return fnv_oid(FNV_INIT, &id);
}

static bench_t benches[] = {
	{ "index", "read_100k", NULL, NULL, run_read_big, NULL, NULL, 1 },
	{ "index", "read_git_x10", NULL, NULL, run_read_wd, NULL, NULL, 10 },
	{ "index", "write_100k", NULL, setup_big, run_write, teardown_copy, NULL, 1 },
	{ "index", "write_git", NULL, setup_wd, run_write, teardown_copy, NULL, 1 },
	{ "index", "find_bypath_100k", prep_paths, setup_big, run_find, teardown_copy, NULL, 0 },
	{ "index", "find_prefix", NULL, setup_big, run_find_prefix, teardown_copy, NULL, 10000 },
	{ "index", "add_remove_100k", prep_paths, setup_big, run_add_remove, teardown_copy, NULL, ADD_N * 3 },
	{ "index", "remove_directory", NULL, setup_big, run_remove_dir, teardown_copy, NULL, 20 },
	{ "index", "conflicts", prep_paths, setup_big, run_conflicts, teardown_copy, NULL, CONFLICT_N },
	{ "index", "read_tree_x10", NULL, setup_tree, run_read_tree, teardown_tree, NULL, 10 },
	{ "index", "write_tree_git", NULL, setup_write_tree, run_write_tree, teardown_write_tree, NULL, 1 },
};

BENCH_MAIN(benches)
