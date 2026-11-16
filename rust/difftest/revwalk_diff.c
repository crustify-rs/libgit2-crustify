/*
 * Differential test of the revision walker: the same walks of git.git and
 * of the test repositories, against the C reference and the Rust port.
 * Every sorting mode (all 8 combinations), with and without first-parent,
 * a hide callback and a pathspec, over many pushes, hides, globs and
 * ranges; walker reuse (reset, walking to the end, sorting mid-walk); the
 * internal push options (insert_by_date for fetch negotiation, from_glob);
 * and the errors (codes and messages).
 *
 * For each walk it prints the count, a hash of the whole ordered sequence,
 * and its first and last ids, so any difference of order shows.
 *
 * The data: $BENCH_DATA/git.git (default /work/bench-data/git.git), and
 * tests/resources/testrepo.git (run from the top of the source tree).
 *
 * difftest-link: libgit2_tests
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "git2.h"
#include "git2/sys/hashsig.h"
#include "revwalk.h"

static git_repository *repo;
static const char *repo_name;

/*
 * `walk->user_input`. The driver is compiled against this tree's
 * revwalk.h, whose struct is the Rust port's prefix; the C reference has
 * the original struct, mirrored here up to `user_input`.
 */
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

static void out_err(const char *what, int e)
{
	const git_error *err = git_error_last();
	printf("  %s -> %d", what, e);
	if (e < 0)
		printf(" [%d] %s", err ? err->klass : -1, err && err->message ? err->message : "(null)");
	printf("\n");
}

static uint64_t fnv(uint64_t h, const unsigned char *p, size_t n)
{
	while (n--) {
		h ^= *p++;
		h *= 0x100000001b3ULL;
	}
	return h;
}

/* drain a walk: count, hash of the sequence, first and last ids */
static void drain(const char *label, git_revwalk *w, int limit)
{
	git_oid id, first, last;
	uint64_t h = 0xcbf29ce484222325ULL;
	size_t n = 0;
	int e;
	char a[GIT_OID_MAX_HEXSIZE + 1] = "-", b[GIT_OID_MAX_HEXSIZE + 1] = "-";

	while ((limit < 0 || (int)n < limit) && (e = git_revwalk_next(&id, w)) == 0) {
		if (n == 0)
			first = id;
		last = id;
		h = fnv(h, id.id, GIT_OID_SHA1_SIZE);
		n++;
	}
	if (limit >= 0 && (int)n == limit)
		e = 0;
	if (n) {
		git_oid_tostr(a, sizeof(a), &first);
		git_oid_tostr(b, sizeof(b), &last);
	}
	printf("%s/%s: n=%zu h=%016llx first=%s last=%s end=%d", repo_name, label, n,
	       (unsigned long long)h, a, b, e);
	if (e < 0 && e != GIT_ITEROVER) {
		const git_error *err = git_error_last();
		printf(" err=[%d] %s", err ? err->klass : -1, err && err->message ? err->message : "(null)");
	}
	printf("\n");
}

typedef struct {
	const char *push[4];
	const char *hide[4];
	const char *range;
} spec;

static int apply(git_revwalk *w, const char *s, int hide)
{
	if (!strncmp(s, "glob:", 5))
		return hide ? git_revwalk_hide_glob(w, s + 5) : git_revwalk_push_glob(w, s + 5);
	if (!strcmp(s, "HEAD"))
		return hide ? git_revwalk_hide_head(w) : git_revwalk_push_head(w);
	if (!strncmp(s, "oid:", 4)) {
		git_oid id;
		git_object *o;
		int e;
		if ((e = git_revparse_single(&o, repo, s + 4)) < 0)
			return e;
		git_oid_cpy(&id, git_object_id(o));
		git_object_free(o);
		return hide ? git_revwalk_hide(w, &id) : git_revwalk_push(w, &id);
	}
	return hide ? git_revwalk_hide_ref(w, s) : git_revwalk_push_ref(w, s);
}

/* hide every commit whose first id byte is below the payload's value */
static int hide_low(const git_oid *id, void *payload)
{
	return id->id[0] < *(unsigned char *)payload;
}

static void walk_spec(const char *name, const spec *sp, unsigned sort, int first_parent,
		      int hidecb, int limit)
{
	git_revwalk *w;
	char label[512];
	unsigned char threshold = 0x18;
	int i, e;

	if ((e = git_revwalk_new(&w, repo)) < 0) {
		out_err("new", e);
		return;
	}
	git_revwalk_sorting(w, sort);
	if (first_parent)
		git_revwalk_simplify_first_parent(w);
	if (hidecb)
		git_revwalk_add_hide_cb(w, hide_low, &threshold);
	for (i = 0; i < 4 && sp->push[i]; i++)
		if ((e = apply(w, sp->push[i], 0)) < 0)
			out_err(sp->push[i], e);
	for (i = 0; i < 4 && sp->hide[i]; i++)
		if ((e = apply(w, sp->hide[i], 1)) < 0)
			out_err(sp->hide[i], e);
	if (sp->range && (e = git_revwalk_push_range(w, sp->range)) < 0)
		out_err(sp->range, e);
	snprintf(label, sizeof(label), "%s sort=%u fp=%d cb=%d", name, sort, first_parent, hidecb);
	drain(label, w, limit);
	git_revwalk_free(w);
}

