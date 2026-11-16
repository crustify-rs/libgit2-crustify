/*
 * Differential test of src/util/wildmatch.c, src/util/regexp.c (PCRE2)
 * and src/util/rand.c: wildmatch on git's t3070 cases and on random
 * patterns and texts with every flag combination; regular expressions
 * compiled (messages of the bad ones), matched and searched; the rand
 * sequences of fixed seeds.
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "wildmatch.h"
#include "regexp.h"
#include "rand.h"

static unsigned long seed = 31337;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

/* (text, pattern) from tests/util/wildmatch.c (git's t3070). */
static const char *t3070[][2] = {
	{ "foo", "foo" },
	{ "foo", "bar" },
	{ "", "" },
	{ "foo", "???" },
	{ "foo", "??" },
	{ "foo", "*" },
	{ "foo", "f*" },
	{ "foo", "*f" },
	{ "foo", "*foo*" },
	{ "foobar", "*ob*a*r*" },
	{ "aaaaaaabababab", "*ab" },
	{ "foo*", "foo\\*" },
	{ "foobar", "foo\\*bar" },
	{ "f\\oo", "f\\\\oo" },
	{ "ball", "*[al]?" },
	{ "ten", "[ten]" },
	{ "ten", "**[!te]" },
	{ "ten", "**[!ten]" },
	{ "ten", "t[a-g]n" },
	{ "ten", "t[!a-g]n" },
	{ "ton", "t[!a-g]n" },
	{ "ton", "t[^a-g]n" },
	{ "a]b", "a[]]b" },
	{ "a-b", "a[]-]b" },
	{ "a]b", "a[]-]b" },
	{ "aab", "a[]-]b" },
	{ "aab", "a[]a-]b" },
	{ "]", "]" },
	{ "foo/baz/bar", "foo*bar" },
	{ "foo/baz/bar", "foo**bar" },
	{ "foobazbar", "foo**bar" },
	{ "foo/baz/bar", "foo/**/bar" },
	{ "foo/baz/bar", "foo/**/**/bar" },
	{ "foo/b/a/z/bar", "foo/**/bar" },
	{ "foo/b/a/z/bar", "foo/**/**/bar" },
	{ "foo/bar", "foo/**/bar" },
	{ "foo/bar", "foo/**/**/bar" },
	{ "foo/bar", "foo?bar" },
	{ "foo/bar", "foo[/]bar" },
	{ "foo/bar", "foo[^a-z]bar" },
	{ "foo/bar", "f[^eiu][^eiu][^eiu][^eiu][^eiu]r" },
	{ "foo-bar", "f[^eiu][^eiu][^eiu][^eiu][^eiu]r" },
	{ "foo", "**/foo" },
	{ "XXX/foo", "**/foo" },
	{ "bar/baz/foo", "**/foo" },
	{ "bar/baz/foo", "*/foo" },
	{ "foo/bar/baz", "**/bar*" },
	{ "deep/foo/bar/baz", "**/bar/*" },
	{ "deep/foo/bar/baz/", "**/bar/*" },
	{ "deep/foo/bar/baz/", "**/bar/**" },
	{ "deep/foo/bar", "**/bar/*" },
	{ "deep/foo/bar/", "**/bar/**" },
	{ "foo/bar/baz", "**/bar**" },
	{ "foo/bar/baz/x", "*/bar/**" },
	{ "deep/foo/bar/baz/x", "*/bar/**" },
	{ "deep/foo/bar/baz/x", "**/bar/*/*" },
	{ "acrt", "a[c-c]st" },
	{ "acrt", "a[c-c]rt" },
	{ "]", "[!]-]" },
	{ "a", "[!]-]" },
	{ "", "\\" },
	{ "\\", "\\" },
	{ "XXX/\\", "*/\\" },
	{ "XXX/\\", "*/\\\\" },
	{ "foo", "foo" },
	{ "@foo", "@foo" },
	{ "foo", "@foo" },
	{ "[ab]", "\\[ab]" },
	{ "[ab]", "[[]ab]" },
	{ "[ab]", "[[:]ab]" },
	{ "[ab]", "[[::]ab]" },
	{ "[ab]", "[[:digit]ab]" },
	{ "[ab]", "[\\[:]ab]" },
	{ "?a?b", "\\??\\?b" },
	{ "abc", "\\a\\b\\c" },
	{ "foo", "" },
	{ "foo/bar/baz/to", "**/t[o]" },
	{ "a1B", "[[:alpha:]][[:digit:]][[:upper:]]" },
	{ "a", "[[:digit:][:upper:][:space:]]" },
	{ "A", "[[:digit:][:upper:][:space:]]" },
	{ "1", "[[:digit:][:upper:][:space:]]" },
	{ "1", "[[:digit:][:upper:][:spaci:]]" },
	{ " ", "[[:digit:][:upper:][:space:]]" },
	{ ".", "[[:digit:][:upper:][:space:]]" },
	{ ".", "[[:digit:][:punct:][:space:]]" },
	{ "5", "[[:xdigit:]]" },
	{ "f", "[[:xdigit:]]" },
	{ "D", "[[:xdigit:]]" },
	{ "_", "[[:alnum:][:alpha:][:blank:][:cntrl:][:digit:][:graph:][:lower:][:print:][:punct:][:space:][:upper:][:xdigit:]]" },
	{ ".", "[^[:alnum:][:alpha:][:blank:][:cntrl:][:digit:][:lower:][:space:][:upper:][:xdigit:]]" },
	{ "5", "[a-c[:digit:]x-z]" },
	{ "b", "[a-c[:digit:]x-z]" },
	{ "y", "[a-c[:digit:]x-z]" },
	{ "q", "[a-c[:digit:]x-z]" },
	{ "]", "[\\\\-^]" },
	{ "[", "[\\\\-^]" },
	{ "-", "[\\-_]" },
	{ "]", "[\\]]" },
	{ "\\]", "[\\]]" },
	{ "\\", "[\\]]" },
	{ "ab", "a[]b" },
	{ "a[]b", "a[]b" },
	{ "ab[", "ab[" },
	{ "ab", "[!" },
	{ "ab", "[-" },
	{ "-", "[-]" },
	{ "-", "[a-" },
	{ "-", "[!a-" },
	{ "-", "[--A]" },
	{ "5", "[--A]" },
	{ " ", "[ --]" },
	{ "$", "[ --]" },
	{ "-", "[ --]" },
	{ "0", "[ --]" },
	{ "-", "[---]" },
	{ "-", "[------]" },
	{ "j", "[a-e-n]" },
	{ "-", "[a-e-n]" },
	{ "a", "[!------]" },
	{ "[", "[]-a]" },
	{ "^", "[]-a]" },
	{ "^", "[!]-a]" },
	{ "[", "[!]-a]" },
	{ "^", "[a^bc]" },
	{ "-b]", "[a-]b]" },
	{ "\\", "[\\]" },
	{ "\\", "[\\\\]" },
	{ "\\", "[!\\\\]" },
	{ "G", "[A-\\\\]" },
	{ "aaabbb", "b*a" },
	{ "aabcaa", "*ba*" },
	{ ",", "[,]" },
	{ ",", "[\\\\,]" },
	{ "\\", "[\\\\,]" },
	{ "-", "[,-.]" },
	{ "+", "[,-.]" },
	{ "-.]", "[,-.]" },
	{ "2", "[\\1-\\3]" },
	{ "3", "[\\1-\\3]" },
	{ "4", "[\\1-\\3]" },
	{ "\\", "[[-\\]]" },
	{ "[", "[[-\\]]" },
	{ "]", "[[-\\]]" },
	{ "-", "[[-\\]]" },
	{ "-adobe-courier-bold-o-normal--12-120-75-75-m-70-iso8859-1", "-*-*-*-*-*-*-12-*-*-*-m-*-*-*" },
	{ "-adobe-courier-bold-o-normal--12-120-75-75-X-70-iso8859-1", "-*-*-*-*-*-*-12-*-*-*-m-*-*-*" },
	{ "-adobe-courier-bold-o-normal--12-120-75-75-/-70-iso8859-1", "-*-*-*-*-*-*-12-*-*-*-m-*-*-*" },
	{ "XXX/adobe/courier/bold/o/normal//12/120/75/75/m/70/iso8859/1", "XXX/*/*/*/*/*/*/12/*/*/*/m/*/*/*" },
	{ "XXX/adobe/courier/bold/o/normal//12/120/75/75/X/70/iso8859/1", "XXX/*/*/*/*/*/*/12/*/*/*/m/*/*/*" },
	{ "abcd/abcdefg/abcdefghijk/abcdefghijklmnop.txt", "**/*a*b*g*n*t" },
	{ "abcd/abcdefg/abcdefghijk/abcdefghijklmnop.txtz", "**/*a*b*g*n*t" },
	{ "foo", "*/*/*" },
	{ "foo/bar", "*/*/*" },
	{ "foo/bba/arr", "*/*/*" },
	{ "foo/bb/aa/rr", "*/*/*" },
	{ "foo/bb/aa/rr", "**/**/**" },
	{ "abcXdefXghi", "*X*i" },
	{ "ab/cXd/efXg/hi", "*X*i" },
	{ "ab/cXd/efXg/hi", "*/*X*/*/*i" },
	{ "ab/cXd/efXg/hi", "**/*X*/**/*i" },
	{ "foo", "fo" },
	{ "foo/bar", "foo/bar" },
	{ "foo/bar", "foo/*" },
	{ "foo/bba/arr", "foo/*" },
	{ "foo/bba/arr", "foo/**" },
	{ "foo/bba/arr", "foo*" },
	{ "foo/bba/arr", "foo**" },
	{ "foo/bba/arr", "foo/*arr" },
	{ "foo/bba/arr", "foo/**arr" },
	{ "foo/bba/arr", "foo/*z" },
	{ "foo/bba/arr", "foo/**z" },
	{ "foo/bar", "foo?bar" },
	{ "foo/bar", "foo[/]bar" },
	{ "foo/bar", "foo[^a-z]bar" },
	{ "ab/cXd/efXg/hi", "*Xg*i" },
	{ "a", "[A-Z]" },
	{ "A", "[A-Z]" },
	{ "A", "[a-z]" },
	{ "a", "[a-z]" },
	{ "a", "[[:upper:]]" },
	{ "A", "[[:upper:]]" },
	{ "A", "[[:lower:]]" },
	{ "a", "[[:lower:]]" },
	{ "A", "[B-Za]" },
	{ "a", "[B-Za]" },
	{ "A", "[B-a]" },
	{ "a", "[B-a]" },
	{ "z", "[Z-y]" },
	{ "Z", "[Z-y]" },
};

