//! Time-zone engine for datetime.c.
//!
//! GNU R on Linux is built without `--with-internal-tzcode`, so datetime.c
//! takes its "PATH 1" and calls the C library's `tzset`, `localtime_r`,
//! `gmtime_r`, `mktime` and reads `tzname`. To behave byte-identically we
//! port those from glibc 2.41 (time/tzset.c, time/tzfile.c,
//! time/offtime.c, time/mktime.c), reading the system tz database
//! (`/usr/share/zoneinfo`, TZif files) the way R's own src/extra/tzone
//! does. That includes glibc's quirks: `tzname` is global state that
//! `localtime_r` updates for every converted time, `tzset()` only re-reads
//! the zone when `TZ` changed, and `mktime` remembers the offset of its
//! previous call as its first guess.
//!
//! R sets `TZ` with `setenv()` while converting; we emulate the variable
//! with an override (see [`set_env_tz`]) instead of mutating the process
//! environment.

use std::cell::RefCell;
use std::rc::Rc;

pub const TZDIR: &str = "/usr/share/zoneinfo";
pub const TZDEFAULT: &str = "/etc/localtime";
pub const TZDEFRULES: &str = "posixrules";

const SECS_PER_HOUR: i64 = 60 * 60;
const SECS_PER_DAY: i64 = SECS_PER_HOUR * 24;
const TM_YEAR_BASE: i64 = 1900;
const EPOCH_YEAR: i64 = 1970;

pub const MON_YDAY: [[i32; 13]; 2] = [
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365],
    [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366],
];

/// C's `struct tm` with the BSD fields `tm_gmtoff` and `tm_zone`
/// (`None` is a NULL `tm_zone`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub mday: i32,
    pub mon: i32,
    pub year: i32,
    pub wday: i32,
    pub yday: i32,
    pub isdst: i32,
    pub gmtoff: i64,
    pub zone: Option<crate::value::RString>,
}

/// glibc's `__isleap`.
pub fn isleap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

// ----------------------------------------------------------------------
// time/offtime.c

/// `__offtime`: break `t` + `offset` down into year, yday, mon, mday,
/// wday, hour, min and sec. Returns false if the year overflows `int`.
pub fn offtime(t: i64, offset: i64, tp: &mut Tm) -> bool {
    let mut days = t / SECS_PER_DAY;
    let mut rem = t % SECS_PER_DAY;
    rem += offset;
    while rem < 0 {
        rem += SECS_PER_DAY;
        days -= 1;
    }
    while rem >= SECS_PER_DAY {
        rem -= SECS_PER_DAY;
        days += 1;
    }
    tp.hour = (rem / SECS_PER_HOUR) as i32;
    rem %= SECS_PER_HOUR;
    tp.min = (rem / 60) as i32;
    tp.sec = (rem % 60) as i32;
    // January 1, 1970 was a Thursday.
    tp.wday = ((4 + days) % 7) as i32;
    if tp.wday < 0 {
        tp.wday += 7;
    }
    let mut y: i64 = 1970;
    fn div(a: i64, b: i64) -> i64 {
        a / b - ((a % b < 0) as i64)
    }
    fn leaps_thru_end_of(y: i64) -> i64 {
        div(y, 4) - div(y, 100) + div(y, 400)
    }
    while days < 0 || days >= if isleap(y) { 366 } else { 365 } {
        // Guess a corrected year, assuming 365 days per year.
        let yg = y + days / 365 - ((days % 365 < 0) as i64);
        // Adjust DAYS and Y to match the guessed year.
        days -= (yg - y) * 365 + leaps_thru_end_of(yg - 1) - leaps_thru_end_of(y - 1);
        y = yg;
    }
    let year = y - 1900;
    if year < i32::MIN as i64 || year > i32::MAX as i64 {
        return false;
    }
    tp.year = year as i32;
    tp.yday = days as i32;
    let ip = &MON_YDAY[isleap(y) as usize];
    let mut m = 11;
    while days < ip[m] as i64 {
        m -= 1;
    }
    days -= ip[m] as i64;
    tp.mon = m as i32;
    tp.mday = (days + 1) as i32;
    true
}

// ----------------------------------------------------------------------
// time/tzfile.c

#[derive(Clone, Copy, Debug, Default)]
struct TtInfo {
    offset: i32,
    isdst: u8,
    idx: u8,
    isstd: bool,
    isgmt: bool,
}

#[derive(Clone, Debug, Default)]
struct TzFile {
    transitions: Vec<i64>,
    type_idxs: Vec<u8>,
    types: Vec<TtInfo>,
    zone_names: Vec<u8>,
    leaps: Vec<(i64, i64)>,
    tzspec: Option<String>,
}

impl TzFile {
    fn name(&self, idx: usize) -> crate::value::RString {
        let bytes = self.zone_names.get(idx..).unwrap_or(&[]);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        crate::value::RString::from(String::from_utf8_lossy(&bytes[..end]).as_ref())
    }
}

