/* Review l-obj r1: signature.c differential driver (C vs Rust).
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2/global.h"
#include "git2/signature.h"
#include "git2/sys/alloc.h"
#include "git2/repository.h"
#include "git2/config.h"
#include "signature.h"
#include "str.h"
#include <sys/mman.h>
#include <unistd.h>
#include <inttypes.h>
#include <limits.h>
#include <time.h>

static uint64_t st = 0x123456789abcdefull;
static uint64_t rnd(void) { st ^= st >> 12; st ^= st << 25; st ^= st >> 27; return st * 2685821657736338717ull; }

static void q(const char *s, size_t n)
{
	size_t i;
	putchar('"');
	for (i = 0; i < n; i++) {
		unsigned char c = (unsigned char)s[i];
		if (c >= 32 && c < 127 && c != '\\' && c != '"') putchar(c);
		else printf("\\x%02x", c);
	}
	putchar('"');
}
static void qs(const char *s) { if (!s) printf("NULL"); else q(s, strlen(s)); }

static void err(void)
{
	const git_error *e = git_error_last();
	printf(" err=%d:", e ? e->klass : -99);
	qs(e ? e->message : NULL);
	printf("\n");
}

static void reset_err(void) { git_error_set_str(GIT_ERROR_NET, "sentinel"); }

/* guard page */
static char *page; static size_t pgsz;
static const char *guarded(const char *s, size_t n)
{
	char *p = page + pgsz - n;
	memcpy(p, s, n);
	return p;
}

static void parse_case(const char *in, size_t n, const char *header, char ender)
{
	git_signature sig;
	const char *b = guarded(in, n), *p = b;
	int rc;
	memset(&sig, 0xAB, sizeof(sig));
	reset_err();
	rc = git_signature__parse(&sig, &p, b + n, header, ender);
	printf("parse hdr=%s ender=%d in=", header ? header : "(null)", (int)ender);
	q(in, n);
	printf(" rc=%d", rc);
	if (rc == 0) {
		printf(" name="); qs(sig.name);
		printf(" email="); qs(sig.email);
		printf(" t=%" PRId64 " off=%d sign=%d used=%ld", (int64_t)sig.when.time, sig.when.offset, sig.when.sign, (long)(p - b));
		git__free(sig.name); git__free(sig.email);
	} else {
		printf(" n=%p e=%p t=%" PRId64 " off=%d sign=%d p_moved=%d", (void *)sig.name, (void *)sig.email,
			(int64_t)sig.when.time, sig.when.offset, sig.when.sign, p != b);
	}
	err();
}

static void from_buffer_case(const char *in)
{
	git_signature *sig = (git_signature *)0x1;
	int rc;
	reset_err();
	rc = git_signature_from_buffer(&sig, in);
	printf("from_buffer "); qs(in);
	printf(" rc=%d out=%s", rc, sig == NULL ? "NULL" : sig == (git_signature *)0x1 ? "untouched" : "set");
	if (rc == 0 && sig) {
		printf(" name="); qs(sig->name); printf(" email="); qs(sig->email);
		printf(" t=%" PRId64 " off=%d sign=%d", (int64_t)sig->when.time, sig->when.offset, sig->when.sign);
		git_signature_free(sig);
	}
	err();
}

