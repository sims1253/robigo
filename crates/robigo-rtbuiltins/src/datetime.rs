//! datetime.c's `.Internal(strptime(x, format, tz))` (with Rstrptime.h's
//! `R_strptime`), `.Internal(as.POSIXct(x, tz))`, `.Internal(as.POSIXlt(x,
//! tz))`, `.Internal(Date2POSIXlt(x, tz))` and `.Internal(POSIXlt2Date(x))`,
//! as GNU R builds them on Linux: the C library's `localtime_r`/`gmtime_r`
//! and `mktime` (`PATH 1`, no `USE_INTERNAL_MKTIME`; glibc's mktime works
//! before 1902, before 1970 and after 2037, and its `time_t` has no leap
//! seconds) and R's own `timegm00` for UTC.
//!
//! GNU R converts in a time zone other than UTC or GMT by setting `TZ` in
//! the process environment around the conversion (`set_tz`/`reset_tz`):
//! here a [`clib::TzSession`], which holds the environment for the
//! conversions, and needs the embedder's grant of environment writes.
//! `strptime` in such a zone is not implemented yet (its warnings would
//! have to wait for the session's end).

use crate::attrib::{Attr, Attrs};
use crate::util::not_yet;
use robigo_rtcore::clib::{self, Tm};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, RSym, Ref, Root, View};
use robigo_text::tre::{iswspace, towlower};

const NA_INTEGER: i32 = i32::MIN;
const NA_REAL: f64 = robigo_text::NA_REAL;

/// The components of a "POSIXlt" list (datetime.c's `ltnames`).
const LTNAMES: [&[u8]; 11] =
    [b"sec", b"min", b"hour", b"mday", b"mon", b"year", b"wday", b"yday", b"isdst", b"zone", b"gmtoff"];

/// C's `(int) d` on x86-64 (`cvttsd2si`): truncation, and `INT_MIN` for NaN
/// and values outside `int` (where Rust's `as` saturates).
fn c_int(d: f64) -> i32 {
    if d.is_nan() || d >= 2147483648.0 || d <= -2147483649.0 { i32::MIN } else { d as i32 }
}

const MONTH_DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

// R: datetime.c:isleap, days_in_year, days_in_month (C int arithmetic,
// which wraps on the NA year)
fn isleap(y: i32) -> bool {
    (y.wrapping_rem(4) == 0 && y.wrapping_rem(100) != 0) || y.wrapping_rem(400) == 0
}
fn days_in_year(y: i32) -> i32 {
    if isleap(y) { 366 } else { 365 }
}
fn days_in_month(mon: i32, yr: i32) -> i32 {
    if mon == 1 && isleap(1900i32.wrapping_add(yr)) { 29 } else { MONTH_DAYS[mon as usize] }
}

/// `validate_tm(tm)`: `tm` adjusted to a valid date-time; 0 if it was one,
/// -1 if it cannot be, else about the number of corrections.
// R: datetime.c:validate_tm
fn validate_tm(tm: &mut Tm) -> i32 {
    let mut res = 0;
    if tm.sec < 0 || tm.sec > 60 {
        res += 1;
        let tmp = tm.sec / 60;
        tm.sec -= 60 * tmp;
        tm.min = tm.min.wrapping_add(tmp);
        if tm.sec < 0 {
            tm.sec += 60;
            tm.min = tm.min.wrapping_sub(1);
        }
    }
    if tm.min < 0 || tm.min > 59 {
        res += 1;
        let tmp = tm.min / 60;
        tm.min -= 60 * tmp;
        tm.hour = tm.hour.wrapping_add(tmp);
        if tm.min < 0 {
            tm.min += 60;
            tm.hour = tm.hour.wrapping_sub(1);
        }
    }
    if tm.hour == 24 && tm.min == 0 && tm.sec == 0 {
        // 24:00:00
        tm.hour = 0;
        tm.mday = tm.mday.wrapping_add(1);
        if (0..=11).contains(&tm.mon) && tm.mday > days_in_month(tm.mon, tm.year) {
            tm.mon += 1;
            tm.mday = 1;
            if tm.mon == 12 {
                tm.year = tm.year.wrapping_add(1);
                tm.mon = 0;
            }
        }
    } else if tm.hour < 0 || tm.hour > 23 {
        res += 1;
        let tmp = tm.hour / 24;
        tm.hour -= 24 * tmp;
        tm.mday = tm.mday.wrapping_add(tmp);
        if tm.hour < 0 {
            tm.hour += 24;
            tm.mday = tm.mday.wrapping_sub(1);
        }
    }
    // defer fixing mday until we know the year
    if tm.mon < 0 || tm.mon > 11 {
        res += 1;
        let tmp = tm.mon / 12;
        tm.mon -= 12 * tmp;
        tm.year = tm.year.wrapping_add(tmp);
        if tm.mon < 0 {
            tm.mon += 12;
            tm.year = tm.year.wrapping_sub(1);
        }
    }
    // a limit on the loops of about 3000x round
    if tm.mday < -1000000 || tm.mday > 1000000 {
        return -1;
    }
    if tm.mday.abs() > 366 {
        res += 1;
        // first spin back until January
        while tm.mon > 0 {
            tm.mon -= 1;
            tm.mday += days_in_month(tm.mon, tm.year);
        }
        // then spin on/back by years
        while tm.mday < 1 {
            tm.year = tm.year.wrapping_sub(1);
            tm.mday += 365 + isleap(1900i32.wrapping_add(tm.year)) as i32;
        }
        loop {
            let tmp = 365 + isleap(1900i32.wrapping_add(tm.year)) as i32;
            if tm.mday <= tmp {
                break;
            }
            tm.mday -= tmp;
            tm.year = tm.year.wrapping_add(1);
        }
    }
    while tm.mday < 1 {
        res += 1;
        tm.mon -= 1;
        if tm.mon < 0 {
            tm.mon += 12;
            tm.year = tm.year.wrapping_sub(1);
        }
        tm.mday += days_in_month(tm.mon, tm.year);
    }
    loop {
        let tmp = days_in_month(tm.mon, tm.year);
        if tm.mday <= tmp {
            break;
        }
        res += 1;
        tm.mon += 1;
        if tm.mon > 11 {
            tm.mon -= 12;
            tm.year = tm.year.wrapping_add(1);
        }
        tm.mday -= tmp;
    }
    res
}

/// `mkdate00(tm)`: days since 1970-01-01, setting `tm`'s `yday` and `wday`
/// (NA for a date with an NA field).
// R: datetime.c:mkdate00
fn mkdate00(tm: &mut Tm) -> f64 {
    if tm.mday == NA_INTEGER || tm.year == NA_INTEGER || tm.mon == NA_INTEGER {
        tm.yday = NA_INTEGER;
        tm.wday = NA_INTEGER;
        return NA_REAL;
    }
    let mut day = tm.mday - 1;
    let mut year0 = 1900i32.wrapping_add(tm.year);
    let mut excess = 0.0f64;
    // (`-year0 / 400` of INT_MIN: what the oracle's build computes, the
    // quotient of the exact negation; `(int)(excess * 400)` as C casts)
    if year0 >= 400 {
        excess = ((year0 / 400) - 1) as f64;
        year0 = year0.wrapping_sub(c_int(excess * 400.0));
    } else if year0 < 0 {
        excess = (-1 - (-(year0 as i64) / 400)) as f64;
        year0 = year0.wrapping_sub(c_int(excess * 400.0));
    }
    for &d in MONTH_DAYS.iter().take(tm.mon.max(0) as usize) {
        day += d;
    }
    if tm.mon > 1 && isleap(year0) {
        day += 1;
    }
    tm.yday = day;
    if year0 > 1970 {
        for year in 1970..year0 {
            day += days_in_year(year);
        }
    } else if year0 < 1970 {
        let mut year = 1969;
        while year >= year0 {
            day -= days_in_year(year);
            year -= 1;
        }
    }
    // weekday: Epoch day was a Thursday
    tm.wday = ((day % 7) + 4) % 7;
    if tm.wday < 0 {
        tm.wday += 7;
    }
    day as f64 + excess * 146097.0
}

