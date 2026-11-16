/*
 * Differential test of work package u1: every line printed must be the same
 * when linked against the C reference and against the Rust port.
 * Covers oid (parsing, formatting, comparison, the shortener, errors),
 * oidarray, utf8, varint, pool (paging and offsets), pqueue (heap order,
 * fixed size), parse (git_parse_ctx) and strlist.
 *
 * difftest-link: libgit2_tests
 *
 * Build and compare: RUST_BUILD=/work/builds/u1 rust/difftest/run.sh rust/difftest/u1_diff.c
 */

#include "common.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "git2/oid.h"
#include "oid.h"
#include "oidarray.h"
#include "parse.h"
#include "pool.h"
#include "pqueue.h"
#include "strlist.h"
#include "utf8.h"
#include "varint.h"

static unsigned long seed = 4242;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void show_err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%s rc=%d class=%d msg=%s\n", what, rc, e->klass, e->message);
	git_error_clear();
}

static void show_oid(const char *what, int rc, const git_oid *o)
{
	size_t i;
	printf("%s rc=%d type=%u id=", what, rc, o->type);
	for (i = 0; i < GIT_OID_MAX_SIZE; i++)
		printf("%02x", o->id[i]);
	printf("\n");
	if (rc < 0)
		show_err(what, rc);
}

static const char hexchars[] = "0123456789abcdefABCDEFxg \0";

static void random_hex(char *buf, size_t len, int mostly_valid)
{
	size_t i;
	for (i = 0; i < len; i++) {
		unsigned r = rnd() % 100;
		buf[i] = (mostly_valid && r < 97) ? hexchars[rnd() % 22] : hexchars[rnd() % (sizeof(hexchars) - 1)];
	}
}

static void test_oid_parse(void)
{
	char buf[80];
	git_oid o;
	int i, t;

	for (i = 0; i < 2000; i++) {
		size_t len = rnd() % 70;
		t = (int)(rnd() % 4);
		random_hex(buf, sizeof(buf), i % 3 != 0);
		buf[len] = '\0';
		memset(&o, 0x5a, sizeof(o));
		show_oid("from_prefix", git_oid_from_prefix(&o, buf, len, t), &o);
		memset(&o, 0x5a, sizeof(o));
		show_oid("from_string", git_oid_from_string(&o, buf, t), &o);
		if (len == 40 || len == 64) {
			memset(&o, 0x5a, sizeof(o));
			show_oid("from_string_exact", git_oid_from_string(&o, buf, len == 40 ? GIT_OID_SHA1 : GIT_OID_SHA256), &o);
		}
	}
	show_oid("from_raw", git_oid_from_raw(&o, (const unsigned char *)"0123456789abcdefghijklmnopqrstuvwxyz", GIT_OID_SHA1), &o);
	show_oid("from_raw256", git_oid_from_raw(&o, (const unsigned char *)"0123456789abcdefghijklmnopqrstuvwxyz", GIT_OID_SHA256), &o);
	memset(&o, 0x11, sizeof(o));
	show_oid("from_raw_bad", git_oid_from_raw(&o, (const unsigned char *)"x", 7), &o);
	show_oid("from_prefix_null", git_oid_from_prefix(&o, NULL, 3, GIT_OID_SHA1), &o);
	show_oid("from_prefix_nullout", git_oid_from_prefix(NULL, "abc", 3, GIT_OID_SHA1), &o);
	show_oid("from_string_bad_type", git_oid_from_string(&o, "abc", 9), &o);
}

static void random_oid(git_oid *o, int type)
{
	size_t i;
	memset(o, 0, sizeof(*o));
	o->type = (unsigned char)type;
	for (i = 0; i < (type == GIT_OID_SHA256 ? 32u : 20u); i++)
		o->id[i] = (unsigned char)(rnd() % 4 == 0 ? 0 : rnd());
}

