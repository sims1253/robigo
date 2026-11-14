//! apply.c: `.Internal(lapply(X, FUN))`, `.Internal(vapply(X, FUN,
//! FUN.VALUE, USE.NAMES))` and `.Internal(islistfactor(X, recursive))`.
//!
//! `lapply` and `vapply` are specials: their arguments are the symbols `X`
//! and `FUN` of the closure that wraps them, and each element is computed
//! by evaluating the call `FUN(X[[i]], ...)` in that closure's
//! environment, with `i` bound there to the index (written in place, as
//! GNU R does, until the callee captures it). GNU R evaluates that call
//! with `R_forceAndCall(call, 1, rho)`, which forces `X[[i]]` before the
//! callee's body runs; here it runs through the evaluator hook, where a
//! callee that never reads its first argument leaves `X[[i]]` unevaluated
//! (unobservable unless `[[` has a method with effects).

use crate::attrib::{self, Attr, Attrs};
use crate::coerce::{self, vector::coerce};
use crate::registry;
use crate::util::{not_yet, type_to_char};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Built, Char, Cx, Diag, Lgl, Out, RResult, Rcomplex, Ref, Root, Scope, View};

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
}

impl<'s> Loop<'s> {
    fn new(cx: &mut Cx, s: Scope<'s>, x: Root<'_>, fun: Root<'_>, rho: Root<'_>) -> RResult<Loop<'s>> {
        let isym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"i")?.into())))?;
        let fcall = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let b2 = cx.install(b"[[")?;
                let dots = cx.install(b"...")?;
                let tmp = cx.lcons(b2.into(), cx.cons(x.bind(cx), cx.cons(isym.bind(cx), cx.nil())?)?)?;
                cx.lcons(fun.bind(cx), cx.cons(tmp, cx.cons(dots.into(), cx.nil())?)?)
            })
        })?;
        // MARK_NOT_MUTABLE(R_fcall): the call is not handed to R code to
        // modify; nothing here writes it
        let ind = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[0i32])?.finish().into())))?;
        let rho = s.root(cx, rho.bind(cx));
        cx.define_var(isym, ind, rho)?;
        let loc = s.out(cx);
        let locr = loc.root();
        cx.find_var_loc_in_frame(rho, isym, loc)?;
        Ok(Loop { fcall, isym, ind, loc: locr, rho })
    }

    /// Element `i`'s value (`R_forceAndCall(R_fcall, 1, rho)`), written to
    /// `out`; then `i`'s vector made fresh again if the callee captured or
    /// replaced it.
    fn call(&self, cx: &mut Cx, i: usize, out: Out<'_>) -> RResult<()> {
        {
            let mut u = cx.unique_mut::<i32>(self.ind)?;
            u.as_mut_slice()[0] = (i + 1) as i32;
        }
        cx.eval(self.fcall, self.rho, out)?;
        cx.scope(|cx, s| {
            let cur = s.capture(cx, |cx, o| cx.var_loc_value(self.loc, o))?;
            if cur.bind(cx) != self.ind.bind(cx) || self.ind.bind(cx).maybe_shared() {
                // ind has been captured or removed by FUN: fix it up
                let fresh = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_copy(&[0i32])?.finish().into())))?;
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
    registry::lookup("lapply", true).unwrap().check_arity(cx, call, args)?;
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
        let lp = Loop::new(cx, s, x, fun, rho)?;
        let results: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
        for (i, &slot) in results.iter().enumerate() {
            cx.scope(|cx, t| {
                let r = t.capture(cx, |cx, o| lp.call(cx, i, o))?;
                slot.replace_from(cx, r);
                Ok(())
            })?;
        }
        cx.build(out, |cx| {
            let mut ans = cx.alloc_vector::<Ref<'_>>(n)?;
            for (i, r) in results.iter().enumerate() {
                ans.set(i, r.bind(cx));
            }
            if let Some(nm) = attrib::names(xx.bind(cx)) {
                let mut a = Attrs::new();
                a.install(Attr { tag: cx.install(b"names")?, value: nm });
                a.apply(cx, &mut ans)?;
            }
            Ok(ans.finish().into())
        })
    })
}

