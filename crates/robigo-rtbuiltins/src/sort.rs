//! sort.c: `.Internal(sort, psort, qsort, order, rank, is.unsorted,
//! sorted_fpass)` and the `xtfrm` primitive; radixsort.c's
//! `.Internal(radixsort())` ([`radix`]); altclasses.c's `wrap_meta`.
//!
//! The sorts are GNU R's algorithms, ported as they are: Shell sort with
//! Sedgewick's increments (`sort2_body`, `sort2_with_index`,
//! `orderVector`), `Psort`'s quickselect and Singleton's quicksort
//! (qsort-body.c). Where the comparison is a total order (an index breaks
//! every tie) any correct sort gives the same result, but `.Internal(sort())`
//! and `psort` move values that compare equal (`-0` and `0`, strings that
//! collate equal), and a comparison that calls R (`.gt`, `rank` of an
//! object) is observable call by call: only the same algorithm agrees.

use crate::text::{self, CollateError};
use crate::util::{as_logical, not_yet};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Locale, Out, RError, RResult, Rcomplex, Ref, Root, View};
use std::borrow::Cow;

pub mod radix;

// ------------------------------------------------------------------
// Part I: comparison

const NA_INTEGER: i32 = i32::MIN;

/// `icmp`.
// R: sort.c:icmp
fn icmp(x: i32, y: i32, nalast: bool) -> i32 {
    if x == NA_INTEGER && y == NA_INTEGER {
        return 0;
    }
    if x == NA_INTEGER {
        return if nalast { 1 } else { -1 };
    }
    if y == NA_INTEGER {
        return if nalast { -1 } else { 1 };
    }
    (x > y) as i32 - (x < y) as i32
}

/// `rcmp`: every NaN (NA too) equal, after (`nalast`) or before the
/// numbers.
// R: sort.c:rcmp
fn rcmp(x: f64, y: f64, nalast: bool) -> i32 {
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
    (x > y) as i32 - (x < y) as i32
}

/// `ccmp`: the real parts (two NaN ones are equal, whatever follows), then
/// the imaginary ones.
// R: sort.c:ccmp
fn ccmp(x: Rcomplex, y: Rcomplex, nalast: bool) -> i32 {
    if x.r.is_nan() && y.r.is_nan() {
        return 0;
    }
    match rcmp(x.r, y.r, nalast) {
        0 => rcmp(x.i, y.i, nalast),
        c => c,
    }
}

/// `scmp`: NA, the same CHARSXP, then `Scollate`.
// R: sort.c:scmp
fn scmp(x: Char<'_>, y: Char<'_>, nalast: bool, na: Char<'_>, loc: Locale) -> Result<i32, CollateError> {
    if x == na && y == na {
        return Ok(0);
    }
    if x == na {
        return Ok(if nalast { 1 } else { -1 });
    }
    if y == na {
        return Ok(if nalast { -1 } else { 1 });
    }
    if x == y {
        return Ok(0);
    }
    collate(x, y, loc)
}

/// `Scollate(x, y)` as -1, 0 or 1.
fn collate(x: Char<'_>, y: Char<'_>, loc: Locale) -> Result<i32, CollateError> {
    Ok(text::scollate(crate::coerce::text(x), crate::coerce::text(y), loc)? as i32)
}

