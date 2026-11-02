//! The C library, behind safe functions.
//!
//! Builtins call what POSIX offers and `std` does not (or offers with other
//! semantics) through this module, so the unsafe code for it is here and only
//! here (`docs/architecture.md`, "Unsafe code"). Each wrapper owns its
//! contract: NUL-terminated arguments, buffers that outlive the call, and
//! copying results out of the C library's static storage before returning.
//!
//! The evaluator runs on one thread, so nothing here can race with itself on
//! that static storage. The only other thread robigo starts is the reader of
//! `system(timeout =)`, which calls none of this.

use std::ffi::{CStr, CString, OsStr, c_char, c_int, c_void};
use std::fs::File;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

/// A copy of a C string the C library owns.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string.
unsafe fn copy_cstr(p: *const c_char) -> Option<String> {
    // SAFETY: not null here; the caller promises termination.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn c_path(path: impl AsRef<OsStr>) -> Option<CString> {
    CString::new(path.as_ref().as_bytes()).ok()
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

// ----------------------------------------------------------------------
// Environment

/// `setenv`. `std::env::set_var` is unsafe because another thread could be
/// reading the environment.
pub fn set_env(name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    // SAFETY: called during startup or from the evaluator, the only thread
    // that reads the environment (see the module comment).
    unsafe { std::env::set_var(name, value) }
}

/// `unsetenv`. See [`set_env`].
pub fn remove_env(name: impl AsRef<OsStr>) {
    // SAFETY: as in `set_env`.
    unsafe { std::env::remove_var(name) }
}

/// C's `atoi`, which accepts trailing text.
pub fn atoi(text: &str) -> Option<i32> {
    let text = CString::new(text).ok()?;
    // SAFETY: a NUL-terminated string.
    Some(unsafe { libc::atoi(text.as_ptr()) })
}

// ----------------------------------------------------------------------
// Errors and formatting

/// `strerror(code)`.
pub fn strerror(code: i32) -> String {
    // SAFETY: strerror returns a NUL-terminated string, copied at once.
    unsafe { copy_cstr(libc::strerror(code)) }.unwrap_or_default()
}

/// `sprintf("%.16g\n", x)`, as `OutReal` writes ASCII serialization.
pub fn format_g16_line(x: f64) -> String {
    let mut buf = [0u8; 128];
    // SAFETY: `snprintf` is told the size of `buf`, and the one conversion
    // of the format takes a double. It always terminates what it writes.
    unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), c"%.16g\n".as_ptr(), x) };
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

// ----------------------------------------------------------------------
// Encodings and libraries

/// Whether `iconv` can convert `from` to `to`; R reports a failing
/// `iconv_open` as an unsupported conversion.
pub fn iconv_supported(to: &str, from: &str) -> bool {
    let (Ok(t), Ok(f)) = (CString::new(to), CString::new(from)) else { return false };
    // SAFETY: both are NUL-terminated for the call's duration.
    let cd = unsafe { libc::iconv_open(t.as_ptr(), f.as_ptr()) };
    if cd as isize == -1 {
        return false;
    }
    // SAFETY: `cd` is the descriptor iconv_open just returned.
    unsafe { libc::iconv_close(cd) };
    true
}

unsafe extern "C" {
    safe fn lzma_version_string() -> *const c_char;
    /// C99's `catan`.
    safe fn catan(z: CDouble) -> CDouble;
    /// libgcc's complex division, which C's `/` on `double complex` uses.
    safe fn __divdc3(a: f64, b: f64, c: f64, d: f64) -> CDouble;
}

/// liblzma's runtime version.
pub fn lzma_version() -> String {
    // SAFETY: liblzma returns a static NUL-terminated string.
    unsafe { copy_cstr(lzma_version_string()) }.unwrap_or_default()
}

/// glibc's runtime version.
#[cfg(target_env = "gnu")]
pub fn glibc_version() -> String {
    // SAFETY: glibc returns a static NUL-terminated string.
    unsafe { copy_cstr(libc::gnu_get_libc_version()) }.unwrap_or_default()
}

/// C's `double complex`, which the x86-64 ABI passes in two registers.
#[repr(C)]
#[derive(Clone, Copy)]
struct CDouble {
    re: f64,
    im: f64,
}

/// `catan(a / b)`, with the complex division and arctangent of the C
/// library (R: complex.c:z_atan2).
pub fn catan_of_quotient(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let q = __divdc3(a.0, a.1, b.0, b.1);
    let t = catan(q);
    (t.re, t.im)
}

// ----------------------------------------------------------------------
// Time and identity

