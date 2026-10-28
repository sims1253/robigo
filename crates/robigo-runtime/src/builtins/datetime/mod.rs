//! Date-time conversion internals (GNU R's src/main/datetime.c and
//! Rstrptime.h): `as.POSIXlt`, `as.POSIXct`, `format.POSIXlt`,
//! `strptime`, `Date2POSIXlt`, `POSIXlt2Date`, `balancePOSIXlt` and
//! `unCfillPOSIXlt`.
//!
//! GNU R on Linux uses datetime.c's "PATH 1" (the C library's time
//! functions); `tz` ports the glibc functions it calls and `strftime`
//! glibc's strftime, so results match the oracle byte for byte.
//! `tzone_name` is Windows-only in R and therefore not provided.

mod strftime;
mod strptime;
pub mod tz;

use std::rc::Rc;

use super::{arg, internal, prim};
use crate::coerce::{as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;
use tz::Tm;

pub static DEFS: &[Builtin] = &[
    internal("as.POSIXlt", do_as_posixlt, 2),
    internal("as.POSIXct", do_as_posixct, 2),
    internal("format.POSIXlt", do_format_posixlt, 4),
    internal("strptime", do_strptime, 3),
    internal("Date2POSIXlt", do_d2posixlt, 2),
    internal("POSIXlt2Date", do_posixlt2d, 1),
    internal("balancePOSIXlt", do_balance_posixlt, 3),
    prim("unCfillPOSIXlt", do_uncfill_posixlt, 1),
];

/// `checkArity` for these builtins.
fn check_arity(args: &[Arg], n: usize, name: &str, call: &Value, internal: bool) -> RResult<()> {
    if args.len() != n {
        let what = if internal { format!(".Internal({})", name) } else { format!("'{}'", name) };
        let msg = format!(
            "{} argument{} passed to {} which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            what,
            n
        );
        // R: util.c:Rf_checkArityCall -- `.Internal`s use error() (the
        // context call), primitives errorcall(call).
        return Err(if internal { Flow::error(msg) } else { Flow::error_call(call.clone(), msg) });
    }
    Ok(())
}

// ----------------------------------------------------------------------
// struct tm helpers (datetime.c)

const MONTH_DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn isleap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_year(y: i64) -> i64 {
    if isleap(y) { 366 } else { 365 }
}

/// `days_in_month(mon, yr)` for base-1900 years.
fn days_in_month(mon: i64, yr: i64) -> i64 {
    if mon == 1 && isleap(1900 + yr) { 29 } else { MONTH_DAYS[mon.clamp(0, 11) as usize] as i64 }
}

/// `validate_tm`: adjust a struct tm to be a valid scalar date-time.
/// Returns 0 if valid, -1 if invalid and uncorrectable, or a positive
/// number approximating the number of corrections done.
// R: datetime.c:validate_tm
fn validate_tm(tm: &mut Tm) -> i32 {
    let mut res = 0;
    let (mut sec, mut min, mut hour, mut mday, mut mon, mut year) =
        (tm.sec as i64, tm.min as i64, tm.hour as i64, tm.mday as i64, tm.mon as i64, tm.year as i64);
    if !(0..=60).contains(&sec) {
        res += 1;
        let tmp = sec / 60;
        sec -= 60 * tmp;
        min += tmp;
        if sec < 0 {
            sec += 60;
            min -= 1;
        }
    }
    if !(0..=59).contains(&min) {
        res += 1;
        let tmp = min / 60;
        min -= 60 * tmp;
        hour += tmp;
        if min < 0 {
            min += 60;
            hour -= 1;
        }
    }
    if hour == 24 && min == 0 && sec == 0 {
        // 24:00:00
        hour = 0;
        mday += 1;
        if (0..=11).contains(&mon) && mday > days_in_month(mon, year) {
            mon += 1;
            mday = 1;
            if mon == 12 {
                year += 1;
                mon = 0;
            }
        }
    } else if !(0..=23).contains(&hour) {
        res += 1;
        let tmp = hour / 24;
        hour -= 24 * tmp;
        mday += tmp;
        if hour < 0 {
            hour += 24;
            mday -= 1;
        }
    }
    // defer fixing mday until we know the year
    if !(0..=11).contains(&mon) {
        res += 1;
        let tmp = mon / 12;
        mon -= 12 * tmp;
        year += tmp;
        if mon < 0 {
            mon += 12;
            year -= 1;
        }
    }
    let store = |tm: &mut Tm, sec: i64, min: i64, hour: i64, mday: i64, mon: i64, year: i64| {
        tm.sec = sec as i32;
        tm.min = min as i32;
        tm.hour = hour as i32;
        tm.mday = mday as i32;
        tm.mon = mon as i32;
        tm.year = year as i32;
    };
    // A limit on the loops of about 3000x round.
    if !(-1000000..=1000000).contains(&mday) {
        store(tm, sec, min, hour, mday, mon, year);
        return -1;
    }
    if mday.abs() > 366 {
        res += 1;
        // first spin back until January
        while mon > 0 {
            mon -= 1;
            mday += days_in_month(mon, year);
        }
        // then spin on/back by years
        while mday < 1 {
            year -= 1;
            mday += 365 + isleap(1900 + year) as i64;
        }
        loop {
            let tmp = 365 + isleap(1900 + year) as i64;
            if mday <= tmp {
                break;
            }
            mday -= tmp;
            year += 1;
        }
    }
    while mday < 1 {
        res += 1;
        mon -= 1;
        if mon < 0 {
            mon += 12;
            year -= 1;
        }
        mday += days_in_month(mon, year);
    }
    loop {
        let tmp = days_in_month(mon, year);
        if mday <= tmp {
            break;
        }
        res += 1;
        mon += 1;
        if mon > 11 {
            mon -= 12;
            year += 1;
        }
        mday -= tmp;
    }
    store(tm, sec, min, hour, mday, mon, year);
    res
}

/// glibc and strftime overflow when the year does not fit an int.
fn likely_strftime_overflow(tm: &Tm) -> bool {
    let year = 1900.0 + tm.year as f64;
    !(year < i32::MAX as f64 && year > i32::MIN as f64)
}

/// `mkdate00`: days since the epoch; fixes up `yday` and `wday`.
// R: datetime.c:mkdate00
fn mkdate00(tm: &mut Tm) -> f64 {
    if tm.mday == NA_INTEGER || tm.year == NA_INTEGER || tm.mon == NA_INTEGER {
        tm.yday = NA_INTEGER;
        tm.wday = NA_INTEGER;
        return NA_REAL;
    }
    let mut day = tm.mday as i64 - 1;
    let mut year0 = 1900 + tm.year as i64;
    let mut excess = 0.0f64;
    if year0 >= 400 {
        excess = (year0 / 400) as f64 - 1.0;
        year0 -= (excess * 400.0) as i64;
    } else if year0 < 0 {
        excess = -1.0 - (-year0 / 400) as f64;
        year0 -= (excess * 400.0) as i64;
    }
    for i in 0..tm.mon.clamp(0, 12) {
        day += MONTH_DAYS[i as usize] as i64;
    }
    if tm.mon > 1 && isleap(year0) {
        day += 1;
    }
    tm.yday = day as i32;
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
    tm.wday = ((day % 7) + 4) as i32 % 7;
    if tm.wday < 0 {
        tm.wday += 7;
    }
    day as f64 + excess * 146097.0
}

/// `timegm00`: no checking, returns double.
fn timegm00(tm: &mut Tm) -> f64 {
    let day = mkdate00(tm);
    tm.sec as f64 + (tm.min as f64 * 60.0) + (tm.hour as f64 * 3600.0) + day * 86400.0
}

/// `mktime0` (PATH 1): `(value, errno set)`.
fn mktime0(tm: &mut Tm, local: bool) -> (f64, bool) {
    if validate_tm(tm) < 0 {
        return (-1.0, true);
    }
    if !local {
        return (timegm00(tm), false);
    }
    match tz::mktime(tm) {
        Ok(t) => (t as f64, false),
        Err(()) => (-1.0, true),
    }
}

/// `(time_t) d`, rounded towards minus infinity (LOCALTIME_MK_).
fn time_t_of(d: f64) -> i64 {
    let mut t = d as i64;
    if d < 0.0 && d != t as f64 {
        t -= 1;
    }
    t
}

/// `localtime0` (PATH 1, with POSIX leap seconds).
fn localtime0(d: f64, local: bool) -> Option<Tm> {
    let t = time_t_of(d);
    if local { tz::localtime_r(t) } else { tz::gmtime_r(t) }
}

/// Resets `TZ` when dropped (`prepare_reset_tz`/`set_tz`/`reset_tz`),
/// also on error exits (R's context `cend`).
struct TzGuard {
    saved: Option<tz::EnvTzState>,
}

impl TzGuard {
    fn new() -> TzGuard {
        TzGuard { saved: None }
    }

    /// `set_tz`: `setenv("TZ", tz)` and `tzset()`.
    fn set_tz(&mut self, tz: &str) {
        if self.saved.is_none() {
            self.saved = Some(tz::env_tz_state());
        }
        tz::set_env_tz(Some(tz));
        tz::tzset();
    }
}

impl Drop for TzGuard {
    /// `reset_tz`: restore the previous `TZ` and `tzset()`.
    fn drop(&mut self) {
        if let Some(st) = self.saved.take() {
            tz::restore_env_tz_state(st);
            tz::tzset();
        }
    }
}

/// The `tz` argument: a length-one character vector; `""` means `TZ`.
fn tz_arg(stz: &Value) -> RResult<String> {
    match stz {
        Value::Str(v) if v.data.len() == 1 => {
            let tz = v.data[0].as_deref().unwrap_or("NA").to_string();
            if tz.is_empty() {
                if let Some(p) = tz::getenv_tz() {
                    return Ok(p);
                }
            }
            Ok(tz)
        }
        _ => Err(Flow::error("invalid 'tz' value")),
    }
}

fn is_utc(tz: &str) -> bool {
    tz == "GMT" || tz == "UTC"
}

/// Set up the time zone as do_asPOSIXlt and friends do.
fn enter_tz(tz: &str, is_utc: bool) -> TzGuard {
    let mut g = TzGuard::new();
    if !is_utc && !tz.is_empty() {
        g.set_tz(tz);
    } else {
        tz::tzset();
    }
    g
}

// ----------------------------------------------------------------------
// POSIXlt objects

const LTNAMES: [&str; 11] = ["sec", "min", "hour", "mday", "mon", "year", "wday", "yday", "isdst", "zone", "gmtoff"];

fn sym_balanced() -> Sym {
    Sym::new("balanced")
}

fn sym_tzone() -> Sym {
    Sym::new("tzone")
}

/// A new POSIXlt under construction (the `ans` of datetime.c).
struct LtBuilder {
    sec: Vec<f64>,
    ints: [Vec<i32>; 8],
    zone: Vec<RStr>,
    gmtoff: Vec<i32>,
}

impl LtBuilder {
    fn new(n: usize) -> LtBuilder {
        LtBuilder {
            sec: Vec::with_capacity(n),
            ints: Default::default(),
            zone: Vec::with_capacity(n),
            gmtoff: Vec::with_capacity(n),
        }
    }

    /// `makelt`.
    fn makelt(&mut self, tm: &Tm, valid: bool, frac_secs: f64) {
        if valid {
            self.sec.push(tm.sec as f64 + frac_secs);
            for (j, v) in
                [tm.min, tm.hour, tm.mday, tm.mon, tm.year, tm.wday, tm.yday, tm.isdst].into_iter().enumerate()
            {
                self.ints[j].push(v);
            }
        } else {
            self.sec.push(frac_secs);
            for j in 0..7 {
                self.ints[j].push(NA_INTEGER);
            }
            self.ints[7].push(-1);
        }
    }

    /// The list with names (`ncomp` components).
    fn into_list(self, ncomp: usize, year_names: Option<Value>) -> Value {
        let mut comps: Vec<Value> = Vec::with_capacity(ncomp);
        comps.push(Value::reals(self.sec));
        for (j, v) in self.ints.into_iter().enumerate() {
            let mut c = Value::ints(v);
            if j == 4 {
                if let Some(nm) = &year_names {
                    c.set_attr_raw(s::NAMES, nm.clone());
                }
            }
            comps.push(c);
        }
        if ncomp >= 10 {
            comps.push(Value::strs(self.zone));
        }
        if ncomp >= 11 {
            comps.push(Value::ints(self.gmtoff));
        }
        let mut ans = Value::list(comps);
        ans.set_attr_raw(s::NAMES, Value::strs_from(&LTNAMES[..ncomp]));
        ans
    }
}

fn posixlt_class() -> Value {
    Value::strs_from(["POSIXlt", "POSIXt"])
}

/// END_MAKElt: names, class, tzone, names(x) on year, balanced.
fn end_makelt(b: LtBuilder, tzone: Value, year_names: Option<Value>) -> Value {
    let mut ans = b.into_list(11, year_names);
    ans.set_attr_raw(s::CLASS, posixlt_class());
    ans.set_attr_raw(sym_tzone(), tzone);
    ans.set_attr_raw(sym_balanced(), Value::lgl(true));
    ans
}

/// BEGIN_MAKElt: the "tzone" attribute.
fn begin_makelt(tz: &str, is_utc: bool) -> Value {
    if is_utc {
        Value::str(tz)
    } else {
        let n = tz::tzname();
        Value::strs_from([tz, &n[0], &n[1]])
    }
}

/// Names padded/recycled to length `n` (xlengthgets then recycling).
fn recycle_names(nm: &Value, n: usize) -> Value {
    let v = match nm {
        Value::Str(v) => &v.data,
        _ => return nm.clone(),
    };
    if v.is_empty() {
        return Value::strs(vec![None; n]);
    }
    Value::strs((0..n).map(|i| v[i % v.len()].clone()).collect())
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Real(_))
}

