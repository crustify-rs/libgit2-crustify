/*
 * Differential test of src/util/date.c (git_date_offset_parse,
 * git_date_parse, git_date_rfc2822_fmt): every line printed must be the
 * same when linked against the C reference and against the Rust port.
 *
 * "now" is fixed: this program defines time(), which both the C and the
 * Rust (libc::time) resolve to.  Each input is parsed at several "now"s and
 * in several time zones (TZ), from a hand-written list (git's and libgit2's
 * test inputs, odd ones) and from random token sequences.
 */

#include "git2_util.h"
#include "git2/global.h"
#include "date.h"

#include <time.h>

static time_t fixed_now = 1234567890;

time_t time(time_t *t)
{
	if (t)
		*t = fixed_now;
	return fixed_now;
}

static unsigned long seed = 4242;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void show(const char *in)
{
	git_time_t out = -777;
	int offset = -777, rc;
	const char *p;

	rc = git_date_offset_parse(&out, &offset, in);
	printf("[");
	for (p = in; *p; p++)
		printf((*p >= 32 && *p < 127 && *p != '\\') ? "%c" : "\\x%02x", (unsigned char)*p);
	printf("] rc=%d out=%lld off=%d\n", rc, (long long)out, offset);
}

static const char *fixed[] = {
	"", " ", "\n", "@", "@0 +0000", "@1234567890 +0100", "@1234567890 -0130\n",
	"@1234567890 +01000", "@9 +0000", "@8 + 100", "@8 +-100", "@18446744073709551615 +0000",
	"@12x +0000", "0 +0000", "1234567890", "1234567890 +0200", "99999999", "100000000",
	"99999999999999999999", "18446744073709551616 +0000", "-1", "+5",
	"2008-12-05", "2008-12-05 10:11:12", "2008-12-05 10:11:12 +0200", "2008-12-05T10:11:12Z",
	"2008.12.05", "05.12.2008", "12/05/2008", "12/05/08", "5/12/08", "2008-05-12",
	"2008-31-12", "2038-1-1", "2039-1-1", "2099-12-31 23:59:59", "2100-01-01",
	"1969-12-31", "1970-1-1 01:00:00+03", "1970-01-01 00:00:00 +0000",
	"Thu, 5 Dec 2008 10:11:12 -0130", "Thu Dec 5 10:11:12 2008 +0100",
	"Fri, 13 Feb 2009 23:31:30 +0000", "Mon Jan 1 00:00:00 2024", "July 17, 2003",
	"2003-7-17 08:23i+03", "17 July 2003", "Jul 17 2003 08:23 PM", "Jul 17 2003 12:00 AM",
	"10:11:12", "23:59:60", "24:00:00", "25:00", "12:61", "12:30 pm", "12:30 am", "1 pm",
	"10:11:12 PST", "10:11:12 PDT", "10:11:12 UTC", "10:11:12 Z", "10:11:12 Zulu",
	"10:11:12 CEST", "10:11:12 MESZ", "10:11:12 IDLW", "10:11:12 NZDT", "10:11:12 XYZ",
	"10:11:12 +05:30", "10:11:12 -05:30", "10:11:12 +5:30", "10:11:12 +0530",
	"10:11:12 +053", "10:11:12 +05", "10:11:12 +24", "10:11:12 +2400", "10:11:12 +1260",
	"10:11:12 +05: 3", "10:11:12 +05:-3", "10:11:12 +05:+3", "10:11:12 +05:x",
	"10:11:12 +4294967296", "10:11:12 +18446744073709551615",
	"1400", "1401", "1999", "0100", "0000", "2008 1400", "-1400 2008-12-05",
	"4294967297-01-01", "2008-4294967297-01", "2008-01-99999999999999999999",
	"1.2.3", "31.12.99", "12.31.99", "99.12.31", "70.1.1", "71.1.1", "37.1.1", "38.1.1",
	"Dec 6, 1992", "Dec 06 1992", "Dec 0006 1992", "01 Apr 05", "Apr 05", "05", "5",
	"now", "NOW", "yesterday", "never", "noon", "midnight", "tea", "noon yesterday",
	"midnight today", "tea time", "nowhere", "PM", "AM", "3 pm", "3pm", "3.pm",
	"2 days ago", "2.days.ago", "3 weeks ago", "1 week", "last week", "last month",
	"last year", "one hour ago", "ten minutes ago", "eleven days ago", "5 seconds ago",
	"5 second ago", "5 sec ago", "1 year ago", "2 years ago", "13 months ago", "1 month",
	"100 months ago", "2147483647 months ago", "2147483647 days ago",
	"4294967295 days ago", "4294967296 days ago", "2147483648 weeks ago",
	"last friday", "friday", "3 fridays ago", "2147483647 fridays ago", "sunday",
	"Monday 2 weeks ago", "1 day ago 10:00", "Dec", "dec 2", "december 25", "Dec 2 2007",
	"Dec 02", "Dec 002", "5 Dec", "1992", "92", "07", "6", "40", "100",
	"2008-12-05 10:11:12.123", "2008-12-05 10:11:12 +02:00", "20070606", "2007 06 06",
	"10.11.12", "10/11/12", "10-11-12", "1:2", "1:2:3:4", "12:00:00 Jan 1 1970 UTC",
	"Jan 1 1970 00:00:00 -1200", "\x80\xff", "Ma\xe4rz", "abc 123 def", "NEITHER_INTEGER_NOR_DATETIME",
	"2008-12-05\n10:11:12", "  2008-12-05", "+0200", "-0200 2008-12-05 10:11:12",
	NULL
};

