//! Files, directories and multibyte helpers of GNU R's sysutils.c,
//! platform.c and util.c that packages (and GNU R's tools.so) call.

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::Sexp;
use crate::envir::{R_ExpandFileName, R_NewEnv};
use crate::errors::error_str_owned;
use crate::strings::Rf_translateCharFP;

unsafe extern "C" {
    fn mbrtowc(wc: *mut libc::wchar_t, s: *const c_char, n: usize, ps: *mut libc::mbstate_t) -> usize;
}

// R: sysutils.c:RC_fopen
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RC_fopen(fn_: Sexp, mode: *const c_char, expand: c_int) -> *mut libc::FILE {
    if fn_ == crate::state::core().na_string {
        return std::ptr::null_mut();
    }
    let filename = Rf_translateCharFP(fn_);
    if filename.is_null() {
        return std::ptr::null_mut();
    }
    let res = if expand != 0 { R_ExpandFileName(filename) } else { filename };
    libc::fopen(res, mode)
}

// R: sysutils.c:R_FileExists
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_FileExists(path: *const c_char) -> bool {
    let mut sb: libc::stat = std::mem::zeroed();
    libc::stat(R_ExpandFileName(path), &mut sb) == 0
}

/// platform.c's `struct R_DIR_INTERNAL` on Unix: the stream, then the
/// `struct R_dirent { char *d_name; }` handed out by `R_readdir`.
#[repr(C)]
pub struct RDir {
    dirp: *mut libc::DIR,
    d_name: *mut c_char,
}

// R: platform.c:R_opendir
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_opendir(name: *const c_char) -> *mut RDir {
    let dirp = libc::opendir(name);
    if dirp.is_null() {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(RDir { dirp, d_name: std::ptr::null_mut() }))
}

// R: platform.c:R_readdir
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_readdir(rdir: *mut RDir) -> *mut c_void {
    if rdir.is_null() {
        *libc::__errno_location() = libc::EFAULT;
        return std::ptr::null_mut();
    }
    let de = libc::readdir((*rdir).dirp);
    if de.is_null() {
        return std::ptr::null_mut();
    }
    (*rdir).d_name = (*de).d_name.as_mut_ptr();
    std::ptr::addr_of_mut!((*rdir).d_name) as *mut c_void
}

// R: platform.c:R_closedir
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_closedir(rdir: *mut RDir) -> c_int {
    if rdir.is_null() {
        *libc::__errno_location() = libc::EFAULT;
        return -1;
    }
    let r = Box::from_raw(rdir);
    libc::closedir(r.dirp)
}

// R: util.c:Rf_strchr (robigo's locale is UTF-8, where it is strchr)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_strchr(s: *const c_char, c: c_int) -> *mut c_char {
    libc::strchr(s, c)
}

// R: util.c:Mbrtowc
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_mbrtowc(
    wc: *mut libc::wchar_t,
    s: *const c_char,
    n: usize,
    ps: *mut libc::mbstate_t,
) -> usize {
    if n == 0 || *s == 0 {
        return 0;
    }
    let used = mbrtowc(wc, s, n, ps);
    if used as isize >= 0 {
        return used;
    }
    // a readable version of the string: valid characters as they are,
    // invalid bytes as <xx>
    let bytes = CStr::from_ptr(s).to_bytes();
    let mut err = Vec::new();
    let mut p = 0;
    let mut n = n;
    let mut used = used;
    while p < bytes.len() {
        if p > 0 {
            used = mbrtowc(std::ptr::null_mut(), s.add(p), n, ps);
        }
        if used == 0 {
            break;
        } else if (used as isize) > 0 {
            err.extend_from_slice(&bytes[p..p + used]);
            p += used;
            n = n.wrapping_sub(used);
        } else {
            err.extend_from_slice(format!("<{:02x}>", bytes[p]).as_bytes());
            p += 1;
            n = n.wrapping_sub(1);
        }
    }
    error_str_owned(format!("invalid multibyte string at '{}'", String::from_utf8_lossy(&err)))
}

// R: envir.c:R_NewHashedEnv (robigo's environments size themselves)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_NewHashedEnv(enclos: Sexp, size: c_int) -> Sexp {
    R_NewEnv(enclos, 1, size)
}
