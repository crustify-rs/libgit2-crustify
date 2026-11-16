//! Port of `src/util/date.c`: git's date parser (`git_date_parse`,
//! `git_date_offset_parse`, "approxidate") and the RFC 2822 formatter.
//!
//! The parser is pure logic over the C library's time conversions, which it
//! reaches through a [`Clock`]: [`SystemClock`] calls `time`, `gmtime_r`,
//! `localtime_r` and `mktime` as the C did (so the local time zone, `TZ`,
//! and the normalizations of `mktime` are the C library's), and
//! [`UtcClock`] is a pure-Rust, UTC, fixed-"now" clock for deterministic
//! use (and Miri).
//!
//! The C's quirks are kept: C's `strtol`/`strtoul` (leading white space,
//! signs, overflow to `LONG_MAX`/`ULONG_MAX`), the narrowing of `long`s to
//! `int`s, the fields a failed `gmtime_r` leaves behind. Where the C
//! overflows a signed `int` (undefined behaviour), the Rust wraps
//! (rust/DEFECTS.md).

use core::ffi::c_int;

use super::ctype_compat::{isalnum, isalpha, isdigit, isspace, toupper};
use super::errors::{ErrorCode, Result};
use super::str::GitStr;

// ---------------------------------------------------------------------------
// Broken-down time and the clocks
// ---------------------------------------------------------------------------

/// `struct tm`, without `tm_zone` (which nothing here reads).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tm {
    /// `tm_sec`
    pub sec: i32,
    /// `tm_min`
    pub min: i32,
    /// `tm_hour`
    pub hour: i32,
    /// `tm_mday`
    pub mday: i32,
    /// `tm_mon`
    pub mon: i32,
    /// `tm_year`
    pub year: i32,
    /// `tm_wday`
    pub wday: i32,
    /// `tm_yday`
    pub yday: i32,
    /// `tm_isdst`
    pub isdst: i32,
    /// `tm_gmtoff`
    pub gmtoff: i64,
}

/// The time functions the parser uses, as the C library's.
pub trait Clock {
    /// `time(NULL)`: -1 on failure.
    fn time(&self) -> i64;
    /// `gmtime_r(&t, tm) != NULL`. On failure `tm` may be partly written,
    /// as the C library leaves it.
    fn gmtime(&self, t: i64, tm: &mut Tm) -> bool;
    /// `localtime_r(&t, tm) != NULL`, as `gmtime`.
    fn localtime(&self, t: i64, tm: &mut Tm) -> bool;
    /// `mktime(tm)`: the time of the local `tm`, which it normalizes; -1 on
    /// failure.
    fn mktime(&self, tm: &mut Tm) -> i64;
}

/// The C library's clock and local time zone.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Tm {
    fn to_libc(self) -> libc::tm {
        libc::tm {
            tm_sec: self.sec,
            tm_min: self.min,
            tm_hour: self.hour,
            tm_mday: self.mday,
            tm_mon: self.mon,
            tm_year: self.year,
            tm_wday: self.wday,
            tm_yday: self.yday,
            tm_isdst: self.isdst,
            tm_gmtoff: self.gmtoff,
            tm_zone: core::ptr::null(),
        }
    }

    fn set_from_libc(&mut self, t: &libc::tm) {
        *self = Tm {
            sec: t.tm_sec,
            min: t.tm_min,
            hour: t.tm_hour,
            mday: t.tm_mday,
            mon: t.tm_mon,
            year: t.tm_year,
            wday: t.tm_wday,
            yday: t.tm_yday,
            isdst: t.tm_isdst,
            gmtoff: t.tm_gmtoff,
        };
    }
}

impl Clock for SystemClock {
    fn time(&self) -> i64 {
        // SAFETY: `time(NULL)` has no precondition.
        unsafe { libc::time(core::ptr::null_mut()) }
    }

    fn gmtime(&self, t: i64, tm: &mut Tm) -> bool {
        let mut c = tm.to_libc();
        // SAFETY: both pointers are to locals; `gmtime_r` is reentrant.
        let ok = !unsafe { libc::gmtime_r(&raw const t, &raw mut c) }.is_null();
        tm.set_from_libc(&c);
        ok
    }

    fn localtime(&self, t: i64, tm: &mut Tm) -> bool {
        let mut c = tm.to_libc();
        // SAFETY: as `gmtime_r`; `localtime_r` reads the time zone, which
        // the C library guards.
        let ok = !unsafe { libc::localtime_r(&raw const t, &raw mut c) }.is_null();
        tm.set_from_libc(&c);
        ok
    }

    fn mktime(&self, tm: &mut Tm) -> i64 {
        let mut c = tm.to_libc();
        // SAFETY: a pointer to a local; `mktime` reads the time zone, which
        // the C library guards.
        let t = unsafe { libc::mktime(&raw mut c) };
        tm.set_from_libc(&c);
        t
    }
}

