/* sort_gnu_style moves the parser to a last option of kind VALUE that has no value, "so
 * that we fail on it", whether or not its value is optional: an optional one does not
 * fail, and the bare arguments before it are never parsed. "file -S" leaves the required
 * argument unset and reports it missing. */
#include <stdio.h>
#include "opt.h"
int main(void) {
	char *file = NULL, *key = NULL;
	cli_opt_spec specs[] = {
		{ CLI_OPT_TYPE_VALUE, "sign", 'S', &key, 0, CLI_OPT_USAGE_VALUE_OPTIONAL },
		{ CLI_OPT_TYPE_ARG,   "file", 0,   &file, 0, CLI_OPT_USAGE_REQUIRED },
		{ 0 }
	};
	char *args[] = { "file", "-S" };
	cli_opt opt;
	cli_opt_status_t s = cli_opt_parse(&opt, specs, args, 2, CLI_OPT_PARSE_FORCE_GNU);
	printf("status %d spec %s file %s\n", s, opt.spec ? opt.spec->name : "(none)", file);
	return 0;
}
