#include "ic.h"
#include "tm.h"
int main(void) {
	git_repository *repo; git_oid id; git_libgit2_init();
	git_reference *h;
	CK(git_repository_open(&repo, "/work/bench-data/refs.git"));
	git_strarray a; CK(git_reference_list(&a, repo));
	const char *names[4] = { "refs/heads/main", a.strings[a.count / 2], a.strings[a.count / 3], "refs/heads/nope" };
	for (int i = 0; i < 4; i++) printf("%s\n", names[i]);
	TIME_MIN("name_to_id mix", 9, 20000, { int e = git_reference_name_to_id(&id, repo, names[_i & 3]); if (e < 0 && e != GIT_ENOTFOUND) CK(e); });
	TIME_MIN("head", 9, 3000, { CK(git_repository_head(&h, repo)); git_reference_free(h); git_repository_head_detached(repo); });
	return 0;
}
