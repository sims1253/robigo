//! builtin.c's `.Internal`s over the isolate's environments: `environment`
//! (`do_envir`) and `new.env` (`do_newenv`).

use crate::util::as_integer;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Out, RResult, Root};

/// `.Internal(environment(fun))`: a closure's environment, the calling
/// frame for NULL (`R_GlobalContext->sysparent`), else the object's
/// `.Environment` attribute.
// R: builtin.c:do_envir
pub fn do_envir(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("environment", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let v = match x.sexptype() {
        CLOSXP => match x.view() {
            robigo_rtcore::View::Closure(c) => c.env(),
            _ => unreachable!(),
        },
        NILSXP => cx.global_context().sysparent(),
        _ => crate::attrib::get_value(x, b".Environment").unwrap_or(cx.nil()),
    };
    out.set(cx, v);
    Ok(())
}

/// `.Internal(new.env(hash, parent, size))`.
// R: builtin.c:do_newenv
pub fn do_newenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("new.env", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let hash = as_integer(cx, args.root(0))?;
    let enclos = args.get(cx, 1);
    if enclos.sexptype() == NILSXP {
        return Err(cx.error("use of NULL environment is defunct"));
    }
    if enclos.sexptype() != ENVSXP {
        // (simple_as_environment: an S4 object's data slot)
        if enclos.is_s4() && enclos.sexptype() == OBJSXP {
            return Err(crate::util::not_yet(cx, None, "an S4 environment object as 'enclos'"));
        }
        return Err(cx.error("'enclos' must be an environment"));
    }
    let size = if hash != 0 {
        let s = as_integer(cx, args.root(2))?;
        if s == i32::MIN { 0 } else { s }
    } else {
        0
    };
    cx.build(out, |cx| cx.new_env(args.get(cx, 1), hash != 0, size))
}

/// `asVecSize(x)`: a vector length from the first element (-999 for
/// none), with GNU R's errors.
// R: builtin.c:asVecSize
fn as_vec_size(cx: &mut Cx, x: Root<'_>) -> RResult<i64> {
    const XLEN_MAX: f64 = 4503599627370496.0;
    let v = x.bind(cx);
    let real = |cx: &mut Cx, d: f64| -> RResult<i64> {
        if d.is_nan() {
            return Err(cx.error("vector size cannot be NA/NaN"));
        }
        if !d.is_finite() {
            return Err(cx.error("vector size cannot be infinite"));
        }
        if d > XLEN_MAX {
            return Err(cx.error("vector size specified is too large"));
        }
        Ok(d as i64)
    };
    if crate::util::is_vector_atomic(v) && v.len().unwrap_or(0) >= 1 {
        match v.view() {
            robigo_rtcore::View::Int(i) => {
                let r = i.get(0);
                if r == i32::MIN {
                    return Err(cx.error("vector size cannot be NA"));
                }
                return Ok(r as i64);
            }
            robigo_rtcore::View::Real(d) => {
                let d = d.get(0);
                return real(cx, d);
            }
            robigo_rtcore::View::Str(_) => {
                return Err(crate::util::not_yet(cx, None, "a character vector size (asReal)"));
            }
            _ => {}
        }
    }
    Ok(-999)
}

/// `.Internal(vector(mode, length))`: a vector of zeros, FALSE, "" or
/// NULLs.
// R: builtin.c:do_makevector
pub fn do_makevector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("vector", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    if crate::array::r_length(cx, args.get(cx, 1)) != 1 {
        return Err(cx.error("invalid 'length' argument"));
    }
    let len = as_vec_size(cx, args.root(1))?;
    if len < 0 {
        return Err(cx.error("invalid 'length' argument"));
    }
    let len = len as usize;
    let mode_s = {
        let m = args.get(cx, 0);
        match crate::coerce::Strings::of(cx, m) {
            Ok(s) if s.len() == 1 => s.get_or_na(0).bytes().to_vec(),
            Ok(_) => return Err(cx.error("invalid 'mode' argument")),
            Err(e) => return Err(cx.error(&e.message())),
        }
    };
    let mode = crate::util::str2type(&mode_s).or(if mode_s == b"double" { Some(REALSXP) } else { None });
    match mode {
        Some(LGLSXP) => cx.build(out, |cx| Ok(cx.alloc_with(len, |_| robigo_rtcore::Lgl(0))?.finish().into())),
        Some(INTSXP) => cx.build(out, |cx| Ok(cx.alloc_with(len, |_| 0i32)?.finish().into())),
        Some(REALSXP) => cx.build(out, |cx| Ok(cx.alloc_with(len, |_| 0f64)?.finish().into())),
        Some(CPLXSXP) => {
            cx.build(out, |cx| Ok(cx.alloc_with(len, |_| robigo_rtcore::Rcomplex { r: 0.0, i: 0.0 })?.finish().into()))
        }
        Some(RAWSXP) => cx.build(out, |cx| Ok(cx.alloc_with(len, |_| 0u8)?.finish().into())),
        Some(STRSXP) => cx.build(out, |cx| {
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(len)?;
            for i in 0..len {
                v.set(i, cx.blank_string());
            }
            Ok(v.finish().into())
        }),
        Some(VECSXP) => cx.build(out, |cx| Ok(cx.alloc_vector::<robigo_rtcore::Ref<'_>>(len)?.finish().into())),
        Some(EXPRSXP) => cx.build(out, |cx| Ok(cx.alloc_expression(len)?.finish().into())),
        Some(LISTSXP) => {
            if len > i32::MAX as usize {
                return Err(cx.error("too long for a pairlist"));
            }
            cx.build(out, |cx| {
                let mut l = cx.nil();
                for _ in 0..len {
                    l = cx.cons(cx.nil(), l)?;
                }
                Ok(l)
            })
        }
        _ => {
            let msg = format!("vector: cannot make a vector of mode '{}'.", String::from_utf8_lossy(&mode_s));
            Err(cx.error(&msg))
        }
    }
}
