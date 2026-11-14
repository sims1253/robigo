//! Small pieces of GNU R's util.c and coerce.c every builtin here uses:
//! type names for messages, the type predicates, `asLogical`, and the
//! error for what is not implemented yet.

use crate::attrib;
use crate::kernel::logic;
use robigo_rtcore::types::*;
use robigo_rtcore::{Char, Cx, RError, Rcomplex, Ref};

/// GNU R's `type2char`: the first name `TypeTable` gives a type.
// R: util.c:TypeTable, util.c:type2char
pub fn type2char(t: u8) -> &'static str {
    match t {
        NILSXP => "NULL",
        SYMSXP => "symbol",
        LISTSXP => "pairlist",
        CLOSXP => "closure",
        ENVSXP => "environment",
        PROMSXP => "promise",
        LANGSXP => "language",
        SPECIALSXP => "special",
        BUILTINSXP => "builtin",
        CHARSXP => "char",
        LGLSXP => "logical",
        INTSXP => "integer",
        REALSXP => "double",
        CPLXSXP => "complex",
        STRSXP => "character",
        DOTSXP => "...",
        ANYSXP => "any",
        EXPRSXP => "expression",
        VECSXP => "list",
        EXTPTRSXP => "externalptr",
        BCODESXP => "bytecode",
        WEAKREFSXP => "weakref",
        RAWSXP => "raw",
        OBJSXP => "S4",
        _ => "unknown type",
    }
}

