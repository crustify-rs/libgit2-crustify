/* cli_opt_parse keeps the specs given in alloca(args_len + 1) entries; a bundle of
 * short options gives one spec per option, so -ab alone writes three entries into two. */
#include <stdio.h>
#include "opt.h"
int main(void) {
	int a = 0, b = 0;
	cli_opt_spec specs[] = {
		{ CLI_OPT_TYPE_SWITCH, NULL, 'a', &a, 1 },
		{ CLI_OPT_TYPE_SWITCH, NULL, 'b', &b, 1 },
		{ 0 }
	};
	char *args[] = { "-ab" };
	cli_opt opt;
	cli_opt_status_t s = cli_opt_parse(&opt, specs, args, 1, CLI_OPT_PARSE_DEFAULT);
	printf("status %d a %d b %d\n", s, a, b);
	return 0;
}
