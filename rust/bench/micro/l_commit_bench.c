/* Per-call cost of commit.c, commit_list.c and tree.c (WP l-commit) on
 * git.git's objects, through the internal API (in memory: the raw objects
 * are read in the untimed setup), so that parsing is measured without the
 * odb.  Each line: "bench <name> <ns per object>" (best of 5 runs).
 *
 *   commit_parse_full     git_commit__parse_raw of every commit + git_commit__free
 *   commit_parse_quick    git_commit__parse_ext(QUICK) of every commit (revwalk's parse)
 *   commit_summary_body   parse + git_commit_summary + git_commit_body + header_field
 *   tree_parse            git_tree__parse_raw of 40,000 trees + git_tree__free
 *   tree_byname           git_tree_entry_byname of every entry of those trees
 *   treebuilder_from_tree git_treebuilder_new(source) + entrycount + free
 *   revwalk_all           git_revwalk of every commit of git.git (commit_list parse, odb reads)
 *
 * difftest-link: libgit2_tests
 *
 * Run: rust/bench/micro/run_l_commit.sh [ROUNDS] [FILTER]
 */
#include "common.h"
#include "git2.h"
#include "commit.h"
#include "tree.h"
#include "odb.h"
#include "object.h"
#include <time.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e3 + t.tv_nsec / 1e6; }
static volatile size_t sink;

typedef struct { char *data; size_t len; git_oid id; } raw_obj;
static raw_obj *commits, *trees;
static size_t ncommits, ntrees;
#define MAX_TREES 40000

static git_repository *repo;

static int collect(const git_oid *id, void *payload)
{
	git_odb *odb = payload;
	size_t len;
	git_object_t t;
	git_odb_object *o;
	raw_obj **arr;
	size_t *n;
	if (git_odb_read_header(&len, &t, odb, id) < 0)
		return -1;
	if (t == GIT_OBJECT_COMMIT) { arr = &commits; n = &ncommits; }
	else if (t == GIT_OBJECT_TREE && ntrees < MAX_TREES) { arr = &trees; n = &ntrees; }
	else return 0;
	if (git_odb_read(&o, odb, id) < 0)
		return -1;
	if ((*n & (*n - 1)) == 0)
		*arr = realloc(*arr, (*n ? *n * 2 : 1) * sizeof(raw_obj));
	(*arr)[*n].len = git_odb_object_size(o);
	(*arr)[*n].data = malloc((*arr)[*n].len + 1);
	memcpy((*arr)[*n].data, git_odb_object_data(o), (*arr)[*n].len + 1);
	(*arr)[*n].id = *id;
	(*n)++;
	git_odb_object_free(o);
	return 0;
}

#define BENCH(name, n, ...) do { double best = 1e18; int r_; size_t i; \
	if (only && !strstr(name, only)) break; \
	for (r_ = 0; r_ < 5; r_++) { double t = now(); for (i = 0; i < (n); i++) { __VA_ARGS__; } t = now() - t; if (t < best) best = t; } \
	printf("bench %-22s %8.1f ns\n", name, best * 1e6 / (n)); } while (0)

static void parse_commit(size_t i, int quick)
{
	git_commit *c = git__calloc(1, sizeof(git_commit));
	c->object.repo = repo;
	if (quick) {
		git_odb_object fake;
		git_commit__parse_options opts = { GIT_OID_SHA1, GIT_COMMIT_PARSE_QUICK };
		memset(&fake, 0, sizeof(fake));
		fake.cached.oid = commits[i].id;
		fake.cached.size = commits[i].len;
		fake.cached.type = GIT_OBJECT_COMMIT;
		fake.buffer = commits[i].data;
		if (git_commit__parse_ext(c, &fake, &opts) < 0)
			abort();
		sink += (size_t)c->committer->when.time + c->parent_ids.size;
	} else {
		if (git_commit__parse_raw(c, commits[i].data, commits[i].len, GIT_OID_SHA1) < 0)
			abort();
		sink += c->parent_ids.size + strlen(c->author->name);
	}
	git_commit__free(c);
}

int main(int argc, char **argv)
{
	const char *only = argc > 1 ? argv[1] : NULL;
	git_odb *odb;
	git_tree **parsed;
	size_t j, nentries = 0;
	git_buf b = GIT_BUF_INIT;

	git_libgit2_init();
	if (git_repository_open(&repo, "/work/bench-data/git.git") < 0)
		return 1;
	git_repository_odb(&odb, repo);
	git_odb_foreach(odb, collect, odb);
	fprintf(stderr, "%zu commits, %zu trees\n", ncommits, ntrees);

	BENCH("commit_parse_full", ncommits, parse_commit(i, 0));
	BENCH("commit_parse_quick", ncommits, parse_commit(i, 1));
	BENCH("commit_summary_body", ncommits, {
		git_commit *c = git__calloc(1, sizeof(git_commit));
		git_commit__parse_raw(c, commits[i].data, commits[i].len, GIT_OID_SHA1);
		sink += strlen(git_commit_summary(c));
		sink += git_commit_body(c) ? 1 : 0;
		if (git_commit_header_field(&b, c, "committer") == 0)
			sink += b.size;
		git_commit__free(c);
	});
	BENCH("tree_parse", ntrees, {
		git_tree *t = git__calloc(1, sizeof(git_tree));
		if (git_tree__parse_raw(t, trees[i].data, trees[i].len, GIT_OID_SHA1) < 0)
			abort();
		sink += t->entries.size;
		git_tree__free(t);
	});

	parsed = malloc(ntrees * sizeof(*parsed));
	for (j = 0; j < ntrees; j++) {
		parsed[j] = git__calloc(1, sizeof(git_tree));
		parsed[j]->object.repo = repo;
		git_tree__parse_raw(parsed[j], trees[j].data, trees[j].len, GIT_OID_SHA1);
		nentries += parsed[j]->entries.size;
	}
	BENCH("tree_byname", ntrees, {
		size_t k, n = git_tree_entrycount(parsed[i]);
		for (k = 0; k < n; k++)
			sink += (size_t)git_tree_entry_byname(parsed[i], git_tree_entry_byindex(parsed[i], k)->filename);
	});
	BENCH("treebuilder_from_tree", ntrees, {
		git_treebuilder *bld;
		git_treebuilder_new(&bld, repo, parsed[i]);
		sink += git_treebuilder_entrycount(bld);
		git_treebuilder_free(bld);
	});
	BENCH("revwalk_all", 1, {
		git_revwalk *w;
		git_oid id;
		git_repository *r;
		git_repository_open(&r, "/work/bench-data/git.git");
		git_revwalk_new(&w, r);
		git_revwalk_push_glob(w, "refs/*");
		while (git_revwalk_next(&id, w) == 0)
			sink++;
		git_revwalk_free(w);
		git_repository_free(r);
	});
	fprintf(stderr, "%zu tree entries\n", nentries);
	git_buf_dispose(&b);
	git_odb_free(odb);
	return 0;
}
