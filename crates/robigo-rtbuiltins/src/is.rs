//! coerce.c's `do_is`: the `is.*` type predicates (`is.null`,
//! `is.character`, `is.numeric`, `is.function`, ...), one function per
//! names.c `code`. `is.numeric`, `is.matrix` and `is.array` are internal
//! generics for objects.

use crate::attrib;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Dispatched, Generic, Lgl, Out, RResult, Root};

/// `check1arg(args, call, formal)`: a named argument must partially match
/// `formal`.
// R: errors.c:check1arg
pub fn check1arg(cx: &mut Cx, args: Args<'_>, call: Root<'_>, formal: &str) -> RResult<()> {
    let Some(tag) = (!args.is_empty()).then(|| args.name(cx, 0)).flatten() else { return Ok(()) };
    let supplied = String::from_utf8_lossy(tag.name().bytes()).into_owned();
    if !formal.as_bytes().starts_with(supplied.as_bytes()) {
        let msg = format!("supplied argument name '{supplied}' does not match '{formal}'");
        return Err(cx.error_call(Some(call), &msg));
    }
    Ok(())
}

/// `checkArity(op, args)` for a primitive of arity 1.
fn check_arity1(cx: &mut Cx, call: Root<'_>, args: Args<'_>, name: &str) -> RResult<()> {
    let n = args.len();
    if n != 1 {
        let s = if n == 1 { "" } else { "s" };
        return Err(cx.error_call(Some(call), &format!("{n} argument{s} passed to '{name}' which requires 1")));
    }
    Ok(())
}

/// The names.c name of each code `do_is` serves.
const fn is_name(code: i32) -> &'static str {
    match code {
        c if c == NILSXP as i32 => "is.null",
        c if c == LGLSXP as i32 => "is.logical",
        c if c == INTSXP as i32 => "is.integer",
        c if c == REALSXP as i32 => "is.double",
        c if c == CPLXSXP as i32 => "is.complex",
        c if c == STRSXP as i32 => "is.character",
        c if c == SYMSXP as i32 => "is.symbol",
        c if c == ENVSXP as i32 => "is.environment",
        c if c == VECSXP as i32 => "is.list",
        c if c == LISTSXP as i32 => "is.pairlist",
        c if c == EXPRSXP as i32 => "is.expression",
        c if c == RAWSXP as i32 => "is.raw",
        50 => "is.object",
        51 => "isS4",
        100 => "is.numeric",
        101 => "is.matrix",
        102 => "is.array",
        200 => "is.atomic",
        201 => "is.recursive",
        300 => "is.call",
        301 => "is.language",
        302 => "is.function",
        _ => "is.single",
    }
}

/// `do_is` for the names.c code `CODE` (`STRSXP` for `is.character`,
/// 100 for `is.numeric`, ...). `is.name` is `is.symbol`'s code under its
/// own row.
// R: coerce.c:do_is
pub fn do_is<const CODE: i32>(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    is(cx, call, args, env, out, CODE, is_name(CODE))
}

/// `is.name`: `is.symbol` under its own name (for `checkArity`'s message).
pub fn do_is_name(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    is(cx, call, args, env, out, SYMSXP as i32, "is.name")
}

