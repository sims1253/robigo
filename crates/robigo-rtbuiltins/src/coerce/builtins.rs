//! coerce.c's builtins over [`super::vector`]: `as.character`,
//! `as.integer`, `as.double`, `as.complex`, `as.logical`, `as.raw`
//! (`do_asatomic`), `.Internal(as.vector(x, mode))`, `typeof`,
//! `.Internal(is.vector(x, mode))`, `is.na`, `is.nan`, `is.finite`,
//! `is.infinite`, `anyNA` and `storage.mode<-`.

use super::vector::{Warns, coerce};
use crate::attrib::{self, Attrs};
use crate::is::check1arg;
use crate::registry;
use crate::util::{is_numeric, not_yet, str2type, type_to_char, type2char};
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Built, Char, Cx, Diag, Dispatched, Encoding, FreshVec, Generic, Lgl, NeedGc, Out, RResult, Ref, Root, View,
};

/// `DispatchOrEval(call, op, generic, args, rho, &ans, dropmissing, 1)` for
/// a builtin: a method runs only when the first argument is an object.
pub fn dispatch_or_eval<'o>(
    cx: &mut Cx,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    if !args.is_empty() && args.get(cx, 0).is_object() {
        cx.try_dispatch(g, call, args, env, out)
    } else {
        Ok(Dispatched::Default(out))
    }
}

/// `x` without its attributes: itself when it has none, else a copy
/// (`duplicate` then `CLEAR_ATTRIB`, or `CLEAR_ATTRIB` in place when not
/// referenced: the copy is the same value).
fn without_attrib<'g>(cx: &'g Cx, x: Ref<'g>) -> Result<Ref<'g>, NeedGc> {
    if !attrib::has_attrib(x) && !x.is_object() && !x.is_s4() {
        return Ok(x);
    }
    let none = Attrs::new();
    copy_with(cx, x, &none)
}

/// A copy of the vector `x` with the attributes `a` (none of `x`'s own).
pub fn copy_with<'g>(cx: &'g Cx, x: Ref<'g>, a: &Attrs<'g>) -> Result<Ref<'g>, NeedGc> {
    macro_rules! plain {
        ($t:ty, $v:expr) => {{
            let mut c = cx.alloc_copy::<$t>($v.as_slice().unwrap())?;
            a.apply(cx, &mut c)?;
            c.finish().into()
        }};
    }
    Ok(match x.view() {
        View::Lgl(v) => plain!(Lgl, v),
        View::Int(v) => plain!(i32, v),
        View::Real(v) => plain!(f64, v),
        View::Cplx(v) => plain!(robigo_rtcore::Rcomplex, v),
        View::Raw(v) => plain!(u8, v),
        View::Str(v) => {
            let mut c = cx.alloc_vector::<Char<'g>>(v.len())?;
            for i in 0..v.len() {
                c.set(i, v.get(i));
            }
            a.apply(cx, &mut c)?;
            c.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut c = if x.sexptype() == EXPRSXP { cx.alloc_expression(v.len())? } else { cx.alloc_vector(v.len())? };
            for i in 0..v.len() {
                c.set(i, v.get(i));
            }
            a.apply(cx, &mut c)?;
            c.finish().into()
        }
        _ => x,
    })
}

