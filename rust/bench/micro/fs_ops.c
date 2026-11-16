/*
 * Per-call timings of the fs_path functions (ns per call, best of 5 runs of
 * N calls), C reference vs Rust port: BENCH=fs_ops build.sh, then run
 * out/fs_ops_c and out/fs_ops_rust.
 */
#include "git2_util.h"
#include "git2/global.h"
#include "fs_path.h"
#include <stdio.h>
#include <string.h>
#include <time.h>

static double now_ns(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (double)ts.tv_sec * 1e9 + (double)ts.tv_nsec;
}

static volatile size_t sink;
#define N 2000000
#define TIME(name, body) do { \
	double best = 1e30; int rr_; \
	for (rr_ = 0; rr_ < 5; rr_++) { size_t i; double s = now_ns(); \
		for (i = 0; i < N; i++) { body; } \
		s = (now_ns() - s) / N; if (s < best) best = s; } \
	printf("%-28s %7.1f ns\n", name, best); } while (0)

int main(void)
{
	git_str b = GIT_STR_INIT;
	const char *p = "/usr/lib/libgit2/src/util/fs_path.c";
	const char *r = "src/./util/../libgit2/./fs_path.c";
	ssize_t root;
	git_libgit2_init();
	TIME("dirname_r", sink += git_fs_path_dirname_r(&b, p));
	TIME("basename_r", sink += git_fs_path_basename_r(&b, p));
	TIME("dirname_r(NULL)", sink += git_fs_path_dirname_r(NULL, p));
	TIME("git_str_sets", sink += git_str_sets(&b, p));
	TIME("root", sink += git_fs_path_root(p));
	TIME("join_unrooted", { git_fs_path_join_unrooted(&b, "a/b/c", "/base/dir", &root); sink += b.size; });
	TIME("to_dir", { git_str_truncate(&b, 3); git_fs_path_to_dir(&b); sink += b.size; });
	TIME("resolve_relative", { git_str_sets(&b, r); git_fs_path_resolve_relative(&b, 0); sink += b.size; });
	TIME("is_valid", sink += git_fs_path_is_valid(p, 0x1ff));
	TIME("cmp", sink += git_fs_path_cmp(p, 20, 0, r, 20, 1, strncmp));
	TIME("common_dirlen", sink += git_fs_path_common_dirlen(p, "/usr/lib/libgit2/src/x"));
	git_str_dispose(&b);
	git_libgit2_shutdown();
	return (int)(sink & 1);
}
