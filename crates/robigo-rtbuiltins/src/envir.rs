//! envir.c's `.Internal`s over the isolate's environments (N1a's entry
//! points on `Cx`): `assign`, `get`, `get0`, `exists` (`do_get`, one
//! function for the three) and `mget`.
//!
//! Not here yet, each an error that says so: a frame number as `envir`
//! (`R_sysframe` needs C2's contexts), environments that are S4 objects
//! (`R_getS4DataSlot`), and the condition class of the missing-argument
//! error (S1 makes it a `missingArgError`; the message is GNU R's).

use crate::attrib::{Attr, Attrs};
use crate::text::to_native;
use crate::util::{as_logical, not_yet, str2type};
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RError, RResult, Ref, Root, Scope, View};

/// GNU R's `FUNSXP`: the mode "function" (closures and primitives).
const FUNSXP: u8 = 99;

/// The `mode` of a lookup (`str2mode`): any value, or values of a type
/// (`FUNSXP` for functions), an S4 object being `OBJSXP` with `s4`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Mode {
    ty: u8,
    s4: bool,
}

impl Mode {
    const ANY: Mode = Mode { ty: ANYSXP, s4: false };
}

/// `str2mode(modestr, &wants_S4)`: None for a name `str2type` does not
/// know.
// R: envir.c:str2mode
fn str2mode(s: &[u8]) -> Option<Mode> {
    match s {
        b"function" => Some(Mode { ty: FUNSXP, s4: false }),
        b"S4" => Some(Mode { ty: OBJSXP, s4: true }),
        _ => str2type(s).map(|ty| Mode { ty, s4: false }),
    }
}

/// `findVar1mode`'s type classes: integer counts as double, every function
/// as a closure.
fn mode_class(t: u8) -> u8 {
    match t {
        INTSXP => REALSXP,
        FUNSXP | BUILTINSXP | SPECIALSXP => CLOSXP,
        t => t,
    }
}

/// `installTrChar(c)`: the symbol named by the native translation of `c`,
/// rooted in `s`, with `install`'s errors.
// R: sysutils.c:installTrChar, names.c:install
fn install_tr<'s>(cx: &mut Cx, s: Scope<'s>, c: Root<'_>, i: usize) -> RResult<Root<'s>> {
    let name: Vec<u8> = {
        let ch = c.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
        match to_native(crate::coerce::text(ch), cx.locale()) {
            Ok(n) => n.into_owned(),
            Err(e) => return Err(cx.error(&e.to_string())),
        }
    };
    install(cx, s, &name)
}

/// `install(name)`, rooted, with its errors.
// R: names.c:install
fn install<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    if name.is_empty() {
        return Err(cx.error("attempt to use zero-length variable name"));
    }
    if name.len() > 10000 {
        return Err(cx.error("variable names are limited to 10000 bytes"));
    }
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| Ok(cx.install(name)?.into()))?;
    Ok(r)
}

/// `simple_as_environment(arg)` failing: an S4 object is robigo's to add
/// (`R_getS4DataSlot`), anything else GNU R's "invalid 'envir' argument".
fn bad_envir(cx: &mut Cx, call: Root<'_>, env: Root<'_>) -> RError {
    let e = env.bind(cx);
    if e.is_s4() && e.sexptype() == OBJSXP {
        not_yet(cx, Some(call), "an S4 object as an environment")
    } else {
        cx.error("invalid 'envir' argument")
    }
}

/// `.Internal(assign(x, value, envir, inherits))`.
// R: envir.c:do_assign
pub fn do_assign(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let def = crate::registry::lookup("assign", true).expect("robigo: assign in the registry");
    def.check_arity(cx, call, args)?;
    let x = args.root(0);
    let n = match x.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => 0,
    };
    if n == 0 {
        return Err(cx.error("invalid first argument"));
    }
    if n > 1 {
        cx.warning("only the first element is used as variable name")?;
    }
    cx.scope(|cx, s| {
        let name = install_tr(cx, s, x, 0)?;
        let (val, aenv) = (args.root(1), args.root(2));
        match aenv.bind(cx).sexptype() {
            NILSXP => return Err(cx.error("use of NULL environment is defunct")),
            ENVSXP => {}
            _ => return Err(bad_envir(cx, call, aenv)),
        }
        let inherits = as_logical(cx, args.get(cx, 3));
        if inherits == i32::MIN {
            return Err(cx.error("invalid 'inherits' argument"));
        }
        if inherits != 0 {
            cx.set_var(name, val, aenv)?;
        } else {
            cx.define_var(name, val, aenv)?;
        }
        out.set(cx, val.bind(cx));
        Ok(())
    })
}

