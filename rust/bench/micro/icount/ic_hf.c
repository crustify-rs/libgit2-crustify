#include "ic.h"
#define CN 50
int main(void) {
	git_repository *repo; git_revwalk *w; git_oid id; static git_commit *c[CN]; git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git")); CK(git_revwalk_new(&w, repo)); CK(git_revwalk_push_head(w));
	for (int i = 0; i < CN; i++) { CK(git_revwalk_next(&id, w)); CK(git_commit_lookup(&c[i], repo, &id)); }
	for (int i = 0; i < 100; i++) { git_buf b = GIT_BUF_INIT; CK(git_commit_header_field(&b, c[i % CN], "tree")); git_buf_dispose(&b); }
	MARK(); for (int i = 0; i < 100; i++) { git_buf b = GIT_BUF_INIT; CK(git_commit_header_field(&b, c[i % CN], "tree")); git_buf_dispose(&b); } MARK();
	return 0;
}