/// A clock in UTC (local time is UTC) whose "now" is fixed, in pure Rust.
#[derive(Clone, Copy, Debug, Default)]
pub struct UtcClock {
    /// What `time(NULL)` returns.
    pub now: i64,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (`month` 1..=12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Clock for UtcClock {
    fn time(&self) -> i64 {
        self.now
    }

    fn gmtime(&self, t: i64, tm: &mut Tm) -> bool {
        let days = t.div_euclid(86_400);
        let rem = t.rem_euclid(86_400);
        // Civil from days (H. Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        let Ok(year) = i32::try_from(y - 1900) else {
            return false;
        };
        let jan1 = days_from_civil(y, 1, 1);
        *tm = Tm {
            sec: (rem % 60) as i32,
            min: (rem / 60 % 60) as i32,
            hour: (rem / 3600) as i32,
            mday: d as i32,
            mon: (m - 1) as i32,
            year,
            wday: (days + 4).rem_euclid(7) as i32,
            yday: (days - jan1) as i32,
            isdst: 0,
            gmtoff: 0,
        };
        true
    }

    fn localtime(&self, t: i64, tm: &mut Tm) -> bool {
        self.gmtime(t, tm)
    }

    fn mktime(&self, tm: &mut Tm) -> i64 {
        let mon = i64::from(tm.mon);
        let y = i64::from(tm.year) + 1900 + mon.div_euclid(12);
        let m = mon.rem_euclid(12) + 1;
        let days = days_from_civil(y, m, 1) + i64::from(tm.mday) - 1;
        let t =
            days * 86_400 + i64::from(tm.hour) * 3600 + i64::from(tm.min) * 60 + i64::from(tm.sec);
        if self.gmtime(t, tm) { t } else { -1 }
    }
}

// ---------------------------------------------------------------------------
// C's string scanning
// ---------------------------------------------------------------------------

/// A C string: reads NUL at and past its end.
#[derive(Clone, Copy)]
struct Z<'a>(&'a [u8]);

impl Z<'_> {
    #[inline(always)]
    fn at(self, i: usize) -> u8 {
        self.0.get(i).copied().unwrap_or(0)
    }
}

/// C's `strtoul`/`strtol` in base 10 from `i`: white space, a sign, digits.
/// `(negative, magnitude or None on u64 overflow, end)`; `end` is `i` when
/// there are no digits.
fn scan(s: Z<'_>, i: usize) -> (bool, Option<u64>, usize) {
    let mut j = i;
    while isspace(s.at(j)) {
        j += 1;
    }
    let neg = match s.at(j) {
        b'-' => {
            j += 1;
            true
        }
        b'+' => {
            j += 1;
            false
        }
        _ => false,
    };
    if !isdigit(s.at(j)) {
        return (false, Some(0), i);
    }
    let mut v: Option<u64> = Some(0);
    while isdigit(s.at(j)) {
        let d = u64::from(s.at(j) - b'0');
        v = v
            .and_then(|v| v.checked_mul(10))
            .and_then(|v| v.checked_add(d));
        j += 1;
    }
    (neg, v, j)
}

/// `strtoul(s + i, &end, 10)`.
fn strtoul(s: Z<'_>, i: usize) -> (u64, usize) {
    match scan(s, i) {
        (_, None, e) => (u64::MAX, e),
        (true, Some(v), e) => (v.wrapping_neg(), e),
        (false, Some(v), e) => (v, e),
    }
}

