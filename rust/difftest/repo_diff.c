/*
 * Differential test of repository.c (WP l-repo): opening, discovery and
 * ownership validation, run on the same scratch trees against the C
 * reference and against the Rust port.  Every line printed must be the
 * same.  difftest-link: libgit2_tests
 *
 * - safe.directory values (exact, `/ *` (slash-star) prefixes, `%(prefix)//`, `*`, empty
 *   resets, trailing slashes, the root) with the owner mocked as "other";
 * - ceiling directory lists (empty items, trailing `:`, relative entries,
 *   a symlinked ceiling) for discovery;
 * - `.git` files (relative, absolute, whitespace, malformed, missing);
 * - extensions and format versions in the repository's config;
 * - init flags and modes (the files and config written).
 *
 * Paths are printed with the scratch root replaced by "<T>".
 *
 *   RUST_BUILD=/work/builds/l-repo rust/difftest/run.sh rust/difftest/repo_diff.c
 */
#include "common.h"
#include "git2.h"
#include "git2/sys/repository.h"
#include "fs_path.h"
#include "futils.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

extern void git_fs_path__set_owner(unsigned int owner);
#define OWNER_OTHER (1 << 4)

static char root[4096];

static void show(const char *s)
{
	size_t n = strlen(root);
	while (*s) {
		if (n && !strncmp(s, root, n)) {
			fputs("<T>", stdout);
			s += n;
		} else {
			putchar(*s++);
		}
	}
}

static void result(const char *what, int e)
{
	const git_error *err = git_error_last();
	printf("%s: %d", what, e);
	if (e < 0) {
		printf(" class=%d '", err ? err->klass : -1);
		show(err ? err->message : "");
		putchar('\'');
	}
	putchar('\n');
	git_error_clear();
}

static void path(char *out, const char *rel)
{
	snprintf(out, 4096, "%s/%s", root, rel);
}

static void write_file(const char *rel, const char *content)
{
	char p[4096];
	FILE *f;
	path(p, rel);
	f = fopen(p, "w");
	if (!f) { perror(p); exit(1); }
	fputs(content, f);
	fclose(f);
}

static void mkdirs(const char *rel)
{
	char p[4096];
	path(p, rel);
	if (git_futils_mkdir(p, 0777, GIT_MKDIR_PATH) < 0) { perror(p); exit(1); }
}

static void init_repo(const char *rel, int bare)
{
	char p[4096];
	git_repository *r;
	path(p, rel);
	if (git_repository_init(&r, p, bare) < 0) {
		fprintf(stderr, "init %s: %s\n", p, git_error_last()->message);
		exit(1);
	}
	git_repository_free(r);
}

static void try_open(const char *label, const char *rel, unsigned flags, const char *ceil)
{
	char p[4096];
	git_repository *r = NULL;
	int e;
	path(p, rel);
	e = git_repository_open_ext(&r, p, flags, ceil);
	printf("[%s] ", label);
	result("open", e);
	if (!e) {
		printf("  path=");
		show(git_repository_path(r));
		printf(" workdir=");
		show(git_repository_workdir(r) ? git_repository_workdir(r) : "(null)");
		printf(" bare=%d worktree=%d oid=%d\n", git_repository_is_bare(r),
		       git_repository_is_worktree(r), (int)git_repository_oid_type(r));
		git_repository_free(r);
	}
}

static void try_discover(const char *label, const char *rel, int across, const char *ceil)
{
	char p[4096];
	git_buf b = GIT_BUF_INIT;
	int e;
	path(p, rel);
	e = git_repository_discover(&b, p, across, ceil);
	printf("[%s] ", label);
	result("discover", e);
	if (!e) {
		printf("  found=");
		show(b.ptr);
		putchar('\n');
	}
	git_buf_dispose(&b);
}

