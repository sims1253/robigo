//! sort.c's API (R_isort, R_rsort, rPsort, revsort, ...) and the
//! unique.c/identical.c entry points, which call the host's ports.

use crate::memory::*;
use crate::state::*;
use robigo_core::types::*;
use robigo_core::{Rcomplex, Sexp};
use std::ffi::c_int;

// R: sort.c:icmp, rcmp, ccmp
fn icmp(x: i32, y: i32, nalast: bool) -> i32 {
    let (na_x, na_y) = (x == i32::MIN, y == i32::MIN);
    if na_x && na_y {
        return 0;
    }
    if na_x {
        return if nalast { 1 } else { -1 };
    }
    if na_y {
        return if nalast { -1 } else { 1 };
    }
    (x > y) as i32 - (x < y) as i32
}

fn rcmp(x: f64, y: f64, nalast: bool) -> i32 {
    let (na_x, na_y) = (x.is_nan(), y.is_nan());
    if na_x && na_y {
        return 0;
    }
    if na_x {
        return if nalast { 1 } else { -1 };
    }
    if na_y {
        return if nalast { -1 } else { 1 };
    }
    (x > y) as i32 - (x < y) as i32
}

fn ccmp(x: Rcomplex, y: Rcomplex, nalast: bool) -> i32 {
    let (na_x, na_y) = (x.r.is_nan(), y.r.is_nan());
    if na_x && na_y {
        return 0;
    }
    if na_x {
        return if nalast { 1 } else { -1 };
    }
    if na_y {
        return if nalast { -1 } else { 1 };
    }
    if x.r < y.r {
        return -1;
    }
    if x.r > y.r {
        return 1;
    }
    let (na_x, na_y) = (x.i.is_nan(), y.i.is_nan());
    if na_x && na_y {
        return 0;
    }
    if na_x {
        return if nalast { 1 } else { -1 };
    }
    if na_y {
        return if nalast { -1 } else { 1 };
    }
    (x.i > y.i) as i32 - (x.i < y.i) as i32
}

// R: sort.c:sort_body (Shell sort, NAs last)
fn shell_sort<T: Copy>(x: &mut [T], cmp: fn(T, T, bool) -> i32) {
    let n = x.len();
    let mut h = 1;
    while h <= n / 9 {
        h = 3 * h + 1;
    }
    while h > 0 {
        for i in h..n {
            let v = x[i];
            let mut j = i;
            while j >= h && cmp(x[j - h], v, true) > 0 {
                x[j] = x[j - h];
                j -= h;
            }
            x[j] = v;
        }
        h /= 3;
    }
}

