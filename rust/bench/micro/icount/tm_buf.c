#include "ic.h"
#include "tm.h"
int main(int argc, char **argv) {
	git_repository *repo; git_commit *c; git_buf b = GIT_BUF_INIT; git_reference *r;
	git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git"));
	CK(git_repository_head(&r, repo)); CK(git_commit_lookup(&c, repo, git_reference_target(r)));
	TIME_MIN("header_field fresh", 15, 200000, { git_buf x = GIT_BUF_INIT; CK(git_commit_header_field(&x, c, _i & 1 ? "author" : "committer")); git_buf_dispose(&x); });
	TIME_MIN("header_field reuse", 15, 200000, { CK(git_commit_header_field(&b, c, _i & 1 ? "author" : "committer")); });
	return 0;
}