/// `strtol(s + i, &end, 10)`.
fn strtol(s: Z<'_>, i: usize) -> (i64, usize) {
    match scan(s, i) {
        (false, Some(v), e) if v <= i64::MAX as u64 => (v as i64, e),
        (false, _, e) => (i64::MAX, e),
        (true, Some(v), e) if v <= 1 << 63 => ((v as i64).wrapping_neg(), e),
        (true, _, e) => (i64::MIN, e),
    }
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

const MONTH_NAMES: [&[u8]; 12] = [
    b"January",
    b"February",
    b"March",
    b"April",
    b"May",
    b"June",
    b"July",
    b"August",
    b"September",
    b"October",
    b"November",
    b"December",
];

const WEEKDAY_NAMES: [&[u8]; 7] = [
    b"Sundays",
    b"Mondays",
    b"Tuesdays",
    b"Wednesdays",
    b"Thursdays",
    b"Fridays",
    b"Saturdays",
];

/// `(name, offset in hours, dst)`. No summer-time conversion: "it's always
/// summer".
const TIMEZONE_NAMES: [(&[u8], i32, i32); 44] = [
    (b"IDLW", -12, 0), // International Date Line West
    (b"NT", -11, 0),   // Nome
    (b"CAT", -10, 0),  // Central Alaska
    (b"HST", -10, 0),  // Hawaii Standard
    (b"HDT", -10, 1),  // Hawaii Daylight
    (b"YST", -9, 0),   // Yukon Standard
    (b"YDT", -9, 1),   // Yukon Daylight
    (b"PST", -8, 0),   // Pacific Standard
    (b"PDT", -8, 1),   // Pacific Daylight
    (b"MST", -7, 0),   // Mountain Standard
    (b"MDT", -7, 1),   // Mountain Daylight
    (b"CST", -6, 0),   // Central Standard
    (b"CDT", -6, 1),   // Central Daylight
    (b"EST", -5, 0),   // Eastern Standard
    (b"EDT", -5, 1),   // Eastern Daylight
    (b"AST", -3, 0),   // Atlantic Standard
    (b"ADT", -3, 1),   // Atlantic Daylight
    (b"WAT", -1, 0),   // West Africa
    (b"GMT", 0, 0),    // Greenwich Mean
    (b"UTC", 0, 0),    // Universal (Coordinated)
    (b"Z", 0, 0),      // Zulu, alias for UTC
    (b"WET", 0, 0),    // Western European
    (b"BST", 0, 1),    // British Summer
    (b"CET", 1, 0),    // Central European
    (b"MET", 1, 0),    // Middle European
    (b"MEWT", 1, 0),   // Middle European Winter
    (b"MEST", 1, 1),   // Middle European Summer
    (b"CEST", 1, 1),   // Central European Summer
    (b"MESZ", 1, 1),   // Middle European Summer
    (b"FWT", 1, 0),    // French Winter
    (b"FST", 1, 1),    // French Summer
    (b"EET", 2, 0),    // Eastern Europe
    (b"EEST", 2, 1),   // Eastern European Daylight
    (b"WAST", 7, 0),   // West Australian Standard
    (b"WADT", 7, 1),   // West Australian Daylight
    (b"CCT", 8, 0),    // China Coast
    (b"JST", 9, 0),    // Japan Standard
    (b"EAST", 10, 0),  // Eastern Australian Standard
    (b"EADT", 10, 1),  // Eastern Australian Daylight
    (b"GST", 10, 0),   // Guam Standard
    (b"NZT", 12, 0),   // New Zealand
    (b"NZST", 12, 0),  // New Zealand Standard
    (b"NZDT", 12, 1),  // New Zealand Daylight
    (b"IDLE", 12, 0),  // International Date Line East
];

const NUMBER_NAMES: [&[u8]; 11] = [
    b"zero", b"one", b"two", b"three", b"four", b"five", b"six", b"seven", b"eight", b"nine",
    b"ten",
];

const TYPELEN: [(&[u8], i32); 5] = [
    (b"seconds", 1),
    (b"minutes", 60),
    (b"hours", 60 * 60),
    (b"days", 24 * 60 * 60),
    (b"weeks", 7 * 24 * 60 * 60),
];

// ---------------------------------------------------------------------------
// The strict parser (parse_date_basic)
// ---------------------------------------------------------------------------

/// `tm_to_time_t`: like `mktime` in UTC, without normalization, for
/// 1970-2099.
fn tm_to_time_t(tm: &Tm) -> i64 {
    const MDAYS: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let year = tm.year.wrapping_sub(70);
    let month = tm.mon;
    let mut day = tm.mday;
    if !(0..=129).contains(&year) {
        return -1;
    }
    if !(0..=11).contains(&month) {
        return -1;
    }
    if month < 2 || (year + 2) % 4 != 0 {
        day = day.wrapping_sub(1);
    }
    if tm.hour < 0 || tm.min < 0 || tm.sec < 0 {
        return -1;
    }
    // C: an `int` day count times `24*60*60UL`, in `unsigned long`.
    let days = (year * 365 + (year + 1) / 4 + MDAYS[month as usize]).wrapping_add(day);
    let secs = tm
        .hour
        .wrapping_mul(3600)
        .wrapping_add(tm.min.wrapping_mul(60))
        .wrapping_add(tm.sec);
    (i64::from(days) as u64)
        .wrapping_mul(86_400)
        .wrapping_add(i64::from(secs) as u64) as i64
}

/// `match_string`: how many bytes of `date` (from `i`) match `s`, ignoring
/// case; 0 if an alphanumeric byte differs.
fn match_string(date: Z<'_>, i: usize, s: &[u8]) -> usize {
    let mut n = 0;
    loop {
        let d = date.at(i + n);
        if d == 0 {
            break;
        }
        let c = s.get(n).copied().unwrap_or(0);
        if d == c || toupper(d) == toupper(c) {
            n += 1;
            continue;
        }
        if !isalnum(d) {
            break;
        }
        return 0;
    }
    n
}

/// `skip_alpha`.
fn skip_alpha(date: Z<'_>, i: usize) -> usize {
    let mut n = 0;
    loop {
        n += 1;
        if !isalpha(date.at(i + n)) {
            return n;
        }
    }
}

/// `match_alpha`: a month, weekday or time zone name, or AM/PM.
fn match_alpha(date: Z<'_>, i: usize, tm: &mut Tm, offset: &mut i32) -> usize {
    for (k, name) in MONTH_NAMES.iter().enumerate() {
        let m = match_string(date, i, name);
        if m >= 3 {
            tm.mon = k as i32;
            return m;
        }
    }
    for (k, name) in WEEKDAY_NAMES.iter().enumerate() {
        let m = match_string(date, i, name);
        if m >= 3 {
            tm.wday = k as i32;
            return m;
        }
    }
    for (name, off, dst) in TIMEZONE_NAMES {
        let m = match_string(date, i, name);
        if m >= 3 || m == name.len() {
            // Bogus, but we like summer.
            let off = off + dst;
            // Only use the zone name's offset if nothing better is known.
            if *offset == -1 {
                *offset = 60 * off;
            }
            return m;
        }
    }
    if match_string(date, i, b"PM") == 2 {
        tm.hour = (tm.hour % 12) + 12;
        return 2;
    }
    if match_string(date, i, b"AM") == 2 {
        tm.hour %= 12;
        return 2;
    }
    // Bad.
    skip_alpha(date, i)
}

/// `is_date`: set the date if `year`-`month`-`day` is one, and (when "now"
/// is known) not more than ten days in the future.
fn is_date(year: i32, month: i32, day: i32, now_tm: Option<&Tm>, now: i64, tm: &mut Tm) -> bool {
    if !(month > 0 && month < 13 && day > 0 && day < 32) {
        return false;
    }
    // C writes `tm` itself when "now" is unknown, else a copy.
    let mut r = *tm;
    r.mon = month - 1;
    r.mday = day;
    let ok = 'ok: {
        if year == -1 {
            match now_tm {
                None => break 'ok true,
                Some(n) => r.year = n.year,
            }
        } else if (1970..2100).contains(&year) {
            r.year = year - 1900;
        } else if year > 70 && year < 100 {
            r.year = year;
        } else if year < 38 {
            r.year = year + 100;
        } else {
            break 'ok false;
        }
        if now_tm.is_none() {
            break 'ok true;
        }
        let specified = tm_to_time_t(&r);
        // Neither commit nor author time should be more than ten days
        // into the future.
        if now.wrapping_add(10 * 24 * 3600) < specified {
            return false;
        }
        tm.mon = r.mon;
        tm.mday = r.mday;
        if year != -1 {
            tm.year = r.year;
        }
        return true;
    };
    if now_tm.is_none() {
        *tm = r;
    }
    ok
}