/// `.Internal(delayedAssign(x, value, eval.env, assign.env))`: `x` bound
/// in `assign.env` to a promise of the expression `value` in `eval.env`.
// R: builtin.c:do_delayed
pub fn do_delayed(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let def = crate::registry::lookup("delayedAssign", true).expect("robigo: delayedAssign in the registry");
    def.check_arity(cx, call, args)?;
    let x = args.root(0);
    let n = match x.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => 0,
    };
    if n == 0 {
        return Err(cx.error("invalid first argument"));
    }
    cx.scope(|cx, s| {
        let name = install_tr(cx, s, x, 0)?;
        let (expr, eenv, aenv) = (args.root(1), args.root(2), args.root(3));
        for (e, what) in [(eenv, "eval.env"), (aenv, "assign.env")] {
            match e.bind(cx).sexptype() {
                NILSXP => return Err(cx.error("use of NULL environment is defunct")),
                ENVSXP => {}
                _ => return Err(cx.error(&format!("invalid '{what}' argument"))),
            }
        }
        let p = s.out(cx);
        let prom = p.root();
        cx.build(p, |cx| cx.mk_promise(expr.bind(cx), eenv.bind(cx)))?;
        cx.define_var(name, prom, aenv)?;
        out.set(cx, cx.nil());
        Ok(())
    })
}

/// `findVar1mode(symbol, rho, mode, wants_S4, inherits, doGet)`: the value
/// of the first binding of `sym` from `rho` (outwards if `inherits`) whose
/// value has the mode, forcing promises to tell; R_UnboundValue if none.
/// With mode "any" a promise is returned unforced.
// R: envir.c:findVar1mode
fn find_var1_mode(
    cx: &mut Cx,
    sym: Root<'_>,
    rho: Root<'_>,
    mode: Mode,
    inherits: bool,
    do_get: bool,
    out: Out<'_>,
) -> RResult<()> {
    let want = mode_class(mode.ty);
    cx.scope(|cx, s| {
        let at = s.root(cx, rho.bind(cx));
        let vl = s.out(cx).root();
        loop {
            if at.bind(cx) == cx.empty_env() {
                break;
            }
            let o = vl.reset(cx);
            if !do_get && mode == Mode::ANY {
                let found = cx.exists_var_in_frame(at, sym)?;
                o.set(cx, if found { cx.nil() } else { cx.unbound() });
            } else {
                cx.find_var_in_frame3(at, sym, do_get, o)?;
            }
            if vl.bind(cx) != cx.unbound() {
                if mode == Mode::ANY {
                    out.set(cx, vl.bind(cx));
                    return Ok(());
                }
                if vl.bind(cx).sexptype() == PROMSXP {
                    let p = s.root(cx, vl.bind(cx));
                    let o = vl.reset(cx);
                    cx.force(p, o)?;
                }
                let v = vl.bind(cx);
                if mode_class(v.sexptype()) == want && (want != OBJSXP || v.is_s4() == mode.s4) {
                    out.set(cx, v);
                    return Ok(());
                }
            }
            if !inherits {
                break;
            }
            let next = match at.bind(cx).view() {
                View::Env(e) => e.enclos(),
                _ => unreachable!("robigo: a lookup walks environments"),
            };
            let next = s.root(cx, next);
            let o = at.reset(cx);
            o.set(cx, next.bind(cx));
        }
        out.set(cx, cx.unbound());
        Ok(())
    })
}

/// The mode argument of `get`/`exists` (`CADDR(args)`), checked.
fn mode_arg(cx: &mut Cx, m: Root<'_>) -> RResult<Mode> {
    let s: Option<Vec<u8>> = m
        .bind(cx)
        .as_vector::<Char<'_>>()
        .and_then(|v| v.as_slice())
        .and_then(|s| s.first())
        .map(|c| c.bytes().to_vec());
    let Some(s) = s else {
        return Err(cx.error("invalid 'mode' argument"));
    };
    match str2mode(&s) {
        Some(m) => Ok(m),
        None => Err(cx.error(&format!("invalid 'mode' argument '{}'", String::from_utf8_lossy(&s)))),
    }
}

