/* objects: commit, tree, blob, tag and signature parsing on git.git, and the
 * object cache hit path.  Cold workloads open a fresh repository in setup()
 * (untimed), so the object cache starts empty. */
#include "bench.h"

static git_repository *repo;
static git_oid *commits;   /* all commits reachable from refs */
static size_t ncommits;

static void open_repo(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("git.git"))); }
static void close_repo(void *ctx) { (void)ctx; git_repository_free(repo); repo = NULL; }

static void prep_commits(void *ctx)
{
	(void)ctx;
	if (!commits) {
		git_repository *r;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		commits = all_commits(r, GIT_SORT_NONE, &ncommits);
		git_repository_free(r);
	}
	bench_ops = ncommits;
}

/* commit lookup + parse of every commit, reading every field */
static uint64_t run_commit_lookup_all(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < ncommits; i++) {
		git_commit *c;
		const git_signature *a;
		unsigned p, np;
		CHECK(git_commit_lookup(&c, repo, &commits[i]));
		a = git_commit_author(c);
		h = fnv_u64(h, (uint64_t)git_commit_time(c));
		h = fnv_u64(h, (uint64_t)a->when.time + (uint64_t)strlen(a->email));
		h = fnv_u64(h, strlen(git_commit_message(c)));
		h = fnv_u64(h, strlen(git_commit_summary(c)));
		h = fnv_oid(h, git_commit_tree_id(c));
		np = git_commit_parentcount(c);
		for (p = 0; p < np; p++)
			h = fnv_oid(h, git_commit_parent_id(c, p));
		h = fnv_u64(h, strlen(git_commit_committer(c)->name));
		git_commit_free(c);
	}
	return h;
}

/* cache hit path: 5,000 commits looked up 40 times each in a warm repo */
#define HIT_N 5000
#define HIT_ROUNDS 40
static void prep_hit(void *ctx) { prep_commits(ctx); bench_ops = HIT_N * HIT_ROUNDS * 2; }
static void open_warm(void *ctx)
{
	size_t i;
	open_repo(ctx);
	for (i = 0; i < HIT_N; i++) {
		git_commit *c;
		CHECK(git_commit_lookup(&c, repo, &commits[i]));
		git_commit_free(c);
	}
}

static uint64_t run_cache_hit(void *ctx)
{
	uint64_t h = 0;
	size_t i, r;
	(void)ctx;
	for (r = 0; r < HIT_ROUNDS; r++)
		for (i = 0; i < HIT_N; i++) {
			git_commit *c;
			git_object *o;
			CHECK(git_commit_lookup(&c, repo, &commits[i]));
			h += git_commit_parentcount(c);
			git_commit_free(c);
			CHECK(git_object_lookup(&o, repo, &commits[i], GIT_OBJECT_ANY));
			h += (uint64_t)git_object_type(o);
			git_object_free(o);
		}
	return h;
}

/* trees: the root tree of 300 first-parent commits, then every subtree
 * reachable from them (mostly cache hits on shared subtrees), iterating
 * every entry */
#define TREE_COMMITS 300
static git_oid tree_roots[TREE_COMMITS];

static void prep_trees(void *ctx)
{
	static int done;
	(void)ctx;
	if (!done) {
		git_repository *r;
		git_revwalk *w;
		size_t i;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_revwalk_new(&w, r));
		git_revwalk_simplify_first_parent(w);
		CHECK(git_revwalk_push_ref(w, "refs/tags/v2.47.0"));
		for (i = 0; i < TREE_COMMITS; i++) {
			git_oid id;
			git_commit *c;
			/* every 20th first-parent commit: distinct trees */
			int k;
			for (k = 0; k < 20; k++)
				CHECK(git_revwalk_next(&id, w));
			CHECK(git_commit_lookup(&c, r, &id));
			tree_roots[i] = *git_commit_tree_id(c);
			git_commit_free(c);
		}
		git_revwalk_free(w);
		git_repository_free(r);
		done = 1;
	}
	bench_ops = TREE_COMMITS;
}

static uint64_t iterate_tree(const git_oid *id)
{
	git_tree *t;
	size_t i, n;
	uint64_t h = 0;
	CHECK(git_tree_lookup(&t, repo, id));
	n = git_tree_entrycount(t);
	for (i = 0; i < n; i++) {
		const git_tree_entry *e = git_tree_entry_byindex(t, i);
		const char *name = git_tree_entry_name(e);
		h += (uint64_t)git_tree_entry_filemode(e) + strlen(name) + git_tree_entry_id(e)->id[0];
		if (git_tree_entry_type(e) == GIT_OBJECT_TREE)
			h += iterate_tree(git_tree_entry_id(e));
	}
	git_tree_free(t);
	return h + n;
}

static uint64_t run_tree_iterate(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < TREE_COMMITS; i++)
		h = h * 31 + iterate_tree(&tree_roots[i]);
	return h;
}

static int walk_cb(const char *root, const git_tree_entry *e, void *payload)
{
	uint64_t *h = payload;
	*h = *h * 31 + strlen(root) + strlen(git_tree_entry_name(e));
	return 0;
}