/// `ascommon(call, u, type)` then (`clear`) `CLEAR_ATTRIB`, under `&Cx`.
/// `CLOSXP` (`asFunction`) is not ported.
// R: coerce.c:ascommon
pub(crate) fn ascommon<'g, 'r>(
    cx: &'g Cx,
    call: Root<'r>,
    u: Ref<'g>,
    ty: u8,
    clear: bool,
) -> Result<Built<'g, 'r, Warns>, NeedGc> {
    let t = u.sexptype();
    let is_vector = attrib::is_vector(u);
    let is_list = matches!(t, LISTSXP | NILSXP);
    let is_language = matches!(t, LANGSXP | NILSXP);
    if is_vector || is_list || is_language || (t == SYMSXP && ty == EXPRSXP) {
        let (v, w) = if ty != ANYSXP && t != ty {
            match coerce(cx, u, ty, !clear)? {
                Built::Done(v, w) => (v, w),
                Built::Fail(d) => return Ok(Built::fail(Diag::error(d.message().to_owned()))),
            }
        } else {
            (u, Warns::new())
        };
        // as.pairlist of an atomic vector drops its attributes (the tags
        // stay: they are the pairlist's names)
        let v = if clear && !matches!(v.sexptype(), NILSXP | LISTSXP | LANGSXP) { without_attrib(cx, v)? } else { v };
        return Ok(Built::done(v).with(w));
    }
    if t == SYMSXP && ty == STRSXP {
        let mut s = cx.alloc_vector::<Char<'g>>(1)?;
        s.set(0, u.as_sym().unwrap().name());
        return Ok(Built::done(s.finish()).with(Warns::new()));
    }
    if t == SYMSXP && ty == SYMSXP {
        return Ok(Built::done(u).with(Warns::new()));
    }
    if t == SYMSXP && ty == VECSXP {
        let mut l = cx.alloc_vector::<Ref<'g>>(1)?;
        l.set(0, u);
        return Ok(Built::done(l.finish()).with(Warns::new()));
    }
    let msg = format!("cannot coerce type '{}' to vector of type '{}'", type_to_char(u), type2char(ty));
    Ok(Built::fail(Diag::error_call(Some(call), msg)))
}

const AS_CHARACTER: Generic = Generic::builtin("as.character");
const AS_INTEGER: Generic = Generic::builtin("as.integer");
const AS_DOUBLE: Generic = Generic::builtin("as.double");
const AS_COMPLEX: Generic = Generic::builtin("as.complex");
const AS_LOGICAL: Generic = Generic::builtin("as.logical");
const AS_RAW: Generic = Generic::builtin("as.raw");

/// `do_asatomic` for names.c's code `CODE`: 0 `as.character`, 1
/// `as.integer`, 2 `as.double`, 3 `as.complex`, 4 `as.logical`, 5
/// `as.raw`.
// R: coerce.c:do_asatomic
pub fn do_asatomic<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    check1arg(cx, args, call, "x")?;
    let (g, ty): (&'static Generic, u8) = match CODE {
        0 => (&AS_CHARACTER, STRSXP),
        1 => (&AS_INTEGER, INTSXP),
        2 => (&AS_DOUBLE, REALSXP),
        3 => (&AS_COMPLEX, CPLXSXP),
        4 => (&AS_LOGICAL, LGLSXP),
        _ => (&AS_RAW, RAWSXP),
    };
    let Dispatched::Default(out) = dispatch_or_eval(cx, g, call, args, env, out)? else { return Ok(()) };
    // checkArity: as.raw takes one argument, the others any number
    if CODE == 5 {
        registry::by_name("as.raw").unwrap().check_arity(cx, call, args)?;
    }
    if args.is_empty() {
        // CAR(R_NilValue) is NULL
        return cx.build_with(out, |cx| ascommon(cx, call, cx.nil(), ty, true))?.signal(cx);
    }
    let x = args.root(0);
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "as.vector() of a package ALTREP vector"));
    }
    let w = cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        if xv.sexptype() == ty {
            return Ok(Built::done(without_attrib(cx, xv)?).with(Warns::new()));
        }
        ascommon(cx, call, xv, ty, true)
    })?;
    w.signal(cx)
}

const AS_VECTOR: Generic = Generic::builtin("as.vector");

