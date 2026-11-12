#include "wrapper.h"

// Static wrappers

void * git__malloc__extern(size_t len) { return git__malloc(len); }
void git__free__extern(void *ptr) { git__free(ptr); }
