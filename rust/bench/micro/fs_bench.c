/*
 * Benchmarks of the filesystem layer (fs_path.c, futils.c, filebuf.c),
 * linked against the C reference objects or against the Rust port (same
 * internal C ABI). Prints "bench <name> median_ms <x> min_ms <x> max_ms <x>".
 *
 * Workloads (BENCH_TREE, default /work/bench-data/libgit2-wd: 15.6k files,
 * 5.9k directories):
 *   path_dirname    git_fs_path_dirname_r + basename_r, 1e6 paths
 *   path_join       git_fs_path_join_unrooted + git_fs_path_to_dir, 1e6
 *   path_resolve    git_fs_path_resolve_relative of "a/./b/../c/..." paths, 5e5
 *   path_valid      git_fs_path_is_valid (defaults | DOS | NT chars), 1e6
 *   path_prettify   git_fs_path_prettify of every directory of the tree
 *   diriter_tree    the whole tree with git_fs_path_diriter (+ stat), as
 *                   iterator.c walks a workdir
 *   direach_tree    the whole tree with git_fs_path_direach
 *   readbuffer      git_futils_readbuffer of 4000 files of the tree
 *   filebuf_commit  200 x (open with SHA-1, 64 KiB in 1 KiB writes and
 *                   printf lines, commit) in a scratch directory
 *   mkdir_rmdir     git_futils_mkdir -p of 2000 nested paths, rmdir_r
 */
#include "git2_util.h"
#include "git2/global.h"
#include "fs_path.h"
#include "futils.h"
#include "filebuf.h"
#include "vector.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static volatile size_t g_sink;
static int g_iters = 7;
static const char *g_tree;
static git_vector g_files = GIT_VECTOR_INIT, g_dirs = GIT_VECTOR_INIT;
static char **g_paths;
#define NPATHS 1000000

#define CHECK(expr) do { if ((expr) < 0) { \
	fprintf(stderr, "FAILED: %s at %s:%d\n", #expr, __FILE__, __LINE__); \
	exit(1); } } while (0)

static double now_ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (double)ts.tv_sec * 1e3 + (double)ts.tv_nsec / 1e6;
}

static int cmp_double(const void *a, const void *b)
{
	double x = *(const double *)a, y = *(const double *)b;
	return (x > y) - (x < y);
}

static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void)
{
	rng_state ^= rng_state >> 12;
	rng_state ^= rng_state << 25;
	rng_state ^= rng_state >> 27;
	return rng_state * 2685821657736338717ull;
}

static const char *g_words[] = { "a", "src", "..", ".", "libgit2", "refs", "heads", "main",
	"objects", "pack", "x", "README.md", "0123456789abcdef", "tests", ".git", "HEAD", "" };
#define NWORDS (sizeof(g_words) / sizeof(g_words[0]))

static void setup_paths(void)
{
	size_t i;
	g_paths = malloc(NPATHS * sizeof(char *));
	for (i = 0; i < NPATHS; i++) {
		char tmp[256];
		size_t n = 1 + rng() % 7, j, len = 0;
		if (rng() % 3 == 0)
			tmp[len++] = '/';
		for (j = 0; j < n; j++) {
			const char *w = g_words[rng() % NWORDS];
			len += (size_t)snprintf(tmp + len, sizeof(tmp) - len, "%s%s", w,
				(j + 1 < n || rng() % 4 == 0) ? "/" : "");
		}
		tmp[len] = '\0';
		g_paths[i] = strdup(tmp);
	}
}

static int collect_cb(void *payload, git_str *path)
{
	struct stat st;
	GIT_UNUSED(payload);
	if (p_lstat(path->ptr, &st) < 0)
		return 0;
	if (S_ISDIR(st.st_mode)) {
		git_vector_insert(&g_dirs, git__strdup(path->ptr));
		return git_fs_path_direach(path, 0, collect_cb, NULL);
	}
	if (S_ISREG(st.st_mode))
		git_vector_insert(&g_files, git__strdup(path->ptr));
	return 0;
}