/// `.Internal(as.vector(x, mode))`.
// R: coerce.c:do_asvector
pub fn do_asvector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Dispatched::Default(out) = dispatch_or_eval(cx, &AS_VECTOR, call, args, env, out)? else { return Ok(()) };
    registry::lookup("as.vector", true).unwrap().check_arity(cx, call, args)?;
    let mode = {
        let m = args.get(cx, 1);
        match m.as_vector::<Char<'_>>() {
            Some(v) if v.len() == 1 => Some(v.get(0).bytes().to_vec()),
            _ => None,
        }
    };
    let Some(mode) = mode else { return Err(cx.error("invalid 'mode' argument")) };
    let ty = if mode == b"function" { Some(CLOSXP) } else { str2type(&mode) };
    let x = args.root(0);
    {
        let xv = x.bind(cx);
        if xv.is_package_altrep() {
            return Err(not_yet(cx, Some(call), "as.vector() of a package ALTREP vector"));
        }
        if xv.is_s4() && xv.sexptype() == OBJSXP {
            return Err(not_yet(cx, Some(call), "as.vector() of an S4 object (R_getS4DataSlot)"));
        }
    }
    let Some(ty) = ty.filter(|&t| {
        matches!(
            t,
            SYMSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | EXPRSXP | VECSXP | LISTSXP | CLOSXP | RAWSXP
        ) || t == ANYSXP
    }) else {
        // "any" and the type itself are taken before this check in GNU R;
        // an unknown mode reaches it
        let same = ty.is_some_and(|t| t == x.bind(cx).sexptype());
        if same {
            out.set(cx, x.bind(cx));
            return Ok(());
        }
        return Err(cx.error("invalid 'mode' argument"));
    };
    if ty == CLOSXP && x.bind(cx).sexptype() != CLOSXP {
        let f = x.bind(cx).sexptype();
        if !matches!(f, BUILTINSXP | SPECIALSXP) {
            return Err(not_yet(cx, Some(call), "as.vector(x, \"function\") (asFunction)"));
        }
    }
    let w = cx.build_with(out, |cx| {
        let xv = x.bind(cx);
        let xt = xv.sexptype();
        if ty == ANYSXP || xt == ty {
            match xt {
                LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
                    return Ok(Built::done(without_attrib(cx, xv)?).with(Warns::new()));
                }
                // _R_IS_AS_VECTOR_EXPERIMENTS_ is unset: a list stays as
                // it is
                EXPRSXP | VECSXP => return Ok(Built::done(xv).with(Warns::new())),
                _ => {}
            }
        }
        if ty == CLOSXP {
            // isFunction(x): a builtin or special is returned as it is
            return Ok(Built::done(xv).with(Warns::new()));
        }
        // CLEAR_ATTRIB unless the result is NULL, a pairlist, a call or a
        // list
        match ascommon(cx, call, xv, ty, false)? {
            Built::Done(v, w) if !matches!(v.sexptype(), NILSXP | LISTSXP | LANGSXP | VECSXP | EXPRSXP) => {
                Ok(Built::done(without_attrib(cx, v)?).with(w))
            }
            b => Ok(b),
        }
    })?;
    w.signal(cx)
}

/// `.Internal(typeof(x))`.
// R: coerce.c:do_typeof
pub fn do_typeof(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("typeof", true).unwrap().check_arity(cx, call, args)?;
    let x = args.root(0);
    cx.build(out, |cx| {
        let name = type_to_char(x.bind(cx));
        mk_string(cx, name.as_bytes())
    })
}

/// `mkString(s)`: a character vector of one native string.
pub fn mk_string<'g>(cx: &'g Cx, s: &[u8]) -> Result<Ref<'g>, NeedGc> {
    let mut v = cx.alloc_vector::<Char<'g>>(1)?;
    v.set(0, cx.mk_char(s, Encoding::Native)?);
    Ok(v.finish().into())
}

/// `.Internal(is.vector(x, mode))`.
// R: coerce.c:do_isvector
pub fn do_isvector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("is.vector", true).unwrap().check_arity(cx, call, args)?;
    let mode = match args.get(cx, 1).as_vector::<Char<'_>>() {
        Some(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
        _ => return Err(cx.error("invalid 'mode' argument")),
    };
    let x = args.get(cx, 0);
    let stype: &[u8] = if mode == b"name" { b"symbol" } else { &mode };
    let mut ans = if stype == b"any" {
        attrib::is_vector(x)
    } else if stype == b"numeric" {
        is_numeric(x) && x.sexptype() != LGLSXP
    } else {
        stype == type_to_char(x).as_bytes()
    };
    // only a names attribute is allowed
    if ans && attrib::attrs(x).any(|a| a.tag.name().bytes() != b"names") {
        ans = false;
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}

