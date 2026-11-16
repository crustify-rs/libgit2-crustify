/* codec (WP u2): hashing, compression and wildmatch through the public API.
 *
 * - mempack_hash_*: git_odb_write into a mempack odb (SHA-1 with
 *   collision detection, and a copy) of a 64 MiB blob and of 100k
 *   object-sized blobs.
 * - repository_hashfile_16m: git_repository_hashfile of a 16 MiB file in
 *   a bare repository (no filters: read and hashed).
 * - pathspec_match: git_pathspec_matches_path (wildmatch) of git-wd's
 *   index paths against typical patterns.
 * Loose and pack reads (inflate, hash) are in bench_odb.c; deflate (only
 * pack-objects and binary diffs reach libgit2's zstream for it, loose
 * writes deflate in filebuf) is in micro/codec_bench.c. */
#include "bench.h"

#define BIG (64u << 20)
#define SMALL_N 100000
#define FILE_SIZE (16u << 20)

static unsigned char *big;
static size_t small_len[SMALL_N];
static size_t small_off[SMALL_N];
static char hashfile[4096];

static void prep_big(void *ctx)
{
	static int done;
	size_t i;
	(void)ctx;
	if (!done) {
		big = xmalloc(BIG);
		rng_seed(7);
		for (i = 0; i < BIG / 8; i++) {
			uint64_t r = rng_next();
			memcpy(big + 8 * i, &r, 8);
		}
		for (i = 0; i < SMALL_N; i++) {
			small_len[i] = 50 + rng_next() % 4000;
			small_off[i] = rng_next() % (BIG - 8192);
		}
		done = 1;
	}
}

static git_odb *modb;
static git_odb_backend *mempack;
static git_repository *bare;

static void open_mempack(void)
{
	if (!modb) {
		CHECK(git_odb_new(&modb));
		CHECK(git_mempack_new(&mempack));
		CHECK(git_odb_add_backend(modb, mempack, 1));
	}
}

static void prep_hash_big(void *ctx) { prep_big(ctx); open_mempack(); bench_ops = 1; }
static void prep_hash_small(void *ctx) { prep_big(ctx); open_mempack(); bench_ops = SMALL_N; }
static void reset_mempack(void *ctx) { (void)ctx; CHECK(git_mempack_reset(mempack)); }

static uint64_t run_hash_big(void *ctx)
{
	git_oid id;
	(void)ctx;
	CHECK(git_odb_write(&id, modb, big, BIG, GIT_OBJECT_BLOB));
	return fnv_oid(FNV_INIT, &id);
}

static uint64_t run_hash_small(void *ctx)
{
	uint64_t h = FNV_INIT;
	git_oid id;
	size_t i;
	(void)ctx;
	for (i = 0; i < SMALL_N; i++) {
		CHECK(git_odb_write(&id, modb, big + small_off[i], small_len[i], GIT_OBJECT_BLOB));
		h = fnv_oid(h, &id);
	}
	return h;
}

static void prep_hashfile(void *ctx)
{
	prep_big(ctx);
	if (!hashfile[0]) {
		snprintf(hashfile, sizeof(hashfile), "%s/file", bench_tmpdir("hashfile"));
		write_file(hashfile, big, FILE_SIZE);
		CHECK(git_repository_open_bare(&bare, bench_data("git.git")));
	}
	bench_ops = 1;
}

static uint64_t run_hashfile(void *ctx)
{
	git_oid id;
	(void)ctx;
	CHECK(git_repository_hashfile(&id, bare, hashfile, GIT_OBJECT_BLOB, NULL));
	return fnv_oid(FNV_INIT, &id);
}

/* pathspec matching */
static char **paths;
static size_t npaths;
static git_pathspec *specs[6];
static const char *patterns[6][2] = {
	{ "*.c", NULL }, { "t/**/*.sh", NULL }, { "Documentation/*.txt", "!Documentation/RelNotes/*" },
	{ "*[0-9]*", NULL }, { "**/Makefile", NULL }, { "contrib/*/", "*.py" },
};

static void prep_pathspec(void *ctx)
{
	static int done;
	(void)ctx;
	if (!done) {
		git_repository *repo;
		git_index *idx;
		size_t i;
		CHECK(git_repository_open(&repo, bench_data("git-wd")));
		CHECK(git_repository_index(&idx, repo));
		npaths = git_index_entrycount(idx);
		paths = xmalloc(npaths * sizeof(*paths));
		for (i = 0; i < npaths; i++)
			paths[i] = xstrdup(git_index_get_byindex(idx, i)->path);
		git_index_free(idx);
		git_repository_free(repo);
		for (i = 0; i < 6; i++) {
			char *p[2] = { (char *)patterns[i][0], (char *)patterns[i][1] };
			git_strarray a = { p, patterns[i][1] ? 2 : 1 };
			CHECK(git_pathspec_new(&specs[i], &a));
		}
		done = 1;
	}
	bench_ops = 20 * npaths * 6;
}

static uint64_t run_pathspec(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t r, i, s;
	(void)ctx;
	for (r = 0; r < 20; r++)
		for (s = 0; s < 6; s++)
			for (i = 0; i < npaths; i++)
				h = fnv_u64(h, (uint64_t)git_pathspec_matches_path(specs[s],
					(s & 1) ? GIT_PATHSPEC_IGNORE_CASE : 0, paths[i]));
	return h;
}

static bench_t benches[] = {
	{ "codec", "mempack_hash_64m", prep_hash_big, reset_mempack, run_hash_big, NULL, NULL, 0 },
	{ "codec", "mempack_hash_small_x100k", prep_hash_small, reset_mempack, run_hash_small, NULL, NULL, 0 },
	{ "codec", "repository_hashfile_16m", prep_hashfile, NULL, run_hashfile, NULL, NULL, 0 },
	{ "codec", "pathspec_match", prep_pathspec, NULL, run_pathspec, NULL, NULL, 0 },
};

BENCH_MAIN(benches)
