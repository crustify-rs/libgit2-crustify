#include "ic.h"
#include "tm.h"
int main(void) {
	git_repository *repo; git_revwalk *w; git_oid master, id; git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git")); CK(git_reference_name_to_id(&master, repo, "refs/heads/master"));
	CK(git_revwalk_new(&w, repo));
	CK(git_revwalk_push(w, &master)); while (git_revwalk_next(&id, w) == 0); 
	long n = 0;
	TIME_MIN("first_parent walk", 9, 1, { CK(git_revwalk_sorting(w, GIT_SORT_NONE)); CK(git_revwalk_simplify_first_parent(w)); CK(git_revwalk_add_hide_cb(w, NULL, NULL)); CK(git_revwalk_push(w, &master)); while (git_revwalk_next(&id, w) == 0) n++; });
	TIME_MIN("unsorted walk", 9, 1, { CK(git_revwalk_sorting(w, GIT_SORT_NONE)); CK(git_revwalk_push(w, &master)); while (git_revwalk_next(&id, w) == 0) n++; });
	printf("%ld\n", n);
	return 0;
}
