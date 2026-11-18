//! apply.c: `.Internal(lapply(X, FUN))`, `.Internal(vapply(X, FUN,
//! FUN.VALUE, USE.NAMES))` and `.Internal(islistfactor(X, recursive))`.
//!
//! `lapply` and `vapply` are specials: their arguments are the symbols `X`
//! and `FUN` of the closure that wraps them, and each element is computed
//! by evaluating the call `FUN(X[[i]], ...)` in that closure's
//! environment, with `i` bound there to the index (written in place, as
//! GNU R does, until the callee captures it). GNU R evaluates that call
//! with `R_forceAndCall(call, 1, rho)`, which forces `X[[i]]` before the
//! callee's body runs: here the `force_and_call` hook, robigo-rt's port.

use crate::attrib::{self, Attr, Attrs};
use crate::coerce::{self, vector::coerce};
use crate::util::{not_yet, type_to_char};
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Rcomplex, Ref, Root, Scope, View};

/// `checkArgIsSymbol(x)`.
fn check_symbol(cx: &mut Cx, x: Root<'_>) -> RResult<()> {
    if x.bind(cx).sexptype() != SYMSXP {
        return Err(cx.error("argument must be a symbol"));
    }
    Ok(())
}

/// The call `FUN(X[[i]], ...)` and the index vector `i` bound in `rho`
/// (and its binding cell), as `do_lapply` and `do_vapply` set them up.
struct Loop<'s> {
    fcall: Root<'s>,
    isym: Root<'s>,
    ind: Root<'s>,
    loc: Root<'s>,
    rho: Root<'s>,
    /// The builtin's protections when it calls `R_forceAndCall`, for the
    /// protect stack's account (robigo_rtcore's ppacct).
    pp: i32,
}

impl<'s> Loop<'s> {
    fn new(cx: &mut Cx, s: Scope<'s>, x: Root<'_>, fun: Root<'_>, rho: Root<'_>, pp: i32) -> RResult<Loop<'s>> {
        let isym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"i")?.into())))?;
        let fcall = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let b2 = cx.install(b"[[")?;
                let dots = cx.sym(RSym::Dots);
                let tmp = cx.lang3(b2.into(), x.bind(cx), isym.bind(cx))?;
                cx.lang3(fun.bind(cx), tmp, dots.into())
            })
        })?;
        // MARK_NOT_MUTABLE(R_fcall): the call is not handed to R code to
        // modify; nothing here writes it
        let ind = s.capture(cx, |cx, o| cx.build_scalar(o, 0i32))?;
        let rho = s.root(cx, rho.bind(cx));
        cx.define_var(isym, ind, rho)?;
        let loc = s.out(cx);
        let locr = loc.root();
        cx.find_var_loc_in_frame(rho, isym, loc)?;
        Ok(Loop { fcall, isym, ind, loc: locr, rho, pp })
    }

    /// Element `i`'s value (`R_forceAndCall(R_fcall, 1, rho)`), written to
    /// `out`; then `i`'s vector made fresh again if the callee captured or
    /// replaced it.
    fn call(&self, cx: &mut Cx, i: usize, out: Out<'_>) -> RResult<()> {
        // `i`'s binding must see the write: GNU R's bare `INTEGER(ind)[0]`
        cx.owned_mut::<i32>(self.ind)?.as_mut_slice()[0] = (i + 1) as i32;
        // (under the builtin's own protections: robigo_rtcore's ppacct)
        cx.with_pp_excess(self.pp, |cx| cx.force_and_call(self.fcall, 1, self.rho, out))?;
        cx.scope(|cx, s| {
            let cur = s.capture(cx, |cx, o| cx.var_loc_value(self.loc, o))?;
            if cur.bind(cx) != self.ind.bind(cx) || cx.shared(self.ind.bind(cx)) {
                // ind has been captured or removed by FUN: fix it up
                let fresh = s.capture(cx, |cx, o| cx.build_scalar(o, 0i32))?;
                self.ind.replace_from(cx, fresh);
                cx.define_var(self.isym, self.ind, self.rho)?;
                let loc = s.capture(cx, |cx, o| cx.find_var_loc_in_frame(self.rho, self.isym, o).map(|_| ()))?;
                self.loc.replace_from(cx, loc);
            }
            Ok(())
        })
    }
}

