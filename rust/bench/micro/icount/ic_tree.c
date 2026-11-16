#include "ic.h"
#include "tm.h"
static const char *paths[] = { "Makefile", "builtin/commit.c", "t/t0000-basic.sh", "Documentation/git-commit.adoc", "compat/mingw.c", "contrib/completion/git-completion.bash", "xdiff/xdiffi.c", "README.md" };
int main(int argc, char **argv) {
	int timing = argc > 1;
	git_repository *repo; git_tree *t; git_reference *r; git_commit *c; git_tree_entry *e;
	git_libgit2_init();
	CK(git_libgit2_opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, GIT_OBJECT_TREE, (size_t)1 << 20));
	CK(git_repository_open(&repo, "/work/bench-data/git.git"));
	CK(git_repository_head(&r, repo)); CK(git_commit_lookup(&c, repo, git_reference_target(r))); CK(git_commit_tree(&t, c));
	for (int i = 0; i < 8; i++) { CK(git_tree_entry_bypath(&e, t, paths[i])); git_tree_entry_free(e); }
	if (timing) { TIME_MIN("bypath", 9, 200000, { CK(git_tree_entry_bypath(&e, t, paths[_i & 7])); git_tree_entry_free(e); }); return 0; }
	MARK(); for (int k = 0; k < 10; k++) for (int i = 0; i < 8; i++) { CK(git_tree_entry_bypath(&e, t, paths[i])); git_tree_entry_free(e); } MARK();
	return 0;
}
