//! R_strptime: a port of R's src/main/Rstrptime.h (a modified glibc
//! strptime). R uses the wide-character variant `w_strptime_internal` in
//! multibyte (UTF-8) locales, so that is what is ported here, working on
//! `char`s.

use super::tz::{self, Tm};
use crate::value::NA_INTEGER;

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

const MON_YDAY: [[i32; 13]; 2] = tz::MON_YDAY;

fn isleap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// `iswspace` in a UTF-8 locale.
pub fn iswspace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r')
        || matches!(c, '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{205f}' | '\u{3000}')
}

/// Compute the day of the week.
fn day_of_the_week(tm: &mut Tm) {
    // R bug fix: day_of_the_week needs year, month, mday set
    if tm.year == NA_INTEGER || tm.mon == NA_INTEGER || tm.mday == NA_INTEGER {
        return;
    }
    let year = tm.year as i64;
    let corr_year = 1900 + year - (tm.mon < 2) as i64;
    let q = corr_year / 4;
    let wday = -473 + 365 * (year - 70) + q - q / 25
        + ((q % 25) < 0) as i64
        + (q / 25) / 4
        + MON_YDAY[0][tm.mon.clamp(0, 12) as usize] as i64
        + tm.mday as i64
        - 1;
    tm.wday = ((wday % 7 + 7) % 7) as i32;
}

/// Compute the day of the year.
fn day_of_the_year(tm: &mut Tm) {
    if tm.year == NA_INTEGER || tm.mon == NA_INTEGER || tm.mday == NA_INTEGER {
        return;
    }
    let leap = isleap((1900i64 + tm.year as i64) as i32) as usize;
    tm.yday = MON_YDAY[leap][tm.mon.clamp(0, 12) as usize] + (tm.mday - 1);
}

/// Case-insensitive prefix match (`w_match_string`); advances `rp`.
fn match_string(cs1: &str, s: &[char], rp: &mut usize) -> bool {
    let n = cs1.chars().count();
    let mut i = *rp;
    for a in cs1.chars() {
        let Some(&b) = s.get(i) else { return false };
        if a.to_lowercase().ne(b.to_lowercase()) {
            return false;
        }
        i += 1;
    }
    *rp += n;
    true
}

/// `wcstod` on `s[rp..]`: the value and the end index (`rp` itself if no
/// conversion could be performed).
fn wcstod(s: &[char], rp: usize) -> (f64, usize) {
    let mut i = rp;
    while i < s.len() && iswspace(s[i]) {
        i += 1;
    }
    let start = i;
    if i < s.len() && (s[i] == '+' || s[i] == '-') {
        i += 1;
    }
    let rest: String = s[i..].iter().take(9).collect::<String>().to_ascii_lowercase();
    for (word, val) in [("infinity", f64::INFINITY), ("inf", f64::INFINITY), ("nan", f64::NAN)] {
        if rest.starts_with(word) {
            let mut end = i + word.len();
            if word == "nan" && s.get(end) == Some(&'(') {
                let mut j = end + 1;
                while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == '_') {
                    j += 1;
                }
                if s.get(j) == Some(&')') {
                    end = j + 1;
                }
            }
            let neg = s[start] == '-';
            return (if neg { -val } else { val }, end);
        }
    }
    // Hexadecimal
    if rest.starts_with("0x") {
        let mut j = i + 2;
        let mut mant = 0f64;
        let mut nd = 0;
        let mut exp: i64 = 0;
        while j < s.len() && s[j].is_ascii_hexdigit() {
            mant = mant * 16.0 + s[j].to_digit(16).unwrap() as f64;
            j += 1;
            nd += 1;
        }
        if j < s.len() && s[j] == '.' {
            j += 1;
            while j < s.len() && s[j].is_ascii_hexdigit() {
                mant = mant * 16.0 + s[j].to_digit(16).unwrap() as f64;
                exp -= 4;
                j += 1;
                nd += 1;
            }
        }
        if nd > 0 {
            if j < s.len() && (s[j] == 'p' || s[j] == 'P') {
                let mut k = j + 1;
                let mut eneg = false;
                if k < s.len() && (s[k] == '+' || s[k] == '-') {
                    eneg = s[k] == '-';
                    k += 1;
                }
                let ks = k;
                let mut e: i64 = 0;
                while k < s.len() && s[k].is_ascii_digit() {
                    e = (e * 10 + s[k].to_digit(10).unwrap() as i64).min(100000);
                    k += 1;
                }
                if k > ks {
                    exp += if eneg { -e } else { e };
                    j = k;
                }
            }
            let v = mant * 2f64.powi(exp.clamp(-100000, 100000) as i32);
            return (if s[start] == '-' { -v } else { v }, j);
        }
        // "0x" without digits parses as 0 followed by "x"
    }
    let mut j = i;
    let mut nd = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        nd += 1;
    }
    if j < s.len() && s[j] == '.' {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            nd += 1;
        }
    }
    if nd == 0 {
        return (0.0, rp);
    }
    if j < s.len() && (s[j] == 'e' || s[j] == 'E') {
        let mut k = j + 1;
        if k < s.len() && (s[k] == '+' || s[k] == '-') {
            k += 1;
        }
        let ks = k;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > ks {
            j = k;
        }
    }
    let text: String = s[start..j].iter().collect();
    (text.parse::<f64>().unwrap_or(0.0), j)
}

