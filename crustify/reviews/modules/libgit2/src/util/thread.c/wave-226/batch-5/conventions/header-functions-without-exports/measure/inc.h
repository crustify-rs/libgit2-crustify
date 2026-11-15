typedef struct { volatile int val; } git_atomic32;
static inline int inline_inc(git_atomic32 *a) { return __atomic_add_fetch(&a->val, 1, __ATOMIC_SEQ_CST); }
static inline int inline_dec(git_atomic32 *a) { return __atomic_sub_fetch(&a->val, 1, __ATOMIC_SEQ_CST); }
int extern_inc(git_atomic32 *a);
int extern_dec(git_atomic32 *a);