static void all_modes(const char *name, const spec *sp)
{
	unsigned sort;
	for (sort = 0; sort < 8; sort++) {
		walk_spec(name, sp, sort, 0, 0, -1);
		walk_spec(name, sp, sort, 1, 0, -1);
		walk_spec(name, sp, sort, 0, 1, -1);
	}
}

static void reuse(void)
{
	git_revwalk *w;
	int v, e;
	char ref[64];

	if (git_revwalk_new(&w, repo) < 0)
		return;
	/* walk to the end, then again without pushing: GIT_ITEROVER */
	git_revwalk_sorting(w, GIT_SORT_TIME | GIT_SORT_REVERSE);
	git_revwalk_push_ref(w, "refs/tags/v2.40.0");
	git_revwalk_hide_ref(w, "refs/tags/v2.39.0");
	drain("reuse/first", w, -1);
	drain("reuse/again", w, -1);
	/* the sorting is reset by the end of a walk */
	git_revwalk_push_ref(w, "refs/tags/v2.41.0");
	git_revwalk_hide_ref(w, "refs/tags/v2.40.0");
	drain("reuse/after-end", w, -1);
	/* a partial walk, then a sorting change (resets) */
	git_revwalk_push_ref(w, "refs/tags/v2.42.0");
	drain("reuse/partial", w, 10);
	git_revwalk_sorting(w, GIT_SORT_TOPOLOGICAL);
	git_revwalk_push_ref(w, "refs/tags/v2.42.0");
	git_revwalk_hide_ref(w, "refs/tags/v2.41.0");
	drain("reuse/after-sorting", w, -1);
	/* partial walk, explicit reset, hide first then push */
	git_revwalk_push_ref(w, "refs/tags/v2.43.0");
	drain("reuse/partial2", w, 7);
	git_revwalk_reset(w);
	git_revwalk_hide_ref(w, "refs/tags/v2.43.0");
	git_revwalk_push_ref(w, "refs/tags/v2.43.0");
	git_revwalk_push_ref(w, "refs/tags/v2.44.0");
	drain("reuse/hide-then-push", w, -1);
	/* only hides */
	git_revwalk_hide_ref(w, "refs/tags/v2.44.0");
	drain("reuse/only-hides", w, -1);
	for (v = 20; v < 47; v += 3) {
		snprintf(ref, sizeof(ref), "refs/tags/v2.%d.0", v);
		if ((e = git_revwalk_push_ref(w, ref)) < 0)
			out_err(ref, e);
		snprintf(ref, sizeof(ref), "refs/tags/v2.%d.0", v - 4);
		git_revwalk_hide_ref(w, ref);
		git_revwalk_sorting(w, v % 2 ? GIT_SORT_TIME : GIT_SORT_NONE);
		snprintf(ref, sizeof(ref), "reuse/loop-%d", v);
		drain(ref, w, -1);
		git_revwalk_reset(w);
	}
	git_revwalk_free(w);
}

static void internal_opts(void)
{
	git_revwalk *w;
	git_revwalk__push_options opts = GIT_REVWALK__PUSH_OPTIONS_INIT;
	git_commit_list *l;
	uint64_t h = 0xcbf29ce484222325ULL;
	size_t n = 0;
	int e;

	/* fetch negotiation: every ref, inserted by date */
	if (git_revwalk_new(&w, repo) < 0)
		return;
	opts.insert_by_date = 1;
	e = git_revwalk__push_glob(w, "refs/*", &opts);
	out_err("push_glob insert_by_date", e);
	for (l = USER_INPUT(w); l; l = l->next) {
		h = fnv(h, l->item->oid.id, GIT_OID_SHA1_SIZE);
		n++;
	}
	printf("%s/user_input by date: n=%zu h=%016llx\n", repo_name, n, (unsigned long long)h);
	git_revwalk_sorting(w, GIT_SORT_TIME);
	drain("negotiation", w, 3000);
	git_revwalk_free(w);

	if (git_revwalk_new(&w, repo) < 0)
		return;
	memset(&opts, 0, sizeof(opts));
	opts.from_glob = 1;
	e = git_revwalk__push_ref(w, "refs/heads/does-not-exist", &opts);
	out_err("push_ref from_glob missing", e);
	opts.from_glob = 0;
	e = git_revwalk__push_ref(w, "refs/heads/does-not-exist", &opts);
	out_err("push_ref missing", e);
	e = git_revwalk__push_glob(w, "tags", NULL);
	out_err("push_glob tags NULL opts", e);
	n = 0;
	h = 0xcbf29ce484222325ULL;
	for (l = USER_INPUT(w); l; l = l->next) {
		h = fnv(h, l->item->oid.id, GIT_OID_SHA1_SIZE);
		n++;
	}
	printf("%s/user_input tags: n=%zu h=%016llx\n", repo_name, n, (unsigned long long)h);
	drain("tags", w, -1);
	git_revwalk_free(w);
}