fn decode32(b: &[u8]) -> i32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn decode64(b: &[u8]) -> i64 {
    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Parse a TZif file as `__tzfile_read` does; `None` where glibc would
/// `goto lose`.
fn parse_tzfile(data: &[u8]) -> Option<TzFile> {
    let mut pos = 0usize;
    let mut trans_width = 4usize;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let s = data.get(*pos..*pos + n)?;
        *pos += n;
        Some(s)
    };
    let (num_transitions, num_types, chars, num_leaps, num_isstd, num_isgmt) = loop {
        let head = take(&mut pos, 44)?;
        if &head[0..4] != b"TZif" {
            return None;
        }
        let version = head[4];
        let num_isgmt = decode32(&head[20..]) as u32 as usize;
        let num_isstd = decode32(&head[24..]) as u32 as usize;
        let num_leaps = decode32(&head[28..]) as u32 as usize;
        let num_transitions = decode32(&head[32..]) as u32 as usize;
        let num_types = decode32(&head[36..]) as u32 as usize;
        let chars = decode32(&head[40..]) as u32 as usize;
        if num_isstd > num_types || num_isgmt > num_types {
            return None;
        }
        if trans_width == 4 && version != 0 {
            // We use the 8-byte format: skip the version 1 data.
            trans_width = 8;
            let to_skip = num_transitions * (4 + 1) + num_types * 6 + chars + num_leaps * 8 + num_isstd + num_isgmt;
            take(&mut pos, to_skip)?;
            continue;
        }
        break (num_transitions, num_types, chars, num_leaps, num_isstd, num_isgmt);
    };
    // Size of the POSIX TZ string at the end of the file.
    let mut tzspec_len = 0usize;
    if trans_width == 8 {
        let rem = data.len().checked_sub(pos)?;
        let fixed = num_transitions * (8 + 1) + num_types * 6 + chars;
        if rem < fixed {
            return None;
        }
        tzspec_len = rem - fixed;
        if tzspec_len < num_leaps * 12 {
            return None;
        }
        tzspec_len -= num_leaps * 12;
        if tzspec_len < num_isstd {
            return None;
        }
        tzspec_len -= num_isstd;
        if tzspec_len == 0 || tzspec_len - 1 < num_isgmt {
            return None;
        }
        tzspec_len -= num_isgmt + 1;
        if tzspec_len == 0 {
            return None;
        }
    }
    let mut f = TzFile::default();
    let tb = take(&mut pos, num_transitions * trans_width)?;
    f.transitions =
        tb.chunks(trans_width).map(|c| if trans_width == 4 { decode32(c) as i64 } else { decode64(c) }).collect();
    f.type_idxs = take(&mut pos, num_transitions)?.to_vec();
    if f.type_idxs.iter().any(|&t| t as usize >= num_types) {
        return None;
    }
    for _ in 0..num_types {
        let x = take(&mut pos, 6)?;
        let isdst = x[4];
        if isdst > 1 {
            return None;
        }
        let idx = x[5];
        if idx as usize > chars {
            return None;
        }
        f.types.push(TtInfo { offset: decode32(x), isdst, idx, isstd: false, isgmt: false });
    }
    f.zone_names = take(&mut pos, chars)?.to_vec();
    for _ in 0..num_leaps {
        let x = take(&mut pos, trans_width)?;
        let tr = if trans_width == 4 { decode32(x) as i64 } else { decode64(x) };
        let c = take(&mut pos, 4)?;
        f.leaps.push((tr, decode32(c) as i64));
    }
    for i in 0..num_isstd {
        f.types[i].isstd = take(&mut pos, 1)?[0] != 0;
    }
    for i in 0..num_isgmt {
        f.types[i].isgmt = take(&mut pos, 1)?[0] != 0;
    }
    if trans_width == 8 {
        // Skip over the newline first.
        let nl = take(&mut pos, 1);
        let spec = take(&mut pos, tzspec_len - 1);
        if let (Some([b'\n']), Some(s)) = (nl, spec) {
            let s = String::from_utf8_lossy(s).into_owned();
            // Don't use an empty TZ string.
            if !s.is_empty() {
                f.tzspec = Some(s);
            }
        }
    }
    Some(f)
}

// ----------------------------------------------------------------------
// time/tzset.c

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum RuleType {
    #[default]
    J0,
    J1,
    M,
}

#[derive(Clone, Debug)]
struct TzRule {
    name: crate::value::RString,
    typ: RuleType,
    m: u16,
    n: u16,
    d: u16,
    secs: i32,
    offset: i32,
    change: i64,
    computed_for: i32,
}

impl Default for TzRule {
    fn default() -> Self {
        TzRule {
            name: crate::value::RString::from(""),
            typ: RuleType::J0,
            m: 0,
            n: 0,
            d: 0,
            secs: 0,
            offset: 0,
            change: 0,
            computed_for: 0,
        }
    }
}

struct State {
    initialized: bool,
    old_tz: Option<String>,
    use_tzfile: bool,
    file: Rc<TzFile>,
    /// (dev, ino, mtime) of the loaded file, as glibc's change check.
    file_id: (u64, u64, i64),
    rules: [TzRule; 2],
    tzname: [crate::value::RString; 2],
    /// mktime's `localtime_offset`.
    localtime_offset: i64,
}