/// `getrusage(who)`; `who` is `RUSAGE_SELF` or `RUSAGE_CHILDREN`.
pub fn rusage(who: c_int) -> libc::rusage {
    // SAFETY: rusage is plain integers, for which zero is valid, and
    // getrusage writes only that struct.
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(who, &mut usage);
        usage
    }
}

/// `ctime(time(NULL))`, newline included.
pub fn ctime_now() -> String {
    let mut buf = [0 as c_char; 64];
    // SAFETY: ctime_r writes at most 26 bytes into a buffer of 64 and NUL
    // terminates them, which `copy_cstr` reads. The null result on failure
    // leaves `buf` empty.
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        libc::ctime_r(&t, buf.as_mut_ptr());
        copy_cstr(buf.as_ptr()).unwrap_or_default()
    }
}

/// What `uname` reports.
pub struct Uname {
    pub sysname: String,
    pub release: String,
    pub version: String,
    pub nodename: String,
    pub machine: String,
}

/// `uname()`, or `None` on failure.
pub fn uname() -> Option<Uname> {
    // SAFETY: utsname is arrays of C chars, for which zero is valid.
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: uname writes only that struct.
    if unsafe { libc::uname(&mut u) } == -1 {
        return None;
    }
    let field = |p: &[c_char]| {
        // SAFETY: uname NUL-terminates each field, and the zeroed start
        // terminates any it did not fill.
        unsafe { copy_cstr(p.as_ptr()) }.unwrap_or_default()
    };
    Some(Uname {
        sysname: field(&u.sysname),
        release: field(&u.release),
        version: field(&u.version),
        nodename: field(&u.nodename),
        machine: field(&u.machine),
    })
}

/// `getlogin()`.
pub fn login_name() -> Option<String> {
    // SAFETY: null or a NUL-terminated static buffer, copied at once.
    unsafe { copy_cstr(libc::getlogin()) }
}

/// `getuid()`.
pub fn uid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::getuid() }
}

/// `geteuid()`.
pub fn euid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::geteuid() }
}

/// The name of user `uid` (`getpwuid`).
pub fn user_name(uid: u32) -> Option<String> {
    // SAFETY: null or a static record, whose name is copied at once.
    unsafe {
        let p = libc::getpwuid(uid);
        if p.is_null() { None } else { copy_cstr((*p).pw_name) }
    }
}

/// The name of group `gid` (`getgrgid`).
pub fn group_name(gid: u32) -> Option<String> {
    // SAFETY: null or a static record, whose name is copied at once.
    unsafe {
        let p = libc::getgrgid(gid);
        if p.is_null() { None } else { copy_cstr((*p).gr_name) }
    }
}

/// `kill(pid, signal)`.
pub fn kill(pid: i32, signal: c_int) {
    // SAFETY: no pointers; a bad pid is an error return.
    unsafe { libc::kill(pid, signal) };
}

// ----------------------------------------------------------------------
// Locale

/// `setlocale(category, locale)`; `locale` `None` queries. The result is
/// the locale's name, or `None` when the C library refuses.
pub fn setlocale(category: c_int, locale: Option<&CStr>) -> Option<String> {
    let locale = locale.map_or(std::ptr::null(), CStr::as_ptr);
    // SAFETY: null or NUL-terminated; the result is null or a C string,
    // which the next call may overwrite and which is copied at once.
    unsafe { copy_cstr(libc::setlocale(category, locale)) }
}

/// The fields of `localeconv()` in the order of R's `Sys.localeconv()`, or
/// `None` when it fails.
pub fn localeconv() -> Option<Vec<(&'static str, String)>> {
    // SAFETY: null or a struct that stays valid until the next localeconv
    // or setlocale; all of it is copied below, before either can run.
    let lc = unsafe { libc::localeconv().as_ref() }?;
    let text = |p: *const c_char| {
        // SAFETY: lconv's string members are null or NUL-terminated.
        unsafe { copy_cstr(p) }.unwrap_or_default()
    };
    let num = |c: c_char| (c as i32).to_string();
    Some(vec![
        ("decimal_point", text(lc.decimal_point)),
        ("thousands_sep", text(lc.thousands_sep)),
        ("grouping", text(lc.grouping)),
        ("int_curr_symbol", text(lc.int_curr_symbol)),
        ("currency_symbol", text(lc.currency_symbol)),
        ("mon_decimal_point", text(lc.mon_decimal_point)),
        ("mon_thousands_sep", text(lc.mon_thousands_sep)),
        ("mon_grouping", text(lc.mon_grouping)),
        ("positive_sign", text(lc.positive_sign)),
        ("negative_sign", text(lc.negative_sign)),
        ("int_frac_digits", num(lc.int_frac_digits)),
        ("frac_digits", num(lc.frac_digits)),
        ("p_cs_precedes", num(lc.p_cs_precedes)),
        ("p_sep_by_space", num(lc.p_sep_by_space)),
        ("n_cs_precedes", num(lc.n_cs_precedes)),
        ("n_sep_by_space", num(lc.n_sep_by_space)),
        ("p_sign_posn", num(lc.p_sign_posn)),
        ("n_sign_posn", num(lc.n_sign_posn)),
    ])
}

