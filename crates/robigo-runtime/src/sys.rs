//! The C library, behind safe functions.
//!
//! Builtins call what POSIX offers and `std` does not (or offers with other
//! semantics) through this module, so the unsafe code for it is here and only
//! here (`docs/architecture.md`, "Unsafe code"). Each wrapper owns its
//! contract: NUL-terminated arguments, buffers that outlive the call, and
//! copying results out of the C library's static storage before returning.
//!
//! Functions that hand out the C library's static storage copy it under one
//! lock (`c_statics`), so these are safe to call from any thread. Changing
//! the environment or the locale is another matter: only a caller can know
//! that no other thread is running C code that reads them, so the first
//! stays at its call sites and [`set_locale`] is `unsafe`.

use std::ffi::{CStr, CString, OsStr, c_char, c_int, c_void};
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
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

/// Held while using a function that returns the C library's static storage,
/// so that safe callers on any thread cannot overwrite each other's results.
fn c_statics() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ----------------------------------------------------------------------
// Environment and numbers

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
    let _statics = c_statics();
    // SAFETY: strerror returns a NUL-terminated string, copied under the lock.
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

/// The C library's conversion descriptor for one source encoding.
pub struct Iconv(libc::iconv_t);

impl Iconv {
    // R: sysutils.c:Riconv_open
    pub fn open(to: &str, from: &str) -> Option<Iconv> {
        use std::ffi::CString;
        let (t, f) = (CString::new(to).ok()?, CString::new(from).ok()?);
        // SAFETY: both arguments are valid NUL-terminated strings.
        let cd = unsafe { libc::iconv_open(t.as_ptr(), f.as_ptr()) };
        (cd as isize != -1).then_some(Iconv(cd))
    }

    /// One `iconv(3)` step; `Err` carries errno. Advances the slices past
    /// what was consumed and produced.
    pub fn step(&self, input: &mut &[u8], out: &mut &mut [u8]) -> Result<(), i32> {
        let mut inbuf = input.as_ptr() as *mut libc::c_char;
        let mut inb = input.len();
        let mut outbuf = out.as_mut_ptr() as *mut libc::c_char;
        let mut outb = out.len();
        // SAFETY: the pointers and lengths come from live slices; iconv
        // does not write through `inbuf` despite its type.
        let res = unsafe { libc::iconv(self.0, &mut inbuf, &mut inb, &mut outbuf, &mut outb) };
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        *input = &input[input.len() - inb..];
        let done = out.len() - outb;
        *out = &mut std::mem::take(out)[done..];
        if res == usize::MAX { Err(err) } else { Ok(()) }
    }

    /// Back to the initial shift state, writing what that takes to `out`.
    pub fn reset(&self, out: &mut &mut [u8]) {
        let mut outbuf = out.as_mut_ptr() as *mut libc::c_char;
        let mut outb = out.len();
        // SAFETY: a null input asks iconv to reset; the output pointer and
        // length come from a live slice.
        unsafe { libc::iconv(self.0, std::ptr::null_mut(), std::ptr::null_mut(), &mut outbuf, &mut outb) };
        let done = out.len() - outb;
        *out = &mut std::mem::take(out)[done..];
    }
}

impl Drop for Iconv {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from iconv_open and is closed once.
        unsafe { libc::iconv_close(self.0) };
    }
}

/// Whether `iconv` can convert `from` to `to`; R reports a failing
/// `iconv_open` as an unsupported conversion.
pub fn iconv_supported(to: &str, from: &str) -> bool {
    Iconv::open(to, from).is_some()
}

/// The locale's character encoding (`nl_langinfo(CODESET)`).
pub fn codeset() -> String {
    let _statics = c_statics();
    // SAFETY: a NUL-terminated string, copied under the lock.
    unsafe { copy_cstr(libc::nl_langinfo(libc::CODESET)) }.unwrap_or_default()
}