impl Default for State {
    fn default() -> Self {
        State {
            initialized: false,
            old_tz: None,
            use_tzfile: false,
            file: Rc::new(TzFile::default()),
            file_id: (0, 0, 0),
            rules: [TzRule::default(), TzRule::default()],
            tzname: [crate::value::RString::from("GMT"), crate::value::RString::from("GMT")],
            localtime_offset: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// `Some(v)`: the emulated value of `TZ` (`None` = unset) while R has
    /// it set via `set_tz`; `None`: use the process environment.
    static TZ_OVERRIDE: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

/// `getenv("TZ")`.
pub fn getenv_tz() -> Option<String> {
    TZ_OVERRIDE.with(|o| match &*o.borrow() {
        Some(v) => v.clone(),
        None => std::env::var("TZ").ok(),
    })
}

/// `setenv("TZ", ...)` / `unsetenv("TZ")` (with `None`).
pub fn set_env_tz(v: Option<&str>) {
    TZ_OVERRIDE.with(|o| *o.borrow_mut() = Some(v.map(|s| s.to_string())));
}

/// Drop the override, going back to the process environment.
pub fn clear_env_tz_override() {
    TZ_OVERRIDE.with(|o| *o.borrow_mut() = None);
}

/// The emulated-`TZ` state, for saving and restoring it.
pub type EnvTzState = Option<Option<String>>;

pub fn env_tz_state() -> EnvTzState {
    TZ_OVERRIDE.with(|o| o.borrow().clone())
}

pub fn restore_env_tz_state(st: EnvTzState) {
    TZ_OVERRIDE.with(|o| *o.borrow_mut() = st);
}

/// `sscanf`'s `%hu` at `s[i..]`: the value and the index after it.
fn scan_hu(s: &[u8], mut i: usize) -> Option<(u16, usize)> {
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut v: u64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((s[i] - b'0') as u64);
        i += 1;
    }
    if i == start {
        return None;
    }
    let v = if neg { (v as u16).wrapping_neg() } else { v as u16 };
    Some((v, i))
}

/// glibc's `sscanf(s, "%hu%n:%hu%n:%hu%n", ...)`: returns the parsed
/// numbers and the bytes consumed by the last successful conversion.
fn scan_hms(s: &[u8]) -> (Vec<u16>, usize) {
    let mut out = Vec::new();
    let mut consumed = 0;
    let mut i = 0;
    for k in 0..3 {
        if k > 0 {
            if s.get(i) != Some(&b':') {
                break;
            }
            i += 1;
        }
        match scan_hu(s, i) {
            Some((v, j)) => {
                out.push(v);
                i = j;
                consumed = j;
            }
            None => break,
        }
    }
    (out, consumed)
}

fn compute_offset(ss: u32, mm: u32, hh: u32) -> u32 {
    ss.min(59) + mm.min(59) * 60 + hh.min(24) * 60 * 60
}

impl State {
    fn update_vars(&mut self) {
        self.tzname[0] = self.rules[0].name.clone();
        self.tzname[1] = self.rules[1].name.clone();
    }

    /// `parse_tzname`.
    fn parse_tzname(&mut self, tz: &[u8], pos: &mut usize, which: usize) -> bool {
        let start = *pos;
        let mut p = start;
        while p < tz.len() && tz[p].is_ascii_alphabetic() {
            p += 1;
        }
        let (s, e, next) = if p - start < 3 {
            let mut p = *pos;
            if tz.get(p) != Some(&b'<') {
                return false;
            }
            p += 1;
            let s = p;
            while p < tz.len() && (tz[p].is_ascii_alphanumeric() || tz[p] == b'+' || tz[p] == b'-') {
                p += 1;
            }
            let e = p;
            if tz.get(p) != Some(&b'>') || e - s < 3 {
                return false;
            }
            (s, e, p + 1)
        } else {
            (start, p, p)
        };
        self.rules[which].name = crate::value::RString::from(String::from_utf8_lossy(&tz[s..e]).as_ref());
        *pos = next;
        true
    }

    /// `parse_offset`.
    fn parse_offset(&mut self, tz: &[u8], pos: &mut usize, which: usize) -> bool {
        let mut p = *pos;
        let c = tz.get(p).copied().unwrap_or(0);
        if which == 0 && (c == 0 || (c != b'+' && c != b'-' && !c.is_ascii_digit())) {
            return false;
        }
        let sign: i32 = if c == b'-' || c == b'+' {
            p += 1;
            if c == b'-' { 1 } else { -1 }
        } else {
            -1
        };
        *pos = p;
        let (nums, consumed) = scan_hms(&tz[p..]);
        if !nums.is_empty() {
            let hh = nums[0] as u32;
            let mm = nums.get(1).copied().unwrap_or(0) as u32;
            let ss = nums.get(2).copied().unwrap_or(0) as u32;
            self.rules[which].offset = sign * compute_offset(ss, mm, hh) as i32;
        } else if which == 0 {
            // Standard time defaults to offset zero.
            self.rules[0].offset = 0;
            return false;
        } else {
            // DST defaults to one hour later than standard time.
            self.rules[1].offset = self.rules[0].offset + 60 * 60;
        }
        *pos = p + consumed;
        true
    }

    /// `parse_rule`.
    fn parse_rule(&mut self, tz: &[u8], pos: &mut usize, which: usize) -> bool {
        let mut p = *pos;
        let at = |i: usize| tz.get(i).copied().unwrap_or(0);
        // Ignore comma to support string following the incorrect
        // specification in early POSIX.1 printings.
        if at(p) == b',' {
            p += 1;
        }
        let r = &mut self.rules[which];
        if at(p) == b'J' || at(p).is_ascii_digit() {
            r.typ = if at(p) == b'J' { RuleType::J1 } else { RuleType::J0 };
            if r.typ == RuleType::J1 {
                p += 1;
                if !at(p).is_ascii_digit() {
                    return false;
                }
            }
            // strtoul
            let s = p;
            let mut d: u64 = 0;
            while at(p).is_ascii_digit() {
                d = d.saturating_mul(10).saturating_add((at(p) - b'0') as u64);
                p += 1;
            }
            if p == s || d > 365 {
                return false;
            }
            if r.typ == RuleType::J1 && d == 0 {
                return false;
            }
            r.d = d as u16;
        } else if at(p) == b'M' {
            r.typ = RuleType::M;
            // sscanf (tz, "M%hu.%hu.%hu%n", ...) == 3
            let mut q = p + 1;
            let mut vals = [0u16; 3];
            for (k, v) in vals.iter_mut().enumerate() {
                if k > 0 {
                    if at(q) != b'.' {
                        return false;
                    }
                    q += 1;
                }
                match scan_hu(tz, q) {
                    Some((x, j)) => {
                        *v = x;
                        q = j;
                    }
                    None => return false,
                }
            }
            r.m = vals[0];
            r.n = vals[1];
            r.d = vals[2];
            if r.m < 1 || r.m > 12 || r.n < 1 || r.n > 5 || r.d > 6 {
                return false;
            }
            p = q;
        } else if at(p) == 0 {
            // Daylight time rules in the U.S. (Energy Policy Act of 2005).
            r.typ = RuleType::M;
            if which == 0 {
                r.m = 3;
                r.n = 2;
                r.d = 0;
            } else {
                r.m = 11;
                r.n = 1;
                r.d = 0;
            }
        } else {
            return false;
        }
        if at(p) != 0 && at(p) != b'/' && at(p) != b',' {
            return false;
        } else if at(p) == b'/' {
            // Get the time of day of the change.
            p += 1;
            if at(p) == 0 {
                return false;
            }
            let negative = at(p) == b'-';
            if negative {
                p += 1;
            }
            let (nums, consumed) = scan_hms(&tz[p.min(tz.len())..]);
            let hh = nums.first().copied().unwrap_or(2) as i32;
            let mm = nums.get(1).copied().unwrap_or(0) as i32;
            let ss = nums.get(2).copied().unwrap_or(0) as i32;
            p += consumed;
            r.secs = (if negative { -1 } else { 1 }) * (hh * 60 * 60 + mm * 60 + ss);
        } else {
            // Default to 2:00 AM.
            r.secs = 2 * 60 * 60;
        }
        r.computed_for = -1;
        *pos = p;
        true
    }

    /// `__tzset_parse_tz`. `allow_default` is false when called for the
    /// TZ string of a loaded file (where glibc would recursively replace
    /// the file being used; no real tzdata footer lacks rules).
    fn tzset_parse_tz(&mut self, tz: &str, allow_default: bool) {
        let tz = tz.as_bytes();
        // Clear out old state and reset to unnamed UTC.
        self.rules = [TzRule::default(), TzRule::default()];
        let mut p = 0usize;
        if self.parse_tzname(tz, &mut p, 0) && self.parse_offset(tz, &mut p, 0) {
            if p < tz.len() {
                if self.parse_tzname(tz, &mut p, 1) {
                    self.parse_offset(tz, &mut p, 1);
                    if (p >= tz.len() || (tz[p] == b',' && p + 1 == tz.len())) && allow_default {
                        // There is no rule. See if there is a default rule file.
                        let (n0, n1) = (self.rules[0].name.clone(), self.rules[1].name.clone());
                        let (o0, o1) = (self.rules[0].offset, self.rules[1].offset);
                        self.tzfile_default(&n0, &n1, o0, o1);
                        if self.use_tzfile {
                            self.old_tz = None;
                            return;
                        }
                    }
                }
                // Figure out the standard <-> DST rules.
                if self.parse_rule(tz, &mut p, 0) {
                    self.parse_rule(tz, &mut p, 1);
                }
            } else {
                // There is no DST.
                self.rules[1].name = self.rules[0].name.clone();
                self.rules[1].offset = self.rules[0].offset;
            }
        }
        self.update_vars();
    }

    /// `tzset_internal`.
    fn tzset_internal(&mut self, always: bool) {
        if self.initialized && !always {
            return;
        }
        self.initialized = true;
        let mut tz = getenv_tz();
        if tz.as_deref() == Some("") {
            // User specified the empty string; use UTC explicitly.
            tz = Some("Universal".to_string());
        }
        if let Some(t) = &tz {
            if let Some(rest) = t.strip_prefix(':') {
                tz = Some(rest.to_string());
            }
        }
        if let (Some(old), Some(t)) = (&self.old_tz, &tz) {
            if old == t {
                return;
            }
        }
        let tz = tz.unwrap_or_else(|| TZDEFAULT.to_string());
        self.old_tz = Some(tz.clone());
        self.tzfile_read(&tz);
        if self.use_tzfile {
            return;
        }
        if tz.is_empty() || tz == TZDEFAULT {
            self.rules = [TzRule::default(), TzRule::default()];
            self.rules[0].name = crate::value::RString::from("UTC");
            self.rules[1].name = crate::value::RString::from("UTC");
            self.rules[0].change = -1;
            self.rules[1].change = -1;
            self.update_vars();
            return;
        }
        self.tzset_parse_tz(&tz, true);
    }

    /// `__tzfile_read`.
    fn tzfile_read(&mut self, file: &str) {
        let was_using_tzfile = self.use_tzfile;
        self.use_tzfile = false;
        if file.is_empty() {
            self.file = Rc::new(TzFile::default());
            return;
        }
        let path = if file.starts_with('/') {
            file.to_string()
        } else {
            let tzdir = match std::env::var("TZDIR") {
                Ok(d) if !d.is_empty() => d,
                _ => TZDIR.to_string(),
            };
            format!("{}/{}", tzdir, file)
        };
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(&path).ok();
        if was_using_tzfile {
            if let Some(m) = &meta {
                if self.file_id == (m.dev(), m.ino(), m.mtime()) {
                    self.use_tzfile = true;
                    return;
                }
            }
        }
        let parsed = std::fs::read(&path).ok().and_then(|d| parse_tzfile(&d));
        let Some(f) = parsed else {
            self.file = Rc::new(TzFile::default());
            return;
        };
        if let Some(m) = &meta {
            self.file_id = (m.dev(), m.ino(), m.mtime());
        }
        // Find the standard and daylight time abbreviations: those of the
        // latest transitions.
        let mut names: [Option<crate::value::RString>; 2] = [None, None];
        for i in (0..f.transitions.len()).rev() {
            let ty = f.types[f.type_idxs[i] as usize];
            let dst = ty.isdst as usize;
            if names[dst].is_none() {
                names[dst] = Some(f.name(ty.idx as usize));
                if names[1 - dst].is_some() {
                    break;
                }
            }
        }
        let n0 = names[0].clone().unwrap_or_else(|| f.name(0));
        let n1 = names[1].clone().unwrap_or_else(|| n0.clone());
        self.tzname = [n0, n1];
        self.file = Rc::new(f);
        self.use_tzfile = true;
    }

    /// `__tzfile_default`: the user's names and offsets with the rules of
    /// the TZDEFRULES file.
    fn tzfile_default(&mut self, std: &crate::value::RString, dst: &crate::value::RString, stdoff: i32, dstoff: i32) {
        self.tzfile_read(TZDEFRULES);
        if !self.use_tzfile {
            return;
        }
        let mut f = (*self.file).clone();
        if f.types.len() < 2 {
            self.use_tzfile = false;
            return;
        }
        // rule_stdoff/rule_dstoff as computed by __tzfile_read.
        let (mut rule_stdoff, mut rule_dstoff) = (0i32, 0i32);
        if f.transitions.is_empty() {
            rule_stdoff = f.types[0].offset;
            rule_dstoff = rule_stdoff;
        } else {
            let mut i = f.transitions.len();
            let mut std_set = false;
            let mut dst_set = false;
            while i > 0 {
                i -= 1;
                let t = f.types[f.type_idxs[i] as usize];
                if !std_set && t.isdst == 0 {
                    std_set = true;
                    rule_stdoff = t.offset;
                } else if !dst_set && t.isdst != 0 {
                    dst_set = true;
                    rule_dstoff = t.offset;
                }
                if std_set && dst_set {
                    break;
                }
            }
            if !dst_set {
                rule_dstoff = rule_stdoff;
            }
        }
        let mut names = Vec::new();
        names.extend_from_slice(std.as_bytes());
        names.push(0);
        names.extend_from_slice(dst.as_bytes());
        names.push(0);
        let mut isdst = false;
        for i in 0..f.transitions.len() {
            let tt = f.types[f.type_idxs[i] as usize];
            f.type_idxs[i] = tt.isdst;
            if tt.isgmt {
                // The transition time is in GMT. No correction to apply.
            } else if isdst && !tt.isstd {
                f.transitions[i] += (dstoff - rule_dstoff) as i64;
            } else {
                f.transitions[i] += (stdoff - rule_stdoff) as i64;
            }
            isdst = tt.isdst != 0;
        }
        f.types.truncate(2);
        f.types[0] = TtInfo { offset: stdoff, isdst: 0, idx: 0, ..f.types[0] };
        f.types[1] = TtInfo { offset: dstoff, isdst: 1, idx: (std.len() + 1) as u8, ..f.types[1] };
        f.zone_names = names;
        self.tzname = [std.clone(), dst.clone()];
        self.file = Rc::new(f);
        // Force rereading TZDEFRULES the next time it is used.
        self.file_id = (0, 0, 0);
    }

    /// `compute_change`.
    fn compute_change(rule: &mut TzRule, year: i32) {
        if year != -1 && rule.computed_for == year {
            return;
        }
        let y = year as i64;
        let mut t: i64 = if year > 1970 {
            ((y - 1970) * 365 + ((y - 1) / 4 - 1970 / 4) - ((y - 1) / 100 - 1970 / 100) + ((y - 1) / 400 - 1970 / 400))
                * SECS_PER_DAY
        } else {
            0
        };
        match rule.typ {
            RuleType::J1 => {
                t += (rule.d as i64 - 1) * SECS_PER_DAY;
                if rule.d >= 60 && isleap(y) {
                    t += SECS_PER_DAY;
                }
            }
            RuleType::J0 => {
                t += rule.d as i64 * SECS_PER_DAY;
            }
            RuleType::M => {
                let leap = isleap(y) as usize;
                let m = rule.m as usize;
                let myday_m1 = MON_YDAY[leap][m - 1];
                let myday_0 = MON_YDAY[leap][m];
                t += myday_m1 as i64 * SECS_PER_DAY;
                // Zeller's Congruence: day-of-week of the first day of month.
                let m1 = (rule.m as i32 + 9) % 12 + 1;
                let yy0 = if rule.m <= 2 { year - 1 } else { year };
                let yy1 = yy0 / 100;
                let yy2 = yy0 % 100;
                let mut dow = ((26 * m1 - 2) / 10 + 1 + yy2 + yy2 / 4 + yy1 / 4 - 2 * yy1) % 7;
                if dow < 0 {
                    dow += 7;
                }
                let mut d = rule.d as i32 - dow;
                if d < 0 {
                    d += 7;
                }
                for _ in 1..rule.n {
                    if d + 7 >= myday_0 - myday_m1 {
                        break;
                    }
                    d += 7;
                }
                t += d as i64 * SECS_PER_DAY;
            }
        }
        rule.change = t - rule.offset as i64 + rule.secs as i64;
        rule.computed_for = year;
    }

    /// `__tz_compute`.
    fn tz_compute(&mut self, timer: i64, tm: &mut Tm, use_localtime: bool) {
        let year = (1900i64 + tm.year as i64) as i32;
        Self::compute_change(&mut self.rules[0], year);
        Self::compute_change(&mut self.rules[1], year);
        if use_localtime {
            let isdst = if self.rules[0].change > self.rules[1].change {
                timer < self.rules[1].change || timer >= self.rules[0].change
            } else {
                timer >= self.rules[0].change && timer < self.rules[1].change
            } as usize;
            tm.isdst = isdst as i32;
            tm.zone = Some(self.tzname[isdst].clone());
            tm.gmtoff = self.rules[isdst].offset as i64;
        }
    }

    /// `__tzfile_compute`; returns (leap correction, leap hit).
    fn tzfile_compute(&mut self, timer: i64, use_localtime: bool, tp: &mut Tm) -> (i64, i32) {
        let f = self.file.clone();
        if use_localtime {
            let mut tzname: [Option<crate::value::RString>; 2] = [None, None];
            let nt = f.transitions.len();
            let ntypes = f.types.len();
            let info: usize;
            if nt == 0 || timer < f.transitions[0] {
                // Before any transition: the first non-DST type.
                let mut i = 0;
                while i < ntypes && f.types[i].isdst != 0 {
                    if tzname[1].is_none() {
                        tzname[1] = Some(f.name(f.types[i].idx as usize));
                    }
                    i += 1;
                }
                if i == ntypes {
                    i = 0;
                }
                tzname[0] = Some(f.name(f.types[i].idx as usize));
                if tzname[1].is_none() {
                    let mut j = i;
                    while j < ntypes {
                        if f.types[j].isdst != 0 {
                            tzname[1] = Some(f.name(f.types[j].idx as usize));
                            break;
                        }
                        j += 1;
                    }
                }
                info = i;
            } else {
                let mut i: usize;
                let mut use_spec = false;
                if timer >= f.transitions[nt - 1] {
                    i = nt;
                    if let Some(spec) = &f.tzspec {
                        use_spec = true;
                        let _ = spec;
                    }
                } else {
                    // Find the first transition after TIMER.
                    let mut lo = 0usize;
                    let mut hi = nt - 1;
                    let guess = (f.transitions[nt - 1] - timer) / 15778476;
                    let mut found = false;
                    i = 0;
                    if (guess as u64) < nt as u64 {
                        i = nt - 1 - guess as usize;
                        if timer < f.transitions[i] {
                            if i < 10 || timer >= f.transitions[i - 10] {
                                while timer < f.transitions[i - 1] {
                                    i -= 1;
                                }
                                found = true;
                            } else {
                                hi = i - 10;
                            }
                        } else if i + 10 >= nt || timer < f.transitions[i + 10] {
                            while timer >= f.transitions[i] {
                                i += 1;
                            }
                            found = true;
                        } else {
                            lo = i + 10;
                        }
                    }
                    if !found {
                        while lo + 1 < hi {
                            i = (lo + hi) / 2;
                            if timer < f.transitions[i] {
                                hi = i;
                            } else {
                                lo = i;
                            }
                        }
                        i = hi;
                    }
                }
                if use_spec {
                    let spec = f.tzspec.clone().unwrap();
                    self.tzset_parse_tz(&spec, false);
                    let mut utc = tp.clone();
                    if offtime(timer, 0, &mut utc) {
                        *tp = utc;
                        self.tz_compute(timer, tp, true);
                        return self.leap_correction(&f, timer);
                    }
                    i = nt;
                }
                // found:
                let prev = f.types[f.type_idxs[i - 1] as usize];
                tzname[prev.isdst as usize] = Some(f.name(prev.idx as usize));
                let mut j = i;
                while j < nt {
                    let ty = f.types[f.type_idxs[j] as usize];
                    let dst = ty.isdst as usize;
                    if tzname[dst].is_none() {
                        tzname[dst] = Some(f.name(ty.idx as usize));
                        if tzname[1 - dst].is_some() {
                            break;
                        }
                    }
                    j += 1;
                }
                if tzname[0].is_none() {
                    tzname[0] = tzname[1].clone();
                }
                info = f.type_idxs[i - 1] as usize;
            }
            let ti = f.types[info];
            let n0 = tzname[0].clone().unwrap_or_else(|| f.name(0));
            let n1 = tzname[1].clone().unwrap_or_else(|| n0.clone());
            self.tzname = [n0, n1];
            tp.isdst = ti.isdst as i32;
            tp.zone = Some(self.tzname[ti.isdst as usize].clone());
            tp.gmtoff = ti.offset as i64;
        }
        self.leap_correction(&f, timer)
    }

    fn leap_correction(&self, f: &TzFile, timer: i64) -> (i64, i32) {
        let mut i = f.leaps.len();
        loop {
            if i == 0 {
                return (0, 0);
            }
            i -= 1;
            if timer >= f.leaps[i].0 {
                break;
            }
        }
        let correct = f.leaps[i].1;
        let mut hit = 0;
        if timer == f.leaps[i].0 && f.leaps[i].1 > if i == 0 { 0 } else { f.leaps[i - 1].1 } {
            hit = 1;
            while i > 0 && f.leaps[i].0 == f.leaps[i - 1].0 + 1 && f.leaps[i].1 == f.leaps[i - 1].1 + 1 {
                hit += 1;
                i -= 1;
            }
        }
        (correct, hit)
    }

    /// `__tz_convert`.
    fn tz_convert(&mut self, timer: i64, use_localtime: bool) -> Option<Tm> {
        self.tzset_internal(false);
        let mut tp = Tm::default();
        let (lc, extra) = if self.use_tzfile {
            self.tzfile_compute(timer, use_localtime, &mut tp)
        } else {
            if !offtime(timer, 0, &mut tp) {
                return None;
            }
            self.tz_compute(timer, &mut tp, use_localtime);
            (0, 0)
        };
        if !use_localtime {
            tp.isdst = 0;
            tp.zone = Some(crate::value::RString::from("GMT"));
            tp.gmtoff = 0;
        }
        if offtime(timer, tp.gmtoff - lc, &mut tp) {
            tp.sec += extra;
            Some(tp)
        } else {
            None
        }
    }

    fn tzset(&mut self) {
        self.tzset_internal(true);
        if !self.use_tzfile {
            self.update_vars();
        }
    }
}

// ----------------------------------------------------------------------
// public C-library-like entry points

/// `tzset()`.
pub fn tzset() {
    STATE.with(|s| s.borrow_mut().tzset());
}

/// The current `tzname[0]`, `tzname[1]`.
pub fn tzname() -> [crate::value::RString; 2] {
    STATE.with(|s| s.borrow().tzname.clone())
}

/// `localtime_r`.
pub fn localtime_r(t: i64) -> Option<Tm> {
    STATE.with(|s| s.borrow_mut().tz_convert(t, true))
}

/// `gmtime_r`.
pub fn gmtime_r(t: i64) -> Option<Tm> {
    STATE.with(|s| s.borrow_mut().tz_convert(t, false))
}

/// `mktime`: `Err(())` is a return of -1 with `errno` set.
pub fn mktime(tp: &mut Tm) -> Result<i64, ()> {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.tzset();
        let mut off = st.localtime_offset;
        drop(st);
        let r = mktime_internal(tp, &mut off);
        s.borrow_mut().localtime_offset = off;
        r
    })
}

