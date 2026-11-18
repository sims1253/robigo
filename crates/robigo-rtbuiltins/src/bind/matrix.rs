//! bind.c's default `cbind` and `rbind`: what `do_bind` does once no
//! method was found for its arguments (robigo-rt's `bind::do_bind` makes
//! the promises, dispatches and names each argument). The answer type is
//! `c()`'s (`AnswerType`); the fill and the dimnames are `cbind`'s and
//! `rbind`'s.

use super::{Bind, answer_type, mode};
use crate::attrib::{self, Attr, Attrs};
use crate::util::{not_yet, type2char};
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, Lgl, NeedGc, Out, RResult, Rcomplex, Ref, Root, Scope, View};

/// One argument of `cbind`/`rbind`'s default, as `do_bind` hands it over:
/// its value (`PRVALUE(CAR(t))`) and its label, the column (row) name a
/// vector argument gives (`PRINTNAME(TAG(t))`, else the symbol of
/// `substitute(CAR(t))` with `deparse.level` 1, else its deparse with
/// `deparse.level` 2; None when none of those applies).
#[derive(Clone, Copy)]
pub struct BindArg<'r> {
    pub value: Root<'r>,
    pub label: Option<Root<'r>>,
}

/// `isMatrix(u)`: a vector whose `dim` is an integer vector of length 2.
fn is_matrix(u: Ref<'_>) -> bool {
    attrib::is_vector(u) && attrib::dims(u).is_some_and(|d| d.len() == 2)
}

/// `length(getAttrib(u, R_DimSymbol)) == 2`: the matrix's two extents.
fn dim2(u: Ref<'_>) -> Option<(usize, usize)> {
    match attrib::dims(u) {
        Some(&[r, c]) => Some((r as usize, c as usize)),
        _ => None,
    }
}

/// `length(u)`.
fn length(u: Ref<'_>) -> usize {
    crate::coerce::xlength(u)
}

/// `getAttrib(u, R_DimNamesSymbol)` when it is a list of two (GNU R's
/// `GetRowNames`, `GetColNames` of it).
fn dimnames2(u: Ref<'_>) -> Option<(Ref<'_>, Ref<'_>)> {
    let dn = attrib::get_value(u, b"dimnames")?;
    match dn.view() {
        View::List(l) if l.len() == 2 => Some((l.get(0), l.get(1))),
        _ => None,
    }
}