/// Whether the locale is a UTF-8 one (R: platform.c:R_check_locale's
/// `utf8locale`, a case-insensitive match of the codeset).
pub fn utf8locale() -> bool {
    codeset().eq_ignore_ascii_case("UTF-8")
}

unsafe extern "C" {
    #[cfg(target_os = "linux")]
    safe fn __ctype_get_mb_cur_max() -> usize;
    #[cfg(target_os = "macos")]
    safe fn ___mb_cur_max() -> c_int;
}

/// The C library's `MB_CUR_MAX` for the current locale.
pub fn mb_cur_max() -> usize {
    #[cfg(target_os = "linux")]
    return __ctype_get_mb_cur_max();
    #[cfg(target_os = "macos")]
    return ___mb_cur_max() as usize;
}

unsafe extern "C" {
    safe fn iswprint(wc: u32) -> c_int;
    safe fn wcwidth(wc: i32) -> c_int;
    fn mbrtowc(pwc: *mut i32, s: *const c_char, n: usize, ps: *mut c_void) -> usize;
}

/// C's `isprint` in the current locale (`LC_CTYPE`), for one byte.
pub fn is_print(b: u8) -> bool {
    // SAFETY: any value of `unsigned char` is a valid argument.
    unsafe { libc::isprint(b as c_int) != 0 }
}

/// C's `isalpha` in the current locale, for one byte.
pub fn is_alpha(b: u8) -> bool {
    // SAFETY: any value of `unsigned char` is a valid argument.
    unsafe { libc::isalpha(b as c_int) != 0 }
}

/// C's `isalnum` in the current locale, for one byte.
pub fn is_alnum(b: u8) -> bool {
    // SAFETY: any value of `unsigned char` is a valid argument.
    unsafe { libc::isalnum(b as c_int) != 0 }
}

/// C's `iswprint` in the current locale, for one wide character.
pub fn is_wprint(wc: u32) -> bool {
    iswprint(wc) != 0
}

/// C's `wcwidth` in the current locale (`-1` for unprintable characters).
pub fn wc_width(wc: u32) -> i32 {
    wcwidth(wc as i32)
}

/// A shift state for [`MbDecoder::next`], C's `mbstate_t` (128 bytes holds
/// glibc's and macOS's).
pub struct MbDecoder([u64; 16]);

impl MbDecoder {
    pub fn new() -> MbDecoder {
        MbDecoder([0; 16])
    }

    /// C's `mbrtowc` on the start of `s`: the character and how many bytes
    /// it took (`0` for a NUL), or `None` for an invalid or incomplete one.
    /// Both failures (`(size_t)-1` and `(size_t)-2`) reset the shift state,
    /// so an incomplete character is not buffered: callers must re-feed
    /// from the start of a character, never just the remaining bytes.
    pub fn next(&mut self, s: &[u8]) -> Option<(u32, usize)> {
        let mut wc = 0i32;
        // SAFETY: `s` is live for `s.len()` bytes, `wc` and the state are
        // writable, and the state is larger than any `mbstate_t`.
        let res = unsafe { mbrtowc(&mut wc, s.as_ptr().cast(), s.len(), self.0.as_mut_ptr().cast()) };
        if res >= usize::MAX - 1 {
            // An invalid sequence leaves the state undefined; an incomplete
            // one would keep its prefix, which callers never resume.
            self.0 = [0; 16];
            None
        } else {
            Some((wc as u32, res))
        }
    }
}

impl Default for MbDecoder {
    fn default() -> Self {
        Self::new()
    }
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
    let _statics = c_statics();
    // SAFETY: null or a NUL-terminated static buffer, copied under the lock.
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
    let _statics = c_statics();
    // SAFETY: null or a static record, whose name is copied under the lock.
    unsafe {
        let p = libc::getpwuid(uid);
        if p.is_null() { None } else { copy_cstr((*p).pw_name) }
    }
}