/// `.Internal(exists(x, envir, mode, inherits))` (`CODE` 0),
/// `.Internal(get(x, envir, mode, inherits))` (1) and
/// `.Internal(get0(x, envir, mode, inherits, value_if_not_exists))` (2).
// R: envir.c:do_get
pub fn do_get<const CODE: u8>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let name = match CODE {
        0 => "exists",
        1 => "get",
        _ => "get0",
    };
    crate::registry::lookup(name, true).expect("robigo: do_get in the registry").check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        // the name: a symbol, or a string
        let x = args.root(0);
        let t1 = match x.bind(cx).view() {
            View::Sym(_) => x,
            View::Str(v) if !v.is_empty() && !v.get(0).bytes().is_empty() => {
                if v.len() > 1 {
                    return Err(cx.error("first argument has length > 1"));
                }
                install_tr(cx, s, x, 0)?
            }
            _ => return Err(cx.error("invalid first argument")),
        };
        let genv = args.root(1);
        match genv.bind(cx).sexptype() {
            ENVSXP => {}
            REALSXP | INTSXP => return Err(not_yet(cx, Some(call), "a frame number as 'envir' (R_sysframe)")),
            NILSXP => return Err(cx.error("use of NULL environment is defunct")),
            _ => return Err(bad_envir(cx, call, genv)),
        }
        let mode = mode_arg(cx, args.root(2))?;
        let inherits = as_logical(cx, args.get(cx, 3));
        if inherits == i32::MIN {
            return Err(cx.error("invalid 'inherits' argument"));
        }
        let rv = s.out(cx);
        let rval = rv.root();
        find_var1_mode(cx, t1, genv, mode, inherits != 0, CODE != 0, rv)?;
        if rval.bind(cx) == cx.missing_arg() {
            let sym = t1.bind(cx).as_sym().unwrap().name().bytes().to_vec();
            let msg = format!("argument \"{}\" is missing, with no default", String::from_utf8_lossy(&sym));
            return Err(cx.error_call(Some(call), &msg));
        }
        let unbound = rval.bind(cx) == cx.unbound();
        match CODE {
            0 => cx.build(out, |cx| Ok(cx.alloc_copy(&[robigo_rtcore::Lgl(!unbound as i32)])?.finish().into())),
            _ if unbound && CODE == 2 => {
                out.set(cx, args.get(cx, 4));
                Ok(())
            }
            _ if unbound => {
                let nm = String::from_utf8_lossy(t1.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
                let msg = if mode == Mode::ANY {
                    format!("object '{nm}' not found")
                } else {
                    let m = args.get(cx, 2).as_vector::<Char<'_>>().unwrap().get(0).bytes().to_vec();
                    format!("object '{nm}' of mode '{}' was not found", String::from_utf8_lossy(&m))
                };
                Err(cx.error(&msg))
            }
            _ => get_value(cx, rval, out),
        }
    })
}

/// `GET_VALUE`: a promise is forced (`eval(rval, genv)`).
fn get_value(cx: &mut Cx, rval: Root<'_>, out: Out<'_>) -> RResult<()> {
    if rval.bind(cx).sexptype() == PROMSXP {
        return cx.force(rval, out);
    }
    out.set(cx, rval.bind(cx));
    Ok(())
}