/// GNU R's `R_typeToChar(x)`: `type2char`, but an OBJSXP that is not S4 is
/// "object".
// R: util.c:R_typeToChar
pub fn type_to_char(x: Ref<'_>) -> &'static str {
    if x.sexptype() == OBJSXP && !x.is_s4() { "object" } else { type2char(x.sexptype()) }
}

/// GNU R's `str2type`: the type a name stands for (the aliases `numeric`
/// and `name` included), None for none.
// R: util.c:str2type
pub fn str2type(s: &[u8]) -> Option<u8> {
    Some(match s {
        b"NULL" => NILSXP,
        b"symbol" | b"name" => SYMSXP,
        b"pairlist" => LISTSXP,
        b"closure" => CLOSXP,
        b"environment" => ENVSXP,
        b"promise" => PROMSXP,
        b"language" => LANGSXP,
        b"special" => SPECIALSXP,
        b"builtin" => BUILTINSXP,
        b"char" => CHARSXP,
        b"logical" => LGLSXP,
        b"integer" => INTSXP,
        b"double" | b"numeric" => REALSXP,
        b"complex" => CPLXSXP,
        b"character" => STRSXP,
        b"..." => DOTSXP,
        b"any" => ANYSXP,
        b"expression" => EXPRSXP,
        b"list" => VECSXP,
        b"externalptr" => EXTPTRSXP,
        b"bytecode" => BCODESXP,
        b"weakref" => WEAKREFSXP,
        b"raw" => RAWSXP,
        b"S4" | b"object" => OBJSXP,
        _ => return None,
    })
}

/// GNU R's `isNumeric`: integer (not a factor), logical or double.
pub fn is_numeric(x: Ref<'_>) -> bool {
    match x.sexptype() {
        INTSXP => !attrib::inherits(x, b"factor"),
        LGLSXP | REALSXP => true,
        _ => false,
    }
}

/// GNU R's `isNumber`: [`is_numeric`] or complex.
pub fn is_number(x: Ref<'_>) -> bool {
    is_numeric(x) || x.sexptype() == CPLXSXP
}

/// GNU R's `isVectorAtomic`.
pub fn is_vector_atomic(x: Ref<'_>) -> bool {
    matches!(x.sexptype(), LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP)
}

/// `StringTrue` / `StringFalse` (util.c): the spellings `as.logical` reads.
// R: util.c:StringTrue, util.c:StringFalse
fn logical_from_string(c: Char<'_>, na: Char<'_>) -> i32 {
    if c == na {
        return logic::NA;
    }
    match c.bytes() {
        b"T" | b"True" | b"TRUE" | b"true" => 1,
        b"F" | b"False" | b"FALSE" | b"false" => 0,
        _ => logic::NA,
    }
}

/// `LogicalFromComplex`.
// R: coerce.c:LogicalFromComplex
pub fn logical_from_complex(z: Rcomplex) -> i32 {
    if z.r.is_nan() || z.i.is_nan() { logic::NA } else { (z.r != 0.0 || z.i != 0.0) as i32 }
}

/// GNU R's `asLogical2(x, checking = 0)`, i.e. `asLogical`: the first
/// element of an atomic vector as a logical; NA for anything else (and
/// for a package ALTREP vector, which the callers reject first).
// R: coerce.c:asLogical2
pub fn as_logical(cx: &Cx, x: Ref<'_>) -> i32 {
    as_logical_n(cx, x).0
}

/// [`as_logical`] and the vector's length (for `asLogical2`'s checking
/// mode, which refuses more than one element).
pub fn as_logical_n(cx: &Cx, x: Ref<'_>) -> (i32, usize) {
    let n = x.len().unwrap_or(0);
    if !is_vector_atomic(x) || n < 1 || x.is_package_altrep() {
        return (logic::NA, n);
    }
    let v = match x.view() {
        robigo_rtcore::View::Lgl(v) => v.get(0).0,
        robigo_rtcore::View::Int(v) => logic::from_int(v.get(0)),
        robigo_rtcore::View::Real(v) => logic::from_real(v.get(0)),
        robigo_rtcore::View::Cplx(v) => logical_from_complex(v.get(0)),
        robigo_rtcore::View::Str(v) => logical_from_string(v.get(0), cx.na_string()),
        robigo_rtcore::View::Raw(v) => logic::from_int(v.get(0) as i32),
        _ => logic::NA,
    };
    (v, n)
}

/// The error for a builtin path robigo does not have yet (AGENTS rule 6),
/// reported against `call`.
#[cold]
pub fn not_yet(cx: &mut Cx, call: Option<robigo_rtcore::Root<'_>>, what: &str) -> RError {
    cx.error_call(call, &format!("robigo: {what} is not implemented yet"))
}

/// GNU R's `asInteger(x)`: the first element of an atomic vector as an
/// integer, `CoercionWarning`'s warnings for what `IntegerFromReal` and
/// `IntegerFromComplex` flag; NA for anything else (a zero-length vector
/// too, as GNU R's). A character element is "not implemented yet"
/// (`String2Real`).
// R: coerce.c:asInteger, IntegerFromReal, IntegerFromComplex, CoercionWarning
pub fn as_integer(cx: &mut Cx, x: robigo_rtcore::Root<'_>) -> robigo_rtcore::RResult<i32> {
    use crate::kernel::arith::NA_INTEGER;
    let v = x.bind(cx);
    if !is_vector_atomic(v) || v.len().unwrap_or(0) < 1 || v.is_package_altrep() {
        return Ok(NA_INTEGER);
    }
    // CoercionWarning's flags (WARN_INT_NA, WARN_IMAG)
    let (r, int_na, imag) = match v.view() {
        robigo_rtcore::View::Lgl(l) => (l.get(0).0, false, false),
        robigo_rtcore::View::Int(i) => (i.get(0), false, false),
        robigo_rtcore::View::Real(d) => {
            let d = d.get(0);
            if d.is_nan() {
                (NA_INTEGER, false, false)
            } else if d >= 2147483648.0 || d <= -2147483648.0 {
                // x >= INT_MAX+1. || x <= INT_MIN
                (NA_INTEGER, true, false)
            } else {
                (d as i32, false, false)
            }
        }
        robigo_rtcore::View::Cplx(z) => {
            let z = z.get(0);
            if z.r.is_nan() || z.i.is_nan() {
                (NA_INTEGER, false, false)
            } else if z.r > 2147483648.0 || z.r <= -2147483648.0 {
                // x.r > INT_MAX+1. || x.r <= INT_MIN: 2^31 itself gets through
                (NA_INTEGER, true, false)
            } else {
                // (int) x.r; for 2^31, outside int's range, GNU R's x86-64
                // build gives INT_MIN (cvttsd2si), which is NA_INTEGER
                let r = if z.r >= 2147483648.0 { NA_INTEGER } else { z.r as i32 };
                (r, false, z.i != 0.0)
            }
        }
        robigo_rtcore::View::Raw(r) => (r.get(0) as i32, false, false),
        _ => return Err(not_yet(cx, None, "asInteger of a character value")),
    };
    if int_na {
        cx.warning("NAs introduced by coercion to integer range")?;
    }
    if imag {
        cx.warning("imaginary parts discarded in coercion")?;
    }
    Ok(r)
}

/// GNU R's `asBool2(x, call)`: `asLogical2(x, checking = 1, call)`, NA an
/// error ("NA in coercion to boolean"), an atomic vector of more than one
/// element an error ("'length = %lld' in coercion to 'logical(1)'"), both
/// against `call`. As in `asLogical2`, the length check is for atomic
/// vectors only and a zero-length one is NA: a list, or `logical(0)`, is
/// "NA in coercion to boolean".
// R: coerce.c:asBool2, asLogical2
pub fn as_bool2(
    cx: &mut Cx,
    x: robigo_rtcore::Root<'_>,
    call: robigo_rtcore::Root<'_>,
) -> robigo_rtcore::RResult<bool> {
    let (v, n) = as_logical_n(cx, x.bind(cx));
    if is_vector_atomic(x.bind(cx)) && n > 1 {
        let msg = format!("'length = {n}' in coercion to 'logical(1)'");
        return Err(cx.error_call(Some(call), &msg));
    }
    if v == logic::NA {
        return Err(cx.error_call(Some(call), "NA in coercion to boolean"));
    }
    Ok(v != 0)
}

/// GNU R's `check1arg(args, call, formal)`: the first argument, if named,
/// must be a prefix of `formal`; a partial match warns under
/// `options(warnPartialMatchArgs)` ("partial argument match of '%s' to
/// '%s'", against `call`).
// R: util.c:Rf_check1arg, errors.c:R_makePartialMatchWarningCondition
pub fn check1arg(
    cx: &mut Cx,
    args: robigo_rtcore::Args<'_>,
    call: robigo_rtcore::Root<'_>,
    formal: &str,
) -> robigo_rtcore::RResult<()> {
    let Some(tag) = (!args.is_empty()).then(|| args.name(cx, 0)).flatten() else { return Ok(()) };
    let supplied = String::from_utf8_lossy(tag.name().bytes()).into_owned();
    if !formal.starts_with(&supplied) {
        let msg = format!("supplied argument name '{supplied}' does not match '{formal}'");
        return Err(cx.error_call(Some(call), &msg));
    }
    if cx.warn_partial_match_args() && !supplied.is_empty() && supplied.len() < formal.len() {
        let msg = format!("partial argument match of '{supplied}' to '{formal}'");
        cx.warning_call(Some(call), &msg)?;
    }
    Ok(())
}
