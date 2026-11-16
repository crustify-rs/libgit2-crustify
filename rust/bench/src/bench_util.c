/* util through the public API: git_buf (filled and disposed by the library)
 * and git_strarray (lists returned by the library). */
#include "bench.h"

static git_repository *repo;
static git_commit **commits;
#define COMMITS_N 5000

static void open_gitgit(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("git.git"))); }
static void open_refs(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("refs.git"))); }
static void close_repo(void *ctx) { (void)ctx; git_repository_free(repo); repo = NULL; }

static void setup_commits(void *ctx)
{
	git_revwalk *w;
	size_t i;
	open_gitgit(ctx);
	commits = xmalloc(COMMITS_N * sizeof(*commits));
	CHECK(git_revwalk_new(&w, repo));
	CHECK(git_revwalk_push_ref(w, "refs/tags/v2.47.0"));
	for (i = 0; i < COMMITS_N; i++) {
		git_oid id;
		CHECK(git_revwalk_next(&id, w));
		CHECK(git_commit_lookup(&commits[i], repo, &id));
	}
	git_revwalk_free(w);
}

static void teardown_commits(void *ctx)
{
	size_t i;
	for (i = 0; i < COMMITS_N; i++)
		git_commit_free(commits[i]);
	free(commits);
	close_repo(ctx);
}

/* git_commit_header_field: copies a header value into a git_buf */
static uint64_t run_header_field(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < COMMITS_N * 4; i++) {
		git_buf buf = GIT_BUF_INIT;
		static const char *fields[] = { "tree", "author", "committer", "parent" };
		int e = git_commit_header_field(&buf, commits[i % COMMITS_N], fields[i / COMMITS_N]);
		if (e != GIT_ENOTFOUND) {
			CHECK(e);
			h = fnv(h, buf.ptr, buf.size);
		}
		git_buf_dispose(&buf);
	}
	return h;
}

/* git_commit_extract_signature-free path: git_object_short_id (git_buf of a
 * unique abbreviation; uses the odb prefix lookup) */
#define SHORT_N 2000
static uint64_t run_short_id(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < SHORT_N; i++) {
		git_buf buf = GIT_BUF_INIT;
		CHECK(git_object_short_id(&buf, (git_object *)commits[i]));
		h = fnv(h, buf.ptr, buf.size);
		git_buf_dispose(&buf);
	}
	return h;
}

/* a git_buf reused across calls (grown once, refilled) */
static uint64_t run_buf_reuse(void *ctx)
{
	uint64_t h = FNV_INIT;
	git_buf buf = GIT_BUF_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < COMMITS_N * 4; i++) {
		CHECK(git_commit_header_field(&buf, commits[i % COMMITS_N], i % 2 ? "author" : "committer"));
		h = fnv(h, buf.ptr, buf.size > 8 ? 8 : buf.size);
	}
	git_buf_dispose(&buf);
	return h;
}

/* git_strarray: git_reference_list (56k names) and git_tag_list */
static uint64_t run_reference_list(void *ctx)
{
	git_strarray a = { 0 };
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	CHECK(git_reference_list(&a, repo));
	for (i = 0; i < a.count; i++)
		h += strlen(a.strings[i]);
	h = (h << 20) ^ a.count;
	git_strarray_dispose(&a);
	return h;
}

/* git_tag_list on git.git (~1,000 tags) */
static uint64_t run_tag_list(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 5; i++) {
		git_strarray a = { 0 };
		size_t j;
		CHECK(git_tag_list(&a, repo));
		for (j = 0; j < a.count; j++)
			h += strlen(a.strings[j]);
		h = h * 31 + a.count;
		git_strarray_dispose(&a);
	}
	return h;
}

/* small lists: git_tag_list_match with a pattern */
static uint64_t run_small_lists(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 20; i++) {
		git_strarray a = { 0 };
		CHECK(git_tag_list_match(&a, "v2.4?.0", repo));
		h += a.count;
		git_strarray_dispose(&a);
	}
	return h;
}

static bench_t benches[] = {
	{ "util", "buf_commit_header_field", NULL, setup_commits, run_header_field, teardown_commits, NULL, COMMITS_N * 4 },
	{ "util", "buf_reuse", NULL, setup_commits, run_buf_reuse, teardown_commits, NULL, COMMITS_N * 4 },
	{ "util", "buf_object_short_id", NULL, setup_commits, run_short_id, teardown_commits, NULL, SHORT_N },
	{ "util", "strarray_reference_list_56k", NULL, open_refs, run_reference_list, close_repo, NULL, 1 },
	{ "util", "strarray_tag_list_x5", NULL, open_gitgit, run_tag_list, close_repo, NULL, 5 },
	{ "util", "strarray_tag_list_match_x20", NULL, open_gitgit, run_small_lists, close_repo, NULL, 20 },
};

BENCH_MAIN(benches)
