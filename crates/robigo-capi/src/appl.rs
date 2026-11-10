//! Small pieces of GNU R's src/appl, src/nmath and main/util.c that
//! packages and GNU R's own natives call (interv, d1mach, the Fortran
//! helpers), plus internal entry points base-package sources use.

use crate::errors::{Res, ret, throw};
use crate::memory::*;
use crate::state::*;
use robigo_core::Sexp;
use std::ffi::{c_char, c_int};

// R: appl/interv.c:findInterval2 (xt 1-indexed in the C code)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn findInterval2(
    xt: *const f64,
    n: c_int,
    x: f64,
    rightmost_closed: c_int,
    all_inside: c_int,
    left_open: c_int,
    ilo: c_int,
    mflag: *mut c_int,
) -> c_int {
    let (rc, ai, lo) = (rightmost_closed != 0, all_inside != 0, left_open != 0);
    let xt1 = |i: c_int| *xt.offset(i as isize - 1);
    let grtr = |v: f64| x > v || (!lo && x >= v);
    let smlr = |v: f64| x < v || (lo && x <= v);
    let left_boundary = |mflag: *mut c_int| -> c_int {
        *mflag = -1;
        if ai || (rc && x == xt1(1)) { 1 } else { 0 }
    };
    let right_boundary = |mflag: *mut c_int| -> c_int {
        *mflag = 1;
        if ai || (rc && x == xt1(n)) { n - 1 } else { n }
    };
    if n == 0 {
        *mflag = 0;
        return 0;
    }
    let mut ilo = ilo;
    if ilo <= 0 {
        if smlr(xt1(1)) {
            return left_boundary(mflag);
        }
        ilo = 1;
    }
    let mut ihi = ilo + 1;
    if ihi >= n {
        if grtr(xt1(n)) {
            return right_boundary(mflag);
        }
        if n <= 1 {
            return left_boundary(mflag);
        }
        ilo = n - 1;
        ihi = n;
    }
    // the search phases end in one of the two bisections (L50 / L51)
    'search: {
        if smlr(xt1(ihi)) {
            if grtr(xt1(ilo)) {
                *mflag = 0;
                return ilo;
            }
            let mut istep = 1;
            loop {
                ihi = ilo;
                ilo = ihi - istep;
                if ilo <= 1 {
                    break;
                }
                if (!lo && x >= xt1(ilo)) || (lo && x > xt1(ilo)) {
                    break 'search;
                }
                istep *= 2;
            }
            ilo = 1;
            if smlr(xt1(1)) {
                return left_boundary(mflag);
            }
        } else {
            let mut istep = 1;
            loop {
                ilo = ihi;
                ihi = ilo + istep;
                if ihi >= n {
                    break;
                }
                if (!lo && x < xt1(ihi)) || (lo && x <= xt1(ihi)) {
                    break 'search;
                }
                istep *= 2;
            }
            if grtr(xt1(n)) {
                return right_boundary(mflag);
            }
            ihi = n;
        }
    }
    loop {
        let middle = (ilo + ihi) / 2;
        if middle == ilo {
            *mflag = 0;
            return ilo;
        }
        let go_right = if lo { x > xt1(middle) } else { x >= xt1(middle) };
        if go_right {
            ilo = middle;
        } else {
            ihi = middle;
        }
    }
}

// R: appl/interv.c:findInterval
#[unsafe(no_mangle)]
pub unsafe extern "C" fn findInterval(
    xt: *const f64,
    n: c_int,
    x: f64,
    rightmost_closed: c_int,
    all_inside: c_int,
    ilo: c_int,
    mflag: *mut c_int,
) -> c_int {
    findInterval2(xt, n, x, rightmost_closed, all_inside, 0, ilo, mflag)
}

// R: appl/interv.c:F77_SUB(interv)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn interv_(
    xt: *const f64,
    n: *const c_int,
    x: *const f64,
    rightmost_closed: *const c_int,
    all_inside: *const c_int,
    ilo: *const c_int,
    mflag: *mut c_int,
) -> c_int {
    findInterval(xt, *n, *x, *rightmost_closed, *all_inside, *ilo, mflag)
}