/// `.Internal(lapply(X, FUN))`.
// R: apply.c:do_lapply
pub fn do_lapply(cx: &mut Cx, call: Root<'_>, args: Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("lapply", true).check_arity(cx, call, args)?;
    let x = args.root(0);
    check_symbol(cx, x)?;
    cx.scope(|cx, s| {
        let xx = s.capture(cx, |cx, o| cx.eval(x, rho, o))?;
        let n = match xx.bind(cx).std_vector() {
            Some(v) => v.len(),
            None if xx.bind(cx).is_package_altrep() => {
                return Err(not_yet(cx, Some(call), "lapply() over a package ALTREP vector"));
            }
            None => coerce::xlength(xx.bind(cx)),
        };
        let fun = args.root(1);
        check_symbol(cx, fun)?;
        // the answer, named by XX's names before FUN runs; each result goes
        // in as FUN returns it (SET_VECTOR_ELT's link), so a later call of
        // FUN that writes a result an earlier one returned copies it
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
                if let Some(nm) = attrib::names(xx.bind(cx)) {
                    let mut a = Attrs::new();
                    a.install(Attr { tag: cx.sym(RSym::Names), value: nm });
                    a.apply(cx, &mut ans)?;
                }
                Ok(ans.finish().into())
            })
        })?;
        // PROTECT(XX), PROTECT(ans), PROTECT(tmp), PROTECT(R_fcall),
        // PROTECT_WITH_INDEX(ind), PROTECT_WITH_INDEX(loc.cell)
        let lp = Loop::new(cx, s, x, fun, rho, 6)?;
        for k in 0..n {
            // PLANT(A4, log): three elements' calls made last first (the
            // results still land in their places)
            let i = if cfg!(feature = "diff-plant") && n == 3 { n - 1 - k } else { k };
            cx.scope(|cx, t| {
                let r = t.capture(cx, |cx, o| lp.call(cx, i, o))?;
                cx.owned_list(ans)?.set(i, r);
                Ok(())
            })?;
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

/// `.Internal(vapply(X, FUN, FUN.VALUE, USE.NAMES))`.
// R: apply.c:do_vapply
pub fn do_vapply(cx: &mut Cx, call: Root<'_>, args: Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("vapply", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let x = args.root(0);
        // (under PROTECT(X), then PROTECT(XX), PROTECT(value))
        let xx = s.capture(cx, |cx, o| cx.with_pp_excess(1, |cx| cx.eval(x, rho, o)))?;
        let fun = args.root(1);
        let value = s.capture(cx, |cx, o| cx.with_pp_excess(2, |cx| cx.eval(args.root(2), rho, o)))?;
        if !attrib::is_vector(value.bind(cx)) {
            return Err(cx.error("'FUN.VALUE' must be a vector"));
        }
        let use_names = {
            let u = s.capture(cx, |cx, o| cx.with_pp_excess(3, |cx| cx.eval(args.root(3), rho, o)))?;
            crate::util::as_logical(cx, u.bind(cx))
        };
        if use_names == i32::MIN {
            return Err(cx.error("invalid 'USE.NAMES' value"));
        }
        let use_names = use_names != 0;
        if xx.bind(cx).is_package_altrep() {
            return Err(not_yet(cx, Some(call), "vapply() over a package ALTREP vector"));
        }
        let n = coerce::xlength(xx.bind(cx));
        let (common_len, common_type, dim_v) = {
            let v = value.bind(cx);
            let dim = attrib::dims(v).map(|d| d.to_vec()).filter(|d| !d.is_empty());
            (v.len().unwrap_or(0), v.sexptype(), dim)
        };
        if !matches!(common_type, CPLXSXP | REALSXP | INTSXP | LGLSXP | RAWSXP | STRSXP | VECSXP) {
            let msg = format!("type '{}' is not supported", type_to_char(value.bind(cx)));
            return Err(cx.error(&msg));
        }
        let array_value = dim_v.is_some();
        // PROTECT(ans = allocVector(commonType, n*commonLen)), filled as
        // FUN returns
        let total = n * common_len;
        let ans = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                Ok(match common_type {
                    CPLXSXP => cx.alloc_vector::<Rcomplex>(total)?.finish().into(),
                    REALSXP => cx.alloc_vector::<f64>(total)?.finish().into(),
                    INTSXP => cx.alloc_vector::<i32>(total)?.finish().into(),
                    LGLSXP => cx.alloc_vector::<Lgl>(total)?.finish().into(),
                    RAWSXP => cx.alloc_vector::<u8>(total)?.finish().into(),
                    STRSXP => cx.alloc_vector::<Char<'_>>(total)?.finish().into(),
                    _ => cx.alloc_vector::<Ref<'_>>(total)?.finish().into(),
                })
            })
        })?;
        // the names and the row names (from FUN.VALUE, else the first
        // result)
        let (names, row_names) = if use_names {
            let xv = xx.bind(cx);
            let nm = match attrib::names(xv) {
                Some(nm) => nm,
                None if xv.sexptype() == STRSXP => xv,
                None => cx.nil(),
            };
            let rn = rownames_of(value.bind(cx), array_value).unwrap_or(cx.nil());
            (s.root(cx, nm), s.root(cx, rn))
        } else {
            (s.root(cx, cx.nil()), s.root(cx, cx.nil()))
        };
        let fun_sym = fun;
        // PROTECT(X), PROTECT(XX), PROTECT(value), PROTECT(ans), with
        // names PROTECT(names) and PROTECT_WITH_INDEX(rowNames), then
        // PROTECT(ind), PROTECT(tmp), PROTECT(R_fcall)
        let lp = Loop::new(cx, s, x, fun_sym, rho, 7 + 2 * use_names as i32)?;
        // each value goes into `ans` as FUN returns it: its data copied, its
        // elements linked (SET_STRING_ELT, SET_VECTOR_ELT), so a later call
        // of FUN that writes a value an earlier one returned writes an
        // object `ans` does not hold
        let mut buf = Buf::default();
        for i in 0..n {
            cx.scope(|cx, t| {
                let val = t.capture(cx, |cx, o| lp.call(cx, i, o))?;
                // (GNU R reads a package ALTREP value through its methods,
                // materializing it: docs/ledger/g3/2026-11-16-c1-link-walk.md,
                // DEFERRED; one of robigo's is read below, as INTEGER(val)
                // and REAL(val) read it)
                if val.bind(cx).is_package_altrep() {
                    return Err(not_yet(cx, Some(call), "vapply() of a package ALTREP result of FUN"));
                }
                let len = coerce::xlength(val.bind(cx));
                if len != common_len {
                    let msg =
                        format!("values must be length {common_len},\n but FUN(X[[{}]]) result is length {len}", i + 1);
                    return Err(cx.error(&msg));
                }
                let vt = val.bind(cx).sexptype();
                let val = if vt != common_type {
                    let okay = match common_type {
                        CPLXSXP => matches!(vt, REALSXP | INTSXP | LGLSXP),
                        REALSXP => matches!(vt, INTSXP | LGLSXP),
                        INTSXP => vt == LGLSXP,
                        _ => false,
                    };
                    if !okay {
                        let msg = format!(
                            "values must be type '{}',\n but FUN(X[[{}]]) result is type '{}'",
                            type_to_char(value.bind(cx)),
                            i + 1,
                            type_to_char(val.bind(cx))
                        );
                        return Err(cx.error(&msg));
                    }
                    let w = t.out(cx);
                    let wr = w.root();
                    let warns = cx.build_with(w, |cx| coerce(cx, val.bind(cx), common_type, true))?;
                    warns.signal(cx)?;
                    wr
                } else {
                    val
                };
                // take row names from the first result only
                if i == 0 && use_names && row_names.bind(cx).sexptype() == NILSXP {
                    let rn = rownames_of(val.bind(cx), array_value).map(|rn| t.root(cx, rn));
                    if let Some(rn) = rn {
                        row_names.replace_from(cx, rn);
                    }
                }
                let off = i * common_len;
                // INTEGER(val), REAL(val), LOGICAL(val): DATAPTR
                cx.materialize(val)?;
                match common_type {
                    CPLXSXP => buf.cplx.copy(cx, ans, off, val),
                    REALSXP => buf.real.copy(cx, ans, off, val),
                    INTSXP => buf.int.copy(cx, ans, off, val),
                    LGLSXP => buf.lgl.copy(cx, ans, off, val),
                    RAWSXP => buf.raw.copy(cx, ans, off, val),
                    _ => {
                        let mut u = cx.owned_list(ans)?;
                        for j in 0..common_len {
                            u.copy_from(off + j, val, j);
                        }
                        Ok(())
                    }
                }
            })?;
        }
        if common_len != 1 {
            let dim: Vec<i32> = match &dim_v {
                Some(d) => d.iter().copied().chain([n as i32]).collect(),
                None => vec![common_len as i32, n as i32],
            };
            set_attr(cx, s, ans, RSym::Dim, |cx| Ok(cx.alloc_copy(&dim)?.finish().into()))?;
        }
        if use_names {
            let rnk_v = dim_v.as_ref().map_or(1, |d| d.len());
            let (nm, rn) = (names.bind(cx).sexptype() != NILSXP, row_names.bind(cx).sexptype() != NILSXP);
            if common_len == 1 {
                if nm {
                    set_attr(cx, s, ans, RSym::Names, |cx| Ok(names.bind(cx)))?;
                }
            } else if nm || rn {
                if array_value && rn {
                    let ok = matches!(row_names.bind(cx).view(), View::List(l) if l.len() == rnk_v);
                    if !ok {
                        let msg = format!("dimnames(<value>) is neither NULL nor list of length {rnk_v}");
                        return Err(cx.error(&msg));
                    }
                }
                set_attr(cx, s, ans, RSym::DimNames, |cx| {
                    let mut dn = cx.alloc_vector::<Ref<'_>>(rnk_v + 1)?;
                    let rnv = row_names.bind(cx);
                    match rnv.view() {
                        View::List(l) if array_value && rn => {
                            for j in 0..rnk_v {
                                dn.set(j, l.get(j));
                            }
                        }
                        _ => dn.set(0, rnv),
                    }
                    dn.set(rnk_v, names.bind(cx));
                    Ok(dn.finish().into())
                })?;
            }
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}

