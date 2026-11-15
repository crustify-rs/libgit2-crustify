import sys, re
root = sys.argv[1]
p = root + "/src/util/thread.h"
s = open(p).read()
start = s.index('/*\n * Atomically sets the contents of *a to be val.\n */\nGIT_INLINE(void) git_atomic32_set')
end = s.index('#else\n\n#define git_threads_global_init\tgit__noop')
proto = '''extern void git_atomic32_set(git_atomic32 *a, int val);
extern int git_atomic32_inc(git_atomic32 *a);
extern int git_atomic32_add(git_atomic32 *a, int32_t addend);
extern int git_atomic32_dec(git_atomic32 *a);
extern int git_atomic32_get(git_atomic32 *a);

extern void *git_atomic__compare_and_swap(
	void * volatile *ptr, void *oldval, void *newval);

extern volatile void *git_atomic__swap(
	void * volatile *ptr, void *newval);

extern volatile void *git_atomic__load(void * volatile *ptr);

#ifdef GIT_ARCH_64
extern int64_t git_atomic64_add(git_atomic64 *a, int64_t addend);
extern void git_atomic64_set(git_atomic64 *a, int64_t val);
extern int64_t git_atomic64_get(git_atomic64 *a);
#endif

'''
s = s[:start] + proto + s[end:]
open(p, "w").write(s)
q = root + "/src/util/unix/pthread.h"
t = open(q).read()
old = "GIT_INLINE(int) git_threads_global_init(void) { return 0; }"
assert old in t
open(q, "w").write(t.replace(old, "extern int git_threads_global_init(void);"))