/// `valid_POSIXlt(x, nm)`: checks and coerces the components; returns the
/// components (coerced).
// R: datetime.c:valid_POSIXlt
fn valid_posixlt(interp: &mut Interp, x: &Value, nm: usize) -> RResult<Vec<Value>> {
    let n_comp = x.len();
    let n_check = n_comp.min(nm);
    let Value::List(l) = x else {
        return Err(Flow::error("a valid \"POSIXlt\" object is a list of at least 9 elements"));
    };
    if n_comp < 9 {
        return Err(Flow::error("a valid \"POSIXlt\" object is a list of at least 9 elements"));
    }
    let nms = match x.get_attr(s::NAMES) {
        Some(Value::Str(v)) => v.data.clone(),
        _ => vec![],
    };
    if nms.len() < 9 {
        return Err(Flow::error("a valid \"POSIXlt\" object has names"));
    }
    for i in 0..n_check {
        let nm = nms[i].as_deref().unwrap_or("NA");
        if nm != LTNAMES[i] {
            return Err(Flow::error(format!(
                "a valid \"POSIXlt\" object has element {} with name '{}' which should be '{}'",
                i + 1,
                nm,
                LTNAMES[i]
            )));
        }
    }
    let mut comps = l.data.clone();
    for i in 0..9.min(nm) {
        if !is_num(&comps[i]) {
            return Err(Flow::error(format!("a valid \"POSIXlt\" object has a numeric element {}", LTNAMES[i])));
        }
    }
    comps[0] = interp.coerce(&comps[0], VType::Real)?;
    for i in 1..n_check {
        if i == 9 {
            continue; // skip zone
        }
        comps[i] = interp.coerce(&comps[i], VType::Int)?;
    }
    if n_check >= 10 && !matches!(comps[9], Value::Str(_)) {
        return Err(Flow::error(format!("a valid \"POSIXlt\" object has a character element {}", LTNAMES[9])));
    }
    if n_check >= 11 && !is_num(&comps[10]) {
        return Err(Flow::error(format!("a valid \"POSIXlt\" object has a numeric element {}", LTNAMES[10])));
    }
    // check the tzone attribute.
    if let Some(tz) = x.get_attr(sym_tzone()) {
        let Value::Str(t) = &tz else {
            return Err(Flow::error("invalid 'attr(x, \"tzone\")'"));
        };
        if t.data.len() != 1 && t.data.len() != 3 {
            return Err(Flow::error("attr(x, \"tzone\") should have length 1 or 3"));
        }
    }
    Ok(comps)
}

