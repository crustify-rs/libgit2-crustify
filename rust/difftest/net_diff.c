/*
 * Differential test of src/util/net.c (git_net_url): every line printed
 * must be the same when linked against the C reference and against the
 * Rust port.  Covers the four parsers on the clar cases, on hand-written
 * malformed inputs and on random strings over a URL-heavy alphabet; the
 * formatting, joinpath, redirects, the no_proxy patterns, certificate host
 * matching and dup/swap.
 *
 * Calls that dereference a NULL field in the C (is_ipv6 without a host,
 * is_default_port without a port, patterns without host/port, redirects to
 * a location without a scheme or host) are only made when the C is defined;
 * the Rust's answers there are covered by its own tests (DEFECTS.md).
 *
 * Build and compare: RUST_BUILD=/work/builds/u4 rust/difftest/run.sh rust/difftest/net_diff.c
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "net.h"
#include "str.h"

static unsigned long seed = 4242;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

#define S(x) ((x) ? (x) : "(NULL)")

static void show_err(int rc)
{
	const git_error *e = git_error_last();
	if (rc < 0)
		printf("  rc=%d class=%d msg=%s\n", rc, e ? e->klass : -1, e ? e->message : "-");
	else
		printf("  rc=%d\n", rc);
}

static void show_url(const git_net_url *u)
{
	printf("  scheme=%s host=%s port=%s path=%s query=%s fragment=%s user=%s pass=%s ps=%u\n",
	       S(u->scheme), S(u->host), S(u->port), S(u->path), S(u->query),
	       S(u->fragment), S(u->username), S(u->password), u->port_specified);
}

static const char *patterns[] = {
	"*", "", "example.com", ".example.com", "*.example.com", "example.com:443",
	"*.example.com:80", ".com", "*.com", "localhost", "localhost:8080",
	"127.0.0.1", "::1", ":80", "*.", ".", "*:443", "EXAMPLE.com", "e", "*.e",
	"foo.example.com", "ample.com", "example.com:", "a,b", "[::1]",
};

static void probe(git_net_url *u)
{
	git_str buf = GIT_STR_INIT;
	size_t i;
	int rc;

	printf("  valid=%d", git_net_url_valid(u));
	if (u->host)
		printf(" ipv6=%d", git_net_url_is_ipv6(u));
	if (u->port || !u->scheme)
		printf(" default_port=%d", git_net_url_is_default_port(u));
	printf("\n");

	if (u->scheme && u->host) {
		rc = git_net_url_fmt(&buf, u);
		printf("  fmt rc=%d [%s]\n", rc, buf.ptr);
		git_str_clear(&buf);
	}
	rc = git_net_url_fmt_path(&buf, u);
	printf("  fmt_path rc=%d [%s]\n", rc, buf.ptr);
	git_str_dispose(&buf);

	if (u->host && u->port) {
		printf("  patterns ");
		for (i = 0; i < ARRAY_SIZE(patterns); i++)
			printf("%d", git_net_url_matches_pattern(u, patterns[i]));
		printf(" list=%d%d%d%d\n",
		       git_net_url_matches_pattern_list(u, "foo,*.example.com,bar"),
		       git_net_url_matches_pattern_list(u, ",,localhost,"),
		       git_net_url_matches_pattern_list(u, ""),
		       git_net_url_matches_pattern_list(u, "nope,.com:443"));
	}

	{
		git_net_url d = GIT_NET_URL_INIT;
		rc = git_net_url_dup(&d, u);
		printf("  dup rc=%d\n", rc);
		show_url(&d);
		git_net_url_dispose(&d);
	}
}

typedef int (*parse_fn)(git_net_url *, const char *);

static void run_parser(const char *name, parse_fn fn, const char *in, int deep)
{
	git_net_url u;
	int rc;

	memset(&u, 0xab, sizeof(u)); /* the parsers must clear it */
	git_error_clear();
	rc = fn(&u, in);
	printf("%s(\"%s\")\n", name, in);
	show_err(rc);
	show_url(&u);
	if (rc == 0 && deep)
		probe(&u);
	git_net_url_dispose(&u);
}

static void parse_all(const char *in, int deep)
{
	run_parser("parse", git_net_url_parse, in, deep);
	run_parser("parse_scp", git_net_url_parse_scp, in, deep);
	run_parser("parse_http", git_net_url_parse_http, in, deep);
	run_parser("standard_or_scp", git_net_url_parse_standard_or_scp, in, deep);
	printf("is_url=%d\n", git_net_str_is_url(in));
}