#[allow(non_snake_case)]
fn is(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>, CODE: i32, name: &str) -> RResult<()> {
    check_arity1(cx, call, args, name)?;
    check1arg(cx, args, call, "x")?;
    let mut out = out;
    if (100..200).contains(&CODE) && args.get(cx, 0).is_object() {
        static GENERICS: [Generic; 3] =
            [Generic::builtin("is.numeric"), Generic::builtin("is.matrix"), Generic::builtin("is.array")];
        out = match cx.try_dispatch(&GENERICS[(CODE - 100) as usize], call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(out) => out,
        };
    }
    let x = args.get(cx, 0);
    let t = x.sexptype();
    let code = |t: u8| CODE == t as i32;
    let ans = if CODE == 0 {
        t == NILSXP
    } else if code(LGLSXP) {
        t == LGLSXP
    } else if code(INTSXP) {
        t == INTSXP && !attrib::inherits(x, b"factor")
    } else if code(REALSXP) {
        t == REALSXP
    } else if code(CPLXSXP) {
        t == CPLXSXP
    } else if code(STRSXP) {
        t == STRSXP
    } else if code(SYMSXP) || code(ENVSXP) {
        if x.is_s4() && t == OBJSXP {
            return Err(cx.error_call(Some(call), "robigo: the .xData slot of an S4 object is not implemented yet"));
        }
        t as i32 == CODE
    } else if code(VECSXP) {
        matches!(t, VECSXP | LISTSXP)
    } else if code(LISTSXP) {
        // pairlist() -> NULL
        matches!(t, LISTSXP | NILSXP)
    } else if code(EXPRSXP) {
        t == EXPRSXP
    } else if code(RAWSXP) {
        t == RAWSXP
    } else {
        match CODE {
            50 => x.is_object(),
            51 => x.is_s4(),
            // isNumeric excludes factors
            100 => crate::util::is_numeric(x) && t != LGLSXP,
            101 => attrib::is_vector(x) && attrib::dims(x).is_some_and(|d| d.len() == 2),
            102 => attrib::is_vector(x) && attrib::dims(x).is_some_and(|d| !d.is_empty()),
            200 => matches!(t, CHARSXP | LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP),
            201 => matches!(
                t,
                VECSXP
                    | LISTSXP
                    | CLOSXP
                    | ENVSXP
                    | PROMSXP
                    | LANGSXP
                    | SPECIALSXP
                    | BUILTINSXP
                    | DOTSXP
                    | ANYSXP
                    | EXPRSXP
            ),
            300 => t == LANGSXP,
            301 => matches!(t, SYMSXP | LANGSXP | EXPRSXP),
            302 => matches!(t, CLOSXP | BUILTINSXP | SPECIALSXP),
            // is.single
            _ => return Err(cx.error_call(Some(call), "type \"single\" unimplemented in R")),
        }
    };
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}

const IS_NA: Generic = Generic::builtin("is.na");

