#include "wrapper.h"

// Static wrappers

int git__is_sizet__extern(int64_t p) { return git__is_sizet(p); }
int git__is_ssizet__extern(size_t p) { return git__is_ssizet(p); }
int git__is_uint16__extern(size_t p) { return git__is_uint16(p); }
int git__is_uint32__extern(size_t p) { return git__is_uint32(p); }
int git__is_ulong__extern(int64_t p) { return git__is_ulong(p); }
int git__is_int__extern(int64_t p) { return git__is_int(p); }
void * git__malloc__extern(size_t len) { return git__malloc(len); }
void * git__realloc__extern(void *ptr, size_t size) { return git__realloc(ptr, size); }
void git__free__extern(void *ptr) { git__free(ptr); }
