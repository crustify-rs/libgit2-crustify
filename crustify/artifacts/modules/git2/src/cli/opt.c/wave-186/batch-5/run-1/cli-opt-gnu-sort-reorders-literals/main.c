/* sort_gnu_style runs at the first bare argument even after a literal "--" was parsed,
 * and moves the arguments after it that look like options ahead of the others: the
 * literal arguments change order, so "-- a -d" gives the first argument "-d". */
#include <stdio.h>
#include "opt.h"
int main(void) {
	int d = 0;
	char *first = NULL, *second = NULL;
	cli_opt_spec specs[] = {
		{ CLI_OPT_TYPE_BOOL,    "debug", 'd', &d },
		{ CLI_OPT_TYPE_LITERAL },
		{ CLI_OPT_TYPE_ARG,     "first",  0,  &first },
		{ CLI_OPT_TYPE_ARG,     "second", 0,  &second },
		{ 0 }
	};
	char *args[] = { "--", "a", "-d" };
	cli_opt opt;
	cli_opt_status_t s = cli_opt_parse(&opt, specs, args, 3, CLI_OPT_PARSE_FORCE_GNU);
	printf("status %d d %d first %s second %s\n", s, d, first, second);
	return 0;
}
