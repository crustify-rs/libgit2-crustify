#include "wrapper.h"

// Static wrappers

int git_threads_global_init__extern(void) { return git_threads_global_init(); }
void git_atomic32_set__extern(git_atomic32 *a, int val) { git_atomic32_set(a, val); }
int git_atomic32_inc__extern(git_atomic32 *a) { return git_atomic32_inc(a); }
int git_atomic32_add__extern(git_atomic32 *a, int32_t addend) { return git_atomic32_add(a, addend); }
int git_atomic32_dec__extern(git_atomic32 *a) { return git_atomic32_dec(a); }
int git_atomic32_get__extern(git_atomic32 *a) { return git_atomic32_get(a); }
void * git_atomic__compare_and_swap__extern(void **ptr, void *oldval, void *newval) { return git_atomic__compare_and_swap(ptr, oldval, newval); }
void * git_atomic__swap__extern(void **ptr, void *newval) { return git_atomic__swap(ptr, newval); }
void * git_atomic__load__extern(void **ptr) { return git_atomic__load(ptr); }
int64_t git_atomic64_add__extern(git_atomic64 *a, int64_t addend) { return git_atomic64_add(a, addend); }
void git_atomic64_set__extern(git_atomic64 *a, int64_t val) { git_atomic64_set(a, val); }
int64_t git_atomic64_get__extern(git_atomic64 *a) { return git_atomic64_get(a); }
void * git__malloc__extern(size_t len) { return git__malloc(len); }
void * git__realloc__extern(void *ptr, size_t size) { return git__realloc(ptr, size); }
void git__free__extern(void *ptr) { git__free(ptr); }
