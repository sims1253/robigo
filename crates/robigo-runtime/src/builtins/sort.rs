//! Sorting, ordering and ranking (GNU R's sort.c, radixsort.c, qsort.c
//! and the `wrap_meta` stub of altclasses.c).
//!
//! The order produced by `order()`/`radixsort` is fully determined by the
//! comparison keys (all of R's algorithms here are stable or break ties
//! by index), so those are implemented as stable sorts on R's keys. The
//! in-place algorithms whose results depend on the algorithm itself
//! (`psort`, `qsort` with ties, Shell sort of values) are ported as is.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::summary::{as_bool2, check_arity_internal};
use super::{internal, prim};
use crate::collate::Scollate;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("sort", do_sort, 2),
    internal("psort", do_psort, 2),
    internal("qsort", do_qsort, 2),
    internal("order", do_order, -1),
    internal("radixsort", do_radixsort, -1),
    internal("rank", do_rank, 3),
    prim("xtfrm", do_xtfrm, 1).arg1("x"),
    internal("is.unsorted", do_isunsorted, -1),
    internal("sorted_fpass", do_sorted_fpass, 3),
    internal("wrap_meta", do_wrap_meta, 3),
];

fn sign(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// ----------------------------------------------------------------------
// Part I: comparison utilities (sort.c)

pub fn icmp(x: i32, y: i32, nalast: bool) -> i32 {
    if x == NA_INTEGER && y == NA_INTEGER {
        return 0;
    }
    if x == NA_INTEGER {
        return if nalast { 1 } else { -1 };
    }
    if y == NA_INTEGER {
        return if nalast { -1 } else { 1 };
    }
    sign(x.cmp(&y))
}

pub fn rcmp(x: f64, y: f64, nalast: bool) -> i32 {
    let (nax, nay) = (x.is_nan(), y.is_nan());
    if nax && nay {
        return 0;
    }
    if nax {
        return if nalast { 1 } else { -1 };
    }
    if nay {
        return if nalast { -1 } else { 1 };
    }
    if x < y {
        -1
    } else if x > y {
        1
    } else {
        0
    }
}

pub fn ccmp(x: Complex, y: Complex, nalast: bool) -> i32 {
    let c = rcmp(x.re, y.re, nalast);
    if c != 0 || (x.re.is_nan() && y.re.is_nan()) {
        return c;
    }
    rcmp(x.im, y.im, nalast)
}

#[inline(always)]
pub fn scmp(x: &RStr, y: &RStr, nalast: bool, coll: &Scollate) -> i32 {
    match (x, y) {
        (None, None) => 0,
        (None, _) => {
            if nalast {
                1
            } else {
                -1
            }
        }
        (_, None) => {
            if nalast {
                -1
            } else {
                1
            }
        }
        (Some(a), Some(b)) => {
            if std::ptr::eq(a.as_ptr(), b.as_ptr()) && a.len() == b.len() {
                0
            } else {
                sign(coll.cmp(a, b))
            }
        }
    }
}

/// R's `isUnsorted(x, strictly)` for atomic vectors.
pub fn is_unsorted(x: &Value, strictly: bool) -> RResult<bool> {
    if !x.is_atomic() {
        return Err(Flow::error("only atomic vectors can be tested to be sorted"));
    }
    let bad = |c: i32| if strictly { c >= 0 } else { c > 0 };
    Ok(match x {
        Value::Lgl(v) | Value::Int(v) => v.data.windows(2).any(|w| bad(sign(w[0].cmp(&w[1])))),
        Value::Real(v) => v.data.windows(2).any(|w| if strictly { w[0] >= w[1] } else { w[0] > w[1] }),
        Value::Cplx(v) => v.data.windows(2).any(|w| bad(ccmp(w[0], w[1], true))),
        Value::Str(v) => {
            let coll = Scollate::new();
            let r = v.data.windows(2).any(|w| bad(scmp(&w[0], &w[1], true, &coll)));
            coll.finish()?;
            r
        }
        Value::Raw(v) => v.data.windows(2).any(|w| if strictly { w[0] >= w[1] } else { w[0] > w[1] }),
        _ => false,
    })
}

// R: sort.c:do_isunsorted
fn do_isunsorted(i: &mut Interp, call: &Value, mut args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.len() == 2 {
        args.insert(1, Arg::pos(Value::lgl(false)));
    }
    check_arity_internal(&args, 3, "is.unsorted", call)?;
    if let Some(r) = i.try_dispatch("is.unsorted", call, &args, env)? {
        return Ok(r);
    }
    let x = &args[0].value;
    let strictly_arg = &args[2].value;
    // INTEGER_IS_SORTED / REAL_IS_SORTED: only an ALTREP class knows
    if let (Value::Int(_) | Value::Real(_), Some(h)) = (x, crate::altrep::lookup(x)) {
        let sorted = crate::altrep::sorted(i, h, false)?.unwrap_or(i32::MIN);
        // not strict since we don't memoize that
        if crate::coerce::as_logical(strictly_arg) == 0 {
            match sorted {
                // KNOWN_INCR
                1 | 2 => return Ok(Value::lgl(false)),
                // KNOWN_DECR: TRUE iff more than one unique value (SORTED_VEC_NONCONST)
                -1 | -2 => {
                    let n = x.len();
                    // R: sort.c:SORTED_VEC_NONCONST uses the class's ELT methods.
                    let nonconst = if n > 1 {
                        let first = crate::altrep::elt(i, h, 0)?;
                        let last = crate::altrep::elt(i, h, n - 1)?;
                        match (&first, &last) {
                            (Value::Int(a), Value::Int(b)) => a.data[0] != b.data[0],
                            (Value::Real(a), Value::Real(b)) => a.data[0] != b.data[0],
                            _ => false,
                        }
                    } else {
                        false
                    };
                    return Ok(Value::lgl(nonconst));
                }
                // KNOWN_UNSORTED
                0 => return Ok(Value::lgl(true)),
                _ => {}
            }
        }
    }
    let strictly = crate::coerce::as_logical(strictly_arg);
    if strictly == NA_LOGICAL {
        return Err(Flow::error("invalid 'strictly' argument"));
    }
    if x.is_atomic() {
        if x.len() < 2 {
            return Ok(Value::lgl(false));
        }
        return Ok(Value::lgl(is_unsorted(x, strictly != 0)?));
    }
    if x.is_object() {
        let f = i.find_fun(Sym::new(".gtn"), env)?;
        return i.call_value(&f, vec![Arg::pos(x.clone()), Arg::pos(strictly_arg.clone())], env);
    }
    Ok(Value::lgl_na())
}

// ----------------------------------------------------------------------
// Part II: complete sorting

const INCS: [usize; 21] = [
    274878693377,
    68719869953,
    17180065793,
    4295065601,
    1073790977,
    268460033,
    67121153,
    16783361,
    4197377,
    1050113,
    262913,
    65921,
    16577,
    4193,
    1073,
    281,
    77,
    23,
    8,
    1,
    0,
];

/// `sort2_body`: Shell sort moving `x[j-h]` while `before(x[j-h], v)`.
fn shell_sort2<T: Clone>(x: &mut [T], moves: impl Fn(&T, &T) -> bool) {
    let n = x.len();
    let mut t = 0;
    while INCS[t] > n {
        t += 1;
    }
    while t < 20 {
        let h = INCS[t];
        for i in h..n {
            let v = x[i].clone();
            let mut j = i;
            while j >= h && moves(&x[j - h], &v) {
                x[j] = x[j - h].clone();
                j -= h;
            }
            x[j] = v;
        }
        t += 1;
    }
}

/// `sortVector(s, decreasing)`.
pub fn sort_vector(s: &mut Value, decreasing: bool) -> RResult<()> {
    let n = s.len();
    if n < 2 || (!decreasing && !is_unsorted(s, false)?) {
        return Ok(());
    }
    match s {
        Value::Lgl(v) | Value::Int(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            if decreasing { shell_sort2(d, |a, b| a < b) } else { shell_sort2(d, |a, b| a > b) }
        }
        Value::Real(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            if decreasing { shell_sort2(d, |a, b| a < b) } else { shell_sort2(d, |a, b| a > b) }
        }
        Value::Cplx(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            if decreasing {
                shell_sort2(d, |a, b| a.re < b.re || (a.re == b.re && a.im < b.im))
            } else {
                shell_sort2(d, |a, b| a.re > b.re || (a.re == b.re && a.im > b.im))
            }
        }
        Value::Str(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            let coll = Scollate::new();
            if decreasing {
                shell_sort2(d, |a, b| scmp(a, b, true, &coll) < 0)
            } else {
                shell_sort2(d, |a, b| scmp(a, b, true, &coll) > 0)
            }
            coll.finish()?;
        }
        other => {
            return Err(Flow::error(format!("unimplemented type '{}' in 'sortVector'\n", other.type_name())));
        }
    }
    Ok(())
}

// R: sort.c:do_sort
fn do_sort(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "sort", call)?;
    let decreasing = crate::coerce::as_logical(&args[1].value);
    if decreasing == NA_LOGICAL {
        return Err(Flow::error("'decreasing' must be TRUE or FALSE"));
    }
    let x = &args[0].value;
    if x.is_null() {
        return Ok(Value::Null);
    }
    if !x.is_atomic() {
        return Err(Flow::error("only atomic vectors can be sorted"));
    }
    if matches!(x, Value::Raw(_)) {
        return Err(Flow::error("raw vectors cannot be sorted"));
    }
    let mut ans = x.clone();
    ans.clear_attrs();
    sort_vector(&mut ans, decreasing != 0)?;
    Ok(ans)
}