/// `match_multi_number`: `num[-./:]num[same]num` from `start`, whose
/// first number ends at `end` (`c`).
fn match_multi_number(
    clock: &impl Clock,
    num: u64,
    c: u8,
    date: Z<'_>,
    start: usize,
    end: usize,
    tm: &mut Tm,
) -> usize {
    let (num2, mut e) = strtol(date, end + 1);
    let mut num3: i64 = -1;
    if date.at(e) == c && isdigit(date.at(e + 1)) {
        (num3, e) = strtol(date, e + 1);
    }

    // Time? Date?
    match c {
        b':' => {
            if num3 < 0 {
                num3 = 0;
            }
            if num < 25 && (0..60).contains(&num2) && (0..=60).contains(&num3) {
                tm.hour = num as i32;
                tm.min = num2 as i32;
                tm.sec = num3 as i32;
            } else {
                return 0;
            }
        }
        b'-' | b'/' | b'.' => {
            let now = clock.time();
            let mut now_tm = Tm::default();
            let refuse_future = clock.gmtime(now, &mut now_tm).then_some(&now_tm);
            // C passes `unsigned long` and `long`s as `int`s.
            let (n1, n2, n3) = (num as i32, num2 as i32, num3 as i32);
            let ok = (num > 70
                && (is_date(n1, n2, n3, refuse_future, now, tm) // yyyy-mm-dd
                    || is_date(n1, n3, n2, refuse_future, now, tm))) // yyyy-dd-mm
                // mm/dd/yy[yy] first, unless the separator is '.' (dd.mm.yy[yy])
                || (c != b'.' && is_date(n3, n1, n2, refuse_future, now, tm))
                // European dd.mm.yy[yy] or funny US dd/mm/yy[yy]
                || is_date(n3, n2, n1, refuse_future, now, tm)
                // Funny European mm.dd.yy
                || (c == b'.' && is_date(n3, n1, n2, refuse_future, now, tm));
            if !ok {
                return 0;
            }
        }
        _ => {}
    }
    e - start
}

/// `nodate`: no field of the date or time is set yet.
fn nodate(tm: &Tm) -> bool {
    (tm.year & tm.mon & tm.mday & tm.hour & tm.min & tm.sec) < 0
}

/// `match_digit`: a number. Time? Year? Date?
fn match_digit(
    clock: &impl Clock,
    date: Z<'_>,
    i: usize,
    tm: &mut Tm,
    offset: &mut i32,
    tm_gmt: &mut bool,
) -> usize {
    let (num, end) = strtoul(date, i);

    // Seconds since 1970? For numbers of more than 8 digits, so as not to
    // rule out 20070606 as YYYYMMDD.
    if num >= 100_000_000 && nodate(tm) && clock.gmtime(num as i64, tm) {
        *tm_gmt = true;
        return end - i;
    }

    // num[-.:/]num[same]num?
    let c = date.at(end);
    if matches!(c, b':' | b'.' | b'/' | b'-') && isdigit(date.at(end + 1)) {
        let m = match_multi_number(clock, num, c, date, i, end, tm);
        if m != 0 {
            return m;
        }
    }

    // Guess from the number of digits.
    let mut n = 0;
    loop {
        n += 1;
        if !isdigit(date.at(i + n)) {
            break;
        }
    }

    // A four-digit year or a time zone?
    if n == 4 {
        if num <= 1400 && *offset == -1 {
            let minutes = (num % 100) as u32;
            let hours = (num / 100) as u32;
            *offset = (hours * 60 + minutes) as i32;
        } else if num > 1900 && num < 2100 {
            tm.year = (num - 1900) as i32;
        }
        return n;
    }

    // Days and months have one or two digits.
    if n > 2 {
        return n;
    }

    // Day of month before month or year in 1..12: 01 Apr 05 is April 1st,
    // 2005.
    if num > 0 && num < 32 && tm.mday < 0 {
        tm.mday = num as i32;
        return n;
    }

    // A two-digit year?
    if n == 2 && tm.year < 0 {
        if num < 10 && tm.mday >= 0 {
            tm.year = num as i32 + 100;
            return n;
        }
        if num >= 70 {
            tm.year = num as i32;
            return n;
        }
    }

    if num > 0 && num < 13 && tm.mon < 0 {
        tm.mon = num as i32 - 1;
    }
    n
}

