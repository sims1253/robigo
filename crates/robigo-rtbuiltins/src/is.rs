//! coerce.c's `do_is`: the `is.*` type predicates (`is.null`,
//! `is.character`, `is.numeric`, `is.function`, ...), one function per
//! names.c `code`. `is.numeric`, `is.matrix` and `is.array` are internal
//! generics for objects.

use crate::attrib;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Dispatched, Generic, Lgl, Out, RResult, Root};

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
    crate::util::check1arg(cx, args, call, "x")?;
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
    crate::util::check1arg(cx, args, call, "x")?;
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
