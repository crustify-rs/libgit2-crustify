/*
 * Extra differential test of the revision walker, from the l-revwalk
 * review (round 1, /work/reviews/l-revwalk-r1-repro/rw_extra.c), with the
 * callback-replacement case of its re2/rw_replace_cb.c.
 *
 * - the sequence of hide-callback invocations (ids hashed, count), so
 *   that a different number or order of calls shows
 * - re-entrant hide callbacks that call back into the walk (C defines it)
 * - pushes while a walk is in progress, duplicate pushes, sorting changes
 *   between pushes, insert_by_date + uninteresting, merge-heavy ranges
 * - C consumers of the walk: merge bases, ahead/behind, descendant_of,
 *   describe, packbuilder_insert_walk
 * - a hide callback that replaces itself (git_revwalk_add_hide_cb from
 *   inside it) and invalidates its old payload
 *
 * difftest-link: libgit2_tests
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "git2.h"
#include "revwalk.h"

static git_repository *repo;

#ifdef GIT_RUST
# define USER_INPUT(w) ((w)->user_input)
#else
struct c_revwalk {
	git_repository *repo;
	git_odb *odb;
	git_revwalk_oidmap commits;
	git_pool commit_pool;
	git_commit_list *iterator_topo;
	git_commit_list *iterator_rand;
	git_commit_list *iterator_reverse;
	git_pqueue iterator_time;
	int (*get_next)(git_commit_list_node **, git_revwalk *);
	int (*enqueue)(git_revwalk *, git_commit_list_node *);
	unsigned walking:1, first_parent: 1, did_hide: 1, did_push: 1, limited: 1;
	unsigned int sorting;
	git_commit_list *user_input;
};
# define USER_INPUT(w) (((struct c_revwalk *)(w))->user_input)
#endif

static uint64_t fnv(uint64_t h, const unsigned char *p, size_t n)
{
	while (n--) {
		h ^= *p++;
		h *= 0x100000001b3ULL;
	}
	return h;
}

static void err(const char *what, int e)
{
	const git_error *er = git_error_last();
	printf("  %s -> %d", what, e);
	if (e < 0)
		printf(" [%d] %s", er ? er->klass : -1, er && er->message ? er->message : "(null)");
	printf("\n");
}

static void drain(const char *label, git_revwalk *w, int limit)
{
	git_oid id;
	uint64_t h = 0xcbf29ce484222325ULL;
	size_t n = 0;
	int e = 0;
	while ((limit < 0 || (int)n < limit) && (e = git_revwalk_next(&id, w)) == 0) {
		h = fnv(h, id.id, 20);
		n++;
	}
	printf("%s: n=%zu h=%016llx end=%d\n", label, n, (unsigned long long)h, e);
	if (e < 0 && e != GIT_ITEROVER)
		err("  next", e);
}

static git_oid rev(const char *s)
{
	git_object *o;
	git_oid id;
	memset(&id, 0, sizeof(id));
	if (git_revparse_single(&o, repo, s) == 0) {
		git_object *c;
		if (git_object_peel(&c, o, GIT_OBJECT_COMMIT) == 0) {
			git_oid_cpy(&id, git_object_id(c));
			git_object_free(c);
		}
		git_object_free(o);
	} else {
		printf("cannot parse %s\n", s);
	}
	return id;
}

/* -- callback sequence ------------------------------------------------ */

typedef struct {
	uint64_t h;
	size_t n;
	unsigned char thr;
	int mod;
	git_revwalk *walk;
	int reenter; /* 0 none, 1 hide, 2 push, 3 sorting topo, 4 reset, 5 sorting time */
	git_oid target;
	int fired;
} cbstate;

static int logging_cb(const git_oid *id, void *payload)
{
	cbstate *s = payload;
	s->h = fnv(s->h, id->id, 20);
	s->n++;
	if (s->reenter && !s->fired && s->n == 50) {
		int e = 0;
		s->fired = 1;
		switch (s->reenter) {
		case 1: e = git_revwalk_hide(s->walk, &s->target); break;
		case 2: e = git_revwalk_push(s->walk, &s->target); break;
		case 3: e = git_revwalk_sorting(s->walk, GIT_SORT_TOPOLOGICAL); break;
		case 4: e = git_revwalk_reset(s->walk); break;
		case 5: e = git_revwalk_sorting(s->walk, GIT_SORT_TIME); break;
		case 6: git_revwalk_simplify_first_parent(s->walk); break;
		}
		printf("  reentered %d -> %d\n", s->reenter, e);
	}
	if (s->mod && (s->n % s->mod) == 0)
		return 1;
	return id->id[0] < s->thr;
}

