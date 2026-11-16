#include "ic.h"
#include "tm.h"
#include <git2/sys/mempack.h>
#include <git2/sys/odb_backend.h>
int main(void) {
	git_odb *odb; git_odb_backend *mp; git_oid id;
	size_t N = 64 << 20; char *big = malloc(N);
	git_libgit2_init();
	for (size_t i = 0; i < N; i++) big[i] = (char)(i * 7 + i / 13);
	CK(git_odb_new(&odb)); CK(git_mempack_new(&mp)); CK(git_odb_add_backend(odb, mp, 1000));
	double best = 1e30, bh = 1e30;
	for (int r = 0; r < 7; r++) {
		double t0 = cpu_now(); CK(git_odb_write(&id, odb, big, N, GIT_OBJECT_BLOB)); double d = cpu_now() - t0; if (d < best) best = d;
		CK(git_mempack_reset(mp));

	}
	printf("write 64m %.1f ms  hash 64m %.1f ms\n", best / 1e6, bh / 1e6);
	return 0;
}