/// `.Internal(mget(x, envir, mode, ifnotfound, inherits))`: a list of the
/// values, named by `x`.
// R: envir.c:do_mget, envir.c:gfind
pub fn do_mget(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("mget", true).expect("robigo: mget in the registry").check_arity(cx, call, args)?;
    let x = args.root(0);
    let nvals = match x.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("invalid first argument")),
    };
    // GNU R tests element 0's text for every position
    if nvals > 0 && x.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes().is_empty() {
        return Err(cx.error("invalid name in position 1"));
    }
    let envr = args.root(1);
    match envr.bind(cx).sexptype() {
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        ENVSXP => {}
        _ => return Err(cx.error("second argument must be an environment")),
    }
    let mode = args.root(2);
    let nmode = match mode.bind(cx).view() {
        View::Str(v) => v.len(),
        _ => return Err(cx.error("invalid 'mode' argument")),
    };
    if nmode != nvals && nmode != 1 {
        return Err(cx.error("wrong length for 'mode' argument"));
    }
    // coerceVector(ifnotfound, VECSXP): a list, or an atomic vector's
    // elements one by one
    let ifnf = args.root(3);
    let nifnfnd = match ifnf.bind(cx).view() {
        View::List(v) | View::Expr(v) => v.len(),
        View::Null => 0,
        View::Lgl(_) | View::Int(_) | View::Real(_) | View::Cplx(_) | View::Str(_) | View::Raw(_) => {
            ifnf.bind(cx).len().unwrap_or(0)
        }
        _ => return Err(not_yet(cx, Some(call), "this 'ifnotfound' (coerceVector to a list)")),
    };
    if nifnfnd != nvals && nifnfnd != 1 {
        return Err(cx.error("wrong length for 'ifnotfound' argument"));
    }
    let inherits = as_logical(cx, args.get(cx, 4));
    if inherits == i32::MIN {
        return Err(cx.error("invalid 'inherits' argument"));
    }
    cx.scope(|cx, s| {
        let mut vals = Vec::with_capacity(nvals);
        for i in 0..nvals {
            let m = mode.bind(cx).as_vector::<Char<'_>>().unwrap().get(i % nmode).bytes().to_vec();
            let Some(gmode) = str2mode(&m) else {
                return Err(cx.error(&format!("invalid 'mode' argument '{}'", String::from_utf8_lossy(&m))));
            };
            let nf = ifnotfound_elt(cx, s, ifnf, i % nifnfnd)?;
            let r = gfind(cx, s, x, i, envr, gmode, nf, inherits != 0, env)?;
            vals.push(r);
        }
        cx.build(out, |cx| {
            let mut ans = cx.alloc_vector::<Ref<'_>>(nvals)?;
            for (i, r) in vals.iter().enumerate() {
                ans.set(i, r.bind(cx));
            }
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.install(b"names")?, value: x.bind(cx) });
            a.apply(cx, &mut ans)?;
            Ok(ans.finish().into())
        })
    })
}

/// Element `i` of `coerceVector(ifnotfound, VECSXP)`, rooted.
fn ifnotfound_elt<'s>(cx: &mut Cx, s: Scope<'s>, ifnf: Root<'_>, i: usize) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let v = ifnf.bind(cx);
        Ok(match v.view() {
            View::List(l) | View::Expr(l) => l.get(i),
            View::Lgl(l) => cx.alloc_copy(&[l.get(i)])?.finish().into(),
            View::Int(l) => cx.alloc_copy(&[l.get(i)])?.finish().into(),
            View::Real(l) => cx.alloc_copy(&[l.get(i)])?.finish().into(),
            View::Cplx(l) => cx.alloc_copy(&[l.get(i)])?.finish().into(),
            View::Raw(l) => cx.alloc_copy(&[l.get(i)])?.finish().into(),
            View::Str(l) => {
                let mut f = cx.alloc_vector::<Char<'_>>(1)?;
                f.set(0, l.get(i));
                f.finish().into()
            }
            _ => unreachable!("robigo: checked in do_mget"),
        })
    })?;
    Ok(r)
}

/// `gfind(name, env, mode, wants_S4, ifnotfound, inherits, enclos)`: the
/// value, `ifnotfound` (called with the name when it is a function) if
/// there is none; a promise is forced.
// R: envir.c:gfind
#[allow(clippy::too_many_arguments)]
fn gfind<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    x: Root<'_>,
    i: usize,
    env: Root<'_>,
    mode: Mode,
    ifnotfound: Root<'_>,
    inherits: bool,
    enclos: Root<'_>,
) -> RResult<Root<'s>> {
    let t1 = install_tr(cx, s, x, i)?;
    let o = s.out(cx);
    let rval = o.root();
    find_var1_mode(cx, t1, env, mode, inherits, true, o)?;
    if rval.bind(cx) == cx.unbound() {
        let is_fun = matches!(ifnotfound.bind(cx).sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP);
        let o = rval.reset(cx);
        if is_fun {
            // LCONS(ifnotfound, LCONS(mkString(name), R_NilValue))
            let c = s.out(cx);
            let fcall = c.root();
            cx.build(c, |cx| {
                let ch = x.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
                let name = to_native(crate::coerce::text(ch), cx.locale()).expect("robigo: installed above");
                // mkString(name): a native string
                let mut v = cx.alloc_vector::<Char<'_>>(1)?;
                v.set(0, cx.mk_char(&name, Encoding::Native)?);
                let arg = cx.lcons(v.finish().into(), cx.nil())?;
                cx.lcons(ifnotfound.bind(cx), arg)
            })?;
            cx.eval(fcall, enclos, o)?;
        } else {
            o.set(cx, ifnotfound.bind(cx));
        }
    }
    if rval.bind(cx).sexptype() == PROMSXP {
        let p = s.root(cx, rval.bind(cx));
        let o = rval.reset(cx);
        cx.force(p, o)?;
    }
    Ok(rval)
}