static void cb_walk(const char *name, const char *push, const char *hide, unsigned sort,
		    int fp, unsigned char thr, int mod, int reenter, const char *target, int reset_first)
{
	git_revwalk *w;
	cbstate s = { 0xcbf29ce484222325ULL, 0, thr, mod, NULL, reenter, { { 0 } }, 0 };
	char label[256];
	git_oid p = rev(push);

	if (git_revwalk_new(&w, repo) < 0)
		return;
	s.walk = w;
	if (target)
		s.target = rev(target);
	git_revwalk_add_hide_cb(w, logging_cb, &s);
	if (reset_first) {
		/* a reset leaves the callback but clears `limited` */
		git_revwalk_reset(w);
	}
	git_revwalk_sorting(w, sort);
	if (fp)
		git_revwalk_simplify_first_parent(w);
	git_revwalk_push(w, &p);
	if (hide) {
		git_oid hh = rev(hide);
		git_revwalk_hide(w, &hh);
	}
	snprintf(label, sizeof(label), "cb %s sort=%u fp=%d thr=%u mod=%d re=%d rf=%d", name, sort,
		 fp, thr, mod, reenter, reset_first);
	drain(label, w, -1);
	printf("  cb calls=%zu h=%016llx\n", s.n, (unsigned long long)s.h);
	/* the walk again, after its reset */
	s.n = 0; s.h = 0xcbf29ce484222325ULL;
	git_revwalk_push(w, &p);
	drain("  again", w, -1);
	printf("  cb calls=%zu h=%016llx\n", s.n, (unsigned long long)s.h);
	git_revwalk_free(w);
}

static void callbacks(void)
{
	unsigned sort;
	int fp;
	for (sort = 0; sort < 8; sort++)
		for (fp = 0; fp < 2; fp++) {
			cb_walk("a", "v2.41.0", "v2.40.0", sort, fp, 0x20, 0, 0, NULL, 0);
			cb_walk("b", "v2.41.0", "v2.39.0", sort, fp, 0x00, 7, 0, NULL, 0);
			cb_walk("c", "v2.41.0", "v2.40.0", sort, fp, 0x08, 0, 0, NULL, 1);
			cb_walk("d", "v1.6.0", NULL, sort, fp, 0x30, 13, 0, NULL, 0);
		}
	for (sort = 0; sort < 8; sort += 1) {
		int re;
		for (re = 1; re <= 6; re++) {
			cb_walk("re", "v2.41.0", "v2.40.0", sort, 0, 0x08, 0, re, "v2.40.0~200", 0);
			cb_walk("re-unlimited", "v2.41.0", "v2.40.0", sort, 0, 0x08, 0, re, "v2.39.0", 1);
		}
	}
}

/* -- odd pushes and hides ---------------------------------------------- */