static const char *tokens[] = {
	"2008", "12", "5", "05", "1", "31", "32", "99", "70", "1969", "2100", "1234567890",
	"100000000", "20070606", "-", "/", ".", ":", " ", ",", "+", "+0200", "-0130", "+05:30",
	"+5", "-12", "+1400", "Jan", "February", "Mar", "dec", "Mon", "tuesday", "sun", "PST",
	"UTC", "Z", "CEST", "IDLE", "am", "PM", "noon", "midnight", "tea", "yesterday", "now",
	"never", "last", "one", "ten", "days", "day", "weeks", "hours", "minute", "seconds",
	"months", "years", "ago", "@", "\n", "0", "00", "007", "10:11:12", "23:59:60", "25:00",
	"12:61", "2008-12-05", "12/05/2008", "05.12.2008", "2008.12.05", "1.2.3",
	"99999999999999999999", "4294967297", "  -3", "xyz", "2147483647", "fridays", "x"
};

static void rfc2822(git_time_t t, int offset)
{
	git_str s = GIT_STR_INIT;
	int rc = git_date_rfc2822_fmt(&s, t, offset);
	printf("rfc2822 %lld %d rc=%d [%s]\n", (long long)t, offset, rc, s.ptr ? s.ptr : "");
	git_str_dispose(&s);
}

int main(void)
{
	static const char *zones[] = { "UTC", "America/New_York", "Europe/Berlin", "Asia/Kolkata", "Pacific/Chatham" };
	static const time_t nows[] = { 1234567890, 1700000000, 1710054000, 1, 4102444799 };
	size_t z, n, i, k;
	char buf[512];

	git_libgit2_init();

	for (z = 0; z < ARRAY_SIZE(zones); z++) {
		setenv("TZ", zones[z], 1);
		tzset();
		for (n = 0; n < ARRAY_SIZE(nows); n++) {
			fixed_now = nows[n];
			printf("== TZ=%s now=%lld\n", zones[z], (long long)fixed_now);
			for (i = 0; fixed[i]; i++)
				show(fixed[i]);
			seed = 4242 + n;
			for (i = 0; i < 3000; i++) {
				size_t len = 0, count = 1 + rnd() % 6;
				buf[0] = '\0';
				for (k = 0; k < count; k++) {
					const char *t = tokens[rnd() % ARRAY_SIZE(tokens)];
					const char *sep = (rnd() % 3) ? " " : "";
					if (len + strlen(t) + 2 >= sizeof(buf))
						break;
					strcat(buf, t);
					strcat(buf, sep);
					len = strlen(buf);
				}
				show(buf);
			}
		}
	}

	{
		static const git_time_t times[] = { 0, 1, -1, 1234567890, 1397031663, 951782400, 4102444800LL,
			-86400LL * 365 * 3000, 67768036191676799LL, 67768036191676800LL, INT64_MAX, INT64_MIN };
		static const int offsets[] = { 0, 60, -60, 90, -90, 120, -120, 330, -1, 59, -59, 1439, -1440, 6000, -6000, 2147483647, -2147483647 - 1 };
		for (i = 0; i < ARRAY_SIZE(times); i++)
			for (k = 0; k < ARRAY_SIZE(offsets); k++)
				rfc2822(times[i], offsets[k]);
	}

	git_libgit2_shutdown();
	return 0;
}
