/* Review l-obj r1: git_signature_default / git_signature_default_from_env, C vs Rust.
 * Run with HOME and XDG_CONFIG_HOME pointing at an empty dir; argv[1] = scratch repo path.
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2/global.h"
#include "git2/signature.h"
#include "git2/repository.h"
#include "git2/config.h"
#include <inttypes.h>
#include <time.h>

static void qs(const char *s)
{
	if (!s) { printf("NULL"); return; }
	putchar('"');
	for (; *s; s++) {
		unsigned char c = (unsigned char)*s;
		if (c >= 32 && c < 127 && c != '\\' && c != '"') putchar(c); else printf("\\x%02x", c);
	}
	putchar('"');
}
static void err(void) { const git_error *e = git_error_last(); printf(" err=%d:", e->klass); qs(e->message); printf("\n"); }
static void sig(const char *what, git_signature *s, int show_time)
{
	printf(" %s=", what);
	if (s == NULL) { printf("NULL"); return; }
	if (s == (git_signature *)0x1) { printf("untouched"); return; }
	qs(s->name); printf("/"); qs(s->email);
	if (show_time) printf(" t=%" PRId64, (int64_t)s->when.time);
	else printf(" tnow=%d", (long long)(time(NULL) - s->when.time) < 3);
	printf(" off=%d sign=%d", s->when.offset, s->when.sign);
}

static void cfg(git_repository *repo, const char *name, const char *email)
{
	git_config *c;
	git_repository_config(&c, repo);
	if (name) git_config_set_string(c, "user.name", name); else git_config_delete_entry(c, "user.name");
	if (email) git_config_set_string(c, "user.email", email); else git_config_delete_entry(c, "user.email");
	git_config_free(c);
}

static void env(const char *k, const char *v) { if (v) setenv(k, v, 1); else unsetenv(k); }

static void run(git_repository *repo, const char *label, int which)
{
	git_signature *a = (git_signature *)0x1, *c = (git_signature *)0x1;
	int rc;
	git_error_set_str(GIT_ERROR_NET, "sentinel");
	rc = git_signature_default_from_env(which & 1 ? &a : NULL, which & 2 ? &c : NULL, repo);
	printf("env %s which=%d rc=%d", label, which, rc);
	sig("a", a, getenv("GIT_AUTHOR_DATE") != NULL);
	sig("c", c, getenv("GIT_COMMITTER_DATE") != NULL);
	err();
	if (rc == 0) { if (which & 1) git_signature_free(a); if (which & 2) git_signature_free(c); }
}

int main(int argc, char **argv)
{
	git_repository *repo;
	git_signature *s;
	int rc, i, j;
	static const char *names[] = { NULL, "Cfg Name", " <x> ", "  ", "\xff" };
	static const char *emails[] = { NULL, "cfg@x", "" };
	static const char *dates[] = { NULL, "1234567890 +0200", "@1234567890 -0500", "1234567890", "2005-04-07T22:13:13+0200",
		"Thu, 07 Apr 2005 22:13:13 -0130", "2005-04-07 22:13:13", "garbage", "", "1234567890 +9999", "1234567890 +1460",
		"99999999999999999999 +0100", "-1 +0100", "@-5 +0000", "1112911993 +0100 x" };
	(void)argc;
	setenv("TZ", "UTC", 1); tzset();
	git_libgit2_init();
	if (git_repository_init(&repo, argv[1], 0) < 0) { printf("init failed\n"); return 1; }

	for (i = 0; i < 5; i++)
		for (j = 0; j < 3; j++) {
			cfg(repo, names[i], emails[j]);
			s = (git_signature *)0x1;
			git_error_set_str(GIT_ERROR_NET, "sentinel");
			rc = git_signature_default(&s, repo);
			printf("default %d %d rc=%d", i, j, rc); sig("s", s, 0); err();
			if (!rc) git_signature_free(s);
		}

	unsetenv("EMAIL");
	for (i = 0; i < 5; i++)
		for (j = 0; j < 3; j++) {
			char l[32];
			cfg(repo, names[i], emails[j]);
			snprintf(l, sizeof(l), "cfg%d%d", i, j);
			run(repo, l, 3);
			env("EMAIL", "envmail@x");
			snprintf(l, sizeof(l), "cfg%d%d+EMAIL", i, j);
			run(repo, l, 3);
			run(repo, l, 1);
			run(repo, l, 2);
			env("EMAIL", NULL);
		}
	cfg(repo, "Cfg Name", "cfg@x");
	for (i = 0; i < (int)ARRAY_SIZE(dates); i++) {
		if (dates[i] && !strcmp(dates[i], "2005-04-07 22:13:13")) continue; /* approxidate path: C offset uninit (documented) */
		if (dates[i] && !strcmp(dates[i], "garbage")) { /* approxidate: also C-uninit offset; print rc only */ }
		env("GIT_AUTHOR_DATE", dates[i]);
		env("GIT_COMMITTER_DATE", dates[(i + 3) % ARRAY_SIZE(dates)]);
		if (dates[(i + 3) % ARRAY_SIZE(dates)] && !strcmp(dates[(i + 3) % ARRAY_SIZE(dates)], "2005-04-07 22:13:13")) env("GIT_COMMITTER_DATE", NULL);
		run(repo, dates[i] ? dates[i] : "(none)", 3);
	}
	env("GIT_AUTHOR_DATE", NULL); env("GIT_COMMITTER_DATE", NULL);
	{
		static const char *envnames[] = { "Env Name", "", " ", "<x>", "a>b", "\t n \t" };
		for (i = 0; i < (int)ARRAY_SIZE(envnames); i++) {
			env("GIT_AUTHOR_NAME", envnames[i]);
			env("GIT_COMMITTER_EMAIL", envnames[(i + 1) % ARRAY_SIZE(envnames)]);
			run(repo, envnames[i], 3);
			run(repo, envnames[i], 1);
			run(repo, envnames[i], 2);
		}
	}
	env("GIT_AUTHOR_NAME", NULL); env("GIT_COMMITTER_EMAIL", NULL);
	git_error_set_str(GIT_ERROR_NET, "sentinel");
	printf("env NULL outs rc=%d", git_signature_default_from_env(NULL, NULL, repo)); err();
	git_error_set_str(GIT_ERROR_NET, "sentinel");
	s = (git_signature *)0x1;
	printf("env NULL repo rc=%d", git_signature_default_from_env(&s, NULL, NULL)); err();
	git_repository_free(repo);
	git_libgit2_shutdown();
	return 0;
}
