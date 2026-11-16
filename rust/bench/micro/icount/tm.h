#include <time.h>
static double cpu_now(void){struct timespec t;clock_gettime(CLOCK_PROCESS_CPUTIME_ID,&t);return t.tv_sec*1e9+t.tv_nsec;}
#define TIME_MIN(label, reps, n, body) do { double best=1e30; for (int _r=0;_r<(reps);_r++){ double t0=cpu_now(); for (long _i=0;_i<(n);_i++){ body; } double d=cpu_now()-t0; if(d<best)best=d;} printf("%-28s %8.1f ns/op\n", label, best/(n)); } while(0)