/// `do_globalenv`, `do_baseenv`, `do_emptyenv`: the three fixed
/// environments.
// R: envir.c:do_globalenv, do_baseenv, do_emptyenv
fn fixed_env(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    out: Out<'_>,
    name: &str,
    which: fn(&Cx) -> Ref<'_>,
) -> RResult<()> {
    crate::registry::by_name(name).expect("robigo: a registry row").check_arity(cx, call, args)?;
    out.set(cx, which(cx));
    Ok(())
}

pub fn do_globalenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    fixed_env(cx, call, args, out, "globalenv", Cx::global_env)
}

pub fn do_baseenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    fixed_env(cx, call, args, out, "baseenv", Cx::base_env)
}

pub fn do_emptyenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    fixed_env(cx, call, args, out, "emptyenv", Cx::empty_env)
}

/// `.Internal(getBindingType(sym, env))`: the kind of `sym`'s binding in
/// `env`'s frame, as `R_GetBindingType` reports it ("unbound", "value",
/// "missing", "delayed", "forced", "active"). An argument not read yet is
/// "delayed" however robigo holds it (docs/g3.md §6.2).
// R: envir.c:do_bindingType
pub fn do_binding_type(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let def = crate::registry::lookup("getBindingType", true).expect("robigo: getBindingType in the registry");
    def.check_arity(cx, call, args)?;
    let t = cx.binding_type(args.root(0), args.root(1))?;
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<Char<'_>>(1)?;
        v.set(0, cx.mk_char(t.name().as_bytes(), Encoding::Native)?);
        Ok(v.finish().into())
    })
}

// ---- new.env, `...` (B2b) ---------------------------------------------------

/// `.Internal(new.env(hash, parent, size))`.
// R: envir.c:do_newenv
pub fn do_newenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("new.env", true).unwrap().check_arity(cx, call, args)?;
    let hash = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    let enclos = args.get(cx, 1);
    match enclos.sexptype() {
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        ENVSXP => {}
        OBJSXP if enclos.is_s4() => {
            return Err(not_yet(cx, Some(call), "an S4 object as an environment (R_getS4DataSlot)"));
        }
        _ => return Err(cx.error("'enclos' must be an environment")),
    }
    let size = if hash != 0 {
        let s = crate::coerce::vector::as_integer_warn(cx, args.root(2))?;
        if s == i32::MIN { 0 } else { s }
    } else {
        0
    };
    cx.build(out, |cx| cx.new_env(args.get(cx, 1), hash != 0, size))
}

/// `R_findDotsEnv(env)` then the frame's `...` value: the first
/// environment outwards whose frame binds `...` to a `...` list or the
/// missing argument, and that value; None if there is none.
// R: envir.c:R_findDotsEnv
fn dots_value<'s>(cx: &mut Cx, s: Scope<'s>, env: Root<'_>) -> RResult<Option<Root<'s>>> {
    let dots = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b"...")?.into())))?;
    let e = s.root(cx, env.bind(cx));
    loop {
        if e.bind(cx) == cx.empty_env() {
            return Ok(None);
        }
        let v = s.capture(cx, |cx, o| cx.find_var_in_frame3(e, dots, true, o))?;
        let vv = v.bind(cx);
        if vv != cx.unbound() && (vv == cx.missing_arg() || vv.sexptype() == DOTSXP) {
            return Ok(Some(v));
        }
        let next = match e.bind(cx).view() {
            View::Env(r) => s.root(cx, r.enclos()),
            _ => return Ok(None),
        };
        e.replace_from(cx, next);
    }
}

/// `length_DOTS(vl)`: a `...` list's length, 0 for the missing argument.
fn dots_len(x: Ref<'_>) -> usize {
    if x.sexptype() == DOTSXP { crate::coerce::xlength(x) } else { 0 }
}

