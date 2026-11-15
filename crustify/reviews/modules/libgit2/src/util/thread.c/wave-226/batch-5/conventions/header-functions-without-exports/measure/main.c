#include <stdio.h>
#include <time.h>
#include <stdlib.h>
#include "inc.h"
static double now(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec+t.tv_nsec*1e-9;}
int main(int argc,char**argv){
  long n = argc>1?atol(argv[1]):200000000L; git_atomic32 a={0}; double t0,t1,t2; long s=0;
  for(int r=0;r<3;r++){
  t0=now(); for(long i=0;i<n;i++){ s+=inline_inc(&a); s+=inline_dec(&a);} t1=now();
  for(long i=0;i<n;i++){ s+=extern_inc(&a); s+=extern_dec(&a);} t2=now();
  printf("pair(inc+dec) inline %.2f ns, exported call %.2f ns (+%.1f%%)\n",(t1-t0)/n*1e9,(t2-t1)/n*1e9,((t2-t1)/(t1-t0)-1)*100);}
  return (int)(s&1);
}