/// `timegm00(tm)`: seconds since the epoch of a UTC date-time (no checks).
// R: datetime.c:timegm00
fn timegm00(tm: &mut Tm) -> f64 {
    let day = mkdate00(tm);
    if day.to_bits() == NA_REAL.to_bits() {
        return NA_REAL;
    }
    tm.sec as f64 + (tm.min as f64 * 60.0) + (tm.hour as f64 * 3600.0) + day * 86400.0
}

/// `mktime0(tm, local = 0)` (`PATH 1`): `-1` (and `errno` set, the `true`)
/// when `tm` cannot be validated, else `timegm00`'s result.
// R: datetime.c:mktime0
fn mktime0_utc(tm: &mut Tm) -> (f64, bool) {
    mktime0(tm, None)
}

/// `mktime0(tm, local)` (`PATH 1`, every `HAVE_WORKING_MKTIME_*` and
/// `HAVE_POSIX_LEAPSECONDS`): `-1` and `errno` set when `tm` cannot be
/// validated; else `timegm00` (UTC, `local` None) or the C library's
/// `mktime` in the session's zone, with whether it set `errno`.
// R: datetime.c:mktime0
fn mktime0(tm: &mut Tm, local: Option<&clib::TzSession<'_>>) -> (f64, bool) {
    if validate_tm(tm) < 0 {
        return (-1.0, true); // errno = EOVERFLOW
    }
    match local {
        None => (timegm00(tm), false),
        Some(s) => s.mktime(tm),
    }
}

/// `localtime0(&d, local, ltm)` (`PATH 1`): `localtime_r` in the session's
/// zone, or `gmtime_r` (`local` None), of `d` floored (`LOCALTIME_MK_`).
// R: datetime.c:localtime0, LOCALTIME_MK_
fn localtime0(d: f64, local: Option<&clib::TzSession<'_>>) -> Option<Tm> {
    match local {
        None => gmtime0(d),
        Some(s) => s.localtime(time_t_of(d)),
    }
}

/// `LOCALTIME_MK_`'s `(time_t) d`, one less for a negative fraction.
fn time_t_of(d: f64) -> i64 {
    // (time_t) d, as C converts (NaN and out of range give INT64_MIN on
    // x86-64)
    let mut t = if d.is_nan() || d >= 9.223372036854776e18 || d < -9.223372036854776e18 { i64::MIN } else { d as i64 };
    if d < 0.0 && d != t as f64 {
        t = t.wrapping_sub(1);
    }
    t
}

/// `localtime0(&d, local = 0, ltm)` (`PATH 1`): gmtime_r of `d` floored.
// R: datetime.c:localtime0, LOCALTIME_MK_
fn gmtime0(d: f64) -> Option<Tm> {
    clib::gmtime(time_t_of(d))
}

/// `glibc_fix(tm, &invalid)`: the current local date's year, and month and
/// day when neither is known; an unknown day with a known month is invalid.
// R: datetime.c:glibc_fix
fn glibc_fix(tm: &mut Tm, invalid: &mut bool) {
    let tm0 = clib::localtime(clib::time_now()).unwrap_or_default();
    if tm.year == NA_INTEGER {
        tm.year = tm0.year;
    }
    if tm.mon != NA_INTEGER && tm.mday != NA_INTEGER {
        return;
    }
    // at least one of the month and the day of the month is missing
    if tm.yday != NA_INTEGER {
        // since we have yday, let that take precedence over mon/mday
        let (mut yday, mut mon) = (tm.yday, 0);
        loop {
            // past December (a day 366 in a common year) C reads beyond
            // month_days; the month of 12 it leaves makes the date invalid
            if mon > 11 {
                break;
            }
            let tmp = days_in_month(mon, tm.year);
            if yday < tmp {
                break;
            }
            yday -= tmp;
            mon += 1;
        }
        tm.mon = mon;
        tm.mday = yday + 1;
    } else {
        if tm.mday == NA_INTEGER {
            if tm.mon != NA_INTEGER {
                *invalid = true;
                return;
            }
            tm.mday = tm0.mday;
        }
        if tm.mon == NA_INTEGER {
            tm.mon = tm0.mon;
        }
    }
}

// --- R_strptime (Rstrptime.h) ---------------------------------------------

const WEEKDAY_NAME: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const AB_WEEKDAY_NAME: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAME: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const AB_MONTH_NAME: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const AM_PM: [&str; 2] = ["AM", "PM"];

// R: Rstrptime.h:__mon_yday
const MON_YDAY: [[i32; 13]; 2] = [
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365],
    [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366],
];

// R: Rstrptime.h:__isleap
fn gisleap(year: i32) -> bool {
    year.wrapping_rem(4) == 0 && (year.wrapping_rem(100) != 0 || year.wrapping_rem(400) == 0)
}

/// `__mon_yday[leap][t_mon - 1]` as C reads it: the two rows are one array,
/// so `t_mon` 0 of the leap row is the normal row's last entry (a negative
/// `yday` from `%U`/`%W` gets there). Before the normal row C reads outside
/// the table; `None` makes that element's day of the month NA, which its
/// month of -1 makes invalid in GNU R anyway.
fn mon_yday_prev(leap: usize, t_mon: usize) -> Option<i32> {
    let k = (leap * 13 + t_mon).checked_sub(1)?;
    Some(MON_YDAY[k / 13][k % 13])
}

/// `day_of_the_week(tm)`.
// R: Rstrptime.h:day_of_the_week
fn day_of_the_week(tm: &mut Tm) {
    // R bug fix: day_of_the_week needs year, month, mday set
    if tm.year == NA_INTEGER || tm.mon == NA_INTEGER || tm.mday == NA_INTEGER {
        return;
    }
    let corr_year = 1900i32.wrapping_add(tm.year) - (tm.mon < 2) as i32;
    let q = corr_year / 4;
    let wday = -473 + (365i32.wrapping_mul(tm.year - 70)) + q - (q / 25) + ((q % 25) < 0) as i32 + ((q / 25) / 4)
        // no check on month in range
        + MON_YDAY[0][tm.mon.clamp(0, 12) as usize]
        + tm.mday
        - 1;
    tm.wday = ((wday % 7) + 7) % 7;
}

/// `day_of_the_year(tm)`.
// R: Rstrptime.h:day_of_the_year
fn day_of_the_year(tm: &mut Tm) {
    if tm.year == NA_INTEGER || tm.mon == NA_INTEGER || tm.mday == NA_INTEGER {
        return;
    }
    tm.yday = MON_YDAY[gisleap(1900i32.wrapping_add(tm.year)) as usize][tm.mon.clamp(0, 12) as usize] + (tm.mday - 1);
}