static const char *cases[] = {
	/* tests/util/url/parse.c, http.c, scp.c */
	"http://example.com/resource", "http://example.com:8080/resource",
	"http://user:pass@example.com:8080/resource?q=1#frag",
	"http://us%65r:pa%73s@example.com/", "https://example.com",
	"https://example.com/", "http://example.com:/", "http://example.com:",
	"http://[fe80::dcad:beff:fe00:0001]/resource",
	"http://[fe80::dcad:beff:fe00:0001]:9090/resource",
	"http://user:pass@[fe80::dcad:beff:fe00:0001]:9090/resource",
	"http://[fe80::dcad:beff:fe00:0001", "http://fe80::dcad:beff:fe00:0001]/",
	"http://[fe80::dcad:beff:fe00:000x]/", "http://[]/", "http://[:]/",
	"git://example.com/repo.git", "ssh://git@example.com/repo.git",
	"ssh+git://example.com/", "git+ssh://example.com/", "SSH://Example.COM/",
	"HTTPS://USER@EXAMPLE.COM/PATH", "ssh://user@name@example.com/",
	"http://user@name@example.com/", "git://user@name@example.com/",
	"g://a@b@c/", "file:///tmp/foo", "file://localhost/tmp/foo", "file:",
	"mailto:user@example.com", "a:b", "a:", ":", "://", ":///", "x://",
	"x:///", "x://@", "x://@/", "x://:@/", "x://u:@h/", "x://:p@h/",
	"x://h:p:q/", "x://h]/", "x://h[/", "x://[h]/", "x://u@[::1]x/",
	"x://u@x[::1]/", "/absolute/path", "relative/path", "relative",
	"C:\\Windows\\path", "c:/foo", "", " ", "?", "#", "http://h?q#f?g#h",
	"http://h/#?", "http://h/?#", "http://h/%", "http://h/?a%20b%2", "http://%41%42/",
	"http://h/p?%zz", "http://h%00/", "http://12345/", "http://12345:678/",
	"http://:80/", "http://@:80/", "localhost", "localhost:8080",
	"localhost:8080/path", "user@localhost", "user:pass@localhost:1/p?q#f",
	"[::1]", "[::1]:8080/x", "example.com/path/to", "a@b:c",
	"git@github.com:libgit2/libgit2.git", "github.com:libgit2/libgit2.git",
	"git@[fe80::dcad:beff:fe00:0001]:repo", "[fe80::dcad:beff:fe00:0001]:repo",
	"[git@github.com:22]:libgit2/libgit2.git", "[github.com:22]:repo",
	"[github.com:42]:x", "[github.com]:x", "[[github.com]]:x", "[[[x]]]:x",
	"git@[github.com:42]:x", "host:", "@host:path", ":path", "host",
	"user@host", "user@host:", "user@host:p", "u@h@i:p", "[::1]:", "[::1]x:p",
	"[a:b:c]:p", "[ab]:p", "h]:p", "[h:p]x", "[h:p]:", "[h:p]:x", "h:p:q",
	"git@github.com:22:repo", "a@:b", "a@::b", "a@[::1]:b", "a@[x]:b", "[:]:x",
	"http://example.com:65536/", "http://-/", "HTTP://H/",
	"https://user:p%40ss@host/", "https://us@er:pass@host/",
	"https://ex%2Fample.com/", "http://h:8080", "http://h:8080?x",
	NULL
};

static void test_parsers(void)
{
	const char **c;
	for (c = cases; *c; c++)
		parse_all(*c, 1);
}

static void test_random(void)
{
	static const char alphabet[] = "abcAB09:/@[]?#%.+-_ \\~";
	char buf[48];
	int i;

	for (i = 0; i < 30000; i++) {
		size_t len = rnd() % 24, j;
		for (j = 0; j < len; j++)
			buf[j] = alphabet[rnd() % (sizeof(alphabet) - 1)];
		buf[len] = '\0';
		/* bias towards well-formed prefixes */
		if (i % 3 == 0 && len < 40) {
			static const char *pfx[] = { "http://", "ssh://", "git@", "[", "https://u:p@", "g://" };
			const char *p = pfx[rnd() % ARRAY_SIZE(pfx)];
			memmove(buf + strlen(p), buf, len + 1);
			memcpy(buf, p, strlen(p));
		}
		parse_all(buf, i % 10 == 0);
	}
}

static void test_hostnames(void)
{
	static const char *hosts[] = { "example.com", "www.example.com", "EXAMPLE.com",
		"a.b.example.com", "example.org", "", "com", "x", "foo.bar", "foo.", ".foo", "www.e.x.a" };
	static const char *pats[] = { "example.com", "*.example.com", "*", "*.com", "www.*.com",
		"w*.example.com", "*w.example.com", "*.*.com", "", "*.", "e*e.com", "EXAMPLE.COM",
		"*xample.com", "x*", "foo*", "*.foo", "*o.bar", "*a" };
	size_t i, j;
	for (i = 0; i < ARRAY_SIZE(hosts); i++) {
		printf("cert %s:", hosts[i]);
		for (j = 0; j < ARRAY_SIZE(pats); j++)
			printf("%d", git_net_hostname_matches_cert(hosts[i], pats[j]));
		printf("\n");
	}
}

/* random hosts and certificate patterns: mixed case, wildcards followed by
 * upper-case or high-bit bytes, dots */