fn reals_of(v: &Value) -> &[f64] {
    match v {
        Value::Real(r) => &r.data,
        _ => &[],
    }
}

fn ints_of(v: &Value) -> &[i32] {
    match v {
        Value::Int(r) => &r.data,
        _ => &[],
    }
}

fn check_nlen(nlen: &[usize], idx: &[usize]) -> RResult<()> {
    for &i in idx {
        if nlen[i] == 0 {
            return Err(Flow::error(format!("zero-length component [[{}]] in non-empty \"POSIXlt\" structure", i + 1)));
        }
    }
    Ok(())
}

/// Reads element `i` (recycled) of the first nine components into a tm.
fn tm_from_comps(comps: &[Value], nlen: &[usize], i: usize) -> (f64, Tm) {
    let secs = reals_of(&comps[0])[i % nlen[0]];
    let g = |k: usize| ints_of(&comps[k]).get(i % nlen[k].max(1)).copied().unwrap_or(NA_INTEGER);
    let fsecs = secs.floor();
    let tm = Tm {
        sec: if secs.is_finite() { fsecs as i32 } else { NA_INTEGER },
        min: g(1),
        hour: g(2),
        mday: g(3),
        mon: g(4),
        year: g(5),
        wday: g(6),
        yday: g(7),
        isdst: g(8),
        gmtoff: 0,
        zone: None,
    };
    (secs, tm)
}

