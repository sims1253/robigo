//! builtin.c's `.Internal(vector(mode, length))` and
//! `.Internal(environment(fun))`, and util.c's `nargs()`.

use crate::attrib;
use crate::coerce::vector::{as_real, coerce};
use crate::registry;
use crate::util::{is_vector_atomic, not_yet, str2type};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

/// `asVecSize(x)`: a vector size from the first element of an integer,
/// double or character vector; `Err` is GNU R's error message, `Ok(None)`
/// its -999 (the caller's "invalid argument").
// R: builtin.c:asVecSize
pub fn as_vec_size(cx: &Cx, x: Ref<'_>) -> Result<Option<usize>, &'static str> {
    if !is_vector_atomic(x) || x.len().unwrap_or(0) < 1 {
        return Ok(None);
    }
    let d = match x.view() {
        View::Int(v) => {
            let r = v.get(0);
            if r == i32::MIN {
                return Err("vector size cannot be NA");
            }
            return Ok((r >= 0).then_some(r as usize));
        }
        View::Real(v) => v.get(0),
        View::Str(_) => as_real(cx, x).0,
        _ => return Ok(None),
    };
    if d.is_nan() {
        return Err("vector size cannot be NA/NaN");
    }
    if !d.is_finite() {
        return Err("vector size cannot be infinite");
    }
    if d > 4503599627370496.0 {
        return Err("vector size specified is too large");
    }
    // (R_xlen_t) d: a negative size is the caller's error
    Ok(if d <= -1.0 { None } else { Some(d as usize) })
}

/// `.Internal(vector(mode, length))`.
// R: builtin.c:do_makevector
pub fn do_makevector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("vector", true).unwrap().check_arity(cx, call, args)?;
    let lenx = args.get(cx, 1);
    if crate::coerce::xlength(lenx) != 1 {
        return Err(cx.error("invalid 'length' argument"));
    }
    let len = match as_vec_size(cx, lenx) {
        Err(m) => return Err(cx.error(m)),
        Ok(None) => return Err(cx.error("invalid 'length' argument")),
        Ok(Some(n)) => n,
    };
    // coerceVector(mode, STRSXP)
    let mode = cx.scope(|cx, s| {
        let m = s.out(cx);
        let mr = m.root();
        let w = cx.build_with(m, |cx| coerce(cx, args.get(cx, 0), STRSXP, true))?;
        w.signal(cx)?;
        let v = mr.bind(cx);
        match v.as_vector::<Char<'_>>() {
            Some(sv) if sv.len() == 1 => Ok(sv.get(0).bytes().to_vec()),
            _ => Err(cx.error("invalid 'mode' argument")),
        }
    })?;
    let ty = str2type(&mode);
    match ty {
        Some(LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | EXPRSXP | VECSXP | RAWSXP) => {}
        Some(LISTSXP) => {
            if len > i32::MAX as usize {
                return Err(cx.error("too long for a pairlist"));
            }
            return cx.build(out, |cx| {
                let mut l = cx.nil();
                for _ in 0..len {
                    l = cx.cons(cx.nil(), l)?;
                }
                Ok(l)
            });
        }
        _ => {
            let msg = format!("vector: cannot make a vector of mode '{}'.", String::from_utf8_lossy(&mode));
            return Err(cx.error(&msg));
        }
    }
    cx.build(out, |cx| {
        Ok(match ty.unwrap() {
            LGLSXP => cx.alloc_vector::<Lgl>(len)?.finish().into(),
            INTSXP => cx.alloc_vector::<i32>(len)?.finish().into(),
            REALSXP => cx.alloc_vector::<f64>(len)?.finish().into(),
            CPLXSXP => cx.alloc_vector::<Rcomplex>(len)?.finish().into(),
            RAWSXP => cx.alloc_vector::<u8>(len)?.finish().into(),
            STRSXP => cx.alloc_vector::<Char<'_>>(len)?.finish().into(),
            EXPRSXP => cx.alloc_expression(len)?.finish().into(),
            _ => cx.alloc_vector::<Ref<'_>>(len)?.finish().into(),
        })
    })
}

/// `.Internal(environment(fun))`: a closure's environment, the calling
/// frame for NULL, else the `.Environment` attribute.
// R: builtin.c:do_envir
pub fn do_envir(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("environment", true).unwrap().check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let r = match x.view() {
        View::Closure(c) => c.env(),
        View::Null => cx.global_context().sysparent(),
        _ => attrib::get_value(x, b".Environment").unwrap_or(cx.nil()),
    };
    out.set(cx, r);
    Ok(())
}

/// `nargs()`: the number of arguments supplied to the call whose frame is
/// the caller's environment.
// R: util.c:do_nargs
pub fn do_nargs(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("nargs").unwrap().check_arity(cx, call, args)?;
    let n = cx.nargs(env.bind(cx));
    cx.build(out, |cx| Ok(cx.alloc_copy(&[n])?.finish().into()))
}

/// `length(x)`: an internal generic; a method's double result that is a
/// whole number in the integer range becomes an integer.
// R: array.c:do_length
pub fn do_length(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("length").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    if args.get(cx, 0).is_object() {
        static LENGTH: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("length");
        let res = out.root();
        match cx.try_dispatch(&LENGTH, call, args, env, out)? {
            robigo_rtcore::Dispatched::Done => {
                let d = match res.bind(cx).view() {
                    View::Real(v) if v.len() == 1 => Some(v.get(0)),
                    _ => None,
                };
                if let Some(d) = d
                    && d.is_finite()
                    && d >= 0.0
                    && d <= i32::MAX as f64
                    && d.floor() == d
                {
                    let o = res.reset(cx);
                    cx.build(o, |cx| Ok(cx.alloc_copy(&[d as i32])?.finish().into()))?;
                }
                return Ok(());
            }
            robigo_rtcore::Dispatched::Default(o) => return length_dflt(cx, call, args.root(0), o),
        }
    }
    length_dflt(cx, call, args.root(0), out)
}

fn length_dflt(cx: &mut Cx, call: Root<'_>, x: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = match length(cx, x.bind(cx)) {
        Some(n) => n,
        None => return Err(not_yet(cx, Some(call), "length() of this environment")),
    };
    cx.build(out, |cx| {
        if n > i32::MAX as usize {
            Ok(cx.alloc_copy(&[n as f64])?.finish().into())
        } else {
            Ok(cx.alloc_copy(&[n as i32])?.finish().into())
        }
    })
}

/// GNU R's `xlength(x)`: a vector's length, a pairlist's or call's cells,
/// an environment's bindings (`Rf_envxlength`, all of them), 1 for any
/// other object. None for an environment robigo cannot list.
// R: Rinlinedfuns.h:xlength, envir.c:Rf_envxlength
pub fn length(cx: &Cx, x: Ref<'_>) -> Option<usize> {
    Some(match x.view() {
        View::Env(e) => cx.frame_bindings(e, true)?.len(),
        _ => crate::coerce::xlength(x),
    })
}