/// `is.na(x)`: an internal generic; element-wise NA tests of atomic
/// vectors, a list's length-one atomic elements, FALSE for raw; the result
/// takes `x`'s dim and names (or dimnames).
// R: coerce.c:do_isna, copyDimAndNames
pub fn do_isna(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity1(cx, call, args, "is.na")?;
    check1arg(cx, args, call, "x")?;
    let out = match cx.try_dispatch(&IS_NA, call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(o) => o,
    };
    if args.any_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "is.na() of a package ALTREP vector"));
    }
    let x = args.get(cx, 0);
    let t = x.sexptype();
    let known = matches!(t, LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | LISTSXP | VECSXP | RAWSXP | NILSXP);
    if !known {
        let msg = format!("is.na() applied to non-(list or vector) of type '{}'", crate::util::type_to_char(x));
        cx.warning_call(Some(call), &msg)?;
    }
    cx.build(out, |cx| {
        let x = args.get(cx, 0);
        let na = cx.na_string();
        let elt_na = |s: robigo_rtcore::Ref<'_>| -> bool {
            if !attrib::is_vector(s) || s.len() != Some(1) {
                return false;
            }
            match s.view() {
                robigo_rtcore::View::Lgl(v) => v.get(0).0 == i32::MIN,
                robigo_rtcore::View::Int(v) => v.get(0) == i32::MIN,
                robigo_rtcore::View::Real(v) => v.get(0).is_nan(),
                robigo_rtcore::View::Str(v) => v.get(0) == na,
                robigo_rtcore::View::Cplx(v) => v.get(0).r.is_nan() || v.get(0).i.is_nan(),
                _ => false,
            }
        };
        let flags: Vec<bool> = match x.view() {
            robigo_rtcore::View::Lgl(v) => v.as_slice().unwrap().iter().map(|l| l.0 == i32::MIN).collect(),
            robigo_rtcore::View::Int(v) => v.as_slice().unwrap().iter().map(|&i| i == i32::MIN).collect(),
            robigo_rtcore::View::Real(v) => v.as_slice().unwrap().iter().map(|d| d.is_nan()).collect(),
            robigo_rtcore::View::Cplx(v) => {
                v.as_slice().unwrap().iter().map(|z| z.r.is_nan() || z.i.is_nan()).collect()
            }
            robigo_rtcore::View::Str(v) => v.as_slice().unwrap().iter().map(|&c| c == na).collect(),
            robigo_rtcore::View::List(v) => (0..v.len()).map(|i| elt_na(v.get(i))).collect(),
            robigo_rtcore::View::Pairlist(_) => {
                let mut f = Vec::new();
                let mut c = x;
                while let Some(cell) = c.as_cons() {
                    f.push(elt_na(cell.car()));
                    c = cell.cdr();
                }
                f
            }
            robigo_rtcore::View::Raw(v) => vec![false; v.len()],
            robigo_rtcore::View::Null => Vec::new(),
            _ => vec![false; crate::array::r_length(cx, x)],
        };
        let mut v = cx.alloc_vector::<Lgl>(flags.len())?;
        for (i, &f) in flags.iter().enumerate() {
            v.as_mut_slice()[i] = Lgl(f as i32);
        }
        // copyDimAndNames
        if attrib::is_vector(x) {
            let mut a = attrib::Attrs::new();
            if let Some(d) = attrib::get(x, b"dim") {
                a.install(d);
            }
            let names = if attrib::is_array(x) { attrib::get(x, b"dimnames") } else { attrib::get(x, b"names") };
            if let Some(n) = names {
                a.install(n);
            }
            a.apply(cx, &mut v)?;
        }
        Ok(v.finish().into())
    })
}

/// `.Internal(typeof(x))`.
// R: coerce.c:do_typeof
pub fn do_typeof(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("typeof", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let x = args.get(cx, 0);
    let name: &str = if x.sexptype() == OBJSXP && !x.is_s4() { "object" } else { crate::util::type2char(x.sexptype()) };
    let name = name.as_bytes().to_vec();
    cx.build(out, |cx| {
        let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
        v.set(0, cx.mk_char(&name, robigo_rtcore::Encoding::Native)?);
        Ok(v.finish().into())
    })
}

/// `.Internal(is.vector(x, mode))`: of the mode ("any": atomic or a list,
/// "numeric": integer or double, "name": a symbol, else the type's name),
/// with no attribute but names.
// R: coerce.c:do_isvector
pub fn do_isvector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("is.vector", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let (x, mode) = (args.get(cx, 0), args.get(cx, 1));
    let Some(m) = mode.as_vector::<robigo_rtcore::Char<'_>>().filter(|v| mode.sexptype() == STRSXP && v.len() == 1)
    else {
        return Err(cx.error("invalid 'mode' argument"));
    };
    let mut stype = m.get(0).bytes();
    if stype == b"name" {
        stype = b"symbol";
    }
    let mut ans = if stype == b"any" {
        attrib::is_vector(x)
    } else if stype == b"numeric" {
        matches!(x.sexptype(), INTSXP | REALSXP) && !attrib::inherits(x, b"factor")
    } else {
        stype == crate::util::type_to_char(x).as_bytes()
    };
    // (_R_IS_AS_VECTOR_EXPERIMENTS_ is unset)
    if ans && attrib::attrs(x).any(|a| a.tag.name().bytes() != b"names") {
        ans = false;
    }
    cx.build(out, |cx| Ok(cx.alloc_copy(&[Lgl(ans as i32)])?.finish().into()))
}

// ---- as.character and as.vector (S1) ----------------------------------------------

/// A copy of `x` without attributes, or `x` itself if it has none or is
/// not referenced (`CLEAR_ATTRIB` on a duplicate).
fn without_attrib<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>) -> RResult<Root<'s>> {
    let v = x.bind(cx);
    if !attrib::has_attrib(v) && !v.is_object() {
        return Ok(s.root(cx, v));
    }
    let r = if v.maybe_referenced() {
        let o = s.out(cx);
        let r = o.root();
        cx.duplicate(x, o)?;
        r
    } else {
        s.root(cx, v)
    };
    let nil = s.root(cx, cx.nil());
    cx.set_attrib_list(r, nil);
    cx.set_object_bit(r, false);
    cx.set_s4_bit(r, false);
    Ok(r)
}