/// A key's data as the comparisons read it (a logical's as its ints).
enum Key<'g> {
    Int(Cow<'g, [i32]>),
    Real(&'g [f64]),
    Cplx(&'g [Rcomplex]),
    Str(&'g [Char<'g>]),
    /// Another type, named as `UNIMPLEMENTED_TYPE` names it.
    Other(&'static str),
}

impl<'g> Key<'g> {
    fn of(x: Ref<'g>) -> Key<'g> {
        match x.view() {
            View::Lgl(v) => Key::Int(Cow::Owned(v.as_slice().unwrap_or(&[]).iter().map(|l| l.0).collect())),
            View::Int(v) => Key::Int(Cow::Borrowed(v.as_slice().unwrap_or(&[]))),
            View::Real(v) => Key::Real(v.as_slice().unwrap_or(&[])),
            View::Cplx(v) => Key::Cplx(v.as_slice().unwrap_or(&[])),
            View::Str(v) => Key::Str(v.as_slice().unwrap_or(&[])),
            _ => Key::Other(crate::util::type_to_char(x)),
        }
    }

    /// `x[i]` against `x[j]` with `icmp`, `rcmp`, `ccmp` or `scmp`;
    /// `UNIMPLEMENTED_TYPE(func)` for another type.
    fn cmp(
        &self,
        i: usize,
        j: usize,
        nalast: bool,
        na: Char<'_>,
        loc: Locale,
        func: &'static str,
    ) -> Result<i32, CmpErr> {
        Ok(match self {
            Key::Int(x) => icmp(x[i], x[j], nalast),
            Key::Real(x) => rcmp(x[i], x[j], nalast),
            Key::Cplx(x) => ccmp(x[i], x[j], nalast),
            Key::Str(x) => scmp(x[i], x[j], nalast, na, loc)?,
            Key::Other(t) => return Err(CmpErr::Type(t, func)),
        })
    }
}

/// A comparison's failure: Scollate's, or `UNIMPLEMENTED_TYPE`'s (the
/// type, the function).
pub(crate) enum CmpErr {
    Collate(CollateError),
    Type(&'static str, &'static str),
}

impl From<CollateError> for CmpErr {
    fn from(e: CollateError) -> CmpErr {
        CmpErr::Collate(e)
    }
}

impl CmpErr {
    pub(crate) fn raise(self, cx: &mut Cx, call: Root<'_>) -> RError {
        match self {
            CmpErr::Collate(e) => e.raise(cx, Some(call)),
            CmpErr::Type(t, f) => unimplemented_type(cx, t, f),
        }
    }
}

/// `UNIMPLEMENTED_TYPE(func, x)`: `error()`'s message, newline and all.
// R: util.c:UNIMPLEMENTED_TYPEt
fn unimplemented_type(cx: &mut Cx, t: &str, func: &str) -> RError {
    cx.error(&format!("unimplemented type '{t}' in '{func}'\n"))
}

// ------------------------------------------------------------------
// is.unsorted

/// `isUnsorted(x, strictly)` of an atomic vector without NAs (the R code
/// removes them): GNU R's loops over one region, its `INT_MIN` and `-Inf`
/// barrier values included.
// R: sort.c:isUnsorted
fn is_unsorted(x: Ref<'_>, strictly: bool, na: Char<'_>, loc: Locale) -> Result<bool, CmpErr> {
    let n = crate::coerce::xlength(x);
    if n < 2 {
        return Ok(false);
    }
    Ok(match x.view() {
        View::Lgl(_) | View::Int(_) => {
            let Key::Int(v) = Key::of(x) else { unreachable!() };
            if strictly {
                // (GNU R's barrier INT_MIN >= x[0]: only an NA first)
                v[0] == NA_INTEGER || v.windows(2).any(|w| w[0] >= w[1])
            } else {
                v.windows(2).any(|w| w[0] > w[1])
            }
        }
        View::Real(v) => {
            let v = v.as_slice().unwrap_or(&[]);
            // the barrier (-Inf) is checked after the first region only
            if strictly {
                v.windows(2).any(|w| w[0] >= w[1])
            } else {
                f64::NEG_INFINITY > v[0] || v.windows(2).any(|w| w[0] > w[1])
            }
        }
        View::Cplx(v) => {
            let v = v.as_slice().unwrap_or(&[]);
            let c = if strictly { 0 } else { 1 };
            v.windows(2).any(|w| ccmp(w[0], w[1], true) >= c)
        }
        View::Str(v) => {
            let v = v.as_slice().unwrap_or(&[]);
            let c = if strictly { 0 } else { 1 };
            for w in v.windows(2) {
                if scmp(w[0], w[1], true, na, loc)? >= c {
                    return Ok(true);
                }
            }
            false
        }
        View::Raw(v) => {
            let v = v.as_slice().unwrap_or(&[]);
            if strictly { v.windows(2).any(|w| w[0] >= w[1]) } else { v.windows(2).any(|w| w[0] > w[1]) }
        }
        _ => return Err(CmpErr::Type(crate::util::type_to_char(x), "isUnsorted")),
    })
}

/// `.Internal(is.unsorted(x, na.rm, strictly))`: an internal generic; for
/// an object that is not atomic, `.gtn(x, strictly)`.
// R: sort.c:do_isunsorted
pub fn do_isunsorted(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // an old call's two arguments get na.rm = FALSE inserted
    let row = crate::row!("is.unsorted", true);
    if args.len() != 2 {
        row.check_arity(cx, call, args)?;
    }
    let robigo_rtcore::Dispatched::Default(out) =
        crate::coerce::dispatch_or_eval(cx, &IS_UNSORTED, call, args, env, out)?
    else {
        return Ok(());
    };
    let x = args.root(0);
    let strictly_arg = args.root(args.len() - 1);
    let strictly = as_logical(cx, strictly_arg.bind(cx));
    if strictly == NA_INTEGER {
        return Err(cx.error("invalid 'strictly' argument"));
    }
    let xv = x.bind(cx);
    if crate::util::is_vector_atomic(xv) {
        if xv.is_package_altrep() {
            return Err(not_yet(cx, Some(call), "is.unsorted() of a package ALTREP vector"));
        }
        let r = if crate::coerce::xlength(xv) < 2 {
            Ok(false)
        } else {
            is_unsorted(xv, strictly != 0, cx.na_string(), cx.locale())
        };
        return match r {
            Ok(b) => cx.build_scalar(out, Lgl(b as i32)),
            Err(e) => Err(e.raise(cx, call)),
        };
    }
    if xv.is_object() {
        // .gtn(x, strictly)
        return cx.scope(|cx, s| {
            let f = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b".gtn")?.into())))?;
            let c = s.capture(cx, |cx, o| cx.build(o, |cx| cx.lang3(f.bind(cx), x.bind(cx), strictly_arg.bind(cx))))?;
            cx.eval(c, env, out)
        });
    }
    cx.build_scalar(out, Lgl::NA)
}

static IS_UNSORTED: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("is.unsorted");

// ------------------------------------------------------------------
// Part II: complete sorting

/// Sedgewick's increments, `incs` (long vectors: from 2^38 down).
// R: sort.c:incs
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

/// `sort2_body`: Shell sort of `x` with `incs`, moving `x[j - h]` up while
/// `gt(x[j - h], v)` (`>`, or `<` when decreasing).
// R: sort.c:sort2_body
fn sort2<T: Copy, E>(x: &mut [T], mut gt: impl FnMut(&T, &T) -> Result<bool, E>) -> Result<(), E> {
    let n = x.len();
    let mut t = 0;
    while INCS[t] > n {
        t += 1;
    }
    while t < INCS.len() - 1 {
        let h = INCS[t];
        for i in h..n {
            let v = x[i];
            let mut j = i;
            while j >= h && gt(&x[j - h], &v)? {
                x[j] = x[j - h];
                j -= h;
            }
            x[j] = v;
        }
        t += 1;
    }
    Ok(())
}

