/*
 * The C khash of hashmap.h on a fixed sequence of operations, printing the
 * map's state and iteration order. The Rust test `tests/u1_hashmap_c.rs`
 * runs the same sequence on `GitHashMap` and compares the output with this
 * program's (built against the C reference with link.sh).
 */

#include "git2_util.h"
#include "hashmap.h"
#include "hashmap_str.h"
#include "git2/global.h"

#define off64_hash(key) (uint32_t)((key)>>33^(key)^(key)<<11)
#define off64_equal(a, b) ((a) == (b))

GIT_HASHMAP_SETUP(test_offmap, int64_t, int64_t, off64_hash, off64_equal);
GIT_HASHMAP_STR_SETUP(test_strmap, int64_t);

static uint64_t seed = 99;
static uint32_t rnd(void)
{
	seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
	return (uint32_t)(seed >> 33);
}

static char keys[4096][16];

int main(void)
{
	test_offmap m = GIT_HASHMAP_INIT;
	test_strmap s = GIT_HASHMAP_INIT;
	git_hashset_str set = GIT_HASHSET_INIT;
	git_hashmap_iter_t it;
	int64_t k, v;
	const char *sk;
	int i, r;

	git_libgit2_init();
	for (i = 0; i < 4096; i++)
		snprintf(keys[i], sizeof(keys[i]), "k%u\xe9%d", rnd() % 100000, i);

	for (i = 0; i < 30000; i++) {
		uint32_t op = rnd() % 10;
		k = (int64_t)(rnd() % 3000);
		k *= (rnd() % 2) ? 1 : 0x100000001LL;
		if (op < 5) r = test_offmap_put(&m, k, i);
		else if (op < 8) r = test_offmap_remove(&m, k);
		else if (op < 9) { v = -1; r = test_offmap_get(&v, &m, k); r = r * 1000 + (int)(v % 1000); }
		else if (i % 997 == 0) { test_offmap_clear(&m); r = 7; }
		else r = test_offmap_contains(&m, k);
		if (i % 50 == 0)
			printf("off %d op=%u r=%d size=%u buckets=%u occ=%u\n", i, op, r, m.size, m.n_buckets, m.n_occupied);
		if (i % 1000 == 999) {
			it = 0;
			printf("order");
			while (test_offmap_iterate(&it, &k, &v, &m) == 0)
				printf(" %lld=%lld", (long long)k, (long long)v);
			printf("\n");
		}

		sk = keys[rnd() % 4096];
		op = rnd() % 10;
		if (op < 6) r = test_strmap_put(&s, sk, i);
		else if (op < 9) r = test_strmap_remove(&s, sk);
		else r = test_strmap_contains(&s, sk);
		if (i % 50 == 0)
			printf("str %d op=%u r=%d size=%u buckets=%u occ=%u\n", i, op, r, s.size, s.n_buckets, s.n_occupied);
		if (i % 3 == 0) {
			r = git_hashset_str_add(&set, sk);
			if (i % 999 == 0) printf("set size=%u buckets=%u\n", set.size, set.n_buckets);
		}
	}
	it = 0;
	printf("strorder");
	while (test_strmap_iterate(&it, &sk, &v, &s) == 0)
		printf(" %s=%lld", sk, (long long)v);
	printf("\n");
	printf("setorder");
	for (it = 0; it < set.n_buckets; it++)
		if (!GIT_HASHMAP_IS_EITHER(set.flags, it))
			printf(" %s", set.keys[it]);
	printf("\n");
	r = test_offmap__resize(&m, 5);
	printf("resize5 r=%d buckets=%u\n", r, m.n_buckets);
	r = test_offmap__resize(&m, 100000);
	printf("resize100000 r=%d buckets=%u size=%u\n", r, m.n_buckets, m.size);
	it = 0;
	printf("order");
	while (test_offmap_iterate(&it, &k, &v, &m) == 0)
		printf(" %lld=%lld", (long long)k, (long long)v);
	printf("\n");
	test_offmap_dispose(&m);
	test_strmap_dispose(&s);
	git_hashset_str_dispose(&set);
	return 0;
}