/// `copyDimAndNames(x, ans)`'s attributes for a fresh result: `dim`, then
/// `dimnames` for an array or `names` otherwise.
// R: coerce.c:copyDimAndNames
fn dim_and_names<'g>(x: Ref<'g>) -> Attrs<'g> {
    let mut a = Attrs::new();
    if !attrib::is_vector(x) {
        return a;
    }
    if let Some(d) = attrib::get(x, b"dim") {
        a.install(d);
    }
    let nm = if attrib::is_array(x) { attrib::get(x, b"dimnames") } else { attrib::get(x, b"names") };
    if let Some(n) = nm {
        a.install(n);
    }
    a
}

/// A logical vector of `n` from `f`, with `x`'s `copyDimAndNames`.
fn lgl_like<'g>(cx: &'g Cx, x: Ref<'g>, n: usize, f: impl FnMut(usize) -> bool) -> Result<Ref<'g>, NeedGc> {
    let mut f = f;
    let mut r: FreshVec<'g, Lgl> = cx.alloc_with(n, |i| Lgl(f(i) as i32))?;
    dim_and_names(x).apply(cx, &mut r)?;
    Ok(r.finish().into())
}

/// The element-wise NA test of `is.na` for a list element (`LIST_VEC_NA`).
fn list_elt_na(cx: &Cx, s: Ref<'_>) -> bool {
    if !attrib::is_vector(s) || s.len() != Some(1) {
        return false;
    }
    match s.view() {
        View::Lgl(v) => v.get(0).0 == i32::MIN,
        View::Int(v) => v.get(0) == i32::MIN,
        View::Real(v) => v.get(0).is_nan(),
        View::Str(v) => v.get(0) == cx.na_string(),
        View::Cplx(v) => v.get(0).r.is_nan() || v.get(0).i.is_nan(),
        _ => false,
    }
}

const IS_NA: Generic = Generic { name: "is.na", group: None, drop_missing: true, args_evaluated: true };
const IS_NAN: Generic = Generic { name: "is.nan", group: None, drop_missing: true, args_evaluated: true };
const IS_FINITE: Generic = Generic::builtin("is.finite");
const IS_INFINITE: Generic = Generic::builtin("is.infinite");

/// The arity and `check1arg` of the `is.na` family, then their dispatch.
fn is_family<'o>(
    cx: &mut Cx,
    name: &str,
    g: &'static Generic,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'o>,
) -> RResult<Option<Out<'o>>> {
    registry::by_name(name).unwrap().check_arity(cx, call, args)?;
    check1arg(cx, args, call, "x")?;
    match dispatch_or_eval(cx, g, call, args, env, out)? {
        Dispatched::Done => Ok(None),
        Dispatched::Default(o) => {
            if args.get(cx, 0).is_package_altrep() {
                return Err(not_yet(cx, Some(call), &format!("{name}() of a package ALTREP vector")));
            }
            Ok(Some(o))
        }
    }
}

/// `is.na(x)`.
// R: coerce.c:do_isna
pub fn do_isna(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(out) = is_family(cx, "is.na", &IS_NA, call, args, env, out)? else { return Ok(()) };
    let x = args.root(0);
    let t = x.bind(cx).sexptype();
    let other = !matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | LISTSXP | VECSXP | RAWSXP | NILSXP);
    if other {
        let msg = format!("is.na() applied to non-(list or vector) of type '{}'", type_to_char(x.bind(cx)));
        cx.warning_call(Some(call), &msg)?;
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let na = cx.na_string();
        match xv.view() {
            View::Lgl(v) => lgl_like(cx, xv, v.len(), |i| v.get(i).0 == i32::MIN),
            View::Int(v) => lgl_like(cx, xv, v.len(), |i| v.get(i) == i32::MIN),
            View::Real(v) => lgl_like(cx, xv, v.len(), |i| v.get(i).is_nan()),
            View::Cplx(v) => lgl_like(cx, xv, v.len(), |i| v.get(i).r.is_nan() || v.get(i).i.is_nan()),
            View::Str(v) => lgl_like(cx, xv, v.len(), |i| v.get(i) == na),
            View::List(v) => lgl_like(cx, xv, v.len(), |i| list_elt_na(cx, v.get(i))),
            View::Pairlist(_) => {
                let items = cars(xv);
                lgl_like(cx, xv, items.len(), |i| list_elt_na(cx, items[i]))
            }
            View::Raw(v) => lgl_like(cx, xv, v.len(), |_| false),
            // the other types: xlength(x) FALSEs
            _ => lgl_like(cx, xv, xlength(xv), |_| false),
        }
    })
}