// ----------------------------------------------------------------------
// Files

/// `access(path, mode)`: 0 or -1, as R's `file.access` reports it. A path
/// with a NUL byte is -1.
pub fn access(path: &str, mode: c_int) -> c_int {
    match c_path(path) {
        // SAFETY: a NUL-terminated path, read during the call only.
        Some(c) => unsafe { libc::access(c.as_ptr(), mode) },
        None => -1,
    }
}

/// `umask(mode)`, returning the previous mask.
pub fn set_umask(mode: u32) -> u32 {
    // SAFETY: no pointers, cannot fail.
    unsafe { libc::umask(mode as libc::mode_t) as u32 }
}

/// The process umask: POSIX can only read it by setting it.
pub fn current_umask() -> u32 {
    let mask = set_umask(0);
    set_umask(mask);
    mask
}

/// `utimensat(AT_FDCWD, path, {t, t}, 0)`: both file times, following
/// symbolic links (`std` needs an open file, which a directory the caller
/// cannot read has not).
pub fn set_file_times(path: &str, sec: i64, nsec: i64) -> bool {
    let Some(c) = c_path(path) else { return false };
    let time = libc::timespec { tv_sec: sec as libc::time_t, tv_nsec: nsec as _ };
    let times = [time, time];
    // SAFETY: a NUL-terminated path and an array of the two times utimensat
    // reads, both valid for the call.
    unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), 0) == 0 }
}

/// `readlink(path, buf, max)`, unterminated, with the C result (`Err` holds
/// errno): the caller sees truncation, which `std::fs::read_link` hides.
pub fn readlink(path: &str, max: usize) -> Result<Vec<u8>, i32> {
    let c = c_path(path).ok_or(libc::ENOENT)?;
    let mut buf = vec![0u8; max];
    // SAFETY: `buf` is valid for `max` bytes, which is all readlink writes.
    let n = unsafe { libc::readlink(c.as_ptr(), buf.as_mut_ptr().cast(), max) };
    if n < 0 {
        return Err(last_errno());
    }
    buf.truncate(n as usize);
    Ok(buf)
}

/// The entry names of a directory in `readdir` order, `"."` and `".."`
/// included (`std::fs::read_dir` hides them, and R lists them). `None` when
/// the path is not an openable directory.
pub fn read_dir_names(path: &str) -> Option<Vec<String>> {
    let c = c_path(path)?;
    // SAFETY: a NUL-terminated path.
    let dir = unsafe { libc::opendir(c.as_ptr()) };
    if dir.is_null() {
        return None;
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: `dir` is open, and only this loop reads it.
        let entry = unsafe { libc::readdir(dir) };
        if entry.is_null() {
            break;
        }
        // SAFETY: the entry is valid until the next readdir, and d_name is
        // NUL-terminated. Its bytes are copied here.
        names.extend(unsafe { copy_cstr((*entry).d_name.as_ptr()) });
    }
    // SAFETY: `dir` is open and is not used again.
    unsafe { libc::closedir(dir) };
    Some(names)
}

/// `open(path, flags)` for a FIFO, which needs flags `std` cannot express
/// without also setting `O_CLOEXEC`. `Err` holds errno.
pub fn open(path: &str, flags: c_int) -> Result<File, i32> {
    let c = c_path(path).ok_or(libc::ENOENT)?;
    // SAFETY: a NUL-terminated path; these flags take no mode argument.
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 {
        return Err(last_errno());
    }
    // SAFETY: `fd` was just opened, and nothing else owns it.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `mkfifo(path, 0644)`, returning errno on failure and 0 on success. A
/// path that cannot be a C string reports success: the following `open`
/// reports it.
pub fn mkfifo(path: &str) -> i32 {
    let Some(c) = c_path(path) else { return 0 };
    // SAFETY: a NUL-terminated path.
    if unsafe { libc::mkfifo(c.as_ptr(), 0o644) } == 0 { 0 } else { last_errno() }
}

/// `mkdtemp(template)`: the directory's name, or `None` on failure.
pub fn mkdtemp(template: &str) -> Option<String> {
    let mut bytes = CString::new(template).ok()?.into_bytes_with_nul();
    // SAFETY: a writable NUL-terminated template, edited in place.
    let p = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) };
    // SAFETY: null, or `bytes`, which is still NUL-terminated.
    unsafe { copy_cstr(p) }
}

