//! The C library behind safe functions, for builtins that GNU R implements
//! with it (robigo-rtbuiltins forbids `unsafe`): the clock and broken-down
//! time (`time`, `localtime_r`, `gmtime_r`, which datetime.c calls on Linux),
//! `wcstod` (Rstrptime.h's `%OS`), the password database's home
//! directories (sys-unix.c's `R_ExpandFileName_unix`), `uname`, the login
//! and user names (sys-unix.c's `Sys.info`), `glob` (sysutils.c's
//! `Sys.glob`), libintl's default
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
//!
//! The one exception is the process environment (`setenv`, `unsetenv`:
//! sysutils.c's `Sys.setenv`, `Sys.unsetenv`), which is the process's, not an
//! isolate's, in robigo as in GNU R. See [`EnvWriter::set`] for how its readers
//! on other threads are kept safe ([`ENV`], and glibc 2.41 or later,
//! [`MIN_GLIBC`], or a single isolate).

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
    // localtime_r reads TZ through getenv
    let _env = env_read();
    // SAFETY: a zeroed tm is a valid out-argument; localtime_r writes it and
    // reads only the time zone the environment names (no write meanwhile:
    // ENV held shared).
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() { None } else { Some(Tm::from_c(&tm)) }
    }
}

/// `gmtime_r(&t, &tm)`: `None` when the C library fails.
pub fn gmtime(t: i64) -> Option<Tm> {
    // gmtime_r runs glibc's tzset code (__tz_convert), which reads TZ the
    // first time
    let _env = env_read();
    // SAFETY: as for localtime_r.
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
        // newlocale reads LOCPATH through getenv
        let _env = env_read();
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
    let l = *LOC.get_or_init(|| {
        // (newlocale reads LOCPATH through getenv)
        let _env = env_read();
        // SAFETY: as in c_utf8.
        let l = unsafe { libc::newlocale(libc::LC_CTYPE_MASK, c"C".as_ptr(), std::ptr::null_mut()) };
        l as usize
    });
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
    let _this = ThreadLocale::switch(loc)?;
    // SAFETY: no arguments; it reads the calling thread's locale, `loc`
    // until `_this` drops.
    Some(unsafe { __ctype_get_mb_cur_max() })
}

/// The calling thread's locale switched to a locale object (`uselocale`)
/// until the guard drops, which puts back the one it replaced.
struct ThreadLocale(libc::locale_t);

impl ThreadLocale {
    /// `None` when the C library refuses the switch (nothing changed).
    fn switch(loc: libc::locale_t) -> Option<ThreadLocale> {
        // SAFETY: `loc` is a live locale object (made once, never freed);
        // uselocale returns the thread's locale it replaces, null on error.
        let old = unsafe { libc::uselocale(loc) };
        (!old.is_null()).then_some(ThreadLocale(old))
    }
}

impl Drop for ThreadLocale {
    fn drop(&mut self) {
        // SAFETY: the locale uselocale returned, live while it is any
        // thread's (LC_GLOBAL_LOCALE or one of this module's).
        unsafe { libc::uselocale(self.0) };
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
    // (the name service's modules may read the environment)
    let _env = env_read();
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

/// The user name of `uid` in the password database (`getpwuid(uid)->
/// pw_name`), `None` when there is no such entry (the reentrant lookup,
/// its buffer grown on `ERANGE`).
pub fn passwd_name(uid: u32) -> Option<Vec<u8>> {
    let mut buf = vec![0 as libc::c_char; 16384];
    // (the name service's modules may read the environment)
    let _env = env_read();
    loop {
        // SAFETY: a zeroed passwd is a valid out-argument; getpwuid_r writes
        // it and `result`, whose strings point into `buf`, alive for the
        // read below.
        unsafe {
            let mut pw: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut result);
            if rc == libc::ERANGE {
                let n = buf.len().checked_mul(2)?;
                buf.resize(n, 0);
                continue;
            }
            if rc != 0 || result.is_null() || pw.pw_name.is_null() {
                return None;
            }
            return Some(std::ffi::CStr::from_ptr(pw.pw_name).to_bytes().to_vec());
        }
    }
}

/// The group name of `gid` in the group database (`getgrgid(gid)->
/// gr_name`), `None` when there is no such entry (the reentrant lookup,
/// its buffer grown on `ERANGE`).
pub fn group_name(gid: u32) -> Option<Vec<u8>> {
    let mut buf = vec![0 as libc::c_char; 16384];
    // (the name service's modules may read the environment)
    let _env = env_read();
    loop {
        // SAFETY: a zeroed group is a valid out-argument; getgrgid_r writes
        // it and `result`, whose strings point into `buf`, alive for the
        // read below.
        unsafe {
            let mut gr: libc::group = std::mem::zeroed();
            let mut result: *mut libc::group = std::ptr::null_mut();
            let rc = libc::getgrgid_r(gid, &mut gr, buf.as_mut_ptr(), buf.len(), &mut result);
            if rc == libc::ERANGE {
                let n = buf.len().checked_mul(2)?;
                buf.resize(n, 0);
                continue;
            }
            if rc != 0 || result.is_null() || gr.gr_name.is_null() {
                return None;
            }
            return Some(std::ffi::CStr::from_ptr(gr.gr_name).to_bytes().to_vec());
        }
    }
}

/// `uname(&name)`'s `sysname`, `release`, `version`, `nodename` and
/// `machine` (sys-unix.c's `do_sysinfo` order), `None` when it fails.
pub fn uname() -> Option<[Vec<u8>; 5]> {
    // SAFETY: a zeroed utsname is a valid out-argument; uname writes
    // NUL-terminated strings into its arrays.
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) == -1 {
            return None;
        }
        let s = |f: &[libc::c_char]| std::ffi::CStr::from_ptr(f.as_ptr()).to_bytes().to_vec();
        Some([s(&u.sysname), s(&u.release), s(&u.version), s(&u.nodename), s(&u.machine)])
    }
}

