/*
 * Differential test of the util layer: every line printed must be the same
 * when linked against the C reference and against the Rust port.
 * Covers git_str (growth, encoders and decoders, quoting, joins, printf),
 * strntol, the string helpers, git_vector, tsort and the error messages.
 *
 * Build and compare: rust/difftest/run.sh
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "vector.h"
#include "errors.h"

static unsigned long seed = 12345;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void show_str(const char *what, int rc, const git_str *s)
{
	size_t i;
	printf("%s rc=%d size=%zu asize=%zu oom=%d [", what, rc, s->size, s->asize, git_str_oom(s));
	for (i = 0; s->ptr && !git_str_oom(s) && i < s->size; i++)
		printf("%02x", (unsigned char)s->ptr[i]);
	printf("]\n");
}

static void show_err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%s rc=%d class=%d msg=%s\n", what, rc, e->klass, e->message);
}

static void random_bytes(char *buf, size_t len, int printable)
{
	size_t i;
	for (i = 0; i < len; i++)
		buf[i] = printable ? (char)(32 + rnd() % 95) : (char)(rnd() & 0xff);
}

static void test_growth(void)
{
	git_str s = GIT_STR_INIT;
	int i;
	for (i = 0; i < 200; i++) {
		int rc = git_str_putcn(&s, 'a' + (i % 26), (size_t)(rnd() % 17));
		if (i % 20 == 0) show_str("putcn", rc, &s);
	}
	show_str("grow", git_str_grow(&s, 5000), &s);
	show_str("grow_by", git_str_grow_by(&s, 3), &s);
	git_str_dispose(&s);
	show_str("init", git_str_init(&s, 37), &s);
	show_str("printf", git_str_printf(&s, "%s-%d-%08x-%c-%5.2f", "abc", -42, 0xbeef, 'z', 3.14159), &s);
	show_str("printf_long", git_str_printf(&s, "%0300d", 7), &s);
	git_str_dispose(&s);
}

static void test_codecs(void)
{
	char in[300];
	int round;
	for (round = 0; round < 60; round++) {
		git_str e = GIT_STR_INIT, d = GIT_STR_INIT;
		size_t len = rnd() % 200;
		random_bytes(in, len, 0);
		show_str("b64", git_str_encode_base64(&e, in, len), &e);
		show_str("b64d", git_str_decode_base64(&d, e.ptr, e.size), &d);
		git_str_clear(&e); git_str_clear(&d);
		show_str("b85", git_str_encode_base85(&e, in, len), &e);
		show_str("b85d", git_str_decode_base85(&d, e.ptr, e.size, len), &d);
		git_str_clear(&e);
		show_str("hex", git_str_encode_hexstr(&e, in, len), &e);
		git_str_clear(&e);
		/* garbage in */
		random_bytes(in, len, round % 2);
		show_str("b64g", git_str_decode_base64(&e, in, len & ~3u), &e);
		show_err("b64g_err", 0);
		show_str("b85g", git_str_decode_base85(&e, in, (len / 5) * 5, rnd() % (len + 1)), &e);
		show_str("pct", git_str_decode_percent(&e, in, len), &e);
		git_str_dispose(&e); git_str_dispose(&d);
	}
}

static void test_quote(void)
{
	char in[64];
	int round;
	const char *bad[] = { "\"\\0\"", "\"\\01\"", "\"\\08x\"", "\"\\q\"", "\"abc", "\"a\\\"", "\"\\3777\"", "  \"x\\n\"  " };
	size_t i;
	for (round = 0; round < 200; round++) {
		git_str s = GIT_STR_INIT;
		size_t len = rnd() % 40;
		random_bytes(in, len, round % 3 != 0);
		git_str_put(&s, in, len);
		show_str("quote", git_str_quote(&s), &s);
		show_str("unquote", s.size && s.ptr[0] == '"' ? git_str_unquote(&s) : 0, &s);
		git_str_dispose(&s);
	}
	for (i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
		git_str s = GIT_STR_INIT;
		int rc;
		git_str_puts(&s, bad[i]);
		rc = git_str_unquote(&s);
		show_err("unquote_bad", rc);
		git_str_dispose(&s);
	}
}