/// Run `f` at process exit.
pub fn at_exit(f: extern "C" fn()) {
    // SAFETY: `f` is a plain function, valid for the whole process.
    unsafe { libc::atexit(f) };
}

/// `glob`'s result list over several patterns (`GLOB_APPEND`), released
/// when dropped.
pub struct Glob {
    glob: libc::glob_t,
    started: bool,
}

impl Glob {
    pub fn new() -> Glob {
        // SAFETY: an all-zero glob_t is glob's documented initial state.
        Glob { glob: unsafe { std::mem::zeroed() }, started: false }
    }

    /// Add the matches of `pattern`, returning `glob`'s code.
    pub fn add(&mut self, pattern: &CStr, mark: bool) -> c_int {
        let flags = (if mark { libc::GLOB_MARK } else { 0 }) | (if self.started { libc::GLOB_APPEND } else { 0 });
        self.started = true;
        // SAFETY: a NUL-terminated pattern, and the struct glob owns from
        // here on: GLOB_APPEND is set from the second call on.
        unsafe { libc::glob(pattern.as_ptr(), flags, None, &mut self.glob) }
    }

    /// The paths matched so far.
    pub fn paths(&self) -> Vec<String> {
        if !self.started {
            return Vec::new();
        }
        (0..self.glob.gl_pathc as usize)
            .filter_map(|k| {
                // SAFETY: glob stored gl_pathc NUL-terminated strings.
                unsafe { copy_cstr(*self.glob.gl_pathv.add(k)) }
            })
            .collect()
    }
}

impl Default for Glob {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Glob {
    fn drop(&mut self) {
        if self.started {
            // SAFETY: glob filled this struct, and it is freed once.
            unsafe { libc::globfree(&mut self.glob) };
        }
    }
}

/// A file behind C's buffered `FILE`. Unlike [`File`] it does not close when
/// dropped: `save()` leaves buffered bytes to C's exit-time flush when an
/// error unwinds past it, as R does.
pub struct CFile(*mut libc::FILE);

impl CFile {
    /// `fdopen(fd, "wb")`, closing `fd` on failure.
    pub fn for_writing(file: OwnedFd) -> Option<CFile> {
        let fd = file.into_raw_fd();
        // SAFETY: `fd` is open, and the FILE takes it over.
        let stream = unsafe { libc::fdopen(fd, c"wb".as_ptr()) };
        if stream.is_null() {
            // SAFETY: fdopen failed, so `fd` is still ours to close.
            unsafe { libc::close(fd) };
            return None;
        }
        Some(CFile(stream))
    }

    /// `fwrite`, returning whether all of `bytes` was taken.
    pub fn write(&mut self, bytes: &[u8]) -> bool {
        // SAFETY: the stream is open until `close` consumes `self`, and the
        // pointer and length describe `bytes`.
        unsafe { libc::fwrite(bytes.as_ptr().cast(), 1, bytes.len(), self.0) == bytes.len() }
    }

    /// `fclose`.
    pub fn close(self) {
        // SAFETY: the stream is open, and consuming `self` ends its use.
        unsafe { libc::fclose(self.0) };
    }
}

/// A read-only private file mapping, unmapped when dropped. Only the pages
/// actually read enter the process, so a flat file costs memory for the
/// entries a run fetched, not for all of them.
pub struct Mmap {
    ptr: *mut u8,
    len: usize,
}

impl Mmap {
    /// Map the first `len` bytes of `file`.
    pub fn new(file: &File, len: u64) -> Option<Mmap> {
        use std::os::fd::AsRawFd;
        let len = usize::try_from(len).ok()?;
        if len == 0 || len > isize::MAX as usize {
            return None;
        }
        // SAFETY: a new mapping at an address the kernel picks, so nothing
        // existing is replaced.
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0)
        };
        if p == libc::MAP_FAILED {
            return None;
        }
        Some(Mmap { ptr: p.cast(), len })
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `len` readable bytes are mapped until drop. The one caller
        // maps rdbflat cache files, which a session never writes: they are
        // rebuilt only by `scripts/build-rdbflat.py`, so only rebuilding the
        // cache under a running session can change them.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        // SAFETY: exactly this range was mapped, and `as_slice`'s borrows
        // have ended.
        unsafe { libc::munmap(self.ptr.cast::<c_void>(), self.len) };
    }
}