static const char *pat_tokens[] = {
	"*", "?", "[", "]", "!", "^", "-", "\\", "/", ":", "a", "b", "A", "B", "z", "0", ".",
	"[:alpha:]", "[:upper:]", "[:lower:]", "[:bogus:]", "[:digit:]", "[:space:]", "[:xdigit:]",
	"[:punct:]", "[:alnum:]", "[:blank:]", "[:cntrl:]", "[:graph:]", "[:print:]", "[:",
	"**", "**/", "/**/", "/**", "[a-z]", "[!a]", "[^/]", "[]-]", "[\\]]", "\\*", "\xc3\xa4"
};
static const char *text_tokens[] = {
	"/", "a", "A", "b", "B", "z", "0", ".", "-", "[", "]", ":", "\\", " ", "\t", "*", "?",
	"ab", "foo/", "/bar", "\xc3\xa4", "\x7f", "\x01", "Z"
};

static void esc(const char *s)
{
	for (; *s; s++)
		printf((*s >= 32 && *s < 127 && *s != '\\') ? "%c" : "\\x%02x", (unsigned char)*s);
}

static void wm(const char *pattern, const char *text)
{
	unsigned f;
	printf("wm [");
	esc(pattern);
	printf("] [");
	esc(text);
	printf("]");
	for (f = 0; f < 4; f++)
		printf(" %d", wildmatch(pattern, text, f));
	printf("\n");
}