/// The name of group `gid` (`getgrgid`).
pub fn group_name(gid: u32) -> Option<String> {
    let _statics = c_statics();
    // SAFETY: null or a static record, whose name is copied under the lock.
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

/// The name of the locale of `category` (`setlocale(category, NULL)`).
pub fn locale(category: c_int) -> Option<String> {
    let _statics = c_statics();
    // SAFETY: a null locale only queries. The result is null or a C string,
    // which the next call may overwrite and which is copied under the lock.
    unsafe { copy_cstr(libc::setlocale(category, std::ptr::null())) }
}

/// `setlocale(category, locale)`: the new locale's name, or `None` when the C
/// library refuses.
///
/// # Safety
/// No other thread runs C code that reads the locale, as `snprintf` and every
/// function here do: glibc does not synchronize them with this call.
pub unsafe fn set_locale(category: c_int, locale: &CStr) -> Option<String> {
    let _statics = c_statics();
    // SAFETY: as for `locale`, and the caller rules out other readers.
    unsafe { copy_cstr(libc::setlocale(category, locale.as_ptr())) }
}

/// The fields of `localeconv()` in the order of R's `Sys.localeconv()`, or
/// `None` when it fails.
pub fn localeconv() -> Option<Vec<(&'static str, String)>> {
    let _statics = c_statics();
    // SAFETY: null or a struct that stays valid until the next localeconv
    // or setlocale; all of it is copied below, under the lock both take.
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
    // SAFETY: a NUL-terminated path. The mode is ignored unless `flags`
    // create a file, and a file it creates then has no permissions.
    let fd = unsafe { libc::open(c.as_ptr(), flags, 0 as libc::c_uint) };
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
        (0..self.glob.gl_pathc)
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
    ///
    /// # Safety
    /// Nothing writes to or truncates those bytes while the mapping lives:
    /// `as_slice` hands them out as `&[u8]`, and `MAP_PRIVATE` does not hide
    /// later writes to the file.
    pub unsafe fn new(file: &File, len: u64) -> Option<Mmap> {
        use std::os::fd::AsRawFd;
        let len = usize::try_from(len).ok()?;
        if len == 0 || len > isize::MAX as usize {
            return None;
        }
        // SAFETY: a new mapping at an address the kernel picks, so nothing
        // existing is replaced.
        let p =
            unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return None;
        }
        Some(Mmap { ptr: p.cast(), len })
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `len` readable bytes stay mapped and, by `new`'s contract,
        // unchanged until drop.
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

// ----------------------------------------------------------------------
// Sockets (R: modules/internet/Rsock.c, sock.c)

/// R: Rsock.c's `fd_set`s bound what a socket's descriptor can be.
pub const FD_SETSIZE: i32 = libc::FD_SETSIZE as i32;

fn socket_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Wait for each of `fds` (with its `events`) for `timeout` seconds, for
/// ever if negative, whichever comes first. `Ok(false)`: the time ran out.
pub fn socket_poll(fds: &mut [libc::pollfd], timeout: f64) -> Result<bool, i32> {
    let start = std::time::Instant::now();
    loop {
        let left = timeout - start.elapsed().as_secs_f64();
        let ms = if timeout < 0.0 { 60_000.0 } else { (left.max(0.0) * 1e3).ceil().min(60_000.0) };
        // SAFETY: the pointer and length come from a live slice.
        let howmany = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms as libc::c_int) };
        if howmany < 0 {
            return Err(socket_errno());
        }
        if howmany > 0 {
            return Ok(true);
        }
        if timeout >= 0.0 && start.elapsed().as_secs_f64() >= timeout {
            return Ok(false);
        }
    }
}

/// What to wait for on `fd`; poll ignores a connection that has no socket.
pub fn pollfd(fd: Option<BorrowedFd<'_>>, write: bool) -> libc::pollfd {
    let fd = fd.map_or(-1, |f| f.as_raw_fd());
    libc::pollfd { fd, events: if write { libc::POLLOUT } else { libc::POLLIN }, revents: 0 }
}

