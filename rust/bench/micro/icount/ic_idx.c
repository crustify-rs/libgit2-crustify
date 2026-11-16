#include "ic.h"
#include "tm.h"
#include <unistd.h>
int main(int argc, char **argv) {
	int timing = argc > 1;
	git_index *idx; char cmd[256];
	git_libgit2_init();
	snprintf(cmd, sizeof cmd, "cp /work/bench-data/git-wd/.git/index /tmp/perfwp/idx_%d", getpid()); system(cmd);
	snprintf(cmd, sizeof cmd, "/tmp/perfwp/idx_%d", getpid());
	CK(git_index_open(&idx, cmd));
	if (timing) {
		TIME_MIN("index write", 9, 20, { CK(git_index_write(idx)); });
		unlink(cmd); return 0;
	}
	CK(git_index_write(idx));
	MARK(); CK(git_index_write(idx)); MARK();
	unlink(cmd);
	return 0;
}
