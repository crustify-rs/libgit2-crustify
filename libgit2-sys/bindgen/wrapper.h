/*
 * bindgen input: libgit2's installed public headers, deprecated ones excluded, and the
 * internal headers that declare the internal items the allowlist names, which campaign mode
 * finds on the include path libgit2's own TUs compile with.
 */
#include <git2.h>
#include <git2/sys/alloc.h>
#include <git2/sys/commit.h>
#include <git2/sys/commit_graph.h>
#include <git2/sys/config.h>
#include <git2/sys/credential.h>
#include <git2/sys/diff.h>
#include <git2/sys/email.h>
#include <git2/sys/errors.h>
#include <git2/sys/filter.h>
#include <git2/sys/hashsig.h>
#include <git2/sys/index.h>
#include <git2/sys/mempack.h>
#include <git2/sys/merge.h>
#include <git2/sys/midx.h>
#include <git2/sys/odb_backend.h>
#include <git2/sys/openssl.h>
#include <git2/sys/path.h>
#include <git2/sys/refdb_backend.h>
#include <git2/sys/refs.h>
#include <git2/sys/remote.h>
#include <git2/sys/repository.h>
#include <git2/sys/stream.h>
#include <git2/sys/transport.h>
<<<<<<< refs/heads/crustify/waves/wave-169
/*
 * Internal headers whose items the batches bind; each needs the include directories the
 * variant's C build compiles it with (build.rs, campaign mode).
 */
#include "reftable-basics.h"
=======
/* Internal headers. */
#include "basics.h" /* deps/reftable */

>>>>>>> d21c78c70aa8564ad8021d6dfba9670e232b7905
#include "shims.h"