/// `VectorToPairList(x)`: a pairlist of the list's elements tagged by its
/// non-empty names, with the list's other attributes.
// R: coerce.c:VectorToPairList
fn vector_to_pairlist<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>) -> RResult<Root<'s>> {
    let n = x.bind(cx).len().unwrap_or(0);
    let names: Vec<Option<Vec<u8>>> = {
        let v = x.bind(cx);
        let nm = attrib::get_value(v, b"names");
        (0..n)
            .map(|i| {
                nm.and_then(|nm| nm.as_vector::<robigo_rtcore::Char<'_>>())
                    .map(|nv| nv.get(i).bytes().to_vec())
                    .filter(|b| !b.is_empty())
            })
            .collect()
    };
    let o = s.out(cx);
    let r = o.root();
    cx.build(o, |cx| {
        let v = x.bind(cx).as_vector::<robigo_rtcore::Ref<'_>>().unwrap();
        let mut l = cx.nil();
        for i in (0..n).rev() {
            l = match &names[i] {
                Some(b) => {
                    let t = cx.install(b)?;
                    cx.cons_tagged(v.get(i), l, t)?
                }
                None => cx.cons(v.get(i), l)?,
            };
        }
        Ok(l)
    })?;
    if n > 0 {
        // copyMostAttrib(x, xnew)
        let most: Vec<(robigo_rtcore::Ref<'_>, robigo_rtcore::Ref<'_>)> = attrib::attrs(x.bind(cx))
            .filter(|a| !matches!(a.tag.name().bytes(), b"names" | b"dim" | b"dimnames"))
            .map(|a| (a.tag.as_ref(), a.value))
            .collect();
        let most: Vec<(Root<'_>, Root<'_>)> = most.into_iter().map(|(t, v)| (s.root(cx, t), s.root(cx, v))).collect();
        for (t, v) in most {
            cx.install_attrib(r, t, v)?;
        }
        if x.bind(cx).is_object() {
            cx.set_object_bit(r, true);
        }
    }
    Ok(r)
}

