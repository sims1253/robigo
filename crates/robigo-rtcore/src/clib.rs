//! The C library behind safe functions, for builtins that GNU R implements
//! with it (robigo-rtbuiltins forbids `unsafe`): the clock and broken-down
//! time (`time`, `localtime_r`, `gmtime_r`, which datetime.c calls on Linux),
//! `wcstod` (Rstrptime.h's `%OS`), the password database's home
//! directories (sys-unix.c's `R_ExpandFileName_unix`), libintl's default
//! message directory (errors.c's `bindtextdomain`) and `MB_CUR_MAX`
//! (platform.c's `R_check_locale`). None of them changes the process. The
//! time and number functions read the environment's `TZ` and the C library's
//! locale, which robigo never sets (the isolate's locale is its own,
//! `Locale`); the password lookups read the password database; the message
//! directory is asked of libintl's binding table for a domain no one binds.
//!
//! Like the rest of robigo-rt (version.rs's `platform()`), this module is for
//! Linux with glibc, the one C library robigo-rt builds and is tested on (the
//! macOS lanes build robigo-cli only): `bindtextdomain` is glibc's libc's
//! (on macOS it is libintl's) and `__ctype_get_mb_cur_max` is what glibc's
//! `MB_CUR_MAX` expands to. A port to another C library starts here.

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
}

impl Tm {
    fn from_c(t: &libc::tm) -> Tm {
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
    fn wcstod_l(nptr: *const libc::wchar_t, endptr: *mut *mut libc::wchar_t, loc: libc::locale_t) -> f64;
    /// What glibc's and musl's `MB_CUR_MAX` macro calls: the most bytes of
    /// a character in the calling thread's `LC_CTYPE`.
    fn __ctype_get_mb_cur_max() -> libc::size_t;
}

/// The C library's `C.UTF-8` locale (`C.utf8` in older glibc), made once
/// for the process (an object, not the process's locale, which robigo never
/// sets): GNU R's `LC_CTYPE` in the oracle's environment, with
/// `LC_NUMERIC` "C" (the base), as GNU R always has it. `None` if the C
/// library has neither.
fn c_utf8() -> Option<libc::locale_t> {
    static LOC: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let l = *LOC.get_or_init(|| {
        for name in [c"C.UTF-8", c"C.utf8"] {
            // SAFETY: newlocale makes a new object from a NUL-terminated
            // name and no base; it is never freed (kept for the process,
            // read only).
            let l = unsafe { libc::newlocale(libc::LC_CTYPE_MASK, name.as_ptr(), std::ptr::null_mut()) };
            if !l.is_null() {
                return l as usize;
            }
        }
        0
    });
    (l != 0).then_some(l as libc::locale_t)
}

/// The C library's `C` locale (`LC_CTYPE`), made once for the process, as
/// [`c_utf8`]'s.
fn c_locale() -> Option<libc::locale_t> {
    static LOC: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    // SAFETY: as in c_utf8.
    let l = *LOC
        .get_or_init(|| unsafe { libc::newlocale(libc::LC_CTYPE_MASK, c"C".as_ptr(), std::ptr::null_mut()) } as usize);
    (l != 0).then_some(l as libc::locale_t)
}

/// `MB_CUR_MAX` in the C library's `C.UTF-8` (`utf8`) or `C` locale, the
/// two `LC_CTYPE`s robigo models (`Locale`). The C library answers for the
/// calling thread's locale, so the object is made this thread's (`uselocale`)
/// for the read only. `None` when the C library has no such locale (or under
/// Miri, which has no C library).
// R: platform.c:R_check_locale (MB_CUR_MAX)
pub fn mb_cur_max(utf8: bool) -> Option<usize> {
    if cfg!(miri) {
        return None;
    }
    let loc = if utf8 { c_utf8()? } else { c_locale()? };
    // SAFETY: `loc` is a live locale object; uselocale makes it this
    // thread's and returns the one it replaces (null: nothing changed),
    // which the second call puts back before anything else here reads it.
    unsafe {
        let old = libc::uselocale(loc);
        if old.is_null() {
            return None;
        }
        let n = __ctype_get_mb_cur_max();
        libc::uselocale(old);
        Some(n)
    }
}

/// `wcstod(s, &end)` in a UTF-8 locale (white space as `iswspace` reads it
/// there; `.` the decimal point) of the wide characters `s` (a NUL is
/// appended): the number and how many of `s` it read. `None` when the C
/// library has no C.UTF-8 locale: the result never depends on the
/// process's locale.
pub fn wcstod_of(s: &[u32]) -> Option<(f64, usize)> {
    let loc = c_utf8()?;
    let mut buf: Vec<libc::wchar_t> = Vec::with_capacity(s.len() + 1);
    buf.extend(s.iter().map(|&c| c as libc::wchar_t));
    buf.push(0);
    let mut end: *mut libc::wchar_t = std::ptr::null_mut();
    // SAFETY: a NUL-terminated wide string, alive for the call; `end` is set
    // to a position inside it; `loc` is a live locale object.
    let v = unsafe { wcstod_l(buf.as_ptr(), &mut end, loc) };
    // SAFETY: `end` points into `buf` (at its start when nothing was read).
    let used = unsafe { end.offset_from(buf.as_ptr()) } as usize;
    Some((v, used.min(s.len())))
}

/// The home directory in the password database of `user` (`getpwnam`), or
/// of the process's user (`getpwuid(getuid())`) for `None`; `None` when
/// there is no such entry. (The reentrant lookups, with the buffer grown
/// while they say `ERANGE`, as glibc's `getpwnam` does.)
pub fn passwd_home(user: Option<&[u8]>) -> Option<Vec<u8>> {
    let name = match user {
        Some(u) => Some(std::ffi::CString::new(u).ok()?),
        None => None,
    };
    let mut buf = vec![0 as libc::c_char; 16384];
    loop {
        // SAFETY: a zeroed passwd is a valid out-argument; the reentrant
        // lookups write it and `result`, whose strings point into `buf`,
        // alive for the reads below; `name` is NUL-terminated.
        unsafe {
            let mut pw: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = match &name {
                Some(n) => libc::getpwnam_r(n.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut result),
                None => libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut result),
            };
            if rc == libc::ERANGE {
                // the entry did not fit
                let n = buf.len().checked_mul(2)?;
                buf.resize(n, 0);
                continue;
            }
            if rc != 0 || result.is_null() || pw.pw_dir.is_null() {
                return None;
            }
            return Some(std::ffi::CStr::from_ptr(pw.pw_dir).to_bytes().to_vec());
        }
    }
}

