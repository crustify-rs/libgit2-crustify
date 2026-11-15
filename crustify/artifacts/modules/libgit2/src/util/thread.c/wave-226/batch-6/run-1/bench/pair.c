/*
 * The cost of an increment-and-decrement pair of one git_atomic32, as GIT_REFCOUNT_INC and
 * GIT_REFCOUNT_DEC make it: built against thread.h's `static inline` bodies, the C inlines
 * each builtin; built against the switch's prototypes, it calls the module's exports.
 */
#include "git2_util.h"
#include <time.h>

static double now(void)
{
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec + t.tv_nsec * 1e-9;
}

int main(int argc, char **argv)
{
	long n = argc > 1 ? atol(argv[1]) : 100000000L, i, s = 0;
	int r;
	git_atomic32 a = {0};

	for (r = 0; r < 5; r++) {
		double t0 = now(), t1;
		for (i = 0; i < n; i++) {
			s += git_atomic32_inc(&a);
			s += git_atomic32_dec(&a);
		}
		t1 = now();
		printf("%.2f\n", (t1 - t0) / n * 1e9);
	}
	return (int)(s & 1) + git_atomic32_get(&a);
}