fn any_na(tm: &Tm) -> bool {
    tm.min == NA_INTEGER
        || tm.hour == NA_INTEGER
        || tm.mday == NA_INTEGER
        || tm.mon == NA_INTEGER
        || tm.year == NA_INTEGER
}

// ----------------------------------------------------------------------
// .Internal(as.POSIXlt(x, tz))

// R: datetime.c:do_asPOSIXlt
fn do_as_posixlt(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "as.POSIXlt", call, true)?;
    let x = interp.coerce(&arg(&args, 0), VType::Real)?;
    let tz = tz_arg(&arg(&args, 1))?;
    let utc = is_utc(&tz);
    let guard = enter_tz(&tz, utc);
    let tzone = begin_makelt(&tz, utc);
    let xs = reals_of(&x).to_vec();
    let mut b = LtBuilder::new(xs.len());
    for &d in &xs {
        let (ptm, valid) = if d.is_finite() {
            match localtime0(d, !utc) {
                Some(t) => (t, true),
                None => (Tm::default(), false),
            }
        } else {
            (Tm::default(), false)
        };
        b.makelt(&ptm, valid, if valid { d - d.floor() } else { d });
        if utc {
            b.zone.push(rstr(&tz));
            b.gmtoff.push(0);
        } else {
            let p: Rc<str> =
                if valid && ptm.isdst >= 0 { tz::tzname()[ptm.isdst.min(1) as usize].clone() } else { Rc::from("") };
            b.zone.push(Some(p));
            b.gmtoff.push(if valid { ptm.gmtoff as i32 } else { NA_INTEGER });
        }
    }
    drop(guard);
    Ok(end_makelt(b, tzone, x.get_attr(s::NAMES)))
}

// ----------------------------------------------------------------------
// .Internal(as.POSIXct(x, tz))

// R: datetime.c:do_asPOSIXct
fn do_as_posixct(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "as.POSIXct", call, true)?;
    let x = arg(&args, 0);
    let comps = valid_posixlt(interp, &x, 9)?;
    let tz = tz_arg(&arg(&args, 1))?;
    let utc = is_utc(&tz);
    let guard = enter_tz(&tz, utc);
    let mut nlen = [0usize; 9];
    let mut n = 0;
    for i in (0..6).chain(std::iter::once(8)) {
        nlen[i] = comps[i].len();
        n = n.max(nlen[i]);
    }
    if n > 0 {
        check_nlen(&nlen, &[0, 1, 2, 3, 4, 5, 8])?;
    }
    let mut ans = Vec::with_capacity(n);
    for i in 0..n {
        let (secs, mut tm) = tm_from_comps(&comps, &nlen, i);
        let fsecs = secs.floor();
        tm.wday = 0;
        tm.yday = 0;
        tm.isdst = if utc { 0 } else { ints_of(&comps[8])[i % nlen[8]] };
        if !secs.is_finite() {
            ans.push(secs);
        } else if any_na(&tm) {
            ans.push(NA_REAL);
        } else {
            let (tmp, errno) = mktime0(&mut tm, !utc);
            ans.push(if tmp == -1.0 && errno { NA_REAL } else { tmp + (secs - fsecs) });
        }
    }
    let mut out = Value::reals(ans);
    if let Some(nm) = comps[5].get_attr(s::NAMES) {
        out.set_attr_raw(s::NAMES, nm);
    }
    out.set_attr_raw(s::CLASS, Value::strs_from(["POSIXct", "POSIXt"]));
    drop(guard);
    Ok(out)
}

// ----------------------------------------------------------------------
// .Internal(format.POSIXlt(x, format, usetz, digits))