/// `setAttrib(ans, name, value)` on the answer `vapply` built, `value`
/// built by `f`.
fn set_attr<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    ans: Root<'_>,
    name: RSym,
    mut f: impl for<'g> FnMut(&'g Cx) -> Result<Ref<'g>, robigo_rtcore::NeedGc>,
) -> RResult<()> {
    let v = s.capture(cx, |cx, o| cx.build(o, &mut f))?;
    let tag = s.root(cx, cx.sym(name));
    cx.install_attrib(ans, tag, v)
}

/// The values of one plain type `vapply` copies into its answer, through a
/// buffer kept across the calls of FUN (the answer is written while no
/// value of the heap is borrowed).
struct Copier<T>(Vec<T>);

impl<T> Default for Copier<T> {
    fn default() -> Self {
        Copier(Vec::new())
    }
}

impl<T: robigo_rtcore::Plain> Copier<T>
where
    for<'g> T: robigo_rtcore::Elt<'g>,
{
    /// `memcpy(T(ans) + off, T(val), length(val))`.
    fn copy(&mut self, cx: &mut Cx, ans: Root<'_>, off: usize, val: Root<'_>) -> RResult<()> {
        self.0.clear();
        let v = val.bind(cx).as_vector::<T>().expect("robigo: vapply's value is a standard vector of its type");
        match v.as_slice() {
            Some(xs) => self.0.extend_from_slice(xs),
            None => self.0.extend((0..v.len()).map(|j| v.get(j))),
        }
        cx.owned_mut::<T>(ans)?.as_mut_slice()[off..off + self.0.len()].copy_from_slice(&self.0);
        Ok(())
    }
}