/// One `R_strptime` call's state: the fields it fills, and the warnings it
/// gave (GNU R's `warning()`s, in order).
struct Strptime<'a> {
    tm: &'a mut Tm,
    psecs: &'a mut f64,
    poffset: &'a mut i32,
    /// `mbcslocale`: `w_strptime_internal` (`%OS` read by `wcstod`), else
    /// `strptime_internal` (`R_strtod`).
    wide: bool,
    loc: robigo_text::Locale,
    warnings: Vec<String>,
}

/// What stopped a conversion: GNU R's `error()`.
struct StrptimeError(String);

impl Strptime<'_> {
    /// `w_strptime_internal(rp, fmt, ...)` / `strptime_internal`: the position
    /// after what `fmt` matched in `s`, or `None` (C's NULL). `s` and `fmt`
    /// are code points (bytes outside a multibyte locale); past their end is
    /// C's NUL.
    // R: Rstrptime.h:w_strptime_internal, strptime_internal
    fn internal(&mut self, s: &[u32], mut rp: usize, fmt: &[u32]) -> Result<Option<usize>, StrptimeError> {
        let at = |i: usize| s.get(i).copied().unwrap_or(0);
        let fat = |i: usize| fmt.get(i).copied().unwrap_or(0);
        let mut f = 0usize;
        let (mut have_i, mut is_pm) = (false, false);
        let mut century = -1;
        let mut want_century = false;
        let (mut have_wday, mut want_xday, mut have_yday, mut have_mon, mut have_mday) =
            (false, false, false, false, false);
        let (mut have_uweek, mut have_wweek) = (false, false);
        let mut week_no = 0;
        macro_rules! fail {
            () => {
                return Ok(None)
            };
        }
        // get_number(from, to, n): spaces skipped, at least one digit, at
        // most n, in [from, to]
        macro_rules! get_number {
            ($from:expr, $to:expr, $n:expr) => {{
                let mut n: i32 = $n;
                let mut val: i32 = 0;
                while at(rp) == ' ' as u32 {
                    rp += 1;
                }
                if at(rp) < '0' as u32 || at(rp) > '9' as u32 {
                    fail!();
                }
                loop {
                    val = val.wrapping_mul(10).wrapping_add((at(rp) - '0' as u32) as i32);
                    rp += 1;
                    n -= 1;
                    if !(n > 0 && at(rp) >= '0' as u32 && at(rp) <= '9' as u32) {
                        break;
                    }
                }
                if !($from..=$to).contains(&val) {
                    fail!();
                }
                val
            }};
        }
        // w_match_string: case-insensitive, advancing on a match
        let loc = self.loc;
        let match_string = |name: &str, rp: &mut usize| -> bool {
            let mut i = *rp;
            for c in name.chars() {
                let b = s.get(i).copied().unwrap_or(0);
                if b == 0 || towlower(c as u32, loc) != towlower(b, loc) {
                    return false;
                }
                i += 1;
            }
            *rp = i;
            true
        };
        while fat(f) != 0 {
            // a white space in the format matches 0 or more white space
            if iswspace(fat(f), self.loc) {
                while iswspace(at(rp), self.loc) {
                    rp += 1;
                }
                f += 1;
                continue;
            }
            // any character but `%` must be matched by the same character
            if fat(f) != '%' as u32 {
                let (a, b) = (fat(f), at(rp));
                f += 1;
                rp += 1;
                if a != b {
                    fail!();
                }
                continue;
            }
            f += 1;
            // (the `E` modifier's `goto start_over`)
            loop {
                let c = char::from_u32(fat(f)).unwrap_or('\0');
                f += 1;
                match c {
                    '%' => {
                        let b = at(rp);
                        rp += 1;
                        if b != '%' as u32 {
                            fail!();
                        }
                    }
                    'a' | 'A' => {
                        // try full name first
                        let mut cnt = (0..7).find(|&k| match_string(WEEKDAY_NAME[k], &mut rp));
                        if cnt.is_none() {
                            cnt = (0..7).find(|&k| match_string(AB_WEEKDAY_NAME[k], &mut rp));
                        }
                        let Some(cnt) = cnt else { fail!() };
                        self.tm.wday = cnt as i32;
                        have_wday = true;
                    }
                    'b' | 'B' | 'h' => {
                        let mut cnt = (0..12).find(|&k| match_string(MONTH_NAME[k], &mut rp));
                        if cnt.is_none() {
                            cnt = (0..12).find(|&k| match_string(AB_MONTH_NAME[k], &mut rp));
                        }
                        let Some(cnt) = cnt else { fail!() };
                        self.tm.mon = cnt as i32;
                        want_xday = true;
                    }
                    'c' => match self.recursive(s, rp, "%a %b %e %H:%M:%S %Y")? {
                        Some(r) => rp = r,
                        None => fail!(),
                    },
                    'C' => {
                        century = get_number!(0, 99, 2);
                        want_xday = true;
                    }
                    'd' | 'e' => {
                        self.tm.mday = get_number!(1, 31, 2);
                        have_mday = true;
                        want_xday = true;
                    }
                    'F' => {
                        match self.recursive(s, rp, "%Y-%m-%d")? {
                            Some(r) => rp = r,
                            None => fail!(),
                        }
                        want_xday = true;
                    }
                    'x' | 'D' => {
                        match self.recursive(s, rp, "%y/%m/%d")? {
                            Some(r) => rp = r,
                            None => fail!(),
                        }
                        want_xday = true;
                    }
                    'k' | 'H' => {
                        // allow 24:00:00
                        self.tm.hour = get_number!(0, 24, 2);
                        have_i = false;
                    }
                    'l' | 'I' => {
                        self.tm.hour = get_number!(1, 12, 2) % 12;
                        have_i = true;
                    }
                    'j' => {
                        self.tm.yday = get_number!(1, 366, 3) - 1;
                        have_yday = true;
                    }
                    'm' => {
                        self.tm.mon = get_number!(1, 12, 2) - 1;
                        have_mon = true;
                        want_xday = true;
                    }
                    'M' => self.tm.min = get_number!(0, 59, 2),
                    'n' | 't' => {
                        while iswspace(at(rp), self.loc) {
                            rp += 1;
                        }
                    }
                    'p' => {
                        if !match_string(AM_PM[0], &mut rp) {
                            if match_string(AM_PM[1], &mut rp) {
                                is_pm = true;
                            } else {
                                fail!();
                            }
                        }
                    }
                    'r' => match self.recursive(s, rp, "%I:%M:%S %p")? {
                        Some(r) => rp = r,
                        None => fail!(),
                    },
                    'R' => match self.recursive(s, rp, "%H:%M")? {
                        Some(r) => rp = r,
                        None => fail!(),
                    },
                    's' => {
                        // the number of seconds may be very high: read it
                        // character by character (time_t, which wraps)
                        if at(rp) < '0' as u32 || at(rp) > '9' as u32 {
                            fail!();
                        }
                        let mut secs: i64 = 0;
                        loop {
                            secs = secs.wrapping_mul(10).wrapping_add((at(rp) - '0' as u32) as i64);
                            rp += 1;
                            if !(at(rp) >= '0' as u32 && at(rp) <= '9' as u32) {
                                break;
                            }
                        }
                        match clib::localtime(secs) {
                            Some(t) => *self.tm = t,
                            None => fail!(),
                        }
                    }
                    'S' => self.tm.sec = get_number!(0, 61, 2),
                    'X' | 'T' => match self.recursive(s, rp, "%H:%M:%S")? {
                        Some(r) => rp = r,
                        None => fail!(),
                    },
                    'u' => {
                        self.tm.wday = get_number!(1, 7, 1) % 7;
                        have_wday = true;
                    }
                    'g' => {
                        // XXX This cannot determine any field in TM.
                        let _ = get_number!(0, 99, 2);
                    }
                    'G' => {
                        if at(rp) < '0' as u32 || at(rp) > '9' as u32 {
                            fail!();
                        }
                        // XXX Ignore the number
                        loop {
                            rp += 1;
                            if !(at(rp) >= '0' as u32 && at(rp) <= '9' as u32) {
                                break;
                            }
                        }
                    }
                    'U' => {
                        week_no = get_number!(0, 53, 2);
                        have_uweek = true;
                    }
                    'W' => {
                        week_no = get_number!(0, 53, 2);
                        have_wweek = true;
                    }
                    'V' => {
                        let _ = get_number!(0, 53, 2);
                    }
                    'w' => {
                        self.tm.wday = get_number!(0, 6, 1);
                        have_wday = true;
                    }
                    'y' => {
                        // values in the range 69-99 refer to the twentieth
                        // century
                        let ival = get_number!(0, 99, 2);
                        self.tm.year = if ival >= 69 { ival } else { ival + 100 };
                        want_century = true;
                        want_xday = true;
                    }
                    'Y' => {
                        self.tm.year = get_number!(0, 9999, 4) - 1900;
                        want_century = false;
                        want_xday = true;
                    }
                    'z' => {
                        let (mut n, mut val) = (0, 0i32);
                        while at(rp) == ' ' as u32 {
                            rp += 1;
                        }
                        if at(rp) != '+' as u32 && at(rp) != '-' as u32 {
                            fail!();
                        }
                        let neg = at(rp) == '-' as u32;
                        rp += 1;
                        while n < 4 && at(rp) >= '0' as u32 && at(rp) <= '9' as u32 {
                            val = val * 10 + (at(rp) - '0' as u32) as i32;
                            rp += 1;
                            n += 1;
                        }
                        if n != 4 {
                            fail!();
                        }
                        // the minutes into decimal
                        if val % 100 >= 60 {
                            fail!();
                        }
                        val = (val / 100) * 100 + ((val % 100) * 50) / 30;
                        if val > 1400 {
                            self.warnings.push("values for %z outside +/-1400 are an error".to_owned());
                            fail!();
                        }
                        let off = (val * 3600) / 100;
                        *self.poffset = if neg { -off } else { off };
                    }
                    'Z' => return Err(StrptimeError("use of %Z for input is not supported".to_owned())),
                    'E' => {
                        // no information about the era format: the normal one
                        let n = char::from_u32(fat(f)).unwrap_or('\0');
                        if !matches!(n, 'c' | 'C' | 'y' | 'Y' | 'x' | 'X') {
                            fail!();
                        }
                        continue;
                    }
                    'O' => {
                        let o = char::from_u32(fat(f)).unwrap_or('\0');
                        f += 1;
                        match o {
                            'd' | 'e' => {
                                self.tm.mday = get_number!(1, 31, 2);
                                have_mday = true;
                                want_xday = true;
                            }
                            'H' => {
                                self.tm.hour = get_number!(0, 23, 2);
                                have_i = false;
                            }
                            'I' => {
                                self.tm.hour = get_number!(1, 12, 2) % 12;
                                have_i = true;
                            }
                            'm' => {
                                self.tm.mon = get_number!(1, 12, 2) - 1;
                                have_mon = true;
                                want_xday = true;
                            }
                            'M' => self.tm.min = get_number!(0, 59, 2),
                            'S' => {
                                if fat(f) >= '0' as u32 && fat(f) <= '9' as u32 {
                                    f += 1;
                                }
                                let rest = &s[rp.min(s.len())..];
                                let (sval, used) = if self.wide {
                                    match clib::wcstod_of(rest) {
                                        Some(r) => r,
                                        None => {
                                            return Err(StrptimeError(
                                                "robigo: strptime()'s %OS without a C.UTF-8 locale in the C \
                                                 library is not implemented yet"
                                                    .to_owned(),
                                            ));
                                        }
                                    }
                                } else {
                                    // R_strtod, to read "Inf"
                                    let b: Vec<u8> = rest.iter().map(|&c| c as u8).collect();
                                    robigo_text::strtod::r_strtod(&b)
                                };
                                if (0.0..=61.0).contains(&sval) {
                                    self.tm.sec = c_int(sval);
                                    *self.psecs = sval;
                                } else if sval.is_infinite() {
                                    *self.psecs = sval;
                                } else {
                                    self.tm.sec = NA_INTEGER;
                                    *self.psecs = NA_REAL;
                                }
                                rp += used;
                            }
                            'U' => {
                                week_no = get_number!(0, 53, 2);
                                have_uweek = true;
                            }
                            'W' => {
                                week_no = get_number!(0, 53, 2);
                                have_wweek = true;
                            }
                            'V' => {
                                let _ = get_number!(0, 53, 2);
                            }
                            'w' => {
                                self.tm.wday = get_number!(0, 6, 1);
                                have_wday = true;
                            }
                            'y' => {
                                let ival = get_number!(0, 99, 2);
                                self.tm.year = if ival >= 69 { ival } else { ival + 100 };
                                want_xday = true;
                            }
                            _ => fail!(),
                        }
                    }
                    _ => fail!(),
                }
                break;
            }
        }
        let tm = &mut *self.tm;
        if have_i && is_pm {
            tm.hour += 12;
        }
        if century != -1 {
            tm.year = if want_century { tm.year % 100 + (century - 19) * 100 } else { (century - 19) * 100 };
        }
        if want_xday && !have_wday {
            if !(have_mon && have_mday) && have_yday {
                // have_yday, so this must have come from %j
                let mut t_mon = 0usize;
                let yr = 1900i32.wrapping_add(tm.year);
                let leap = gisleap(yr) as usize;
                if tm.yday > if gisleap(yr) { 365 } else { 364 } {
                    self.warnings.push(format!("day-of-year {} in year {} is invalid\n", tm.yday + 1, yr));
                    t_mon = 12; // this will give an invalid mday, so invalid tm
                } else {
                    while MON_YDAY[leap][t_mon] <= tm.yday {
                        t_mon += 1;
                    }
                }
                if !have_mon {
                    tm.mon = t_mon as i32 - 1;
                }
                if !have_mday {
                    tm.mday = mon_yday_prev(leap, t_mon).map_or(NA_INTEGER, |d| tm.yday - d + 1);
                }
            }
            day_of_the_week(tm);
        }
        if want_xday && !have_yday {
            day_of_the_year(tm);
        }
        if (have_uweek || have_wweek) && have_wday {
            let (save_wday, save_mday, save_mon) = (tm.wday, tm.mday, tm.mon);
            let w_offset = if have_uweek { 0 } else { 1 };
            tm.mday = 1;
            tm.mon = 0;
            day_of_the_week(tm);
            if have_mday {
                tm.mday = save_mday;
            }
            if have_mon {
                tm.mon = save_mon;
            }
            if !have_yday {
                // yday from week and day-of-the-week (not validated)
                let del = save_wday - w_offset;
                tm.yday = (7 - (tm.wday - w_offset)) % 7 + (week_no - (del >= 0) as i32) * 7 + del;
            }
            if !have_mday || !have_mon {
                let mut t_mon = 0usize;
                let yr = 1900i32.wrapping_add(tm.year);
                let leap = gisleap(yr) as usize;
                if tm.yday > if gisleap(yr) { 365 } else { 364 } {
                    self.warnings.push(format!("(0-based) yday {} in year {} is invalid\n", tm.yday, yr));
                    t_mon = 12;
                } else {
                    while MON_YDAY[leap][t_mon] <= tm.yday {
                        t_mon += 1;
                    }
                }
                if !have_mon {
                    tm.mon = t_mon as i32 - 1;
                }
                if !have_mday {
                    tm.mday = mon_yday_prev(leap, t_mon).map_or(NA_INTEGER, |d| tm.yday - d + 1);
                }
            }
            tm.wday = save_wday;
        }
        Ok(Some(rp))
    }

    /// `recursive(new_fmt)`.
    fn recursive(&mut self, s: &[u32], rp: usize, fmt: &str) -> Result<Option<usize>, StrptimeError> {
        let f: Vec<u32> = fmt.chars().map(|c| c as u32).collect();
        self.internal(s, rp, &f)
    }
}