/// `ascommon(call, u, type)`: `u` coerced to `type` for `as.vector` and
/// the `as.*` primitives.
// R: coerce.c:ascommon
fn ascommon<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, call: Root<'_>, u: Root<'_>, ty: u8) -> RResult<Root<'s>> {
    let ut = u.bind(cx).sexptype();
    if ty == CLOSXP {
        return Err(crate::util::not_yet(cx, Some(call), "as.function (asFunction)"));
    }
    let is_vec = attrib::is_vector(u.bind(cx)) || matches!(ut, NILSXP | LISTSXP | LANGSXP);
    if is_vec || (ut == SYMSXP && ty == EXPRSXP) {
        let v = if ty != ANYSXP && ut != ty {
            if ty == LISTSXP && matches!(ut, VECSXP | EXPRSXP) {
                vector_to_pairlist(cx, s, u)?
            } else if ty == LISTSXP {
                return Err(crate::util::not_yet(cx, Some(call), "as.pairlist() of an atomic vector"));
            } else {
                let o = s.out(cx);
                let r = o.root();
                crate::coerce::coerce_vector(cx, u, ty, o)?;
                r
            }
        } else {
            s.root(cx, u.bind(cx))
        };
        if ty == LISTSXP && !matches!(ut, LANGSXP | LISTSXP | EXPRSXP | VECSXP) {
            return without_attrib(cx, s, v);
        }
        return Ok(v);
    }
    if ut == SYMSXP && ty == STRSXP {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
            v.set(0, u.bind(cx).as_sym().unwrap().name());
            Ok(v.finish().into())
        })?;
        return Ok(r);
    }
    if ut == SYMSXP && ty == SYMSXP {
        return Ok(s.root(cx, u.bind(cx)));
    }
    if ut == SYMSXP && ty == VECSXP {
        let o = s.out(cx);
        let r = o.root();
        cx.build(o, |cx| {
            let mut v = cx.alloc_vector::<robigo_rtcore::Ref<'_>>(1)?;
            v.set(0, u.bind(cx));
            Ok(v.finish().into())
        })?;
        return Ok(r);
    }
    let msg = format!(
        "cannot coerce type '{}' to vector of type '{}'",
        crate::util::type_to_char(u.bind(cx)),
        crate::util::type2char(ty)
    );
    Err(cx.error_call(Some(call), &msg))
}