static void test_oid_format(void)
{
	char buf[80], *s;
	git_oid o, p;
	int i, rc;
	size_t n;

	for (i = 0; i < 500; i++) {
		random_oid(&o, i % 5 == 4 ? (int)(rnd() % 4) : (i % 2) + 1);
		memset(buf, 'z', sizeof(buf));
		rc = git_oid_fmt(buf, &o);
		printf("fmt rc=%d %.*s|%c\n", rc, 70, buf, buf[70]);
		if (rc < 0) show_err("fmt", rc);
		n = rnd() % 70;
		memset(buf, 'z', sizeof(buf));
		rc = git_oid_nfmt(buf, n, &o);
		printf("nfmt n=%zu rc=%d ", n, rc);
		{ size_t j; for (j = 0; j < 72; j++) printf("%02x", (unsigned char)buf[j]); }
		printf("\n");
		if (rc < 0) show_err("nfmt", rc);
		memset(buf, 'z', sizeof(buf));
		rc = git_oid_pathfmt(buf, &o);
		printf("pathfmt rc=%d %.*s\n", rc, 70, buf);
		if (rc < 0) show_err("pathfmt", rc);
		memset(buf, 'z', sizeof(buf));
		s = git_oid_tostr(buf, n, &o);
		printf("tostr n=%zu [%s] same=%d\n", n, s, s == buf);
		if (git_error_last()->klass) show_err("tostr", 0);
		s = git_oid_tostr_s(&o);
		printf("tostr_s [%s]\n", s ? s : "(null)");
		if (git_error_last()->klass) show_err("tostr_s", 0);
		s = git_oid_allocfmt(&o);
		printf("allocfmt [%s]\n", s ? s : "(null)");
		git__free(s);

		random_oid(&p, (i % 2) + 1);
		if (rnd() % 3 == 0) { git_oid_cpy(&p, &o); p.id[rnd() % 20] ^= (unsigned char)(1u << (rnd() % 8)); }
		n = rnd() % 70;
		printf("cmp=%d equal=%d ncmp(%zu)=%d zero=%d\n", git_oid_cmp(&o, &p), git_oid_equal(&o, &p),
			n, git_oid_ncmp(&o, &p, n), git_oid_is_zero(&o));
		if (o.type == GIT_OID_SHA1 || o.type == GIT_OID_SHA256) {
			git_oid_fmt(buf, &o);
			buf[git_oid_hexsize(o.type)] = '\0';
			n = rnd() % (git_oid_hexsize(o.type) + 1);
			if (rnd() % 4 == 0) buf[rnd() % (n + 1)] = hexchars[rnd() % (sizeof(hexchars) - 1)];
			buf[n] = '\0';
			printf("strcmp [%s] %d streq %d\n", buf, git_oid_strcmp(&o, buf), git_oid_streq(&o, buf));
			if (rnd() % 2) buf[0] = (char)(buf[0] == 'f' ? '0' : buf[0] + 1);
			printf("strcmp2 [%s] %d\n", buf, git_oid_strcmp(&o, buf));
		}
	}
	printf("tostr null out [%s]\n", git_oid_tostr(NULL, 5, &o));
	printf("tostr zero [%s]\n", git_oid_tostr(buf, 0, &o));
	printf("tostr null oid [%s]\n", git_oid_tostr(buf, 10, NULL));
	memset(buf, 'z', 10);
	printf("nfmt null rc=%d %02x%02x\n", git_oid_nfmt(buf, 10, NULL), buf[0], buf[9]);
	printf("empty blob %s tree %s\n", git_oid_tostr_s(&git_oid__empty_blob_sha1), git_oid_tostr(buf, 41, &git_oid__empty_tree_sha1));
}

/* The shortener keeps pointers into the strings it was given (its contract:
 * they stay alive and unchanged), so each id gets its own buffer. */
static char shorten_bufs[40000][41];