/// `R_strptime(buf, format, tm, psecs, poffset)`: whether `buf` matched
/// `format`. In a multibyte locale both are converted to wide characters
/// first (`mbstowcs`, at most 1000 of them).
// R: Rstrptime.h:R_strptime
fn r_strptime(p: &mut Strptime<'_>, buf: &[u8], format: &[u8]) -> Result<bool, StrptimeError> {
    let (b, f): (Vec<u32>, Vec<u32>) = if p.wide {
        let wide = |x: &[u8], invalid: &str, long: &str| -> Result<Vec<u32>, StrptimeError> {
            let s = std::str::from_utf8(x).map_err(|_| StrptimeError(invalid.to_owned()))?;
            let w: Vec<u32> = s.chars().map(|c| c as u32).collect();
            if w.len() > 1000 {
                return Err(StrptimeError(long.to_owned()));
            }
            Ok(w)
        };
        let b = wide(buf, "invalid multibyte input string", "input string is too long")?;
        let f = wide(format, "invalid multibyte format string", "format string is too long")?;
        (b, f)
    } else {
        (buf.iter().map(|&c| c as u32).collect(), format.iter().map(|&c| c as u32).collect())
    };
    // (a C string ends at its first NUL)
    let cut = |v: Vec<u32>| -> Vec<u32> {
        match v.iter().position(|&c| c == 0) {
            Some(k) => v[..k].to_vec(),
            None => v,
        }
    };
    let (b, f) = (cut(b), cut(f));
    Ok(p.internal(&b, 0, &f)?.is_some())
}