/// `match_tz`: `[+-]hhmm`, `[+-]hh:mm` or `[+-]hh` at `i`.
fn match_tz(date: Z<'_>, i: usize, offp: &mut i32) -> usize {
    let (h, mut end) = strtoul(date, i + 1);
    let mut hour = h as i32;
    let n = end - (i + 1);
    let mut min: i32 = 0;

    if n == 4 {
        // hhmm
        min = hour % 100;
        hour /= 100;
    } else if n != 2 {
        min = 99; // random stuff
    } else if date.at(end) == b':' {
        // hh:mm?
        let (m, e) = strtoul(date, end + 1);
        min = m as i32;
        end = e;
        if end - (i + 1) != 5 {
            min = 99; // random stuff
        }
    } // otherwise "hh"

    // No offset of 24 hours or more, nor bad minutes.
    if min < 60 && hour < 24 {
        let mut offset = hour.wrapping_mul(60).wrapping_add(min);
        if date.at(i) == b'-' {
            offset = offset.wrapping_neg();
        }
        *offp = offset;
    }
    end - i
}

/// `match_object_header_date`: `"<stamp> [+-]hhmm"` (after the `@`), as in
/// an object header, alone.
fn match_object_header_date(date: Z<'_>, i: usize) -> Option<(i64, i32)> {
    let c = date.at(i);
    if !(b'0'..=b'8').contains(&c) {
        return None;
    }
    let (stamp, end) = strtoul(date, i);
    if date.at(end) != b' ' || stamp == u64::MAX || !matches!(date.at(end + 1), b'+' | b'-') {
        return None;
    }
    let d = end + 2;
    let (ofs, e) = strtol(date, d);
    let mut ofs = ofs as i32;
    if (date.at(e) != 0 && date.at(e) != b'\n') || e != d + 4 {
        return None;
    }
    ofs = (ofs / 100) * 60 + (ofs % 100);
    if date.at(d - 1) == b'-' {
        ofs = -ofs;
    }
    Some((stamp as i64, ofs))
}

/// `parse_date_basic`: the timestamp and the offset, or `None`.
fn parse_date_basic(clock: &impl Clock, date: Z<'_>) -> Option<(i64, i32)> {
    let mut tm = Tm {
        year: -1,
        mon: -1,
        mday: -1,
        isdst: -1,
        hour: -1,
        min: -1,
        sec: -1,
        ..Tm::default()
    };
    let mut offset: i32 = -1;
    let mut tm_gmt = false;

    if date.at(0) == b'@'
        && let Some(r) = match_object_header_date(date, 1)
    {
        return Some(r);
    }

    let mut i = 0;
    loop {
        let c = date.at(i);
        // Stop at the end of the string or a newline.
        if c == 0 || c == b'\n' {
            break;
        }
        let mut m = if isalpha(c) {
            match_alpha(date, i, &mut tm, &mut offset)
        } else if isdigit(c) {
            match_digit(clock, date, i, &mut tm, &mut offset, &mut tm_gmt)
        } else if (c == b'-' || c == b'+') && isdigit(date.at(i + 1)) {
            match_tz(date, i, &mut offset)
        } else {
            0
        };
        if m == 0 {
            // Bad.
            m = 1;
        }
        i += m;
    }

    let mut timestamp = tm_to_time_t(&tm);
    if offset == -1 {
        // mktime uses the local time zone.
        offset = (timestamp.wrapping_sub(clock.mktime(&mut tm)) as i32) / 60;
    }
    if timestamp == -1 {
        return None;
    }
    if !tm_gmt {
        timestamp = timestamp.wrapping_sub(i64::from(offset.wrapping_mul(60)));
    }
    Some((timestamp, offset))
}

// ---------------------------------------------------------------------------
// approxidate
// ---------------------------------------------------------------------------

/// `update_tm`: fill the date from "now" if unset, go back `sec` seconds,
/// and return the time.
fn update_tm(clock: &impl Clock, tm: &mut Tm, now: &Tm, sec: u64) -> i64 {
    if tm.mday < 0 {
        tm.mday = now.mday;
    }
    if tm.mon < 0 {
        tm.mon = now.mon;
    }
    if tm.year < 0 {
        tm.year = now.year;
        if tm.mon > now.mon {
            tm.year -= 1;
        }
    }
    // C: `time_t - unsigned long`, back in a `time_t`.
    let n = clock.mktime(tm).wrapping_sub(sec as i64);
    clock.localtime(n, tm);
    n
}

/// `date_time`: today's (or, if past, yesterday's) `hour` o'clock.
fn date_time(clock: &impl Clock, tm: &mut Tm, now: &Tm, hour: i32) {
    if tm.hour < hour {
        update_tm(clock, tm, now, 24 * 60 * 60);
    }
    tm.hour = hour;
    tm.min = 0;
    tm.sec = 0;
}