/// The name of the user logged in on the process's controlling terminal
/// (`getlogin()`, here its reentrant `getlogin_r`, which glibc's `getlogin`
/// calls), `None` when there is none.
pub fn login_name() -> Option<Vec<u8>> {
    let mut buf = vec![0 as libc::c_char; 256];
    // (glibc looks the login uid up in the password database, whose name
    // service modules may read the environment)
    let _env = env_read();
    loop {
        // SAFETY: getlogin_r writes at most `buf.len()` bytes, NUL included,
        // into `buf`.
        let rc = unsafe { getlogin_r(buf.as_mut_ptr(), buf.len()) };
        if rc == libc::ERANGE {
            let n = buf.len().checked_mul(2)?;
            buf.resize(n, 0);
            continue;
        }
        if rc != 0 {
            return None;
        }
        // SAFETY: on success `buf` holds a NUL-terminated name.
        return Some(unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_bytes().to_vec());
    }
}

// (POSIX's, in glibc's libc; the libc crate does not declare it)
unsafe extern "C" {
    fn getlogin_r(name: *mut libc::c_char, size: libc::size_t) -> libc::c_int;
}

/// The process's real user id (`getuid()`).
pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// The process's effective user id (`geteuid()`).
pub fn euid() -> u32 {
    // SAFETY: as for getuid.
    unsafe { libc::geteuid() }
}

/// What `glob(3)` returned for one pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobRes {
    /// 0, or `GLOB_NOMATCH`
    Ok,
    /// `GLOB_ABORTED`: a read error
    Aborted,
    /// `GLOB_NOSPACE`: out of memory
    NoSpace,
}

/// A `glob_t` that patterns are matched into, one `glob(3)` call each
/// (`GLOB_APPEND` after the first), as sysutils.c's `do_glob` does; freed
/// (`globfree`) on drop. No flag here expands `~` (`GLOB_TILDE`), so glob
/// reads directories, not the environment or the password database.
pub struct Glob {
    buf: libc::glob_t,
    initialized: bool,
}

impl Default for Glob {
    fn default() -> Self {
        Self::new()
    }
}

impl Glob {
    pub fn new() -> Glob {
        // SAFETY: a zeroed glob_t (null gl_pathv, counts 0) is the value
        // glob's first call (no GLOB_APPEND) overwrites.
        Glob { buf: unsafe { std::mem::zeroed() }, initialized: false }
    }

