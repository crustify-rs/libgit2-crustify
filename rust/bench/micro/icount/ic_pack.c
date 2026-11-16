#include "ic.h"
#include "tm.h"
static git_oid ids[500000]; static size_t nids;
static int cb(const git_oid *id, void *p) { (void)p; if (nids < 500000) ids[nids++] = *id; return 0; }
int main(int argc, char **argv) {
	int n = argc > 1 ? atoi(argv[1]) : 1000;
	int timing = argc > 2;
	git_odb *odb; git_oid miss[200];
	git_libgit2_init();
	CK(git_odb_open(&odb, "/work/bench-data/git.git/objects"));
	CK(git_odb_foreach(odb, cb, NULL));
	git_odb_free(odb);
	/* shuffle deterministically */
	unsigned x = 12345; for (size_t i = nids - 1; i > 0; i--) { x = x * 1103515245u + 12345u; size_t j = x % (i + 1); git_oid t = ids[i]; ids[i] = ids[j]; ids[j] = t; }
	for (int i = 0; i < 200; i++) { unsigned char raw[20]; for (int j = 0; j < 20; j++) { x = x * 1103515245u + 12345u; raw[j] = x >> 24; } git_oid_from_raw(&miss[i], raw, GIT_OID_SHA1); }
	if (timing) {
		TIME_MIN("open+exists 50000 hits", 7, 1, { CK(git_odb_open(&odb, "/work/bench-data/git.git/objects")); for (int k = 0; k < 50000; k++) if (!git_odb_exists(odb, &ids[k])) exit(2); git_odb_free(odb); });
		CK(git_odb_open(&odb, "/work/bench-data/git.git/objects"));
		git_odb_exists(odb, &ids[0]);
		TIME_MIN("exists hit", 7, 200000, { if (!git_odb_exists(odb, &ids[_i])) exit(2); });
		TIME_MIN("exists miss", 7, 2000, { if (git_odb_exists(odb, &miss[_i % 200])) exit(2); });
		return 0;
	}
	CK(git_odb_open(&odb, "/work/bench-data/git.git/objects"));
	MARK(); if (!git_odb_exists(odb, &ids[0])) return 2; MARK();
	MARK(); for (int i = 1; i <= n; i++) if (!git_odb_exists(odb, &ids[i])) return 2; MARK();
	MARK(); for (int i = 0; i < 20; i++) if (git_odb_exists(odb, &miss[i])) return 2; MARK();
	return 0;
}