// ----------------------------------------------------------------------
// time/mktime.c

fn leapyear(year: i64) -> bool {
    (year & 3) == 0 && (year % 100 != 0 || ((year / 100) & 3) == ((-(TM_YEAR_BASE / 100)) & 3))
}

fn isdst_differ(a: i32, b: i32) -> bool {
    ((a == 0) != (b == 0)) && 0 <= a && 0 <= b
}

#[allow(clippy::too_many_arguments)]
fn ydhms_diff(
    year1: i64,
    yday1: i64,
    hour1: i64,
    min1: i64,
    sec1: i64,
    year0: i64,
    yday0: i64,
    hour0: i64,
    min0: i64,
    sec0: i64,
) -> i64 {
    // Compute intervening leap days correctly even if year is negative.
    let a4 = (year1 >> 2) + (TM_YEAR_BASE >> 2) - ((year1 & 3) == 0) as i64;
    let b4 = (year0 >> 2) + (TM_YEAR_BASE >> 2) - ((year0 & 3) == 0) as i64;
    let a100 = (a4 + (a4 < 0) as i64) / 25 - (a4 < 0) as i64;
    let b100 = (b4 + (b4 < 0) as i64) / 25 - (b4 < 0) as i64;
    let a400 = a100 >> 2;
    let b400 = b100 >> 2;
    let intervening_leap_days = (a4 - b4) - (a100 - b100) + (a400 - b400);
    let years = year1 - year0;
    let days = 365 * years + yday1 - yday0 + intervening_leap_days;
    let hours = 24 * days + hour1 - hour0;
    let minutes = 60 * hours + min1 - min0;
    60 * minutes + sec1 - sec0
}