/// R: Rsock.c:R_SocketWait -- 0 when ready, 1 on time out, or `-errno`.
pub fn socket_wait(fd: BorrowedFd<'_>, write: bool, timeout: i32) -> isize {
    if fd.as_raw_fd() >= FD_SETSIZE {
        return -(libc::EINVAL as isize);
    }
    match socket_poll(&mut [pollfd(Some(fd), write)], timeout as f64) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => -(e as isize),
    }
}

fn set_nonblocking(fd: BorrowedFd<'_>) -> bool {
    let fd = fd.as_raw_fd();
    // SAFETY: fcntl with integer arguments on a live descriptor.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    }
}

fn set_int_option(fd: BorrowedFd<'_>, level: i32, name: i32) {
    let val: libc::c_int = 1;
    let (p, size) = (&val as *const _ as *const libc::c_void, size_of::<libc::c_int>() as _);
    // SAFETY: the option's value is a live c_int of the stated size.
    unsafe { libc::setsockopt(fd.as_raw_fd(), level, name, p, size) };
}

/// A new socket of the C library's, which nothing else knows of.
fn new_socket(domain: i32, protocol: i32) -> Option<OwnedFd> {
    // SAFETY: plain call; a descriptor it returns has no other owner.
    unsafe {
        let s = libc::socket(domain, libc::SOCK_STREAM, protocol);
        (s >= 0).then(|| OwnedFd::from_raw_fd(s))
    }
}

fn inet_addr(ip: std::net::Ipv4Addr, port: i32) -> libc::sockaddr_in {
    // SAFETY: sockaddr_in is plain data, valid when zeroed.
    let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    a.sin_family = libc::AF_INET as _;
    a.sin_port = (port as u16).to_be();
    a.sin_addr.s_addr = u32::from(ip).to_be();
    a
}

/// R: sock.c:Sock_open, non-blocking -- socket(), bind() and listen().
pub fn socket_listen(port: i32) -> Option<OwnedFd> {
    // R: sock.c MAXBACKLOG
    const MAXBACKLOG: i32 = libc::SOMAXCONN;
    let sock = new_socket(libc::AF_INET, 0)?;
    if !set_nonblocking(sock.as_fd()) {
        return None;
    }
    set_int_option(sock.as_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR);
    let fd = sock.as_raw_fd();
    let server = inet_addr(std::net::Ipv4Addr::UNSPECIFIED, port);
    let size = size_of::<libc::sockaddr_in>() as libc::socklen_t;
    // SAFETY: integer arguments; the address is a live sockaddr_in.
    unsafe {
        // child processes, those of system() too, must not hold the port
        let flags = libc::fcntl(fd, libc::F_GETFD, 0);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) == -1 {
            return None;
        }
        if libc::bind(fd, &server as *const _ as *const libc::sockaddr, size) < 0 || libc::listen(fd, MAXBACKLOG) < 0 {
            return None;
        }
    }
    Some(sock)
}

/// The name of a peer, or "unknown" (R: sock.c:Sock_listen, gethostbyaddr).
fn peer_name(addr: &libc::sockaddr_in) -> String {
    let mut host = [0 as libc::c_char; 256];
    // SAFETY: the address and the buffer are live, with their sizes.
    let rc = unsafe {
        libc::getnameinfo(
            addr as *const _ as *const libc::sockaddr,
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
            host.as_mut_ptr(),
            host.len() as _,
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        )
    };
    if rc != 0 {
        return "unknown".to_string();
    }
    // SAFETY: getnameinfo wrote a NUL-terminated name.
    unsafe { std::ffi::CStr::from_ptr(host.as_ptr()) }.to_string_lossy().into_owned()
}