    /// Match `pattern` (`GLOB_MARK`: a `/` after each directory, for
    /// `mark`), appending to the earlier matches.
    pub fn append(&mut self, pattern: &[u8], mark: bool) -> GlobRes {
        // a pattern with a NUL in it is C's pattern up to the NUL
        let end = pattern.iter().position(|&b| b == 0).unwrap_or(pattern.len());
        let pat = std::ffi::CString::new(&pattern[..end]).expect("no NUL before end");
        let flags = if mark { libc::GLOB_MARK } else { 0 } | if self.initialized { libc::GLOB_APPEND } else { 0 };
        // SAFETY: `pat` is NUL-terminated; `buf` is zeroed or what earlier
        // calls left (GLOB_APPEND then); no error callback.
        let res = unsafe { libc::glob(pat.as_ptr(), flags, None, &mut self.buf) };
        self.initialized = true;
        match res {
            libc::GLOB_ABORTED => GlobRes::Aborted,
            libc::GLOB_NOSPACE => GlobRes::NoSpace,
            _ => GlobRes::Ok,
        }
    }

    /// The matches so far, in glob's order (`gl_pathv`).
    pub fn paths(&self) -> Vec<Vec<u8>> {
        if !self.initialized || self.buf.gl_pathv.is_null() {
            return Vec::new();
        }
        (0..self.buf.gl_pathc)
            .map(|i| {
                // SAFETY: gl_pathv holds gl_pathc NUL-terminated strings
                // (after gl_offs, 0 here), alive until globfree.
                unsafe { std::ffi::CStr::from_ptr(*self.buf.gl_pathv.add(i)) }.to_bytes().to_vec()
            })
            .collect()
    }
}

impl Drop for Glob {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: `buf` is what glob filled in.
            unsafe { libc::globfree(&mut self.buf) };
        }
    }
}

/// The process's file mode creation mask, read as GNU R reads it: `um =
/// umask(0); umask(um)` (a process global: another thread creating a file
/// in between is created with no mask, as in GNU R).
pub fn umask_get() -> u32 {
    // SAFETY: umask has no preconditions.
    let um = unsafe { libc::umask(0) };
    // SAFETY: as above.
    unsafe { libc::umask(um) };
    um as u32
}

// Like the rest of this module (glibc's `bindtextdomain`,
// `__ctype_get_mb_cur_max`), the environment writer below is for Linux
// with glibc: its soundness rests on glibc's `getenv` (MIN_GLIBC), so
// another C library must not build it silently.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
compile_error!(
    "robigo-rtcore's clib is for Linux with glibc (the environment writer relies on glibc's getenv); a port to another C library starts here"
);

/// The process environment's lock: every wrapper here whose C call reads
/// the environment holds it shared for that call ([`env_read`]:
/// `localtime`, `gmtime`, the `C.UTF-8` locale's `newlocale`,
/// `passwd_home`, `passwd_name`, `group_name`, `login_name`, `env_locale`, `c_locale`'s
/// `newlocale`, `default_textdomain_dir`),
/// and an [`EnvWriter`] holds it
/// exclusive. So robigo's own safe wrappers never read while it writes, on
/// any glibc, with or without isolates. Lock order: the live isolates
/// (`isolate::LIVE_ISOLATES`), then this, then std's environment lock.
static ENV: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// [`ENV`] shared, for a C call that reads the environment. (Never while
/// this thread holds an [`EnvWriter`]: the lock does not nest.)
fn env_read() -> std::sync::RwLockReadGuard<'static, ()> {
    ENV.read().unwrap_or_else(|e| e.into_inner())
}

/// The oldest glibc whose `getenv` is safe against a concurrent `setenv`
/// or `unsetenv` on another thread: 2.41, the first release with commit
/// 7a61e7f557a9 ("stdlib: Make getenv thread-safe in more cases",
/// `X-Git-Tag: glibc-2.41~465`), which never frees an environment array
/// that `getenv` may be walking, publishes each new one with a release
/// store, and retries a `getenv` that raced a change.
pub const MIN_GLIBC: (u32, u32) = (2, 41);

/// A glibc version string's major and minor numbers: the leading digits of
/// its first two components ("2.41-vendor" and "2.41.9000" are 2.41);
/// what is not a number is 0, so a string that is not a version is
/// (0, 0), which only ever refuses.
fn parse_glibc_version(v: &[u8]) -> (u32, u32) {
    let mut parts = v.split(|&b| b == b'.').map(|p| {
        p.iter()
            .take_while(|b| b.is_ascii_digit())
            .fold(0u32, |n, &d| n.saturating_mul(10).saturating_add((d - b'0') as u32))
    });
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// The running C library's version (`gnu_get_libc_version`), read once.
fn glibc_version() -> (u32, u32) {
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const libc::c_char;
    }
    static V: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    // SAFETY: glibc returns a static NUL-terminated string.
    *V.get_or_init(|| parse_glibc_version(unsafe { std::ffi::CStr::from_ptr(gnu_get_libc_version()) }.to_bytes()))
}