fn tm_diff(year: i64, yday: i64, hour: i64, min: i64, sec: i64, tp: &Tm) -> i64 {
    ydhms_diff(year, yday, hour, min, sec, tp.year as i64, tp.yday as i64, tp.hour as i64, tp.min as i64, tp.sec as i64)
}

/// `ranged_convert` with `__localtime64_r`.
fn ranged_convert(t: &mut i64) -> Option<Tm> {
    if let Some(r) = localtime_r(*t) {
        return Some(r);
    }
    // Binary search between a known bad and a known good value.
    let mut bad = *t;
    let mut ok: i64 = 0;
    let mut oktm: Option<Tm> = None;
    loop {
        let mid = (ok >> 1) + (bad >> 1) + ((ok | bad) & 1);
        if mid == ok || mid == bad {
            break;
        }
        match localtime_r(mid) {
            Some(tm) => {
                ok = mid;
                oktm = Some(tm);
            }
            None => bad = mid,
        }
    }
    let tm = oktm?;
    *t = ok;
    Some(tm)
}

/// `__mktime_internal` with `convert = __localtime64_r`.
fn mktime_internal(tp: &mut Tm, offset: &mut i64) -> Result<i64, ()> {
    let mut remaining_probes = 6;
    let mut sec = tp.sec as i64;
    let min = tp.min as i64;
    let hour = tp.hour as i64;
    let mday = tp.mday as i64;
    let mon = tp.mon;
    let year_requested = tp.year as i64;
    let isdst = tp.isdst;
    let mut dst2 = false;

    // Ensure that mon is in range, and set year accordingly.
    let mon_remainder = mon % 12;
    let negative_mon_remainder = (mon_remainder < 0) as i32;
    let mon_years = mon / 12 - negative_mon_remainder;
    let year = year_requested + mon_years as i64;
    let mon_yday = MON_YDAY[leapyear(year) as usize][(mon_remainder + 12 * negative_mon_remainder) as usize] as i64 - 1;
    let yday = mon_yday + mday;

    let off = *offset;
    let sec_requested = sec;
    // LEAP_SECONDS_POSSIBLE
    sec = sec.clamp(0, 59);

    let negative_offset_guess = off.wrapping_neg();
    let t0 = ydhms_diff(year, yday, hour, min, sec, EPOCH_YEAR - TM_YEAR_BASE, 0, 0, 0, negative_offset_guess);
    let mut t = t0;
    let mut t1 = t0;
    let mut t2 = t0;
    let mut tm: Tm;
    let mut goto_offset_found = false;
    loop {
        tm = ranged_convert(&mut t).ok_or(())?;
        let dt = tm_diff(year, yday, hour, min, sec, &tm);
        if dt == 0 {
            break;
        }
        if t == t1
            && t != t2
            && (tm.isdst < 0
                || if isdst < 0 { (dst2 as i32) <= (tm.isdst != 0) as i32 } else { (isdst != 0) != (tm.isdst != 0) })
        {
            // Oscillating around a spring-forward gap.
            goto_offset_found = true;
            break;
        }
        remaining_probes -= 1;
        if remaining_probes == 0 {
            return Err(());
        }
        t1 = t2;
        t2 = t;
        t += dt;
        dst2 = tm.isdst != 0;
    }

    if !goto_offset_found && isdst_differ(isdst, tm.isdst) {
        // tm.tm_isdst has the wrong value. Look for a neighboring time
        // with the right value, and use its UTC offset.
        let dst_difference = (isdst == 0) as i64 - (tm.isdst == 0) as i64;
        let stride: i64 = 601200;
        let duration_max: i64 = 457243209;
        let delta_bound = duration_max / 2 + stride;
        let mut found = false;
        let mut delta = stride;
        'outer: while delta < delta_bound {
            for direction in [-1i64, 1] {
                let mut ot = t + delta * direction;
                let otm = ranged_convert(&mut ot).ok_or(())?;
                if !isdst_differ(isdst, otm.isdst) {
                    // Extrapolate back to the desired time.
                    let gt = ot + tm_diff(year, yday, hour, min, sec, &otm);
                    if let Some(r) = localtime_r(gt) {
                        tm = r;
                        t = gt;
                        found = true;
                        break 'outer;
                    }
                }
            }
            delta += stride;
        }
        if !found {
            // No unusual DST offset was found nearby. Assume one-hour DST.
            t += 60 * 60 * dst_difference;
            match localtime_r(t) {
                Some(r) => tm = r,
                None => return Err(()),
            }
        }
    }

    // offset_found:
    *offset = t.wrapping_sub(t0).wrapping_sub(negative_offset_guess);

    if sec_requested != tm.sec as i64 {
        // Adjust time to reflect the tm_sec requested.
        let mut sec_adjustment = (sec == 0 && tm.sec == 60) as i64;
        sec_adjustment -= sec;
        sec_adjustment += sec_requested;
        t += sec_adjustment;
        tm = localtime_r(t).ok_or(())?;
    }
    *tp = tm;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offtime_epoch() {
        let mut tm = Tm::default();
        assert!(offtime(0, 0, &mut tm));
        assert_eq!((tm.year, tm.mon, tm.mday, tm.wday, tm.yday), (70, 0, 1, 4, 0));
        assert!(offtime(-1, 0, &mut tm));
        assert_eq!((tm.year, tm.mon, tm.mday, tm.hour, tm.min, tm.sec), (69, 11, 31, 23, 59, 59));
    }

    #[test]
    fn posix_rules() {
        set_env_tz(Some("EST5EDT,M3.2.0,M11.1.0"));
        tzset();
        let t = localtime_r(1721059200).unwrap(); // 2024-07-15 16:00 UTC
        assert_eq!((t.hour, t.isdst, t.gmtoff), (12, 1, -14400));
        assert_eq!(t.zone.as_deref(), Some("EDT"));
        let mut tm = Tm { year: 124, mon: 6, mday: 15, hour: 12, isdst: -1, ..Default::default() };
        assert_eq!(mktime(&mut tm), Ok(1721059200));
        clear_env_tz_override();
    }
}