// --- the builtins -------------------------------------------------------

/// `GET_tz_n_CHECK`'s `tz`: the `tz` argument, or `TZ` for "", and
/// whether it is UTC (`isUTC`).
// R: datetime.c:GET_tz_n_CHECK
fn get_tz(cx: &mut Cx, stz: Root<'_>) -> RResult<(Vec<u8>, bool)> {
    let tz = match stz.bind(cx).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'tz' value")),
    };
    let tz = if tz.is_empty() {
        use std::os::unix::ffi::OsStrExt;
        std::env::var_os("TZ").map(|p| p.as_bytes().to_vec()).unwrap_or_default()
    } else {
        tz
    };
    let is_utc = tz == b"GMT" || tz == b"UTC";
    Ok((tz, is_utc))
}

/// `GET_tz_n_CHECK`'s `set_tz(tz, &tzsi)` when the zone is not UTC: the
/// session the conversions run in, or None for UTC (gmtime, timegm00).
// R: datetime.c:GET_tz_n_CHECK, set_tz
fn open_tz<'c>(cx: &'c Cx, tz: &[u8], is_utc: bool) -> Result<Option<clib::TzSession<'c>>, TzOpenError> {
    if is_utc {
        return Ok(None);
    }
    let w = clib::env_writer(cx).map_err(TzOpenError::Refused)?;
    match w.set_tz(tz) {
        Ok(s) => Ok(Some(s)),
        Err(clib::SetTzError::TooLong) => Err(TzOpenError::TooLong),
    }
}

/// Why [`open_tz`] made no session.
enum TzOpenError {
    Refused(clib::EnvWriteRefused),
    TooLong,
}

/// [`open_tz`]'s failure as the error GNU R raises (`set_tz`'s), or
/// robigo's when the process environment may not be written.
fn tz_open_error(cx: &mut Cx, e: TzOpenError, what: &str) -> robigo_rtcore::RError {
    match e {
        TzOpenError::TooLong => cx.error("time zone specification is too long"),
        TzOpenError::Refused(r) => not_yet(
            cx,
            None,
            &format!(
                "{what} in a time zone other than UTC or GMT without the process environment's writes ({r:?}): GNU R sets TZ"
            ),
        ),
    }
}

/// `set_tz`'s warning, once the session has ended.
fn tz_warn(cx: &mut Cx, set_failed: bool) -> RResult<()> {
    if set_failed {
        cx.warning("problem with setting timezone")?;
    }
    Ok(())
}

