//! builtin.c's `.Internal(vector(mode, length))` and
//! `.Internal(environment(fun))`, and util.c's `nargs()`.

use crate::attrib;
use crate::coerce::vector::{as_real, coerce, coercion_warning};
use crate::util::{is_vector_atomic, not_yet, str2type};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Lgl, Out, RResult, Rcomplex, Ref, Root, View};

/// `asVecSize(args[i])`: a vector size from the first element of an
/// integer, double or character vector; `Err` is GNU R's error message,
/// `Ok(None)` its -999 (the caller's "invalid argument"). A string is read
/// by `asReal`, which warns "NAs introduced by coercion" first.
// R: builtin.c:asVecSize
pub fn as_vec_size(cx: &mut Cx, args: Args<'_>, i: usize) -> RResult<Result<Option<usize>, &'static str>> {
    let (size, warn) = vec_size(cx, args.get(cx, i));
    coercion_warning(cx, warn)?;
    Ok(size)
}

/// [`as_vec_size`]'s size, and `asReal`'s warning bits.
fn vec_size(cx: &Cx, x: Ref<'_>) -> (Result<Option<usize>, &'static str>, u8) {
    if !is_vector_atomic(x) || x.len().unwrap_or(0) < 1 {
        return (Ok(None), 0);
    }
    let (d, warn) = match x.view() {
        View::Int(v) => {
            let r = v.get(0);
            if r == i32::MIN {
                return (Err("vector size cannot be NA"), 0);
            }
            return (Ok((r >= 0).then_some(r as usize)), 0);
        }
        View::Real(v) => (v.get(0), 0),
        View::Str(_) => as_real(cx, x),
        _ => return (Ok(None), 0),
    };
    (vec_size_of(d), warn)
}

/// The rest of `asVecSize`, on the size as a double.
fn vec_size_of(d: f64) -> Result<Option<usize>, &'static str> {
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
    crate::row!("vector", true).check_arity(cx, call, args)?;
    if crate::coerce::xlength(args.get(cx, 1)) != 1 {
        return Err(cx.error("invalid 'length' argument"));
    }
    let len = match as_vec_size(cx, args, 1)? {
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
        match v.as_scalar::<Char<'_>>() {
            Some(c) => Ok(c.bytes().to_vec()),
            None => Err(cx.error("invalid 'mode' argument")),
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
    crate::row!("environment", true).check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let r = match x.view() {
        View::Closure(c) => c.env(),
        View::Null => cx.global_context().sysparent(),
        _ => attrib::get_value(x, b".Environment").unwrap_or(cx.nil()),
    };
    out.set(cx, r);
    Ok(())
}

/// `.Internal(body(fun))`: a closure's body expression (`BODY_EXPR`: byte
/// code's source); for a primitive NULL, for anything else NULL with a
/// warning.
// R: builtin.c:do_body
pub fn do_body(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("body", true).check_arity(cx, call, args)?;
    let f = args.get(cx, 0);
    match f.view() {
        View::Closure(c) => {
            let b = crate::bytecode::expr(cx, c.body());
            out.set(cx, b);
            Ok(())
        }
        View::Builtin(_) | View::Special(_) => cx.build(out, |cx| Ok(cx.nil())),
        _ => {
            cx.warning_call(Some(call), "argument is not a function")?;
            cx.build(out, |cx| Ok(cx.nil()))
        }
    }
}

/// `.Internal(bodyCode(fun))`: a closure's BODY as it is (its byte code
/// when compiled), else NULL.
// R: builtin.c:do_bodyCode
pub fn do_bodycode(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("bodyCode", true).check_arity(cx, call, args)?;
    let f = args.get(cx, 0);
    let r = match f.view() {
        View::Closure(c) => c.body(),
        _ => cx.nil(),
    };
    out.set(cx, r);
    Ok(())
}

/// `.Internal(parent.env(env))`: an environment's enclosure.
// R: builtin.c:do_parentenv
pub fn do_parentenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("parent.env", true).check_arity(cx, call, args)?;
    let e = args.get(cx, 0);
    let ev = match e.view() {
        View::Env(ev) => ev,
        _ if e.sexptype() == OBJSXP && e.is_s4() => {
            return Err(not_yet(cx, Some(call), "parent.env() of an S4 object's environment"));
        }
        _ => return Err(cx.error("argument is not an environment")),
    };
    if e.sexp() == cx.empty_env().sexp() {
        return Err(cx.error("the empty environment has no parent"));
    }
    let p = ev.enclos();
    out.set(cx, p);
    Ok(())
}