static void test_joins(void)
{
	const char *parts[] = { "", "/", "a", "a/", "/a", "//a//", "bc", "/" };
	size_t n = sizeof(parts) / sizeof(parts[0]), i, j, k;
	for (i = 0; i < n; i++) for (j = 0; j < n; j++) {
		git_str s = GIT_STR_INIT;
		show_str("join", git_str_join(&s, '/', parts[i], parts[j]), &s);
		show_str("join_self", git_str_joinpath(&s, s.ptr, parts[j]), &s);
		for (k = 0; k < n; k += 3)
			show_str("join3", git_str_join3(&s, '/', parts[i], parts[j], parts[k]), &s);
		git_str_clear(&s);
		git_str_puts(&s, parts[i]);
		show_str("join_n", git_str_join_n(&s, '/', 3, parts[j], NULL, parts[(i + j) % n]), &s);
		show_str("join_n_self", git_str_join_n(&s, '/', 2, s.ptr, parts[j]), &s);
		show_str("join_n4", git_str_join_n(&s, ';', 4, parts[j], parts[i], "", parts[j]), &s);
		git_str_dispose(&s);
	}
}

static void test_misc_str(void)
{
	const char *texts[] = { "a\r\nb\rc\n", "\r\n\r\n", "plain", "\r", "x\n\n\r\ny", "" };
	size_t i;
	for (i = 0; i < sizeof(texts) / sizeof(texts[0]); i++) {
		git_str src = GIT_STR_INIT, t = GIT_STR_INIT;
		git_str_text_stats st;
		bool bin;
		git_str_puts(&src, texts[i]);
		show_str("crlf_to_lf", git_str_crlf_to_lf(&t, &src), &t);
		show_str("lf_to_crlf", git_str_lf_to_crlf(&t, &src), &t);
		bin = git_str_gather_text_stats(&st, &src, false);
		printf("stats %d %u %u %u %u %u %u %d bin=%d\n", st.bom, st.nul, st.cr, st.lf, st.crlf,
			st.printable, st.nonprintable, (int)bin, git_str_is_binary(&src));
		git_str_puts_escaped(&t, texts[i], "\r\n", "<", ">");
		show_str("escaped", 0, &t);
		/* Pre-grow: C's splice uses a stale pointer when it reallocates
		 * (rust/DEFECTS.md), which is undefined behaviour. */
		git_str_grow(&t, t.size + 8);
		show_str("splice", git_str_splice(&t, 1, t.size > 2 ? 2 : 0, "XYZ", 3), &t);
		git_str_rtrim(&t);
		show_str("rtrim", 0, &t);
		git_str_dispose(&src); git_str_dispose(&t);
	}
}

static void test_strtol(void)
{
	const char *nums[] = { "0", "  42", "-17", "+5x", "0x1F", "0X", "017", "019", "z", "",
		"9223372036854775807", "9223372036854775808", "-9223372036854775808",
		"-9223372036854775809", "2147483647", "2147483648", "-2147483649", " \t\n12" };
	int bases[] = { 0, 8, 10, 16, 36, 37 };
	size_t i, b;
	for (i = 0; i < sizeof(nums) / sizeof(nums[0]); i++) for (b = 0; b < sizeof(bases) / sizeof(bases[0]); b++) {
		int64_t r64 = 777; int32_t r32 = 777;
		const char *end = NULL;
		int rc = git__strntol64(&r64, nums[i], strlen(nums[i]), &end, bases[b]);
		printf("strtol64 %s/%d rc=%d v=%lld end=%ld\n", nums[i], bases[b], rc, (long long)r64, end ? (long)(end - nums[i]) : -1L);
		if (rc) show_err("  err", rc);
		end = NULL;
		rc = git__strntol32(&r32, nums[i], strlen(nums[i]), &end, bases[b]);
		printf("strtol32 %s/%d rc=%d v=%d end=%ld\n", nums[i], bases[b], rc, r32, end ? (long)(end - nums[i]) : -1L);
		if (rc) show_err("  err", rc);
	}
}