/// `.Internal(strptime(x, format, tz))`: `x` read with `format`, a "POSIXlt"
/// list of 11 components, recycled to the longer of the two.
// R: datetime.c:do_strptime
pub fn do_strptime(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("strptime", true).check_arity(cx, call, args)?;
    if args.get(cx, 0).sexptype() != STRSXP {
        return Err(cx.error("invalid 'x' argument"));
    }
    let sformat = args.get(cx, 1);
    if sformat.sexptype() != STRSXP || sformat.len() == Some(0) {
        return Err(cx.error("invalid 'format' argument"));
    }
    let (tz, is_utc) = get_tz(cx, args.root(2))?;
    if !is_utc {
        let shown = if tz.is_empty() { "the system's".to_owned() } else { String::from_utf8_lossy(&tz).into_owned() };
        return Err(not_yet(cx, None, &format!("strptime() in a time zone other than UTC or GMT ({shown})")));
    }
    let n = args.get(cx, 0).len().unwrap_or(0);
    let m = args.get(cx, 1).len().unwrap_or(0);
    let nn = if n > 0 { m.max(n) } else { 0 };
    let loc = cx.locale();
    let wide = loc.utf8;
    let mut rows: Vec<(Tm, bool, f64)> = Vec::with_capacity(nn);
    for i in 0..nn {
        // (the strings, translated, before anything can collect)
        let tr = {
            let x = args.get(cx, 0).as_vector::<Char<'_>>().unwrap().get(i % n);
            let f = args.get(cx, 1).as_vector::<Char<'_>>().unwrap().get(i % m);
            if x == cx.na_string() {
                None
            } else {
                let tr = |c: Char<'_>| crate::text::to_native(crate::coerce::text(c), loc).map(|b| b.into_owned());
                Some((tr(x), tr(f)))
            }
        };
        let (na, xs, fs) = match tr {
            None => (true, Vec::new(), Vec::new()),
            Some((Ok(a), Ok(b))) => (false, a, b),
            Some(_) => return Err(cx.error("translating strings with \"bytes\" encoding is not allowed")),
        };
        let mut tm = Tm {
            sec: 0,
            min: 0,
            hour: 0,
            year: NA_INTEGER,
            mon: NA_INTEGER,
            mday: NA_INTEGER,
            yday: NA_INTEGER,
            wday: NA_INTEGER,
            gmtoff: NA_INTEGER as i64,
            isdst: -1,
        };
        let mut psecs = 0.0f64;
        let mut offset = NA_INTEGER;
        let mut invalid = na;
        if !na {
            let mut p = Strptime {
                tm: &mut tm,
                psecs: &mut psecs,
                poffset: &mut offset,
                wide,
                loc: crate::text::ctype(loc),
                warnings: Vec::new(),
            };
            let r = r_strptime(&mut p, &xs, &fs);
            let warnings = std::mem::take(&mut p.warnings);
            for w in warnings {
                cx.warning(&w)?;
            }
            match r {
                Ok(ok) => invalid = !ok,
                Err(StrptimeError(m)) => return Err(cx.error(&m)),
            }
        }
        let mut ptm: Option<Tm> = None; // ptm = &tm2 when set
        if !invalid {
            // Solaris sets missing fields to 0
            if tm.mday == 0 {
                tm.mday = NA_INTEGER;
            }
            if tm.mon == NA_INTEGER || tm.mday == NA_INTEGER || tm.year == NA_INTEGER {
                glibc_fix(&mut tm, &mut invalid);
            }
            tm.isdst = -1;
            if offset != NA_INTEGER {
                // the offset is known but not the time zone: to time_t,
                // adjusted, and back
                // (GNU R's `invalid = true` when mktime0 fails is overwritten
                // by validate_tm's verdict below, so it is not set here)
                let mut tm2 = tm.clone();
                let (t0, _) = mktime0_utc(&mut tm2);
                if t0 != -1.0 {
                    // localtime0(&t0, !isUTC = 0, &tm2): gmtime_r. A failed
                    // one leaves `ptm` NULL in GNU R; a validated tm_year
                    // always fits, and the year was validated
                    if let Some(t) = gmtime0(t0 - offset as f64) {
                        ptm = Some(t);
                    }
                }
            } else {
                // wday, yday, isdst set, without adjusting at DST boundaries
                let mut tm2 = tm.clone();
                tm.isdst = 0; // isUTC
                if mktime0_utc(&mut tm2).0 != -1.0 {
                    tm.wday = tm2.wday;
                    tm.yday = tm2.yday;
                }
            }
            invalid = validate_tm(&mut tm) != 0;
        }
        let frac = if invalid { NA_REAL } else { psecs - psecs.floor() };
        rows.push((ptm.unwrap_or(tm), !invalid, frac));
    }
    let x = args.root(0);
    let rows: Vec<LtRow> = rows
        .into_iter()
        .map(|(tm, valid, frac)| LtRow { tm, valid, frac, zone: tz.clone(), gmtoff: 0 }) // isUTC
        .collect();
    cx.build(out, |cx| {
        // END_MAKElt: the year's names are x's (recycled when N > n)
        let year_names = match crate::attrib::names(x.bind(cx)).and_then(|v| v.as_vector::<Char<'_>>()) {
            Some(nm) if nn > n => {
                let mut nm3 = cx.alloc_vector::<Char<'_>>(nn)?;
                for j in 0..nn {
                    nm3.set(j, nm.get(j % n));
                }
                Some(Ref::from(nm3.finish()))
            }
            Some(_) => crate::attrib::names(x.bind(cx)),
            None => None,
        };
        make_lt(cx, &rows, std::slice::from_ref(&tz), year_names)
    })
}

/// One element of a "POSIXlt" result: `makelt(tm, ans, i, valid,
/// frac_secs)`'s arguments, and the zone and `gmtoff` stored with it.
struct LtRow {
    tm: Tm,
    valid: bool,
    frac: f64,
    zone: Vec<u8>,
    gmtoff: i32,
}

/// The "POSIXlt" list of `rows` (`makelt` for each, the zone and gmtoff
/// components), then `END_MAKElt`: its names, class `c("POSIXlt",
/// "POSIXt")`, the `tzone` attribute, the year component named
/// `year_names`, and `balanced = TRUE`.
// R: datetime.c:makelt, END_MAKElt
fn make_lt<'g>(
    cx: &'g Cx,
    rows: &[LtRow],
    tzone: &[Vec<u8>],
    year_names: Option<Ref<'g>>,
) -> Result<Ref<'g>, robigo_rtcore::NeedGc> {
    let nn = rows.len();
    let mut sec = cx.alloc_vector::<f64>(nn)?;
    let mut ints: Vec<_> = (0..8).map(|_| cx.alloc_vector::<i32>(nn)).collect::<Result<Vec<_>, _>>()?;
    let mut zone = cx.alloc_vector::<Char<'_>>(nn)?;
    let mut gmtoff = cx.alloc_vector::<i32>(nn)?;
    for (i, r) in rows.iter().enumerate() {
        let t = &r.tm;
        if r.valid {
            sec.as_mut_slice()[i] = t.sec as f64 + r.frac;
            for (k, v) in [t.min, t.hour, t.mday, t.mon, t.year, t.wday, t.yday, t.isdst].into_iter().enumerate() {
                ints[k].as_mut_slice()[i] = v;
            }
        } else {
            sec.as_mut_slice()[i] = r.frac;
            for v in ints.iter_mut().take(7) {
                v.as_mut_slice()[i] = NA_INTEGER;
            }
            ints[7].as_mut_slice()[i] = -1; // isdst
        }
        zone.set(i, cx.mk_char(&r.zone, Encoding::Native)?);
        gmtoff.as_mut_slice()[i] = r.gmtoff;
    }
    let mut comps: Vec<Ref<'_>> = Vec::with_capacity(11);
    comps.push(sec.finish().into());
    for (k, v) in ints.into_iter().enumerate() {
        if k == 4
            && let Some(nm) = year_names
        {
            let mut v = v;
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
            a.apply(cx, &mut v)?;
            comps.push(v.finish().into());
        } else {
            comps.push(v.finish().into());
        }
    }
    comps.push(zone.finish().into());
    comps.push(gmtoff.finish().into());
    let mut ans = cx.alloc_vector::<Ref<'_>>(11)?;
    for (i, c) in comps.into_iter().enumerate() {
        ans.set(i, c);
    }
    let mut names = cx.alloc_vector::<Char<'_>>(11)?;
    for (i, nm) in LTNAMES.iter().enumerate() {
        names.set(i, cx.mk_char(nm, Encoding::Native)?);
    }
    let mut class = cx.alloc_vector::<Char<'_>>(2)?;
    class.set(0, cx.mk_char(b"POSIXlt", Encoding::Native)?);
    class.set(1, cx.mk_char(b"POSIXt", Encoding::Native)?);
    let mut tzv = cx.alloc_vector::<Char<'_>>(tzone.len())?;
    for (i, t) in tzone.iter().enumerate() {
        tzv.set(i, cx.mk_char(t, Encoding::Native)?);
    }
    let mut a = Attrs::new();
    a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
    a.set_class(cx.sym(RSym::Class), Some(class.finish().into())); // classgets
    a.install(Attr { tag: cx.install(b"tzone")?, value: tzv.finish().into() });
    a.install(Attr { tag: cx.install(b"balanced")?, value: cx.logical_value(1) });
    a.apply(cx, &mut ans)?;
    Ok(ans.finish().into())
}

