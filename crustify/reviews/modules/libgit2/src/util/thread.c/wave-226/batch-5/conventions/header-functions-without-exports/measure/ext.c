#include "inc.h"
int extern_inc(git_atomic32 *a) { return __atomic_add_fetch(&a->val, 1, __ATOMIC_SEQ_CST); }
int extern_dec(git_atomic32 *a) { return __atomic_sub_fetch(&a->val, 1, __ATOMIC_SEQ_CST); }