/// `...length()`.
// R: envir.c:do_dotsLength, R_DotsLength
pub fn do_dots_length(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("...length").unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let Some(v) = dots_value(cx, s, env)? else {
            return Err(cx.error("incorrect context: the current call has no '...' to look in"));
        };
        let n = dots_len(v.bind(cx)) as i32;
        cx.build(out, |cx| Ok(cx.alloc_copy(&[n])?.finish().into()))
    })
}

/// `...names()`.
// R: envir.c:do_dotsNames, R_DotsNames
pub fn do_dots_names(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("...names").unwrap().check_arity(cx, call, args)?;
    cx.scope(|cx, s| {
        let Some(v) = dots_value(cx, s, env)? else {
            return Err(cx.error("incorrect context: the current call has no '...' to look in"));
        };
        cx.build(out, |cx| {
            let vv = v.bind(cx);
            if vv.sexptype() != DOTSXP {
                return Ok(cx.nil());
            }
            let mut tags = Vec::new();
            let mut c = vv.as_cons();
            while let Some(cell) = c {
                tags.push(cell.tag().as_sym().map(|s| s.name()));
                c = cell.cdr().as_cons();
            }
            if tags.iter().all(Option::is_none) {
                return Ok(cx.nil());
            }
            let mut r = cx.alloc_vector::<Char<'_>>(tags.len())?;
            for (i, t) in tags.iter().enumerate() {
                if let Some(t) = t {
                    r.set(i, *t);
                }
            }
            Ok(r.finish().into())
        })
    })
}

/// `...elt(n)`: the `n`th element of `...`, evaluated.
// R: envir.c:do_dotsElt, R_DotsElt, ddfindInFrame
pub fn do_dots_elt(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("...elt").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "n")?;
    let si = args.get(cx, 0);
    if !crate::util::is_numeric(si) || crate::coerce::xlength(si) != 1 {
        return Err(cx.error_call(Some(call), "indexing '...' with an invalid index"));
    }
    let i = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
    cx.scope(|cx, s| {
        let v = dots_value(cx, s, env)?;
        if i <= 0 {
            return Err(cx.error(&format!("indexing '...' with non-positive index {i}")));
        }
        let Some(v) = v else {
            return Err(cx.error(&format!("..{i} used in an incorrect context, no ... to look in")));
        };
        let n = dots_len(v.bind(cx));
        if n < i as usize {
            let msg = if i == 1 {
                format!("the ... list contains fewer than {i} element")
            } else {
                format!("the ... list contains fewer than {i} elements")
            };
            return Err(cx.error(&msg));
        }
        let elt = {
            let mut c = v.bind(cx).as_cons().unwrap();
            for _ in 1..i {
                c = c.cdr().as_cons().unwrap();
            }
            s.root(cx, c.car())
        };
        // R_DotsElt: eval(val, env) of a promise or the missing argument
        let ev = elt.bind(cx);
        if ev.sexptype() == PROMSXP || ev == cx.missing_arg() {
            cx.eval(elt, env, out)
        } else {
            out.set(cx, ev);
            Ok(())
        }
    })
}