static void odd(void)
{
	git_revwalk *w;
	git_oid a = rev("v2.41.0"), b = rev("v2.40.0"), c = rev("v2.41.0^2"), d = rev("v2.30.0"),
		m1 = rev("v2.41.0^1"), root = rev("e83c5163316f89bfbde7d9ab23ca2e25604af290");
	unsigned sort;

	for (sort = 0; sort < 8; sort++) {
		char l[128];
		git_revwalk_new(&w, repo);
		git_revwalk_sorting(w, sort);
		/* duplicate pushes, push of a hidden commit's ancestor, side branch */
		git_revwalk_push(w, &a);
		git_revwalk_push(w, &a);
		git_revwalk_push(w, &c);
		git_revwalk_hide(w, &b);
		git_revwalk_push(w, &d);
		git_revwalk_hide(w, &m1);
		snprintf(l, sizeof(l), "odd1 sort=%u", sort);
		drain(l, w, -1);

		/* partial walk, then a push mid-walk, then go on */
		git_revwalk_sorting(w, sort);
		git_revwalk_push(w, &b);
		git_revwalk_hide(w, &d);
		snprintf(l, sizeof(l), "odd2a sort=%u", sort);
		drain(l, w, 30);
		git_revwalk_push(w, &a);
		git_revwalk_hide(w, &c);
		snprintf(l, sizeof(l), "odd2b sort=%u", sort);
		drain(l, w, -1);
		/* the extra push/hide was in user_input when reset: gone */
		snprintf(l, sizeof(l), "odd2c sort=%u", sort);
		drain(l, w, -1);

		/* sorting set, then cleared: limited stays */
		git_revwalk_sorting(w, sort);
		git_revwalk_sorting(w, GIT_SORT_NONE);
		git_revwalk_push(w, &b);
		git_revwalk_hide(w, &d);
		snprintf(l, sizeof(l), "odd3 sort=%u", sort);
		drain(l, w, -1);

		/* first parent mid-way */
		git_revwalk_sorting(w, sort);
		git_revwalk_push(w, &a);
		git_revwalk_hide(w, &d);
		drain("  odd4a", w, 5);
		git_revwalk_simplify_first_parent(w);
		snprintf(l, sizeof(l), "odd4b sort=%u", sort);
		drain(l, w, -1);

		/* hide the root, push old history */
		git_revwalk_sorting(w, sort);
		git_revwalk_push(w, &root);
		git_revwalk_hide(w, &root);
		drain("  odd5", w, -1);
		git_revwalk_sorting(w, sort);
		git_revwalk_push_range(w, "v1.0.0..v0.99");
		git_revwalk_push(w, &root);
		snprintf(l, sizeof(l), "odd6 sort=%u", sort);
		drain(l, w, -1);
		git_revwalk_free(w);
	}

	/* insert_by_date with uninteresting */
	{
		git_revwalk__push_options o = GIT_REVWALK__PUSH_OPTIONS_INIT;
		git_commit_list *l;
		uint64_t h = 0xcbf29ce484222325ULL;
		size_t n = 0;
		git_revwalk_new(&w, repo);
		o.insert_by_date = 1;
		err("glob tags by date", git_revwalk__push_glob(w, "tags/v2.*", &o));
		o.uninteresting = 1;
		err("glob tags/v1 by date hide", git_revwalk__push_glob(w, "tags/v1.*", &o));
		o.uninteresting = 0;
		err("push_commit v2.0 by date", git_revwalk__push_commit(w, &d, &o));
		for (l = USER_INPUT(w); l; l = l->next) {
			h = fnv(h, l->item->oid.id, 20);
			n++;
		}
		printf("user_input n=%zu h=%016llx\n", n, (unsigned long long)h);
		drain("by-date none", w, -1);
		git_revwalk_free(w);
	}

	/* merge-heavy ranges in every mode */
	{
		const char *ranges[] = { "v2.41.0^2~5..v2.41.0", "v2.20.0..v2.20.0^2", "v2.38.0^..v2.38.0",
					 "v1.7.0..v1.7.0^2", "v2.0.0~50..v2.0.0", "master~300..master" };
		size_t i;
		for (i = 0; i < sizeof(ranges) / sizeof(ranges[0]); i++)
			for (sort = 0; sort < 8; sort++) {
				int fp;
				for (fp = 0; fp < 2; fp++) {
					char l[128];
					git_revwalk_new(&w, repo);
					git_revwalk_sorting(w, sort);
					if (fp)
						git_revwalk_simplify_first_parent(w);
					err(ranges[i], git_revwalk_push_range(w, ranges[i]));
					git_revwalk_hide_glob(w, "tags/v2.3*");
					snprintf(l, sizeof(l), "mrange %s sort=%u fp=%d", ranges[i], sort, fp);
					drain(l, w, -1);
					git_revwalk_free(w);
				}
			}
	}
}

/* -- C consumers ------------------------------------------------------- */