/// A pairlist's or call's elements.
fn cars(x: Ref<'_>) -> Vec<Ref<'_>> {
    let mut v = Vec::new();
    let mut c = x.as_cons();
    while let Some(cell) = c {
        v.push(cell.car());
        c = cell.cdr().as_cons();
    }
    v
}

/// GNU R's `xlength(x)`: a vector's length, a pairlist's or call's number
/// of cells, an environment's number of bindings is not needed here (1 for
/// any other object).
pub fn xlength(x: Ref<'_>) -> usize {
    match x.sexptype() {
        NILSXP => 0,
        LISTSXP | LANGSXP | DOTSXP => cars(x).len(),
        _ => x.len().unwrap_or(1),
    }
}

/// `is.nan(x)`.
// R: coerce.c:do_isnan
pub fn do_isnan(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(out) = is_family(cx, "is.nan", &IS_NAN, call, args, env, out)? else { return Ok(()) };
    finite_family(cx, call, args.root(0), out, |v| v.is_nan() && !robigo_text::is_na(v), false)
}

/// `is.finite(x)`.
// R: coerce.c:do_isfinite
pub fn do_isfinite(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(out) = is_family(cx, "is.finite", &IS_FINITE, call, args, env, out)? else { return Ok(()) };
    finite_family(cx, call, args.root(0), out, f64::is_finite, true)
}

/// `is.infinite(x)`.
// R: coerce.c:do_isinfinite
pub fn do_isinfinite(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let Some(out) = is_family(cx, "is.infinite", &IS_INFINITE, call, args, env, out)? else { return Ok(()) };
    finite_family(cx, call, args.root(0), out, f64::is_infinite, false)
}

/// The shared loop of `is.nan`, `is.finite`, `is.infinite`: `real` tests a
/// double (both parts of a complex: all for finite, any otherwise); an
/// integer or logical is `int_finite` when not NA (is.finite) and FALSE
/// otherwise.
fn finite_family(
    cx: &mut Cx,
    call: Root<'_>,
    x: Root<'_>,
    out: Out<'_>,
    real: fn(f64) -> bool,
    finite: bool,
) -> RResult<()> {
    let t = x.bind(cx).sexptype();
    if !matches!(t, STRSXP | RAWSXP | NILSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP) {
        let msg = format!("default method not implemented for type '{}'", type_to_char(x.bind(cx)));
        return Err(cx.error_call(Some(call), &msg));
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        match xv.view() {
            View::Lgl(v) => lgl_like(cx, xv, v.len(), |i| finite && v.get(i).0 != i32::MIN),
            View::Int(v) => lgl_like(cx, xv, v.len(), |i| finite && v.get(i) != i32::MIN),
            View::Real(v) => lgl_like(cx, xv, v.len(), |i| real(v.get(i))),
            View::Cplx(v) => lgl_like(cx, xv, v.len(), |i| {
                let z = v.get(i);
                if finite { real(z.r) && real(z.i) } else { real(z.r) || real(z.i) }
            }),
            _ => lgl_like(cx, xv, xlength(xv), |_| false),
        }
    })
}

const ANY_NA: Generic = Generic::builtin("anyNA");