// R: nmath/d1mach.c
#[unsafe(no_mangle)]
pub extern "C" fn Rf_d1mach(i: c_int) -> f64 {
    match i {
        1 => f64::MIN_POSITIVE,
        2 => f64::MAX,
        3 => 0.5 * f64::EPSILON,
        4 => f64::EPSILON,
        5 => std::f64::consts::LOG10_2,
        _ => 0.0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn d1mach_(i: *const c_int) -> f64 {
    Rf_d1mach(*i)
}

// R: util.c:F77_SUB(rchkusr)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rchkusr_() {
    ret(crate::errors::check_user_interrupt())
}

// R: nmath's caches are robigo-numerics' own: nothing to free
#[unsafe(no_mangle)]
pub extern "C" fn signrank_free() {}

#[unsafe(no_mangle)]
pub extern "C" fn wilcox_free() {}

// R: memory.c:allocLang
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_allocLang(n: c_int) -> Sexp {
    ret(alloc_lang(n))
}

unsafe fn alloc_lang(n: c_int) -> Res<Sexp> {
    if n > 0 {
        let rest = protect(Rf_allocList(n - 1))?;
        let r = Rf_lcons(R_NilValue, rest);
        unprotect(1)?;
        Ok(r)
    } else {
        Ok(R_NilValue)
    }
}

// R: util.c:UNIMPLEMENTED_TYPE
#[unsafe(no_mangle)]
pub unsafe extern "C" fn UNIMPLEMENTED_TYPE(s: *const c_char, x: Sexp) -> ! {
    throw(crate::errors::error_str(&format!(
        "unimplemented type '{}' in '{}'\n",
        crate::util::type_name(x.typ()),
        std::ffi::CStr::from_ptr(s).to_string_lossy()
    )))
}

// R: sort.c's Rf_ name for rPsort
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_rPsort(x: *mut f64, n: c_int, k: c_int) {
    crate::sort::rPsort(x, n, k)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_iPsort(x: *mut c_int, n: c_int, k: c_int) {
    crate::sort::iPsort(x, n, k)
}

// R: Defn.h IS_CACHED (a CHARSXP in the global cache)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn IS_CACHED(x: Sexp) -> c_int {
    (x.gp() & robigo_core::gp::CACHED != 0) as c_int
}

// R: deparse.c:deparse1line / deparse1 — the host's deparser
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_deparse1line(call: Sexp, _abbrev: c_int) -> Sexp {
    ret(crate::host::op("deparse1line\0", &[call]))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_deparse1(call: Sexp, _abbrev: c_int, opts: c_int) -> Sexp {
    ret(deparse1(call, opts))
}

unsafe fn deparse1(call: Sexp, opts: c_int) -> Res<Sexp> {
    let o = protect(crate::list::Rf_ScalarInteger(opts))?;
    let r = crate::host::op("deparse\0", &[call, o])?;
    unprotect(1)?;
    Ok(r)
}

// ---- main/xxxpr.f and print.c: Fortran's printing and error helpers.
// A Fortran CHARACTER argument's length is passed as a hidden trailing
// argument (FC_LEN_T, size_t).

unsafe fn label(label: *const c_char, nchar: c_int, len: usize, what: &str) -> Res<()> {
    let nc = if nchar < 0 { len as c_int } else { nchar };
    if nc > 255 {
        crate::errors::warning_str(&format!("invalid character length in '{what}'"))?;
    } else if nc > 0 {
        let b = std::slice::from_raw_parts(label as *const u8, nc as usize);
        let mut s = b.to_vec();
        s.push(b'\n');
        crate::host::write(0, &s);
    }
    Ok(())
}

unsafe fn print_vec(t: u8, data: *const u8, n: c_int, esz: usize) -> Res<()> {
    if n <= 0 {
        return Ok(());
    }
    let v = protect(try_alloc_vector(t, n as usize)?)?;
    std::ptr::copy_nonoverlapping(data, v.std_data(), n as usize * esz);
    crate::host::op("printValue\0", &[v])?;
    unprotect(1)
}

// R: print.c:printIntegerVector / printRealVector (indx = 1)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_printIntegerVector(x: *const c_int, n: isize, _indx: c_int) {
    ret(print_vec(robigo_core::types::INTSXP, x as *const u8, n as c_int, 4))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_printRealVector(x: *const f64, n: isize, _indx: c_int) {
    ret(print_vec(robigo_core::types::REALSXP, x as *const u8, n as c_int, 8))
}

unsafe fn intpr0(l: *const c_char, nchar: c_int, data: *const c_int, ndata: c_int, len: usize) -> Res<()> {
    label(l, nchar, len, "intpr")?;
    print_vec(robigo_core::types::INTSXP, data as *const u8, ndata, 4)
}

unsafe fn dblep0(l: *const c_char, nchar: c_int, data: *const f64, ndata: c_int, len: usize) -> Res<()> {
    label(l, nchar, len, "dblepr")?;
    print_vec(robigo_core::types::REALSXP, data as *const u8, ndata, 8)
}

unsafe fn realp0(l: *const c_char, nchar: c_int, data: *const f32, ndata: c_int, len: usize) -> Res<()> {
    label(l, nchar, len, "realpr")?;
    let n = ndata.max(0) as usize;
    if n == 0 {
        return Ok(());
    }
    let v = protect(try_alloc_vector(robigo_core::types::REALSXP, n)?)?;
    let d = v.std_data() as *mut f64;
    for k in 0..n {
        *d.add(k) = *data.add(k) as f64;
    }
    crate::host::op("printValue\0", &[v])?;
    unprotect(1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn intpr0_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const c_int,
    ndata: *const c_int,
    len: usize,
) {
    ret(intpr0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dblep0_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const f64,
    ndata: *const c_int,
    len: usize,
) {
    ret(dblep0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realp0_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const f32,
    ndata: *const c_int,
    len: usize,
) {
    ret(realp0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn intpr_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const c_int,
    ndata: *const c_int,
    len: usize,
) {
    ret(intpr0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dblepr_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const f64,
    ndata: *const c_int,
    len: usize,
) {
    ret(dblep0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpr_(
    l: *const c_char,
    nchar: *const c_int,
    data: *const f32,
    ndata: *const c_int,
    len: usize,
) {
    ret(realp0(l, *nchar, data, *ndata, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn intpr1_(l: *const c_char, nchar: *const c_int, var: *const c_int, len: usize) {
    ret(intpr0(l, *nchar, var, 1, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dblepr1_(l: *const c_char, nchar: *const c_int, var: *const f64, len: usize) {
    ret(dblep0(l, *nchar, var, 1, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpr1_(l: *const c_char, nchar: *const c_int, var: *const f32, len: usize) {
    ret(realp0(l, *nchar, var, 1, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn labelpr_(l: *const c_char, nchar: *const c_int, len: usize) {
    ret(label(l, *nchar, len, "intpr"))
}

/// util.c's rexitc/rwarnc: at most 255 characters of the message.
unsafe fn fortran_msg(msg: *const c_char, nchar: c_int, what: &str) -> Res<String> {
    let mut nc = nchar.max(0) as usize;
    if nc > 255 {
        crate::errors::warning_str(&format!("{what} message truncated to 255 chars"))?;
        nc = 255;
    }
    let b = std::slice::from_raw_parts(msg as *const u8, nc);
    let b = b.split(|&c| c == 0).next().unwrap_or(&[]);
    Ok(String::from_utf8_lossy(b).into_owned())
}

unsafe fn rexitc(msg: *const c_char, nchar: c_int) -> crate::errors::Unwind {
    match fortran_msg(msg, nchar, "error") {
        Ok(m) => crate::errors::error_str(&m),
        Err(u) => u,
    }
}

unsafe fn rwarnc(msg: *const c_char, nchar: c_int) -> Res<()> {
    crate::errors::warning_str(&fortran_msg(msg, nchar, "warning")?)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexitc_(msg: *const c_char, nchar: *const c_int, _len: usize) -> ! {
    throw(rexitc(msg, *nchar))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rwarnc_(msg: *const c_char, nchar: *const c_int, _len: usize) {
    ret(rwarnc(msg, *nchar))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rexit_(msg: *const c_char, len: usize) -> ! {
    throw(rexitc(msg, len as c_int))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rwarn_(msg: *const c_char, len: usize) {
    ret(rwarnc(msg, len as c_int))
}