/// The type `do_bind` builds, or its error for an expression.
// R: bind.c:do_bind
fn bind_mode(cx: &mut Cx, args: &[BindArg<'_>]) -> RResult<Option<u8>> {
    let mut d = Bind::default();
    for a in args {
        answer_type(a.value.bind(cx), false, false, &mut d);
    }
    // zero-extent matrices shouldn't give NULL, but cbind(NULL) should
    if d.flags == 0 && d.len == 0 {
        return Ok(None);
    }
    let m = mode(d.flags);
    match m {
        NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | RAWSXP => {}
        _ => return Err(cx.error(&format!("cannot create a matrix from type '{}'", type2char(m)))),
    }
    if m == NILSXP {
        // (no argument gives it: a type bit always comes with a length)
        return Err(not_yet(cx, None, "a NULL matrix from cbind()/rbind()"));
    }
    Ok(Some(m))
}

/// `cbind`/`rbind`'s default (`cbind` when `cbind`) of `args`, written to
/// `out`; NULL when every argument is NULL.
// R: bind.c:do_bind, cbind, rbind
pub fn bind_default(cx: &mut Cx, cbind: bool, args: &[BindArg<'_>], out: Out<'_>) -> RResult<()> {
    for a in args {
        let v = a.value.bind(cx);
        if v.is_package_altrep() {
            let what = if cbind { "cbind() of a package ALTREP vector" } else { "rbind() of a package ALTREP vector" };
            return Err(not_yet(cx, None, what));
        }
        if matches!(v.sexptype(), LISTSXP | LANGSXP) && v.as_cons().is_some_and(|c| has_tags(c)) {
            // getAttrib(u, R_NamesSymbol) of a pairlist is its tags
            return Err(not_yet(cx, None, "cbind()/rbind() of a pairlist with tags"));
        }
    }
    let Some(mode) = bind_mode(cx, args)? else {
        out.set(cx, cx.nil());
        return Ok(());
    };
    for a in args {
        cx.materialize(a.value)?;
    }
    if cbind { do_cbind(cx, args, mode, out) } else { do_rbind(cx, args, mode, out) }
}

fn has_tags(mut c: robigo_rtcore::Cons<'_>) -> bool {
    loop {
        if c.tag().sexptype() != NILSXP {
            return true;
        }
        match c.cdr().as_cons() {
            Some(n) => c = n,
            None => return false,
        }
    }
}

/// The shape and the dimnames' plan of a `cbind` (`rbind`, with rows and
/// columns swapped) of the arguments.
struct Plan {
    rows: usize,
    cols: usize,
    lenmin: usize,
    have_rnames: bool,
    have_cnames: bool,
}

/// `cbind`'s (`rbind`'s, `cbind` false) first three passes: the zero-row
/// (zero-column) case, the matrices' conformability, the vectors' and the
/// dimnames. Errors and the recycling warning are GNU R's.
// R: bind.c:cbind, rbind
fn plan(cx: &mut Cx, args: &[BindArg<'_>], cbind: bool) -> RResult<Plan> {
    // check if we are in the zero-row (zero-cols) case
    let lenmin = args
        .iter()
        .any(|a| {
            let u = a.value.bind(cx);
            let n = if is_matrix(u) {
                let (r, c) = dim2(u).unwrap();
                if cbind { r } else { c }
            } else {
                length(u)
            };
            n > 0
        })
        .into();
    // check conformability of matrix arguments
    let (mut rows, mut cols) = (0usize, 0usize);
    let mut m: Option<usize> = None;
    for (na, a) in args.iter().enumerate() {
        let u = a.value.bind(cx);
        if let Some((r, c)) = dim2(u) {
            let (this, other) = if cbind { (r, c) } else { (c, r) };
            match m {
                None => m = Some(this),
                Some(m) if m != this => {
                    let msg = if cbind {
                        format!("number of rows of matrices must match (see arg {})", na + 1)
                    } else {
                        format!("number of columns of matrices must match (see arg {})", na + 1)
                    };
                    return Err(cx.error(&msg));
                }
                _ => {}
            }
            if cbind { cols += other } else { rows += other }
        } else if length(u) >= lenmin {
            if cbind {
                rows = rows.max(length(u));
                cols += 1;
            } else {
                cols = cols.max(length(u));
                rows += 1;
            }
        }
    }
    if let Some(m) = m {
        if cbind { rows = m } else { cols = m }
    }
    // the extent the vectors recycle to: rows (cbind) or cols (rbind)
    let ext = if cbind { rows } else { cols };
    // Check conformability of vector arguments. -- Look for dimnames.
    let (mut have_this, mut have_other) = (false, false);
    let (mut mnames, mut nnames) = (0usize, 0usize);
    let mut warned = false;
    for (na, a) in args.iter().enumerate() {
        let u = a.value.bind(cx);
        if dim2(u).is_some() {
            if let Some((rn, cn)) = dimnames2(u) {
                let (along, across) = if cbind { (cn, rn) } else { (rn, cn) };
                if along.sexptype() != NILSXP {
                    have_this = true;
                }
                if across.sexptype() != NILSXP {
                    mnames = m.unwrap_or(0);
                }
            }
        } else {
            let k = length(u);
            if !warned && k > 0 && (k > ext || ext % k != 0) {
                warned = true;
                let msg = if cbind {
                    format!("number of rows of result is not a multiple of vector length (arg {})", na + 1)
                } else {
                    format!("number of columns of result is not a multiple of vector length (arg {})", na + 1)
                };
                cx.warning(&msg)?;
            }
            if k >= lenmin && a.label.is_some() {
                have_this = true;
            }
            let u = a.value.bind(cx);
            nnames = nnames.max(attrib::names(u).map_or(0, length));
        }
    }
    if mnames != 0 || nnames == ext {
        have_other = true;
    }
    let (have_rnames, have_cnames) = if cbind { (have_other, have_this) } else { (have_this, have_other) };
    Ok(Plan { rows, cols, lenmin, have_rnames, have_cnames })
}

/// Each argument the fill uses (a matrix, or a vector at least `lenmin`
/// long) coerced to `mode` (`coerceVector`), with whether it is a matrix
/// and its rows; for a list result, a value of a type `coerceVector`
/// does not take into a list is the value itself (`cbind`'s `default:`
/// case; `rbind` coerces all).
fn coerced<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    args: &[BindArg<'_>],
    mode: u8,
    lenmin: usize,
    cbind: bool,
) -> RResult<Vec<Option<(Root<'s>, bool, usize)>>> {
    let mut v = Vec::with_capacity(args.len());
    for a in args {
        let u = a.value.bind(cx);
        let mat = is_matrix(u);
        if !(mat || length(u) >= lenmin) {
            v.push(None);
            continue;
        }
        let urows = if mat { dim2(u).unwrap().0 } else { 1 };
        let keep = cbind
            && mode == VECSXP
            && !matches!(
                u.sexptype(),
                NILSXP | LANGSXP | RAWSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | LISTSXP
            );
        let c = if keep || u.sexptype() == mode {
            s.root(cx, u)
        } else {
            s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, a.value, mode, o))?
        };
        if !keep {
            cx.materialize(c)?;
        }
        v.push(Some((c, mat, urows)));
    }
    Ok(v)
}

/// A destination of the fill: one element of the result set from element
/// `si` of the source `src` (both of the result's type).
fn put<'g>(r: &mut Fill<'g>, di: usize, src: Ref<'g>, si: usize) {
    match (r, src.view()) {
        (Fill::Lgl(r), View::Lgl(v)) => r.as_mut_slice()[di] = v.as_slice().unwrap()[si],
        (Fill::Int(r), View::Int(v)) => r.as_mut_slice()[di] = v.as_slice().unwrap()[si],
        (Fill::Real(r), View::Real(v)) => r.as_mut_slice()[di] = v.as_slice().unwrap()[si],
        (Fill::Cplx(r), View::Cplx(v)) => r.as_mut_slice()[di] = v.as_slice().unwrap()[si],
        (Fill::Raw(r), View::Raw(v)) => r.as_mut_slice()[di] = v.as_slice().unwrap()[si],
        (Fill::Str(r), View::Str(v)) => r.set(di, v.get(si)),
        (Fill::List(r), View::List(v)) => r.set(di, v.get(si)),
        // a value cbind puts into a list whole
        (Fill::List(r), _) => r.set(di, src),
        _ => unreachable!("robigo: a bind source coerced to the result's type"),
    }
}