/// `anyNA(x, recursive = FALSE)`.
// R: coerce.c:do_anyNA
pub fn do_any_na(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    if args.is_empty() || args.len() > 2 {
        return Err(cx.error_call(Some(call), "anyNA takes 1 or 2 arguments"));
    }
    let Dispatched::Default(out) = dispatch_or_eval(cx, &ANY_NA, call, args, env, out)? else { return Ok(()) };
    let recursive = if args.len() == 1 {
        check1arg(cx, args, call, "x")?;
        false
    } else {
        // matchArgs_NR over the formals (x, recursive): exact names, then
        // partial, then by position
        let names: Vec<Option<Vec<u8>>> = (0..2).map(|i| args.name(cx, i).map(|s| s.name().bytes().to_vec())).collect();
        let is_rec = |n: &Option<Vec<u8>>| n.as_ref().is_some_and(|n| !n.is_empty() && b"recursive".starts_with(n));
        let is_x = |n: &Option<Vec<u8>>| n.as_deref() == Some(b"x");
        let ri = if is_rec(&names[1]) || is_x(&names[0]) || names[1].is_none() && !is_rec(&names[0]) { 1 } else { 0 };
        let xi = 1 - ri;
        let other = |n: &Option<Vec<u8>>| n.as_ref().is_some_and(|n| !n.is_empty()) && !is_x(n) && !is_rec(n);
        if other(&names[0]) || other(&names[1]) || (is_x(&names[ri])) || is_rec(&names[xi]) {
            return Err(not_yet(cx, Some(call), "anyNA() with these argument names"));
        }
        if xi != 0 {
            return Err(not_yet(cx, Some(call), "anyNA() with 'x' not first"));
        }
        let r = args.root(ri);
        crate::util::as_bool2(cx, call, r)?
    };
    let ans = any_na(cx, call, args.root(0), recursive, env)?;
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}

/// `anyNA(call, op, args, env)`: an object, or a list not searched
/// recursively, through R's `any(is.na(x))`.
// R: coerce.c:anyNA
fn any_na(cx: &mut Cx, call: Root<'_>, x: Root<'_>, recursive: bool, env: Root<'_>) -> RResult<bool> {
    let (t, obj) = {
        let xv = x.bind(cx);
        (xv.sexptype(), xv.is_object())
    };
    let is_list = matches!(t, VECSXP | LISTSXP);
    if obj || (is_list && !recursive) {
        return cx.scope(|cx, s| {
            let e = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let isna = cx.install(b"is.na")?;
                    let any = cx.install(b"any")?;
                    let inner = cx.lcons(isna.into(), cx.cons(x.bind(cx), cx.nil())?)?;
                    cx.lcons(any.into(), cx.cons(inner, cx.nil())?)
                })
            })?;
            let r = s.capture(cx, |cx, o| cx.eval(e, env, o))?;
            Ok(crate::util::as_logical(cx, r.bind(cx)) == 1)
        });
    }
    if x.bind(cx).is_package_altrep() {
        return Err(not_yet(cx, Some(call), "anyNA() of a package ALTREP vector"));
    }
    let xv = x.bind(cx);
    let na = cx.na_string();
    Ok(match xv.view() {
        View::Real(v) => v.as_slice().unwrap().iter().any(|x| x.is_nan()),
        View::Int(v) => v.as_slice().unwrap().contains(&i32::MIN),
        View::Lgl(v) => v.as_slice().unwrap().iter().any(|x| x.0 == i32::MIN),
        View::Cplx(v) => v.as_slice().unwrap().iter().any(|z| z.r.is_nan() || z.i.is_nan()),
        View::Str(v) => v.as_slice().unwrap().contains(&na),
        View::Raw(_) | View::Null => false,
        View::List(_) | View::Pairlist(_) => {
            let n = xlength(xv);
            for i in 0..n {
                let found = cx.scope(|cx, s| {
                    let e = s.root(cx, nth(x.bind(cx), i));
                    if e.bind(cx).is_object() {
                        return Err(not_yet(cx, Some(call), "anyNA(recursive = TRUE) of a list holding objects"));
                    }
                    any_na(cx, call, e, true, env)
                })?;
                if found {
                    return Ok(true);
                }
            }
            false
        }
        _ => {
            let msg = format!("anyNA() applied to non-(list or vector) of type '{}'", type_to_char(xv));
            return Err(cx.error(&msg));
        }
    })
}