/// `valid_POSIXlt(x, nm)`'s checks, and the components coerced as it
/// coerces them (`sec` to double, the next ones to integer).
// R: datetime.c:valid_POSIXlt
fn valid_posixlt<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>, nm: usize) -> RResult<Vec<Root<'s>>> {
    let xv = x.bind(cx);
    let n_comp = xv.len().unwrap_or(0);
    let n_check = n_comp.min(nm);
    if xv.sexptype() != VECSXP || n_comp < 9 {
        return Err(cx.error("a valid \"POSIXlt\" object is a list of at least 9 elements"));
    }
    let nms = crate::attrib::names(xv).and_then(|v| v.as_vector::<Char<'_>>());
    if nms.map_or(0, |v| v.len()) < 9 {
        return Err(cx.error("a valid \"POSIXlt\" object has names"));
    }
    let nms = nms.unwrap();
    for (i, want) in LTNAMES.iter().enumerate().take(n_check) {
        let got = nms.get(i).bytes();
        if got != *want {
            let msg = format!(
                "a valid \"POSIXlt\" object has element {} with name '{}' which should be '{}'",
                i + 1,
                String::from_utf8_lossy(got),
                String::from_utf8_lossy(want)
            );
            return Err(cx.error(&msg));
        }
    }
    let elts = xv.as_vector::<Ref<'_>>().unwrap();
    for (i, nm) in LTNAMES.iter().enumerate().take(9.min(nm)) {
        if !matches!(elts.get(i).sexptype(), INTSXP | REALSXP) {
            let msg = format!("a valid \"POSIXlt\" object has a numeric element {}", String::from_utf8_lossy(nm));
            return Err(cx.error(&msg));
        }
    }
    let mut comps = Vec::with_capacity(n_check);
    for i in 0..n_check {
        let c = {
            let e = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i);
            s.root(cx, e)
        };
        let c = if i == 9 {
            c
        } else {
            s.capture(cx, |cx, o| {
                crate::coerce::vector::coerce_vector(cx, c, if i == 0 { REALSXP } else { INTSXP }, o)
            })?
        };
        comps.push(c);
    }
    if n_check >= 10 && comps[9].bind(cx).sexptype() != STRSXP {
        return Err(cx.error("a valid \"POSIXlt\" object has a character element zone"));
    }
    if n_check >= 11 {
        let g = x.bind(cx).as_vector::<Ref<'_>>().unwrap().get(10);
        if !matches!(g.sexptype(), INTSXP | REALSXP) {
            return Err(cx.error("a valid \"POSIXlt\" object has a numeric element gmtoff"));
        }
    }
    if let Some(tz) = crate::attrib::get_value(x.bind(cx), b"tzone") {
        if tz.sexptype() != STRSXP {
            return Err(cx.error("invalid 'attr(x, \"tzone\")'"));
        }
        let l = tz.len().unwrap_or(0);
        if l != 1 && l != 3 {
            return Err(cx.error("attr(x, \"tzone\") should have length 1 or 3"));
        }
    }
    Ok(comps)
}

/// `.Internal(as.POSIXct(x, tz))`: a "POSIXlt" list as seconds since the
/// epoch, of class "POSIXct".
// R: datetime.c:do_asPOSIXct
pub fn do_as_posixct(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("as.POSIXct", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let comps = valid_posixlt(cx, s, args.root(0), 9)?;
        let (tz, is_utc) = get_tz(cx, args.root(1))?;
        let len = |cx: &Cx, i: usize| comps[i].bind(cx).len().unwrap_or(0);
        let mut nlen = [0usize; 9];
        let mut n = 0;
        for i in (0..6).chain([8]) {
            nlen[i] = len(cx, i);
            n = n.max(nlen[i]);
        }
        if n > 0 {
            for i in (0..6).chain([8]) {
                if nlen[i] == 0 {
                    let msg = format!("zero-length component [[{}]] in non-empty \"POSIXlt\" structure", i + 1);
                    return Err(cx.error(&msg));
                }
            }
        }
        let converted = {
            let cx: &Cx = cx;
            match open_tz(cx, &tz, is_utc) {
                Err(e) => Err(e),
                Ok(sess) => {
                    let mut ans = Vec::with_capacity(n);
                    for i in 0..n {
                        let real = |cx: &Cx, k: usize| match comps[k].bind(cx).view() {
                            View::Real(v) => v.as_slice().unwrap()[i % nlen[k]],
                            _ => unreachable!("coerced"),
                        };
                        let int = |cx: &Cx, k: usize| match comps[k].bind(cx).view() {
                            View::Int(v) => v.as_slice().unwrap()[i % nlen[k]],
                            _ => unreachable!("coerced"),
                        };
                        let secs = real(cx, 0);
                        let fsecs = secs.floor();
                        let mut tm = Tm {
                            // avoid (int) NAN
                            sec: if secs.is_finite() { c_int(fsecs) } else { NA_INTEGER },
                            min: int(cx, 1),
                            hour: int(cx, 2),
                            mday: int(cx, 3),
                            mon: int(cx, 4),
                            year: int(cx, 5),
                            // mktime ignores tm.tm_wday and tm.tm_yday
                            isdst: if is_utc { 0 } else { int(cx, 8) },
                            ..Tm::default()
                        };
                        let v = if !secs.is_finite() {
                            secs
                        } else if [tm.min, tm.hour, tm.mday, tm.mon, tm.year].contains(&NA_INTEGER) {
                            NA_REAL
                        } else {
                            // MKTIME_SETS_ERRNO: -1 is an error only with errno set
                            let (tmp, errno) = mktime0(&mut tm, sess.as_ref());
                            if tmp == -1.0 && errno { NA_REAL } else { tmp + (secs - fsecs) }
                        };
                        ans.push(v);
                    }
                    let set_failed = sess.as_ref().is_some_and(|s| s.set_failed);
                    // reset_tz
                    drop(sess);
                    Ok((ans, set_failed))
                }
            }
        };
        let (ans, set_failed) = match converted {
            Ok(v) => v,
            Err(e) => return Err(tz_open_error(cx, e, "as.POSIXct()")),
        };
        tz_warn(cx, set_failed)?;
        cx.build(out, |cx| {
            let mut v = cx.alloc_copy(&ans)?;
            let mut a = Attrs::new();
            if let Some(nm) = crate::attrib::names(comps[5].bind(cx)) {
                a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
            }
            let mut class = cx.alloc_vector::<Char<'_>>(2)?;
            class.set(0, cx.mk_char(b"POSIXct", Encoding::Native)?);
            class.set(1, cx.mk_char(b"POSIXt", Encoding::Native)?);
            a.set_class(cx.sym(RSym::Class), Some(class.finish().into())); // classgets
            a.apply(cx, &mut v)?;
            Ok(v.finish().into())
        })
    })
}

/// `coerceVector(x, REALSXP)` (its warnings given) as plain doubles, and
/// `x`'s names (`getAttrib(x, R_NamesSymbol)`, which the coercion keeps).
fn coerce_real<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>) -> RResult<(Vec<f64>, Root<'s>)> {
    let xr = s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, x, REALSXP, o))?;
    let v = xr.bind(cx).as_vector::<f64>().unwrap();
    let xs: Vec<f64> = match v.dataptr_or_null() {
        Some(sl) => sl.to_vec(),
        None => (0..v.len()).map(|i| v.get(i)).collect(),
    };
    Ok((xs, xr))
}