static void setup_tree(void)
{
	git_str p = GIT_STR_INIT;
	git_str_sets(&p, g_tree);
	git_vector_insert(&g_dirs, git__strdup(g_tree));
	CHECK(git_fs_path_direach(&p, 0, collect_cb, NULL));
	git_str_dispose(&p);
}

/* ---- workloads ---- */

static void w_path_dirname(void)
{
	git_str b = GIT_STR_INIT;
	size_t i, n = 0;
	for (i = 0; i < NPATHS; i++) {
		n += (size_t)git_fs_path_dirname_r(&b, g_paths[i]);
		n += (size_t)git_fs_path_basename_r(&b, g_paths[i]);
	}
	g_sink += n + b.size;
	git_str_dispose(&b);
}

static void w_path_join(void)
{
	git_str b = GIT_STR_INIT;
	size_t i, n = 0;
	ssize_t root;
	for (i = 0; i < NPATHS; i++) {
		CHECK(git_fs_path_join_unrooted(&b, g_paths[i], "/base/dir", &root));
		CHECK(git_fs_path_to_dir(&b));
		n += b.size + (size_t)root;
	}
	g_sink += n;
	git_str_dispose(&b);
}

static void w_path_resolve(void)
{
	git_str b = GIT_STR_INIT;
	size_t i, n = 0;
	for (i = 0; i < NPATHS / 2; i++) {
		git_str_sets(&b, g_paths[i]);
		if (git_fs_path_resolve_relative(&b, 0) == 0)
			n += b.size;
	}
	git_error_clear();
	g_sink += n;
	git_str_dispose(&b);
}

static void w_path_valid(void)
{
	size_t i, n = 0;
	unsigned flags = GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS | GIT_FS_PATH_REJECT_DOS_PATHS |
		GIT_FS_PATH_REJECT_NT_CHARS | GIT_FS_PATH_REJECT_TRAILING_DOT;
	for (i = 0; i < NPATHS; i++)
		n += git_fs_path_is_valid(g_paths[i], flags);
	g_sink += n;
}

static void w_path_prettify(void)
{
	git_str b = GIT_STR_INIT;
	size_t i;
	char *d;
	git_vector_foreach(&g_dirs, i, d) {
		CHECK(git_fs_path_prettify(&b, d, NULL));
		g_sink += b.size;
	}
	git_str_dispose(&b);
}

static size_t diriter_walk(const char *path)
{
	git_fs_path_diriter it = GIT_FS_PATH_DIRITER_INIT;
	git_vector subdirs = GIT_VECTOR_INIT;
	const char *full;
	size_t len, n = 0, i;
	char *s;
	struct stat st;
	int error;

	CHECK(git_fs_path_diriter_init(&it, path, 0));
	while ((error = git_fs_path_diriter_next(&it)) == 0) {
		CHECK(git_fs_path_diriter_fullpath(&full, &len, &it));
		CHECK(git_fs_path_diriter_stat(&st, &it));
		n++;
		if (S_ISDIR(st.st_mode))
			git_vector_insert(&subdirs, git__strndup(full, len));
	}
	git_fs_path_diriter_free(&it);
	git_vector_foreach(&subdirs, i, s) {
		n += diriter_walk(s);
		git__free(s);
	}
	git_vector_dispose(&subdirs);
	return n;
}

static void w_diriter_tree(void)
{
	g_sink += diriter_walk(g_tree);
}

static int count_cb(void *payload, git_str *path)
{
	size_t *n = payload;
	struct stat st;
	(*n)++;
	if (p_lstat(path->ptr, &st) == 0 && S_ISDIR(st.st_mode))
		return git_fs_path_direach(path, 0, count_cb, payload);
	return 0;
}

static void w_direach_tree(void)
{
	git_str p = GIT_STR_INIT;
	size_t n = 0;
	git_str_sets(&p, g_tree);
	CHECK(git_fs_path_direach(&p, 0, count_cb, &n));
	g_sink += n;
	git_str_dispose(&p);
}