static void test_hostnames_random(void)
{
	static const char alphabet[] = "aAbBzZ.*-x\x80\xc3\xff";
	char host[16], pat[16], line[101];
	int i, n = 0;

	for (i = 0; i < 200000; i++) {
		size_t hl = rnd() % 12, pl = rnd() % 10, j;
		for (j = 0; j < hl; j++)
			host[j] = alphabet[rnd() % (sizeof(alphabet) - 1)];
		host[hl] = '\0';
		for (j = 0; j < pl; j++)
			pat[j] = alphabet[rnd() % (sizeof(alphabet) - 1)];
		pat[pl] = '\0';
		line[n++] = '0' + git_net_hostname_matches_cert(host, pat);
		if (n == 100) {
			line[n] = '\0';
			printf("certrnd %s\n", line);
			n = 0;
		}
	}
}

static void test_joinpath(void)
{
	static const char *bases[] = { "http://example.com/a/b", "http://example.com/a/b/",
		"http://example.com", "http://u:p@example.com:8080/a///?x=1#f", "a:b", "relative" };
	static const char *paths[] = { "/c/d", "c/d", "/c/d?foo", "?", "?q", "", "/", "///",
		"//x//", "/c?", "?a?b", "x?/y" };
	size_t i, j;
	for (i = 0; i < ARRAY_SIZE(bases); i++) {
		git_net_url base = GIT_NET_URL_INIT;
		if (git_net_url_parse(&base, bases[i]) < 0)
			continue;
		for (j = 0; j < ARRAY_SIZE(paths); j++) {
			git_net_url out = GIT_NET_URL_INIT;
			int rc = git_net_url_joinpath(&out, &base, paths[j]);
			printf("joinpath(%s, %s)\n", bases[i], paths[j]);
			show_err(rc);
			show_url(&out);
			git_net_url_dispose(&out);
		}
		git_net_url_dispose(&base);
	}
}

static void test_redirect(void)
{
	static const char *froms[] = { "http://example.com/foo/bar/info/refs",
		"https://example.com/foo/info/refs?service=git-upload-pack",
		"http://example.com/info/refs", "http://user:pass@example.com:8080/x" };
	static const char *locs[] = { "/newloc/info/refs", "/newloc/info/refs?service=git-upload-pack",
		"http://example.com/other/info/refs", "https://example.com/other/info/refs?service=git-upload-pack",
		"https://other.com/info/refs", "HTTP://EXAMPLE.COM/x", "http://Example.com/info/refs",
		"ssh://example.com/x", "git://example.com/x", "http://example.com", "/info/refs",
		"/", "/info/refs?service=git-upload-pack", "http://[::1]/x", "http://x:y@example.com:9/info/refs?a" };
	static const char *suffixes[] = { NULL, "/info/refs", "/info/refs?service=git-upload-pack",
		"?service=git-upload-pack", "", "/refs" };
	size_t i, j, k;
	int off;
	for (i = 0; i < ARRAY_SIZE(froms); i++)
	for (j = 0; j < ARRAY_SIZE(locs); j++)
	for (k = 0; k < ARRAY_SIZE(suffixes); k++)
	for (off = 0; off < 2; off++) {
		git_net_url u = GIT_NET_URL_INIT;
		int rc;
		if (git_net_url_parse(&u, froms[i]) < 0)
			continue;
		/* the C reads a NULL query when the suffix has a query */
		if (suffixes[k] && strchr(suffixes[k], '?') && !strchr(locs[j], '?') && locs[j][0] == '/' && !u.query)
			goto skip;
		if (suffixes[k] && strchr(suffixes[k], '?') && locs[j][0] != '/' && !strchr(locs[j], '?'))
			goto skip;
		git_error_clear();
		rc = git_net_url_apply_redirect(&u, locs[j], off, suffixes[k]);
		printf("redirect(%s, %s, %d, %s)\n", froms[i], locs[j], off, S(suffixes[k]));
		show_err(rc);
		show_url(&u);
	skip:
		git_net_url_dispose(&u);
	}
}

static void test_swap(void)
{
	git_net_url a = GIT_NET_URL_INIT, b = GIT_NET_URL_INIT;
	git_net_url_parse(&a, "http://a.com/x");
	git_net_url_parse(&b, "ssh://u@b.com:22/y");
	git_net_url_swap(&a, &b);
	show_url(&a);
	show_url(&b);
	git_net_url_swap(&a, &a);
	show_url(&a);
	git_net_url_dispose(&a);
	git_net_url_dispose(&b);
}

static void test_errors(void)
{
	git_str buf = GIT_STR_INIT;
	git_net_url u = GIT_NET_URL_INIT;
	int rc;
	rc = git_net_url_fmt(&buf, &u);
	show_err(rc);
	u.scheme = git__strdup("http");
	rc = git_net_url_fmt(&buf, &u);
	show_err(rc);
	rc = git_net_url_fmt(&buf, NULL);
	show_err(rc);
	rc = git_net_url_apply_redirect(NULL, "x", 0, NULL);
	show_err(rc);
	rc = git_net_url_apply_redirect(&u, NULL, 0, NULL);
	show_err(rc);
	git_net_url_dispose(&u);
	git_str_dispose(&buf);
}

int main(void)
{
	git_libgit2_init();
	test_parsers();
	test_hostnames();
	test_hostnames_random();
	test_joinpath();
	test_redirect();
	test_swap();
	test_errors();
	test_random();
	git_libgit2_shutdown();
	return 0;
}