/// `date_pm` (`pm`) and `date_am`.
fn date_am_pm(tm: &mut Tm, num: &mut i32, pm: bool) {
    let n = *num;
    *num = 0;
    let mut hour = tm.hour;
    if n != 0 {
        hour = n;
        tm.min = 0;
        tm.sec = 0;
    }
    tm.hour = (hour % 12) + if pm { 12 } else { 0 };
}

/// The special words of approxidate (`special[]`); returns whether `date`
/// at `i` is one, after applying it.
fn special(
    clock: &impl Clock,
    date: Z<'_>,
    i: usize,
    tm: &mut Tm,
    now: &Tm,
    num: &mut i32,
) -> bool {
    const NAMES: [&[u8]; 8] = [
        b"yesterday",
        b"noon",
        b"midnight",
        b"tea",
        b"PM",
        b"AM",
        b"never",
        b"now",
    ];
    let Some(k) = NAMES
        .iter()
        .position(|name| match_string(date, i, name) == name.len())
    else {
        return false;
    };
    match k {
        0 => {
            update_tm(clock, tm, now, 24 * 60 * 60);
        }
        1 => date_time(clock, tm, now, 12),
        2 => date_time(clock, tm, now, 0),
        3 => date_time(clock, tm, now, 17),
        4 => date_am_pm(tm, num, true),
        5 => date_am_pm(tm, num, false),
        6 => {
            clock.localtime(0, tm);
        }
        _ => {
            update_tm(clock, tm, now, 0);
        }
    }
    true
}

/// `approxidate_alpha`: a word at `i`; returns the index after it.
fn approxidate_alpha(
    clock: &impl Clock,
    date: Z<'_>,
    i: usize,
    tm: &mut Tm,
    now: &Tm,
    num: &mut i32,
    touched: &mut bool,
) -> usize {
    let mut end = i;
    loop {
        end += 1;
        if !isalpha(date.at(end)) {
            break;
        }
    }

    for (k, name) in MONTH_NAMES.iter().enumerate() {
        if match_string(date, i, name) >= 3 {
            tm.mon = k as i32;
            *touched = true;
            return end;
        }
    }

    if special(clock, date, i, tm, now, num) {
        *touched = true;
        return end;
    }

    if *num == 0 {
        for (k, name) in NUMBER_NAMES.iter().enumerate().skip(1) {
            if match_string(date, i, name) == name.len() {
                *num = k as i32;
                *touched = true;
                return end;
            }
        }
        if match_string(date, i, b"last") == 4 {
            *num = 1;
            *touched = true;
        }
        return end;
    }

    for (name, length) in TYPELEN {
        if match_string(date, i, name) >= name.len() - 1 {
            // C: `int * (unsigned long)int`.
            let sec = (i64::from(length) as u64).wrapping_mul(i64::from(*num) as u64);
            update_tm(clock, tm, now, sec);
            *num = 0;
            *touched = true;
            return end;
        }
    }

    for (k, name) in WEEKDAY_NAMES.iter().enumerate() {
        if match_string(date, i, name) >= 3 {
            let mut n = num.wrapping_sub(1);
            *num = 0;
            let mut diff = tm.wday - k as i32;
            if diff <= 0 {
                n = n.wrapping_add(1);
            }
            diff = diff.wrapping_add(n.wrapping_mul(7));
            let sec = i64::from(diff.wrapping_mul(24 * 60 * 60)) as u64;
            update_tm(clock, tm, now, sec);
            *touched = true;
            return end;
        }
    }

    if match_string(date, i, b"months") >= 5 {
        update_tm(clock, tm, now, 0); // fill in the date fields if needed
        let mut n = tm.mon.wrapping_sub(*num);
        *num = 0;
        if n < 0 {
            // C adds 12 (and takes a year) until `n` is not negative.
            let k = (-i64::from(n) + 11) / 12;
            n = (i64::from(n) + 12 * k) as i32;
            tm.year = (i64::from(tm.year) - k) as i32;
        }
        tm.mon = n;
        *touched = true;
        return end;
    }

    if match_string(date, i, b"years") >= 4 {
        update_tm(clock, tm, now, 0); // fill in the date fields if needed
        tm.year = tm.year.wrapping_sub(*num);
        *num = 0;
        *touched = true;
        return end;
    }

    end
}

/// `approxidate_digit`: a number at `i`; returns the index after it.
fn approxidate_digit(
    clock: &impl Clock,
    date: Z<'_>,
    i: usize,
    tm: &mut Tm,
    num: &mut i32,
) -> usize {
    let (number, end) = strtoul(date, i);
    let c = date.at(end);
    if matches!(c, b':' | b'.' | b'/' | b'-') && isdigit(date.at(end + 1)) {
        let m = match_multi_number(clock, number, c, date, i, end, tm);
        if m != 0 {
            return i + m;
        }
    }
    // Zero-padding only for small numbers ("Dec 02", never "Dec 0002").
    if date.at(i) != b'0' || end - i <= 2 {
        *num = number as i32;
    }
    end
}

