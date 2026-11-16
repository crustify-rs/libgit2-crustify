/* repository: open (bare, with a working directory), discover, init, head
 * resolution. */
#include "bench.h"

#define OPEN_N 1000

static uint64_t open_n(const char *rel, unsigned flags)
{
	uint64_t h = 0;
	int i;
	for (i = 0; i < OPEN_N; i++) {
		git_repository *r;
		CHECK(git_repository_open_ext(&r, bench_data(rel), flags, NULL));
		h += (uint64_t)git_repository_is_bare(r) + strlen(git_repository_path(r));
		git_repository_free(r);
	}
	return h;
}

static uint64_t run_open_bare(void *ctx) { (void)ctx; return open_n("git.git", GIT_REPOSITORY_OPEN_NO_SEARCH); }
static uint64_t run_open_wd(void *ctx) { (void)ctx; return open_n("git-wd", GIT_REPOSITORY_OPEN_NO_SEARCH); }
static uint64_t run_open_search(void *ctx) { (void)ctx; return open_n("git-wd/t/a/b/c/d/e/f/g/h", 0); }

static uint64_t run_discover(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < OPEN_N; i++) {
		git_buf buf = GIT_BUF_INIT;
		CHECK(git_repository_discover(&buf, bench_data("git-wd/t/a/b/c/d/e/f/g/h"), 0, NULL));
		h += buf.size;
		git_buf_dispose(&buf);
	}
	return h;
}

#define INIT_N 50
static const char *initdir;
static void setup_init(void *ctx) { (void)ctx; initdir = bench_tmpdir("init"); }
static void teardown_init(void *ctx) { (void)ctx; rm_rf(initdir); }

static uint64_t run_init(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < INIT_N; i++) {
		git_repository *r;
		char path[4096];
		snprintf(path, sizeof(path), "%s/r%d", initdir, i);
		CHECK(git_repository_init(&r, path, i % 2));
		h += (uint64_t)git_repository_is_empty(r) + (uint64_t)git_repository_head_unborn(r) * 2;
		git_repository_free(r);
	}
	return h;
}

static uint64_t run_init_ext(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < INIT_N; i++) {
		git_repository *r;
		git_repository_init_options opts = GIT_REPOSITORY_INIT_OPTIONS_INIT;
		char path[4096];
		snprintf(path, sizeof(path), "%s/x%d/sub/dir", initdir, i);
		opts.flags = GIT_REPOSITORY_INIT_MKPATH | GIT_REPOSITORY_INIT_NO_REINIT;
		opts.initial_head = "main";
		opts.description = "bench repository";
		CHECK(git_repository_init_ext(&r, path, &opts));
		h += strlen(git_repository_workdir(r));
		git_repository_free(r);
	}
	return h;
}

/* HEAD resolution in an open repository */
#define HEAD_N 3000
static git_repository *repo;
static void open_gitgit(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("git.git"))); }
static void open_refs(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("refs.git"))); }
static void close_repo(void *ctx) { (void)ctx; git_repository_free(repo); repo = NULL; }

static uint64_t run_head(void *ctx)
{
	uint64_t h = FNV_INIT;
	int i;
	(void)ctx;
	for (i = 0; i < HEAD_N; i++) {
		git_reference *r;
		CHECK(git_repository_head(&r, repo));
		h = fnv_oid(h, git_reference_target(r));
		git_reference_free(r);
		h += (uint64_t)git_repository_head_detached(repo);
	}
	return h;
}

static uint64_t run_head_commit(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < HEAD_N; i++) {
		git_object *o;
		CHECK(git_revparse_single(&o, repo, "HEAD"));
		h += git_object_id(o)->id[0];
		git_object_free(o);
	}
	return h;
}

static bench_t benches[] = {
	{ "repository", "open_bare_x1000", NULL, NULL, run_open_bare, NULL, NULL, OPEN_N },
	{ "repository", "open_workdir_x1000", NULL, NULL, run_open_wd, NULL, NULL, OPEN_N },
	{ "repository", "open_search_x1000", NULL, NULL, run_open_search, NULL, NULL, OPEN_N },
	{ "repository", "discover_x1000", NULL, NULL, run_discover, NULL, NULL, OPEN_N },
	{ "repository", "init_x50", NULL, setup_init, run_init, teardown_init, NULL, INIT_N },
	{ "repository", "init_ext_x50", NULL, setup_init, run_init_ext, teardown_init, NULL, INIT_N },
	{ "repository", "head_packed", NULL, open_gitgit, run_head, close_repo, NULL, HEAD_N },
	{ "repository", "head_loose", NULL, open_refs, run_head, close_repo, NULL, HEAD_N },
	{ "repository", "revparse_head", NULL, open_gitgit, run_head_commit, close_repo, NULL, HEAD_N },
};

BENCH_MAIN(benches)