/// The data `sortVector` sorts, out of the duplicate: values, or for
/// strings the positions (the algorithm moves positions as it would move
/// the CHARSXPs, so the arrangement is the same).
enum Vals {
    Int(Vec<i32>),
    Real(Vec<f64>),
    Cplx(Vec<Rcomplex>),
    Str(Vec<usize>),
}

/// `sortVector(s, decreasing)`: nothing for fewer than two values or,
/// increasing, when `isUnsorted(s, FALSE)` says it is sorted.
// R: sort.c:sortVector, R_isort2, R_rsort2, R_csort2, ssort2
fn sort_vector(x: Ref<'_>, decreasing: bool, na: Char<'_>, loc: Locale) -> Result<Vals, CmpErr> {
    let n = crate::coerce::xlength(x);
    let sort = n >= 2 && (decreasing || is_unsorted(x, false, na, loc)?);
    Ok(match x.view() {
        View::Lgl(_) | View::Int(_) => {
            let Key::Int(v) = Key::of(x) else { unreachable!() };
            let mut v = v.into_owned();
            if sort {
                sort2(&mut v, |a, b| Ok::<_, CmpErr>(if decreasing { a < b } else { a > b }))?;
            }
            Vals::Int(v)
        }
        View::Real(v) => {
            let mut v = v.as_slice().unwrap_or(&[]).to_vec();
            if sort {
                sort2(&mut v, |a, b| Ok::<_, CmpErr>(if decreasing { a < b } else { a > b }))?;
            }
            Vals::Real(v)
        }
        View::Cplx(v) => {
            let mut v = v.as_slice().unwrap_or(&[]).to_vec();
            if sort {
                sort2(&mut v, |a, b| {
                    Ok::<_, CmpErr>(if decreasing {
                        a.r < b.r || (a.r == b.r && a.i < b.i)
                    } else {
                        a.r > b.r || (a.r == b.r && a.i > b.i)
                    })
                })?;
            }
            Vals::Cplx(v)
        }
        View::Str(v) => {
            let v = v.as_slice().unwrap_or(&[]);
            let mut p: Vec<usize> = (0..n).collect();
            if sort {
                sort2(&mut p, |&a, &b| {
                    let c = scmp(v[a], v[b], true, na, loc)?;
                    Ok::<_, CmpErr>(if decreasing { c < 0 } else { c > 0 })
                })?;
            }
            Vals::Str(p)
        }
        _ => return Err(CmpErr::Type(crate::util::type_to_char(x), "sortVector")),
    })
}