unsafe extern "C" {
    fn bindtextdomain(domainname: *const libc::c_char, dirname: *const libc::c_char) -> *mut libc::c_char;
}

/// The C library's default directory of message catalogues
/// (`_nl_default_dirname`): what `bindtextdomain(domain, NULL)` gives for a
/// domain nothing has bound (glibc: `LOCALEDIR`, e.g. "/usr/share/locale");
/// `None` if the C library answers NULL. Only asked, never set: robigo
/// keeps its own bindings per isolate.
pub fn default_textdomain_dir() -> Option<Vec<u8>> {
    // a domain name no one else can have bound: the process's id and this
    // function's address
    let name = std::ffi::CString::new(format!(
        "robigo-unbound-{}-{:x}",
        std::process::id(),
        default_textdomain_dir as fn() -> Option<Vec<u8>> as usize
    ))
    .unwrap();
    // SAFETY: with a NULL dirname, bindtextdomain only reads its table and
    // returns a string it owns (the default directory, a constant of the
    // library, for a domain nothing bound), copied before anything else
    // could change it; the domain is NUL-terminated.
    unsafe {
        let p = bindtextdomain(name.as_ptr(), std::ptr::null());
        (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::wcstod_of;

    // (the C library's message catalogue table is not Miri's)
    #[test]
    #[cfg_attr(miri, ignore)]
    fn the_default_message_directory_is_an_absolute_path() {
        let d = super::default_textdomain_dir().expect("libintl answers for an unbound domain");
        assert!(d.starts_with(b"/"), "{}", String::from_utf8_lossy(&d));
    }

    fn w(s: &str) -> Vec<u32> {
        s.chars().map(|c| c as u32).collect()
    }

    // (the C library's locale objects are not Miri's)
    #[test]
    #[cfg_attr(miri, ignore)]
    fn mb_cur_max_is_the_c_librarys() {
        // SAFETY: a null argument only queries the thread's locale.
        let before = unsafe { libc::uselocale(std::ptr::null_mut()) };
        // glibc's (the oracle image's): 6 in C.UTF-8, 1 in C
        if cfg!(target_env = "gnu") {
            assert_eq!(super::mb_cur_max(true), Some(6));
        }
        assert_eq!(super::mb_cur_max(false), Some(1));
        // the thread's locale is as it was
        // SAFETY: as above.
        assert_eq!(unsafe { libc::uselocale(std::ptr::null_mut()) }, before);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn wcstod_reads_c_utf8_numbers_whatever_the_process_locale() {
        // '.' is the decimal point, ',' ends the number
        assert_eq!(wcstod_of(&w("1.5x")), Some((1.5, 3)));
        assert_eq!(wcstod_of(&w("1,5")), Some((1.0, 1)));
        // C.UTF-8's white space (an em space) is skipped
        assert_eq!(wcstod_of(&w("\u{2003}2.5!")), Some((2.5, 4)));
        assert_eq!(wcstod_of(&w("x")), Some((0.0, 0)));
        // a ',' decimal locale set for the process changes nothing
        // SAFETY: the test thread sets LC_NUMERIC and restores "C"; no
        // other test reads the process locale.
        unsafe {
            let de = libc::setlocale(libc::LC_NUMERIC, c"de_DE.UTF-8".as_ptr());
            assert_eq!(wcstod_of(&w("1.5")), Some((1.5, 3)));
            if !de.is_null() {
                libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
            }
        }
    }
}