static void test_string_helpers(void)
{
	const char *w[] = { "", "a", "A", "ab", "aB", "abc", "Abd", "refs/", "refs/heads", "ReFs/HEADS/x", "\xc4", "\xe4" };
	size_t n = sizeof(w) / sizeof(w[0]), i, j;
	for (i = 0; i < n; i++) for (j = 0; j < n; j++) {
		printf("cmp %zu %zu: %d %d %d %d %d %d %d %d %d\n", i, j,
			git__strcasecmp(w[i], w[j]), git__strcasesort_cmp(w[i], w[j]),
			git__strncasecmp(w[i], w[j], 2), git__strncasecmp(w[i], w[j], 0),
			git__prefixcmp(w[i], w[j]), git__prefixcmp_icase(w[i], w[j]),
			git__prefixncmp(w[i], 3, w[j]), git__suffixcmp(w[i], w[j]),
			git__suffixcmp_icase(w[i], w[j]));
	}
	for (i = 0; i < 50; i++) {
		char h[64], nd[8];
		size_t hl = rnd() % 64, nl = rnd() % 4;
		const void *f;
		for (j = 0; j < hl; j++) h[j] = "ab"[rnd() % 2];
		for (j = 0; j < nl; j++) nd[j] = "ab"[rnd() % 2];
		f = git__memmem(h, hl, nd, nl);
		printf("memmem %ld\n", f ? (long)((const char *)f - h) : -1L);
		printf("hash %08x\n", git__hash(h, (int)hl, (uint32_t)i));
	}
	{
		const char *bools[] = { "true", "YES", "On", "false", "no", "OFF", "", "maybe" };
		int v;
		for (i = 0; i < 8; i++) {
			v = 9;
			printf("bool %s %d %d\n", bools[i], git__parse_bool(&v, bools[i]), v);
		}
		printf("bool NULL %d %d\n", git__parse_bool(&v, NULL), v);
	}
	{
		char s1[] = "a\\b\\\\c\\", s2[] = "  x,y,,z ", *p, *t;
		printf("unescape %zu %s\n", git__unescape(s1), s1);
		p = s2;
		while ((t = git__strtok(&p, " ,")) != NULL) printf("tok [%s]\n", t);
		strcpy(s2, "x,y,,z");
		p = s2;
		while ((t = git__strsep(&p, ",")) != NULL) printf("sep [%s]\n", t);
		printf("sep rest [%s]\n", p);
	}
}

static int cmp_str(const void *a, const void *b) { return strcmp(a, b); }
static int cmp_first(const void *a, const void *b) { return *(const char *)a - *(const char *)b; }
static int dup_cb(void **old, void *new) { (void)old; return ((char *)new)[1] == '!' ? -7 : 0; }
static int match_cb(const git_vector *v, size_t idx, void *p) { (void)p; return ((char *)v->contents[idx])[0] == 'b'; }

static void show_vec(const char *what, int rc, git_vector *v)
{
	size_t i;
	printf("%s rc=%d len=%zu alloc=%zu sorted=%d [", what, rc, v->length, v->_alloc_size, git_vector_is_sorted(v));
	for (i = 0; i < v->length; i++) printf("%s,", v->contents[i] ? (char *)v->contents[i] : "NULL");
	printf("]\n");
}