/// [`Copier`]s of the plain types.
#[derive(Default)]
struct Buf {
    cplx: Copier<Rcomplex>,
    real: Copier<f64>,
    int: Copier<i32>,
    lgl: Copier<Lgl>,
    raw: Copier<u8>,
}

/// The row names `vapply` takes from a value: its dimnames for an array
/// FUN.VALUE, else its names.
fn rownames_of(v: Ref<'_>, array_value: bool) -> Option<Ref<'_>> {
    if array_value { attrib::get_value(v, b"dimnames") } else { attrib::names(v) }
}

/// GNU R's `isFactor`.
fn is_factor(x: Ref<'_>) -> bool {
    x.sexptype() == INTSXP && attrib::inherits(x, b"factor")
}

/// `islistfactor(X)`: TRUE, FALSE or NA (a list with no leaves).
// R: apply.c:islistfactor
fn islistfactor(x: Ref<'_>) -> i32 {
    match x.view() {
        View::List(v) | View::Expr(v) => {
            let mut ans = i32::MIN;
            for i in 0..v.len() {
                match islistfactor(v.get(i)) {
                    0 => return 0,
                    1 => ans = 1,
                    _ => {}
                }
            }
            ans
        }
        _ => is_factor(x) as i32,
    }
}

/// `.Internal(islistfactor(X, recursive))`.
// R: apply.c:do_islistfactor
pub fn do_islistfactor(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("islistfactor", true).check_arity(cx, call, args)?;
    let recursive = crate::util::as_bool2(cx, call, args.root(1))?;
    let x = args.get(cx, 0);
    let n = coerce::xlength(x);
    let ans = if n == 0 || !matches!(x.sexptype(), VECSXP | EXPRSXP) {
        false
    } else if !recursive {
        let v = x.as_vector::<Ref<'_>>().unwrap();
        (0..v.len()).all(|i| is_factor(v.get(i)))
    } else {
        islistfactor(x) == 1
    };
    cx.build_scalar(out, Lgl(ans as i32))
}

/// `.Internal(mapply(FUN, dots, MoreArgs))`: `FUN` called on the `i`-th
/// elements of the lists in `dots`, recycled to the longest, and the
/// arguments in `MoreArgs`; each call is `R_forceAndCall` of
/// `FUN(dots[[1L]][[i]], dots[[2L]][[i]], ..., <MoreArgs>)` in the calling
/// frame, its index constants written in place between calls.
// R: mapply.c:do_mapply
pub fn do_mapply(cx: &mut Cx, call: Root<'_>, args: Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("mapply", true).check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let f = args.root(0);
        let constant = args.root(2);
        let varying = if args.get(cx, 1).sexptype() != VECSXP {
            // (rarely, hence checking)
            s.capture(cx, |cx, o| coerce::vector::coerce_vector(cx, args.root(1), VECSXP, o))?
        } else {
            args.root(1)
        };
        let m = varying.bind(cx).len().unwrap_or(0);
        let mut lengths: Vec<i64> = Vec::with_capacity(m);
        let (mut zero, mut longest) = (0, 0i64);
        for i in 0..m {
            let tmp1 = s.root(cx, varying.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
            let mut len = match crate::builtin::length_mut(cx, tmp1)? {
                Some(n) => n as i64,
                None => return Err(not_yet(cx, Some(call), "mapply() over an environment robigo cannot list")),
            };
            if tmp1.bind(cx).is_object() {
                // possibly dispatch on length()
                static LENGTH: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("length");
                let r = cx.scope(|cx, s2| {
                    let a = s2.args(cx, &[tmp1.bind(cx)], None);
                    let o = s2.out(cx);
                    let res = o.root();
                    match cx.try_dispatch(&LENGTH, call, a, rho, o)? {
                        robigo_rtcore::Dispatched::Done => {
                            let v = res.bind(cx);
                            Ok(Some(if v.sexptype() == REALSXP {
                                // (R_xlen_t) REAL(ans)[0]
                                v.as_vector::<f64>().unwrap().as_slice().unwrap().first().map_or(0, |&d| d as i64)
                            } else {
                                coerce::vector::as_integer_warn(cx, res)? as i64
                            }))
                        }
                        robigo_rtcore::Dispatched::Default(_) => Ok(None),
                    }
                })?;
                if let Some(n) = r {
                    len = n;
                }
            }
            lengths.push(len);
            if len == 0 {
                zero += 1;
            }
            longest = longest.max(len);
            if zero > 0 && longest > 0 {
                // warning(_("zero-length input leads to zero-length result"));
                return cx.build(out, |cx| Ok(cx.alloc_vector::<Ref<'_>>(0)?.finish().into()));
            }
        }
        let vnames = s.capture(cx, |cx, o| {
            let v = attrib::names(varying.bind(cx)).unwrap_or(cx.nil());
            o.set(cx, v);
            Ok(())
        })?;
        let named = vnames.bind(cx) != cx.nil();
        // build a call like f(dots[[1]][[4]], dots[[2]][[4]], dots[[3]][[4]], d=7)
        let cv = constant.bind(cx);
        let fcall = match cv.sexptype() {
            NILSXP => s.root(cx, cx.nil()),
            VECSXP | EXPRSXP => {
                s.capture(cx, |cx, o| cx.build(o, |cx| coerce::vector::vector_to_pair_list(cx, constant.bind(cx))))?
            }
            // a pairlist passes the check but is not used: fcall stays
            // R_NilValue
            LISTSXP | LANGSXP | DOTSXP => s.root(cx, cx.nil()),
            _ => return Err(cx.error("argument 'MoreArgs' of 'mapply' is not a list or pairlist")),
        };
        let real_indx = longest > i32::MAX as i64;
        let dots = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"dots")?.into())))?;
        let b2 = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"[[")?.into())))?;
        let mut nindex: Vec<Root<'_>> = Vec::with_capacity(m);
        for j in (0..m).rev() {
            let mj = s.capture(cx, |cx, o| cx.build_scalar(o, (j + 1) as i32))?;
            let nj = if real_indx {
                s.capture(cx, |cx, o| cx.build_scalar(o, 0f64))?
            } else {
                s.capture(cx, |cx, o| cx.build_scalar(o, 0i32))?
            };
            nindex.push(nj);
            let tag = if named && !vnames.bind(cx).as_vector::<Char<'_>>().unwrap().get(j).bytes().is_empty() {
                Some(crate::envir::install_tr(cx, s, vnames, j)?)
            } else {
                None
            };
            let cell = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let tmp1 = cx.lang3(b2.bind(cx), dots.bind(cx), mj.bind(cx))?;
                    let tmp2 = cx.lang3(b2.bind(cx), tmp1, nj.bind(cx))?;
                    match tag {
                        Some(t) => cx.lcons_tagged(tmp2, fcall.bind(cx), t.bind(cx).as_sym().unwrap()),
                        None => cx.lcons(tmp2, fcall.bind(cx)),
                    }
                })
            })?;
            fcall.replace_from(cx, cell);
        }
        nindex.reverse();
        let fcall = s.capture(cx, |cx, o| cx.build(o, |cx| cx.lcons(f.bind(cx), fcall.bind(cx))))?;
        let longest = longest as usize;
        let ans = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(longest)?.finish().into())))?;
        let mut counters = vec![0i64; m];
        for i in 0..longest {
            for j in 0..m {
                counters[j] += 1;
                if counters[j] > lengths[j] {
                    counters[j] = 1;
                }
                if real_indx {
                    cx.set_plain_shared(nindex[j], 0, counters[j] as f64)?;
                } else {
                    cx.set_plain_shared(nindex[j], 0, counters[j] as i32)?;
                }
            }
            cx.scope(|cx, t| {
                // (under PROTECT(varyingArgs)?, vnames, mindex, nindex,
                // fcall and ans: robigo_rtcore's ppacct)
                let r = t.capture(cx, |cx, o| cx.with_pp_excess(5, |cx| cx.force_and_call(fcall, m as i32, rho, o)))?;
                // lazy_duplicate of a referenced value: the value itself
                // under reference counting
                cx.owned_list(ans)?.set(i, r);
                Ok(())
            })?;
        }
        for j in 0..m {
            if counters[j] != lengths[j] {
                cx.warning("longer argument not a multiple of length of shorter")?;
            }
        }
        out.set(cx, ans.bind(cx));
        Ok(())
    })
}