static uint64_t run_tree_walk(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < 20; i++) {
		git_tree *t;
		CHECK(git_tree_lookup(&t, repo, &tree_roots[i]));
		CHECK(git_tree_walk(t, i % 2 ? GIT_TREEWALK_POST : GIT_TREEWALK_PRE, walk_cb, &h));
		git_tree_free(t);
	}
	return h;
}

/* tree entry lookup by name and by path, in a warm repo */
static char **paths;
static size_t npaths;
static git_tree *head_tree;

static int path_cb(const char *root, const git_tree_entry *e, void *payload)
{
	char buf[4096];
	(void)payload;
	if (git_tree_entry_type(e) != GIT_OBJECT_BLOB) return 0;
	snprintf(buf, sizeof(buf), "%s%s", root, git_tree_entry_name(e));
	paths = xrealloc(paths, (npaths + 1) * sizeof(*paths));
	paths[npaths++] = xstrdup(buf);
	return 0;
}

static void prep_paths(void *ctx)
{
	prep_trees(ctx);
	if (!paths) {
		git_repository *r;
		git_tree *t;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_tree_lookup(&t, r, &tree_roots[0]));
		CHECK(git_tree_walk(t, GIT_TREEWALK_PRE, path_cb, NULL));
		git_tree_free(t);
		git_repository_free(r);
		rng_seed(5);
		shuffle(paths, npaths, sizeof(*paths));
	}
	bench_ops = npaths * 10;
}

/* Trees larger than 4 KiB are not cached by default (the root tree and t/
 * are), which makes every bypath lookup re-read them from the pack.  Allow
 * caching them here so that the workload measures the lookup itself. */
static void open_head_tree(void *ctx)
{
	CHECK(git_libgit2_opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, GIT_OBJECT_TREE, (size_t)1 << 20));
	open_repo(ctx);
	CHECK(git_tree_lookup(&head_tree, repo, &tree_roots[0]));
}

static void close_head_tree(void *ctx)
{
	git_tree_free(head_tree);
	close_repo(ctx);
	CHECK(git_libgit2_opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, GIT_OBJECT_TREE, (size_t)4096));
}

static uint64_t run_tree_bypath(void *ctx)
{
	uint64_t h = 0;
	size_t i, r;
	(void)ctx;
	for (r = 0; r < 10; r++)
		for (i = 0; i < npaths; i++) {
			git_tree_entry *e;
			const char *slash = strrchr(paths[i], '/');
			CHECK(git_tree_entry_bypath(&e, head_tree, paths[i]));
			h += git_tree_entry_id(e)->id[1];
			git_tree_entry_free(e);
			if (!slash) {
				const git_tree_entry *ne = git_tree_entry_byname(head_tree, paths[i]);
				h += ne ? 1 : 0;
			}
		}
	return h;
}

/* blobs: every blob of the v2.47.0 tree */
static git_oid *blobs;
static size_t nblobs;

static int blob_cb(const char *root, const git_tree_entry *e, void *payload)
{
	(void)root; (void)payload;
	if (git_tree_entry_type(e) != GIT_OBJECT_BLOB) return 0;
	blobs = xrealloc(blobs, (nblobs + 1) * sizeof(*blobs));
	blobs[nblobs++] = *git_tree_entry_id(e);
	return 0;
}

static void prep_blobs(void *ctx)
{
	prep_trees(ctx);
	if (!blobs) {
		git_repository *r;
		git_tree *t;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_tree_lookup(&t, r, &tree_roots[0]));
		CHECK(git_tree_walk(t, GIT_TREEWALK_PRE, blob_cb, NULL));
		git_tree_free(t);
		git_repository_free(r);
	}
	bench_ops = nblobs;
}

static uint64_t run_blob_lookup(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < nblobs; i++) {
		git_blob *b;
		CHECK(git_blob_lookup(&b, repo, &blobs[i]));
		h = fnv_sample(h, git_blob_rawcontent(b), (size_t)git_blob_rawsize(b));
		h = fnv_u64(h, (uint64_t)git_blob_is_binary(b));
		git_blob_free(b);
	}
	return h;
}

/* annotated tags: all of them */
static git_oid *tags;
static size_t ntags;

static int tag_cb(const char *name, git_oid *oid, void *payload)
{
	git_repository *r = payload;
	git_object_t t;
	size_t len;
	git_odb *odb;
	(void)name;
	CHECK(git_repository_odb(&odb, r));
	CHECK(git_odb_read_header(&len, &t, odb, oid));
	git_odb_free(odb);
	if (t == GIT_OBJECT_TAG) {
		tags = xrealloc(tags, (ntags + 1) * sizeof(*tags));
		tags[ntags++] = *oid;
	}
	return 0;
}

static void prep_tags(void *ctx)
{
	(void)ctx;
	if (!tags) {
		git_repository *r;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_tag_foreach(r, tag_cb, r));
		git_repository_free(r);
	}
	bench_ops = ntags;
}