/// The embedder's grant of the process environment's writes: what lets an
/// isolate's `Sys.setenv`/`Sys.unsetenv` write at all
/// ([`crate::OwnedIsolate::grant_env_writes`]). A safe function cannot
/// know what every thread of the process reads, so the grant is the
/// embedder's `unsafe` statement of it, made once ([`EnvWrites::assume`]).
#[derive(Debug)]
pub struct EnvWrites(());

impl EnvWrites {
    /// The grant.
    ///
    /// # Safety
    ///
    /// For as long as an isolate holds it, no read of the environment can
    /// overlap one of its writes unsynchronized. The writes happen on the
    /// isolate's own thread ([`EnvWriter`] is neither `Send` nor `Sync`),
    /// so:
    /// - a read on that thread is sequential with the writes and allowed,
    ///   whatever reads it (C code included);
    /// - a read on any other thread goes through `std::env`, through this
    ///   module's guarded wrappers, or through a C library whose `getenv`
    ///   tolerates a concurrent `setenv` (glibc [`MIN_GLIBC`] or later);
    /// - nothing walks `environ` by hand on another thread meanwhile.
    ///
    /// On a glibc older than [`MIN_GLIBC`] the embedder also assumes that
    /// no other isolate runs native code while the holder writes, and that
    /// no thread started by loaded native code reads the environment while
    /// the isolate may write. (GNU R's rule that R's API belongs to the one
    /// R thread does not cover such threads: a worker can call `getenv`
    /// without touching R's API. This is an assumption, not a consequence.)
    pub unsafe fn assume() -> EnvWrites {
        EnvWrites(())
    }
}

/// Whether the process environment may be written now, the second check
/// [`env_writer`] makes after the grant: always on a glibc `glibc` of at
/// least [`MIN_GLIBC`]; on an older one only while at most one isolate
/// lives.
pub fn env_write_allowed(glibc: (u32, u32), live_isolates: usize) -> bool {
    glibc >= MIN_GLIBC || live_isolates <= 1
}

/// Why the environment was not written.
#[derive(Debug, PartialEq, Eq)]
pub enum EnvWriteRefused {
    /// This isolate was not given the process environment
    /// ([`EnvWrites`]).
    NotGranted,
    /// [`env_write_allowed`] refused: a glibc older than [`MIN_GLIBC`] and
    /// several isolates.
    SeveralIsolates,
}

/// The permission to write the process environment, for one whole builtin
/// call (`Sys.setenv`, `Sys.unsetenv`) of the isolate `cx` names: while it
/// lives, this thread holds the live-isolate lock (no isolate starts or
/// ends) and [`ENV`] exclusive (no wrapper here reads). The calls made
/// while holding it must not start an isolate or reach a wrapper that
/// takes [`ENV`] (the lock does not nest): only translation and its own
/// methods.
pub struct EnvWriter<'a> {
    // dropped in this order: ENV, then the live isolates (the reverse of
    // taking them)
    _env: std::sync::RwLockWriteGuard<'static, ()>,
    _live: std::sync::MutexGuard<'static, usize>,
    // the isolate whose thread this is, which also makes the writer
    // neither Send nor Sync (`Cx` is neither): the writes stay on that
    // thread, which EnvWrites::assume's contract counts on
    // (tests/ui/env_writer_not_sync.rs)
    _cx: &'a crate::Cx,
}

/// An [`EnvWriter`] for the isolate `cx` (the caller is on its thread:
/// `cx` is its capability), or why not: the isolate holds no
/// [`EnvWrites`], or [`env_write_allowed`] says no for the running glibc
/// and the live isolates.
pub fn env_writer(cx: &crate::Cx) -> Result<EnvWriter<'_>, EnvWriteRefused> {
    cx.env_writes().ok_or(EnvWriteRefused::NotGranted)?;
    let live = crate::isolate::live_isolates();
    if !env_write_allowed(glibc_version(), *live) {
        return Err(EnvWriteRefused::SeveralIsolates);
    }
    let env = ENV.write().unwrap_or_else(|e| e.into_inner());
    Ok(EnvWriter { _env: env, _live: live, _cx: cx })
}