// R: datetime.c:do_formatPOSIXlt
fn do_format_posixlt(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 4, "format.POSIXlt", call, true)?;
    let x = arg(&args, 0);
    let comps = valid_posixlt(interp, &x, 11)?;
    let sformat = match arg(&args, 1) {
        Value::Str(v) if !v.data.is_empty() => v.data.clone(),
        _ => return Err(Flow::error("invalid 'format' argument")),
    };
    let m = sformat.len();
    let usetz = as_logical(&arg(&args, 2));
    if usetz == NA_LOGICAL {
        return Err(Flow::error("invalid 'usetz' argument"));
    }
    let digits = as_integer(&arg(&args, 3));
    let tzattr = x.get_attr(sym_tzone());
    let tzv: Option<Vec<RStr>> = match &tzattr {
        None => None,
        Some(Value::Str(v)) => Some(v.data.clone()),
        Some(_) => return Err(Flow::error("invalid 'attr(x, \"tzone\")'")),
    };
    let fmt_str = |k: usize| sformat[k].as_deref().unwrap_or("NA").to_string();
    let mut guard = TzGuard::new();
    if let Some(tz) = &tzv {
        let tz1 = tz.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_else(|| "NA".into());
        if !tz1.is_empty() {
            // If the format includes %Z or %z we need to set TZ accordingly.
            let need_tz = (0..m).any(|k| {
                let p = fmt_str(k);
                p.contains("%Z") || p.contains("%z")
            });
            if need_tz {
                guard.set_tz(&tz1);
            }
        }
    }
    let mut tm = Tm::default();
    let nn = comps.len().min(11);
    let mut nlen = [0usize; 11];
    let mut n = 0;
    for i in 0..nn {
        nlen[i] = comps[i].len();
        n = n.max(nlen[i]);
    }
    if n > 0 {
        check_nlen(&nlen, &(0..nn).collect::<Vec<_>>())?;
    }
    let big_n = if n > 0 { m.max(n) } else { 0 };
    let have_zone = comps.len() >= 11;
    let mut ans: Vec<RStr> = Vec::with_capacity(big_n);
    let mut ns0: i32 = -1;
    for i in 0..big_n {
        let secs = reals_of(&comps[0])[i % nlen[0]];
        let fsecs = secs.floor();
        tm.sec = if secs.is_finite() && fsecs >= i32::MIN as f64 && fsecs <= i32::MAX as f64 {
            fsecs as i32
        } else {
            NA_INTEGER
        };
        let g = |k: usize| ints_of(&comps[k])[i % nlen[k]];
        tm.min = g(1);
        tm.hour = g(2);
        tm.mday = g(3);
        tm.mon = g(4);
        tm.year = g(5);
        tm.wday = g(6);
        tm.yday = g(7);
        tm.isdst = g(8);
        let zone_i: Option<String> = if have_zone {
            let z = match &comps[9] {
                Value::Str(v) => v.data[i % nlen[9]].as_deref().unwrap_or("NA").to_string(),
                _ => String::new(),
            };
            // strncpy(tm_zone, ..., 20 - 1)
            let mut zz = z.clone();
            if zz.len() > 19 {
                let mut cut = 19;
                while !zz.is_char_boundary(cut) {
                    cut -= 1;
                }
                zz.truncate(cut);
            }
            tm.zone = Some(Rc::from(zz.as_str()));
            Some(z)
        } else {
            None
        };
        if !secs.is_finite() {
            ans.push(if is_na_real(secs) {
                None
            } else if secs.is_nan() {
                rstr("NaN")
            } else if secs > 0.0 {
                rstr("Inf")
            } else {
                rstr("-Inf")
            });
        } else if any_na(&tm) || validate_tm(&mut tm) < 0 || likely_strftime_overflow(&tm) {
            ans.push(None);
        } else {
            let q = fmt_str(i % m);
            let mut buf2 = q.clone();
            if let Some(pos) = q.find("%OS") {
                let mut nused = 4;
                let mut ns: i32 = q.as_bytes().get(pos + 3).map(|&c| c as i32 - '0' as i32).unwrap_or(-('0' as i32));
                buf2.truncate(pos);
                if !(0..=9).contains(&ns) {
                    // not a digit
                    if ns0 == -1 {
                        ns0 = if digits == NA_INTEGER { 0 } else { digits };
                    }
                    ns = ns0;
                    nused = 3;
                }
                if ns > 6 {
                    ns = 6;
                }
                if ns > 0 {
                    // truncate to avoid nuisances such as PR#14579
                    let t = 10f64.powi(ns);
                    let s = tm.sec as f64 + (secs - fsecs);
                    let s = ((s * t) as i32) as f64 / t;
                    buf2.push_str(&format!("{:0w$.p$}", s, w = (ns + 3) as usize, p = ns as usize));
                } else {
                    buf2.push_str("%S");
                }
                buf2.push_str(&q[(pos + nused).min(q.len())..]);
            }
            if have_zone {
                // Got coerced if necessary above
                let tmp = ints_of(&comps[10])[i % nlen[10]];
                if tmp == NA_INTEGER && buf2.contains("%z") {
                    tm.gmtoff = 0;
                    // At least on glibc this corrects it
                    let _ = tz::mktime(&mut tm);
                } else {
                    tm.gmtoff = tmp as i64;
                }
            }
            let mut buff = match strftime::strftime(&buf2, &tm) {
                Some(s) => s,
                None => return Err(Flow::error("output string exceeded 2048 bytes")),
            };
            if buff.is_empty() && !matches!(buf2.as_str(), "%Z" | "%z" | "%P" | "%p") {
                return Err(Flow::error("output string exceeded 2048 bytes"));
            }
            if usetz != 0 {
                if let Some(z) = &zone_i {
                    if !z.is_empty() {
                        buff.push(' ');
                        buff.push_str(z);
                    }
                } else if let Some(tz) = &tzv {
                    let mut ii = 0;
                    if tz.len() == 3 {
                        ii = if tm.isdst > 0 {
                            2
                        } else if tm.isdst == 0 {
                            1
                        } else {
                            0
                        };
                    }
                    let p = tz[ii].as_deref().unwrap_or("NA");
                    if !p.is_empty() {
                        buff.push(' ');
                        buff.push_str(p);
                    }
                }
            }
            ans.push(Some(Rc::from(buff.as_str())));
        }
    }
    let mut out = Value::strs(ans);
    if let Some(nm) = comps[5].get_attr(s::NAMES) {
        // pad nm to length n, then recycle to length N.
        let nm2: Vec<RStr> = match &nm {
            Value::Str(v) => (0..n).map(|j| v.data.get(j).cloned().unwrap_or(None)).collect(),
            _ => vec![None; n],
        };
        let nm3: Vec<RStr> = (0..big_n).map(|j| nm2[j % n].clone()).collect();
        out.set_attr_raw(s::NAMES, Value::strs(nm3));
    }
    drop(guard);
    Ok(out)
}