static void gen(char *buf, size_t size, const char **tokens, size_t ntokens, size_t max)
{
	size_t n = rnd() % (max + 1), k;
	buf[0] = '\0';
	for (k = 0; k < n; k++) {
		const char *t = tokens[rnd() % ntokens];
		if (strlen(buf) + strlen(t) + 1 >= size)
			break;
		strcat(buf, t);
	}
}

static void re(const char *pattern, int flags, const char **subjects)
{
	git_regexp r = GIT_REGEX_INIT;
	git_regmatch m[5];
	int rc;
	size_t i, n;

	git_error_clear();
	rc = git_regexp_compile(&r, pattern, flags);
	printf("re [");
	esc(pattern);
	printf("] flags=%d rc=%d", flags, rc);
	if (rc < 0) {
		const git_error *e = git_error_last();
		printf(" class=%d msg=%s null=%d\n", e ? e->klass : -1, e ? e->message : "", r == NULL);
		return;
	}
	printf("\n");
	for (i = 0; subjects[i]; i++) {
		printf("  [");
		esc(subjects[i]);
		printf("] match=%d", git_regexp_match(&r, subjects[i]));
		for (n = 0; n <= 5; n++) {
			size_t j;
			memset(m, 0x5a, sizeof(m));
			rc = git_regexp_search(&r, subjects[i], n, m);
			printf(" | n=%zu rc=%d", n, rc);
			for (j = 0; rc == 0 && j < n; j++)
				printf(" %zd,%zd", m[j].start, m[j].end);
		}
		printf("\n");
	}
	git_regexp_dispose(&r);
	printf("  disposed null=%d\n", r == NULL);
}