/// The result vector being filled.
enum Fill<'g> {
    Lgl(robigo_rtcore::FreshVec<'g, Lgl>),
    Int(robigo_rtcore::FreshVec<'g, i32>),
    Real(robigo_rtcore::FreshVec<'g, f64>),
    Cplx(robigo_rtcore::FreshVec<'g, Rcomplex>),
    Raw(robigo_rtcore::FreshVec<'g, u8>),
    Str(robigo_rtcore::FreshVec<'g, Char<'g>>),
    List(robigo_rtcore::FreshVec<'g, Ref<'g>>),
}

impl<'g> Fill<'g> {
    /// `allocMatrix(mode, rows, cols)`'s vector (its `dim` set by the
    /// caller). An atomic result's elements are all written; a list's and
    /// a character vector's start as NULL and "".
    fn alloc(cx: &'g Cx, mode: u8, n: usize) -> Result<Fill<'g>, NeedGc> {
        Ok(match mode {
            LGLSXP => Fill::Lgl(cx.alloc_vector(n)?),
            INTSXP => Fill::Int(cx.alloc_vector(n)?),
            REALSXP => Fill::Real(cx.alloc_vector(n)?),
            CPLXSXP => Fill::Cplx(cx.alloc_vector(n)?),
            RAWSXP => Fill::Raw(cx.alloc_vector(n)?),
            STRSXP => Fill::Str(cx.alloc_vector(n)?),
            _ => Fill::List(cx.alloc_vector(n)?),
        })
    }

    fn finish(self, cx: &'g Cx, a: &Attrs<'g>) -> Result<Ref<'g>, NeedGc> {
        macro_rules! f {
            ($v:ident) => {{
                let mut v = $v;
                a.apply(cx, &mut v)?;
                v.finish().into()
            }};
        }
        Ok(match self {
            Fill::Lgl(v) => f!(v),
            Fill::Int(v) => f!(v),
            Fill::Real(v) => f!(v),
            Fill::Cplx(v) => f!(v),
            Fill::Raw(v) => f!(v),
            Fill::Str(v) => f!(v),
            Fill::List(v) => f!(v),
        })
    }
}

/// The `dim` attribute `c(rows, cols)`.
fn dim_attr<'g>(cx: &'g Cx, rows: usize, cols: usize) -> Result<Attrs<'g>, NeedGc> {
    let mut a = Attrs::new();
    let d = cx.alloc_copy(&[rows as i32, cols as i32])?.finish().into();
    a.install(Attr { tag: cx.sym(RSym::Dim), value: d });
    Ok(a)
}

/// `cbind(call, args, mode, rho, deparse_level)`.
// R: bind.c:cbind
fn do_cbind(cx: &mut Cx, args: &[BindArg<'_>], mode: u8, out: Out<'_>) -> RResult<()> {
    let p = plan(cx, args, true)?;
    if p.rows.checked_mul(p.cols).is_none() {
        return Err(not_yet(cx, None, "a cbind() result this large"));
    }
    cx.scope(|cx, s| {
        let src = coerced(cx, s, args, mode, p.lenmin, true)?;
        let result = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut r = Fill::alloc(cx, mode, p.rows * p.cols)?;
                let mut n = 0usize;
                for &(c, mat, _) in src.iter().flatten() {
                    let u = c.bind(cx);
                    // k: the source's length; idx: the elements it fills
                    // (a vector recycled to the rows)
                    let k = match u.view() {
                        View::List(_) | View::Str(_) | View::Lgl(_) | View::Int(_) | View::Real(_) | View::Cplx(_)
                        | View::Raw(_) => u.len().unwrap(),
                        // (a value a list takes whole: one per row)
                        _ => 1,
                    };
                    let whole = !matches!(
                        u.view(),
                        View::List(_) | View::Str(_) | View::Lgl(_) | View::Int(_) | View::Real(_) | View::Cplx(_)
                            | View::Raw(_)
                    );
                    let idx = if mat { k } else { p.rows };
                    if k > 0 {
                        for i in 0..idx {
                            if whole {
                                if let Fill::List(r) = &mut r {
                                    r.set(n + i, u);
                                }
                            } else {
                                put(&mut r, n + i, u, i % k);
                            }
                        }
                    }
                    n += idx;
                }
                let a = dim_attr(cx, p.rows, p.cols)?;
                r.finish(cx, &a)
            })
        })?;
        if p.have_cnames || p.have_rnames {
            let dn = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut nam = if p.have_cnames { Some(cx.alloc_vector::<Char<'_>>(p.cols)?) } else { None };
                    let mut rn: Option<Ref<'_>> = None;
                    let mut j = 0;
                    for a in args {
                        let u = a.value.bind(cx);
                        if is_matrix(u) {
                            let v = dimnames2(u);
                            if p.have_rnames
                                && rn.is_none()
                                && let Some((r, _)) = v
                                && r.sexptype() != NILSXP
                            {
                                rn = Some(r);
                            }
                            let tnam = v.map(|v| v.1).filter(|t| t.sexptype() != NILSXP);
                            if let Some(t) = tnam {
                                let t = t.as_vector::<Char<'_>>().unwrap();
                                let nam = nam.as_mut().unwrap();
                                for i in 0..t.len() {
                                    nam.set(j, t.get(i));
                                    j += 1;
                                }
                            } else if let Some(nam) = nam.as_mut() {
                                for _ in 0..dim2(u).unwrap().1 {
                                    nam.set(j, cx.blank_string());
                                    j += 1;
                                }
                            }
                        } else if length(u) >= p.lenmin {
                            let un = attrib::names(u);
                            if p.have_rnames
                                && rn.is_none()
                                && let Some(un) = un
                                && length(un) == p.rows
                            {
                                rn = Some(un);
                            }
                            if let Some(l) = a.label {
                                nam.as_mut().unwrap().set(j, l.bind(cx).as_char().unwrap());
                                j += 1;
                            } else if let Some(nam) = nam.as_mut() {
                                nam.set(j, cx.blank_string());
                                j += 1;
                            }
                        }
                    }
                    let mut dn = cx.alloc_vector::<Ref<'_>>(2)?;
                    dn.set(0, rn.unwrap_or(cx.nil()));
                    dn.set(1, nam.map_or(cx.nil(), |n| n.finish().into()));
                    Ok(dn.finish().into())
                })
            })?;
            crate::attrib::set_attrib(cx, result, b"dimnames", dn)?;
        }
        out.set(cx, result.bind(cx));
        Ok(())
    })
}