/// `nargs()`: the number of arguments supplied to the call whose frame is
/// the caller's environment.
// R: util.c:do_nargs
pub fn do_nargs(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("nargs").check_arity(cx, call, args)?;
    let n = cx.nargs(env.bind(cx));
    cx.build_scalar(out, n)
}

/// `length(x)`: an internal generic; a method's double result that is a
/// whole number in the integer range becomes an integer.
// R: array.c:do_length
pub fn do_length(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("length").check_arity(cx, call, args)?;
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
                    cx.build_scalar(o, d as i32)?;
                }
                return Ok(());
            }
            robigo_rtcore::Dispatched::Default(o) => return length_dflt(cx, call, args.root(0), o),
        }
    }
    length_dflt(cx, call, args.root(0), out)
}

fn length_dflt(cx: &mut Cx, call: Root<'_>, x: Root<'_>, out: Out<'_>) -> RResult<()> {
    if x.bind(cx).is_package_altrep() {
        return length_altrep(cx, call, x, out);
    }
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

/// [`length_dflt`] of a package ALTREP object: GNU R returns
/// `ScalarInteger(length(x))` after `xlength(x)`, so the class's Length
/// method runs twice, and the second answer is the result.
// R: array.c:do_length, altrep.c:ALTREP_LENGTH
#[inline(never)]
fn length_altrep(cx: &mut Cx, call: Root<'_>, x: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = cx.altrep_length(x)?;
    if n > i32::MAX as usize {
        return cx.build_scalar(out, n as f64);
    }
    match cx.altrep_length(x)? {
        n if n > i32::MAX as usize => Err(not_yet(cx, Some(call), "a long ALTREP length() from length")),
        n => cx.build_scalar(out, n as i32),
    }
}

/// GNU R's `xlength(x)`: a vector's length, a pairlist's or call's cells,
/// an environment's bindings (`Rf_envxlength`, all of them), 1 for any
/// other object. None for an environment robigo cannot list, and for a
/// package ALTREP object, whose length is its class's C method
/// ([`length_mut`]).
// R: Rinlinedfuns.h:xlength, envir.c:Rf_envxlength
pub fn length(cx: &Cx, x: Ref<'_>) -> Option<usize> {
    Some(match x.view() {
        View::Env(e) => cx.frame_bindings(e, true)?.len(),
        _ if x.is_package_altrep() => return None,
        _ => crate::coerce::xlength(x),
    })
}

/// [`length`], with a package ALTREP object's Length method run
/// (`ALTREP_LENGTH`: C that may allocate and re-enter R).
// R: Rinlinedfuns.h:xlength, altrep.c:ALTREP_LENGTH
pub fn length_mut(cx: &mut Cx, x: Root<'_>) -> RResult<Option<usize>> {
    if x.bind(cx).is_package_altrep() {
        return cx.altrep_length(x).map(Some);
    }
    Ok(length(cx, x.bind(cx)))
}

/// `.Internal(formals(fun))`: a closure's formals (the list itself, as
/// GNU R returns it, shared); NULL for a builtin or special, with a
/// warning for anything else.
// R: builtin.c:do_formals
pub fn do_formals(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("formals", true).check_arity(cx, call, args)?;
    let f = args.get(cx, 0);
    match f.view() {
        View::Closure(c) => {
            let formals = c.formals();
            out.set(cx, formals);
            Ok(())
        }
        View::Builtin(_) | View::Special(_) => cx.build(out, |cx| Ok(cx.nil())),
        _ => {
            cx.warning_call(Some(call), "argument is not a function")?;
            cx.build(out, |cx| Ok(cx.nil()))
        }
    }
}

const LENGTH_GETS: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("length<-");

/// `length(x) <- value`: an internal generic; else `xlengthgets(x,
/// asVecSize(value))`: the first elements, padded with NA (`raw(0)`'s 0, a
/// list's NULL), names alike; no other attribute.
// R: builtin.c:do_lengthgets, xlengthgets, asVecSize
pub fn do_lengthgets(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("length<-").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let robigo_rtcore::Dispatched::Default(out) =
        crate::coerce::dispatch_or_eval(cx, &LENGTH_GETS, call, args, env, out)?
    else {
        return Ok(());
    };
    if crate::coerce::xlength(args.get(cx, 1)) != 1 {
        return Err(cx.error("wrong length for 'value' argument"));
    }
    let len: i64 = match as_vec_size(cx, args, 1)? {
        Ok(Some(k)) => k as i64,
        Ok(None) => -999,
        Err(m) => return Err(cx.error(m)),
    };
    let x = args.get(cx, 0);
    let t = x.sexptype();
    if !attrib::is_vector(x) && !matches!(t, LISTSXP | NILSXP) {
        return Err(cx.error("cannot set length of non-(vector or list)"));
    }
    if len < 0 {
        return Err(cx.error("invalid value"));
    }
    let len = len as usize;
    if t == NILSXP && len > 0 {
        cx.warning("length of NULL cannot be changed")?;
    }
    if crate::coerce::xlength(args.get(cx, 0)) == len {
        out.set(cx, args.get(cx, 0));
        return Ok(());
    }
    if args.get(cx, 0).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "length<- of a package ALTREP vector"));
    }
    let x = args.root(0);
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let lenx = crate::coerce::xlength(xv);
        let k = len.min(lenx);
        // the names, cut or padded with ""
        let mut attrs = attrib::Attrs::new();
        if let Some(nm) = attrib::names(xv).and_then(|n| n.as_vector::<Char<'_>>()) {
            if !matches!(t, LISTSXP | NILSXP) {
                let mut n = cx.alloc_vector::<Char<'_>>(len)?;
                let blank = cx.mk_char(b"", robigo_rtcore::Encoding::Native)?;
                for i in 0..len {
                    n.set(i, if i < k { nm.get(i) } else { blank });
                }
                attrs.install(attrib::Attr { tag: cx.sym(robigo_rtcore::RSym::Names), value: n.finish().into() });
            }
        }
        macro_rules! fin {
            ($v:expr) => {{
                let mut f = $v;
                attrs.apply(cx, &mut f)?;
                f.finish().into()
            }};
        }
        Ok(match xv.view() {
            View::Null => cx.nil(),
            View::Lgl(v) => {
                let mut a = vec![Lgl::NA; len];
                a[..k].copy_from_slice(&v.as_slice().unwrap()[..k]);
                fin!(cx.alloc_copy(&a)?)
            }
            View::Int(v) => {
                let mut a = vec![i32::MIN; len];
                a[..k].copy_from_slice(&v.as_slice().unwrap()[..k]);
                fin!(cx.alloc_copy(&a)?)
            }
            View::Real(v) => {
                let mut a = vec![robigo_numerics::NA_REAL; len];
                a[..k].copy_from_slice(&v.as_slice().unwrap()[..k]);
                fin!(cx.alloc_copy(&a)?)
            }
            View::Cplx(v) => {
                let na = Rcomplex { r: robigo_numerics::NA_REAL, i: robigo_numerics::NA_REAL };
                let mut a = vec![na; len];
                a[..k].copy_from_slice(&v.as_slice().unwrap()[..k]);
                fin!(cx.alloc_copy(&a)?)
            }
            View::Raw(v) => {
                let mut a = vec![0u8; len];
                a[..k].copy_from_slice(&v.as_slice().unwrap()[..k]);
                fin!(cx.alloc_copy(&a)?)
            }
            View::Str(v) => {
                let mut a = cx.alloc_vector::<Char<'_>>(len)?;
                let na = cx.na_string();
                for i in 0..len {
                    a.set(i, if i < k { v.get(i) } else { na });
                }
                fin!(a)
            }
            View::List(v) | View::Expr(v) => {
                let mut a = if t == EXPRSXP { cx.alloc_expression(len)? } else { cx.alloc_vector::<Ref<'_>>(len)? };
                for i in 0..k {
                    a.set(i, v.get(i));
                }
                fin!(a)
            }
            View::Pairlist(_) => {
                // the first cars and tags, then NULLs
                let mut cells: Vec<(Ref<'_>, Ref<'_>)> = Vec::new();
                let mut c = xv.as_cons();
                while let Some(cell) = c {
                    cells.push((cell.car(), cell.tag()));
                    c = cell.cdr().as_cons();
                }
                let mut head = cx.nil();
                for i in (0..len).rev() {
                    head = match cells.get(i) {
                        Some(&(car, tag)) => match tag.as_sym() {
                            Some(s) => cx.cons_tagged(car, head, s)?,
                            None => cx.cons(car, head)?,
                        },
                        None => cx.cons(cx.nil(), head)?,
                    };
                }
                head
            }
            _ => unreachable!("robigo: a vector or a pairlist"),
        })
    })
}
