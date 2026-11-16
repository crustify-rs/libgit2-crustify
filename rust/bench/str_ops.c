/* Per-operation cost of the hot git_str calls (best of 5 runs of 5M calls).
 * Build with ../difftest/link.sh str_ops.c out; run out/str_ops_c and out/str_ops_rust. */
#include "git2_util.h"
#include "git2/global.h"
#include <time.h>
static double now(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1e3+t.tv_nsec/1e6;}
#define N 5000000
int main(int argc,char**argv){
	git_str s=GIT_STR_INIT; int i,r; double t,best;
	git_libgit2_init();
	const char *w = argc > 5 ? argv[5] : "word";
#define BENCH(name, setup, body) best=1e9; for(r=0;r<5;r++){ setup; t=now(); for(i=0;i<N;i++){ body; } t=now()-t; if(t<best)best=t; } printf("%-10s %6.2f ns\n", name, best*1e6/N);
	BENCH("put", git_str_clear(&s), git_str_put(&s, w, 4))
	BENCH("putc", git_str_clear(&s), git_str_putc(&s, 'x'))
	BENCH("puts", git_str_clear(&s), git_str_puts(&s, w))
	BENCH("joinpath", , git_str_joinpath(&s, "refs/heads", w))
	BENCH("join_self", git_str_sets(&s,"a"), if((i&15)==0) git_str_sets(&s,"root"); git_str_joinpath(&s, s.ptr, w))
	BENCH("join_n3", , git_str_join_n(&s, '/', 3, "refs", "heads", w))
	BENCH("sets", , git_str_sets(&s, "refs/heads/master"))
	printf("%zu\n", s.size);
	return 0;
}
