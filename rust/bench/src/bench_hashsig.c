/* hashsig: similarity signatures (rename detection) of the files of git
 * v2.47.0 (~4,500 real source files), of 1 MiB and 16 MiB buffers made from
 * them, and comparison of signatures. */
#include "bench.h"

static char **bufs;
static size_t *lens, nbufs, total;
static char *big;      /* concatenation of all files */
static size_t big_len;
static git_hashsig **sigs;

static int blob_cb(const char *root, const git_tree_entry *e, void *payload)
{
	git_repository *r = payload;
	git_blob *b;
	(void)root;
	if (git_tree_entry_type(e) != GIT_OBJECT_BLOB) return 0;
	CHECK(git_blob_lookup(&b, r, git_tree_entry_id(e)));
	bufs = xrealloc(bufs, (nbufs + 1) * sizeof(*bufs));
	lens = xrealloc(lens, (nbufs + 1) * sizeof(*lens));
	lens[nbufs] = (size_t)git_blob_rawsize(b);
	bufs[nbufs] = xmalloc(lens[nbufs]);
	memcpy(bufs[nbufs], git_blob_rawcontent(b), lens[nbufs]);
	total += lens[nbufs];
	nbufs++;
	git_blob_free(b);
	return 0;
}

static void prep(void *ctx)
{
	(void)ctx;
	if (!bufs) {
		git_repository *r;
		git_object *t;
		size_t i, k;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_revparse_single(&t, r, "v2.47.0^{tree}"));
		CHECK(git_tree_walk((git_tree *)t, GIT_TREEWALK_PRE, blob_cb, r));
		git_object_free(t);
		git_repository_free(r);
		big_len = 16 << 20;
		big = xmalloc(big_len);
		for (i = 0, k = 0; i < big_len; k = (k + 7) % nbufs) {
			size_t n = lens[k] < big_len - i ? lens[k] : big_len - i;
			memcpy(big + i, bufs[k], n);
			i += n;
		}
	}
	bench_ops = nbufs;
}

static uint64_t create_all(git_hashsig_option_t opt)
{
	uint64_t h = 0;
	size_t i;
	for (i = 0; i < nbufs; i++) {
		git_hashsig *s;
		int e = git_hashsig_create(&s, bufs[i], lens[i], opt);
		if (e == GIT_EBUFS) { h += 1; continue; } /* too little data */
		CHECK(e);
		h += 2 + (uint64_t)git_hashsig_compare(s, s);
		git_hashsig_free(s);
	}
	return h;
}

static uint64_t run_files_normal(void *ctx) { (void)ctx; return create_all(GIT_HASHSIG_NORMAL); }
static uint64_t run_files_ignore_ws(void *ctx) { (void)ctx; return create_all(GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_ALLOW_SMALL_FILES); }
static uint64_t run_files_smart_cr(void *ctx) { (void)ctx; return create_all(GIT_HASHSIG_SMART_WHITESPACE); }

static uint64_t create_big(size_t len, int n)
{
	uint64_t h = 0;
	int i;
	for (i = 0; i < n; i++) {
		git_hashsig *s, *t;
		CHECK(git_hashsig_create(&s, big + i * 4096, len, GIT_HASHSIG_NORMAL));
		CHECK(git_hashsig_create(&t, big + i * 4096 + 17, len, GIT_HASHSIG_NORMAL));
		h = h * 101 + (uint64_t)git_hashsig_compare(s, t);
		git_hashsig_free(s);
		git_hashsig_free(t);
	}
	return h;
}

static uint64_t run_1m(void *ctx) { (void)ctx; return create_big(1 << 20, 8); }
static uint64_t run_15m(void *ctx) { (void)ctx; return create_big((16 << 20) - 8 * 4096 - 17, 1); }

/* compare: each signature with its 20 neighbours */
static void setup_sigs(void *ctx)
{
	size_t i;
	(void)ctx;
	if (sigs) return;
	sigs = xmalloc(nbufs * sizeof(*sigs));
	for (i = 0; i < nbufs; i++)
		if (git_hashsig_create(&sigs[i], bufs[i], lens[i], GIT_HASHSIG_ALLOW_SMALL_FILES) < 0)
			sigs[i] = NULL;
}

static uint64_t run_compare(void *ctx)
{
	uint64_t h = 0;
	size_t i, j;
	(void)ctx;
	for (i = 0; i < nbufs; i++)
		for (j = 1; j <= 20; j++) {
			size_t k = (i + j) % nbufs;
			if (sigs[i] && sigs[k])
				h += (uint64_t)git_hashsig_compare(sigs[i], sigs[k]);
		}
	return h;
}

/* from files on disk (git-wd's checkout) */
#define FILES_N 1000
static char *files[FILES_N];
static void prep_files(void *ctx)
{
	git_index *ix;
	size_t i, n;
	prep(ctx);
	if (!files[0]) {
		CHECK(git_index_open(&ix, bench_data("git-wd/.git/index")));
		n = git_index_entrycount(ix);
		for (i = 0; i < FILES_N; i++) {
			char path[4096];
			snprintf(path, sizeof(path), "%s/%s", bench_data("git-wd"), git_index_get_byindex(ix, i * n / FILES_N)->path);
			files[i] = xstrdup(path);
		}
		git_index_free(ix);
	}
	bench_ops = FILES_N;
}

static uint64_t run_fromfile(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < FILES_N; i++) {
		git_hashsig *s;
		int e = git_hashsig_create_fromfile(&s, files[i], GIT_HASHSIG_NORMAL);
		if (e == GIT_EBUFS) { h += 1; continue; }
		CHECK(e);
		h += 2 + (uint64_t)git_hashsig_compare(s, s);
		git_hashsig_free(s);
	}
	return h;
}

static void prep_compare(void *ctx) { prep(ctx); setup_sigs(ctx); bench_ops = nbufs * 20; }

static bench_t benches[] = {
	{ "hashsig", "create_files_normal", prep, NULL, run_files_normal, NULL, NULL, 0 },
	{ "hashsig", "create_files_ignore_ws", prep, NULL, run_files_ignore_ws, NULL, NULL, 0 },
	{ "hashsig", "create_files_smart_ws", prep, NULL, run_files_smart_cr, NULL, NULL, 0 },
	{ "hashsig", "create_1MiB_x16", prep, NULL, run_1m, NULL, NULL, 16 },
	{ "hashsig", "create_16MiB_x2", prep, NULL, run_15m, NULL, NULL, 2 },
	{ "hashsig", "compare", prep_compare, NULL, run_compare, NULL, NULL, 0 },
	{ "hashsig", "create_fromfile", prep_files, NULL, run_fromfile, NULL, NULL, 0 },
};

BENCH_MAIN(benches)