/// `.Internal(vapply(X, FUN, FUN.VALUE, USE.NAMES))`.
// R: apply.c:do_vapply
pub fn do_vapply(cx: &mut Cx, call: Root<'_>, args: Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("vapply", true).unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let x = args.root(0);
        let xx = s.capture(cx, |cx, o| cx.eval(x, rho, o))?;
        let fun = args.root(1);
        let value = s.capture(cx, |cx, o| cx.eval(args.root(2), rho, o))?;
        if !attrib::is_vector(value.bind(cx)) {
            return Err(cx.error("'FUN.VALUE' must be a vector"));
        }
        let use_names = {
            let u = s.capture(cx, |cx, o| cx.eval(args.root(3), rho, o))?;
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
        let lp = Loop::new(cx, s, x, fun_sym, rho)?;
        let vals: Vec<Root<'_>> = (0..n).map(|_| s.out(cx).root()).collect();
        for (i, &slot) in vals.iter().enumerate() {
            cx.scope(|cx, t| {
                let val = t.capture(cx, |cx, o| lp.call(cx, i, o))?;
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
                if i == 0 && use_names && row_names.bind(cx).sexptype() == NILSXP {
                    let rn = rownames_of(val.bind(cx), array_value).map(|rn| t.root(cx, rn));
                    if let Some(rn) = rn {
                        row_names.replace_from(cx, rn);
                    }
                }
                slot.replace_from(cx, val);
                Ok(())
            })?;
        }
        cx.build_with(out, |cx| {
            let total = n * common_len;
            let get = |i: usize| vals[i].bind(cx);
            let mut a = Attrs::new();
            let rnk_v = dim_v.as_ref().map_or(1, |d| d.len());
            if common_len != 1 {
                let mut dim: Vec<i32> = match &dim_v {
                    Some(d) => d.clone(),
                    None => vec![common_len as i32],
                };
                dim.push(n as i32);
                a.install(Attr { tag: cx.install(b"dim")?, value: cx.alloc_copy(&dim)?.finish().into() });
            }
            if use_names {
                let nm = names.bind(cx);
                let rn = row_names.bind(cx);
                if common_len == 1 {
                    if nm.sexptype() != NILSXP {
                        a.install(Attr { tag: cx.install(b"names")?, value: nm });
                    }
                } else if nm.sexptype() != NILSXP || rn.sexptype() != NILSXP {
                    let mut dn = cx.alloc_vector::<Ref<'_>>(rnk_v + 1)?;
                    if array_value && rn.sexptype() != NILSXP {
                        match rn.view() {
                            View::List(l) if l.len() == rnk_v => {
                                for j in 0..rnk_v {
                                    dn.set(j, l.get(j));
                                }
                            }
                            _ => {
                                let msg = format!("dimnames(<value>) is neither NULL nor list of length {rnk_v}");
                                return Ok(Built::fail(Diag::error(msg)));
                            }
                        }
                    } else {
                        dn.set(0, rn);
                    }
                    dn.set(rnk_v, nm);
                    a.install(Attr { tag: cx.install(b"dimnames")?, value: dn.finish().into() });
                }
            }
            macro_rules! plain {
                ($t:ty, $view:ident) => {{
                    let mut r = cx.alloc_vector::<$t>(total)?;
                    {
                        let o = r.as_mut_slice();
                        for i in 0..n {
                            if let View::$view(v) = get(i).view() {
                                o[i * common_len..(i + 1) * common_len].copy_from_slice(v.as_slice().unwrap());
                            }
                        }
                    }
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }};
            }
            let r: Ref<'_> = match common_type {
                CPLXSXP => plain!(Rcomplex, Cplx),
                REALSXP => plain!(f64, Real),
                INTSXP => plain!(i32, Int),
                LGLSXP => plain!(Lgl, Lgl),
                RAWSXP => plain!(u8, Raw),
                STRSXP => {
                    let mut r = cx.alloc_vector::<Char<'_>>(total)?;
                    for i in 0..n {
                        let v = get(i).as_vector::<Char<'_>>().unwrap();
                        for j in 0..common_len {
                            r.set(i * common_len + j, v.get(j));
                        }
                    }
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }
                _ => {
                    let mut r = cx.alloc_vector::<Ref<'_>>(total)?;
                    for i in 0..n {
                        let v = get(i).as_vector::<Ref<'_>>().unwrap();
                        for j in 0..common_len {
                            r.set(i * common_len + j, v.get(j));
                        }
                    }
                    a.apply(cx, &mut r)?;
                    r.finish().into()
                }
            };
            Ok(Built::done(r))
        })
    })
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
    registry::lookup("islistfactor", true).unwrap().check_arity(cx, call, args)?;
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
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}