/// `.Internal(sort(x, decreasing))`: a copy of `x` without attributes,
/// sorted.
// R: sort.c:do_sort
pub fn do_sort(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("sort", true).check_arity(cx, call, args)?;
    let decreasing = as_logical(cx, args.get(cx, 1));
    if decreasing == NA_INTEGER {
        return Err(cx.error("'decreasing' must be TRUE or FALSE"));
    }
    let x = args.get(cx, 0);
    if x.sexptype() == NILSXP {
        return cx.build(out, |cx| Ok(cx.nil()));
    }
    if !crate::util::is_vector_atomic(x) {
        return Err(cx.error("only atomic vectors can be sorted"));
    }
    if x.sexptype() == RAWSXP {
        return Err(cx.error("raw vectors cannot be sorted"));
    }
    if x.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "sort() of a package ALTREP vector"));
    }
    let t = x.sexptype();
    let vals = match sort_vector(x, decreasing != 0, cx.na_string(), cx.locale()) {
        Ok(v) => v,
        Err(e) => return Err(e.raise(cx, call)),
    };
    cx.build(out, |cx| {
        Ok(match vals {
            Vals::Int(ref v) if t == LGLSXP => {
                cx.alloc_copy(&v.iter().map(|&i| Lgl(i)).collect::<Vec<_>>())?.finish().into()
            }
            Vals::Int(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Real(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Cplx(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Str(ref p) => {
                let x = args.get(cx, 0).as_vector::<Char<'_>>().unwrap();
                let mut s = cx.alloc_vector::<Char<'_>>(p.len())?;
                for (i, &k) in p.iter().enumerate() {
                    s.set(i, x.get(k));
                }
                s.finish().into()
            }
        })
    })
}

// ------------------------------------------------------------------
// Part III: partial sorting

/// `psort_body`: quickselect, so that `x[k]` is in its place, the smaller
/// to its left and the larger to its right (0-based, `lo <= k <= hi`).
// R: sort.c:psort_body
fn psort2<T: Copy>(
    x: &mut [T],
    lo: usize,
    hi: usize,
    k: usize,
    mut cmp: impl FnMut(&T, &T) -> Result<i32, CmpErr>,
) -> Result<(), CmpErr> {
    let (mut l, mut r) = (lo as isize, hi as isize);
    let k = k as isize;
    while l < r {
        let v = x[k as usize];
        let (mut i, mut j) = (l, r);
        while i <= j {
            while cmp(&x[i as usize], &v)? < 0 {
                i += 1;
            }
            while cmp(&v, &x[j as usize])? < 0 {
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
    Ok(())
}

/// `Psort0`: each of the 1-based positions `ind` (sorted) in its place,
/// the one nearest the middle first.
// R: sort.c:Psort0, Psort
fn psort0(
    v: &mut Vals,
    cmp_str: &mut dyn FnMut(usize, usize) -> Result<i32, CmpErr>,
    lo: isize,
    hi: isize,
    ind: &[usize],
) -> Result<(), CmpErr> {
    if ind.is_empty() || hi - lo < 1 {
        return Ok(());
    }
    let (lo_u, hi_u) = (lo as usize, hi as usize);
    let mut one = |v: &mut Vals, z: usize| -> Result<(), CmpErr> {
        match v {
            Vals::Int(x) => psort2(x, lo_u, hi_u, z, |a, b| Ok(icmp(*a, *b, true))),
            Vals::Real(x) => psort2(x, lo_u, hi_u, z, |a, b| Ok(rcmp(*a, *b, true))),
            Vals::Cplx(x) => psort2(x, lo_u, hi_u, z, |a, b| Ok(ccmp(*a, *b, true))),
            Vals::Str(p) => psort2(p, lo_u, hi_u, z, |&a, &b| cmp_str(a, b)),
        }
    };
    if ind.len() <= 1 {
        return one(v, ind[0] - 1);
    }
    let mid = (lo + hi) / 2;
    let mut this = 0;
    for (i, &k) in ind.iter().enumerate() {
        if (k as isize) - 1 <= mid {
            this = i;
        }
    }
    let z = ind[this] - 1;
    one(v, z)?;
    psort0(v, cmp_str, lo, z as isize - 1, &ind[..this])?;
    psort0(v, cmp_str, z as isize + 1, hi, &ind[this + 1..])
}

// ------------------------------------------------------------------
// Part IV: rank and order

/// `sincs`: the increments of the index sorts.
// R: sort.c:sincs
const SINCS: [usize; 17] = [
    1073790977, 268460033, 67121153, 16783361, 4197377, 1050113, 262913, 65921, 16577, 4193, 1073, 281, 77, 23, 8, 1, 0,
];

/// The first increment for `n` elements (`for (t = 0; sincs[t] > n; t++)`).
fn first_inc(n: usize) -> usize {
    let mut t = 0;
    while SINCS[t] > n {
        t += 1;
    }
    t
}

/// `sort2_with_index` over `indx[lo..=hi]`, from increment `t`, moving up
/// while `less(indx[j - h], itmp)`.
// R: sort.c:sort2_with_index
fn sort2_with_index<E>(
    indx: &mut [usize],
    lo: usize,
    hi: usize,
    mut t: usize,
    mut less: impl FnMut(usize, usize) -> Result<bool, E>,
) -> Result<(), E> {
    while t < 16 {
        let h = SINCS[t];
        let mut i = lo + h;
        while i <= hi {
            let itmp = indx[i];
            let mut j = i;
            while j >= lo + h && less(indx[j - h], itmp)? {
                indx[j] = indx[j - h];
                j -= h;
            }
            indx[j] = itmp;
            i += 1;
        }
        t += 1;
    }
    Ok(())
}

/// `orderVector(indx, n, key, nalast, decreasing, listgreater)`: the
/// keys in turn (`nalast ^ decreasing` placing NAs), the index last.
// R: sort.c:orderVector, listgreater
fn order_vector(
    indx: &mut [usize],
    keys: &[Key<'_>],
    nalast: bool,
    decreasing: bool,
    na: Char<'_>,
    loc: Locale,
) -> Result<(), CmpErr> {
    let n = indx.len();
    if n < 2 {
        return Ok(());
    }
    let nl = nalast ^ decreasing;
    let greater = |i: usize, j: usize| -> Result<bool, CmpErr> {
        let mut c = -1;
        for k in keys {
            c = k.cmp(i, j, nl, na, loc, "listgreater")?;
            if decreasing {
                c = -c;
            }
            if c > 0 {
                return Ok(true);
            }
            if c < 0 {
                return Ok(false);
            }
        }
        Ok(!(c == 0 && i < j))
    };
    let mut t = first_inc(n);
    while t < 16 {
        let h = SINCS[t];
        for i in h..n {
            let itmp = indx[i];
            let mut j = i;
            while j >= h && greater(indx[j - h], itmp)? {
                indx[j] = indx[j - h];
                j -= h;
            }
            indx[j] = itmp;
        }
        t += 1;
    }
    Ok(())
}

/// `orderVector1(indx, n, key, nalast, decreasing, R_NilValue)` for an
/// atomic `key`: the NAs to one end first, then the rest by value, the
/// index breaking ties.
// R: sort.c:orderVector1
pub(crate) fn order_vector1(
    indx: &mut [usize],
    key: Ref<'_>,
    nalast: bool,
    decreasing: bool,
    na: Char<'_>,
    loc: Locale,
) -> Result<(), CmpErr> {
    let n = indx.len();
    if n < 2 {
        return Ok(());
    }
    let k = Key::of(key);
    let isna: Vec<bool> = match &k {
        Key::Int(x) => x.iter().map(|&v| v == NA_INTEGER).collect(),
        Key::Real(x) => x.iter().map(|v| v.is_nan()).collect(),
        Key::Cplx(x) => x.iter().map(|v| v.r.is_nan() || v.i.is_nan()).collect(),
        Key::Str(x) => x.iter().map(|&v| v == na).collect(),
        Key::Other(t) => return Err(CmpErr::Type(t, "orderVector1")),
    };
    let numna = isna.iter().filter(|&&b| b).count();
    let (mut lo, mut hi) = (0, n - 1);
    if numna > 0 {
        let isna: Vec<bool> = if nalast { isna } else { isna.iter().map(|b| !b).collect() };
        sort2_with_index(indx, 0, n - 1, first_inc(n), |a, b| {
            Ok::<_, CmpErr>((isna[a] && !isna[b]) || (isna[a] == isna[b] && a > b))
        })?;
        if n - numna < 2 {
            return Ok(());
        }
        if nalast {
            hi -= numna;
        } else {
            lo += numna;
        }
    }
    order_values(indx, lo, hi, &k, decreasing, loc)
}

/// `orderVector1`'s sort of `indx[lo..=hi]` by the values of an atomic key
/// (its typed `less` macros: plain `<`/`>` on numbers, `ccmp(, , 0)`,
/// `Scollate` with no NA check: under `rank` NAs are not moved first).
// R: sort.c:orderVector1
fn order_values(
    indx: &mut [usize],
    lo: usize,
    hi: usize,
    k: &Key<'_>,
    decreasing: bool,
    loc: Locale,
) -> Result<(), CmpErr> {
    let t = first_inc(hi - lo + 1);
    match k {
        Key::Int(x) => sort2_with_index(indx, lo, hi, t, |a, b| {
            Ok(if decreasing { x[a] < x[b] || (x[a] == x[b] && a > b) } else { x[a] > x[b] || (x[a] == x[b] && a > b) })
        }),
        Key::Real(x) => sort2_with_index(indx, lo, hi, t, |a, b| {
            Ok(if decreasing { x[a] < x[b] || (x[a] == x[b] && a > b) } else { x[a] > x[b] || (x[a] == x[b] && a > b) })
        }),
        Key::Cplx(x) => sort2_with_index(indx, lo, hi, t, |a, b| {
            let c = ccmp(x[a], x[b], false);
            let eq = x[a].r == x[b].r && x[a].i == x[b].i && a > b;
            Ok(if decreasing { c < 0 || eq } else { c > 0 || eq })
        }),
        Key::Str(x) => sort2_with_index(indx, lo, hi, t, |a, b| {
            let c = collate(x[a], x[b], loc)?;
            Ok(if decreasing { c < 0 || (c == 0 && a > b) } else { c > 0 || (c == 0 && a > b) })
        }),
        Key::Other(t) => Err(CmpErr::Type(t, "greater")),
    }
}

/// `.Internal(order(na.last, decreasing, ...))`.
// R: sort.c:do_order
pub fn do_order(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // GNU R reads CAR and CADR of what it has (asLogical(NULL) is NA)
    let nalast = if args.is_empty() { NA_INTEGER } else { as_logical(cx, args.get(cx, 0)) };
    if nalast == NA_INTEGER {
        return Err(cx.error("invalid 'na.last' value"));
    }
    let decreasing = if args.len() < 2 { NA_INTEGER } else { as_logical(cx, args.get(cx, 1)) };
    if decreasing == NA_INTEGER {
        return Err(cx.error("'decreasing' must be TRUE or FALSE"));
    }
    let (nalast, decreasing) = (nalast != 0, decreasing != 0);
    if args.len() == 2 {
        return cx.build(out, |cx| Ok(cx.nil()));
    }
    let nkeys = args.len() - 2;
    let n0 = args.get(cx, 2);
    let n = if crate::attrib::is_vector(n0) { Some(crate::coerce::xlength(n0)) } else { None };
    for k in 0..nkeys {
        let x = args.get(cx, 2 + k);
        if !crate::attrib::is_vector(x) {
            return Err(cx.error(&format!("argument {} is not a vector", k + 1)));
        }
        if Some(crate::coerce::xlength(x)) != n {
            return Err(cx.error("argument lengths differ"));
        }
        if x.is_package_altrep() {
            return Err(not_yet(cx, Some(call), "order() of a package ALTREP vector"));
        }
    }
    let n = n.unwrap_or(0);
    let mut indx: Vec<usize> = (0..n).collect();
    if n != 0 {
        let (na, loc) = (cx.na_string(), cx.locale());
        let r = if nkeys == 1 {
            order_vector1(&mut indx, args.get(cx, 2), nalast, decreasing, na, loc)
        } else {
            let keys: Vec<Key<'_>> = (0..nkeys).map(|k| Key::of(args.get(cx, 2 + k))).collect();
            order_vector(&mut indx, &keys, nalast, decreasing, na, loc)
        };
        if let Err(e) = r {
            return Err(e.raise(cx, call));
        }
    }
    let ans: Vec<i32> = indx.iter().map(|&i| i as i32 + 1).collect();
    cx.build(out, |cx| Ok(cx.alloc_copy(&ans)?.finish().into()))
}

/// `.gt(x, i, j)` in `rho` (1-based `i`, `j`) as `asInteger` reads it: how
/// `rank` compares the elements of an object.
// R: sort.c:greater, equal (isObject(x) && !isNull(rho))
fn gt_call(cx: &mut Cx, x: Root<'_>, i: usize, j: usize, rho: Root<'_>) -> RResult<i32> {
    cx.scope(|cx, s| {
        let call = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let sj = cx.alloc_copy(&[j as i32 + 1])?.finish().into();
                let tail = cx.cons(sj, cx.nil())?;
                let si = cx.alloc_copy(&[i as i32 + 1])?.finish().into();
                let tail = cx.cons(si, tail)?;
                let tail = cx.cons(x.bind(cx), tail)?;
                let f = cx.install(b".gt")?;
                cx.lcons(f.into(), tail)
            })
        })?;
        let r = s.capture(cx, |cx, o| cx.eval(call, rho, o))?;
        let (v, _) = crate::coerce::vector::as_integer(cx, r.bind(cx));
        Ok(v)
    })
}

/// `.Internal(rank(x, length(x), ties.method))`: `orderVector1` with `rho`
/// (no NAs moved first; an object compared by `.gt`), then each run of
/// equal elements ranked.
// R: sort.c:do_rank
pub fn do_rank(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("rank", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    let xv = x.bind(cx);
    if xv.sexptype() == RAWSXP && !xv.is_object() {
        return Err(cx.error("raw vectors cannot be sorted"));
    }
    let sn = args.get(cx, 1);
    let n: usize = if sn.sexptype() == REALSXP {
        let d = sn.as_vector::<f64>().map_or(f64::NAN, |v| if v.is_empty() { f64::NAN } else { v.get(0) });
        if d.is_nan() {
            return Err(cx.error("vector size cannot be NA/NaN"));
        }
        if !d.is_finite() {
            return Err(cx.error("vector size cannot be infinite"));
        }
        if d > 4503599627370496.0 {
            return Err(cx.error("vector size specified is too large"));
        }
        if d < 0.0 {
            return Err(cx.error("invalid 'length(xx)' value"));
        }
        d as usize
    } else {
        let (nn, _) = crate::coerce::vector::as_integer(cx, sn);
        if nn == NA_INTEGER || nn < 0 {
            return Err(cx.error("invalid 'length(xx)' value"));
        }
        nn as usize
    };
    let ties = as_char_bytes(args.get(cx, 2));
    #[derive(Clone, Copy, PartialEq)]
    enum Ties {
        Average,
        Max,
        Min,
    }
    let ties = match ties.as_slice() {
        b"average" => Ties::Average,
        b"max" => Ties::Max,
        b"min" => Ties::Min,
        _ => return Err(cx.error("invalid ties.method for rank() [should never happen]")),
    };
    if n > crate::coerce::xlength(xv) {
        // GNU R reads past the vector
        return Err(not_yet(cx, Some(call), ".Internal(rank()) of more elements than the vector has"));
    }
    if xv.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "rank() of a package ALTREP vector"));
    }
    let mut indx: Vec<usize> = (0..n).collect();
    // runs of equal elements, as `equal` finds them
    let mut eq: Vec<bool> = Vec::new();
    // orderVector1 returns at once for fewer than two
    if n > 1 {
        if xv.is_object() {
            // greater(a, b): c > 0 || (c == 0 && b < a); equal: c == 0
            let t = first_inc(n);
            sort2_with_index(&mut indx, 0, n - 1, t, |a, b| {
                let c = gt_call(cx, x, a, b, env)?;
                Ok::<_, RError>(c > 0 || (c == 0 && b < a))
            })?;
            for w in 0..n - 1 {
                eq.push(gt_call(cx, x, indx[w], indx[w + 1], env)? == 0);
            }
        } else {
            let (na, loc) = (cx.na_string(), cx.locale());
            let k = Key::of(x.bind(cx));
            let r = order_values(&mut indx, 0, n - 1, &k, false, loc).and_then(|()| {
                (0..n - 1)
                    .map(|w| Ok(k.cmp(indx[w], indx[w + 1], true, na, loc, "equal")? == 0))
                    .collect::<Result<Vec<bool>, CmpErr>>()
            });
            match r {
                Ok(e) => eq = e,
                Err(e) => return Err(e.raise(cx, call)),
            }
        }
    }
    // the runs: equal() is asked only up to the end of each
    let mut real = vec![0f64; if ties == Ties::Average { n } else { 0 }];
    let mut int = vec![0i32; if ties == Ties::Average { 0 } else { n }];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n - 1 && eq[j] {
            j += 1;
        }
        for k in i..=j {
            match ties {
                Ties::Average => real[indx[k]] = (i as f64 + j as f64 + 2.0) / 2.0,
                Ties::Max => int[indx[k]] = j as i32 + 1,
                Ties::Min => int[indx[k]] = i as i32 + 1,
            }
        }
        i = j + 1;
    }
    cx.build(out, |cx| {
        Ok(if ties == Ties::Average {
            cx.alloc_copy(&real)?.finish().into()
        } else {
            cx.alloc_copy(&int)?.finish().into()
        })
    })
}