// ----------------------------------------------------------------------
// .Internal(strptime(as.character(x), format, tz))

/// `glibc_fix`: set mon and mday which glibc does not always set, using
/// the current date where needed.
fn glibc_fix(tm: &mut Tm, invalid: &mut bool) {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let tm0 = tz::localtime_r(now).unwrap_or_default();
    if tm.year == NA_INTEGER {
        tm.year = tm0.year;
    }
    if tm.mon != NA_INTEGER && tm.mday != NA_INTEGER {
        return;
    }
    // at least one of the month and the day of the month is missing
    if tm.yday != NA_INTEGER {
        // since we have yday, let that take precedence over mon/mday
        let mut yday = tm.yday as i64;
        let mut mon = 0i64;
        loop {
            let tmp = days_in_month(mon, tm.year as i64);
            if yday < tmp || mon >= 11 {
                break;
            }
            yday -= tmp;
            mon += 1;
        }
        tm.mon = mon as i32;
        tm.mday = yday as i32 + 1;
    } else {
        if tm.mday == NA_INTEGER {
            if tm.mon != NA_INTEGER {
                *invalid = true;
                return;
            } else {
                tm.mday = tm0.mday;
            }
        }
        if tm.mon == NA_INTEGER {
            tm.mon = tm0.mon;
        }
    }
}

// R: datetime.c:do_strptime
fn do_strptime(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 3, "strptime", call, true)?;
    let x = arg(&args, 0);
    let Value::Str(xs) = &x else { return Err(Flow::error("invalid 'x' argument")) };
    let sformat = match arg(&args, 1) {
        Value::Str(v) if !v.data.is_empty() => v.data.clone(),
        _ => return Err(Flow::error("invalid 'format' argument")),
    };
    let tz = tz_arg(&arg(&args, 2))?;
    let utc = is_utc(&tz);
    let guard = enter_tz(&tz, utc);
    let tzone = begin_makelt(&tz, utc);
    let n = xs.data.len();
    let m = sformat.len();
    let big_n = if n > 0 { m.max(n) } else { 0 };
    let mut b = LtBuilder::new(big_n);
    for i in 0..big_n {
        let mut st = strptime::Strptime {
            tm: Tm {
                year: NA_INTEGER,
                mon: NA_INTEGER,
                mday: NA_INTEGER,
                yday: NA_INTEGER,
                wday: NA_INTEGER,
                gmtoff: NA_INTEGER as i64,
                isdst: -1,
                ..Default::default()
            },
            psecs: 0.0,
            offset: NA_INTEGER,
        };
        let mut invalid = match &xs.data[i % n] {
            None => true,
            Some(s) => {
                let fmt = sformat[i % m].as_deref().unwrap_or("NA");
                let mut warnings = Vec::new();
                let r = strptime::r_strptime(s, fmt, &mut st, &mut warnings);
                for w in warnings {
                    interp.warning(w.trim_end_matches('\n'))?;
                }
                match r {
                    Ok(()) => false,
                    Err(strptime::Failure::NoMatch) => true,
                    Err(strptime::Failure::Error(e)) => return Err(Flow::error(e)),
                }
            }
        };
        let psecs = st.psecs;
        let offset = st.offset;
        let mut tm = st.tm;
        let mut ptm_alt: Option<Tm> = None;
        let mut lt_failed = false;
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
                // we know the offset, but not the timezone so all we can
                // do is to convert to time_t, adjust and convert back
                let mut tm2 = tm.clone();
                let (t0, _) = mktime0(&mut tm2, false);
                if t0 != -1.0 {
                    let t0 = t0 - offset as f64;
                    match localtime0(t0, !utc) {
                        Some(t) => ptm_alt = Some(t),
                        // localtime returning NULL: R would read through
                        // a NULL pointer; report NA instead.
                        None => lt_failed = true,
                    }
                }
                // (R sets invalid = true when t0 == -1, but that is
                // overridden by the validate_tm() result below.)
            } else {
                // we do want to set wday, yday, isdst, but not to adjust
                // structure at DST boundaries
                let mut tm2 = tm.clone();
                if utc {
                    tm.isdst = 0;
                }
                // mktime _may_ result in error e.g. during the spring-forward gap
                if mktime0(&mut tm2, !utc).0 != -1.0 {
                    tm.wday = tm2.wday;
                    tm.yday = tm2.yday;
                    if !utc && tm.hour == tm2.hour && tm.min == tm2.min {
                        tm.isdst = tm2.isdst;
                    }
                }
            }
            // NB: this overrides the earlier settings of 'invalid', as in R.
            invalid = validate_tm(&mut tm) != 0 || lt_failed;
        }
        let ptm = ptm_alt.as_ref().unwrap_or(&tm);
        b.makelt(ptm, !invalid, if invalid { NA_REAL } else { psecs - psecs.floor() });
        if utc {
            b.zone.push(rstr(&tz));
            b.gmtoff.push(0);
        } else {
            let p: Rc<str> = if !invalid && tm.isdst >= 0 {
                match &tm.zone {
                    Some(z) => z.clone(),
                    None => tz::tzname()[tm.isdst.min(1) as usize].clone(),
                }
            } else {
                Rc::from("")
            };
            b.zone.push(Some(p));
            b.gmtoff.push(if invalid { NA_INTEGER } else { tm.gmtoff as i32 });
        }
    }
    drop(guard);
    let nm = x.get_attr(s::NAMES).map(|nm| if big_n > n { recycle_names(&nm, big_n) } else { nm });
    Ok(end_makelt(b, tzone, nm))
}