/// `.Internal(as.POSIXlt(x, tz))`: seconds since the epoch as a "POSIXlt"
/// list, in UTC (`gmtime_r`) or, for any other zone, the C library's
/// `localtime_r` with `TZ` set to it.
// R: datetime.c:do_asPOSIXlt
pub fn do_as_posixlt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("as.POSIXlt", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let (xs, x) = coerce_real(cx, s, args.root(0))?;
        // GET_tz_CHECK_n_TZONE
        let (tz, is_utc) = get_tz(cx, args.root(1))?;
        let converted = {
            let cx: &Cx = cx;
            match open_tz(cx, &tz, is_utc) {
                Err(e) => Err(e),
                Ok(sess) => {
                    // SET_TZONE: now, in case the conversions change it
                    let tzone: Vec<Vec<u8>> = match &sess {
                        None => vec![tz.clone()],
                        Some(s) => {
                            let [a, b] = s.tzname();
                            vec![tz.clone(), a, b]
                        }
                    };
                    let rows: Vec<LtRow> = xs
                        .iter()
                        .map(|&d| {
                            let ptm = if d.is_finite() { localtime0(d, sess.as_ref()) } else { None };
                            let valid = ptm.is_some();
                            let frac = if valid { d - d.floor() } else { d };
                            let tm = ptm.unwrap_or_default();
                            let (zone, gmtoff) = if is_utc {
                                (tz.clone(), 0)
                            } else {
                                // R_tzname[tm_isdst] as this conversion left it (glibc's
                                // localtime_r sets tzname for the time converted); or
                                // ptm->tm_zone, which POSIX does not specify
                                let zone = match &sess {
                                    Some(s) if valid && tm.isdst >= 0 => {
                                        let [a, b] = s.tzname();
                                        if tm.isdst == 0 { a } else { b }
                                    }
                                    _ => Vec::new(),
                                };
                                (zone, if valid { tm.gmtoff as i32 } else { NA_INTEGER })
                            };
                            LtRow { tm, valid, frac, zone, gmtoff }
                        })
                        .collect();
                    let set_failed = sess.as_ref().is_some_and(|s| s.set_failed);
                    // reset_tz
                    drop(sess);
                    Ok(((tzone, rows), set_failed))
                }
            }
        };
        let ((tzone, rows), set_failed) = match converted {
            Ok(v) => v,
            Err(e) => return Err(tz_open_error(cx, e, "as.POSIXlt()")),
        };
        tz_warn(cx, set_failed)?;
        cx.build(out, |cx| make_lt(cx, &rows, &tzone, crate::attrib::names(x.bind(cx))))
    })
}

/// `.Internal(Date2POSIXlt(x, tz))`: days since the epoch as a "POSIXlt"
/// list at midnight UTC, its zone `tz` ("" is "UTC").
// R: datetime.c:do_D2POSIXlt
pub fn do_d2posixlt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Date2POSIXlt", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let (xs, x) = coerce_real(cx, s, args.root(0))?;
        let stz = args.get(cx, 1);
        let tz: Vec<u8> = match stz.as_vector::<Char<'_>>() {
            Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
            _ => return Err(cx.error("invalid 'tz' value")),
        };
        let tz = if tz.is_empty() { b"UTC".to_vec() } else { tz };
        let rows: Vec<LtRow> = xs
            .iter()
            .map(|&x_i| {
                let mut tm = Tm::default();
                let mut valid = x_i.is_finite();
                if valid {
                    // every 400 years is exactly 146097 days long and the
                    // pattern is repeated
                    let rounds = (x_i.floor() / 146097.0).floor();
                    let mut day = c_int(x_i.floor() - 146097.0 * rounds);
                    // weekday: 1970-01-01 was a Thursday
                    tm.wday = ((day % 7) + 4) % 7;
                    if tm.wday < 0 {
                        tm.wday += 7;
                    }
                    // year & day within year
                    let mut y: i32 = 1970;
                    if day >= 0 {
                        loop {
                            let tmp = days_in_year(y);
                            if day < tmp {
                                break;
                            }
                            day -= tmp;
                            y += 1;
                        }
                    } else {
                        while day < 0 {
                            y -= 1;
                            day += days_in_year(y);
                        }
                    }
                    // Avoid overflows
                    let year0 = (y - 1900) as f64 + rounds * 400.0;
                    if year0 > i32::MAX as f64 || year0 < i32::MIN as f64 {
                        valid = false;
                    } else {
                        let y = year0 as i32;
                        tm.year = y;
                        tm.yday = day;
                        // month within year
                        let mut mon = 0;
                        loop {
                            let tmp = days_in_month(mon, y);
                            if day < tmp {
                                break;
                            }
                            day -= tmp;
                            mon += 1;
                        }
                        tm.mon = mon;
                        tm.mday = day + 1;
                        tm.isdst = 0; // no dst in GMT
                    }
                }
                LtRow { tm, valid, frac: if valid { 0.0 } else { x_i }, zone: tz.clone(), gmtoff: 0 }
            })
            .collect();
        cx.build(out, |cx| make_lt(cx, &rows, std::slice::from_ref(&tz), crate::attrib::names(x.bind(cx))))
    })
}

/// `.Internal(POSIXlt2Date(x))`: a "POSIXlt" list as days since the epoch,
/// of class "Date".
// R: datetime.c:do_POSIXlt2D
pub fn do_posixlt2d(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("POSIXlt2Date", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let comps = valid_posixlt(cx, s, args.root(0), 6)?;
        let mut nlen = [0usize; 9];
        let mut n = 0;
        for i in 0..6 {
            nlen[i] = comps[i].bind(cx).len().unwrap_or(0);
            n = n.max(nlen[i]);
        }
        // isdst, not coerced (valid_POSIXlt(x, 6) checked it is numeric)
        let isdst = {
            let x = args.get(cx, 0);
            x.as_vector::<Ref<'_>>().unwrap().get(8)
        };
        if isdst.is_package_altrep() {
            return Err(not_yet(cx, Some(call), "POSIXlt2Date() of a package ALTREP 'isdst'"));
        }
        nlen[8] = crate::coerce::xlength(isdst);
        n = n.max(nlen[8]);
        if n > 0 {
            for i in (0..6).chain([8]) {
                if nlen[i] == 0 {
                    let msg = format!("zero-length component [[{}]] in non-empty \"POSIXlt\" structure", i + 1);
                    return Err(cx.error(&msg));
                }
            }
        }
        let mut ans = Vec::with_capacity(n);
        for i in 0..n {
            let real = |cx: &Cx, k: usize| match comps[k].bind(cx).view() {
                View::Real(v) => v.get(i % nlen[k]),
                _ => unreachable!("coerced"),
            };
            let int = |cx: &Cx, k: usize| match comps[k].bind(cx).view() {
                View::Int(v) => v.get(i % nlen[k]),
                _ => unreachable!("coerced"),
            };
            // need to treat {sec, min, hour} in the out-of-range case
            // (where the fixup may change the day, month, ...)
            let secs = real(cx, 0);
            let fsecs = secs.floor();
            let mut tm = Tm {
                // avoid (int) NAN
                sec: if secs.is_finite() { c_int(fsecs) } else { NA_INTEGER },
                min: int(cx, 1),
                hour: int(cx, 2),
                mday: int(cx, 3),
                mon: int(cx, 4),
                year: int(cx, 5),
                isdst: 0,
                ..Tm::default()
            };
            let v = if !secs.is_finite() {
                secs
            } else if [tm.min, tm.hour, tm.mday, tm.mon, tm.year].contains(&NA_INTEGER) {
                NA_REAL
            } else if validate_tm(&mut tm) < 0 {
                NA_REAL
            } else {
                mkdate00(&mut tm)
            };
            ans.push(v);
        }
        cx.build(out, |cx| {
            let mut v = cx.alloc_copy(&ans)?;
            let mut a = Attrs::new();
            if let Some(nm) = crate::attrib::names(comps[5].bind(cx)) {
                a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
            }
            let mut class = cx.alloc_vector::<Char<'_>>(1)?;
            class.set(0, cx.mk_char(b"Date", Encoding::Native)?);
            a.set_class(cx.sym(RSym::Class), Some(class.finish().into())); // classgets
            a.apply(cx, &mut v)?;
            Ok(v.finish().into())
        })
    })
}
