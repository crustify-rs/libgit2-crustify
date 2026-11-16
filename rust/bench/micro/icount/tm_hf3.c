#include "ic.h"
#include "tm.h"
#define CN 50
int main(void) {
	git_repository *repo; git_revwalk *w; git_oid id; static git_commit *c[CN]; git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git")); CK(git_revwalk_new(&w, repo)); CK(git_revwalk_push_head(w));
	for (int i = 0; i < CN; i++) { CK(git_revwalk_next(&id, w)); CK(git_commit_lookup(&c[i], repo, &id)); }
	static const char *fields[] = { "tree", "author", "committer", "parent" };
	for (int f = 0; f < 4; f++) {
		char lbl[32]; snprintf(lbl, sizeof lbl, "%s", fields[f]);
		TIME_MIN(lbl, 15, 20000, { git_buf b = GIT_BUF_INIT; int e = git_commit_header_field(&b, c[_i % CN], fields[f]); if (e != GIT_ENOTFOUND) CK(e); git_buf_dispose(&b); });
	}
	return 0;
}