static const char *corpus[] = {
	"a <b> 1 +0100",
	"a <b> 1 -0000", "a <b> 1 +0000", "a <b> 1 +000", "a <b> 1 +00000", "a <b> 1 +000000",
	"a <b> 1 +9999", "a <b> 1 -9999", "a <b> 1 +1459", "a <b> 1 +1460", "a <b> 1 +1500", "a <b> 1 -1500",
	"a <b> 1 +1", "a <b> 1 -1", "a <b> 1 +", "a <b> 1 -", "a <b> 1 x0100", "a <b> 1 0100", "a <b> 1 +01a0",
	"a <b> 1 + 0100", "a <b> 1 +\t0100", "a <b> 1 +\v100", "a <b> 1 +\f100", "a <b> 1 +\r100", "a <b> 1 +\n100",
	"a <b> 1 +-100", "a <b> 1 ++100", "a <b> 1 -+100", "a <b> 1 --100", "a <b> 1 +-2147483648", "a <b> 1 +2147483648",
	"a <b> 1 -2147483649", "a <b> 1 +99999999999999999999", "a <b> 1 +0x10", "a <b> 1 +\xff", "a <b> 1 \xff" "0100",
	"a <b> 1 +0100 trailing", "a <b> 1 +0100\n", "a <b> 1 +0100\nmore", "a <b> 1  +0100", "a <b> 1\t+0100", "a <b> 1",
	"a <b> +1 +0100", "a <b> -1 +0100", "a <b> 01 +0100", "a <b> 1a +0100", "a <b> a1 +0100",
	"a <b>  1 +0100", "a <b> \v1 +0100", "a <b> \f1", "a <b> \r1", "a <b> \n1", "a <b> \t 1", "a <b> \x85" "1",
	"a <b> 9223372036854775807 +0100", "a <b> 9223372036854775808 +0100", "a <b> -9223372036854775808",
	"a <b> -9223372036854775809", "a <b> 99999999999999999999999999999999999999 +0100",
	"a <b> 000000000000000000000000000000000000001 +0100", "a <b> 18446744073709551616",
	"a <b>", "a <b> ", "a <b>  ", "a <b>   ", "a <b>x", "a <b>xy", "a <b>x1", "a <b>x1 +0100", "a <b>1 +0100",
	"<b> 1", "<> 1", "a <> 1", " <b> 1", "a < > 1", "a <\t> 1", "a b> 1", "a <b 1", "a >b< 1", "a <b> <c> 1",
	"a <b> c> 1", "a <b>> 1", "a <<b> 1", "a<b>1", "a>b<c>d 1", ">a<b> 1", "a <b><c> 1",
	"\"a\" <'b'> 1", ",;:a:;, <\\b\\> 1", "\x01" "a\x1f <\x7f" "b\x80> 1", "\xc3\xa9 <\xff\xfe> 1", "\x80" "a\x80 <\xa0" "b\xa0> 1",
	" \t\n\r\v\f a \t\n\r\v\f <  b  > 1", "a <b> 1 +0100 <c>", "", " ", "<", ">", "<>", "><",
	"author a <b> 1 +0100", "author a <b> 1 +0100\n", "author \n", "author", "author ",
	"a <b> 1 +01009", "a <b> 1 +0100999999", "a <b> 1 -0", "a <b> 1 +0", "a <b> 1 -00", "a <b> 1 -059", "a <b> 1 -060",
	"a <b> 1 +1400", "a <b> 1 +1401", "a <b> 1 -1460", "a <b> 1 +-1459", "a <b> 1 +-99999",
};

static const char *headers[] = { NULL, "", "a", "a ", "author ", "author a <b> 1 +0100", "author a <b> 1 +0100\n", "\xff" };
static const char enders[] = { 0, '\n', '>', ' ', '<', '1', (char)0xff };