// ----------------------------------------------------------------------
// Part III: partial sorting

/// `psort_body`: partial sort so that `x[k]` is in place.
fn psort2<T: Clone>(x: &mut [T], lo: usize, hi: usize, k: usize, cmp: &impl Fn(&T, &T) -> i32) {
    let (mut l, mut r) = (lo as isize, hi as isize);
    let k = k as isize;
    while l < r {
        let v = x[k as usize].clone();
        let (mut i, mut j) = (l, r);
        while i <= j {
            while cmp(&x[i as usize], &v) < 0 {
                i += 1;
            }
            while cmp(&v, &x[j as usize]) < 0 {
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

/// `Psort0`: `ind` are 1-based.
fn psort0<T: Clone>(x: &mut [T], lo: isize, hi: isize, ind: &[usize], cmp: &impl Fn(&T, &T) -> i32) {
    if ind.is_empty() || hi - lo < 1 {
        return;
    }
    if ind.len() <= 1 {
        psort2(x, lo as usize, hi as usize, ind[0] - 1, cmp);
    } else {
        let mid = (lo + hi) / 2;
        let mut this = 0;
        for (k, &v) in ind.iter().enumerate() {
            if v as isize - 1 <= mid {
                this = k;
            }
        }
        let z = ind[this] as isize - 1;
        psort2(x, lo as usize, hi as usize, z as usize, cmp);
        psort0(x, lo, z - 1, &ind[..this], cmp);
        psort0(x, z + 1, hi, &ind[this + 1..], cmp);
    }
}

// R: sort.c:do_psort
fn do_psort(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "psort", call)?;
    let x = &args[0].value;
    if !x.is_atomic() {
        return Err(Flow::error("only atomic vectors can be sorted"));
    }
    if matches!(x, Value::Raw(_)) {
        return Err(Flow::error("raw vectors cannot be sorted"));
    }
    let n = x.len();
    let p = crate::coerce::coerce_quiet(&args[1].value, VType::Int);
    let pv: Vec<i32> = match &p {
        Value::Int(v) => v.data.clone(),
        _ => vec![],
    };
    let mut l = Vec::with_capacity(pv.len());
    for &k in &pv {
        if k == NA_INTEGER {
            return Err(Flow::error("NA index"));
        }
        if k < 1 || k as usize > n {
            return Err(Flow::error(format!("'partial' index {} outside bounds", k)));
        }
        l.push(k as usize);
    }
    let mut ans = x.clone();
    ans.clear_attrs();
    let hi = n as isize - 1;
    match &mut ans {
        Value::Lgl(v) | Value::Int(v) => {
            psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &i32, b: &i32| icmp(*a, *b, true))
        }
        Value::Real(v) => psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &f64, b: &f64| rcmp(*a, *b, true)),
        Value::Cplx(v) => {
            psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &Complex, b: &Complex| ccmp(*a, *b, true))
        }
        Value::Str(v) => {
            let coll = Scollate::new();
            psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &RStr, b: &RStr| scmp(a, b, true, &coll));
            coll.finish()?;
        }
        _ => {}
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// qsort.c

/// Port of `qsort-body.c` (Singleton's quicksort, ACM algorithm 347) on
/// a 1-based view of `v`; sorts `idx` alongside when given.
#[allow(unused_assignments)]
fn r_qsort<T: Copy + PartialOrd>(v: &mut [T], mut idx: Option<&mut [i32]>) {
    let n = v.len() as isize;
    if n == 0 {
        return;
    }
    // 1-based accessors
    macro_rules! V {
        ($k:expr) => {
            v[($k - 1) as usize]
        };
    }
    macro_rules! I {
        ($k:expr) => {
            idx.as_mut().unwrap()[($k - 1) as usize]
        };
    }
    let has_idx = idx.is_some();
    let mut il = [0isize; 40];
    let mut iu = [0isize; 40];
    let mut r = 0.375f64;
    let (mut i, mut j): (isize, isize) = (1, n);
    let ii = i;
    let mut m: usize = 1;
    let (mut k, mut l, mut ij): (isize, isize, isize) = (0, 0, 0);
    let mut vt: T = v[0];
    let mut vtt: T;
    let mut it: i32 = 0;
    let mut tt: i32 = 0;

    #[derive(PartialEq)]
    enum L {
        L10,
        L20,
        L80,
        Tail,
        L100,
    }
    let mut state = L::L10;
    loop {
        match state {
            L::L10 => {
                if i < j {
                    if r < 0.5898437 {
                        r += 0.0390625;
                    } else {
                        r -= 0.21875;
                    }
                    state = L::L20;
                } else {
                    state = L::L80;
                }
            }
            L::L20 => {
                k = i;
                ij = i + ((j - i) as f64 * r) as isize;
                if has_idx {
                    it = I!(ij);
                }
                vt = V!(ij);
                if V!(i) > vt {
                    if has_idx {
                        I!(ij) = I!(i);
                        I!(i) = it;
                        it = I!(ij);
                    }
                    V!(ij) = V!(i);
                    V!(i) = vt;
                    vt = V!(ij);
                }
                l = j;
                if V!(j) < vt {
                    if has_idx {
                        I!(ij) = I!(j);
                        I!(j) = it;
                        it = I!(ij);
                    }
                    V!(ij) = V!(j);
                    V!(j) = vt;
                    vt = V!(ij);
                    if V!(i) > vt {
                        if has_idx {
                            I!(ij) = I!(i);
                            I!(i) = it;
                            it = I!(ij);
                        }
                        V!(ij) = V!(i);
                        V!(i) = vt;
                        vt = V!(ij);
                    }
                }
                loop {
                    loop {
                        l -= 1;
                        if !(V!(l) > vt) {
                            break;
                        }
                    }
                    if has_idx {
                        tt = I!(l);
                    }
                    vtt = V!(l);
                    loop {
                        k += 1;
                        if !(V!(k) < vt) {
                            break;
                        }
                    }
                    if k > l {
                        break;
                    }
                    if has_idx {
                        I!(l) = I!(k);
                        I!(k) = tt;
                    }
                    V!(l) = V!(k);
                    V!(k) = vtt;
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
                state = L::Tail;
            }
            L::L80 => {
                if m == 1 {
                    return;
                }
                i = il[m];
                j = iu[m];
                m -= 1;
                state = L::Tail;
            }
            L::Tail => {
                if j - i > 10 {
                    state = L::L20;
                } else if i == ii {
                    state = L::L10;
                } else {
                    i -= 1;
                    state = L::L100;
                }
            }
            L::L100 => {
                // do { ++i; if (i == j) goto L80; it = I[i+1]; vt = v[i+1]; } while (v[i] <= vt);
                let mut to_l80 = false;
                loop {
                    i += 1;
                    if i == j {
                        to_l80 = true;
                        break;
                    }
                    if has_idx {
                        it = I!(i + 1);
                    }
                    vt = V!(i + 1);
                    if !(V!(i) <= vt) {
                        break;
                    }
                }
                if to_l80 {
                    state = L::L80;
                    continue;
                }
                vt = V!(i + 1);
                k = i;
                loop {
                    if has_idx {
                        I!(k + 1) = I!(k);
                    }
                    V!(k + 1) = V!(k);
                    k -= 1;
                    if !(vt < V!(k)) {
                        break;
                    }
                }
                if has_idx {
                    I!(k + 1) = it;
                }
                V!(k + 1) = vt;
                state = L::L100;
            }
        }
    }
}

// R: qsort.c:do_qsort
fn do_qsort(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "qsort", call)?;
    let x = &args[0].value;
    if !matches!(x, Value::Lgl(_) | Value::Int(_) | Value::Real(_)) || x.inherits("factor") {
        return Err(Flow::error("argument is not a numeric vector"));
    }
    let mut sx = x.clone();
    sx.clear_attrs();
    let indx_ret = crate::coerce::as_logical(&args[1].value);
    let n = sx.len();
    let mut ix: Vec<i32> = (1..=n as i32).collect();
    match &mut sx {
        Value::Real(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            r_qsort(d, if indx_ret != 0 { Some(&mut ix) } else { None });
        }
        Value::Int(v) | Value::Lgl(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            r_qsort(d, if indx_ret != 0 { Some(&mut ix) } else { None });
        }
        _ => {}
    }
    if indx_ret != 0 {
        return Ok(Value::named_list(vec![("x", sx), ("ix", Value::ints(ix))]));
    }
    Ok(sx)
}

// ----------------------------------------------------------------------
// Part IV: rank and order

/// Stable merge sort of indices with a fallible comparator.
fn stable_sort_by<F>(v: &mut [usize], cmp: &mut F) -> RResult<()>
where
    F: FnMut(usize, usize) -> RResult<Ordering>,
{
    let n = v.len();
    if n < 2 {
        return Ok(());
    }
    let mid = n / 2;
    stable_sort_by(&mut v[..mid], cmp)?;
    stable_sort_by(&mut v[mid..], cmp)?;
    let left = v[..mid].to_vec();
    let right = v[mid..].to_vec();
    let (mut a, mut b, mut k) = (0, 0, 0);
    while a < left.len() && b < right.len() {
        if cmp(right[b], left[a])? == Ordering::Less {
            v[k] = right[b];
            b += 1;
        } else {
            v[k] = left[a];
            a += 1;
        }
        k += 1;
    }
    while a < left.len() {
        v[k] = left[a];
        a += 1;
        k += 1;
    }
    while b < right.len() {
        v[k] = right[b];
        b += 1;
        k += 1;
    }
    Ok(())
}

const SINCS: [usize; 17] = [
    1073790977, 268460033, 67121153, 16783361, 4197377, 1050113, 262913, 65921, 16577, 4193, 1073, 281, 77, 23, 8, 1, 0,
];

/// `sort2_with_index` of orderVector1 (lo = 0, hi = n - 1).
fn shell_sort_index<F>(indx: &mut [usize], less: &mut F) -> RResult<()>
where
    F: FnMut(usize, usize) -> RResult<bool>,
{
    let n = indx.len();
    let mut t = 0;
    while SINCS[t] > n {
        t += 1;
    }
    while t < 16 {
        let h = SINCS[t];
        for i in h..n {
            let itmp = indx[i];
            let mut j = i;
            while j >= h && less(indx[j - h], itmp)? {
                indx[j] = indx[j - h];
                j -= h;
            }
            indx[j] = itmp;
        }
        t += 1;
    }
    Ok(())
}

fn is_na_at(x: &Value, k: usize) -> bool {
    match x {
        Value::Lgl(v) | Value::Int(v) => v.data[k] == NA_INTEGER,
        Value::Real(v) => v.data[k].is_nan(),
        Value::Cplx(v) => v.data[k].re.is_nan() || v.data[k].im.is_nan(),
        Value::Str(v) => v.data[k].is_none(),
        _ => false,
    }
}

/// Compare elements `a` and `b` of `x` as sort.c's `*cmp` do.
fn elt_cmp(x: &Value, a: usize, b: usize, nalast: bool, coll: &Scollate) -> RResult<i32> {
    Ok(match x {
        Value::Lgl(v) | Value::Int(v) => icmp(v.data[a], v.data[b], nalast),
        Value::Real(v) => rcmp(v.data[a], v.data[b], nalast),
        Value::Cplx(v) => ccmp(v.data[a], v.data[b], nalast),
        Value::Str(v) => scmp(&v.data[a], &v.data[b], nalast, coll),
        other => {
            return Err(Flow::error(format!("unimplemented type '{}' in 'listgreater'\n", other.type_name())));
        }
    })
}

/// `.gt(x, i, j)` callback for classed objects.
fn gt_callback(i: &mut Interp, x: &Value, a: usize, b: usize, env: &Env) -> RResult<i32> {
    let f = i.find_fun(Sym::new(".gt"), env)?;
    let r = i.call_value(
        &f,
        vec![Arg::pos(x.clone()), Arg::pos(Value::int(a as i32 + 1)), Arg::pos(Value::int(b as i32 + 1))],
        env,
    )?;
    Ok(crate::coerce::as_integer(&r))
}

/// The order of two elements of which at least one is NA: NAs go to one
/// end, whatever the direction.
fn na_order(na_a: bool, na_b: bool, nalast: bool) -> Ordering {
    match (na_a, na_b) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nalast {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        _ => {
            if nalast {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    }
}

/// A sort key compared as [`elt_cmp`] would, with strings replaced by
/// their dense ranks in collation order: strings that collate equal share a
/// rank, so comparing ranks is comparing strings, without collating for
/// each of the n log n comparisons.
enum KeyCol<'a> {
    Int(&'a [i32]),
    Real(&'a [f64]),
    Cplx(&'a [Complex]),
    Rank(Vec<i32>),
}

impl<'a> KeyCol<'a> {
    fn new(x: &'a Value) -> Option<RResult<Self>> {
        Some(Ok(match x {
            Value::Lgl(v) | Value::Int(v) => KeyCol::Int(&v.data),
            Value::Real(v) => KeyCol::Real(&v.data),
            Value::Cplx(v) => KeyCol::Cplx(&v.data),
            Value::Str(v) => match collation_ranks(&v.data) {
                Ok(r) => KeyCol::Rank(r),
                Err(e) => return Some(Err(e)),
            },
            _ => return None,
        }))
    }

    #[inline]
    fn cmp(&self, a: usize, b: usize, nalast: bool) -> i32 {
        match self {
            KeyCol::Int(v) => icmp(v[a], v[b], nalast),
            KeyCol::Real(v) => rcmp(v[a], v[b], nalast),
            KeyCol::Cplx(v) => ccmp(v[a], v[b], nalast),
            // NA ranks are NA_INTEGER: icmp places them as scmp does.
            KeyCol::Rank(r) => icmp(r[a], r[b], nalast),
        }
    }
}

/// The rank of each string among the distinct strings of `x` in collation
/// order (0, 1, ...; `NA_INTEGER` for NA).
fn collation_ranks(x: &[RStr]) -> RResult<Vec<i32>> {
    let coll = Scollate::new();
    let r = string_ranks(x, |a, b| coll.cmp(a, b));
    coll.finish()?;
    Ok(r)
}

// R: sort.c:do_order
fn do_order(_i: &mut Interp, _call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    let nalast = crate::coerce::as_logical(&super::arg(&args, 0));
    if nalast == NA_LOGICAL {
        return Err(Flow::error("invalid 'na.last' value"));
    }
    let nalast = nalast != 0;
    let decreasing = crate::coerce::as_logical(&super::arg(&args, 1));
    if decreasing == NA_LOGICAL {
        return Err(Flow::error("'decreasing' must be TRUE or FALSE"));
    }
    let decreasing = decreasing != 0;
    let keys: Vec<Value> = args.iter().skip(2).map(|a| a.value.clone()).collect();
    if keys.is_empty() {
        return Ok(Value::Null);
    }
    let n = if keys[0].is_vector() { keys[0].len() as isize } else { -1 };
    for (k, x) in keys.iter().enumerate() {
        if !x.is_vector() {
            return Err(Flow::error(format!("argument {} is not a vector", k + 1)));
        }
        if x.len() as isize != n {
            return Err(Flow::error("argument lengths differ"));
        }
    }
    let n = n as usize;
    if n == 0 {
        return Ok(Value::ints(vec![]));
    }
    // Integer, logical, double and string keys (strings by their collation
    // ranks): orderVector1/listgreater compare R's keys with NAs last when
    // `nalast` whatever `decreasing`, exactly radixsort's keys, so the
    // stable radix order is this order. A column is ranked when the radix
    // sort reaches it, as R collates a later key only for ties in the
    // earlier ones.
    if keys.iter().all(|x| matches!(x, Value::Int(_) | Value::Lgl(_) | Value::Real(_) | Value::Str(_))) {
        let order = if decreasing { -1 } else { 1 };
        let (o, _) = radix_order(n, keys.len(), if nalast { 1 } else { -1 }, false, |c| {
            let col = match &keys[c] {
                Value::Int(v) | Value::Lgl(v) => RadixCol::Int(&v.data),
                Value::Real(v) => RadixCol::Real(&v.data),
                Value::Str(v) => RadixCol::Rank(collation_ranks(&v.data)?),
                _ => RadixCol::Int(&[]),
            };
            Ok((col, order))
        })?;
        return Ok(Value::ints(radix_result(o)));
    }
    let mut o: Vec<usize> = (0..n).collect();
    // Keys of the comparable types, strings replaced by their collation
    // ranks: no comparison can fail, so any stable sort gives the same
    // permutation as the general path below.
    if let Some(cols) = keys.iter().map(KeyCol::new).collect::<Option<Vec<_>>>() {
        let cols = cols.into_iter().collect::<RResult<Vec<_>>>()?;
        if let [col] = &cols[..] {
            let key = &keys[0];
            o.sort_by(|&a, &b| match (is_na_at(key, a), is_na_at(key, b)) {
                (false, false) => {
                    let c = col.cmp(a, b, nalast);
                    (if decreasing { -c } else { c }).cmp(&0)
                }
                (na_a, na_b) => na_order(na_a, na_b, nalast),
            });
        } else {
            o.sort_by(|&a, &b| {
                for col in &cols {
                    let c = col.cmp(a, b, nalast ^ decreasing);
                    let c = if decreasing { -c } else { c };
                    if c != 0 {
                        return c.cmp(&0);
                    }
                }
                Ordering::Equal
            });
        }
        return Ok(Value::ints(o.into_iter().map(|k| k as i32 + 1).collect()));
    }
    if keys.len() == 1 {
        let key = &keys[0];
        if !matches!(key, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Str(_) | Value::Cplx(_)) {
            return Err(Flow::error(format!("unimplemented type '{}' in 'orderVector1'\n", key.type_name())));
        }
        // NAs to one end, then the rest by value; ties by index.
        let coll = Scollate::new();
        let mut cmp = |a: usize, b: usize| -> RResult<Ordering> {
            let (na_a, na_b) = (is_na_at(key, a), is_na_at(key, b));
            if na_a || na_b {
                return Ok(match (na_a, na_b) {
                    (true, true) => Ordering::Equal,
                    (true, false) => {
                        if nalast {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        }
                    }
                    _ => {
                        if nalast {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        }
                    }
                });
            }
            let c = elt_cmp(key, a, b, nalast, &coll)?;
            let c = if decreasing { -c } else { c };
            Ok(c.cmp(&0))
        };
        stable_sort_by(&mut o, &mut cmp)?;
        coll.finish()?;
    } else {
        let coll = Scollate::new();
        let mut cmp = |a: usize, b: usize| -> RResult<Ordering> {
            for key in &keys {
                let c = elt_cmp(key, a, b, nalast ^ decreasing, &coll)?;
                let c = if decreasing { -c } else { c };
                if c != 0 {
                    return Ok(c.cmp(&0));
                }
            }
            Ok(Ordering::Equal)
        };
        stable_sort_by(&mut o, &mut cmp)?;
        coll.finish()?;
    }
    Ok(Value::ints(o.into_iter().map(|k| k as i32 + 1).collect()))
}

// R: sort.c:do_rank
fn do_rank(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "rank", call)?;
    let x = args[0].value.clone();
    if matches!(x, Value::Raw(_)) && !x.is_object() {
        return Err(Flow::error("raw vectors cannot be sorted"));
    }
    let sn = &args[1].value;
    let n: usize = if let Value::Real(v) = sn {
        let d = v.data.first().copied().unwrap_or(NA_REAL);
        if d.is_nan() {
            return Err(Flow::error("vector size cannot be NA/NaN"));
        }
        if !d.is_finite() {
            return Err(Flow::error("vector size cannot be infinite"));
        }
        if d > crate::coerce::R_XLEN_T_MAX {
            return Err(Flow::error("vector size specified is too large"));
        }
        if d < 0.0 {
            return Err(Flow::error("invalid 'length(xx)' value"));
        }
        d as usize
    } else {
        let nn = crate::coerce::as_integer(sn);
        if nn == NA_INTEGER || nn < 0 {
            return Err(Flow::error("invalid 'length(xx)' value"));
        }
        nn as usize
    };
    let ties = crate::coerce::as_string(&args[2].value);
    let ties = ties.as_deref().unwrap_or("NA");
    #[derive(PartialEq)]
    enum Ties {
        Average,
        Max,
        Min,
    }
    let kind = match ties {
        "average" => Ties::Average,
        "max" => Ties::Max,
        "min" => Ties::Min,
        _ => return Err(Flow::error("invalid ties.method for rank() [should never happen]")),
    };
    let mut ik = vec![0i32; n];
    let mut rk = vec![0f64; n];
    if n > 0 {
        let obj = x.is_object();
        let coll = Scollate::new();
        let mut o: Vec<usize> = (0..n).collect();
        {
            // orderVector1() with rho: raw comparisons (NAs were removed by
            // rank()), or the .gt() callback for classed objects.
            let mut less = |a: usize, b: usize| -> RResult<bool> {
                if obj {
                    let c = gt_callback(i, &x, a, b, env)?;
                    return Ok(c > 0 || (c == 0 && b < a));
                }
                Ok(match &x {
                    Value::Lgl(v) | Value::Int(v) => {
                        let (p, q) = (v.data[a], v.data[b]);
                        p > q || (p == q && a > b)
                    }
                    Value::Real(v) => {
                        let (p, q) = (v.data[a], v.data[b]);
                        p > q || (p == q && a > b)
                    }
                    Value::Cplx(v) => {
                        let (p, q) = (v.data[a], v.data[b]);
                        ccmp(p, q, false) > 0 || (p.re == q.re && p.im == q.im && a > b)
                    }
                    Value::Str(v) => {
                        let c = coll.cmp(v.data[a].as_deref().unwrap_or("NA"), v.data[b].as_deref().unwrap_or("NA"));
                        c == Ordering::Greater || (c == Ordering::Equal && a > b)
                    }
                    other => {
                        return Err(Flow::error(format!("unimplemented type '{}' in 'greater'\n", other.type_name())));
                    }
                })
            };
            shell_sort_index(&mut o, &mut less)?;
        }
        let mut k0 = 0;
        while k0 < n {
            let mut j = k0;
            while j < n - 1 {
                let c = if obj {
                    gt_callback(i, &x, o[j], o[j + 1], env)?
                } else {
                    elt_cmp(&x, o[j], o[j + 1], true, &coll)?
                };
                if c != 0 {
                    break;
                }
                j += 1;
            }
            for &ok in &o[k0..=j] {
                match kind {
                    Ties::Average => rk[ok] = (k0 as f64 + j as f64 + 2.0) / 2.0,
                    Ties::Max => ik[ok] = j as i32 + 1,
                    Ties::Min => ik[ok] = k0 as i32 + 1,
                }
            }
            k0 = j + 1;
        }
        coll.finish()?;
    }
    Ok(if kind == Ties::Average { Value::reals(rk) } else { Value::ints(ik) })
}

// R: sort.c:do_xtfrm
fn do_xtfrm(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if let Some(r) = i.try_dispatch("xtfrm", call, &args, env)? {
        return Ok(r);
    }
    let f = i.find_fun(Sym::new("xtfrm.default"), env)?;
    let pargs = i.forced_promargs(call, &args, env);
    match &f {
        Value::Closure(c) => i.apply_closure(call, c, pargs, env, None),
        _ => i.apply_function(&f, pargs, env, call),
    }
}

// ----------------------------------------------------------------------
// sorted_fpass, wrap_meta (ALTREP metadata; robigo has no ALTREP wrappers)

// R: sort.c:do_sorted_fpass
fn do_sorted_fpass(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "sorted_fpass", call)?;
    let decr = crate::coerce::as_integer(&args[1].value);
    let nalast = crate::coerce::as_integer(&args[2].value);
    if decr == NA_INTEGER {
        return Ok(Value::lgl(false));
    }
    let nalast = if nalast == NA_INTEGER { 1 } else { nalast };
    let wanted = if decr != 0 {
        if nalast != 0 { -1 } else { -2 }
    } else if nalast != 0 {
        1
    } else {
        2
    };
    // Non-ALTREP vectors have unknown sortedness; only the scan for
    // increasing integer sequences applies.
    if let Value::Int(v) = &args[0].value {
        if wanted > 0 && !v.data.is_empty() {
            let mut last = v.data[0];
            if last != NA_INTEGER {
                for &next in &v.data[1..] {
                    if next < last || next == NA_INTEGER {
                        return Ok(Value::lgl(false));
                    }
                    last = next;
                }
                return Ok(Value::lgl(true));
            }
        }
    }
    Ok(Value::lgl(false))
}

// R: altclasses.c:do_wrap_meta
fn do_wrap_meta(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "wrap_meta", call)?;
    let x = args[0].value.clone();
    if !matches!(
        x,
        Value::Int(_)
            | Value::Real(_)
            | Value::Lgl(_)
            | Value::Cplx(_)
            | Value::Raw(_)
            | Value::Str(_)
            | Value::List(_)
    ) {
        return Ok(x);
    }
    if x.has_attrs() {
        return Ok(x);
    }
    let srt = crate::coerce::as_integer(&args[1].value);
    let no_na = crate::coerce::as_integer(&args[2].value);
    if srt != NA_INTEGER && !(-2..=2).contains(&srt) {
        return Err(Flow::error("srt must be -2, -1, 0, or +1, +2, or NA"));
    }
    if !(0..=1).contains(&no_na) {
        return Err(Flow::error("no_na must be 0 or +1"));
    }
    // The wrapper only carries metadata; its value is `x`.
    Ok(x)
}

// ----------------------------------------------------------------------
// radixsort.c
//
// The order is fully determined by the keys (radixsort.c's twiddles), so
// any stable sort of them gives GNU R's permutation; groups are the runs
// of equal keys. The rows of a group being sorted are kept in a `u32`
// order vector, the keys of the group in one scratch buffer, and the
// sort itself permutes only positions: a counting sort for a small key
// range (icount), packed key/position words for a range that fits 32
// bits (iradix), and an MSD radix with insertion sort of small buckets
// for wider double keys (dradix).

/// `dtwiddle`: the sortable unsigned key of a double.
#[inline]
fn dtwiddle(x: f64, order: i32, nalast: i32, dround: u32) -> u64 {
    let d = order as f64 * x;
    let mut u = d.to_bits();
    if d.is_finite() {
        let dmask1: u64 = if dround > 0 { 1 << (8 * dround - 1) } else { 0 };
        u = if d != 0.0 { u.wrapping_add((u & dmask1) << 1) } else { 0 };
    } else if d.is_nan() {
        return if nalast == 1 { !0u64 } else { 0 };
    }
    let mask = if u & 0x8000_0000_0000_0000 != 0 { !0u64 } else { 0x8000_0000_0000_0000 };
    let dmask2: u64 = if dround >= 8 { 0 } else { !0u64 << (dround * 8) };
    (u ^ mask) & dmask2
}

/// `icheck`, as an order-preserving unsigned key (`icheck` of every
/// value fits an `int`). With `nalast == 0` exactly the NAs have key 0,
/// as `dtwiddle` gives NaN.
#[inline]
fn ikey(x: i32, order: i32, nalast: i32) -> u64 {
    let v = if nalast != 1 {
        if x != NA_INTEGER { x * order } else { x }
    } else if x != NA_INTEGER {
        x * order - 1
    } else {
        i32::MAX
    };
    ((v as u32) ^ 0x8000_0000) as u64
}

/// `StrCmp2`: byte order honouring `order` and `nalast`.
fn str_cmp2(x: &RStr, y: &RStr, order: i32, nalast: i32) -> i32 {
    match (x, y) {
        (None, None) => 0,
        (None, _) => nalast,
        (_, None) => -nalast,
        (Some(a), Some(b)) => order * sign(a.as_bytes().cmp(b.as_bytes())),
    }
}

/// A sort column of radixsort: values and how they are keyed.
enum RadixCol<'a> {
    Int(&'a [i32]),
    Real(&'a [f64]),
    /// Strings as their ranks among the distinct strings (`csort_pre`):
    /// sorted as integers.
    Rank(Vec<i32>),
    /// Strings grouped by first appearance (`cgroup`, `sortStr = FALSE`).
    Group(&'a [RStr]),
}

/// Working memory reused across the groups and columns of one sort
/// (radixsort.c's xsub/newo/otmp/xtmp).
#[derive(Default)]
struct RadixScratch {
    /// The keys of the group being sorted, in its current order.
    keys: Vec<u64>,
    /// The sorted positions within the group.
    perm: Vec<u32>,
    /// The group's rows before reordering.
    otmp: Vec<u32>,
    bufs: SortBufs,
}

#[derive(Default)]
struct SortBufs {
    count: Vec<u32>,
    packed: Vec<u64>,
    packed2: Vec<u64>,
    mkeys: Vec<u64>,
    mkeys2: Vec<u64>,
    mpos: Vec<u32>,
    mpos2: Vec<u32>,
}

/// What one pass over a group's keys tells about them.
struct KeyStats {
    lo: u64,
    hi: u64,
    /// Bits in which some key differs from the first one.
    diff: u64,
    /// Non-decreasing.
    asc: bool,
    /// Strictly decreasing.
    desc: bool,
}

fn key_stats(k: &[u64]) -> KeyStats {
    let first = k.first().copied().unwrap_or(0);
    let (mut lo, mut hi, mut diff) = (u64::MAX, 0u64, 0u64);
    for &x in k {
        lo = lo.min(x);
        hi = hi.max(x);
        diff |= x ^ first;
    }
    let (mut asc, mut desc) = (true, k.len() >= 2);
    for w in k.windows(2) {
        asc &= w[1] >= w[0];
        desc &= w[1] < w[0];
        if !(asc || desc) {
            break;
        }
    }
    KeyStats { lo, hi, diff, asc, desc }
}

/// Push the lengths of the runs of equal adjacent keys.
fn push_runs(runs: &mut Vec<u32>, mut keys: impl Iterator<Item = u64>) {
    let Some(mut prev) = keys.next() else { return };
    let mut tt = 1u32;
    for k in keys {
        if k == prev {
            tt += 1;
        } else {
            runs.push(tt);
            tt = 1;
            prev = k;
        }
    }
    runs.push(tt);
}

/// Stable sort of the positions `0..keys.len()` by `keys` into `perm`,
/// pushing the run lengths of equal keys onto `runs`.
fn sort_positions(keys: &[u64], st: &KeyStats, perm: &mut [u32], runs: Option<&mut Vec<u32>>, b: &mut SortBufs) {
    let n = keys.len();
    // Every key minus the smallest is a multiple of 2^tz: sort the
    // quotients (the order is unchanged), as narrow as the keys allow.
    let tz = if st.diff == 0 { 0 } else { st.diff.trailing_zeros() };
    let lo = st.lo;
    let rel = |k: u64| (k - lo) >> tz;
    let range = rel(st.hi);
    if range < 100_000 && range as usize <= n {
        // R: radixsort.c:icount
        let count = &mut b.count;
        count.clear();
        count.resize(range as usize + 1, 0);
        for &k in keys {
            count[rel(k) as usize] += 1;
        }
        let mut sum = 0u32;
        match runs {
            Some(runs) => {
                for c in count.iter_mut() {
                    if *c != 0 {
                        runs.push(*c);
                    }
                    sum += *c;
                    *c = sum;
                }
            }
            None => {
                for c in count.iter_mut() {
                    sum += *c;
                    *c = sum;
                }
            }
        }
        for (i, &k) in keys.iter().enumerate().rev() {
            let c = &mut count[rel(k) as usize];
            *c -= 1;
            perm[*c as usize] = i as u32;
        }
    } else if range <= u32::MAX as u64 {
        // Key and position packed in one word: an unstable sort of the
        // words is a stable sort of the keys.
        let p = &mut b.packed;
        p.clear();
        p.extend(keys.iter().enumerate().map(|(i, &k)| (rel(k) << 32) | i as u64));
        if n <= 256 {
            p.sort_unstable();
        } else {
            lsd_packed(b, 64 - range.leading_zeros());
        }
        let p = &b.packed;
        for (d, &w) in perm.iter_mut().zip(p.iter()) {
            *d = w as u32;
        }
        if let Some(runs) = runs {
            push_runs(runs, p.iter().map(|&w| w >> 32));
        }
    } else {
        b.mkeys.clear();
        b.mkeys.extend(keys.iter().map(|&k| rel(k)));
        b.mpos.clear();
        b.mpos.extend(0..n as u32);
        b.mkeys2.resize(n, 0);
        b.mpos2.resize(n, 0);
        msd_sort(
            &mut b.mkeys,
            &mut b.mpos,
            &mut b.mkeys2[..n],
            &mut b.mpos2[..n],
            64 - range.leading_zeros(),
            &mut b.count,
        );
        perm.copy_from_slice(&b.mpos);
        if let Some(runs) = runs {
            push_runs(runs, b.mkeys.iter().copied());
        }
    }
}

/// LSD radix sort of `b.packed` (keys in the high 32 bits, `bits` wide)
/// in 11-bit digits, skipping digits that all keys share.
fn lsd_packed(b: &mut SortBufs, bits: u32) {
    const W: u32 = 11;
    const B: usize = 1 << W;
    let n = b.packed.len();
    let passes = bits.div_ceil(W).max(1) as usize;
    let hist = &mut b.count;
    hist.clear();
    hist.resize(passes * B, 0);
    for &w in &b.packed {
        let k = w >> 32;
        for p in 0..passes {
            hist[p * B + ((k >> (p as u32 * W)) as usize & (B - 1))] += 1;
        }
    }
    b.packed2.resize(n, 0);
    for p in 0..passes {
        let shift = 32 + p as u32 * W;
        let h = &mut hist[p * B..(p + 1) * B];
        if h[(b.packed[0] >> shift) as usize & (B - 1)] as usize == n {
            continue;
        }
        let mut sum = 0u32;
        for c in h.iter_mut() {
            let t = *c;
            *c = sum;
            sum += t;
        }
        let (src, dst) = (&b.packed, &mut b.packed2);
        for &w in src.iter() {
            let c = &mut h[(w >> shift) as usize & (B - 1)];
            dst[*c as usize] = w;
            *c += 1;
        }
        std::mem::swap(&mut b.packed, &mut b.packed2);
    }
}

/// MSD radix sort of `k` (all below `2^bits`, equal above) carrying the
/// positions `p`, stable; `k2`/`p2` are scratch of the same length.
fn msd_sort(k: &mut [u64], p: &mut [u32], k2: &mut [u64], p2: &mut [u32], bits: u32, count: &mut Vec<u32>) {
    let n = k.len();
    if n <= 32 {
        // Insertion sort: stable, and the positions arrive increasing.
        for i in 1..n {
            let (key, pos) = (k[i], p[i]);
            let mut j = i;
            while j > 0 && k[j - 1] > key {
                k[j] = k[j - 1];
                p[j] = p[j - 1];
                j -= 1;
            }
            k[j] = key;
            p[j] = pos;
        }
        return;
    }
    let mut bits = bits;
    loop {
        if bits == 0 {
            return;
        }
        // About one bucket per key, at most 2^11 of them.
        let w = (usize::BITS - n.leading_zeros()).clamp(4, 11).min(bits);
        let shift = bits - w;
        let mask = (1usize << w) - 1;
        let digit = |x: u64| (x >> shift) as usize & mask;
        count.clear();
        count.resize(mask + 1, 0);
        for &x in k.iter() {
            count[digit(x)] += 1;
        }
        bits = shift;
        if count[digit(k[0])] as usize == n {
            // A digit all keys share.
            continue;
        }
        let mut sum = 0u32;
        for c in count.iter_mut() {
            let t = *c;
            *c = sum;
            sum += t;
        }
        for (&x, &q) in k.iter().zip(p.iter()) {
            let c = &mut count[digit(x)];
            k2[*c as usize] = x;
            p2[*c as usize] = q;
            *c += 1;
        }
        k.copy_from_slice(k2);
        p.copy_from_slice(p2);
        break;
    }
    let shift = bits;
    if shift == 0 {
        return;
    }
    let mut s = 0;
    while s < n {
        let top = k[s] >> shift;
        let mut e = s + 1;
        while e < n && k[e] >> shift == top {
            e += 1;
        }
        if e - s > 1 {
            msd_sort(&mut k[s..e], &mut p[s..e], &mut k2[s..e], &mut p2[s..e], shift, count);
        }
        s = e;
    }
}

/// R: radixsort.c:isort/dsort/csort, preceded by isorted/dsorted/csorted:
/// sort one group `o` (its rows; with `identity`, `o` is `0..n`, the
/// first column) by `key` of each row.
fn sort_by_keys(
    o: &mut [u32],
    identity: bool,
    nalast: i32,
    runs: Option<&mut Vec<u32>>,
    sc: &mut RadixScratch,
    key: impl Fn(usize) -> u64,
) {
    let n = o.len();
    sc.keys.clear();
    if identity {
        sc.keys.extend((0..n).map(&key));
    } else {
        sc.keys.extend(o.iter().map(|&r| key(r as usize)));
    }
    let st = key_stats(&sc.keys);
    if nalast == 0 && st.lo == 0 && (st.hi == 0 || n == 2) {
        // All NA (isorted's -2), or isort's n == 2 case with one NA: the
        // order is kept as it is.
        if let Some(runs) = runs {
            if st.hi == 0 {
                runs.push(n as u32);
            } else {
                runs.extend([1, 1]);
            }
        }
        return;
    }
    // With NAs and `nalast == 0` R sorts even sorted input; a stable sort
    // of it changes nothing, so the shortcuts apply all the same.
    if st.asc {
        if let Some(runs) = runs {
            push_runs(runs, sc.keys.iter().copied());
        }
        return;
    }
    if st.desc {
        o.reverse();
        if let Some(runs) = runs {
            runs.extend(std::iter::repeat_n(1, n));
        }
        return;
    }
    if identity {
        sort_positions(&sc.keys, &st, o, runs, &mut sc.bufs);
    } else {
        sc.perm.clear();
        sc.perm.resize(n, 0);
        sort_positions(&sc.keys, &st, &mut sc.perm, runs, &mut sc.bufs);
        sc.otmp.clear();
        sc.otmp.extend_from_slice(o);
        for (d, &p) in o.iter_mut().zip(&sc.perm) {
            *d = sc.otmp[p as usize];
        }
    }
}

/// R: radixsort.c:csorted + cgroup (`sortStr = FALSE`): sorted input is
/// kept, strictly reversed input reversed, anything else grouped by first
/// appearance.
fn cgroup(x: &[RStr], o: &mut [u32], order: i32, nalast: i32, runs: Option<&mut Vec<u32>>, sc: &mut RadixScratch) {
    let n = o.len();
    let at = |k: usize| &x[o[k] as usize];
    let mut grouped = false;
    if nalast == 0 {
        let na = o.iter().filter(|&&r| x[r as usize].is_none()).count();
        if na == n {
            if let Some(runs) = runs {
                runs.push(n as u32);
            }
            return;
        }
        grouped = na != 0;
    }
    if !grouped {
        let c = |a: usize, b: usize| str_cmp2(at(a), at(b), order, nalast);
        if n >= 2 && c(1, 0) < 0 {
            if (2..n).all(|k| c(k, k - 1) < 0) {
                o.reverse();
                if let Some(runs) = runs {
                    runs.extend(std::iter::repeat_n(1, n));
                }
                return;
            }
            grouped = true;
        } else if (1..n).all(|k| c(k, k - 1) >= 0) {
            if let Some(runs) = runs {
                let mut tt = 1u32;
                for k in 1..n {
                    if at(k) == at(k - 1) {
                        tt += 1;
                    } else {
                        runs.push(tt);
                        tt = 1;
                    }
                }
                runs.push(tt);
            }
            return;
        } else {
            grouped = true;
        }
    }
    debug_assert!(grouped);
    // A counting sort by the index of first appearance.
    let mut ids: HashMap<Option<&str>, u32, crate::sym::StrHash> = HashMap::default();
    let counts = &mut sc.bufs.count;
    counts.clear();
    sc.perm.clear();
    for &r in o.iter() {
        let next = ids.len() as u32;
        let id = *ids.entry(x[r as usize].as_deref()).or_insert(next);
        if id == next {
            counts.push(0);
        }
        counts[id as usize] += 1;
        sc.perm.push(id);
    }
    if let Some(runs) = runs {
        runs.extend_from_slice(counts);
    }
    let mut sum = 0u32;
    for c in counts.iter_mut() {
        let t = *c;
        *c = sum;
        sum += t;
    }
    sc.otmp.clear();
    sc.otmp.extend_from_slice(o);
    for (&r, &id) in sc.otmp.iter().zip(&sc.perm) {
        let c = &mut counts[id as usize];
        o[*c as usize] = r;
        *c += 1;
    }
}

/// Sort one group of rows `o` by column `col`.
#[allow(clippy::too_many_arguments)]
fn radix_group(
    col: &RadixCol,
    o: &mut [u32],
    identity: bool,
    order: i32,
    nalast: i32,
    dround: u32,
    runs: Option<&mut Vec<u32>>,
    sc: &mut RadixScratch,
) {
    match col {
        RadixCol::Int(v) => sort_by_keys(o, identity, nalast, runs, sc, |r| ikey(v[r], order, nalast)),
        RadixCol::Rank(v) => sort_by_keys(o, identity, nalast, runs, sc, |r| ikey(v[r], order, nalast)),
        RadixCol::Real(v) => sort_by_keys(o, identity, nalast, runs, sc, |r| dtwiddle(v[r], order, nalast, dround)),
        RadixCol::Group(v) => cgroup(v, o, order, nalast, runs, sc),
    }
}

/// The rank of each string among the distinct strings of `x` in the
/// order `cmp` (0, 1, ...; `NA_INTEGER` for NA).
fn string_ranks(x: &[RStr], cmp: impl Fn(&str, &str) -> Ordering) -> Vec<i32> {
    let mut ids: HashMap<&str, u32, crate::sym::StrHash> = HashMap::default();
    let mut distinct: Vec<&str> = Vec::new();
    let id: Vec<u32> = x
        .iter()
        .map(|s| match s {
            None => u32::MAX,
            Some(s) => *ids.entry(&**s).or_insert_with(|| {
                distinct.push(s);
                distinct.len() as u32 - 1
            }),
        })
        .collect();
    let mut by_order: Vec<u32> = (0..distinct.len() as u32).collect();
    by_order.sort_unstable_by(|&p, &q| cmp(distinct[p as usize], distinct[q as usize]));
    // Strings that compare equal share a rank.
    let mut rank = vec![0i32; distinct.len()];
    let mut r = 0;
    for (k, &d) in by_order.iter().enumerate() {
        if k > 0 && cmp(distinct[by_order[k - 1] as usize], distinct[d as usize]) != Ordering::Equal {
            r += 1;
        }
        rank[d as usize] = r;
    }
    id.into_iter().map(|d| if d == u32::MAX { NA_INTEGER } else { rank[d as usize] }).collect()
}

/// The radix ordering of the rows `0..n` (R: radixsort.c:do_radixsort's
/// loop over the columns). `column(k)` gives column `k` and its `order`
/// (1, or -1 for decreasing) when the loop reaches it. Returns the order
/// and, when `retgrp`, the group sizes.
fn radix_order<'a>(
    n: usize,
    ncol: usize,
    nalast: i32,
    retgrp: bool,
    mut column: impl FnMut(usize) -> RResult<(RadixCol<'a>, i32)>,
) -> RResult<(Vec<u32>, Vec<u32>)> {
    let dround = if retgrp { 2 } else { 0 };
    let mut o: Vec<u32> = (0..n as u32).collect();
    let mut groups: Vec<u32> = if n > 0 { vec![n as u32] } else { vec![] };
    let mut next: Vec<u32> = Vec::new();
    let mut sc = RadixScratch::default();
    for c in 0..ncol {
        if c > 0 && groups.len() == n && nalast != 0 {
            break;
        }
        let (col, order) = column(c)?;
        // R: stackgrps: group sizes are only needed by a later column or
        // for the result.
        let stack = c + 1 < ncol || retgrp;
        next.clear();
        let mut start = 0;
        for &g in &groups {
            let g = g as usize;
            if g == 1 {
                if stack {
                    next.push(1);
                }
            } else {
                let runs = if stack { Some(&mut next) } else { None };
                radix_group(&col, &mut o[start..start + g], c == 0, order, nalast, dround, runs, &mut sc);
            }
            start += g;
        }
        if stack {
            std::mem::swap(&mut groups, &mut next);
        }
    }
    Ok((o, groups))
}

/// The 1-based result of [`radix_order`].
fn radix_result(o: Vec<u32>) -> Vec<i32> {
    o.into_iter().map(|r| r as i32 + 1).collect()
}

/// `na.last = NA`: rows with an NA in any key are dropped, or set to 0
/// when the groups are returned.
#[cold]
#[inline(never)]
fn radix_drop_na(o: Vec<u32>, keys: &[Value], retgrp: bool) -> Vec<i32> {
    let mut na = vec![false; o.len()];
    for x in keys {
        match x {
            Value::Int(v) | Value::Lgl(v) => na.iter_mut().zip(&v.data).for_each(|(f, &e)| *f |= e == NA_INTEGER),
            Value::Real(v) => na.iter_mut().zip(&v.data).for_each(|(f, &e)| *f |= e.is_nan()),
            Value::Str(v) => na.iter_mut().zip(&v.data).for_each(|(f, e)| *f |= e.is_none()),
            _ => {}
        }
    }
    if retgrp {
        o.into_iter().map(|r| if na[r as usize] { 0 } else { r as i32 + 1 }).collect()
    } else {
        o.into_iter().filter(|&r| !na[r as usize]).map(|r| r as i32 + 1).collect()
    }
}

fn has_na(x: &Value) -> bool {
    match x {
        Value::Int(v) | Value::Lgl(v) => v.data.contains(&NA_INTEGER),
        Value::Real(v) => v.data.iter().any(|e| e.is_nan()),
        Value::Str(v) => v.data.iter().any(|e| e.is_none()),
        _ => false,
    }
}

// R: radixsort.c:do_radixsort
fn do_radixsort(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    // R: radixsort.c:do_radixsort reads na.last, decreasing, retGrp, sortStr
    // before looking at the vectors; a missing sortStr is NULL, which
    // asBool2 rejects with "NA in coercion to boolean".
    let get = |k: usize| args.get(k).map(|a| a.value.clone()).unwrap_or(Value::Null);
    let nl = crate::coerce::as_logical(&get(0));
    let nalast: i32 = if nl == NA_LOGICAL {
        0
    } else if nl == TRUE {
        1
    } else {
        -1
    };
    let decreasing = get(1);
    let retgrp = as_bool2(&get(2), call)?;
    let sort_str = as_bool2(&get(3), call)?;
    let keys: Vec<Value> = args.iter().skip(4).map(|a| a.value.clone()).collect();
    if keys.is_empty() {
        return Ok(Value::Null);
    }
    let nl = if keys[0].is_vector() { keys[0].len() as isize } else { -1 };
    for (k, x) in keys.iter().enumerate() {
        if !x.is_vector() {
            return Err(Flow::error(format!("argument {} is not a vector", k + 1)));
        }
        if x.len() as isize != nl {
            return Err(Flow::error("argument lengths differ"));
        }
    }
    let narg = keys.len();
    if narg != decreasing.len() {
        return Err(Flow::error("length(decreasing) must match the number of order arguments"));
    }
    let dec: Vec<i32> = match &decreasing {
        Value::Lgl(v) => v.data.clone(),
        other => crate::coerce::to_lgl(other, &mut Default::default()),
    };
    if dec.contains(&NA_LOGICAL) {
        return Err(Flow::error("'decreasing' elements must be TRUE or FALSE"));
    }
    let n = nl.max(0) as usize;
    let (o, groups) = radix_order(n, narg, nalast, retgrp, |c| {
        let x = &keys[c];
        let col = match x {
            Value::Int(v) | Value::Lgl(v) => RadixCol::Int(&v.data),
            Value::Real(v) => RadixCol::Real(&v.data),
            Value::Str(v) if sort_str => RadixCol::Rank(string_ranks(&v.data, |a, b| a.as_bytes().cmp(b.as_bytes()))),
            Value::Str(v) => RadixCol::Group(&v.data),
            _ if c == 0 => {
                return Err(Flow::error(format!("First arg is type '{}', not yet supported", x.type_name())));
            }
            _ => {
                return Err(Flow::error(format!("Arg {} is type '{}', not yet supported", c + 1, x.type_name())));
            }
        };
        Ok((col, if dec[c] != 0 { -1 } else { 1 }))
    })?;
    let ans = if nalast == 0 && keys.iter().any(has_na) { radix_drop_na(o, &keys, retgrp) } else { radix_result(o) };
    let mut res = Value::ints(ans);
    if retgrp {
        let mut ends = Vec::with_capacity(groups.len());
        let mut acc = 0;
        for &g in &groups {
            acc += g as i32;
            ends.push(acc);
        }
        let maxgrpn = groups.iter().copied().max().map(|m| m as i32).unwrap_or(NA_INTEGER);
        res.set_attr_raw(Sym::new("ends"), Value::ints(ends));
        res.set_attr_raw(Sym::new("maxgrpn"), Value::int(maxgrpn));
        res.set_attr_raw(s::CLASS, Value::strs_from(["grouping", "integer"]));
    }
    Ok(res)
}