static uint64_t run_tag_lookup(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < ntags; i++) {
		git_tag *t;
		const git_signature *s;
		CHECK(git_tag_lookup(&t, repo, &tags[i]));
		h = fnv_str(h, git_tag_name(t));
		h = fnv_u64(h, strlen(git_tag_message(t)));
		h = fnv_oid(h, git_tag_target_id(t));
		h = fnv_u64(h, (uint64_t)git_tag_target_type(t));
		s = git_tag_tagger(t);
		h = fnv_u64(h, s ? (uint64_t)s->when.time : 0);
		git_tag_free(t);
	}
	return h;
}

/* signatures: parse author lines of 50,000 commits */
#define SIG_N 50000
static char **sigs;

static void prep_sigs(void *ctx)
{
	prep_commits(ctx);
	if (!sigs) {
		git_repository *r;
		size_t i;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		sigs = xmalloc(SIG_N * sizeof(*sigs));
		for (i = 0; i < SIG_N; i++) {
			git_commit *c;
			const char *raw, *p, *e;
			CHECK(git_commit_lookup(&c, r, &commits[i % ncommits]));
			raw = git_commit_raw_header(c);
			p = strstr(raw, i % 2 ? "\ncommitter " : "\nauthor ");
			p = strchr(p, ' ') + 1;
			e = strchr(p, '\n');
			sigs[i] = xmalloc((size_t)(e - p) + 1);
			memcpy(sigs[i], p, (size_t)(e - p));
			sigs[i][e - p] = 0;
			git_commit_free(c);
		}
		git_repository_free(r);
	}
	bench_ops = SIG_N;
}

static uint64_t run_sig_parse(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < SIG_N; i++) {
		git_signature *s, *d;
		CHECK(git_signature_from_buffer(&s, sigs[i]));
		h = fnv_u64(h, (uint64_t)s->when.time + (uint64_t)s->when.offset);
		h = fnv_u64(h, strlen(s->name) * 256 + strlen(s->email));
		CHECK(git_signature_dup(&d, s));
		git_signature_free(s);
		git_signature_free(d);
	}
	return h;
}

static uint64_t run_sig_new(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < SIG_N; i++) {
		git_signature *s;
		CHECK(git_signature_new(&s, "  A U Thor  ", "author@example.com", 1700000000 + (git_time_t)i, (int)(i % 600) - 300));
		h += strlen(s->name) + strlen(s->email) + (uint64_t)s->when.offset;
		git_signature_free(s);
	}
	return h;
}

/* commit creation (git_commit_create_buffer) for 20,000 commits */
#define CREATE_N 20000
static void prep_create(void *ctx) { prep_commits(ctx); bench_ops = CREATE_N; }

static uint64_t run_commit_create_buffer(void *ctx)
{
	uint64_t h = FNV_INIT;
	git_signature *sig;
	git_commit *parent;
	git_tree *tree;
	size_t i;
	(void)ctx;
	CHECK(git_signature_new(&sig, "Bench Mark", "bench@example.com", 1700000000, 60));
	CHECK(git_commit_lookup(&parent, repo, &commits[0]));
	CHECK(git_commit_tree(&tree, parent));
	for (i = 0; i < CREATE_N; i++) {
		git_buf buf = GIT_BUF_INIT;
		const git_commit *parents[1] = { parent };
		char msg[128];
		snprintf(msg, sizeof(msg), "commit number %zu\n\nwith a body line.\n", i);
		CHECK(git_commit_create_buffer(&buf, repo, sig, sig, NULL, msg, tree, 1, parents));
		h = fnv_u64(h, buf.size);
		h = fnv(h, buf.ptr + buf.size - 20, 20);
		git_buf_dispose(&buf);
	}
	git_tree_free(tree);
	git_commit_free(parent);
	git_signature_free(sig);
	return h;
}

static bench_t benches[] = {
	{ "objects", "commit_lookup_all", prep_commits, open_repo, run_commit_lookup_all, close_repo, NULL, 0 },
	{ "objects", "cache_hit", prep_hit, open_warm, run_cache_hit, close_repo, NULL, 0 },
	{ "objects", "tree_lookup_iterate", prep_trees, open_repo, run_tree_iterate, close_repo, NULL, 0 },
	{ "objects", "tree_walk", prep_trees, open_repo, run_tree_walk, close_repo, NULL, 0 },
	{ "objects", "tree_entry_bypath", prep_paths, open_head_tree, run_tree_bypath, close_head_tree, NULL, 0 },
	{ "objects", "blob_lookup", prep_blobs, open_repo, run_blob_lookup, close_repo, NULL, 0 },
	{ "objects", "tag_lookup_all", prep_tags, open_repo, run_tag_lookup, close_repo, NULL, 0 },
	{ "objects", "signature_parse", prep_sigs, NULL, run_sig_parse, NULL, NULL, 0 },
	{ "objects", "signature_new", NULL, NULL, run_sig_new, NULL, NULL, SIG_N },
	{ "objects", "commit_create_buffer", prep_create, open_repo, run_commit_create_buffer, close_repo, NULL, 0 },
};

BENCH_MAIN(benches)