/// R: Rsock.c:R_SockListen -- accept(), for at most `timeout` seconds.
pub fn socket_accept(fd: BorrowedFd<'_>, timeout: i32) -> Option<(OwnedFd, String)> {
    if fd.as_raw_fd() >= FD_SETSIZE {
        return None;
    }
    let start = std::time::Instant::now();
    loop {
        let left = (timeout as f64 - start.elapsed().as_secs_f64()).max(0.0);
        match socket_poll(&mut [pollfd(Some(fd), false)], left) {
            Err(libc::EINTR) => continue,
            Err(_) | Ok(false) => return None,
            Ok(true) => {}
        }
        // SAFETY: sockaddr_in is plain data; accept fills in this much.
        let mut client: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;
        // SAFETY: as above.
        let s = unsafe { libc::accept(fd.as_raw_fd(), &mut client as *mut _ as *mut libc::sockaddr, &mut len) };
        if s < 0 {
            match socket_errno() {
                libc::EINTR | libc::EINPROGRESS | libc::EWOULDBLOCK | libc::ECONNABORTED | libc::EPROTO => continue,
                _ => return None,
            }
        }
        // SAFETY: a descriptor accept returned has no other owner.
        let s = unsafe { OwnedFd::from_raw_fd(s) };
        // the socket was ready, but maybe no longer is: never block on it
        if !set_nonblocking(s.as_fd()) {
            return None;
        }
        return Some((s, peer_name(&client)));
    }
}

/// R: Rsock.c:R_SockConnect, with IPv4 as `R_gethostbyname` gives.
pub fn socket_connect(port: i32, host: &str, timeout: i32) -> Option<OwnedFd> {
    let ip = std::net::ToSocketAddrs::to_socket_addrs(&(host, port as u16))
        .ok()
        .and_then(|mut a| {
            a.find_map(|a| match a {
                std::net::SocketAddr::V4(v) => Some(*v.ip()),
                _ => None,
            })
        })
        .or_else(|| (host == "localhost").then_some(std::net::Ipv4Addr::LOCALHOST))?;
    let s = new_socket(libc::PF_INET, libc::IPPROTO_TCP)?;
    if !set_nonblocking(s.as_fd()) {
        return None;
    }
    let server = inet_addr(ip, port);
    let size = size_of::<libc::sockaddr_in>() as libc::socklen_t;
    // SAFETY: the address is a live sockaddr_in.
    if unsafe { libc::connect(s.as_raw_fd(), &server as *const _ as *const libc::sockaddr, size) } == 0 {
        return Some(s);
    }
    if !matches!(socket_errno(), libc::EINPROGRESS | libc::EWOULDBLOCK) || socket_wait(s.as_fd(), true, timeout) != 0 {
        return None;
    }
    let mut status: libc::c_int = 0;
    let mut len = size_of::<libc::c_int>() as libc::socklen_t;
    let p = &mut status as *mut _ as *mut libc::c_void;
    // SAFETY: `p` and `len` describe the live c_int.
    let rc = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ERROR, p, &mut len) };
    (rc == 0 && status == 0).then_some(s)
}

/// `recv(2)`: bytes read, or errno.
pub fn socket_recv(fd: BorrowedFd<'_>, buf: &mut [u8]) -> Result<usize, i32> {
    // SAFETY: the buffer is live, with this length.
    let res = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
    if res < 0 { Err(socket_errno()) } else { Ok(res as usize) }
}

/// `send(2)`: bytes written, or errno. A vanished peer is a failed write,
/// not the end of the process.
pub fn socket_send(fd: BorrowedFd<'_>, buf: &[u8]) -> Result<usize, i32> {
    // SAFETY: the buffer is live, with this length.
    let res = unsafe { libc::send(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len(), libc::MSG_NOSIGNAL) };
    if res < 0 { Err(socket_errno()) } else { Ok(res as usize) }
}