/// `as.character(x, ...)` (`CODE` 0) and its siblings `as.integer` (1),
/// `as.double` (2), `as.complex` (3), `as.logical` (4), `as.raw` (5): an
/// internal generic; the value coerced, attributes dropped.
// R: coerce.c:do_asatomic
pub fn do_asatomic<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    check1arg(cx, args, call, "x")?;
    static GENERICS: [Generic; 6] = [
        Generic::builtin("as.character"),
        Generic::builtin("as.integer"),
        Generic::builtin("as.double"),
        Generic::builtin("as.complex"),
        Generic::builtin("as.logical"),
        Generic::builtin("as.raw"),
    ];
    let ty = [STRSXP, INTSXP, REALSXP, CPLXSXP, LGLSXP, RAWSXP][CODE as usize];
    let out = match cx.try_dispatch(&GENERICS[CODE as usize], call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(o) => o,
    };
    // checkArity: these primitives take any number of arguments (-1), so
    // there is nothing to check
    cx.scope(|cx, s| {
        let x = args.root(0);
        let r = if x.bind(cx).sexptype() == ty {
            without_attrib(cx, s, x)?
        } else {
            let v = ascommon(cx, s, call, x, ty)?;
            without_attrib(cx, s, v)?
        };
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

const AS_VECTOR: Generic = Generic::builtin("as.vector");

/// `.Internal(as.vector(x, mode))`: an internal generic; `x` coerced to the
/// mode ("any": a vector as it is), attributes dropped except for lists
/// and language.
// R: coerce.c:do_asvector
pub fn do_asvector(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    let out = match cx.try_dispatch(&AS_VECTOR, call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(o) => o,
    };
    crate::registry::lookup("as.vector", true).expect("robigo: a registry row").check_arity(cx, call, args)?;
    let mode = args.get(cx, 1);
    let Some(m) = mode.as_vector::<robigo_rtcore::Char<'_>>().filter(|v| mode.sexptype() == STRSXP && v.len() == 1)
    else {
        return Err(cx.error("invalid 'mode' argument"));
    };
    let ms = m.get(0).bytes().to_vec();
    let ty = if ms == b"function" { Some(CLOSXP) } else { crate::util::str2type(&ms) };
    let x = args.get(cx, 0);
    let xt = x.sexptype();
    if ty == Some(ANYSXP) || Some(xt) == ty {
        match xt {
            LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => {
                return cx.scope(|cx, s| {
                    let r = without_attrib(cx, s, args.root(0))?;
                    out.set(cx, r.bind(cx));
                    Ok(())
                });
            }
            EXPRSXP | VECSXP => {
                // (_R_IS_AS_VECTOR_EXPERIMENTS_ is unset)
                out.set(cx, x);
                return Ok(());
            }
            _ => {}
        }
    }
    if x.is_s4() && xt == OBJSXP {
        return Err(crate::util::not_yet(cx, Some(call), "as.vector() of an S4 object"));
    }
    let Some(ty) = ty.filter(|t| {
        matches!(
            *t,
            SYMSXP
                | LGLSXP
                | INTSXP
                | REALSXP
                | CPLXSXP
                | STRSXP
                | EXPRSXP
                | VECSXP
                | LISTSXP
                | CLOSXP
                | RAWSXP
                | ANYSXP
        )
    }) else {
        return Err(cx.error("invalid 'mode' argument"));
    };
    cx.scope(|cx, s| {
        let r = ascommon(cx, s, call, args.root(0), ty)?;
        let r = match r.bind(cx).sexptype() {
            NILSXP | LISTSXP | LANGSXP | VECSXP | EXPRSXP => r,
            _ => without_attrib(cx, s, r)?,
        };
        out.set(cx, r.bind(cx));
        Ok(())
    })
}

/// The names a list's element `i` carries as a tag (`ItemName`, `!StringBlank`):
/// None when unnamed or blank.
fn item_names(x: robigo_rtcore::Ref<'_>, n: usize) -> Vec<Option<Vec<u8>>> {
    let nm = attrib::get_value(x, b"names").and_then(|nm| nm.as_vector::<robigo_rtcore::Char<'_>>());
    (0..n).map(|i| nm.map(|v| v.get(i).bytes().to_vec()).filter(|b| !b.is_empty())).collect()
}

/// A call from the list `args[[0]]`'s elements, its tags the non-blank
/// names (`allocList` filled, then `SET_TYPEOF(LANGSXP)`), the first
/// element's tag dropped.
fn call_from_list(cx: &mut Cx, args: Args<'_>, idx: usize, names: &[Option<Vec<u8>>], out: Out<'_>) -> RResult<()> {
    let n = names.len();
    cx.build(out, |cx| {
        let v = args.get(cx, idx).as_vector::<robigo_rtcore::Ref<'_>>().unwrap();
        let mut l = cx.nil();
        for i in (1..n).rev() {
            l = match &names[i] {
                Some(b) => cx.cons_tagged(v.get(i), l, cx.install(b)?)?,
                None => cx.cons(v.get(i), l)?,
            };
        }
        cx.lcons(v.get(0), l)
    })
}

const AS_CALL: Generic = Generic::builtin("as.call");

/// `as.call(x)`: a call from a list or expression vector (tagged by its
/// non-blank names), or a call itself. An internal generic.
// R: coerce.c:do_ascall
pub fn do_ascall(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::lookup("as.call", false).expect("robigo: a registry row").check_arity(cx, call, args)?;
    check1arg(cx, args, call, "x")?;
    let out = match cx.try_dispatch(&AS_CALL, call, args, env, out)? {
        Dispatched::Done => return Ok(()),
        Dispatched::Default(out) => out,
    };
    let x = args.get(cx, 0);
    match x.sexptype() {
        LANGSXP => {
            out.set(cx, x);
            Ok(())
        }
        VECSXP | EXPRSXP => {
            let n = x.len().unwrap_or(0);
            if n == 0 {
                return Err(cx.error_call(Some(call), "invalid length 0 argument"));
            }
            let names = item_names(x, n);
            call_from_list(cx, args, 0, &names, out)
        }
        LISTSXP => Err(crate::util::not_yet(cx, Some(call), "as.call() of a pairlist")),
        STRSXP => Err(cx.error_call(Some(call), "as.call(<character>) not feasible; consider str2lang(<char.>)")),
        _ => Err(cx.error_call(Some(call), "invalid argument list")),
    }
}