/// `.Internal(psort(x, partial))`: a copy of `x` without attributes, the
/// positions `partial` (1-based) each in their place.
// R: sort.c:do_psort
pub fn do_psort(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("psort", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    if !crate::util::is_vector_atomic(x) {
        return Err(cx.error("only atomic vectors can be sorted"));
    }
    if x.sexptype() == RAWSXP {
        return Err(cx.error("raw vectors cannot be sorted"));
    }
    if x.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "psort() of a package ALTREP vector"));
    }
    let n = crate::coerce::xlength(x);
    // coerceVector(p, INTSXP), its warnings given
    let p = as_int_vec(cx, args.root(1))?;
    let mut ind = Vec::with_capacity(p.len());
    for &l in &p {
        if l == NA_INTEGER {
            return Err(cx.error("NA index"));
        }
        if l < 1 || l as usize > n {
            return Err(cx.error(&format!("'partial' index {l} outside bounds")));
        }
        ind.push(l as usize);
    }
    let x = args.get(cx, 0);
    let t = x.sexptype();
    let (na, loc) = (cx.na_string(), cx.locale());
    let mut vals = match x.view() {
        View::Lgl(_) | View::Int(_) => {
            let Key::Int(v) = Key::of(x) else { unreachable!() };
            Vals::Int(v.into_owned())
        }
        View::Real(v) => Vals::Real(v.as_slice().unwrap_or(&[]).to_vec()),
        View::Cplx(v) => Vals::Cplx(v.as_slice().unwrap_or(&[]).to_vec()),
        View::Str(_) => Vals::Str((0..n).collect()),
        _ => unreachable!("robigo: atomic, not raw"),
    };
    let strs: &[Char<'_>] = match x.view() {
        View::Str(v) => v.as_slice().unwrap_or(&[]),
        _ => &[],
    };
    let mut cmp_str = |a: usize, b: usize| -> Result<i32, CmpErr> { Ok(scmp(strs[a], strs[b], true, na, loc)?) };
    if let Err(e) = psort0(&mut vals, &mut cmp_str, 0, n as isize - 1, &ind) {
        return Err(e.raise(cx, call));
    }
    build_vals(cx, out, vals, t, args.root(0))
}