// ----------------------------------------------------------------------
// .Internal(Date2POSIXlt(x, tz)), .Internal(POSIXlt2Date(x))

// R: datetime.c:do_D2POSIXlt
fn do_d2posixlt(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 2, "Date2POSIXlt", call, true)?;
    let x = interp.coerce(&arg(&args, 0), VType::Real)?;
    let mut tz = match arg(&args, 1) {
        Value::Str(v) if v.data.len() == 1 => v.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error("invalid 'tz' value")),
    };
    if tz.is_empty() {
        tz = "UTC".into();
    }
    let xs = reals_of(&x).to_vec();
    let mut b = LtBuilder::new(xs.len());
    for &x_i in &xs {
        let mut tm = Tm::default();
        let mut valid = x_i.is_finite();
        if valid {
            // every 400 years is exactly 146097 days long and the pattern
            // is repeated
            let rounds = (x_i.floor() / 146097.0).floor();
            let mut day = (x_i.floor() - 146097.0 * rounds) as i64;
            // weekday: 1970-01-01 was a Thursday
            tm.wday = (((day % 7) + 4) % 7) as i32;
            if tm.wday < 0 {
                tm.wday += 7;
            }
            // year & day within year
            let mut y: i64 = 1970;
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
                tm.year = year0 as i32;
                tm.yday = day as i32;
                // month within year
                let mut mon = 0i64;
                loop {
                    let tmp = days_in_month(mon, tm.year as i64);
                    if day < tmp {
                        break;
                    }
                    day -= tmp;
                    mon += 1;
                }
                tm.mon = mon as i32;
                tm.mday = day as i32 + 1;
                tm.isdst = 0; // no dst in GMT
            }
        }
        b.makelt(&tm, valid, if valid { 0.0 } else { x_i });
        b.zone.push(rstr(&tz));
        b.gmtoff.push(0);
    }
    Ok(end_makelt(b, Value::str(&tz), x.get_attr(s::NAMES)))
}

// R: datetime.c:do_POSIXlt2D
fn do_posixlt2d(interp: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "POSIXlt2Date", call, true)?;
    let x = arg(&args, 0);
    let comps = valid_posixlt(interp, &x, 6)?;
    let mut nlen = [0usize; 9];
    let mut n = 0;
    for i in (0..6).chain(std::iter::once(8)) {
        nlen[i] = comps[i].len();
        n = n.max(nlen[i]);
    }
    if n > 0 {
        check_nlen(&nlen, &[0, 1, 2, 3, 4, 5, 8])?;
    }
    let mut ans = Vec::with_capacity(n);
    for i in 0..n {
        let (secs, mut tm) = tm_from_comps(&comps, &nlen, i);
        tm.isdst = 0;
        if !secs.is_finite() {
            ans.push(secs);
        } else if any_na(&tm) || validate_tm(&mut tm) < 0 {
            ans.push(NA_REAL);
        } else {
            ans.push(mkdate00(&mut tm));
        }
    }
    let mut out = Value::reals(ans);
    if let Some(nm) = comps[5].get_attr(s::NAMES) {
        out.set_attr_raw(s::NAMES, nm);
    }
    out.set_attr_raw(s::CLASS, Value::str("Date"));
    Ok(out)
}

// ----------------------------------------------------------------------
// balancePOSIXlt / unCfillPOSIXlt

/// Recycle `v` (of length `ni`) to length `n` (xlengthgets + fill).
fn recycle_to(v: &Value, n: usize) -> Value {
    let ni = v.len();
    let mut out = match v {
        Value::Real(r) => Value::reals((0..n).map(|i| r.data[i % ni]).collect()),
        Value::Int(r) => Value::ints((0..n).map(|i| r.data[i % ni]).collect()),
        Value::Str(r) => Value::strs((0..n).map(|i| r.data[i % ni].clone()).collect()),
        Value::Lgl(r) => Value::lgls((0..n).map(|i| r.data[i % ni]).collect()),
        other => return other.clone(),
    };
    // xlengthgets keeps names, extending them with ""
    if let Some(Value::Str(nm)) = v.get_attr(s::NAMES) {
        let names: Vec<RStr> = (0..n).map(|i| if i < nm.data.len() { nm.data[i].clone() } else { rstr("") }).collect();
        out.set_attr_raw(s::NAMES, Value::strs(names));
    }
    out
}