pub fn socket_nodelay(fd: BorrowedFd<'_>) {
    set_int_option(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> String {
        let dir = mkdtemp(&format!("{}/robigo-sys-{name}-XXXXXX", std::env::temp_dir().display())).unwrap();
        assert!(std::path::Path::new(&dir).is_dir());
        dir
    }

    #[test]
    fn read_dir_names_lists_the_dot_entries() {
        let dir = scratch("readdir");
        std::fs::write(format!("{dir}/a"), "").unwrap();
        let mut names = read_dir_names(&dir).unwrap();
        names.sort();
        assert_eq!(names, [".", "..", "a"]);
        assert_eq!(read_dir_names(&format!("{dir}/none")), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn readlink_reports_links_truncation_and_errno() {
        let dir = scratch("readlink");
        let link = format!("{dir}/l");
        std::os::unix::fs::symlink("target", &link).unwrap();
        assert_eq!(readlink(&link, 64).unwrap(), b"target");
        assert_eq!(readlink(&link, 3).unwrap(), b"tar");
        assert_eq!(readlink(&dir, 64), Err(libc::EINVAL));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn glob_appends_across_patterns_and_marks_directories() {
        let dir = scratch("glob");
        std::fs::write(format!("{dir}/a.txt"), "").unwrap();
        std::fs::create_dir(format!("{dir}/b")).unwrap();
        let mut g = Glob::new();
        assert_eq!(g.paths(), Vec::<String>::new());
        assert_eq!(g.add(&CString::new(format!("{dir}/*.txt")).unwrap(), false), 0);
        assert_eq!(g.add(&CString::new(format!("{dir}/b")).unwrap(), true), 0);
        assert_eq!(g.paths(), [format!("{dir}/a.txt"), format!("{dir}/b/")]);
        drop(g);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_times_and_access_follow_the_c_library() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("times");
        let file = format!("{dir}/f");
        std::fs::write(&file, "").unwrap();
        assert!(set_file_times(&file, 1000, 5));
        let md = std::fs::metadata(&file).unwrap();
        assert_eq!((md.mtime(), md.mtime_nsec()), (1000, 5));
        assert!(!set_file_times(&format!("{dir}/none"), 0, 0));
        assert_eq!(access(&file, libc::F_OK), 0);
        assert_eq!(access("a\0b", libc::F_OK), -1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saved_bytes_reach_the_file_when_closed() {
        let dir = scratch("cfile");
        let path = format!("{dir}/f");
        let mut file = CFile::for_writing(File::create(&path).unwrap().into()).unwrap();
        assert!(file.write(b"RDX3\n"));
        file.close();
        assert_eq!(std::fs::read(&path).unwrap(), b"RDX3\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_mapping_shows_the_file() {
        let dir = scratch("mmap");
        let path = format!("{dir}/f");
        std::fs::write(&path, "mapped").unwrap();
        let file = File::open(&path).unwrap();
        // SAFETY: only this test uses the file.
        let (map, empty) = unsafe { (Mmap::new(&file, 6), Mmap::new(&file, 0)) };
        assert_eq!(map.unwrap().as_slice(), b"mapped");
        assert!(empty.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn c_library_values_are_copied_out() {
        assert_eq!(format_g16_line(0.1), "0.1\n");
        assert_eq!(format_g16_line(1e300), "1e+300\n");
        assert_eq!(atoi("3junk"), Some(3));
        assert_eq!(atoi("a\0b"), None);
        assert!(!strerror(libc::ENOENT).is_empty());
        assert!(iconv_supported("UTF-8", "latin1"));
        assert!(!iconv_supported("no-such-encoding", "UTF-8"));
        assert!(locale(libc::LC_ALL).is_some());
        assert_eq!(localeconv().unwrap()[0].0, "decimal_point");
        assert_eq!(current_umask(), current_umask());
        assert!(uname().is_some_and(|u| !u.sysname.is_empty()));
        assert!(!lzma_version().is_empty());
    }

    #[test]
    fn complex_helpers_use_the_c_abi() {
        // atan(1 / 1) is pi / 4.
        let (re, im) = catan_of_quotient((1.0, 0.0), (1.0, 0.0));
        assert_eq!((re, im), (std::f64::consts::FRAC_PI_4, 0.0));
    }
}