/// Whether `name` is one the C library's `setenv`/`unsetenv` accept: not
/// empty and without `=` (else `EINVAL`), and, as a C string, without NUL
/// (an R string has none).
fn env_name_ok(name: &[u8]) -> bool {
    !name.is_empty() && !name.contains(&b'=') && !name.contains(&0)
}

impl EnvWriter<'_> {
    /// `setenv(name, value, 1) == 0`: set the process's environment
    /// variable `name` to `value`; false for a name `setenv` refuses
    /// (`EINVAL`).
    ///
    /// The environment is process-wide: every isolate (each on its own
    /// thread) sees the change, as every part of a GNU R process does. Its
    /// readers are the embedder's to keep safe, as [`EnvWrites::assume`]
    /// states; this writer adds what robigo itself can: std's readers take
    /// std's lock, as `set_var` does; this module's C readers hold [`ENV`]
    /// shared, which the writer holds exclusive; and on a glibc older than
    /// [`MIN_GLIBC`] no other isolate lives or starts while it writes.
    pub fn set(&self, name: &[u8], value: &[u8]) -> bool {
        use std::os::unix::ffi::OsStrExt;
        if !env_name_ok(name) || value.contains(&0) {
            return false;
        }
        // SAFETY: the name and value are ones `set_var` accepts (checked
        // above). No thread reads the environment unsynchronized: the
        // embedder's grant (`self._grant`, `EnvWrites::assume`'s contract)
        // covers every reader of the process; robigo's own are std's (std's
        // lock) and this module's (ENV, held exclusive by `self`).
        unsafe { std::env::set_var(std::ffi::OsStr::from_bytes(name), std::ffi::OsStr::from_bytes(value)) };
        true
    }

    /// `unsetenv(name)`: remove the process's environment variable `name`;
    /// a name `unsetenv` refuses (`EINVAL`) changes nothing. Readers as for
    /// [`EnvWriter::set`].
    pub fn unset(&self, name: &[u8]) {
        use std::os::unix::ffi::OsStrExt;
        if !env_name_ok(name) {
            return;
        }
        // SAFETY: as for `set`: a name `remove_var` accepts; readers by the
        // grant's contract and the locks `self` holds.
        unsafe { std::env::remove_var(std::ffi::OsStr::from_bytes(name)) };
    }

    /// `getenv(name) != NULL`: whether the environment has `name`, read
    /// while this writer holds it still.
    pub fn has(&self, name: &[u8]) -> bool {
        use std::os::unix::ffi::OsStrExt;
        !name.contains(&0) && std::env::var_os(std::ffi::OsStr::from_bytes(name)).is_some()
    }
}

/// What `setlocale(category, "")` would set at GNU R's startup, asked of
/// the C library without setting it (robigo-rt's locale is the isolate's,
/// [`crate::Locale`]; the process's stays "C").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvLocale {
    /// `nl_langinfo(CODESET)` of the environment's `LC_CTYPE`, or of "C"
    /// when the C library has no such locale ("ANSI_X3.4-1968" in glibc).
    pub codeset: Vec<u8>,
    /// The environment's `LC_COLLATE` is "C" or "POSIX" (or the C library
    /// has no such locale, so "C" is used).
    pub collate_c: bool,
    /// The categories whose locale the C library does not have, in GNU R's
    /// order (main.c's `setlocale` calls): `LC_CTYPE`, `LC_COLLATE`, ...
    pub failed: Vec<&'static str>,
    /// The names `setlocale(category, NULL)` then gives.
    pub names: LocaleNames,
}

/// The C library's locale category numbers (`LocaleNames` takes them).
pub mod category {
    pub use libc::{
        LC_ALL, LC_COLLATE, LC_CTYPE, LC_MEASUREMENT, LC_MESSAGES, LC_MONETARY, LC_NUMERIC, LC_PAPER, LC_TIME, c_int,
    };
}

/// The C library's category numbers (glibc's `__LC_LAST` of them, `LC_ALL`
/// among them) and names, in its order.
const CATEGORIES: [(libc::c_int, &str); 13] = [
    (libc::LC_CTYPE, "LC_CTYPE"),
    (libc::LC_NUMERIC, "LC_NUMERIC"),
    (libc::LC_TIME, "LC_TIME"),
    (libc::LC_COLLATE, "LC_COLLATE"),
    (libc::LC_MONETARY, "LC_MONETARY"),
    (libc::LC_MESSAGES, "LC_MESSAGES"),
    (libc::LC_ALL, "LC_ALL"),
    (libc::LC_PAPER, "LC_PAPER"),
    (libc::LC_NAME, "LC_NAME"),
    (libc::LC_ADDRESS, "LC_ADDRESS"),
    (libc::LC_TELEPHONE, "LC_TELEPHONE"),
    (libc::LC_MEASUREMENT, "LC_MEASUREMENT"),
    (libc::LC_IDENTIFICATION, "LC_IDENTIFICATION"),
];

