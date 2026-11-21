//! split.c: `.Internal(split(x, f))` (`do_split`, with split-incl.c's two
//! passes): each level's elements of `x` in order (the factor recycled),
//! their names with them, each piece of `x`'s type carrying `x`'s `levels`
//! attribute, the list named by the factor's levels.

use crate::util::not_yet;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Rcomplex, Ref, Root, Scope};

const NA_INTEGER: i32 = i32::MIN;

/// A vector of `x`'s type whose elements are `x`'s at `idx`; None for a
/// type split-incl.c's switch does not take.
fn pick<'g>(cx: &'g Cx, x: Ref<'g>, idx: &[usize]) -> Result<Option<Ref<'g>>, robigo_rtcore::NeedGc> {
    macro_rules! plain {
        ($ty:ty) => {{
            let xs = x.as_vector::<$ty>().unwrap();
            Some(cx.alloc_with::<$ty>(idx.len(), |k| xs.get(idx[k]))?.finish().into())
        }};
    }
    Ok(match x.sexptype() {
        LGLSXP => plain!(Lgl),
        INTSXP => plain!(i32),
        REALSXP => plain!(f64),
        CPLXSXP => plain!(Rcomplex),
        RAWSXP => plain!(u8),
        STRSXP => {
            let xs = x.as_vector::<Char<'_>>().unwrap();
            let mut r = cx.alloc_vector::<Char<'_>>(idx.len())?;
            for (k, &i) in idx.iter().enumerate() {
                r.set(k, xs.get(i));
            }
            Some(r.finish().into())
        }
        VECSXP => {
            let xs = x.as_vector::<Ref<'_>>().unwrap();
            let mut r = cx.alloc_vector::<Ref<'_>>(idx.len())?;
            for (k, &i) in idx.iter().enumerate() {
                r.set(k, xs.get(i));
            }
            Some(r.finish().into())
        }
        _ => None,
    })
}

/// `.Internal(split(x, f))`.
// R: split.c:do_split, split-incl.c
pub fn do_split(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("split", true).check_arity(cx, call, args)?;
    let (x, f) = (args.root(0), args.root(1));
    if !crate::attrib::is_vector(x.bind(cx)) {
        return Err(cx.error("first argument must be a vector"));
    }
    let fv = f.bind(cx);
    if !(fv.sexptype() == INTSXP && crate::attrib::inherits(fv, b"factor")) {
        return Err(cx.error("second argument must be a factor"));
    }
    if x.bind(cx).is_package_altrep() || fv.is_package_altrep() {
        return Err(not_yet(cx, Some(call), "split() of a package ALTREP vector"));
    }
    cx.scope(|cx, s| split(cx, s, x, f, out))
}

fn split<'s>(cx: &mut Cx, s: Scope<'s>, x: Root<'_>, f: Root<'_>, out: Out<'_>) -> RResult<()> {
    // (a compact sequence, as split.default makes with seq_along, read as
    // its elements)
    cx.materialize(x)?;
    cx.materialize(f)?;
    // nlevels(f): length(getAttrib(f, R_LevelsSymbol))
    let levels = crate::attrib::get_value(f.bind(cx), b"levels");
    let nlevs = levels.map_or(0, |l| crate::coerce::xlength(l));
    let nfac = crate::coerce::xlength(f.bind(cx));
    let nobs = crate::coerce::xlength(x.bind(cx));
    if nfac == 0 && nobs > 0 {
        return Err(cx.error("group length is 0 but data length > 0"));
    }
    if nfac > 0 && nobs % nfac != 0 {
        cx.warning("data length is not a multiple of split variable")?;
    }
    // the first pass: each level's elements (MOD_ITERATE1: f recycled)
    let mut idx: Vec<Vec<usize>> = vec![Vec::new(); nlevs];
    {
        let fs = f.bind(cx);
        let fs = fs.as_vector::<i32>().unwrap();
        for i in 0..if nfac > 0 { nobs } else { 0 } {
            let j = fs.get(i % nfac);
            if j != NA_INTEGER {
                // protect against malformed factors
                if j < 1 || j as usize > nlevs {
                    return Err(cx.error("factor has bad level"));
                }
                idx[j as usize - 1].push(i);
            }
        }
    }
    // the second pass's copy, for a type its switch takes; another type is
    // an error once an element would be copied
    let t = x.bind(cx).sexptype();
    let takes = matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | RAWSXP);
    if !takes && idx.iter().any(|v| !v.is_empty()) {
        let msg = format!("unimplemented type '{}' in 'split'\n", crate::util::type2char(t));
        return Err(cx.error(&msg));
    }
    let x_levels = crate::attrib::get_value(x.bind(cx), b"levels").map(|l| s.root(cx, l));
    // getAttrib(x, R_NamesSymbol): a one-dimensional array's dimnames[[1]]
    let names = crate::attrib::names(x.bind(cx)).map(|n| s.root(cx, n));
    let vec = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.alloc_vector::<Ref<'_>>(nlevs)?.finish().into())))?;
    for (j, ix) in idx.iter().enumerate() {
        let piece = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let xv = x.bind(cx);
                match pick(cx, xv, ix)? {
                    Some(v) => Ok(v),
                    // (no element: allocVector(TYPEOF(x), 0))
                    None if t == EXPRSXP => Ok(cx.alloc_expression(0)?.finish().into()),
                    None => Ok(cx.alloc_vector::<Ref<'_>>(0)?.finish().into()),
                }
            })
        })?;
        if let Some(l) = x_levels {
            crate::attrib::set::set_attrib(cx, piece, b"levels", l)?;
        }
        if let Some(nm) = names {
            let nmj = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let src = nm.bind(cx).as_vector::<Char<'_>>().unwrap();
                    let mut r = cx.alloc_vector::<Char<'_>>(ix.len())?;
                    for (k, &i) in ix.iter().enumerate() {
                        r.set(k, src.get(i));
                    }
                    Ok(r.finish().into())
                })
            })?;
            crate::attrib::set::set_attrib(cx, piece, b"names", nmj)?;
        }
        // (vec is fresh: no other reference sees the write)
        cx.set_vector_elt_shared(vec, j, piece)?;
    }
    // setAttrib(vec, R_NamesSymbol, getAttrib(f, R_LevelsSymbol))
    if let Some(l) = crate::attrib::get_value(f.bind(cx), b"levels") {
        let l = s.root(cx, l);
        crate::attrib::set::set_attrib(cx, vec, b"names", l)?;
    }
    out.set(cx, vec.bind(cx));
    Ok(())
}