/// A copy of `x` (type `t`) holding `vals`, without attributes.
fn build_vals(cx: &mut Cx, out: Out<'_>, vals: Vals, t: u8, x: Root<'_>) -> RResult<()> {
    cx.build(out, |cx| {
        Ok(match vals {
            Vals::Int(ref v) if t == LGLSXP => {
                cx.alloc_copy(&v.iter().map(|&i| Lgl(i)).collect::<Vec<_>>())?.finish().into()
            }
            Vals::Int(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Real(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Cplx(ref v) => cx.alloc_copy(v)?.finish().into(),
            Vals::Str(ref p) => {
                let x = x.bind(cx).as_vector::<Char<'_>>().unwrap();
                let mut s = cx.alloc_vector::<Char<'_>>(p.len())?;
                for (i, &k) in p.iter().enumerate() {
                    s.set(i, x.get(k));
                }
                s.finish().into()
            }
        })
    })
}

/// Singleton's quicksort (ACM algorithm 347, with R's changes) of
/// `v[i..=j]` (1-based), and `ix` alongside when given.
// R: qsort-body.c
fn r_qsort<T: Copy + PartialOrd>(v: &mut [T], mut ix: Option<&mut [i32]>, i: usize, j: usize) {
    // 1-based access, as the body has it after `--v`
    macro_rules! v {
        ($k:expr) => {
            v[$k - 1]
        };
    }
    let swap_ix = |ix: &mut Option<&mut [i32]>, a: usize, b: usize| {
        if let Some(ix) = ix.as_deref_mut() {
            ix.swap(a - 1, b - 1);
        }
    };
    let (mut i, mut j) = (i as isize, j as isize);
    let (mut il, mut iu) = ([0isize; 40], [0isize; 40]);
    let mut r = 0.375f64;
    let ii = i;
    let mut m = 1usize;
    let (mut k, mut l);
    // the labels: L10 (a segment), L20 (partition), L80 (pop), L100
    // (insertion sort of a short segment)
    #[derive(PartialEq)]
    enum At {
        L10,
        L20,
        L80,
        Tail,
    }
    let mut at = At::L10;
    loop {
        match at {
            At::L10 => {
                if i < j {
                    if r < 0.5898437 {
                        r += 0.0390625;
                    } else {
                        r -= 0.21875;
                    }
                    at = At::L20;
                } else {
                    at = At::L80;
                }
                continue;
            }
            At::L20 => {
                k = i;
                let ij = i + ((j - i) as f64 * r) as isize;
                let (iu_, ij_, ju) = (i as usize, ij as usize, j as usize);
                let mut vt = v!(ij_);
                if v!(iu_) > vt {
                    v!(ij_) = v!(iu_);
                    v!(iu_) = vt;
                    vt = v!(ij_);
                    swap_ix(&mut ix, ij_, iu_);
                }
                l = j;
                if v!(ju) < vt {
                    v!(ij_) = v!(ju);
                    v!(ju) = vt;
                    vt = v!(ij_);
                    swap_ix(&mut ix, ij_, ju);
                    if v!(iu_) > vt {
                        v!(ij_) = v!(iu_);
                        v!(iu_) = vt;
                        vt = v!(ij_);
                        swap_ix(&mut ix, ij_, iu_);
                    }
                }
                loop {
                    loop {
                        l -= 1;
                        if !(l > i - 1 && v!(l as usize) > vt) {
                            break;
                        }
                    }
                    let vtt = v!(l as usize);
                    loop {
                        k += 1;
                        if !(k < j + 1 && v!(k as usize) < vt) {
                            break;
                        }
                    }
                    if k > l {
                        break;
                    }
                    v!(l as usize) = v!(k as usize);
                    v!(k as usize) = vtt;
                    swap_ix(&mut ix, l as usize, k as usize);
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
                at = At::Tail;
                continue;
            }
            At::L80 => {
                if m == 1 {
                    return;
                }
                i = il[m];
                j = iu[m];
                m -= 1;
                at = At::Tail;
                continue;
            }
            At::Tail => {
                if j - i > 10 {
                    at = At::L20;
                    continue;
                }
                if i == ii {
                    at = At::L10;
                    continue;
                }
                // L100: insertion sort of v[i..=j], v[i - 1] the sentinel
                i -= 1;
                loop {
                    i += 1;
                    if i == j {
                        break;
                    }
                    let vt = v!(i as usize + 1);
                    if v!(i as usize) <= vt {
                        continue;
                    }
                    let it = ix.as_deref().map(|x| x[i as usize]);
                    let mut k = i;
                    loop {
                        v!(k as usize + 1) = v!(k as usize);
                        if let Some(x) = ix.as_deref_mut() {
                            x[k as usize] = x[k as usize - 1];
                        }
                        k -= 1;
                        if !(k >= 1 && vt < v!(k as usize)) {
                            break;
                        }
                    }
                    v!(k as usize + 1) = vt;
                    if let (Some(x), Some(it)) = (ix.as_deref_mut(), it) {
                        x[k as usize] = it;
                    }
                }
                at = At::L80;
                continue;
            }
        }
    }
}

/// `.Internal(qsort(x, index.return))`: a numeric `x` without attributes,
/// sorted by Singleton's quicksort; with the index, `list(x =, ix =)`.
// R: qsort.c:do_qsort
pub fn do_qsort(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("qsort", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    if !crate::util::is_numeric(x) {
        return Err(cx.error("argument is not a numeric vector"));
    }
    if x.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "qsort() of a package ALTREP vector"));
    }
    let t = x.sexptype();
    let index = as_logical(cx, args.get(cx, 1));
    // NA is TRUE here: `if(indx_ret)`
    let index = index != 0;
    let n = crate::coerce::xlength(x);
    let mut ix: Vec<i32> = (1..=n as i32).collect();
    let mut vals = match x.view() {
        View::Real(v) => Vals::Real(v.as_slice().unwrap_or(&[]).to_vec()),
        _ => {
            let Key::Int(v) = Key::of(x) else { unreachable!() };
            Vals::Int(v.into_owned())
        }
    };
    if n > 0 {
        let ixs = if index { Some(&mut ix[..]) } else { None };
        match &mut vals {
            Vals::Real(v) => r_qsort(v, ixs, 1, n),
            Vals::Int(v) => r_qsort(v, ixs, 1, n),
            _ => unreachable!(),
        }
    }
    if !index {
        return build_vals(cx, out, vals, t, args.root(0));
    }
    cx.scope(|cx, s| {
        let sx = s.out(cx);
        let sxr = sx.root();
        build_vals(cx, sx, vals, t, args.root(0))?;
        cx.build(out, |cx| {
            let mut l = cx.alloc_vector::<Ref<'_>>(2)?;
            l.set(0, sxr.bind(cx));
            l.set(1, cx.alloc_copy(&ix)?.finish().into());
            let mut nm = cx.alloc_vector::<Char<'_>>(2)?;
            nm.set(0, cx.mk_char(b"x", robigo_rtcore::Encoding::Native)?);
            nm.set(1, cx.mk_char(b"ix", robigo_rtcore::Encoding::Native)?);
            let mut a = crate::attrib::Attrs::new();
            a.install(crate::attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: nm.finish().into() });
            a.apply(cx, &mut l)?;
            Ok(l.finish().into())
        })
    })
}

