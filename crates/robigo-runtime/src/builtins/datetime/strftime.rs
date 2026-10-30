//! `strftime` as GNU R calls it from do_formatPOSIXlt: a port of glibc
//! 2.41's time/strftime_l.c (`__strftime_internal`) for the C locale,
//! which is what the oracle uses (R's own src/extra/tzone/strftime.c is
//! only used with --with-internal-tzcode).

use super::tz::{self, Tm};

const ABDAY: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DAY: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const ABMON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MON: [&str; 12] = [
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

/// LC_TIME of the C locale.
const D_T_FMT: &str = "%a %b %e %H:%M:%S %Y";
const D_FMT: &str = "%m/%d/%y";
const T_FMT: &str = "%H:%M:%S";
const T_FMT_AMPM: &str = "%I:%M:%S %p";

/// glibc's `maxsize` as passed by R (`char buff[2049]`).
pub const MAXSIZE: usize = 2049;

/// `strftime(buff, 2049, format, tp)`: `None` when the result does not fit
/// (strftime returns 0).
pub fn strftime(format: &str, tp: &Tm) -> Option<String> {
    let mut tzset_called = false;
    let fmt: Vec<char> = format.chars().collect();
    let s = internal(&fmt, tp, &mut tzset_called)?;
    if s.len() >= MAXSIZE { None } else { Some(s) }
}

/// The ISO week day distance helper of strftime_l.c.
fn iso_week_days(yday: i32, wday: i32) -> i32 {
    const ISO_WEEK_START_WDAY: i32 = 1;
    const ISO_WEEK1_WDAY: i32 = 4;
    const YDAY_MINIMUM: i32 = -366;
    let big_enough_multiple_of_7 = (-YDAY_MINIMUM / 7 + 2) * 7;
    yday - (yday - wday + ISO_WEEK1_WDAY + big_enough_multiple_of_7) % 7 + ISO_WEEK1_WDAY - ISO_WEEK_START_WDAY
}

fn isleap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

struct Out {
    s: String,
}

impl Out {
    /// The `add` macro: pad to `width` (zeros if `pad == '0'`), then `text`.
    fn add(&mut self, text: &str, width: i64, pad: char) -> Option<()> {
        let n = text.len() as i64;
        let delta = width - n;
        if delta > 0 {
            if (self.s.len() as i64 + delta + n) >= MAXSIZE as i64 {
                return None;
            }
            let c = if pad == '0' { '0' } else { ' ' };
            for _ in 0..delta {
                self.s.push(c);
            }
        }
        self.s.push_str(text);
        if self.s.len() >= MAXSIZE {
            return None;
        }
        Some(())
    }

    /// The `cpy` macro, with case conversion.
    fn cpy(&mut self, text: &str, width: i64, pad: char, to_upp: bool, to_low: bool) -> Option<()> {
        if to_low {
            self.add(&text.to_ascii_lowercase(), width, pad)
        } else if to_upp {
            self.add(&text.to_ascii_uppercase(), width, pad)
        } else {
            self.add(text, width, pad)
        }
    }
}

enum Spec {
    /// DO_NUMBER / DO_NUMBER_SPACEPAD (digits, value, spacepad)
    Number(i64, i64, bool),
    Text(String),
    Sub(&'static str),
    Bad,
    Nothing,
}

fn internal(format: &[char], tp: &Tm, tzset_called: &mut bool) -> Option<String> {
    let mut out = Out { s: String::new() };
    let mut hour12 = tp.hour;
    if hour12 > 12 {
        hour12 -= 12;
    } else if hour12 == 0 {
        hour12 = 12;
    }
    let mut zone: Option<crate::value::RString> = tp.zone.clone();
    let mut f = 0usize;
    let at = |i: usize| format.get(i).copied().unwrap_or('\0');
    while f < format.len() {
        if format[f] != '%' {
            out.add(&format[f].to_string(), -1, '\0')?;
            f += 1;
            continue;
        }
        let mut pad = '\0';
        let mut to_upp = false;
        let mut to_low = false;
        let mut change_case = false;
        // Check for flags that can modify a format.
        loop {
            f += 1;
            match at(f) {
                c @ ('_' | '-' | '0') => pad = c,
                '^' => to_upp = true,
                '#' => change_case = true,
                _ => break,
            }
        }
        // As a GNU extension we allow to specify the field width.
        let mut width: i64 = -1;
        if at(f).is_ascii_digit() {
            width = 0;
            while at(f).is_ascii_digit() {
                width = (width * 10 + (at(f) as i64 - '0' as i64)).min(i32::MAX as i64);
                f += 1;
            }
        }
        if width >= MAXSIZE as i64 {
            // Any output would overflow the buffer.
            return None;
        }
        let modifier = match at(f) {
            c @ ('E' | 'O') => {
                f += 1;
                c
            }
            _ => '\0',
        };
        let fc = at(f);
        let dn = |d: i64, v: i64| Spec::Number(if d > width { d } else { width }, v, false);
        let spec: Spec = match fc {
            '%' => {
                if modifier != '\0' {
                    Spec::Bad
                } else {
                    Spec::Text("%".into())
                }
            }
            'a' | 'A' => {
                if modifier != '\0' {
                    Spec::Bad
                } else {
                    if change_case {
                        to_upp = true;
                        to_low = false;
                    }
                    let w = tp.wday;
                    let s = if !(0..=6).contains(&w) {
                        "?"
                    } else if fc == 'a' {
                        ABDAY[w as usize]
                    } else {
                        DAY[w as usize]
                    };
                    Spec::Text(s.into())
                }
            }
            'b' | 'h' | 'B' => {
                // glibc applies '#' before rejecting %Eb, but after %EB.
                if change_case && (fc != 'B' || modifier != 'E') {
                    to_upp = true;
                    to_low = false;
                }
                if modifier == 'E' {
                    Spec::Bad
                } else {
                    // The alternative (%O) month names equal the plain
                    // ones in the C locale.
                    let m = tp.mon;
                    let s = if !(0..=11).contains(&m) {
                        "?"
                    } else if fc == 'B' {
                        MON[m as usize]
                    } else {
                        ABMON[m as usize]
                    };
                    Spec::Text(s.into())
                }
            }
            'c' => {
                if modifier == 'O' {
                    Spec::Bad
                } else {
                    Spec::Sub(D_T_FMT)
                }
            }
            'C' => {
                let year = tp.year as i64 + 1900;
                dn(1, year / 100 - ((year % 100) < 0) as i64)
            }
            'x' => {
                if modifier == 'O' {
                    Spec::Bad
                } else {
                    Spec::Sub(D_FMT)
                }
            }
            'D' => {
                if modifier != '\0' {
                    Spec::Bad
                } else {
                    Spec::Sub("%m/%d/%y")
                }
            }
            'd' => {
                if modifier == 'E' {
                    Spec::Bad
                } else {
                    dn(2, tp.mday as i64)
                }
            }
            'e' => {
                if modifier == 'E' {
                    Spec::Bad
                } else {
                    Spec::Number(if 2 > width { 2 } else { width }, tp.mday as i64, true)
                }
            }
            'F' => {
                if modifier != '\0' {
                    Spec::Bad
                } else {
                    Spec::Sub("%Y-%m-%d")
                }
            }
            'H' | 'I' | 'k' | 'l' | 'j' | 'M' | 'm' | 'S' | 'U' | 'W' | 'w' => {
                if modifier == 'E' {
                    Spec::Bad
                } else {
                    match fc {
                        'H' => dn(2, tp.hour as i64),
                        'I' => dn(2, hour12 as i64),
                        'k' => Spec::Number(if 2 > width { 2 } else { width }, tp.hour as i64, true),
                        'l' => Spec::Number(if 2 > width { 2 } else { width }, hour12 as i64, true),
                        'j' => dn(3, 1 + tp.yday as i64),
                        'M' => dn(2, tp.min as i64),
                        'm' => dn(2, tp.mon as i64 + 1),
                        'S' => dn(2, tp.sec as i64),
                        'U' => dn(2, ((tp.yday - tp.wday + 7) / 7) as i64),
                        'W' => dn(2, ((tp.yday - (tp.wday - 1 + 7) % 7 + 7) / 7) as i64),
                        _ => dn(1, tp.wday as i64),
                    }
                }
            }
            'n' => Spec::Text("\n".into()),
            'P' | 'p' => {
                if fc == 'P' {
                    to_low = true;
                }
                if change_case {
                    to_upp = false;
                    to_low = true;
                }
                Spec::Text(if tp.hour > 11 { "PM" } else { "AM" }.into())
            }
            'R' => Spec::Sub("%H:%M"),
            'r' => Spec::Sub(T_FMT_AMPM),
            's' => {
                let mut ltm = tp.clone();
                let t = tz::mktime(&mut ltm).unwrap_or(-1);
                // digits = 1, then do_number_sign_and_padding
                let s = t.unsigned_abs().to_string();
                number_sign_and_padding(&mut out, &s, t < 0, 1, width, pad, to_upp, to_low)?;
                Spec::Nothing
            }
            'X' => {
                if modifier == 'O' {
                    Spec::Bad
                } else {
                    Spec::Sub(T_FMT)
                }
            }
            'T' => Spec::Sub("%H:%M:%S"),
            't' => Spec::Text("\t".into()),
            'u' => dn(1, ((tp.wday - 1 + 7) % 7 + 1) as i64),
            'V' | 'g' | 'G' => {
                if modifier == 'E' {
                    Spec::Bad
                } else {
                    let mut year = tp.year + 1900;
                    let mut days = iso_week_days(tp.yday, tp.wday);
                    if days < 0 {
                        // This ISO week belongs to the previous year.
                        year -= 1;
                        days = iso_week_days(tp.yday + (365 + isleap(year) as i32), tp.wday);
                    } else {
                        let d = iso_week_days(tp.yday - (365 + isleap(year) as i32), tp.wday);
                        if 0 <= d {
                            // This ISO week belongs to the next year.
                            year += 1;
                            days = d;
                        }
                    }
                    match fc {
                        'g' => dn(2, ((year % 100 + 100) % 100) as i64),
                        'G' => dn(1, year as i64),
                        _ => dn(2, (days / 7 + 1) as i64),
                    }
                }
            }
            'Y' => {
                if modifier == 'O' {
                    Spec::Bad
                } else {
                    dn(1, tp.year as i64 + 1900)
                }
            }
            'y' => dn(2, ((tp.year % 100 + 100) % 100) as i64),
            'Z' => {
                if change_case {
                    to_upp = false;
                    to_low = true;
                }
                if zone.as_deref().is_none_or(|z| z.is_empty()) && tp.isdst >= 0 {
                    if !*tzset_called {
                        tz::tzset();
                        *tzset_called = true;
                    }
                    zone = Some(if tp.isdst <= 1 {
                        tz::tzname()[tp.isdst as usize].clone()
                    } else {
                        crate::value::RString::from("?")
                    });
                }
                Spec::Text(zone.as_deref().unwrap_or("").to_string())
            }
            'z' => {
                if tp.isdst < 0 {
                    Spec::Nothing
                } else {
                    let mut diff = tp.gmtoff as i32;
                    if diff < 0 {
                        out.add("-", width, pad)?;
                        diff = -diff;
                    } else {
                        out.add("+", width, pad)?;
                    }
                    diff /= 60;
                    dn(4, ((diff / 60) * 100 + diff % 60) as i64)
                }
            }
            _ => Spec::Bad,
        };
        match spec {
            Spec::Nothing => {}
            Spec::Text(s) => out.cpy(&s, width, pad, to_upp, to_low)?,
            Spec::Sub(sub) => {
                let subfmt: Vec<char> = sub.chars().collect();
                let mut s = internal(&subfmt, tp, tzset_called)?;
                let start = out.s.len();
                out.add(&s, width, pad)?;
                if to_upp {
                    s = out.s[start..].to_ascii_uppercase();
                    out.s.truncate(start);
                    out.s.push_str(&s);
                }
            }
            Spec::Number(digits, value, spacepad) => {
                let mut pad = pad;
                if spacepad && pad != '0' && pad != '-' {
                    pad = '_';
                }
                let s = value.unsigned_abs().to_string();
                number_sign_and_padding(&mut out, &s, value < 0, digits, width, pad, to_upp, to_low)?;
            }
            Spec::Bad => {
                // Unknown format: output the format, including the '%'.
                // (At the end of the format, glibc backs up one char.)
                let end = if fc == '\0' { f - 1 } else { f };
                let mut start = end;
                while format[start] != '%' {
                    start -= 1;
                }
                let s: String = format[start..=end].iter().collect();
                out.cpy(&s, width, pad, to_upp, to_low)?;
                if fc == '\0' {
                    break;
                }
            }
        }
        f += 1;
    }
    Some(out.s)
}

/// The `do_number_sign_and_padding` tail of strftime_l.c.
#[allow(clippy::too_many_arguments)]
fn number_sign_and_padding(
    out: &mut Out,
    digits_str: &str,
    negative: bool,
    digits: i64,
    width: i64,
    pad: char,
    to_upp: bool,
    to_low: bool,
) -> Option<()> {
    let mut buf = String::new();
    if negative {
        buf.push('-');
    }
    buf.push_str(digits_str);
    let mut width = width;
    let mut body: &str = &buf;
    if pad != '-' {
        let padding = digits - buf.len() as i64;
        if padding > 0 {
            if padding as usize >= MAXSIZE {
                return None;
            }
            if pad == '_' {
                for _ in 0..padding {
                    out.s.push(' ');
                }
                width = if width > padding { width - padding } else { 0 };
            } else {
                if negative {
                    body = &buf[1..];
                    out.s.push('-');
                }
                for _ in 0..padding {
                    out.s.push('0');
                }
                width = 0;
            }
        }
    }
    let body = body.to_string();
    out.cpy(&body, width, pad, to_upp, to_low)
}