unsafe fn slice<'a, T>(x: *mut T, n: c_int) -> &'a mut [T] {
    if n <= 0 { &mut [] } else { std::slice::from_raw_parts_mut(x, n as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_isort(x: *mut c_int, n: c_int) {
    shell_sort(slice(x, n), icmp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_rsort(x: *mut f64, n: c_int) {
    shell_sort(slice(x, n), rcmp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_csort(x: *mut Rcomplex, n: c_int) {
    shell_sort(slice(x, n), ccmp)
}

// R: sort.c:rsort_with_index
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsort_with_index(x: *mut f64, indx: *mut c_int, n: c_int) {
    let (x, ix) = (slice(x, n), slice(indx, n));
    let n = x.len();
    let mut h = 1;
    while h <= n / 9 {
        h = 3 * h + 1;
    }
    while h > 0 {
        for i in h..n {
            let (v, iv) = (x[i], ix[i]);
            let mut j = i;
            while j >= h && rcmp(x[j - h], v, true) > 0 {
                x[j] = x[j - h];
                ix[j] = ix[j - h];
                j -= h;
            }
            x[j] = v;
            ix[j] = iv;
        }
        h /= 3;
    }
}

// Utils.h renames sort.c's revsort to Rf_revsort (R's libR exports the
// Rf_ name; Rcpp and other packages call it through R_ext/Utils.h).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_revsort(a: *mut f64, ib: *mut c_int, n: c_int) {
    revsort(a, ib, n)
}

// R: sort.c:revsort — heapsort into descending order, ib alongside
#[unsafe(no_mangle)]
pub unsafe extern "C" fn revsort(a: *mut f64, ib: *mut c_int, n: c_int) {
    if n <= 1 {
        return;
    }
    // 1-based, as the C code
    let a = a.sub(1);
    let ib = ib.sub(1);
    let mut l = ((n >> 1) + 1) as isize;
    let mut ir = n as isize;
    loop {
        let (ra, ii);
        if l > 1 {
            l -= 1;
            ra = *a.offset(l);
            ii = *ib.offset(l);
        } else {
            ra = *a.offset(ir);
            ii = *ib.offset(ir);
            *a.offset(ir) = *a.offset(1);
            *ib.offset(ir) = *ib.offset(1);
            ir -= 1;
            if ir == 1 {
                *a.offset(1) = ra;
                *ib.offset(1) = ii;
                return;
            }
        }
        let mut i = l;
        let mut j = l << 1;
        while j <= ir {
            if j < ir && *a.offset(j) > *a.offset(j + 1) {
                j += 1;
            }
            if ra > *a.offset(j) {
                *a.offset(i) = *a.offset(j);
                *ib.offset(i) = *ib.offset(j);
                i = j;
                j += i;
            } else {
                j = ir + 1;
            }
        }
        *a.offset(i) = ra;
        *ib.offset(i) = ii;
    }
}

// R: sort.c:psort_body
fn psort<T: Copy>(x: &mut [T], k: usize, cmp: fn(T, T, bool) -> i32) {
    if x.is_empty() {
        return;
    }
    let (mut l, mut r) = (0isize, x.len() as isize - 1);
    let k = k as isize;
    while l < r {
        let v = x[k as usize];
        let (mut i, mut j) = (l, r);
        while i <= j {
            while cmp(x[i as usize], v, true) < 0 {
                i += 1;
            }
            while cmp(v, x[j as usize], true) < 0 {
                j -= 1;
            }
            if i <= j {
                x.swap(i as usize, j as usize);
                i += 1;
                j -= 1;
            }
        }
        if j < k {
            l = i;
        }
        if k < i {
            r = j;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn iPsort(x: *mut c_int, n: c_int, k: c_int) {
    psort(slice(x, n), k as usize, icmp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rPsort(x: *mut f64, n: c_int, k: c_int) {
    psort(slice(x, n), k as usize, rcmp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cPsort(x: *mut Rcomplex, n: c_int, k: c_int) {
    psort(slice(x, n), k as usize, ccmp)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_cPsort(x: *mut Rcomplex, n: c_int, k: c_int) {
    cPsort(x, n, k)
}

// R: qsort.c / qsort-body.c (Singleton's CACM #347 quicksort, 1-indexed i..=j)
unsafe fn qsort_body<T: Copy + PartialOrd>(v: *mut T, idx: *mut c_int, mut i: isize, mut j: isize) {
    let mut il = [0isize; 40];
    let mut iu = [0isize; 40];
    let mut r = 0.375f64;
    let v = v.sub(1);
    let has_idx = !idx.is_null();
    let idx = if has_idx { idx.sub(1) } else { idx };
    let ii = i;
    let mut m = 1usize;

    #[derive(PartialEq, Eq)]
    enum Step {
        L10,
        L20,
        L80,
        L100,
    }
    let mut step = Step::L10;
    loop {
        match step {
            Step::L10 => {
                if i < j {
                    if r < 0.5898437 {
                        r += 0.0390625;
                    } else {
                        r -= 0.21875;
                    }
                    step = Step::L20;
                } else {
                    step = Step::L80;
                }
            }
            Step::L20 => {
                let mut k = i;
                let ij = i + ((j - i) as f64 * r) as isize;
                let mut it = if has_idx { *idx.offset(ij) } else { 0 };
                let mut vt = *v.offset(ij);
                if *v.offset(i) > vt {
                    if has_idx {
                        *idx.offset(ij) = *idx.offset(i);
                        *idx.offset(i) = it;
                        it = *idx.offset(ij);
                    }
                    *v.offset(ij) = *v.offset(i);
                    *v.offset(i) = vt;
                    vt = *v.offset(ij);
                }
                let mut l = j;
                if *v.offset(j) < vt {
                    if has_idx {
                        *idx.offset(ij) = *idx.offset(j);
                        *idx.offset(j) = it;
                        it = *idx.offset(ij);
                    }
                    *v.offset(ij) = *v.offset(j);
                    *v.offset(j) = vt;
                    vt = *v.offset(ij);
                    if *v.offset(i) > vt {
                        if has_idx {
                            *idx.offset(ij) = *idx.offset(i);
                            *idx.offset(i) = it;
                        }
                        *v.offset(ij) = *v.offset(i);
                        *v.offset(i) = vt;
                        vt = *v.offset(ij);
                    }
                }
                loop {
                    loop {
                        l -= 1;
                        if !(*v.offset(l) > vt) {
                            break;
                        }
                    }
                    let tt = if has_idx { *idx.offset(l) } else { 0 };
                    let vtt = *v.offset(l);
                    loop {
                        k += 1;
                        if !(*v.offset(k) < vt) {
                            break;
                        }
                    }
                    if k > l {
                        break;
                    }
                    if has_idx {
                        *idx.offset(l) = *idx.offset(k);
                        *idx.offset(k) = tt;
                    }
                    *v.offset(l) = *v.offset(k);
                    *v.offset(k) = vtt;
                }
                m += 1;
                if l - i <= j - k {
                    il[m] = k;
                    iu[m] = j;
                    j = l;
                } else {
                    il[m] = i;
                    iu[m] = l;
                    i = k;
                }
                if j - i > 10 {
                    step = Step::L20;
                } else if i == ii {
                    step = Step::L10;
                } else {
                    i -= 1;
                    step = Step::L100;
                }
            }
            Step::L80 => {
                if m == 1 {
                    return;
                }
                i = il[m];
                j = iu[m];
                m -= 1;
                if j - i > 10 {
                    step = Step::L20;
                } else if i == ii {
                    step = Step::L10;
                } else {
                    i -= 1;
                    step = Step::L100;
                }
            }
            Step::L100 => loop {
                i += 1;
                if i == j {
                    step = Step::L80;
                    break;
                }
                let it = if has_idx { *idx.offset(i + 1) } else { 0 };
                let vt = *v.offset(i + 1);
                if !(*v.offset(i) <= vt) {
                    let mut k = i;
                    loop {
                        if has_idx {
                            *idx.offset(k + 1) = *idx.offset(k);
                        }
                        *v.offset(k + 1) = *v.offset(k);
                        k -= 1;
                        if !(vt < *v.offset(k)) {
                            break;
                        }
                    }
                    if has_idx {
                        *idx.offset(k + 1) = it;
                    }
                    *v.offset(k + 1) = vt;
                    break;
                }
            },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_qsort(v: *mut f64, i: usize, j: usize) {
    qsort_body(v, std::ptr::null_mut(), i as isize, j as isize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_qsort_int(v: *mut c_int, i: usize, j: usize) {
    qsort_body(v, std::ptr::null_mut(), i as isize, j as isize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_qsort_I(v: *mut f64, idx: *mut c_int, i: c_int, j: c_int) {
    qsort_body(v, idx, i as isize, j as isize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_qsort_int_I(v: *mut c_int, idx: *mut c_int, i: c_int, j: c_int) {
    qsort_body(v, idx, i as isize, j as isize)
}

// ---- through the host's .Internal ports ------------------------------------

/// Call the host's `.Internal(name(args...))`.
pub unsafe fn internal(name: &str, args: &[Sexp]) -> Sexp {
    let mut a = Vec::with_capacity(args.len() + 1);
    let n = std::ffi::CString::new(name).unwrap();
    a.push(Rf_protect(crate::strings::Rf_mkString(n.as_ptr())));
    a.extend_from_slice(args);
    let r = crate::host::op("internal\0", &a);
    Rf_unprotect(1);
    r
}

fn lgl(b: bool) -> Sexp {
    unsafe { crate::list::Rf_ScalarLogical(b as c_int) }
}

// R: unique.c:duplicated
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_duplicated(x: Sexp, from_last: c_int) -> Sexp {
    internal("duplicated", &[x, lgl(false), lgl(from_last != 0), crate::list::Rf_ScalarInteger(i32::MIN)])
}

// R: unique.c:any_duplicated (0, or the 1-based index of the first duplicate)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_any_duplicated(x: Sexp, from_last: c_int) -> isize {
    let r = internal("anyDuplicated", &[x, lgl(false), lgl(from_last != 0)]);
    crate::coerce::Rf_asReal(r) as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Rf_any_duplicated3(x: Sexp, incomp: Sexp, from_last: c_int) -> isize {
    let r = internal("anyDuplicated", &[x, incomp, lgl(from_last != 0)]);
    crate::coerce::Rf_asReal(r) as isize
}

// R: identical.c:R_compute_identical — the flag bits of do_identical
#[unsafe(no_mangle)]
pub unsafe extern "C" fn R_compute_identical(x: Sexp, y: Sexp, flags: c_int) -> c_int {
    let r = internal(
        "identical",
        &[
            x,
            y,
            lgl(flags & 1 == 0),
            lgl(flags & 2 == 0),
            lgl(flags & 4 == 0),
            lgl(flags & 8 == 0),
            lgl(flags & 16 != 0),
            lgl(flags & 32 == 0),
            lgl(flags & 64 != 0),
        ],
    );
    (*LOGICAL(r) == 1) as c_int
}

#[allow(dead_code)]
fn _unused() -> Sexp {
    unsafe { R_NilValue }
}
#[allow(dead_code)]
const _T: u8 = INTSXP;