static void safe_directory(void)
{
	char home[4096], repo[4096], val[8192];
	const char *values[] = {
		"*", "", "<R>", "<R>/", "<R>/.git", "<P>/*", "<P>/", "<P>/x/*",
		"%(prefix)/<R>", "%(prefix)//<R>", "/", "/*", "relative/path",
		NULL
	};
	const char *seqs[][3] = {
		{ "*", "", NULL },
		{ "", "*", NULL },
		{ "<R>", "", NULL },
		{ "/nope", "<R>", NULL },
	};
	size_t i, j;

	path(home, "home");
	mkdirs("home");
	git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_GLOBAL, home);
	git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_SYSTEM, "/nonexistent-system");
	git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_XDG, "/nonexistent-xdg");
	init_repo("own/repo", 0);
	init_repo("own/bare.git", 1);
	path(repo, "own/repo");

	for (j = 0; j < 2; j++) {
		const char *target = j ? "own/bare.git" : "own/repo";
		for (i = 0; values[i]; i++) {
			char *o = val;
			const char *v = values[i];
			char rp[4096], pp[4096];
			path(rp, target);
			path(pp, "own");
			o += sprintf(o, "[safe]\n\tdirectory = ");
			while (*v) {
				if (!strncmp(v, "<R>", 3)) { o += sprintf(o, "%s", rp + 0); v += 3; }
				else if (!strncmp(v, "<P>", 3)) { o += sprintf(o, "%s", pp); v += 3; }
				else *o++ = *v++;
			}
			sprintf(o, "\n");
			write_file("home/.gitconfig", val);
			git_fs_path__set_owner(OWNER_OTHER);
			printf("safe.directory=%s ", values[i]);
			try_open(target, target, GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
			git_fs_path__set_owner(0);
		}
	}
	for (i = 0; i < sizeof(seqs) / sizeof(seqs[0]); i++) {
		char *o = val;
		o += sprintf(o, "[safe]\n");
		for (j = 0; j < 3 && seqs[i][j]; j++) {
			if (!strcmp(seqs[i][j], "<R>"))
				o += sprintf(o, "\tdirectory = %s\n", repo);
			else
				o += sprintf(o, "\tdirectory = %s\n", seqs[i][j]);
		}
		write_file("home/.gitconfig", val);
		git_fs_path__set_owner(OWNER_OTHER);
		printf("seq %zu ", i);
		try_open("seq", "own/repo", GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
		git_fs_path__set_owner(0);
	}
	write_file("home/.gitconfig", "");
	/* Owned by us: always fine; with validation off: fine either way. */
	try_open("mine", "own/repo", 0, NULL);
	git_libgit2_opts(GIT_OPT_SET_OWNER_VALIDATION, 0);
	git_fs_path__set_owner(OWNER_OTHER);
	try_open("novalidate", "own/repo", 0, NULL);
	git_fs_path__set_owner(0);
	git_libgit2_opts(GIT_OPT_SET_OWNER_VALIDATION, 1);
}

static void ceilings(void)
{
	char a[4096], b[4096], c[8192], link[4096];
	const char *rels[] = { "", ":", "::", "relative", "relative:" };
	size_t i;

	init_repo("ceil/r", 0);
	mkdirs("ceil/r/a/b/c");
	path(a, "ceil/r/a");
	path(b, "ceil/r/a/b");
	path(link, "ceil/link");
	unlink(link);
	if (symlink(a, link) < 0) { perror("symlink"); exit(1); }

	try_discover("none", "ceil/r/a/b/c", 0, NULL);
	for (i = 0; i < sizeof(rels) / sizeof(rels[0]); i++) {
		printf("ceil='%s' ", rels[i]);
		try_discover("rel", "ceil/r/a/b/c", 0, rels[i]);
	}
	try_discover("a", "ceil/r/a/b/c", 0, a);
	snprintf(c, sizeof(c), "%s/", a);
	try_discover("a/", "ceil/r/a/b/c", 0, c);
	snprintf(c, sizeof(c), ":%s:", b);
	try_discover(":b:", "ceil/r/a/b/c", 0, c);
	snprintf(c, sizeof(c), "relative:%s:/nonexistent", a);
	try_discover("mixed", "ceil/r/a/b/c", 0, c);
	try_discover("symlink", "ceil/r/a/b/c", 0, link);
	snprintf(c, sizeof(c), "%s/b", link);
	try_discover("symlink/b", "ceil/r/a/b/c", 0, c);
	try_discover("at-root", "ceil/r", 0, b);
	try_discover("in-dotgit", "ceil/r/.git/refs", 0, NULL);
	try_open("nosearch", "ceil/r/a", GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
	try_open("search", "ceil/r/a/b", 0, NULL);
	try_open("search-ceil", "ceil/r/a/b", 0, b);
	try_open("bare-flag", "ceil/r/.git", GIT_REPOSITORY_OPEN_BARE, NULL);
	try_open("no-dotgit", "ceil/r", GIT_REPOSITORY_OPEN_NO_DOTGIT | GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
}

static void gitfiles(void)
{
	char real[4096], content[8192];
	size_t i;
	const char *cases[] = {
		"gitdir: ../real.git\n",
		"gitdir:../real.git",
		"gitdir:   \t../real.git  \n\n",
		"gitdir: <ABS>\n",
		"gitdir:",
		"gitdir: \n",
		"gitdir: nonexistent\n",
		"not a gitfile\n",
		"",
		"GITDIR: ../real.git\n",
	};

	init_repo("gf/real.git", 1);
	path(real, "gf/real.git");
	for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		char dir[64], file[96];
		const char *c = cases[i];
		snprintf(dir, sizeof(dir), "gf/w%zu", i);
		snprintf(file, sizeof(file), "%s/.git", dir);
		mkdirs(dir);
		if (!strncmp(c, "gitdir: <ABS>", 13))
			snprintf(content, sizeof(content), "gitdir: %s\n", real);
		else
			snprintf(content, sizeof(content), "%s", c);
		write_file(file, content);
		printf("gitfile %zu ", i);
		try_open("gitfile", dir, 0, NULL);
	}
}

static void formats(void)
{
	const char *cfgs[] = {
		"[core]\n\trepositoryformatversion = 0\n[extensions]\n\tfoo = bar\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tnoop = true\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tfoo = bar\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha512\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = files\n",
		"[core]\n\trepositoryformatversion = 1\n[extensions]\n\trefstorage = other\n",
		"[core]\n\trepositoryformatversion = 2\n",
		"[core]\n\trepositoryformatversion = -1\n",
		"[core]\n\trepositoryformatversion = x\n",
		"[core]\n\tbare = true\n",
		"[core]\n\tbare = false\n\tworktree = ../elsewhere\n",
		"[core]\n\tworktree = \n",
		"",
	};
	size_t i;
	init_repo("fmt/r", 0);
	mkdirs("fmt/elsewhere");
	for (i = 0; i < sizeof(cfgs) / sizeof(cfgs[0]); i++) {
		write_file("fmt/r/.git/config", cfgs[i]);
		printf("format %zu ", i);
		try_open("fmt", "fmt/r", GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
	}
	{
		const char *exts[] = { "foo", "!noop" };
		git_libgit2_opts(GIT_OPT_SET_EXTENSIONS, exts, (size_t)2);
		write_file("fmt/r/.git/config", cfgs[2]);
		try_open("ext-foo", "fmt/r", GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
		write_file("fmt/r/.git/config", cfgs[1]);
		try_open("ext-!noop", "fmt/r", GIT_REPOSITORY_OPEN_NO_SEARCH, NULL);
		git_libgit2_opts(GIT_OPT_SET_EXTENSIONS, NULL, (size_t)0);
	}
}

static void dump(const char *rel)
{
	char p[4096];
	git_str s = GIT_STR_INIT;
	path(p, rel);
	if (git_futils_readbuffer(&s, p) < 0) {
		printf("  %s: (missing)\n", rel);
		git_error_clear();
	} else {
		printf("  %s: ", rel);
		show(s.ptr);
		putchar('\n');
	}
	git_str_dispose(&s);
}

static void inits(void)
{
	struct { const char *path; uint32_t flags; uint32_t mode; const char *wd; const char *head; } cases[] = {
		{ "in/a", GIT_REPOSITORY_INIT_MKPATH, 0, NULL, NULL },
		{ "in/b.git", GIT_REPOSITORY_INIT_MKPATH | GIT_REPOSITORY_INIT_BARE, GIT_REPOSITORY_INIT_SHARED_GROUP, NULL, "trunk" },
		{ "in/c/.git", GIT_REPOSITORY_INIT_MKPATH, GIT_REPOSITORY_INIT_SHARED_ALL, NULL, "refs/heads/x" },
		{ "in/d.git", GIT_REPOSITORY_INIT_MKPATH, 0, "../dwork", NULL },
		{ "in/e.git", GIT_REPOSITORY_INIT_MKPATH | GIT_REPOSITORY_INIT_RELATIVE_GITLINK, 0, "../ework", NULL },
		{ "in/f", 0, 0, NULL, NULL },
		{ "in/f2/g", GIT_REPOSITORY_INIT_MKDIR, 0, NULL, NULL },
		{ "in/notdotgit", GIT_REPOSITORY_INIT_MKPATH | 0, 0, NULL, NULL },
	};
	size_t i;
	for (i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		git_repository_init_options o = GIT_REPOSITORY_INIT_OPTIONS_INIT;
		git_repository *r = NULL;
		char p[4096], d[4096];
		int e;
		o.flags = cases[i].flags;
		o.mode = cases[i].mode;
		o.workdir_path = cases[i].wd;
		o.initial_head = cases[i].head;
		path(p, cases[i].path);
		e = git_repository_init_ext(&r, p, &o);
		printf("init %s flags=%#x ", cases[i].path, (unsigned)o.flags);
		result("init", e);
		if (r) {
			printf("  path=");
			show(git_repository_path(r));
			printf(" workdir=");
			show(git_repository_workdir(r) ? git_repository_workdir(r) : "(null)");
			printf(" empty=%d unborn=%d\n", git_repository_is_empty(r), git_repository_head_unborn(r));
			snprintf(d, sizeof(d), "%s", git_repository_path(r) + strlen(root) + 1);
			git_repository_free(r);
			{
				char f[4200];
				snprintf(f, sizeof(f), "%sHEAD", d); dump(f);
				snprintf(f, sizeof(f), "%sconfig", d); dump(f);
				snprintf(f, sizeof(f), "%sdescription", d); dump(f);
			}
		}
	}
	/* re-init without and with NO_REINIT */
	{
		git_repository_init_options o = GIT_REPOSITORY_INIT_OPTIONS_INIT;
		git_repository *r = NULL;
		char p[4096];
		path(p, "in/a");
		o.flags = GIT_REPOSITORY_INIT_MKPATH;
		result("reinit", git_repository_init_ext(&r, p, &o));
		git_repository_free(r);
		r = NULL;
		o.flags = GIT_REPOSITORY_INIT_NO_REINIT;
		result("reinit-no", git_repository_init_ext(&r, p, &o));
		git_repository_free(r);
	}
	dump("dwork/.git");
	dump("ework/.git");
}

int main(void)
{
	char tmp[4096];
	git_libgit2_init();
	if (!getcwd(tmp, sizeof(tmp))) return 1;
	snprintf(root, sizeof(root), "%s/rust/difftest/out/repo_diff.tmp", tmp);
	git_futils_rmdir_r(root, NULL, GIT_RMDIR_REMOVE_FILES);
	if (git_futils_mkdir(root, 0777, GIT_MKDIR_PATH) < 0) return 1;
	{
		char real[4096];
		if (!realpath(root, real)) return 1;
		snprintf(root, sizeof(root), "%s", real);
	}
	safe_directory();
	ceilings();
	gitfiles();
	formats();
	inits();
	git_futils_rmdir_r(root, NULL, GIT_RMDIR_REMOVE_FILES);
	git_libgit2_shutdown();
	return 0;
}
