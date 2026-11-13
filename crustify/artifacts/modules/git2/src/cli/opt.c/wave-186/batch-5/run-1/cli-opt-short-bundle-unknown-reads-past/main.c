/* parse_short leaves in_short set after an unknown option inside a bundle: the next
 * argument is then read from in_short characters in, past its end when it is shorter.
 * The arguments are heap copies, as ASan bounds them. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "opt.h"
static int show(cli_opt *opt, void *data) {
	(void)data;
	printf("status %d spec %p arg %s\n", opt->status, (void *)opt->spec, opt->arg);
	return 0;
}
int main(void) {
	int d = 0;
	cli_opt_spec specs[] = {
		{ CLI_OPT_TYPE_BOOL, "debug", 'd', &d },
		{ CLI_OPT_TYPE_ARG,  "file",  0,   NULL },
		{ 0 }
	};
	char *args[] = { strdup("-dx"), strdup("y") };
	int rc = cli_opt_foreach(specs, args, 2, CLI_OPT_PARSE_DEFAULT, show, NULL);
	printf("rc %d d %d\n", rc, d);
	return 0;
}
