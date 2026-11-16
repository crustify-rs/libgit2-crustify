#include "ic.h"
#include "tm.h"
#define CN 5000
int main(void) {
	git_repository *repo; git_revwalk *w; git_oid id; static git_commit *c[CN]; git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git")); CK(git_revwalk_new(&w, repo)); CK(git_revwalk_push_head(w));
	for (int i = 0; i < CN; i++) { CK(git_revwalk_next(&id, w)); CK(git_commit_lookup(&c[i], repo, &id)); }
	static const char *fields[] = { "tree", "author", "committer", "parent" };
	TIME_MIN("bench header_field", 15, 1, { for (int i = 0; i < CN * 4; i++) { git_buf b = GIT_BUF_INIT; int e = git_commit_header_field(&b, c[i % CN], fields[i / CN]); if (e != GIT_ENOTFOUND) CK(e); git_buf_dispose(&b); } });
	git_buf b = GIT_BUF_INIT;
	TIME_MIN("bench reuse", 15, 1, { for (int i = 0; i < CN * 4; i++) CK(git_commit_header_field(&b, c[i % CN], i % 2 ? "author" : "committer")); });
	return 0;
}