static void test_shorten(void)
{
	git_oid_shorten *os;
	char *buf;
	int i, j, rc = 0;

	for (j = 0; j < 4; j++) {
		os = git_oid_shorten_new((size_t)j * 3);
		for (i = 0; i < 3000 + j * 2000; i++) {
			git_oid o;
			buf = shorten_bufs[i];
			random_oid(&o, GIT_OID_SHA1);
			git_oid_fmt(buf, &o);
			buf[40] = '\0';
			/* (an invalid digit in a stored tail is C's undefined
			 * behaviour, DEFECTS.md: not tested here) */
			if (j == 3 && i % 50 == 0)
				buf[0] = 'x';
			rc = git_oid_shorten_add(os, buf);
			if (rc < 0) { show_err("shorten_add", rc); }
			else if (i % 100 == 0) printf("shorten %d %d\n", i, rc);
		}
		printf("shorten null %d\n", git_oid_shorten_add(os, NULL));
		git_oid_shorten_free(os);
	}
	/* the set fills up */
	os = git_oid_shorten_new(0);
	for (i = 0; i < 40000; i++) {
		git_oid o;
		buf = shorten_bufs[i];
		random_oid(&o, GIT_OID_SHA1);
		git_oid_fmt(buf, &o);
		buf[40] = '\0';
		if ((rc = git_oid_shorten_add(os, buf)) < 0)
			break;
	}
	printf("full at %d rc=%d\n", i, rc);
	show_err("full", rc);
	printf("again %d\n", git_oid_shorten_add(os, "abc"));
	show_err("again", -1);
	git_oid_shorten_free(os);
	/* duplicates, and prefixes of one another */
	os = git_oid_shorten_new(1);
	printf("dup %d %d %d\n", git_oid_shorten_add(os, "1234567890123456789012345678901234567890"),
		git_oid_shorten_add(os, "1234567890123456789012345678901234567890"),
		git_oid_shorten_add(os, "1234567890123456789012345678901234567891"));
	git_oid_shorten_free(os);
	printf("new big %p\n", (void *)git_oid_shorten_new((size_t)1 << 40));
	show_err("new big", 0);
}

static void test_oidarray(void)
{
	git_array_oid_t arr = GIT_ARRAY_INIT;
	git_oidarray out;
	git_oid ids[10];
	size_t i;

	for (i = 0; i < 10; i++)
		random_oid(&ids[i], GIT_OID_SHA1);
	for (i = 0; i < 40; i++)
		printf("add %d size=%zu asize=%zu\n", git_oidarray__add(&arr, &ids[rnd() % 10]), arr.size, arr.asize);
	for (i = 0; i < 6; i++)
		printf("remove %d size=%zu\n", git_oidarray__remove(&arr, &ids[rnd() % 10]), arr.size);
	git_oidarray__from_array(&out, &arr);
	git_oidarray__reverse(&out);
	for (i = 0; i < out.count; i++)
		printf("rev %s\n", git_oid_tostr_s(&out.ids[i]));
	git_oidarray__to_array(&arr, &out);
	printf("to_array size=%zu asize=%zu\n", arr.size, arr.asize);
	git_oidarray_dispose(&out);
}

static void test_utf8(void)
{
	char buf[16];
	int i, j;

	for (i = 0; i < 3000; i++) {
		size_t len = rnd() % 8;
		uint32_t cp = 0xdead;
		int rc;
		for (j = 0; j < (int)len; j++) {
			unsigned r = rnd() % 6;
			buf[j] = (char)(r == 0 ? rnd() : r == 1 ? 0xc0 + rnd() % 0x20 : r == 2 ? 0xe0 + rnd() % 0x10 :
				r == 3 ? 0xf0 + rnd() % 0x10 : r == 4 ? 0x80 + rnd() % 0x40 : 0x20 + rnd() % 0x60);
		}
		buf[len] = '\0';
		printf("utf8 len=%zu ", len);
		for (j = 0; j < (int)len; j++) printf("%02x", (unsigned char)buf[j]);
		rc = len ? git_utf8_iterate(&cp, buf, len) : 0;
		printf(" it=%d cp=%x", rc, cp);
		cp = 0xdead;
		rc = git_utf8_iterate(&cp, buf, 0);
		printf(" it0=%d cp=%x chars=%zu valid=%zu\n", rc, cp,
			git_utf8_char_length(buf, len), git_utf8_valid_buf_length(buf, len));
	}
}