static void consumers(void)
{
	const char *pairs[][2] = { { "v2.41.0", "v2.40.0" }, { "v2.41.0^2", "v2.41.0^1" },
				   { "v1.7.0", "v2.0.0" }, { "master", "v2.20.0^2" },
				   { "v2.30.0^2", "v2.31.0^2" } };
	size_t i;
	for (i = 0; i < sizeof(pairs) / sizeof(pairs[0]); i++) {
		git_oid a = rev(pairs[i][0]), b = rev(pairs[i][1]), base, ids[3];
		git_oidarray arr;
		size_t ahead = 0, behind = 0;
		char hex[41];
		int e;
		e = git_merge_base(&base, repo, &a, &b);
		git_oid_tostr(hex, sizeof(hex), &base);
		printf("merge_base %s %s -> %d %s\n", pairs[i][0], pairs[i][1], e, e ? "-" : hex);
		ids[0] = a; ids[1] = b; ids[2] = rev("v2.10.0^2");
		e = git_merge_bases_many(&arr, repo, 3, ids);
		if (e == 0) {
			uint64_t h = 0xcbf29ce484222325ULL;
			size_t k;
			for (k = 0; k < arr.count; k++)
				h = fnv(h, arr.ids[k].id, 20);
			printf("  bases_many n=%zu h=%016llx\n", arr.count, (unsigned long long)h);
			git_oidarray_dispose(&arr);
		} else
			err("  bases_many", e);
		e = git_graph_ahead_behind(&ahead, &behind, repo, &a, &b);
		printf("  ahead_behind -> %d %zu %zu\n", e, ahead, behind);
		printf("  descendant_of -> %d %d\n", git_graph_descendant_of(repo, &a, &b),
		       git_graph_descendant_of(repo, &b, &a));
	}
	{
		git_object *o;
		git_describe_result *res;
		git_buf buf = { 0 };
		const char *revs[] = { "v2.41.0~3", "v2.30.0^2~2", "master", "v1.0.0~1" };
		git_describe_options dopts = GIT_DESCRIBE_OPTIONS_INIT;
		git_describe_format_options fopts = GIT_DESCRIBE_FORMAT_OPTIONS_INIT;
		dopts.describe_strategy = GIT_DESCRIBE_TAGS;
		for (i = 0; i < 4; i++) {
			if (git_revparse_single(&o, repo, revs[i]) < 0)
				continue;
			if (git_describe_commit(&res, o, &dopts) == 0) {
				git_describe_format(&buf, res, &fopts);
				printf("describe %s -> %s\n", revs[i], buf.ptr);
				git_buf_dispose(&buf);
				git_describe_result_free(res);
			} else
				err("describe", -1);
			git_object_free(o);
		}
	}
	{
		git_packbuilder *pb;
		git_revwalk *w;
		git_revwalk_new(&w, repo);
		git_revwalk_sorting(w, GIT_SORT_TIME);
		git_revwalk_push_range(w, "v2.40.0~3..v2.40.0");
		if (git_packbuilder_new(&pb, repo) == 0) {
			err("insert_walk", git_packbuilder_insert_walk(pb, w));
			printf("  objects=%zu\n", git_packbuilder_object_count(pb));
			git_packbuilder_free(pb);
		}
		git_revwalk_free(w);
	}
}

/* -- a callback that replaces itself ----------------------------------- */

static git_revwalk *rw_w;
static int a_calls, b_calls, a_after;

static int cb_b(const git_oid *id, void *p)
{
	(void)p;
	b_calls++;
	return id->id[0] < 0x04;
}

static int cb_a(const git_oid *id, void *p)
{
	(void)id;
	if (*(int *)p == 0xdead) /* used after it was replaced */
		a_after++;
	a_calls++;
	if (a_calls == 50) {
		git_revwalk_add_hide_cb(rw_w, cb_b, NULL);
		*(int *)p = 0xdead; /* a C user would free its state here */
	}
	return 0;
}

static void replace_cb(void)
{
	unsigned sort;
	for (sort = 0; sort < 8; sort++) {
		int st = 0;
		char l[64];
		a_calls = b_calls = a_after = 0;
		git_revwalk_new(&rw_w, repo);
		git_revwalk_sorting(rw_w, sort);
		git_revwalk_add_hide_cb(rw_w, cb_a, &st);
		git_revwalk_push_range(rw_w, "v2.40.0..v2.41.0");
		snprintf(l, sizeof(l), "replace_cb sort=%u", sort);
		drain(l, rw_w, -1);
		printf("  a_calls=%d a_after_replace=%d b_calls=%d\n", a_calls, a_after, b_calls);
		git_revwalk_free(rw_w);
	}
}

int main(void)
{
	const char *data = getenv("BENCH_DATA");
	char path[4096];
	git_libgit2_init();
	snprintf(path, sizeof(path), "%s/git.git", data ? data : "/work/bench-data");
	if (git_repository_open(&repo, path) < 0) {
		printf("cannot open %s\n", path);
		return 1;
	}
	callbacks();
	odd();
	consumers();
	replace_cb();
	git_repository_free(repo);
	git_libgit2_shutdown();
	return 0;
}
