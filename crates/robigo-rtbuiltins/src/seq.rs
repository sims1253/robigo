//! seq.c's `:` (`do_colon`, `seq_colon`).
//!
//! Not here yet, each an error that says so: two factors (`cross_colon`),
//! a character, complex or raw operand (`asReal`'s coercions with their
//! warnings), and an environment operand (`envlength`). The result is an ordinary integer or double vector where
//! GNU R makes a compact ALTREP sequence (`R_compact_intrange`): the same
//! values, type and length; only `.Internal(inspect())` and memory show
//! the difference.

use crate::attrib::inherits;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Lgl, Out, RResult, Ref, Root};

/// `R_XLEN_T_MAX`.
const XLEN_MAX: f64 = 4503599627370496.0;

/// `from:to`.
// R: seq.c:do_colon
pub fn do_colon(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::registry::by_name(":").unwrap().check_arity(cx, call, args)?;
    let (s1, s2) = (args.get(cx, 0), args.get(cx, 1));
    if inherits(s1, b"factor") && inherits(s2, b"factor") {
        return Err(crate::util::not_yet(cx, Some(call), "':' of two factors (cross_colon)"));
    }
    let (Some(n1), Some(n2)) = (length(s1), length(s2)) else {
        return Err(crate::util::not_yet(cx, Some(call), "':' of an environment (envlength)"));
    };
    if n1 != 1 || n2 != 1 {
        if n1 == 0 || n2 == 0 {
            return Err(cx.error_call(Some(call), "argument of length 0"));
        }
        // _R_CHECK_LENGTH_COLON_ unset: warn
        let n = if n1 > 1 { n1 } else { n2 };
        let msg = format!("numerical expression has {} elements: only the first used", n as i32);
        cx.warning_call(Some(call), &msg)?;
    }
    let (Some(a), Some(b)) = (as_real(args.get(cx, 0)), as_real(args.get(cx, 1))) else {
        return Err(crate::util::not_yet(cx, Some(call), "':' of a character, complex or raw operand (asReal)"));
    };
    if a.is_nan() || b.is_nan() {
        return Err(cx.error_call(Some(call), "NA/NaN argument"));
    }
    seq_colon(cx, a, b, call, out)
}

// R: seq.c:seq_colon
fn seq_colon(cx: &mut Cx, n1: f64, n2: f64, call: Root<'_>, out: Out<'_>) -> RResult<()> {
    let r = (n2 - n1).abs();
    if r >= XLEN_MAX {
        return Err(cx.error_call(Some(call), "result would be too long a vector"));
    }
    if n1 == (n1 as i64) as f64 && n2 == (n2 as i64) as f64 {
        return intrange(cx, n1 as i64, n2 as i64, call, out);
    }
    let n = (r + 1.0 + f32::EPSILON as f64) as i64;
    let mut use_int = n1 <= i32::MAX as f64 && n1 == (n1 as i32) as f64;
    if use_int {
        if n1 <= i32::MIN as f64 {
            use_int = false;
        } else {
            let dn = n as f64;
            let r = n1 + if n1 <= n2 { dn - 1.0 } else { -(dn - 1.0) };
            if r <= i32::MIN as f64 || r > i32::MAX as f64 {
                use_int = false;
            }
        }
    }
    if use_int {
        let from = n1 as i64;
        let to = if n1 <= n2 { from + n - 1 } else { from - n + 1 };
        return intrange(cx, from, to, call, out);
    }
    let n = n as usize;
    let up = n1 <= n2;
    cx.build(out, |cx| Ok(cx.alloc_with(n, |i| if up { n1 + i as f64 } else { n1 - i as f64 })?.finish().into()))
}

/// `R_compact_intrange(n1, n2)`: integers when both ends are, else doubles.
// R: altclasses.c:R_compact_intrange
fn intrange(cx: &mut Cx, n1: i64, n2: i64, call: Root<'_>, out: Out<'_>) -> RResult<()> {
    let n = (n1 - n2).unsigned_abs() + 1;
    if n as f64 >= XLEN_MAX {
        return Err(cx.error_call(Some(call), "result would be too long a vector"));
    }
    let n = n as usize;
    let up = n1 <= n2;
    let ints = (i32::MIN as i64) < n1 && n1 <= i32::MAX as i64 && (i32::MIN as i64) < n2 && n2 <= i32::MAX as i64;
    cx.build(out, |cx| {
        if ints {
            let a = n1 as i32;
            Ok(cx.alloc_with(n, |i| if up { a + i as i32 } else { a - i as i32 })?.finish().into())
        } else {
            let a = n1 as f64;
            Ok(cx.alloc_with(n, |i| if up { a + i as f64 } else { a - i as f64 })?.finish().into())
        }
    })
}

/// `length(x)` as `do_colon` reads it: a vector's length, a pairlist's,
/// 1 for anything else; `None` for an environment (`envlength`).
// R: memory.c:Rf_length
fn length(x: Ref<'_>) -> Option<usize> {
    Some(match x.sexptype() {
        NILSXP => 0,
        ENVSXP => return None,
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | VECSXP | EXPRSXP | RAWSXP => x.len()?,
        LISTSXP | LANGSXP | DOTSXP => {
            let mut n = 0;
            let mut c = x;
            while let Some(cell) = c.as_cons() {
                n += 1;
                c = cell.cdr();
            }
            n
        }
        _ => 1,
    })
}

/// `asReal(x)` for logical, integer and double operands (no coercion
/// warning is possible from these); `Some(NA)` for what is not an atomic
/// vector; `None` for the character, complex and raw cases.
// R: coerce.c:asReal
fn as_real(x: Ref<'_>) -> Option<f64> {
    let na = robigo_numerics::NA_REAL;
    let n = match x.sexptype() {
        LGLSXP | INTSXP | REALSXP | CPLXSXP | STRSXP | RAWSXP => x.len().unwrap_or(0),
        _ => return Some(na),
    };
    if n == 0 {
        return Some(na);
    }
    match x.sexptype() {
        LGLSXP => {
            let v = x.as_vector::<Lgl>()?.get(0);
            Some(if v == Lgl::NA { na } else { v.0 as f64 })
        }
        INTSXP => {
            let v = x.as_vector::<i32>()?.get(0);
            Some(if v == i32::MIN { na } else { v as f64 })
        }
        REALSXP => Some(x.as_vector::<f64>()?.get(0)),
        // a raw vector is UNIMPLEMENTED_TYPE("asReal") in GNU R
        _ => None,
    }
}