/// Results reported back to do_strptime.
pub struct Strptime {
    pub tm: Tm,
    pub psecs: f64,
    pub offset: i32,
}

pub enum Failure {
    /// The input does not match (R_strptime returned NULL).
    NoMatch,
    /// An R error (`error()` in the C code).
    Error(String),
}

/// `R_strptime(buf, format, tm, psecs, poffset)`; warnings raised by the C
/// code are appended to `warnings`.
pub fn r_strptime(buf: &str, format: &str, st: &mut Strptime, warnings: &mut Vec<String>) -> Result<(), Failure> {
    let wbuf: Vec<char> = buf.chars().collect();
    if wbuf.len() > 1000 {
        return Err(Failure::Error("input string is too long".into()));
    }
    let wfmt: Vec<char> = format.chars().collect();
    if wfmt.len() > 1000 {
        return Err(Failure::Error("format string is too long".into()));
    }
    let mut rp = 0usize;
    strptime_internal(&wbuf, &mut rp, &wfmt, st, warnings)
}

fn strptime_internal(
    s: &[char],
    rp: &mut usize,
    fmt: &[char],
    st: &mut Strptime,
    warnings: &mut Vec<String>,
) -> Result<(), Failure> {
    use Failure::NoMatch;
    let mut have_i = false;
    let mut is_pm = false;
    let mut century: i32 = -1;
    let mut want_century = false;
    let (mut have_wday, mut want_xday, mut have_yday) = (false, false, false);
    let (mut have_mon, mut have_mday) = (false, false);
    let (mut have_uweek, mut have_wweek) = (false, false);
    let mut week_no = 0;
    let at = |i: usize| s.get(i).copied().unwrap_or('\0');
    let mut f = 0usize;

    // get_number(from, to, n)
    fn get_number(s: &[char], rp: &mut usize, from: i32, to: i32, n: i32) -> Result<i32, Failure> {
        let at = |i: usize| s.get(i).copied().unwrap_or('\0');
        while at(*rp) == ' ' {
            *rp += 1;
        }
        if !at(*rp).is_ascii_digit() {
            return Err(Failure::NoMatch);
        }
        let mut val: i32 = 0;
        let mut k = n;
        loop {
            val = val.wrapping_mul(10).wrapping_add(at(*rp) as i32 - '0' as i32);
            *rp += 1;
            k -= 1;
            if !(k > 0 && at(*rp).is_ascii_digit()) {
                break;
            }
        }
        if val < from || val > to {
            return Err(Failure::NoMatch);
        }
        Ok(val)
    }

    macro_rules! recursive {
        ($sub:expr) => {{
            let sub: Vec<char> = $sub.chars().collect();
            strptime_internal(s, rp, &sub, st, warnings)?;
        }};
    }

    while f < fmt.len() {
        // A white space in the format string matches 0 more or white
        // space in the input string.
        if iswspace(fmt[f]) {
            while iswspace(at(*rp)) {
                *rp += 1;
            }
            f += 1;
            continue;
        }
        // Any character but `%' must be matched by the same character in
        // the input string.
        if fmt[f] != '%' {
            if fmt[f] != at(*rp) {
                return Err(NoMatch);
            }
            f += 1;
            *rp += 1;
            continue;
        }
        f += 1;
        let mut c = fmt.get(f).copied().unwrap_or('\0');
        f += 1;
        // We need this for handling the `E' modifier.
        loop {
            match c {
                '%' => {
                    if at(*rp) != '%' {
                        return Err(NoMatch);
                    }
                    *rp += 1;
                }
                'a' | 'A' => {
                    // try full name first
                    let mut cnt = (0..7).find(|&k| match_string(WEEKDAY_NAME[k], s, rp));
                    if cnt.is_none() {
                        cnt = (0..7).find(|&k| match_string(AB_WEEKDAY_NAME[k], s, rp));
                    }
                    let Some(cnt) = cnt else { return Err(NoMatch) };
                    st.tm.wday = cnt as i32;
                    have_wday = true;
                }
                'b' | 'B' | 'h' => {
                    let mut cnt = (0..12).find(|&k| match_string(MONTH_NAME[k], s, rp));
                    if cnt.is_none() {
                        cnt = (0..12).find(|&k| match_string(AB_MONTH_NAME[k], s, rp));
                    }
                    let Some(cnt) = cnt else { return Err(NoMatch) };
                    st.tm.mon = cnt as i32;
                    want_xday = true;
                }
                'c' => recursive!("%a %b %e %H:%M:%S %Y"),
                'C' => {
                    century = get_number(s, rp, 0, 99, 2)?;
                    want_xday = true;
                }
                'd' | 'e' => {
                    st.tm.mday = get_number(s, rp, 1, 31, 2)?;
                    have_mday = true;
                    want_xday = true;
                }
                'F' => {
                    recursive!("%Y-%m-%d");
                    want_xday = true;
                }
                'x' | 'D' => {
                    recursive!("%y/%m/%d");
                    want_xday = true;
                }
                'k' | 'H' => {
                    st.tm.hour = get_number(s, rp, 0, 24, 2)?; // allow 24:00:00
                    have_i = false;
                }
                'l' | 'I' => {
                    st.tm.hour = get_number(s, rp, 1, 12, 2)? % 12;
                    have_i = true;
                }
                'j' => {
                    st.tm.yday = get_number(s, rp, 1, 366, 3)? - 1;
                    have_yday = true;
                }
                'm' => {
                    st.tm.mon = get_number(s, rp, 1, 12, 2)? - 1;
                    have_mon = true;
                    want_xday = true;
                }
                'M' => st.tm.min = get_number(s, rp, 0, 59, 2)?,
                'n' | 't' => {
                    while iswspace(at(*rp)) {
                        *rp += 1;
                    }
                }
                'p' => {
                    if !match_string(AM_PM[0], s, rp) {
                        if match_string(AM_PM[1], s, rp) {
                            is_pm = true;
                        } else {
                            return Err(NoMatch);
                        }
                    }
                }
                'r' => recursive!("%I:%M:%S %p"),
                'R' => recursive!("%H:%M"),
                's' => {
                    // The number of seconds may be very high.
                    if !at(*rp).is_ascii_digit() {
                        return Err(NoMatch);
                    }
                    let mut secs: i64 = 0;
                    while at(*rp).is_ascii_digit() {
                        secs = secs.wrapping_mul(10).wrapping_add(at(*rp) as i64 - '0' as i64);
                        *rp += 1;
                    }
                    match tz::localtime_r(secs) {
                        Some(t) => st.tm = t,
                        None => return Err(NoMatch),
                    }
                }
                'S' => st.tm.sec = get_number(s, rp, 0, 61, 2)?,
                'X' | 'T' => recursive!("%H:%M:%S"),
                'u' => {
                    st.tm.wday = get_number(s, rp, 1, 7, 1)? % 7;
                    have_wday = true;
                }
                'g' => {
                    get_number(s, rp, 0, 99, 2)?;
                }
                'G' => {
                    if !at(*rp).is_ascii_digit() {
                        return Err(NoMatch);
                    }
                    while at(*rp).is_ascii_digit() {
                        *rp += 1;
                    }
                }
                'U' => {
                    week_no = get_number(s, rp, 0, 53, 2)?;
                    have_uweek = true;
                }
                'W' => {
                    week_no = get_number(s, rp, 0, 53, 2)?;
                    have_wweek = true;
                }
                'V' => {
                    get_number(s, rp, 0, 53, 2)?;
                }
                'w' => {
                    st.tm.wday = get_number(s, rp, 0, 6, 1)?;
                    have_wday = true;
                }
                'y' => {
                    let ival = get_number(s, rp, 0, 99, 2)?;
                    st.tm.year = if ival >= 69 { ival } else { ival + 100 };
                    want_century = true;
                    want_xday = true;
                }
                'Y' => {
                    st.tm.year = get_number(s, rp, 0, 9999, 4)? - 1900;
                    want_century = false;
                    want_xday = true;
                }
                'z' => {
                    let mut n = 0;
                    let mut val: i32 = 0;
                    while at(*rp) == ' ' {
                        *rp += 1;
                    }
                    if at(*rp) != '+' && at(*rp) != '-' {
                        return Err(NoMatch);
                    }
                    let neg = at(*rp) == '-';
                    *rp += 1;
                    while n < 4 && at(*rp).is_ascii_digit() {
                        val = val * 10 + (at(*rp) as i32 - '0' as i32);
                        *rp += 1;
                        n += 1;
                    }
                    if n != 4 {
                        return Err(NoMatch);
                    }
                    // We have to convert the minutes into decimal.
                    if val % 100 >= 60 {
                        return Err(NoMatch);
                    }
                    val = (val / 100) * 100 + ((val % 100) * 50) / 30;
                    if val > 1400 {
                        warnings.push("values for %z outside +/-1400 are an error".into());
                        return Err(NoMatch);
                    }
                    let mut off = (val * 3600) / 100;
                    if neg {
                        off = -off;
                    }
                    st.offset = off;
                }
                'Z' => return Err(Failure::Error("use of %Z for input is not supported".into())),
                'E' => {
                    // We have no information about the era format. Just
                    // use the normal format.
                    let nf = fmt.get(f).copied().unwrap_or('\0');
                    if !matches!(nf, 'c' | 'C' | 'y' | 'Y' | 'x' | 'X') {
                        return Err(NoMatch);
                    }
                    c = nf;
                    f += 1;
                    continue;
                }
                'O' => {
                    let oc = fmt.get(f).copied().unwrap_or('\0');
                    f += 1;
                    match oc {
                        'd' | 'e' => {
                            st.tm.mday = get_number(s, rp, 1, 31, 2)?;
                            have_mday = true;
                            want_xday = true;
                        }
                        'H' => {
                            st.tm.hour = get_number(s, rp, 0, 23, 2)?;
                            have_i = false;
                        }
                        'I' => {
                            st.tm.hour = get_number(s, rp, 1, 12, 2)? % 12;
                            have_i = true;
                        }
                        'm' => {
                            st.tm.mon = get_number(s, rp, 1, 12, 2)? - 1;
                            have_mon = true;
                            want_xday = true;
                        }
                        'M' => st.tm.min = get_number(s, rp, 0, 59, 2)?,
                        'S' => {
                            // Match seconds using alternate numeric symbols.
                            let (sval, end) = wcstod(s, *rp);
                            if (0.0..=61.0).contains(&sval) {
                                st.tm.sec = sval as i32;
                                st.psecs = sval;
                            }
                            *rp = end;
                        }
                        'U' => {
                            week_no = get_number(s, rp, 0, 53, 2)?;
                            have_uweek = true;
                        }
                        'W' => {
                            week_no = get_number(s, rp, 0, 53, 2)?;
                            have_wweek = true;
                        }
                        'V' => {
                            get_number(s, rp, 0, 53, 2)?;
                        }
                        'w' => {
                            st.tm.wday = get_number(s, rp, 0, 6, 1)?;
                            have_wday = true;
                        }
                        'y' => {
                            let ival = get_number(s, rp, 0, 99, 2)?;
                            st.tm.year = if ival >= 69 { ival } else { ival + 100 };
                            want_xday = true;
                        }
                        _ => return Err(NoMatch),
                    }
                }
                _ => return Err(NoMatch),
            }
            break;
        }
    }

    let tm = &mut st.tm;
    if have_i && is_pm {
        tm.hour += 12;
    }
    if century != -1 {
        if want_century {
            tm.year = tm.year % 100 + (century - 19) * 100;
        } else {
            // Only the century, but not the year. Strange, but so be it.
            tm.year = (century - 19) * 100;
        }
    }
    if want_xday && !have_wday {
        if !(have_mon && have_mday) && have_yday {
            // have_yday, so this must have come from %j
            let mut t_mon: usize = 0;
            let yr = (1900i64 + tm.year as i64) as i32;
            let leap = isleap(yr) as usize;
            if tm.yday > if isleap(yr) { 365 } else { 364 } {
                warnings.push(format!("day-of-year {} in year {} is invalid\n", tm.yday + 1, yr));
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
                tm.mday = tm.yday - MON_YDAY[leap][t_mon - 1] + 1;
            }
        }
        day_of_the_week(tm);
    }
    if want_xday && !have_yday {
        day_of_the_year(tm);
    }
    if (have_uweek || have_wweek) && have_wday {
        let save_wday = tm.wday;
        let save_mday = tm.mday;
        let save_mon = tm.mon;
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
            // Get yday from week and day-of-the-week.
            tm.yday = (7 - (tm.wday - w_offset)) % 7 + (week_no - 1) * 7 + save_wday - w_offset;
            if tm.yday < 0 {
                tm.yday += 7;
            }
        }
        if !have_mday || !have_mon {
            let mut t_mon: usize = 0;
            let yr = (1900i64 + tm.year as i64) as i32;
            let leap = isleap(yr) as usize;
            if tm.yday > if isleap(yr) { 365 } else { 364 } {
                warnings.push(format!("(0-based) yday {} in year {} is invalid\n", tm.yday, yr));
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
                tm.mday = tm.yday - MON_YDAY[leap][t_mon.max(1) - 1] + 1;
            }
        }
        tm.wday = save_wday;
    }
    Ok(())
}