/// The locale's name per category, as the C library's `setlocale` keeps
/// them (`_nl_global_locale.__names`): what `setlocale(category, NULL)`
/// answers. robigo-rt never sets the process's locale; each isolate keeps
/// the names its R session would have set, and asks the C library to
/// resolve a new one ([`LocaleNames::set`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleNames {
    /// By the C library's category number (the `LC_ALL` slot unused).
    names: Vec<Vec<u8>>,
}

impl Default for LocaleNames {
    /// GNU R's on the oracle (`LANG=C.UTF-8`): `C.UTF-8` for the
    /// categories its startup sets, `C` for the rest.
    fn default() -> LocaleNames {
        let mut n = LocaleNames::c();
        for cat in STARTUP_CATEGORIES {
            n.names[cat.1 as usize] = b"C.UTF-8".to_vec();
        }
        n
    }
}

/// The categories GNU R's startup sets from the environment, in its
/// order (main.c's `setlocale(<category>, "")` calls).
const STARTUP_CATEGORIES: [(&str, libc::c_int, libc::c_int); 7] = [
    ("LC_CTYPE", libc::LC_CTYPE, libc::LC_CTYPE_MASK),
    ("LC_COLLATE", libc::LC_COLLATE, libc::LC_COLLATE_MASK),
    ("LC_TIME", libc::LC_TIME, libc::LC_TIME_MASK),
    ("LC_MESSAGES", libc::LC_MESSAGES, libc::LC_MESSAGES_MASK),
    ("LC_MONETARY", libc::LC_MONETARY, libc::LC_MONETARY_MASK),
    ("LC_PAPER", libc::LC_PAPER, libc::LC_PAPER_MASK),
    ("LC_MEASUREMENT", libc::LC_MEASUREMENT, libc::LC_MEASUREMENT_MASK),
];

/// glibc's `_NL_LOCALE_NAME(category)`: the `nl_langinfo` item of a
/// locale object's name for the category.
fn nl_locale_name(cat: libc::c_int) -> libc::nl_item {
    (cat << 16) | 0xffff
}

impl LocaleNames {
    /// Every category "C" (a process's names before any `setlocale`).
    pub fn c() -> LocaleNames {
        LocaleNames { names: vec![b"C".to_vec(); CATEGORIES.len()] }
    }

    /// `setlocale(cat, NULL)`: the category's name; for `LC_ALL` the one
    /// name when every category has it, else glibc's composite
    /// `LC_CTYPE=...;LC_NUMERIC=...;...`.
    // (glibc: setlocale.c:new_composite_name)
    pub fn get(&self, cat: libc::c_int) -> Vec<u8> {
        assert!(CATEGORIES.iter().any(|c| c.0 == cat), "robigo: locale category {cat} is not the C library's");
        if cat != libc::LC_ALL {
            return self.names[cat as usize].clone();
        }
        let first = &self.names[libc::LC_CTYPE as usize];
        let same = CATEGORIES.iter().filter(|c| c.0 != libc::LC_ALL).all(|c| self.names[c.0 as usize] == *first);
        if same {
            return if first == b"POSIX" { b"C".to_vec() } else { first.clone() };
        }
        let mut s = Vec::new();
        for (c, name) in CATEGORIES.iter().filter(|c| c.0 != libc::LC_ALL) {
            s.extend_from_slice(name.as_bytes());
            s.push(b'=');
            s.extend_from_slice(&self.names[*c as usize]);
            s.push(b';');
        }
        s.pop();
        s
    }

    /// `setlocale(cat, name)` for one category (not `LC_ALL`): the name the
    /// C library resolves `name` to (the environment's for "", "C" for
    /// "POSIX"), set and returned; `None`, nothing set, when it has no such
    /// locale.
    pub fn set(&mut self, cat: libc::c_int, name: &[u8]) -> Option<Vec<u8>> {
        let new = resolve(cat, name)?;
        self.names[cat as usize] = new.clone();
        Some(new)
    }
}