/// Element `i` of a list or pairlist.
fn nth(x: Ref<'_>, i: usize) -> Ref<'_> {
    match x.view() {
        View::List(v) | View::Expr(v) => v.get(i),
        _ => cars(x)[i],
    }
}

/// `storage.mode(x) <- value`.
// R: coerce.c:do_storage_mode
pub fn do_storage_mode(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::by_name("storage.mode<-").unwrap().check_arity(cx, call, args)?;
    check1arg(cx, args, call, "x")?;
    let value = {
        let v = args.get(cx, 1);
        match v.as_vector::<Char<'_>>() {
            Some(s) if !s.is_empty() && s.get(0) != cx.na_string() => Some(s.get(0).bytes().to_vec()),
            _ => None,
        }
    };
    let Some(value) = value else { return Err(cx.error("'value' must be non-null character string")) };
    let Some(ty) = str2type(&value) else {
        return Err(cx.error(match &value[..] {
            b"real" => "use of 'real' is defunct: use 'double' instead",
            b"single" => "use of 'single' is defunct: use mode<- instead",
            _ => "invalid value",
        }));
    };
    let obj = args.root(0);
    let (t, factor, atomic) = {
        let o = obj.bind(cx);
        (o.sexptype(), attrib::inherits(o, b"factor"), crate::util::is_vector_atomic(o))
    };
    if t == ty {
        out.set(cx, obj.bind(cx));
        return Ok(());
    }
    if factor {
        return Err(cx.error("invalid to change the storage mode of a factor"));
    }
    if !atomic && attrib::has_attrib(obj.bind(cx)) {
        return Err(not_yet(cx, Some(call), "'storage.mode<-' of a non-atomic object with attributes"));
    }
    // coerceVector then SHALLOW_DUPLICATE_ATTRIB(ans, obj): an atomic
    // coercion keeps every attribute already
    let w = cx.build_with(out, |cx| coerce(cx, obj.bind(cx), ty, true))?;
    w.signal(cx)
}

/// `.Internal(do.call(what, args, envir))`: the call of `what` (a function,
/// or a name) with the list's elements as its arguments, their non-empty
/// names as tags, evaluated in `envir`.
// R: coerce.c:do_docall
pub fn do_docall(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    registry::lookup("do.call", true).unwrap().check_arity(cx, call, args)?;
    let (fun, a, envir) = (args.root(0), args.root(1), args.root(2));
    let is_fn = matches!(fun.bind(cx).sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP);
    let is_str = fun.bind(cx).as_vector::<Char<'_>>().is_some_and(|v| v.len() == 1);
    if !(is_fn || is_str) {
        return Err(cx.error("'what' must be a function or character string"));
    }
    if !matches!(a.bind(cx).sexptype(), NILSXP | VECSXP) {
        return Err(cx.error("'args' must be a list"));
    }
    if envir.bind(cx).sexptype() != ENVSXP {
        return Err(cx.error("'envir' must be an environment"));
    }
    if is_str && fun.bind(cx).as_vector::<Char<'_>>().unwrap().get(0).bytes() == b".Internal" {
        return Err(cx.error("illegal usage"));
    }
    if fun.bind(cx).sexptype() == SPECIALSXP {
        // PRIMNAME(fun) == ".Internal": the primitive object base binds
        let is_internal = cx.scope(|cx, s| {
            let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(b".Internal")?.into())))?;
            let base = s.root(cx, cx.base_env());
            let v = s.capture(cx, |cx, o| cx.find_var_in_frame3(base, sym, true, o))?;
            Ok::<bool, robigo_rtcore::RError>(v.bind(cx) == fun.bind(cx))
        })?;
        if is_internal {
            return Err(cx.error("illegal usage"));
        }
    }
    cx.scope(|cx, s| {
        let c = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let f: Ref<'_> = if is_str {
                    let c = fun.bind(cx).as_vector::<Char<'_>>().unwrap().get(0);
                    cx.install(c.bytes())?.into()
                } else {
                    fun.bind(cx)
                };
                let av = a.bind(cx);
                let items: Vec<Ref<'_>> = match av.view() {
                    View::List(v) => (0..v.len()).map(|i| v.get(i)).collect(),
                    _ => Vec::new(),
                };
                let names = attrib::names(av).and_then(|n| n.as_vector::<Char<'_>>());
                let mut l = cx.nil();
                for (i, &e) in items.iter().enumerate().rev() {
                    let tag = names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty());
                    l = match tag {
                        Some(t) => cx.cons_tagged(e, l, cx.install(t.bytes())?)?,
                        None => cx.cons(e, l)?,
                    };
                }
                cx.lcons(f, l)
            })
        })?;
        cx.eval(c, envir, out)
    })
}