/// `rbind(call, args, mode, rho, deparse_level)`.
// R: bind.c:rbind
fn do_rbind(cx: &mut Cx, args: &[BindArg<'_>], mode: u8, out: Out<'_>) -> RResult<()> {
    let p = plan(cx, args, false)?;
    if p.rows.checked_mul(p.cols).is_none() {
        return Err(not_yet(cx, None, "an rbind() result this large"));
    }
    cx.scope(|cx, s| {
        let src = coerced(cx, s, args, mode, p.lenmin, false)?;
        let result = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut r = Fill::alloc(cx, mode, p.rows * p.cols)?;
                let mut n = 0usize;
                for &(c, mat, urows) in src.iter().flatten() {
                    let u = c.bind(cx);
                    let k = u.len().unwrap();
                    // srows: the rows this source fills
                    let idx = if mat { urows } else { (k > 0) as usize };
                    // R: duplicate.h:FILL_MATRIX_ITERATE
                    for i in 0..idx {
                        let (mut sidx, mut didx) = (i, n + i);
                        for _ in 0..p.cols {
                            put(&mut r, didx, u, sidx);
                            sidx += idx;
                            if sidx >= k {
                                sidx -= k;
                            }
                            didx += p.rows;
                        }
                    }
                    n += idx;
                }
                let a = dim_attr(cx, p.rows, p.cols)?;
                r.finish(cx, &a)
            })
        })?;
        if p.have_rnames || p.have_cnames {
            let dn = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut nam = if p.have_rnames { Some(cx.alloc_vector::<Char<'_>>(p.rows)?) } else { None };
                    let mut cn: Option<Ref<'_>> = None;
                    let mut j = 0;
                    for a in args {
                        let u = a.value.bind(cx);
                        if is_matrix(u) {
                            let v = dimnames2(u);
                            if p.have_cnames
                                && cn.is_none()
                                && let Some((_, c)) = v
                                && c.sexptype() != NILSXP
                            {
                                cn = Some(c);
                            }
                            let tnam = v.map(|v| v.0).filter(|t| t.sexptype() != NILSXP);
                            if let Some(nam) = nam.as_mut() {
                                if let Some(t) = tnam {
                                    let t = t.as_vector::<Char<'_>>().unwrap();
                                    for i in 0..t.len() {
                                        nam.set(j, t.get(i));
                                        j += 1;
                                    }
                                } else {
                                    for _ in 0..dim2(u).unwrap().0 {
                                        nam.set(j, cx.blank_string());
                                        j += 1;
                                    }
                                }
                            }
                        } else if length(u) >= p.lenmin {
                            let un = attrib::names(u);
                            if p.have_cnames
                                && cn.is_none()
                                && let Some(un) = un
                                && length(un) == p.cols
                            {
                                cn = Some(un);
                            }
                            if let Some(l) = a.label {
                                nam.as_mut().unwrap().set(j, l.bind(cx).as_char().unwrap());
                                j += 1;
                            } else if let Some(nam) = nam.as_mut() {
                                nam.set(j, cx.blank_string());
                                j += 1;
                            }
                        }
                    }
                    let mut dn = cx.alloc_vector::<Ref<'_>>(2)?;
                    dn.set(0, nam.map_or(cx.nil(), |n| n.finish().into()));
                    dn.set(1, cn.unwrap_or(cx.nil()));
                    Ok(dn.finish().into())
                })
            })?;
            crate::attrib::set_attrib(cx, result, b"dimnames", dn)?;
        }
        out.set(cx, result.bind(cx));
        Ok(())
    })
}