// R: datetime.c:balancePOSIXlt
fn balance_posixlt(interp: &mut Interp, x: Value, fill_only: bool, do_class: bool) -> RResult<Value> {
    // R compares the attribute with its shared TRUE/NA objects; we compare
    // values: TRUE means "balanced", NA "filled".
    let bal = match x.get_attr(sym_balanced()) {
        Some(Value::Lgl(v)) if v.data.len() == 1 => Some(v.data[0]),
        _ => None,
    };
    if bal == Some(TRUE) || (fill_only && bal == Some(NA_LOGICAL)) {
        let mut x = x;
        if !do_class {
            x.set_attr_raw(s::CLASS, Value::Null);
        }
        return Ok(x);
    }
    let comps = valid_posixlt(interp, &x, 11)?;
    let n_comp = comps.len().min(11);
    let mut need_fill = false;
    let mut n = 0usize;
    let mut nlen = vec![0usize; n_comp];
    for i in 0..n_comp {
        nlen[i] = comps[i].len();
        if nlen[i] > n {
            n = nlen[i];
        } else if !need_fill && nlen[i] < n {
            need_fill = true;
        }
    }
    // x with the coerced components (valid_POSIXlt changes its argument)
    let with_comps = |comps: Vec<Value>| -> Value {
        let mut y = x.clone();
        if let Value::List(l) = &mut y {
            let l = Rc::make_mut(l);
            for (i, c) in comps.into_iter().enumerate() {
                l.data[i] = c;
            }
        }
        y
    };
    if fill_only && !need_fill {
        // already filled; be fast
        let mut y = with_comps(comps);
        y.set_attr_raw(sym_balanced(), Value::lgl_na());
        if !do_class {
            y.set_attr_raw(s::CLASS, Value::Null);
        }
        return Ok(y);
    }
    if !x.inherits("POSIXlt") {
        return Err(Flow::error("'x' is not a \"POSIXlt\""));
    }
    if n > 0 {
        check_nlen(&nlen, &(0..n_comp).collect::<Vec<_>>())?;
    }
    let nm = comps[5].get_attr(s::NAMES);
    let set_nm = (nlen[5] < n || !fill_only) && nm.is_some();

    if fill_only {
        let mut comps = comps;
        for i in 0..n_comp {
            if nlen[i] != n {
                comps[i] = recycle_to(&comps[i], n);
                if i == 5 && set_nm {
                    // set names(.) = names(x$year), recycling
                    if let Some(Value::Str(nmv)) = &nm {
                        let ni = nmv.data.len();
                        let names: Vec<RStr> = (0..n).map(|j| nmv.data[j % ni].clone()).collect();
                        comps[5].set_attr_raw(s::NAMES, Value::strs(names));
                    }
                }
            }
        }
        let mut y = with_comps(comps);
        if !do_class {
            y.set_attr_raw(s::CLASS, Value::Null);
        }
        y.set_attr_raw(sym_balanced(), Value::lgl_na());
        return Ok(y);
    }

    // fill *and* validate from now on:
    let have_10 = n_comp >= 10;
    let have_11 = n_comp >= 11;
    let mut b = LtBuilder::new(n);
    for i in 0..n {
        let (secs, mut tm) = tm_from_comps(&comps, &nlen, i);
        let fsecs = secs.floor();
        let zone_i: Option<Rc<str>> = if have_10 {
            let z = match &comps[9] {
                Value::Str(v) => v.data[i % nlen[9]].as_deref().unwrap_or("NA").to_string(),
                _ => String::new(),
            };
            let mut zz = z;
            if zz.len() > 19 {
                let mut cut = 19;
                while !zz.is_char_boundary(cut) {
                    cut -= 1;
                }
                zz.truncate(cut);
            }
            Some(Rc::from(zz.as_str()))
        } else {
            None
        };
        tm.gmtoff = if have_11 { ints_of(&comps[10])[i % nlen[10]] as i64 } else { NA_INTEGER as i64 };
        let valid = secs.is_finite() && !any_na(&tm);
        if valid {
            validate_tm(&mut tm);
            // Set correct {yday, wday}
            mkdate00(&mut tm);
        }
        b.makelt(
            &tm,
            valid,
            if valid {
                secs - fsecs
            } else if secs.is_finite() {
                NA_REAL
            } else {
                secs
            },
        );
        if have_10 {
            let p: Rc<str> = if valid && tm.isdst >= 0 {
                zone_i.clone().unwrap_or_else(|| tz::tzname()[tm.isdst.min(1) as usize].clone())
            } else {
                Rc::from("")
            };
            b.zone.push(Some(p));
        }
        if have_11 {
            b.gmtoff.push(if valid { tm.gmtoff as i32 } else { NA_INTEGER });
        }
    }
    let year_names = if set_nm {
        nm.as_ref().map(|v| match v {
            Value::Str(s) if !s.data.is_empty() => {
                Value::strs((0..n).map(|i| s.data[i % s.data.len()].clone()).collect())
            }
            other => other.clone(),
        })
    } else {
        None
    };
    let mut ans = b.into_list(n_comp, year_names);
    if do_class {
        ans.set_attr_raw(s::CLASS, posixlt_class());
    }
    if let Some(tz) = x.get_attr(sym_tzone()) {
        if !matches!(tz, Value::Str(_)) {
            return Err(Flow::error("invalid 'attr(x, \"tzone\")'"));
        }
        ans.set_attr_raw(sym_tzone(), tz);
    }
    ans.set_attr_raw(sym_balanced(), Value::lgl(true));
    Ok(ans)
}

// R: datetime.c:do_balancePOSIXlt (op 0)
fn do_balance_posixlt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 3, "balancePOSIXlt", call, true)?;
    let fill_only = as_logical(&args[1].value);
    if fill_only == NA_LOGICAL {
        return Err(Flow::error("invalid 'fill.only' argument"));
    }
    let do_class = as_logical(&args[2].value);
    if do_class == NA_LOGICAL {
        return Err(Flow::error("invalid 'classed' argument"));
    }
    let x = std::mem::replace(&mut args[0].value, Value::Null);
    balance_posixlt(interp, x, fill_only != 0, do_class != 0)
}

// R: datetime.c:do_balancePOSIXlt (op 1, unCfillPOSIXlt)
fn do_uncfill_posixlt(interp: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "unCfillPOSIXlt", call, false)?;
    let x = std::mem::replace(&mut args[0].value, Value::Null);
    balance_posixlt(interp, x, true, false)
}
