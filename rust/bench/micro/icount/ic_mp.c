#include "ic.h"
#include "tm.h"
#include <git2/sys/mempack.h>
#include <git2/sys/odb_backend.h>
static char data[70000];
int main(int argc, char **argv) {
	int n = argc > 1 ? atoi(argv[1]) : 2000;
	int timing = argc > 2;
	git_odb *odb; git_odb_backend *mp; git_oid id;
	git_libgit2_init();
	for (size_t i = 0; i < sizeof data; i++) data[i] = (char)(i * 7 + i / 13);
	CK(git_odb_new(&odb)); CK(git_mempack_new(&mp)); CK(git_odb_add_backend(odb, mp, 1000));
	if (timing) {
		double best = 1e30;
		for (int r = 0; r < 9; r++) {
			for (int i = 0; i < 20000; i++) { size_t len = 16 + (i * 37) % 4000; data[0] = (char)i; data[1] = (char)(i >> 8); CK(git_odb_write(&id, odb, data, len, GIT_OBJECT_BLOB)); }
			double t0 = cpu_now(); CK(git_mempack_reset(mp)); double d = cpu_now() - t0; if (d < best) best = d;
		}
		printf("reset 20000: %.1f us\n", best / 1000);
		return 0;
	}
	for (int i = 0; i < n; i++) { size_t len = 16 + (i * 37) % 4000; data[0] = (char)i; data[1] = (char)(i >> 8); CK(git_odb_write(&id, odb, data, len, GIT_OBJECT_BLOB)); }
	MARK(); CK(git_mempack_reset(mp)); MARK();

	return 0;
}