static void parse_tests(void)
{
	size_t i, h, k, len, n;
	char buf[256];
	for (i = 0; i < ARRAY_SIZE(corpus); i++) {
		size_t full = strlen(corpus[i]);
		for (h = 0; h < ARRAY_SIZE(headers); h++)
			for (k = 0; k < ARRAY_SIZE(enders); k++)
				parse_case(corpus[i], full, headers[h], enders[k]);
		for (len = 0; len < full; len++) {
			parse_case(corpus[i], len, NULL, 0);
			parse_case(corpus[i], len, NULL, '\n');
		}
		from_buffer_case(corpus[i]);
	}
	/* embedded NULs through parse (explicit length) */
	{
		static const char a[] = "a\0b <c\0d> 1\0 +0100";
		static const char b2[] = "a <b> 1 +01\00000";
		static const char c[] = "a <b> \0" "1 +0100";
		parse_case(a, sizeof(a) - 1, NULL, 0);
		parse_case(a, sizeof(a) - 1, NULL, '\n');
		parse_case(b2, sizeof(b2) - 1, NULL, 0);
		parse_case(c, sizeof(c) - 1, NULL, 0);
		from_buffer_case(a);
	}
	/* fuzz */
	{
		static const char alpha[] = "<> +-0123456789aZ\n\t\v\f\r,;:\"'\\\x01\xff\x80.x\0";
		for (n = 0; n < 300000; n++) {
			size_t l;
			if (n % 2 == 0) {
				const char *s = corpus[rnd() % ARRAY_SIZE(corpus)];
				l = strlen(s);
				memcpy(buf, s, l);
				for (k = rnd() % 5; k > 0 && l > 0; k--) {
					size_t pos = rnd() % l;
					if (rnd() % 3 == 0 && l < 200) { memmove(buf + pos + 1, buf + pos, l - pos); l++; }
					buf[pos] = alpha[rnd() % (sizeof(alpha) - 1)];
				}
			} else {
				l = rnd() % 40;
				for (k = 0; k < l; k++)
					buf[k] = alpha[rnd() % (sizeof(alpha) - 1)];
			}
			parse_case(buf, l, headers[rnd() % ARRAY_SIZE(headers)], enders[rnd() % ARRAY_SIZE(enders)]);
			buf[l] = 0;
			if (n % 7 == 0)
				from_buffer_case(buf);
		}
	}
	/* NULL args */
	{
		git_signature *s = (git_signature *)0x1;
		reset_err(); printf("from_buffer(NULL out) rc=%d", git_signature_from_buffer(NULL, "a <b>")); err();
		reset_err(); printf("from_buffer(NULL buf) rc=%d out=%p", git_signature_from_buffer(&s, NULL), (void *)s); err();
	}
}

static void new_tests(void)
{
	static const char *names[] = { "a", " a ", "<a", "a>", "", " ", ",;:", "\"x\"", "x\\", "\x01y\x7f", "\xc3\xa9",
		"na me", "'q'", "\x80", "\xa0x\xa0", "\xffx\xff", " \t\n\r\v\f", "a<b", "a b c", ".a.", "\x7f" };
	static const int offsets[] = { 0, 1, -1, 60, -60, INT_MAX, INT_MIN };
	size_t a, b, c;
	git_signature *sig;
	int rc;
	for (a = 0; a < ARRAY_SIZE(names); a++)
		for (b = 0; b < ARRAY_SIZE(names); b++) {
			sig = (git_signature *)0x1;
			reset_err();
			rc = git_signature_new(&sig, names[a], names[b], (git_time_t)a * 1000 - 5, offsets[b % ARRAY_SIZE(offsets)]);
			printf("new "); qs(names[a]); printf(" "); qs(names[b]);
			printf(" rc=%d out=%s", rc, sig == NULL ? "NULL" : sig == (git_signature *)0x1 ? "untouched" : "set");
			if (!rc) { printf(" -> "); qs(sig->name); printf(" "); qs(sig->email);
				printf(" t=%" PRId64 " off=%d sign=%d", (int64_t)sig->when.time, sig->when.offset, sig->when.sign); }
			err();
			if (!rc) git_signature_free(sig);
		}
	/* every single byte as name */
	for (c = 1; c < 256; c++) {
		char nm[4] = { 'x', (char)c, 'x', 0 }, n1[2] = { (char)c, 0 };
		sig = NULL; reset_err();
		rc = git_signature_new(&sig, n1, nm, 0, 0);
		printf("new1 %zu rc=%d", c, rc);
		if (!rc) { printf(" "); qs(sig->name); printf(" "); qs(sig->email); git_signature_free(sig); }
		err();
	}
	sig = (git_signature *)0x1;
	reset_err(); printf("new(NULL name) rc=%d out=%p", git_signature_new(&sig, NULL, "e", 0, 0), (void *)sig); err();
	reset_err(); printf("new(NULL email) rc=%d out=%p", git_signature_new(&sig, "n", NULL, 0, 0), (void *)sig); err();
	reset_err(); printf("now(NULL name) rc=%d out=%p", git_signature_now(&sig, NULL, "e"), (void *)sig); err();
	reset_err(); rc = git_signature_now(&sig, "<a", "e"); printf("now(<a) rc=%d out=%p", rc, (void *)sig); err();
}