/// `pending_number`: a number left over is a day of month (as in
/// "Dec 6, 1992"), else a month, else a year.
fn pending_number(tm: &mut Tm, num: &mut i32) {
    let number = *num;
    if number == 0 {
        return;
    }
    *num = 0;
    if tm.mday < 0 && number < 32 {
        tm.mday = number;
    } else if tm.mon < 0 && number < 13 {
        tm.mon = number - 1;
    } else if tm.year < 0 {
        if number > 1969 && number < 2100 {
            tm.year = number - 1900;
        } else if number > 69 && number < 100 {
            tm.year = number;
        } else if number < 38 {
            tm.year = 100 + number;
        }
        // We mess up for number = 00 ?
    }
}

/// `approxidate_str`: the time, and whether anything was understood.
fn approxidate_str(clock: &impl Clock, date: Z<'_>, time_sec: i64) -> (i64, bool) {
    let mut number: i32 = 0;
    let mut touched = false;
    let mut tm = Tm::default();
    clock.localtime(time_sec, &mut tm);
    let now = tm;

    tm.year = -1;
    tm.mon = -1;
    tm.mday = -1;

    let mut i = 0;
    loop {
        let c = date.at(i);
        if c == 0 {
            break;
        }
        i += 1;
        if isdigit(c) {
            pending_number(&mut tm, &mut number);
            i = approxidate_digit(clock, date, i - 1, &mut tm, &mut number);
            touched = true;
            continue;
        }
        if isalpha(c) {
            i = approxidate_alpha(clock, date, i - 1, &mut tm, &now, &mut number, &mut touched);
        }
    }
    pending_number(&mut tm, &mut number);
    (update_tm(clock, &mut tm, &now, 0), touched)
}

/// What `git_date_offset_parse` does: its status, and what it writes to
/// `*out` and `*out_offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParseOutcome {
    /// The return value: 0, or -1 (with no error message, as the C).
    pub status: c_int,
    /// What is written to `*out` (also on approxidate's failure, as C).
    pub time: Option<i64>,
    /// What is written to `*out_offset` (only by the strict parser).
    pub offset: Option<i32>,
}

/// `git_date_offset_parse`, with the C's outputs.
pub(crate) fn offset_parse_raw(clock: &impl Clock, date: &[u8]) -> ParseOutcome {
    let date = Z(date);
    if let Some((t, o)) = parse_date_basic(clock, date) {
        return ParseOutcome {
            status: 0,
            time: Some(t),
            offset: Some(o),
        };
    }
    let now = clock.time();
    if now == -1 {
        return ParseOutcome {
            status: -1,
            time: None,
            offset: None,
        };
    }
    let (t, touched) = approxidate_str(clock, date, now);
    ParseOutcome {
        status: if touched { 0 } else { -1 },
        time: Some(t),
        offset: None,
    }
}

/// `git_date_offset_parse` on `clock`: the time and, from the strict
/// parser, its offset in minutes (`None` from approxidate, which does not
/// set it). `date` ends at its first NUL or its end. Fails with -1 (and
/// no message, as the C) when nothing in it is a date.
pub fn offset_parse_with(clock: &impl Clock, date: &[u8]) -> Result<(i64, Option<i32>)> {
    let r = offset_parse_raw(clock, date);
    match (r.status, r.time) {
        (0, Some(t)) => Ok((t, r.offset)),
        _ => Err(ErrorCode::GENERIC),
    }
}

/// `git_date_offset_parse`, in the local time zone at the current time.
pub fn offset_parse(date: &[u8]) -> Result<(i64, Option<i32>)> {
    offset_parse_with(&SystemClock, date)
}

/// `git_date_parse`.
pub fn parse(date: &[u8]) -> Result<i64> {
    offset_parse(date).map(|(t, _)| t)
}

/// `git_date_rfc2822_fmt` on `clock`: append e.g.
/// `"Mon, 1 Jan 2024 10:00:00 +0100"` to `out`. Fails with -1 (no
/// message) when `gmtime_r` does.
pub(crate) fn rfc2822_fmt_with(
    clock: &impl Clock,
    out: &mut GitStr,
    time: i64,
    offset: i32,
) -> Result<()> {
    let t = time.wrapping_add(i64::from(offset.wrapping_mul(60)));
    let mut gmt = Tm::default();
    if !clock.gmtime(t, &mut gmt) {
        return Err(ErrorCode::GENERIC);
    }
    let wday = WEEKDAY_NAMES[gmt.wday as usize % 7];
    let mon = MONTH_NAMES[gmt.mon as usize % 12];
    // "%.3s, %u %.3s %.4u %02u:%02u:%02u %+03d%02d" (43 bytes).
    out.printf(
        43,
        format_args!(
            "{}, {} {} {:04} {:02}:{:02}:{:02} {:+03}{:02}",
            // The names are ASCII.
            core::str::from_utf8(&wday[..3]).unwrap_or_default(),
            gmt.mday as u32,
            core::str::from_utf8(&mon[..3]).unwrap_or_default(),
            gmt.year.wrapping_add(1900) as u32,
            gmt.hour as u32,
            gmt.min as u32,
            gmt.sec as u32,
            offset / 60,
            offset % 60,
        ),
    )
}

/// `git_date_rfc2822_fmt`.
pub(crate) fn rfc2822_fmt(out: &mut GitStr, time: i64, offset: i32) -> Result<()> {
    rfc2822_fmt_with(&SystemClock, out, time, offset)
}