/// `.Internal(remove(list, envir, inherits))`: each named variable removed
/// from `envir` (or, with `inherits`, the first frame outwards that has
/// it); a warning for each not found.
// R: envir.c:do_remove
pub fn do_remove(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("remove", true).unwrap().check_arity(cx, call, args)?;
    let name = args.root(0);
    match name.bind(cx).sexptype() {
        NILSXP => {
            out.set(cx, cx.nil());
            return Ok(());
        }
        STRSXP => {}
        _ => return Err(cx.error("invalid first argument")),
    }
    match args.get(cx, 1).sexptype() {
        NILSXP => return Err(cx.error("use of NULL environment is defunct")),
        ENVSXP => {}
        OBJSXP if args.get(cx, 1).is_s4() => {
            return Err(not_yet(cx, Some(call), "an S4 object as an environment (R_getS4DataSlot)"));
        }
        _ => return Err(cx.error("invalid 'envir' argument")),
    }
    let inh = as_logical(cx, args.get(cx, 2));
    if inh == i32::MIN {
        return Err(cx.error("invalid 'inherits' argument"));
    }
    let n = name.bind(cx).len().unwrap_or(0);
    for i in 0..n {
        cx.scope(|cx, s| {
            let sym = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let c = name.bind(cx).as_vector::<Char<'_>>().unwrap().get(i);
                    Ok(cx.install(c.bytes())?.into())
                })
            })?;
            let tenv = s.root(cx, args.get(cx, 1));
            let mut done = false;
            while tenv.bind(cx) != cx.empty_env() {
                done = cx.remove_variable(sym, tenv)?;
                if done || inh == 0 {
                    break;
                }
                let next = match tenv.bind(cx).view() {
                    View::Env(e) => s.root(cx, e.enclos()),
                    _ => break,
                };
                tenv.replace_from(cx, next);
            }
            if !done {
                let nm = String::from_utf8_lossy(sym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
                cx.warning(&format!("object '{nm}' not found"))?;
            }
            Ok(())
        })?;
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `as.environment(x)`: an environment itself; a search-list name; a
/// position (`pos2env`).
// R: envir.c:do_as_environment, matchEnvir, pos2env
pub fn do_as_environment(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name("as.environment").unwrap().check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "x")?;
    let arg = args.get(cx, 0);
    if arg.sexptype() == ENVSXP {
        out.set(cx, arg);
        return Ok(());
    }
    static AS_ENV: robigo_rtcore::Generic = robigo_rtcore::Generic::builtin("as.environment");
    let robigo_rtcore::Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &AS_ENV, call, args, env, out)?
    else {
        return Ok(());
    };
    let arg = args.get(cx, 0);
    match arg.sexptype() {
        STRSXP => {
            let what = match arg.as_vector::<Char<'_>>() {
                Some(v) if !v.is_empty() => v.get(0).bytes().to_vec(),
                _ => b"NA".to_vec(),
            };
            let found = match &what[..] {
                b".GlobalEnv" => Some(cx.global_env()),
                b"package:base" => Some(cx.base_env()),
                _ => {
                    // the search list's "name" attributes
                    let mut t = match cx.global_env().view() {
                        View::Env(e) => e.enclos(),
                        _ => cx.empty_env(),
                    };
                    let mut hit = None;
                    while t != cx.empty_env() {
                        let named = crate::attrib::get_value(t, b"name")
                            .and_then(|n| n.as_vector::<Char<'_>>())
                            .is_some_and(|n| !n.is_empty() && n.get(0).bytes() == &what[..]);
                        if named {
                            hit = Some(t);
                            break;
                        }
                        t = match t.view() {
                            View::Env(e) => e.enclos(),
                            _ => break,
                        };
                    }
                    hit
                }
            };
            match found {
                Some(e) => {
                    out.set(cx, e);
                    Ok(())
                }
                None => {
                    let msg = format!("no item called \"{}\" on the search list", String::from_utf8_lossy(&what));
                    Err(cx.error_call(Some(call), &msg))
                }
            }
        }
        REALSXP | INTSXP => {
            let pos = crate::coerce::vector::as_integer_warn(cx, args.root(0))?;
            if arg_len(cx, args) != 1 {
                return Err(not_yet(cx, Some(call), "as.environment() of several positions"));
            }
            if pos == i32::MIN || pos < -1 || pos == 0 {
                return Err(cx.error_call(Some(call), "invalid 'pos' argument"));
            }
            if pos == -1 {
                let mut c = cx.global_context();
                while c.callflag() & robigo_rtcore::ctxt::FUNCTION == 0 {
                    match c.next() {
                        Some(n) => c = n,
                        None => break,
                    }
                }
                if c.callflag() & robigo_rtcore::ctxt::FUNCTION == 0 {
                    return Err(cx.error_call(Some(call), "no enclosing environment"));
                }
                let e = c.sysparent();
                out.set(cx, e);
                return Ok(());
            }
            let mut e = cx.global_env();
            let mut p = pos;
            while e != cx.empty_env() && p > 1 {
                e = match e.view() {
                    View::Env(r) => r.enclos(),
                    _ => break,
                };
                p -= 1;
            }
            if p != 1 {
                return Err(cx.error_call(Some(call), "invalid 'pos' argument"));
            }
            out.set(cx, e);
            Ok(())
        }
        NILSXP => Err(cx.error_call(Some(call), "using 'as.environment(NULL)' is defunct")),
        OBJSXP => Err(not_yet(cx, Some(call), "as.environment() of an S4 object (R_getS4DataSlot)")),
        VECSXP => Err(not_yet(cx, Some(call), "as.environment() of a list (list2env)")),
        _ => Err(cx.error_call(Some(call), "invalid object for 'as.environment'")),
    }
}

fn arg_len(cx: &Cx, args: Args<'_>) -> usize {
    args.get(cx, 0).len().unwrap_or(0)
}