static void test_varint(void)
{
	unsigned char buf[16];
	int i, j;

	for (i = 0; i < 2000; i++) {
		uintmax_t v = ((uintmax_t)rnd() << 32 | rnd()) >> (rnd() % 64);
		size_t len = 99;
		int n = git_encode_varint(buf, rnd() % 12, v);
		int m = git_encode_varint(NULL, 0, v);
		uintmax_t d;
		if (n < 0) n = git_encode_varint(buf, sizeof(buf), v);
		d = git_decode_varint(buf, &len);
		printf("varint %ju n=%d m=%d d=%ju len=%zu\n", v, n, m, d, len);
	}
	for (i = 0; i < 500; i++) {
		size_t len = 99;
		uintmax_t d;
		for (j = 0; j < 16; j++) buf[j] = (unsigned char)(rnd() % 3 ? (0x80 | rnd()) : rnd());
		buf[15] = 0;
		d = git_decode_varint(buf, &len);
		printf("decode %ju len=%zu\n", d, len);
	}
}

static void test_pool(void)
{
	git_pool p;
	char *first, *ptr;
	int i;

	git_pool_init(&p, 1);
	first = git_pool_malloc(&p, 1);
	for (i = 0; i < 3000; i++) {
		size_t n = rnd() % (i % 100 == 0 ? 9000 : 100);
		switch (rnd() % 4) {
		case 0: ptr = git_pool_malloc(&p, n); break;
		case 1: ptr = git_pool_mallocz(&p, n); break;
		case 2: ptr = git_pool_strndup(&p, "abcdefghijklmnopqrstuvwxyz", n % 27); break;
		default: ptr = git_pool_strcat(&p, (rnd() % 2) ? "xyz" : NULL, &"0123456789"[rnd() % 10]); break;
		}
		if (i % 10 == 0)
			printf("pool %d n=%zu pages=%u in=%d first_in=%d align=%d\n", i, n, git_pool__open_pages(&p),
				git_pool__ptr_in_pool(&p, ptr), git_pool__ptr_in_pool(&p, first), (int)((uintptr_t)ptr % 8));
	}
	git_pool_clear(&p);
	printf("cleared %u\n", git_pool__open_pages(&p));
	git_pool_init(&p, 24);
	ptr = git_pool_malloc(&p, 3);
	printf("item24 %td pages=%u\n", (char *)git_pool_malloc(&p, 1) - ptr, git_pool__open_pages(&p));
	printf("strdup on item pool %p\n", (void *)git_pool_strdup(&p, "x"));
	show_err("strdup item", 0);
	printf("init 0 %d\n", git_pool_init(&p, 0));
	show_err("init 0", -1);
	git_pool_clear(&p);
}

static int cmp_int(const void *a, const void *b)
{
	return *(const int *)a - *(const int *)b;
}

static void test_pqueue(void)
{
	int vals[500];
	git_pqueue pq;
	size_t i, j;

	for (i = 0; i < 500; i++)
		vals[i] = (int)(rnd() % 1000);
	for (j = 0; j < 3; j++) {
		git_pqueue_init(&pq, j == 1 ? GIT_PQUEUE_FIXED_SIZE : 0, j == 1 ? 37 : 0, j == 2 ? NULL : cmp_int);
		for (i = 0; i < 500; i++) {
			git_pqueue_insert(&pq, &vals[i]);
			if (i % 7 == 0) {
				int *v = git_pqueue_pop(&pq);
				printf("pq%zu pop %d size=%zu\n", j, v ? *v : -1, git_pqueue_size(&pq));
			}
		}
		for (i = 0; i < git_pqueue_size(&pq); i++)
			printf("pq%zu [%zu]=%d\n", j, i, *(int *)git_pqueue_get(&pq, i));
		while (git_pqueue_size(&pq) > 0)
			printf("pq%zu drain %d\n", j, *(int *)git_pqueue_pop(&pq));
		printf("pq%zu empty pop %p\n", j, git_pqueue_pop(&pq));
		git_pqueue_free(&pq);
	}
}

static void show_ctx(const char *what, int rc, git_parse_ctx *c)
{
	printf("%s rc=%d off=%td remain_len=%zu line_len=%zu line_num=%zu remain_is_content=%d\n", what, rc,
		c->line - c->content, c->remain_len, c->line_len, c->line_num, c->remain == c->content);
}

