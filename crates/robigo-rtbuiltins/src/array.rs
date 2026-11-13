//! array.c's `length` (`do_length`): an internal generic.

use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Dispatched, Generic, Out, RResult, Root, View};

const LENGTH: Generic = Generic::builtin("length");

/// `length(x)`: dispatched on an object (a method's double value that is a
/// whole count becomes an integer), else GNU R's `length()` of the value.
// R: array.c:do_length
pub fn do_length(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("length", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    crate::is::check1arg(cx, args, call, "x")?;
    let res = out.root();
    let out = match cx.try_dispatch(&LENGTH, call, args, env, out)? {
        Dispatched::Done => {
            let a = res.bind(cx);
            if a.sexptype() == REALSXP && a.len() == Some(1) {
                let d = a.as_vector::<f64>().unwrap().get(0);
                if d.is_finite() && d >= 0.0 && d <= i32::MAX as f64 && d.floor() == d {
                    let o = res.reset(cx);
                    return cx.build(o, |cx| Ok(cx.alloc_copy(&[d as i32])?.finish().into()));
                }
            }
            return Ok(());
        }
        Dispatched::Default(o) => o,
    };
    if args.any_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "'length' of a package ALTREP object"));
    }
    let n = r_length(cx, args.get(cx, 0));
    if n > i32::MAX as usize {
        return cx.build(out, |cx| Ok(cx.alloc_copy(&[n as f64])?.finish().into()));
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&[n as i32])?.finish().into()))
}

/// GNU R's `length(x)` (`xlength`): a vector's length, a pairlist's or
/// call's cell count, an environment's number of bindings (`Rf_envlength`),
/// 1 for anything else.
// R: Rinlinedfuns.h:Rf_length, xlength
pub fn r_length(cx: &Cx, x: robigo_rtcore::Ref<'_>) -> usize {
    match x.sexptype() {
        NILSXP => 0,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP => x.len().unwrap_or(0),
        LISTSXP | LANGSXP | DOTSXP => {
            let mut n = 0;
            let mut c = x;
            while let Some(cell) = c.as_cons() {
                n += 1;
                c = cell.cdr();
            }
            n
        }
        ENVSXP => match x.view() {
            View::Env(e) => cx.frame_bindings(e, true).map_or(0, |b| b.len()),
            _ => 0,
        },
        _ => 1,
    }
}