/// The RFC 2822 form of `time` at `offset` minutes from UTC (as
/// `git_date_rfc2822_fmt`), e.g. `"Wed, 9 Apr 2014 10:21:03 +0200"`, on
/// `clock` (whose `gmtime` it uses). Fails with -1 (no message) when the
/// time cannot be broken down.
pub fn rfc2822_with(clock: &impl Clock, time: i64, offset: i32) -> Result<String> {
    let mut s = GitStr::new();
    rfc2822_fmt_with(clock, &mut s, time, offset)?;
    // The format writes ASCII only.
    Ok(String::from_utf8_lossy(s.as_bytes()).into_owned())
}

/// [`rfc2822_with`] on the C library's clock.
pub fn rfc2822(time: i64, offset: i32) -> Result<String> {
    rfc2822_with(&SystemClock, time, offset)
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

pub mod ffi {
    use super::*;
    use crate::git_assert_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};

    /// `git_date_offset_parse`.
    ///
    /// # Safety
    ///
    /// `out` and `out_offset` are writable; `date` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_date_offset_parse(
        mut out: Out<'_, i64>,
        mut out_offset: Out<'_, c_int>,
        date: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let date = date.to_bytes();
            let r = offset_parse_raw(&SystemClock, date);
            if let Some(t) = r.time {
                out.write(t);
            }
            if let Some(o) = r.offset {
                out_offset.write(o);
            }
            r.status
        })
    }

    /// `git_date_parse`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `date` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_date_parse(out: Out<'_, i64>, date: CStrArg<'_>) -> c_int {
        let mut offset: c_int = 0;
        // SAFETY: forwarded to the caller; `offset` is a local.
        unsafe { git_date_offset_parse(out, Out::from(&mut offset), date) }
    }

    /// `git_date_rfc2822_fmt`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_str` that nothing else accesses during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_date_rfc2822_fmt(
        out: *mut GitStr,
        time: i64,
        offset: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: the caller's contract.
            let out = unsafe { &mut *out };
            status(rfc2822_fmt(out, time, offset))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2009-02-13 23:31:30 UTC.
    const NOW: i64 = 1_234_567_890;

    fn p(s: &str) -> Option<(i64, Option<i32>)> {
        offset_parse_with(&UtcClock { now: NOW }, s.as_bytes()).ok()
    }

    #[test]
    fn utc_clock_round_trips() {
        let c = UtcClock::default();
        for t in [0, 1, 86_399, 951_782_400, NOW, -1, -86_401, 4_102_444_800] {
            let mut tm = Tm::default();
            assert!(c.gmtime(t, &mut tm));
            assert_eq!(c.mktime(&mut tm), t);
        }
        let mut tm = Tm::default();
        assert!(c.gmtime(NOW, &mut tm));
        assert_eq!(
            (
                tm.year, tm.mon, tm.mday, tm.hour, tm.min, tm.sec, tm.wday, tm.yday
            ),
            (109, 1, 13, 23, 31, 30, 5, 43)
        );
    }

    #[test]
    fn strict_formats() {
        assert_eq!(p("@1234567890 +0100"), Some((1_234_567_890, Some(60))));
        assert_eq!(
            p("2008-12-05 10:11:12 +0200"),
            Some((1_228_464_672, Some(120)))
        );
        assert_eq!(
            p("Thu, 5 Dec 2008 10:11:12 -0130"),
            Some((1_228_477_272, Some(-90)))
        );
        assert_eq!(p("1234567890"), Some((1_234_567_890, Some(0))));
        assert_eq!(
            p("2008-12-05 10:11:12 PST"),
            Some((1_228_500_672, Some(-480)))
        );
    }

    #[test]
    fn approxidate() {
        assert_eq!(p("now"), Some((NOW, None)));
        assert_eq!(p("yesterday"), Some((NOW - 86_400, None)));
        assert_eq!(p("2 days ago"), Some((NOW - 2 * 86_400, None)));
        assert_eq!(p("3.weeks.ago"), Some((NOW - 21 * 86_400, None)));
        assert_eq!(p("never"), Some((0, None)));
        assert_eq!(p("garbage"), None);
        assert_eq!(p(""), None);
    }

    #[test]
    fn scanning() {
        assert_eq!(strtoul(Z(b"  -3x"), 0), (u64::MAX - 2, 4));
        assert_eq!(strtoul(Z(b"99999999999999999999999"), 0), (u64::MAX, 23));
        assert_eq!(strtol(Z(b"-9223372036854775809"), 0), (i64::MIN, 20));
        assert_eq!(strtol(Z(b"+x"), 0), (0, 0));
    }

    #[test]
    fn rfc2822() {
        let _g = crate::util::test_support::init();
        let c = UtcClock::default();
        let mut s = GitStr::new();
        rfc2822_fmt_with(&c, &mut s, 1_234_567_890, -90).unwrap();
        assert_eq!(s.as_bytes(), b"Fri, 13 Feb 2009 22:01:30 -01-30");
        let mut s = GitStr::new();
        rfc2822_fmt_with(&c, &mut s, 0, 0).unwrap();
        assert_eq!(s.as_bytes(), b"Thu, 1 Jan 1970 00:00:00 +0000");
    }
}