static void test_vector(void)
{
	static char words[400][8];
	git_vector v = GIT_VECTOR_INIT, d;
	size_t i, pos;
	int rc;
	for (i = 0; i < 400; i++) {
		size_t j, l = 1 + rnd() % 3;
		for (j = 0; j < l; j++) words[i][j] = "abc!"[rnd() % 4];
		words[i][l] = 0;
	}
	show_vec("init", git_vector_init(&v, 3, cmp_first), &v);
	for (i = 0; i < 300; i++) {
		rc = git_vector_insert(&v, words[i]);
		if (i % 50 == 0) printf("alloc %zu\n", v._alloc_size);
	}
	git_vector_sort(&v);
	show_vec("sort_stable", 0, &v);
	printf("verify %d\n", git_vector_verify_sorted(&v));
	for (i = 300; i < 330; i++)
		show_vec("insert_sorted", git_vector_insert_sorted(&v, words[i], dup_cb), &v);
	pos = 999;
	rc = git_vector_bsearch(&pos, &v, "b");
	printf("bsearch %d %zu\n", rc, pos);
	rc = git_vector_bsearch(&pos, &v, "!");
	printf("bsearch %d %zu\n", rc, pos);
	rc = git_vector_search(&pos, &v, words[305]);
	printf("search %d %zu\n", rc, pos);
	show_vec("dup", git_vector_dup(&d, &v, cmp_str), &d);
	git_vector_uniq(&d, NULL);
	show_vec("uniq", 0, &d);
	git_vector_remove_matching(&d, match_cb, NULL);
	show_vec("remove_matching", 0, &d);
	show_vec("insert_null", git_vector_insert_null(&d, 1, 3), &d);
	show_vec("remove_range", git_vector_remove_range(&d, 0, 2), &d);
	show_err("remove_range_bad", git_vector_remove_range(&d, 1, 100));
	show_err("insert_null_bad", git_vector_insert_null(&d, 100, 1));
	show_vec("set", git_vector_set(NULL, &d, d.length + 2, "S"), &d);
	git_vector_reverse(&d);
	show_vec("reverse", 0, &d);
	show_vec("resize", git_vector_resize_to(&d, 2), &d);
	show_vec("remove", git_vector_remove(&d, 5), &d);
	git_vector_dispose(&d);
	git_vector_dispose(&v);

	/* tsort, large: random strings, nearly sorted, reversed */
	{
		static void *arr[5000];
		size_t n = 5000, k;
		for (k = 0; k < 3; k++) {
			for (i = 0; i < n; i++) arr[i] = words[k == 0 ? rnd() % 400 : (k == 1 ? (i * 400 / n) : (n - 1 - i) * 400 / n)];
			if (k == 1) for (i = 0; i < 30; i++) { size_t a = rnd() % n, b = rnd() % n; void *t = arr[a]; arr[a] = arr[b]; arr[b] = t; }
			git__tsort(arr, n, cmp_first);
			for (i = 0; i < n; i += 97) printf("%s%s", (char *)arr[i], (i + 97 < n) ? "," : "\n");
			printf("ptrs %ld %ld\n", (long)((char (*)[8])arr[0] - words), (long)((char (*)[8])arr[n - 1] - words));
		}
	}
}

static void test_errors(void)
{
	git_error *saved;
	errno = ENOENT;
	git_error_set(GIT_ERROR_OS, "failed to open '%s'", "/x\xff/y");
	show_err("os", -1);
	printf("errno after %d\n", errno);
	errno = 0;
	git_error_set(GIT_ERROR_OS, "no errno");
	show_err("os0", -1);
	git_error_set(GIT_ERROR_NET, "%s", "");
	show_err("empty", -1);
	git_error_set(GIT_ERROR_REFERENCE, "%0700d|end", 1);
	printf("long %zu %s\n", strlen(git_error_last()->message), git_error_last()->message + 690);
	git_error_set_str(GIT_ERROR_TAG, "plain");
	show_err("set_str", 0);
	printf("set_str NULL %d\n", git_error_set_str(GIT_ERROR_TAG, NULL));
	show_err("set_str_null", 0);
	printf("save %d\n", git_error_save(&saved));
	git_error_clear();
	show_err("cleared", 0);
	printf("exists %d\n", git_error_exists());
	git_error_restore(saved);
	show_err("restored", 0);
	git_error_set_oom();
	show_err("oom", 0);
	printf("save %d\n", git_error_save(&saved));
	git_error_clear();
	git_error_restore(saved);
	show_err("restored_oom", 0);
	git_error_clear();
	printf("save %d\n", git_error_save(&saved));
	git_error_restore(saved);
	show_err("restored_none", 0);
	printf("exists %d\n", git_error_exists());
}

int main(void)
{
	git_libgit2_init();
	test_growth();
	test_codecs();
	test_quote();
	test_joins();
	test_misc_str();
	test_strtol();
	test_string_helpers();
	test_vector();
	test_errors();
	git_libgit2_shutdown();
	printf("after shutdown: %s\n", git_error_last()->message);
	return 0;
}