static void writebuf_tests(void)
{
	static const int offsets[] = { 0, 60, -60, 90, -90, 1, -1, 59, -59, 840, 6000, -6000, 5999 * 60, INT_MAX, INT_MIN, INT_MIN + 1 };
	static const int64_t times[] = { 0, 1, -1, 4294967295LL, 4294967296LL, INT64_MAX, INT64_MIN };
	static const char signs[] = { '+', '-', 0, '*', (char)0xff };
	size_t t, o, s, pre;
	for (t = 0; t < ARRAY_SIZE(times); t++)
		for (o = 0; o < ARRAY_SIZE(offsets); o++)
			for (s = 0; s < ARRAY_SIZE(signs); s++) {
				git_signature sig = { "N", "e@x", { times[t], offsets[o], signs[s] } };
				git_str out = GIT_STR_INIT;
				git_signature__writebuf(&out, s & 1 ? "tagger " : NULL, &sig);
				printf("wb size=%zu asize=%zu ", out.size, out.asize); q(out.ptr, out.size); printf("\n");
				git_str_dispose(&out);
			}
	/* NULL strings, empty header, growth from pre-existing contents */
	{
		git_signature sig = { NULL, NULL, { 5, 5, '+' } };
		git_str out = GIT_STR_INIT;
		git_signature__writebuf(&out, "", &sig);
		printf("wb-null size=%zu asize=%zu ", out.size, out.asize); q(out.ptr, out.size); printf("\n");
		git_str_dispose(&out);
	}
	for (pre = 0; pre < 300; pre += 7) {
		static char longname[400];
		git_signature sig = { longname, "e", { 1, 0, '+' } };
		git_str out = GIT_STR_INIT;
		memset(longname, 'n', sizeof(longname) - 1);
		longname[(pre * 13) % 399] = 0;
		git_str_grow(&out, pre + 1);
		memset(out.ptr, 'p', pre); out.size = pre; out.ptr[pre] = 0;
		git_signature__writebuf(&out, pre % 2 ? "h " : NULL, &sig);
		printf("wb-grow pre=%zu size=%zu asize=%zu\n", pre, out.size, out.asize);
		git_str_dispose(&out);
	}
	/* oom buffer */
	{
		git_signature sig = { "n", "e", { 1, 0, '+' } };
		git_str out = GIT_STR_INIT;
		out.ptr = git_str__oom;
		reset_err();
		git_signature__writebuf(&out, NULL, &sig);
		printf("wb-oom oom=%d size=%zu asize=%zu", git_str_oom(&out), out.size, out.asize); err();
	}
}

static void equal_dup_tests(void)
{
	static const char *strs[] = { "a", "b", "", "ab", "a\xff", "a\x80", "A" };
	size_t i, j;
	for (i = 0; i < 400; i++) {
		git_signature x = { (char *)strs[rnd() % 7], (char *)strs[rnd() % 7], { (int64_t)(rnd() % 2), (int)(rnd() % 3) - 1, "+-\0"[rnd() % 3] } };
		git_signature y = { (char *)strs[rnd() % 7], (char *)strs[rnd() % 7], { (int64_t)(rnd() % 2), (int)(rnd() % 3) - 1, "+-\0"[rnd() % 3] } };
		git_signature *d = (git_signature *)0x1;
		if (i % 3 == 0) y = x;
		printf("eq %d", git_signature__equal(&x, &y));
		reset_err();
		j = git_signature_dup(&d, &x);
		printf(" dup rc=%zu eqdup=%d same_ptr=%d\n", j, git_signature__equal(&x, d), d->name == x.name);
		git_signature_free(d);
	}
	{
		git_signature x = { "a", "b", { 1, 0, '+' } }, *d = (git_signature *)0x1;
		reset_err(); printf("eq(NULL,x)=%d", git_signature__equal(NULL, &x)); err();
		reset_err(); printf("eq(x,NULL)=%d", git_signature__equal(&x, NULL)); err();
		reset_err(); printf("eq(NULL,NULL)=%d", git_signature__equal(NULL, NULL)); err();
		reset_err(); printf("dup(NULL) rc=%d d=%p", git_signature_dup(&d, NULL), (void *)d); err();
	}
	git_signature_free(NULL);
}

