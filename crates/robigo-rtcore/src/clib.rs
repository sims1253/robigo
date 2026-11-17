//! The C library behind safe functions, for builtins that GNU R implements
//! with it (robigo-rtbuiltins forbids `unsafe`): the clock and broken-down
//! time (`time`, `localtime_r`, `gmtime_r`, which datetime.c calls on Linux)
//! and `wcstod` (Rstrptime.h's `%OS`). None of them changes the process: they
//! read the environment's `TZ` and the C library's locale, which robigo never
//! sets (the isolate's locale is its own, `Locale`).

#![allow(unsafe_code)]

/// C's `struct tm` as GNU R reads it (with glibc's `tm_gmtoff` and
/// `tm_zone`, `HAVE_TM_GMTOFF` and `HAVE_TM_ZONE`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// The zone's abbreviation, if the C library gave one.
    pub zone: Option<Vec<u8>>,
}

impl Tm {
    fn from_c(t: &libc::tm) -> Tm {
        let zone = if t.tm_zone.is_null() {
            None
        } else {
            // SAFETY: glibc's tm_zone points at a NUL-terminated abbreviation
            // in its own tables, alive for the process.
            Some(unsafe { std::ffi::CStr::from_ptr(t.tm_zone) }.to_bytes().to_vec())
        };
        Tm {
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
            zone,
        }
    }
}

/// `time(NULL)`.
pub fn time_now() -> i64 {
    // SAFETY: time with a null argument only returns the clock.
    unsafe { libc::time(std::ptr::null_mut()) }
}

/// `localtime_r(&t, &tm)`: `None` when the C library fails (a year that
/// does not fit).
pub fn localtime(t: i64) -> Option<Tm> {
    // SAFETY: a zeroed tm is a valid out-argument; localtime_r writes it and
    // reads only the time zone the environment names.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { None } else { Some(Tm::from_c(&tm)) }
    }
}

/// `gmtime_r(&t, &tm)`: `None` when the C library fails.
pub fn gmtime(t: i64) -> Option<Tm> {
    // SAFETY: as for localtime_r; gmtime_r reads no time zone.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::gmtime_r(&t, &mut tm).is_null() { None } else { Some(Tm::from_c(&tm)) }
    }
}

unsafe extern "C" {
    fn wcstod(nptr: *const libc::wchar_t, endptr: *mut *mut libc::wchar_t) -> f64;
    fn wcstod_l(nptr: *const libc::wchar_t, endptr: *mut *mut libc::wchar_t, loc: libc::locale_t) -> f64;
}

/// The C library's `C.UTF-8` locale, made once for the process (an
/// object, not the process's locale, which robigo never sets): GNU R's
/// `LC_CTYPE` in the oracle's environment, with `LC_NUMERIC` "C" as GNU R
/// always has it. `None` if the C library has no C.UTF-8.
fn c_utf8() -> Option<libc::locale_t> {
    static LOC: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let l = *LOC.get_or_init(|| {
        // SAFETY: newlocale makes a new object from a NUL-terminated name;
        // it is never freed (kept for the process, read only).
        unsafe { libc::newlocale(libc::LC_CTYPE_MASK, c"C.UTF-8".as_ptr(), std::ptr::null_mut()) as usize }
    });
    (l != 0).then_some(l as libc::locale_t)
}

/// `wcstod(s, &end)` in a UTF-8 locale (white space as `iswspace` reads it
/// there; the C library's own locale when it has no C.UTF-8) of the wide
/// characters `s` (a NUL is appended): the number and how many of `s` it
/// read.
pub fn wcstod_of(s: &[u32]) -> (f64, usize) {
    let mut buf: Vec<libc::wchar_t> = s.iter().map(|&c| c as libc::wchar_t).collect();
    buf.push(0);
    let mut end: *mut libc::wchar_t = std::ptr::null_mut();
    let v = match c_utf8() {
        // SAFETY: a NUL-terminated wide string, alive for the call; `end`
        // is set to a position inside it; `loc` is a live locale object.
        Some(loc) => unsafe { wcstod_l(buf.as_ptr(), &mut end, loc) },
        // SAFETY: as above, without a locale object.
        None => unsafe { wcstod(buf.as_ptr(), &mut end) },
    };
    // SAFETY: `end` points into `buf` (at its start when nothing was read).
    let used = unsafe { end.offset_from(buf.as_ptr()) } as usize;
    (v, used.min(s.len()))
}