static void errors(void)
{
	git_revwalk *w;
	git_oid id;
	git_object *o;
	int e;

	if (git_revwalk_new(&w, repo) < 0)
		return;
	e = git_revwalk_next(&id, w);
	out_err("next without push", e);
	out_err("push_range a...b", git_revwalk_push_range(w, "HEAD~3...HEAD"));
	out_err("push_range single", git_revwalk_push_range(w, "HEAD"));
	out_err("push_range bad", git_revwalk_push_range(w, "nope..HEAD"));
	out_err("push_range NULL", git_revwalk_push_range(w, NULL));
	out_err("push_ref bad", git_revwalk_push_ref(w, "refs/heads/nope"));
	out_err("push_ref invalid name", git_revwalk_push_ref(w, "refs/heads/a..b"));
	out_err("push_glob nothing", git_revwalk_push_glob(w, "nothing/here"));
	out_err("push NULL", git_revwalk_push(w, NULL));
	out_err("push_ref NULL", git_revwalk_push_ref(w, NULL));
	out_err("push_glob NULL", git_revwalk_push_glob(w, NULL));
	out_err("sorting NULL", git_revwalk_sorting(NULL, 0));
	out_err("reset NULL", git_revwalk_reset(NULL));
	out_err("next NULL", git_revwalk_next(NULL, w));
	out_err("hide_cb NULL", git_revwalk_add_hide_cb(NULL, NULL, NULL));
	out_err("pathspec NULL", git_revwalk_pathspec(NULL, NULL));
	printf("  repository NULL -> %p\n", (void *)git_revwalk_repository(NULL));
	printf("  repository -> same %d\n", git_revwalk_repository(w) == repo);
	/* a tree and a blob are not committish */
	if (git_revparse_single(&o, repo, "HEAD^{tree}") == 0) {
		out_err("push tree", git_revwalk_push(w, git_object_id(o)));
		out_err("hide tree", git_revwalk_hide(w, git_object_id(o)));
		git_object_free(o);
	}
	memset(&id, 0x42, sizeof(id.id));
	out_err("push missing", git_revwalk_push(w, &id));
	drain("after errors", w, -1);
	git_revwalk_free(w);

	{
		git_revwalk *w2 = (git_revwalk *)1;
		out_err("new NULL repo", git_revwalk_new(&w2, NULL));
	}
}

static void pathspecs(void)
{
	const char *sets[][3] = {
		{ "README", NULL, NULL }, { "*.txt", NULL, NULL }, { "new.txt", "branch_file.txt", NULL },
		{ "a/*", NULL, NULL }, { "nope", NULL, NULL }, { "README", "*.txt", NULL },
		{ "Makefile", NULL, NULL }, { "Documentation/*", NULL, NULL }, { "t/t0000-basic.sh", "*.c", NULL },
	};
	unsigned sort;
	size_t i, n;

	for (i = 0; i < sizeof(sets) / sizeof(sets[0]); i++) {
		char *paths[3];
		git_strarray arr;
		git_pathspec *ps;
		for (n = 0; n < 3 && sets[i][n]; n++)
			paths[n] = (char *)sets[i][n];
		arr.strings = paths;
		arr.count = n;
		if (git_pathspec_new(&ps, &arr) < 0)
			continue;
		for (sort = 0; sort < 8; sort += 3) {
			git_revwalk *w;
			char label[256];
			if (git_revwalk_new(&w, repo) < 0)
				continue;
			git_revwalk_pathspec(w, ps);
			git_revwalk_sorting(w, sort);
			git_revwalk_push_head(w);
			if (!strcmp(repo_name, "git.git"))
				git_revwalk_hide_ref(w, "refs/tags/v2.44.0");
			snprintf(label, sizeof(label), "pathspec[%s%s%s] sort=%u", sets[i][0],
				 sets[i][1] ? "," : "", sets[i][1] ? sets[i][1] : "", sort);
			drain(label, w, -1);
			git_revwalk_free(w);
		}
		git_pathspec_free(ps);
	}
}