/// The name a `newlocale` of the category for `name` has (`None` when the
/// C library has no such locale).
fn resolve(cat: libc::c_int, name: &[u8]) -> Option<Vec<u8>> {
    // one category (the mask is its bit, glibc's LC_*_MASK)
    assert!(
        cat != libc::LC_ALL && CATEGORIES.iter().any(|c| c.0 == cat),
        "robigo: locale category {cat} is not one of the C library's"
    );
    let name = std::ffi::CString::new(name).ok()?;
    // newlocale(mask, "") reads the environment
    let _env = env_read();
    // SAFETY: newlocale makes a new object from a NUL-terminated name and no
    // base; nl_langinfo_l reads its name, copied before the object is freed.
    unsafe {
        let l = libc::newlocale(1 << cat, name.as_ptr(), std::ptr::null_mut());
        if l.is_null() {
            return None;
        }
        let p = libc::nl_langinfo_l(nl_locale_name(cat), l);
        let r = (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_bytes().to_vec());
        libc::freelocale(l);
        r
    }
}

/// [`EnvLocale`] of the process's environment.
// R: main.c:setup_Rmainloop (the setlocale calls), platform.c:R_check_locale
pub fn env_locale() -> EnvLocale {
    let mut failed = Vec::new();
    let mut codeset = None;
    let mut names = LocaleNames::c();
    // newlocale(mask, "") reads LC_ALL, LC_* and LANG through getenv
    let _env = env_read();
    for (name, cat, mask) in STARTUP_CATEGORIES {
        // SAFETY: newlocale makes a new object from a NUL-terminated name
        // (the environment's, "") and no base; nl_langinfo_l reads its
        // codeset and name, each string copied before the object is freed,
        // once.
        unsafe {
            let l = libc::newlocale(mask, c"".as_ptr(), std::ptr::null_mut());
            if l.is_null() {
                failed.push(name);
                continue;
            }
            if mask == libc::LC_CTYPE_MASK {
                let p = libc::nl_langinfo_l(libc::CODESET, l);
                if !p.is_null() {
                    codeset = Some(std::ffi::CStr::from_ptr(p).to_bytes().to_vec());
                }
            }
            let p = libc::nl_langinfo_l(nl_locale_name(cat), l);
            if !p.is_null() {
                names.names[cat as usize] = std::ffi::CStr::from_ptr(p).to_bytes().to_vec();
            }
            libc::freelocale(l);
        }
    }
    let codeset = codeset.unwrap_or_else(|| {
        // SAFETY: "C" always exists; read and freed as above.
        unsafe {
            let l = libc::newlocale(libc::LC_CTYPE_MASK, c"C".as_ptr(), std::ptr::null_mut());
            if l.is_null() {
                b"ANSI_X3.4-1968".to_vec()
            } else {
                let p = libc::nl_langinfo_l(libc::CODESET, l);
                let v = if p.is_null() {
                    b"ANSI_X3.4-1968".to_vec()
                } else {
                    std::ffi::CStr::from_ptr(p).to_bytes().to_vec()
                };
                libc::freelocale(l);
                v
            }
        }
    });
    // setlocale(LC_COLLATE, "")'s name: LC_ALL, else LC_COLLATE, else LANG
    // (the first set and not empty), else "C"
    let get = |n: &str| std::env::var_os(n).filter(|v| !v.is_empty());
    let collate = get("LC_ALL").or_else(|| get("LC_COLLATE")).or_else(|| get("LANG"));
    let collate_c =
        failed.contains(&"LC_COLLATE") || collate.is_none_or(|v| matches!(v.as_encoded_bytes(), b"C" | b"POSIX"));
    EnvLocale { codeset, collate_c, failed, names }
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
    // (libintl may read the environment)
    let _env = env_read();
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

    // (the C library's locale objects are not Miri's)
    #[test]
    #[cfg_attr(miri, ignore)]
    fn locale_names_are_glibc_s() {
        use super::{LocaleNames, category as lc};
        let mut n = LocaleNames::c();
        assert_eq!(n.get(lc::LC_ALL), b"C");
        // POSIX is C, and every category the same is the one name
        assert_eq!(n.set(lc::LC_TIME, b"POSIX"), Some(b"C".to_vec()));
        assert_eq!(n.get(lc::LC_ALL), b"C");
        assert_eq!(n.set(lc::LC_TIME, b"xx_NOWHERE"), None);
        assert_eq!(n.get(lc::LC_TIME), b"C");
        if n.set(lc::LC_TIME, b"C.UTF-8").is_some() {
            assert_eq!(
                n.get(lc::LC_ALL),
                b"LC_CTYPE=C;LC_NUMERIC=C;LC_TIME=C.UTF-8;LC_COLLATE=C;LC_MONETARY=C;LC_MESSAGES=C;\
                  LC_PAPER=C;LC_NAME=C;LC_ADDRESS=C;LC_TELEPHONE=C;LC_MEASUREMENT=C;LC_IDENTIFICATION=C"
            );
        }
    }

    // the rule setenv/unsetenv keep, on both sides of the glibc floor
    // and with one isolate or several
    #[test]
    fn the_environment_is_written_on_a_new_glibc_or_with_one_isolate() {
        use super::{MIN_GLIBC, env_write_allowed};
        for glibc in [MIN_GLIBC, (MIN_GLIBC.0, MIN_GLIBC.1 + 3), (3, 0)] {
            assert!(env_write_allowed(glibc, 0) && env_write_allowed(glibc, 1) && env_write_allowed(glibc, 2));
        }
        for glibc in [(MIN_GLIBC.0, MIN_GLIBC.1 - 1), (2, 39), (1, 99)] {
            assert!(env_write_allowed(glibc, 0) && env_write_allowed(glibc, 1));
            assert!(!env_write_allowed(glibc, 2) && !env_write_allowed(glibc, 8));
        }
    }

    // an isolate the embedder gave no grant cannot write
    #[test]
    #[cfg_attr(miri, ignore)]
    fn an_isolate_without_the_grant_cannot_write() {
        let mut iso = crate::OwnedIsolate::new();
        assert_eq!(super::env_writer(iso.cx()).err(), Some(super::EnvWriteRefused::NotGranted));
    }

    // while a writer lives the live-isolate lock is held, so no isolate
    // starts. (The probe sees only that the lock is held; other tests'
    // isolates hold it just for a count's update, so a free lock would be
    // this writer's failure.) On a glibc older than MIN_GLIBC with other
    // tests' isolates alive the writer is instead refused for that. (The C
    // library is not Miri's.)
    #[test]
    #[cfg_attr(miri, ignore)]
    fn a_writer_holds_the_isolate_registration() {
        let mut iso = crate::OwnedIsolate::new();
        // SAFETY: this test process reads the environment only through
        // std::env and clib, and the writer below writes nothing.
        iso.grant_env_writes(unsafe { super::EnvWrites::assume() });
        match super::env_writer(iso.cx()) {
            Ok(w) => {
                assert!(crate::isolate::live_isolates_try_lock().is_none(), "the registration lock is free");
                drop(w);
            }
            Err(e) => {
                assert_eq!(e, super::EnvWriteRefused::SeveralIsolates);
                assert!(super::glibc_version() < super::MIN_GLIBC);
            }
        }
    }

    // a granted writer sets, reads and unsets a variable
    #[test]
    #[cfg_attr(miri, ignore)]
    fn a_granted_writer_sets_and_unsets() {
        let mut iso = crate::OwnedIsolate::new();
        // SAFETY: this test process reads the environment only through
        // std::env and clib, and only this test uses the variable below.
        iso.grant_env_writes(unsafe { super::EnvWrites::assume() });
        let name = b"ROBIGO_CLIB_GRANTED_WRITER_TEST";
        match super::env_writer(iso.cx()) {
            Ok(w) => {
                assert!(!w.has(name));
                assert!(w.set(name, b"1"));
                assert!(w.has(name));
                assert!(!w.set(b"A=B", b"1") && !w.set(b"", b"1"));
                w.unset(name);
                assert!(!w.has(name));
            }
            // (an old glibc with other tests' isolates alive)
            Err(e) => assert_eq!(e, super::EnvWriteRefused::SeveralIsolates),
        }
    }

    #[test]
    fn a_glibc_version_is_its_leading_numbers() {
        use super::parse_glibc_version as p;
        assert_eq!(p(b"2.41"), (2, 41));
        assert_eq!(p(b"2.41-vendor"), (2, 41));
        assert_eq!(p(b"2.41.9000"), (2, 41));
        assert_eq!(p(b"2.39"), (2, 39));
        assert_eq!(p(b"garbage"), (0, 0));
        assert_eq!(p(b""), (0, 0));
        assert_eq!(p(b"x.y.z"), (0, 0));
    }

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
