//! Sorting, ordering and ranking (GNU R's sort.c, radixsort.c, qsort.c
//! and the `wrap_meta` stub of altclasses.c).
//!
//! The order produced by `order()`/`radixsort` is fully determined by the
//! comparison keys (all of R's algorithms here are stable or break ties
//! by index), so those are implemented as stable sorts on R's keys. The
//! in-place algorithms whose results depend on the algorithm itself
//! (`psort`, `qsort` with ties, Shell sort of values) are ported as is.

use std::cmp::Ordering;

use super::summary::{as_bool2, check_arity_internal};
use super::{internal, prim};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("sort", do_sort, 2),
    internal("psort", do_psort, 2),
    internal("qsort", do_qsort, 2),
    internal("order", do_order, -1),
    internal("radixsort", do_radixsort, -1),
    internal("rank", do_rank, 3),
    prim("xtfrm", do_xtfrm, 1),
    internal("is.unsorted", do_isunsorted, -1),
    internal("sorted_fpass", do_sorted_fpass, 3),
    internal("wrap_meta", do_wrap_meta, 3),
];

/// `Scollate` (string collation used by sort/order/rank).
pub fn scollate(a: &str, b: &str) -> Ordering {
    super::arith::scollate(a, b)
}

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

pub fn scmp(x: &RStr, y: &RStr, nalast: bool) -> i32 {
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
                sign(scollate(a, b))
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
        Value::Str(v) => v.data.windows(2).any(|w| bad(scmp(&w[0], &w[1], true))),
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
            if decreasing {
                shell_sort2(d, |a, b| a < b)
            } else {
                shell_sort2(d, |a, b| a > b)
            }
        }
        Value::Real(v) => {
            let d = &mut std::rc::Rc::make_mut(v).data;
            if decreasing {
                shell_sort2(d, |a, b| a < b)
            } else {
                shell_sort2(d, |a, b| a > b)
            }
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
            if decreasing {
                shell_sort2(d, |a, b| scmp(a, b, true) < 0)
            } else {
                shell_sort2(d, |a, b| scmp(a, b, true) > 0)
            }
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
        Value::Real(v) => {
            psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &f64, b: &f64| rcmp(*a, *b, true))
        }
        Value::Cplx(v) => psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &Complex, b: &Complex| {
            ccmp(*a, *b, true)
        }),
        Value::Str(v) => {
            psort0(&mut std::rc::Rc::make_mut(v).data, 0, hi, &l, &|a: &RStr, b: &RStr| scmp(a, b, true))
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

const SINCS: [usize; 17] =
    [1073790977, 268460033, 67121153, 16783361, 4197377, 1050113, 262913, 65921, 16577, 4193, 1073, 281, 77, 23, 8, 1, 0];

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
fn elt_cmp(x: &Value, a: usize, b: usize, nalast: bool) -> RResult<i32> {
    Ok(match x {
        Value::Lgl(v) | Value::Int(v) => icmp(v.data[a], v.data[b], nalast),
        Value::Real(v) => rcmp(v.data[a], v.data[b], nalast),
        Value::Cplx(v) => ccmp(v.data[a], v.data[b], nalast),
        Value::Str(v) => scmp(&v.data[a], &v.data[b], nalast),
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
    let mut o: Vec<usize> = (0..n).collect();
    if keys.len() == 1 {
        let key = &keys[0];
        if !matches!(key, Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Str(_) | Value::Cplx(_)) {
            return Err(Flow::error(format!("unimplemented type '{}' in 'orderVector1'\n", key.type_name())));
        }
        // NAs to one end, then the rest by value; ties by index.
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
            let c = elt_cmp(key, a, b, nalast)?;
            let c = if decreasing { -c } else { c };
            Ok(c.cmp(&0))
        };
        stable_sort_by(&mut o, &mut cmp)?;
    } else {
        let mut cmp = |a: usize, b: usize| -> RResult<Ordering> {
            for key in &keys {
                let c = elt_cmp(key, a, b, nalast ^ decreasing)?;
                let c = if decreasing { -c } else { c };
                if c != 0 {
                    return Ok(c.cmp(&0));
                }
            }
            Ok(Ordering::Equal)
        };
        stable_sort_by(&mut o, &mut cmp)?;
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
        if d > 4503599627370496.0 {
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
                        let c = scollate(v.data[a].as_deref().unwrap_or("NA"), v.data[b].as_deref().unwrap_or("NA"));
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
                let c = if obj { gt_callback(i, &x, o[j], o[j + 1], env)? } else { elt_cmp(&x, o[j], o[j + 1], true)? };
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
    }
    Ok(if kind == Ties::Average { Value::reals(rk) } else { Value::ints(ik) })
}

// R: sort.c:do_xtfrm
fn do_xtfrm(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    super::check_arity(&args, 1, "xtfrm", call)?;
    super::seq::check1arg(&args, call, "x")?;
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
// sorted_fpass, wrap_meta (ALTREP metadata; rr has no ALTREP wrappers)

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
        if nalast != 0 {
            -1
        } else {
            -2
        }
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
        Value::Int(_) | Value::Real(_) | Value::Lgl(_) | Value::Cplx(_) | Value::Raw(_) | Value::Str(_) | Value::List(_)
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

/// `dtwiddle`: the sortable unsigned key of a double.
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

/// `icheck`.
fn icheck(x: i32, order: i32, nalast: i32) -> i64 {
    if nalast != 1 {
        if x != NA_INTEGER {
            x as i64 * order as i64
        } else {
            x as i64
        }
    } else if x != NA_INTEGER {
        x as i64 * order as i64 - 1
    } else {
        i32::MAX as i64
    }
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

/// Stable sort of `o` by the parallel `keys` (LSD radix, 11-bit digits,
/// skipping digits all keys share). Returns the keys in sorted order.
fn radix_sort_keys(o: &mut [usize], keys: Vec<u64>) -> Vec<u64> {
    let n = o.len();
    let mut src: Vec<(u64, usize)> = keys.into_iter().zip(o.iter().copied()).collect();
    if n < 256 || n > u32::MAX as usize {
        src.sort_by_key(|p| p.0);
    } else {
        const BITS: usize = 11;
        const B: usize = 1 << BITS;
        const PASSES: usize = 6;
        let mut hist = vec![0u32; PASSES * B];
        for &(k, _) in &src {
            for p in 0..PASSES {
                hist[p * B + ((k >> (p * BITS)) as usize & (B - 1))] += 1;
            }
        }
        let mut dst = vec![(0u64, 0usize); n];
        for p in 0..PASSES {
            let h = &mut hist[p * B..(p + 1) * B];
            let first = ((src[0].0 >> (p * BITS)) as usize) & (B - 1);
            if h[first] as usize == n {
                continue;
            }
            let mut sum = 0u32;
            for c in h.iter_mut() {
                let t = *c;
                *c = sum;
                sum += t;
            }
            for &(k, i) in &src {
                let d = (k >> (p * BITS)) as usize & (B - 1);
                dst[h[d] as usize] = (k, i);
                h[d] += 1;
            }
            std::mem::swap(&mut src, &mut dst);
        }
    }
    for (slot, &(_, i)) in o.iter_mut().zip(&src) {
        *slot = i;
    }
    src.into_iter().map(|(k, _)| k).collect()
}

/// Lengths of the runs of equal adjacent keys.
fn key_runs(keys: &[u64]) -> Vec<usize> {
    let mut runs = Vec::new();
    let mut tt = 1;
    for k in 1..keys.len() {
        if keys[k - 1] == keys[k] {
            tt += 1;
        } else {
            runs.push(tt);
            tt = 1;
        }
    }
    if !keys.is_empty() {
        runs.push(tt);
    }
    runs
}

/// Sort `o[s..e]` (row indices) stably by column `x`, returning the run
/// lengths of equal keys.
fn radix_group(
    x: &Value,
    o: &mut [usize],
    order: i32,
    nalast: i32,
    retgrp: bool,
    sort_str: bool,
) -> Vec<usize> {
    let n = o.len();
    let runs_by = |o: &[usize], eq: &dyn Fn(usize, usize) -> bool| -> Vec<usize> {
        let mut runs = Vec::new();
        let mut tt = 1;
        for k in 1..o.len() {
            if eq(o[k - 1], o[k]) {
                tt += 1;
            } else {
                runs.push(tt);
                tt = 1;
            }
        }
        if !o.is_empty() {
            runs.push(tt);
        }
        runs
    };
    match x {
        Value::Int(v) | Value::Lgl(v) => {
            let keys = o.iter().map(|&k| (icheck(v.data[k], order, nalast) as u64) ^ (1 << 63)).collect();
            key_runs(&radix_sort_keys(o, keys))
        }
        Value::Real(v) => {
            let dround = if retgrp { 2 } else { 0 };
            let keys = o.iter().map(|&k| dtwiddle(v.data[k], order, nalast, dround)).collect();
            key_runs(&radix_sort_keys(o, keys))
        }
        Value::Str(v) => {
            if sort_str {
                // csort ranks NA as NA_INTEGER: first unless nalast == 1
                let nl = if nalast == 0 { -1 } else { nalast };
                o.sort_by(|&a, &b| str_cmp2(&v.data[a], &v.data[b], order, nl).cmp(&0));
            } else {
                // csorted() first: keep ascending input, reverse strictly
                // descending input, otherwise group by first appearance.
                let c = |a: usize, b: usize| str_cmp2(&v.data[a], &v.data[b], order, nalast);
                let ascending = (1..n).all(|k| c(o[k], o[k - 1]) >= 0);
                let descending = n >= 2 && (1..n).all(|k| c(o[k], o[k - 1]) < 0);
                if descending {
                    o.reverse();
                } else if !ascending {
                    let mut first: std::collections::HashMap<Option<&str>, usize> = Default::default();
                    let mut rank_of = Vec::with_capacity(n);
                    for &k in o.iter() {
                        let key = v.data[k].as_deref();
                        let next = first.len();
                        let r = *first.entry(key).or_insert(next);
                        rank_of.push((r, k));
                    }
                    rank_of.sort_by_key(|&(r, _)| r);
                    for (slot, (_, k)) in o.iter_mut().zip(rank_of) {
                        *slot = k;
                    }
                }
            }
            runs_by(o, &|a, b| v.data[a] == v.data[b])
        }
        _ => vec![n],
    }
}

// R: radixsort.c:do_radixsort
fn do_radixsort(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    if args.len() < 4 {
        return Err(Flow::error("rr: radixsort needs at least 4 arguments"));
    }
    let nl = crate::coerce::as_logical(&args[0].value);
    let nalast: i32 = if nl == NA_LOGICAL {
        0
    } else if nl == TRUE {
        1
    } else {
        -1
    };
    let decreasing = args[1].value.clone();
    let retgrp = as_bool2(&args[2].value, call)?;
    let sort_str = as_bool2(&args[3].value, call)?;
    let keys: Vec<Value> = args[4..].iter().map(|a| a.value.clone()).collect();
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
    if dec.iter().any(|&d| d == NA_LOGICAL) {
        return Err(Flow::error("'decreasing' elements must be TRUE or FALSE"));
    }
    let n = nl.max(0) as usize;
    let type_ok = |x: &Value| matches!(x, Value::Int(_) | Value::Lgl(_) | Value::Real(_) | Value::Str(_));
    if !type_ok(&keys[0]) {
        return Err(Flow::error(format!("First arg is type '{}', not yet supported", keys[0].type_name())));
    }
    let mut o: Vec<usize> = (0..n).collect();
    let mut groups: Vec<usize> = if n > 0 { vec![n] } else { vec![] };
    for (col, x) in keys.iter().enumerate() {
        if col > 0 {
            if groups.len() == n && nalast != 0 {
                break;
            }
            if !type_ok(x) {
                return Err(Flow::error(format!("Arg {} is type '{}', not yet supported", col + 1, x.type_name())));
            }
        }
        let order = if dec[col] != 0 { -1 } else { 1 };
        let mut new_groups = Vec::with_capacity(groups.len());
        let mut start = 0;
        for &g in &groups {
            if g == 1 {
                new_groups.push(1);
            } else {
                new_groups.extend(radix_group(x, &mut o[start..start + g], order, nalast, retgrp, sort_str));
            }
            start += g;
        }
        groups = new_groups;
    }
    let mut ans: Vec<i32> = o.iter().map(|&k| k as i32 + 1).collect();
    // NA rows, computed in row order once (`o` is a permutation).
    let na_rows: Vec<bool> = if nalast == 0 { (0..n).map(|k| keys.iter().any(|x| is_na_at(x, k))).collect() } else { vec![] };
    let has_na_row = |k: usize| na_rows[k];
    if nalast == 0 && na_rows.contains(&true) {
        if retgrp {
            for (slot, &k) in ans.iter_mut().zip(o.iter()) {
                if has_na_row(k) {
                    *slot = 0;
                }
            }
        } else {
            ans = o.iter().filter(|&&k| !has_na_row(k)).map(|&k| k as i32 + 1).collect();
        }
    }
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