static void w_readbuffer(void)
{
	git_str b = GIT_STR_INIT;
	size_t i, n = 0, max = g_files.length < 4000 ? g_files.length : 4000;
	for (i = 0; i < max; i++) {
		CHECK(git_futils_readbuffer(&b, g_files.contents[i]));
		n += b.size;
	}
	g_sink += n;
	git_str_dispose(&b);
}

static void w_filebuf_commit(void)
{
	char chunk[1024];
	size_t i, j;
	unsigned char hash[GIT_HASH_SHA1_SIZE];
	memset(chunk, 'x', sizeof(chunk));
	for (i = 0; i < 200; i++) {
		git_filebuf fb = GIT_FILEBUF_INIT;
		char name[64];
		snprintf(name, sizeof(name), "fs_bench.tmp/f%zu", i % 20);
		CHECK(git_filebuf_open(&fb, name, GIT_FILEBUF_HASH_SHA1, 0644));
		for (j = 0; j < 48; j++)
			CHECK(git_filebuf_write(&fb, chunk, sizeof(chunk)));
		for (j = 0; j < 400; j++)
			CHECK(git_filebuf_printf(&fb, "%016zx refs/heads/branch-%zu\n", j * 7919, j));
		CHECK(git_filebuf_hash(hash, &fb));
		CHECK(git_filebuf_commit(&fb));
		g_sink += hash[0];
	}
}

static void w_mkdir_rmdir(void)
{
	size_t i;
	char p[128];
	for (i = 0; i < 2000; i++) {
		snprintf(p, sizeof(p), "fs_bench.tmp/m/%zu/%zu/%zu/leaf", i % 7, i % 31, i);
		CHECK(git_futils_mkdir(p, 0755, GIT_MKDIR_PATH));
	}
	CHECK(git_futils_rmdir_r("fs_bench.tmp/m", NULL, GIT_RMDIR_REMOVE_FILES));
}

struct workload { const char *name; void (*fn)(void); };
static const struct workload g_workloads[] = {
	{ "path_dirname", w_path_dirname },
	{ "path_join", w_path_join },
	{ "path_resolve", w_path_resolve },
	{ "path_valid", w_path_valid },
	{ "path_prettify", w_path_prettify },
	{ "diriter_tree", w_diriter_tree },
	{ "direach_tree", w_direach_tree },
	{ "readbuffer", w_readbuffer },
	{ "filebuf_commit", w_filebuf_commit },
	{ "mkdir_rmdir", w_mkdir_rmdir },
};

int main(int argc, char **argv)
{
	const char *filter = argc > 2 ? argv[2] : NULL;
	size_t w;
	if (argc > 1)
		g_iters = atoi(argv[1]);
	g_tree = getenv("BENCH_TREE") ? getenv("BENCH_TREE") : "/work/bench-data/libgit2-wd";
	git_libgit2_init();
	git_futils_rmdir_r("fs_bench.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	CHECK(git_futils_mkdir("fs_bench.tmp", 0755, 0));
	setup_paths();
	setup_tree();
	for (w = 0; w < sizeof(g_workloads) / sizeof(g_workloads[0]); w++) {
		double *t;
		int i;
		if (filter && !strstr(g_workloads[w].name, filter))
			continue;
		t = malloc(sizeof(double) * (size_t)g_iters);
		g_workloads[w].fn(); /* warm-up */
		for (i = 0; i < g_iters; i++) {
			double s = now_ms();
			g_workloads[w].fn();
			t[i] = now_ms() - s;
		}
		qsort(t, (size_t)g_iters, sizeof(double), cmp_double);
		printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f\n", g_workloads[w].name,
			t[g_iters / 2], t[0], t[g_iters - 1]);
		free(t);
	}
	git_futils_rmdir_r("fs_bench.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	fprintf(stderr, "sink %zu files %zu dirs %zu\n", g_sink, g_files.length, g_dirs.length);
	git_libgit2_shutdown();
	return 0;
}