static void git_git(void)
{
	static const spec specs[] = {
		{ { "refs/heads/master" }, { 0 }, 0 },
		{ { "glob:refs/*" }, { 0 }, 0 },
		{ { "glob:tags" }, { 0 }, 0 },
		{ { "glob:heads" }, { "refs/tags/v2.30.0" }, 0 },
		{ { "refs/heads/master" }, { "refs/tags/v2.45.0" }, 0 },
		{ { "refs/heads/master" }, { "glob:tags/v2.4*" }, 0 },
		{ { "refs/tags/v2.47.0", "refs/tags/v2.46.0", "refs/tags/v1.9.0" }, { "refs/tags/v2.20.0" }, 0 },
		{ { "oid:v2.40.0", "oid:v2.30.0~3" }, { "oid:v2.35.0^2", "oid:v2.20.0" }, 0 },
		{ { "oid:v2.44.0^{tree}" }, { 0 }, 0 },
		{ { "refs/tags/v2.40.0" }, { "refs/tags/v2.40.0" }, 0 },
		{ { 0 }, { 0 }, "v2.30.0..v2.47.0" },
		{ { 0 }, { 0 }, "v2.46.0..master" },
		{ { 0 }, { 0 }, "master..v2.46.0" },
		{ { 0 }, { 0 }, "v1.0.0..v1.5.0" },
		{ { 0 }, { 0 }, "v2.45.0^..v2.45.0" },
		{ { "refs/tags/v2.10.0" }, { "refs/tags/v2.9.0" }, "v2.12.0..v2.13.0" },
		{ { "HEAD" }, { "refs/tags/v2.0.0" }, 0 },
	};
	size_t i;
	int v;

	for (i = 0; i < sizeof(specs) / sizeof(specs[0]); i++) {
		char name[32];
		snprintf(name, sizeof(name), "spec%zu", i);
		all_modes(name, &specs[i]);
	}
	/* many short ranges, every mode */
	for (v = 0; v < 47; v++) {
		spec sp = { { 0 }, { 0 }, 0 };
		char range[64], name[64];
		unsigned sort;
		snprintf(range, sizeof(range), "v2.%d.0..v2.%d.0", v, v + 1);
		sp.range = range;
		for (sort = 0; sort < 8; sort++) {
			snprintf(name, sizeof(name), "range-%s", range);
			walk_spec(name, &sp, sort, v % 3 == 0, v % 5 == 0, -1);
		}
	}
	for (v = 1; v < 30; v++) {
		spec sp = { { 0 }, { 0 }, 0 };
		char range[64], name[64];
		snprintf(range, sizeof(range), "master~%d..master~%d", v * 40, v * 7);
		sp.range = range;
		snprintf(name, sizeof(name), "range-%s", range);
		walk_spec(name, &sp, (unsigned)v % 8, 0, 0, -1);
		walk_spec(name, &sp, GIT_SORT_TOPOLOGICAL | GIT_SORT_TIME, 1, 0, -1);
	}
}

static void small_repo(void)
{
	static const spec specs[] = {
		{ { "HEAD" }, { 0 }, 0 },
		{ { "glob:heads" }, { 0 }, 0 },
		{ { "glob:*" }, { 0 }, 0 },
		{ { "glob:tags" }, { 0 }, 0 },
		{ { "HEAD" }, { "refs/heads/br2" }, 0 },
		{ { "glob:heads" }, { "HEAD" }, 0 },
		{ { "oid:a4a7dce" }, { "oid:c47800c" }, 0 },
		{ { 0 }, { 0 }, "9fd738e..a65fedf" },
		{ { 0 }, { 0 }, "be3563a..HEAD" },
	};
	size_t i;
	for (i = 0; i < sizeof(specs) / sizeof(specs[0]); i++) {
		char name[32];
		snprintf(name, sizeof(name), "spec%zu", i);
		all_modes(name, &specs[i]);
	}
}

int main(void)
{
	const char *data = getenv("BENCH_DATA");
	char path[4096];

	git_libgit2_init();

	repo_name = "testrepo";
	if (git_repository_open(&repo, "tests/resources/testrepo.git") < 0) {
		printf("cannot open testrepo.git\n");
		return 1;
	}
	small_repo();
	pathspecs();
	errors();
	internal_opts();
	git_repository_free(repo);

	snprintf(path, sizeof(path), "%s/git.git", data ? data : "/work/bench-data");
	repo_name = "git.git";
	if (git_repository_open(&repo, path) < 0) {
		printf("cannot open %s\n", path);
		return 1;
	}
	git_git();
	reuse();
	internal_opts();
	errors();
	pathspecs();
	git_repository_free(repo);

	git_libgit2_shutdown();
	return 0;
}
