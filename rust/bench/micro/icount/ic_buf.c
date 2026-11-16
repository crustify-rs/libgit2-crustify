#include "ic.h"
int main(int argc, char **argv) {
	int n = argc > 1 ? atoi(argv[1]) : 100;
	git_repository *repo; git_oid id; git_commit *c; git_buf b = GIT_BUF_INIT;
	git_libgit2_init();
	CK(git_repository_open(&repo, "/work/bench-data/git.git"));
	CK(git_oid_from_string(&id, "777489f9e09c8d0dd6b12f9d90de6376330577a2", GIT_OID_SHA1));
	if (git_commit_lookup(&c, repo, &id) < 0) { git_reference *r; CK(git_repository_head(&r, repo)); CK(git_commit_lookup(&c, repo, git_reference_target(r))); }
	/* warm */
	CK(git_commit_header_field(&b, c, "committer")); git_buf_dispose(&b);
	MARK();
	for (int i = 0; i < n; i++) { git_buf x = GIT_BUF_INIT; CK(git_commit_header_field(&x, c, i & 1 ? "author" : "committer")); git_buf_dispose(&x); }
	MARK();
	MARK();
	for (int i = 0; i < n; i++) { CK(git_commit_header_field(&b, c, i & 1 ? "author" : "committer")); }
	MARK();
	MARK();
	for (int i = 0; i < n; i++) { git_buf x = GIT_BUF_INIT; int e = git_commit_header_field(&x, c, "parent"); if (e != GIT_ENOTFOUND) CK(e); git_buf_dispose(&x); }
	MARK();
	return 0;
}