static void test_parse(void)
{
	static const char *texts[] = {
		"key = value\n\t  [section \"sub\"]\n42 0x1f -7 999999999999999999999\n",
		"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 refs/heads/master\nnot an oid\n\n\n",
		"", "\n", "no newline", "   \n  \t\r\nend",
	};
	git_parse_ctx c;
	size_t t;
	int i, rc;

	for (t = 0; t < ARRAY_SIZE(texts); t++) {
		git_parse_ctx_init(&c, texts[t], strlen(texts[t]));
		show_ctx("init", 0, &c);
		for (i = 0; i < 40; i++) {
			int64_t n = -1;
			git_oid o;
			char ch = '?';
			switch (rnd() % 8) {
			case 0: git_parse_advance_line(&c); show_ctx("line", 0, &c); break;
			case 1: rc = git_parse_advance_ws(&c); show_ctx("ws", rc, &c); break;
			case 2: rc = git_parse_advance_nl(&c); show_ctx("nl", rc, &c); break;
			case 3: rc = git_parse_advance_digit(&n, &c, (rnd() % 2) ? 10 : 16); show_ctx("digit", rc, &c);
				printf("  n=%lld\n", (long long)n); if (rc < 0 && git_error_last()->klass) show_err("digit", rc); break;
			case 4: memset(&o, 0, sizeof(o)); rc = git_parse_advance_oid(&o, &c, GIT_OID_SHA1); show_ctx("oid", rc, &c);
				show_oid("  oid", 0, &o); if (git_error_last()->klass) show_err("oid", rc); break;
			case 5: rc = git_parse_peek(&ch, &c, (int)(rnd() % 2)); show_ctx("peek", rc, &c); printf("  ch=%d\n", ch); break;
			case 6: rc = git_parse_advance_expected(&c, "key", 3); show_ctx("expected", rc, &c); break;
			default: if (c.line_len) git_parse_advance_chars(&c, rnd() % c.line_len); show_ctx("chars", 0, &c); break;
			}
		}
	}
	git_parse_ctx_init(&c, NULL, 5);
	show_ctx("init null", 0, &c);
	printf("  content=[%s]\n", c.content);
	git_parse_ctx_clear(&c);
	printf("clear content=[%s] line=%p len=%zu num=%zu\n", c.content, (void *)c.line, c.line_len, c.line_num);
	git_parse_ctx_init(&c, "abc", 3);
	printf("oid bad type %d\n", git_parse_advance_oid(NULL, &c, 0));
	show_err("oid bad type", -1);
}

static void test_strlist(void)
{
	const char *in[] = { "core.bare", "user.name", "remote.origin.url", "" };
	char **out = NULL;
	printf("copy %d\n", git_strlist_copy(&out, in, 4));
	printf("prefix %d %d %d %d\n", git_strlist_contains_prefix((const char **)out, 4, "user", 4),
		git_strlist_contains_prefix((const char **)out, 4, "users", 5),
		git_strlist_contains_prefix((const char **)out, 4, "", 0),
		git_strlist_contains_prefix((const char **)out, 4, "remote.origin.urlx", 17));
	printf("key %d %d %d %d\n", git_strlist_contains_key((const char **)out, 4, "user.name=x", '='),
		git_strlist_contains_key((const char **)out, 4, "user.nam=x", '='),
		git_strlist_contains_key((const char **)out, 4, "core.bare", '='),
		git_strlist_contains_key((const char **)out, 4, "=x", '='));
	git_strlist_free(out, 4);
	printf("copy_with_null %d\n", git_strlist_copy_with_null(&out, in, 3));
	printf("null at 3: %d\n", out[3] == NULL);
	git_strlist_free_with_null(out);
}

int main(void)
{
	git_libgit2_init();
	test_oid_parse();
	test_oid_format();
	test_shorten();
	test_oidarray();
	test_utf8();
	test_varint();
	test_pool();
	test_pqueue();
	test_parse();
	test_strlist();
	git_libgit2_shutdown();
	return 0;
}
