#include "wrapper.h"

// Static wrappers

void * git__malloc__extern(size_t len) { return git__malloc(len); }
void * git__realloc__extern(void *ptr, size_t size) { return git__realloc(ptr, size); }
void git__free__extern(void *ptr) { git__free(ptr); }