static void now_tests(void)
{
	static const char *tzs[] = { "UTC", "America/New_York", "Asia/Kolkata", "Pacific/Chatham", "Asia/Kathmandu",
		"Pacific/Kiritimati", "Pacific/Pago_Pago", "XYZ+13:17", "ABC-14:59:59", "Europe/Dublin", "garbage///" };
	size_t i;
	for (i = 0; i < ARRAY_SIZE(tzs); i++) {
		git_signature *s = NULL; int rc;
		setenv("TZ", tzs[i], 1); tzset();
		rc = git_signature_now(&s, " n ", " e ");
		printf("now TZ=%s rc=%d", tzs[i], rc);
		if (!rc) { printf(" "); qs(s->name); printf(" "); qs(s->email); printf(" off=%d sign=%c tdiff_ok=%d\n", s->when.offset, s->when.sign, (long long)(time(NULL) - s->when.time) < 3); git_signature_free(s); }
		else err();
	}
	setenv("TZ", "UTC", 1); tzset();
}

/* OOM: fail the Nth allocation, count leaks */
static long live, fail_at, count;
static void *m_malloc(size_t n, const char *f, int l) { void *p; (void)f; (void)l; if (++count == fail_at) return NULL; p = malloc(n); if (p) live++; return p; }
static void *m_realloc(void *p, size_t n, const char *f, int l) { void *r; (void)f; (void)l; if (++count == fail_at) return NULL; r = realloc(p, n); if (r && !p) live++; return r; }
static void m_free(void *p) { if (p) live--; free(p); }
static git_allocator oom_alloc = { m_malloc, m_realloc, m_free };

static void oom_tests(void)
{
	long k;
	static const char line[] = "author a <b> 1 +0100\n";
	for (k = 1; k <= 5; k++) {
		git_signature *s = (git_signature *)0x1, sg, src = { "nn", "ee", { 1, 2, '+' } };
		int rc; const char *p = line;
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, &oom_alloc);
		live = 0; count = 0; fail_at = k;
		rc = git_signature_new(&s, "n", "e", 1, 0);
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, NULL);
		printf("oom new k=%ld rc=%d live=%ld out=%s", k, rc, live, s == NULL ? "NULL" : s == (git_signature *)0x1 ? "untouched" : "set"); err();

		s = (git_signature *)0x1;
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, &oom_alloc);
		live = 0; count = 0; fail_at = k;
		rc = git_signature_dup(&s, &src);
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, NULL);
		printf("oom dup k=%ld rc=%d live=%ld out=%s", k, rc, live, s == NULL ? "NULL" : s == (git_signature *)0x1 ? "untouched" : "set"); err();

		s = (git_signature *)0x1;
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, &oom_alloc);
		live = 0; count = 0; fail_at = k;
		rc = git_signature_from_buffer(&s, "a <b> 1 +0100");
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, NULL);
		printf("oom from_buffer k=%ld rc=%d live=%ld out=%s", k, rc, live, s == NULL ? "NULL" : s == (git_signature *)0x1 ? "untouched" : "set"); err();

		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, &oom_alloc);
		live = 0; count = 0; fail_at = k;
		rc = git_signature__parse(&sg, &p, line + sizeof(line) - 1, "author ", '\n');
		git_libgit2_opts(GIT_OPT_SET_ALLOCATOR, NULL);
		printf("oom parse k=%ld rc=%d live=%ld name_null=%d email_null=%d", k, rc, live, sg.name == NULL, sg.email == NULL); err();
	}
}

int main(int argc, char **argv)
{
	const char *only = argc > 1 ? argv[1] : "";
	pgsz = (size_t)sysconf(_SC_PAGESIZE);
	page = mmap(NULL, 2 * pgsz, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	mprotect(page + pgsz, pgsz, PROT_NONE);
	git_libgit2_init();
	setvbuf(stdout, NULL, _IOFBF, 1 << 20);
	if (strstr("parse", only) || !*only) parse_tests();
	if (strstr("new", only) || !*only) new_tests();
	if (strstr("writebuf", only) || !*only) writebuf_tests();
	if (strstr("equal", only) || !*only) equal_dup_tests();
	if (strstr("now", only) || !*only) now_tests();
	if (strstr("oom", only) || !*only) oom_tests();
	git_libgit2_shutdown();
	return 0;
}