/// `.Internal(sorted_fpass(x, decr, nalast))`: whether `x` is known to be
/// sorted so (ALTREP metadata, which robigo's vectors do not carry), or is
/// an increasing integer vector without NAs.
// R: sort.c:do_sorted_fpass, fastpass_sortcheck, makeSortEnum
pub fn do_sorted_fpass(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("sorted_fpass", true).check_arity(cx, call, args)?;
    let (decr, _) = crate::coerce::vector::as_integer(cx, args.get(cx, 1));
    let x = args.get(cx, 0);
    if x.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "sorted_fpass() of a package ALTREP vector (its Is_sorted method)"));
    }
    // makeSortEnum: NA `decr` is UNKNOWN_SORTEDNESS, which is never a
    // wanted sort; otherwise increasing is positive
    let done = decr != NA_INTEGER
        && decr == 0
        && match x.view() {
            View::Int(v) => {
                let v = v.as_slice().unwrap_or(&[]);
                !v.is_empty() && v[0] != NA_INTEGER && v.windows(2).all(|w| w[1] >= w[0] && w[1] != NA_INTEGER)
            }
            _ => false,
        };
    cx.build_scalar(out, Lgl(done as i32))
}

const XTFRM: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("xtfrm");

/// `xtfrm(x)`: an internal generic whose default is the closure
/// `xtfrm.default`.
// R: sort.c:do_xtfrm
pub fn do_xtfrm(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("xtfrm").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let robigo_rtcore::Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &XTFRM, call, args, env, out)?
    else {
        return Ok(());
    };
    cx.scope(|cx, s| {
        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"xtfrm.default")?.into())))?;
        let f = s.capture(cx, |cx, o| cx.find_fun3(sym, env, Some(call), o))?;
        cx.apply(call, f, args, env, out)
    })
}

