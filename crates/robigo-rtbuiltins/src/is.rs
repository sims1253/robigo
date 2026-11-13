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
            robigo_rtcore::View::Cplx(v) => v.as_slice().unwrap().iter().map(|z| z.r.is_nan() || z.i.is_nan()).collect(),
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