int main(void)
{
	static const char *subjects[] = {
		"", "a", "abc", "ABC", "hello world", "foo.bar", "refs/heads/master", "x = 1;",
		"static int foo(void)", "\xc3\xa4\xc3\xb6", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaab", "line1\nline2", NULL
	};
	static const char *patterns[] = {
		"", "a", "^a", "a$", "(a)(b)?(c)", "([a-z]+) ([a-z]+)", "^[^ ]+\\.(bar|baz)$",
		"^refs/(heads|tags)/(.*)$", "(?i)abc", "\\bfoo\\b", "^(\\w+)\\s*=", "^static .*\\(",
		"(a|b)*c", "(x)?(y)?(z)?", "[", "(", "a{2,1}", "\\", "*a", "(?<n>a)\\k<n>", "a++b",
		"(a)\\2", "[[:alpha:]]+", "[[:bogus:]]", "\\p{L}+", "(?=a)", ".*", "^$", "line2$",
		"(?m)^line2$", "a{100000}", "(((((((((((a)))))))))))",
		NULL
	};
	char p[256], t[256];
	size_t i;

	git_libgit2_init();

	for (i = 0; i < ARRAY_SIZE(t3070); i++)
		wm(t3070[i][1], t3070[i][0]);
	for (i = 0; i < 40000; i++) {
		gen(p, sizeof(p), pat_tokens, ARRAY_SIZE(pat_tokens), 6);
		gen(t, sizeof(t), text_tokens, ARRAY_SIZE(text_tokens), 8);
		wm(p, t);
	}

	for (i = 0; patterns[i]; i++) {
		re(patterns[i], 0, subjects);
		re(patterns[i], GIT_REGEXP_ICASE, subjects);
	}
	{
		git_regexp r = (git_regexp)&r;
		int rc;
		git_error_clear();
		rc = git_regexp_compile(&r, NULL, 0);
		printf("re NULL pattern rc=%d null=%d msg=%s\n", rc, r == NULL,
			git_error_last() ? git_error_last()->message : "");
	}
	{
		git_regexp r = NULL;
		printf("re NULL match=%d search=%d\n", git_regexp_match(&r, "a"),
			git_regexp_search(&r, "a", 0, NULL));
		git_regexp_dispose(&r);
	}

	{
		static const uint64_t seeds[] = { 0, 1, 42, 0xdeadbeefcafebabeULL, UINT64_MAX };
		for (i = 0; i < ARRAY_SIZE(seeds); i++) {
			int k;
			git_rand_seed(seeds[i]);
			printf("rand %llu:", (unsigned long long)seeds[i]);
			for (k = 0; k < 8; k++)
				printf(" %016llx", (unsigned long long)git_rand_next());
			printf("\n");
		}
	}

	git_libgit2_shutdown();
	return 0;
}
