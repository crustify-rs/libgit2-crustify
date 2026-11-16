#include "ic.h"
#include "tm.h"
#include <unistd.h>
#include <sys/stat.h>
int main(int argc, char **argv) {
	int timing = argc > 1;
	git_repository *repo, *src; git_reference *r; git_oid id; char dir[256], name[64], cmd[512];
	git_libgit2_init(); git_libgit2_opts(GIT_OPT_ENABLE_STRICT_OBJECT_CREATION, 0);
	snprintf(dir, sizeof dir, "/tmp/perfwp/scr_%d", getpid());
	CK(git_repository_open(&src, "/work/bench-data/git.git")); CK(git_reference_name_to_id(&id, src, "refs/heads/master"));
	int N = timing ? 400 : 20;
	ic_marks = !timing;
	for (int rep = 0; rep < (timing ? 5 : 1); rep++) {
	snprintf(cmd, sizeof cmd, "rm -rf %s", dir); system(cmd);
	CK(git_repository_init(&repo, dir, 1));
	double t0 = cpu_now();
	MARK(); for (int i = 0; i < N; i++) { snprintf(name, sizeof name, "refs/heads/topic/t%04d", i); CK(git_reference_create(&r, repo, name, &id, 0, NULL)); git_reference_free(r); } MARK();
	double t1 = cpu_now();
	git_repository_free(repo); CK(git_repository_open(&repo, dir));
	double t2 = cpu_now();
	MARK(); for (int i = 0; i < N; i++) { snprintf(name, sizeof name, "refs/heads/topic/t%04d", i); CK(git_reference_remove(repo, name)); } MARK();
	double t3 = cpu_now();
	git_repository_free(repo);
	snprintf(cmd, sizeof cmd, "rm -rf %s", dir); system(cmd);
	char d2[300]; snprintf(d2, sizeof d2, "%s_init", dir);
	double t4 = cpu_now();
	MARK(); CK(git_repository_init(&repo, d2, 0)); git_repository_free(repo); MARK();
	double t5 = cpu_now();
	snprintf(cmd, sizeof cmd, "rm -rf %s", d2); system(cmd);
	CK(git_repository_init(&repo, dir, 1)); git_odb *odb; CK(git_repository_odb(&odb, repo));
	static char data[4096]; for (int i = 0; i < 4096; i++) data[i] = i * 7;
	double t6 = cpu_now();
	MARK(); for (int i = 0; i < N; i++) { data[0] = i; data[1] = i >> 8; CK(git_odb_write(&id, odb, data, 100 + i % 3000, GIT_OBJECT_BLOB)); } MARK();
	double t7 = cpu_now();
	git_odb_free(odb); git_repository_free(repo);
	snprintf(cmd, sizeof cmd, "rm -rf %s", dir); system(cmd);
	if (timing) printf("create %.1f delete %.1f init %.1f loosewrite %.1f us/op\n", (t1-t0)/N/1e3, (t3-t2)/N/1e3, (t5-t4)/1e3, (t7-t6)/N/1e3);
	}
	return 0;
}