/// `.Internal(wrap_meta(x, srt, no_na))`: `x` itself (GNU R wraps it in an
/// ALTREP wrapper that records its sortedness; robigo has no wrappers, and
/// the values are the same), after GNU R's checks.
// R: altclasses.c:do_wrap_meta, wrap_meta
pub fn do_wrap_meta(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("wrap_meta", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let wrappable = matches!(x.sexptype(), INTSXP | REALSXP | LGLSXP | CPLXSXP | RAWSXP | STRSXP | VECSXP);
    // an object with attributes is returned as it is, unchecked
    if wrappable && !crate::attrib::has_attrib(x) {
        let (srt, _) = crate::coerce::vector::as_integer(cx, args.get(cx, 1));
        let (no_na, _) = crate::coerce::vector::as_integer(cx, args.get(cx, 2));
        if !matches!(srt, -2 | -1 | 0 | 1 | 2 | NA_INTEGER) {
            return Err(cx.error("srt must be -2, -1, 0, or +1, +2, or NA"));
        }
        if !(0..=1).contains(&no_na) {
            return Err(cx.error("no_na must be 0 or +1"));
        }
    }
    out.set(cx, args.get(cx, 0));
    Ok(())
}

/// `CHAR(asChar(x))` as far as `rank` reads it: a string's first element,
/// a symbol's name; anything else matches no ties method.
// R: coerce.c:asChar
fn as_char_bytes(x: Ref<'_>) -> Vec<u8> {
    match x.view() {
        View::Str(v) if !v.is_empty() => v.get(0).bytes().to_vec(),
        View::Sym(s) => s.name().bytes().to_vec(),
        _ => Vec::new(),
    }
}

/// `coerceVector(p, INTSXP)`'s values, its warnings given.
fn as_int_vec(cx: &mut Cx, p: Root<'_>) -> RResult<Vec<i32>> {
    let (v, w) = cx.scope(|cx, s| {
        let o = s.out(cx);
        let r = o.root();
        let w = cx.build_with(o, |cx| crate::coerce::vector::coerce(cx, p.bind(cx), INTSXP, false))?;
        let v = r.bind(cx).as_ints().unwrap_or(&[]).to_vec();
        Ok((v, w))
    })?;
    w.signal(cx)?;
    Ok(v)
}
