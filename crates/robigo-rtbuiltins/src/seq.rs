//! seq.c's `:` (`do_colon`, `seq_colon`).
//!
//! Not here yet, each an error that says so: two factors (`cross_colon`),
//! a character, complex or raw operand (`asReal`'s coercions with their
//! warnings), and an environment operand (`envlength`). The result is an ordinary integer or double vector where
//! GNU R makes a compact ALTREP sequence (`R_compact_intrange`): the same
//! values, type and length; only `.Internal(inspect())` and memory show
//! the difference.

use crate::attrib::inherits;
use robigo_rtcore::RSym;
use robigo_rtcore::types::*;
use robigo_rtcore::{Args, Cx, Lgl, Out, RResult, Ref, Root};

/// `R_XLEN_T_MAX`.
const XLEN_MAX: f64 = 4503599627370496.0;

/// `from:to`.
// R: seq.c:do_colon
pub fn do_colon(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!(":").check_arity(cx, call, args)?;
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
    if n1 == c_ftoi(n1) as f64 && n2 == c_ftoi(n2) as f64 {
        return intrange(cx, n1 as i64, n2 as i64, call, out);
    }
    let n = c_ftoi(r + 1.0 + f32::EPSILON as f64);
    if n < 0 {
        // allocVector of the length C's conversion gave: `Inf:Inf`, whose
        // `r` is NaN
        return Err(cx.error("negative length vectors are not allowed"));
    }
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

/// C's `(R_xlen_t) x` as x86-64 computes it (`cvttsd2si`): truncation, and
/// the "integer indefinite" `i64::MIN` for NaN and out-of-range values,
/// where Rust's `as` saturates.
fn c_ftoi(x: f64) -> i64 {
    if x.is_nan() || x >= 9223372036854775808.0 || x < -9223372036854775808.0 { i64::MIN } else { x as i64 }
}

/// C's `(int) x` as x86-64 computes it (a 32-bit `cvttsd2si`): truncation,
/// and the "integer indefinite" `i32::MIN` for NaN and out-of-range values,
/// where Rust's `as` saturates.
fn c_ftoi32(x: f64) -> i32 {
    if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 { i32::MIN } else { x as i32 }
}

/// `R_compact_intrange(n1, n2)`: a compact integer sequence when both ends
/// are integers, else a compact real one (the scalar for one element).
// R: altclasses.c:R_compact_intrange
fn intrange(cx: &mut Cx, n1: i64, n2: i64, _call: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.compact_intrange(n1, n2, out)
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

// ---- rep, rep_len, seq_len, seq_along (B2b) ---------------------------------

use crate::attrib::{self as attr, Attr, Attrs};
use crate::coerce::vector::{as_integer, as_real as as_real_w, coerce};
use robigo_rtcore::{Built, Char, Dispatched, Generic, NeedGc, Rcomplex, Scope, View};

/// `xlengthgets(x, len)` of a vector `x` (`lenx <= len`): `x`'s elements,
/// then NA (raw 0, list NULL) up to `len`; names likewise, padded with "".
/// The class is not kept.
// R: builtin.c:xlengthgets
pub fn xlengthgets<'g>(cx: &'g Cx, x: Ref<'g>, len: usize) -> Result<Ref<'g>, NeedGc> {
    let lenx = x.len().unwrap_or(0);
    if lenx == len {
        return Ok(x);
    }
    let mut a = Attrs::new();
    if let Some(nm) = attr::names(x).and_then(|n| n.as_vector::<Char<'g>>()) {
        let mut names = cx.alloc_vector::<Char<'g>>(len)?;
        for i in 0..len.min(lenx) {
            names.set(i, nm.get(i));
        }
        a.install(Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
    }
    macro_rules! plain {
        ($t:ty, $v:expr, $na:expr) => {{
            let v = $v;
            let mut r = cx.alloc_with::<$t>(len, |i| if i < lenx { v.get(i) } else { $na })?;
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match x.view() {
        View::Lgl(v) => plain!(Lgl, v, Lgl::NA),
        View::Int(v) => plain!(i32, v, i32::MIN),
        View::Real(v) => plain!(f64, v, robigo_text::NA_REAL),
        View::Cplx(v) => plain!(Rcomplex, v, Rcomplex { r: robigo_text::NA_REAL, i: robigo_text::NA_REAL }),
        View::Raw(v) => plain!(u8, v, 0),
        View::Str(v) => {
            let mut r = cx.alloc_vector::<Char<'g>>(len)?;
            for i in 0..len {
                r.set(i, if i < lenx { v.get(i) } else { cx.na_string() });
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut r =
                if x.sexptype() == EXPRSXP { cx.alloc_expression(len)? } else { cx.alloc_vector::<Ref<'g>>(len)? };
            for i in 0..lenx.min(len) {
                r.set(i, v.get(i));
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        _ => x,
    })
}

/// The element-index function of a replication: element `k` of the
/// result is element `src(k)` of `x`.
fn replicate<'g>(
    cx: &'g Cx,
    x: Ref<'g>,
    len: usize,
    src: impl Fn(usize) -> usize,
    a: &Attrs<'g>,
) -> Result<Ref<'g>, NeedGc> {
    macro_rules! plain {
        ($t:ty, $v:expr) => {{
            let v = $v.as_slice().unwrap();
            let mut r = cx.alloc_with::<$t>(len, |k| v[src(k)])?;
            a.apply(cx, &mut r)?;
            r.finish().into()
        }};
    }
    Ok(match x.view() {
        View::Lgl(v) => plain!(Lgl, v),
        View::Int(v) => plain!(i32, v),
        View::Real(v) => plain!(f64, v),
        View::Cplx(v) => plain!(Rcomplex, v),
        View::Raw(v) => plain!(u8, v),
        View::Str(v) => {
            let mut r = cx.alloc_vector::<Char<'g>>(len)?;
            for k in 0..len {
                r.set(k, v.get(src(k)));
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        View::List(v) | View::Expr(v) => {
            let mut r =
                if x.sexptype() == EXPRSXP { cx.alloc_expression(len)? } else { cx.alloc_vector::<Ref<'g>>(len)? };
            for k in 0..len {
                r.set(k, v.get(src(k)));
            }
            a.apply(cx, &mut r)?;
            r.finish().into()
        }
        _ => unreachable!("robigo: a vector"),
    })
}

/// `rep4(x, times, len, each, nt)` (`rep3` when `each == 1 && nt == 1`):
/// `times` per element (after `each`), or recycled to `len`.
// R: seq.c:rep4, seq.c:rep3
fn rep4<'g>(
    cx: &'g Cx,
    x: Ref<'g>,
    times: &[usize],
    len: usize,
    each: usize,
    a: &Attrs<'g>,
) -> Result<Ref<'g>, NeedGc> {
    let lx = x.len().unwrap_or(0);
    let nt = times.len();
    if nt == 1 {
        return replicate(cx, x, len, |k| (k / each) % lx, a);
    }
    // the source index of each output element, stopping at len
    let mut idx = Vec::with_capacity(len);
    let mut k = 0;
    'outer: for i in 0..lx {
        let mut sum = 0;
        for _ in 0..each {
            sum += times[k];
            k += 1;
        }
        for _ in 0..sum {
            idx.push(i);
            if idx.len() == len {
                break 'outer;
            }
        }
    }
    // (a `times` that sums below len leaves the rest at its zero value,
    // as GNU R's unfilled allocVector does: never, len is the sum)
    replicate(cx, x, idx.len(), |k| idx[k], a)
}

const REP: Generic = Generic { name: "rep", group: None, drop_missing: false, args_evaluated: true };

/// `rep(x, ...)`: a special that evaluates its arguments, dispatches on
/// the first, and matches `(x, times, length.out, each, ...)` itself.
// R: seq.c:do_rep
pub fn do_rep(cx: &mut Cx, call: Root<'_>, _args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        // DispatchOrEval(..., argsevald = 0): the arguments evaluated
        // (missing ones kept, each linked while the later ones run), then a
        // method for an object
        let args = cx.eval_args_keep_missing(s, call, env)?;
        let Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &REP, call, args, env, out)? else {
            return Ok(());
        };
        rep_dflt(cx, s, call, args, out)
    })
}

/// The integer or double `times`, `length.out` or `each` argument's
/// value, as do_rep reads it (`asReal` for a non-integer: its warnings).
fn rep_count(cx: &mut Cx, v: Option<Root<'_>>) -> RResult<Option<f64>> {
    let Some(v) = v else { return Ok(None) };
    let x = v.bind(cx);
    if x == cx.missing_arg() {
        return Ok(None);
    }
    if x.sexptype() == INTSXP {
        let (n, _) = as_integer(cx, x);
        return Ok((n != i32::MIN).then_some(n as f64));
    }
    let (d, w) = as_real_w(cx, x);
    crate::coerce::vector::coercion_warning(cx, w)?;
    Ok(Some(d))
}

fn rep_dflt(cx: &mut Cx, s: Scope<'_>, call: Root<'_>, args: Args<'_>, out: Out<'_>) -> RResult<()> {
    // rep3 and rep4 read `x` and `times` through INTEGER and REAL, which
    // expand one of robigo's compact sequences
    // R: seq.c:rep3, seq.c:rep4
    cx.materialize_args(args)?;
    let m = crate::util::match_args(cx, call, &["x", "times", "length.out", "each", "..."], args)?;
    fn arg<'g>(cx: &'g Cx, m: &crate::util::Matched, args: Args<'_>, k: usize) -> Option<Ref<'g>> {
        m.formals[k].map(|i| args.get(cx, i)).filter(|v| *v != cx.missing_arg())
    }
    let get = |cx: &Cx, k: usize| arg(cx, &m, args, k).map(|v| (v.sexptype(), crate::coerce::xlength(v)));
    let x = match m.formals[0] {
        Some(i) => s.root(cx, args.get(cx, i)),
        None => s.root(cx, cx.missing_arg()),
    };
    if x.bind(cx).sexptype() == LISTSXP {
        return Err(cx.error_call(Some(call), "replication of pairlists is defunct"));
    }
    if x.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "rep() of a package ALTREP vector"));
    }
    let lx = crate::coerce::xlength(x.bind(cx));
    const XMAX: f64 = 4503599627370496.0;
    // length.out
    let lo = m.formals[2].map(|i| args.root(i));
    let lo_len = get(cx, 2).map_or(1, |v| v.1);
    let lo_type = get(cx, 2).map(|v| v.0);
    let mut len: Option<usize> = None;
    let missing_lo = get(cx, 2).is_none();
    if lo_type == Some(INTSXP) {
        let n = rep_count(cx, lo)?;
        if let Some(n) = n {
            if n < 0.0 {
                return Err(cx.error_call(Some(call), "invalid 'length.out' argument"));
            }
            len = Some(n as usize);
        }
    } else {
        let slen = if missing_lo { f64::NAN } else { rep_count(cx, lo)?.unwrap_or(f64::NAN) };
        if slen.is_finite() {
            if slen <= -1.0 || slen >= XMAX + 1.0 {
                return Err(cx.error_call(Some(call), "invalid 'length.out' argument"));
            }
            len = Some(slen as usize);
        }
    }
    if lo_len != 1 {
        cx.warning_call(Some(call), "first element used of 'length.out' argument")?;
    }
    // each
    let ea = m.formals[3].map(|i| args.root(i));
    let ea_len = get(cx, 3).map_or(1, |v| v.1);
    let missing_each = get(cx, 3).is_none();
    let mut each: Option<usize> = None;
    if get(cx, 3).map(|v| v.0) == Some(INTSXP) {
        if let Some(n) = rep_count(cx, ea)? {
            if n < 0.0 {
                return Err(cx.error_call(Some(call), "invalid 'each' argument"));
            }
            each = Some(n as usize);
        }
    } else {
        let seach = if missing_each { f64::NAN } else { rep_count(cx, ea)?.unwrap_or(f64::NAN) };
        if seach.is_finite() {
            if seach <= -1.0 || (lx > 0 && seach >= XMAX + 1.0) {
                return Err(cx.error_call(Some(call), "invalid 'each' argument"));
            }
            each = if lx == 0 { None } else { Some(seach as usize) };
        }
    }
    if ea_len != 1 {
        cx.warning_call(Some(call), "first element used of 'each' argument")?;
    }
    let each = each.unwrap_or(1);
    if lx == 0 {
        let isnull = x.bind(cx).sexptype() == NILSXP;
        if len.is_some_and(|l| l > 0) && isnull {
            cx.warning_call(Some(call), "'x' is NULL so the result will be NULL")?;
        }
        return cx.build(out, |cx| match len {
            Some(l) if l > 0 && !isnull => xlengthgets(cx, x.bind(cx), l),
            _ => Ok(x.bind(cx)),
        });
    }
    if !attr::is_vector(x.bind(cx)) {
        let msg = format!("attempt to replicate an object of type '{}'", crate::util::type_to_char(x.bind(cx)));
        return Err(cx.error_call(Some(call), &msg));
    }
    // the final length from times and each
    let mut times: Vec<usize> = vec![1];
    let len = match len {
        Some(l) => l,
        None => {
            let tr = get(cx, 1).map(|_| args.root(m.formals[1].unwrap()));
            let tv = match tr {
                None => s.root(cx, cx.nil()),
                Some(t) => {
                    let ty = if t.bind(cx).sexptype() != INTSXP { REALSXP } else { INTSXP };
                    let o = s.out(cx);
                    let r = o.root();
                    let w = cx.build_with(o, |cx| coerce(cx, t.bind(cx), ty, true))?;
                    w.signal(cx)?;
                    r
                }
            };
            let vals: Vec<f64> = match tv.bind(cx).view() {
                View::Null => vec![1.0],
                View::Real(v) => v.as_slice().unwrap().to_vec(),
                View::Int(v) => {
                    v.as_slice().unwrap().iter().map(|&i| if i == i32::MIN { f64::NAN } else { i as f64 }).collect()
                }
                _ => unreachable!("robigo: coerced to a number"),
            };
            let bad = |t: f64| t.is_nan() || t <= -1.0 || t >= XMAX + 1.0;
            let nt = vals.len();
            if nt == 1 {
                if bad(vals[0]) {
                    return Err(cx.error_call(Some(call), "invalid 'times' argument"));
                }
                let it = vals[0] as usize;
                if (lx as f64) * (it as f64) * (each as f64) > XMAX {
                    return Err(cx.error_call(Some(call), "length(x) * 'times' * 'each' is too large"));
                }
                lx * it * each
            } else {
                if nt as f64 != (lx as f64) * (each as f64) {
                    let msg = if each == 1 {
                        "invalid 'times' argument"
                    } else {
                        "invalid 'times' argument, given the value of 'each'"
                    };
                    return Err(cx.error_call(Some(call), msg));
                }
                if vals.iter().any(|&t| bad(t)) {
                    return Err(cx.error_call(Some(call), "invalid 'times' argument"));
                }
                times = vals.iter().map(|&t| t as usize).collect();
                times.iter().sum()
            }
        }
    };
    if len > 0 && each == 0 {
        return Err(cx.error_call(Some(call), "invalid 'each' argument"));
    }
    cx.build(out, |cx| {
        let xv = x.bind(cx);
        let mut a = Attrs::new();
        if let Some(xn) = attr::names(xv).filter(|n| n.len().unwrap_or(0) > 0) {
            let none = Attrs::new();
            let nr = rep4(cx, xn, &times, len, each, &none)?;
            a.install(Attr { tag: cx.sym(RSym::Names), value: nr });
        }
        rep4(cx, xv, &times, len, each, &a)
    })
}

const REP_INT: Generic = Generic::builtin("rep.int");

/// `.Internal(rep.int(x, times))`: `x` repeated `times` times (one count),
/// or each element its own count (`times` as long as `x`); a factor keeps
/// its class and levels, nothing else its attributes.
// R: seq.c:do_rep_int
pub fn do_rep_int(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("rep.int", true).check_arity(cx, call, args)?;
    // DispatchOrEval on "rep.int", then (but for a factor) on "rep"
    let Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &REP_INT, call, args, env, out)? else {
        return Ok(());
    };
    let out = if inherits(args.get(cx, 0), b"factor") {
        out
    } else {
        match crate::coerce::dispatch_or_eval(cx, &REP, call, args, env, out)? {
            Dispatched::Done => return Ok(()),
            Dispatched::Default(o) => o,
        }
    };
    let (s, ncopy) = (args.root(0), args.root(1));
    if !attr::is_vector(ncopy.bind(cx)) {
        let t = crate::util::type_to_char(ncopy.bind(cx));
        return Err(cx.error(&format!("invalid type ({t}) for 'times' (must be a vector)")));
    }
    let sv = s.bind(cx);
    if !attr::is_vector(sv) && sv.sexptype() != NILSXP {
        let t = crate::util::type_to_char(sv);
        return Err(cx.error(&format!("attempt to replicate an object of type '{t}'")));
    }
    if sv.is_package_altrep() || ncopy.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "rep.int() of a package ALTREP vector"));
    }
    let is_null = sv.sexptype() == NILSXP;
    let nc = crate::coerce::xlength(ncopy.bind(cx));
    let ns = crate::coerce::xlength(sv);
    // the source element of each result element: rep2's counts, or rep3's
    // copies of the whole vector
    let (times, len, rep) = if nc == ns {
        let (times, na) = rep2_counts(cx, ncopy)?;
        (times, na, "rep2")
    } else {
        if nc != 1 {
            return Err(cx.error("invalid 'times' value"));
        }
        let n = if ncopy.bind(cx).sexptype() != INTSXP {
            // asReal(ncopy): a raw vector is its UNIMPLEMENTED_TYPE
            if ncopy.bind(cx).sexptype() == RAWSXP {
                return Err(cx.error("unimplemented type 'raw' in 'asReal'\n"));
            }
            let (snc, w) = as_real_w(cx, ncopy.bind(cx));
            crate::coerce::vector::coercion_warning(cx, w)?;
            if !snc.is_finite() || snc <= -1.0 || (ns > 0 && snc >= XLEN_MAX + 1.0) {
                return Err(cx.error("invalid 'times' value"));
            }
            if ns == 0 { 1 } else { snc as usize }
        } else {
            let (n, _) = as_integer(cx, ncopy.bind(cx));
            if n == i32::MIN || n < 0 {
                return Err(cx.error("invalid 'times' value"));
            }
            n as usize
        };
        if (n as f64) * (ns as f64) > XLEN_MAX {
            return Err(cx.error("invalid 'times' value"));
        }
        (vec![n], n * ns, "rep3")
    };
    if is_null {
        // (allocVector(NILSXP, ...) is R_NilValue, which the copy loop's
        // switch does not take)
        return Err(cx.error(&format!("unimplemented type 'NULL' in '{rep}'\n")));
    }
    cx.build(out, |cx| {
        let sv = s.bind(cx);
        let mut a = Attrs::new();
        if inherits(sv, b"factor") {
            let ordered = inherits(sv, b"ordered");
            let mut tmp = cx.alloc_vector::<Char<'_>>(if ordered { 2 } else { 1 })?;
            if ordered {
                tmp.set(0, cx.mk_char(b"ordered", robigo_rtcore::Encoding::Native)?);
                tmp.set(1, cx.mk_char(b"factor", robigo_rtcore::Encoding::Native)?);
            } else {
                tmp.set(0, cx.mk_char(b"factor", robigo_rtcore::Encoding::Native)?);
            }
            a.set_class(cx.sym(RSym::Class), Some(tmp.finish().into()));
            if let Some(l) = attr::get(sv, b"levels") {
                a.install(l);
            }
        }
        if rep == "rep3" { replicate(cx, sv, len, |k| k % ns, &a) } else { rep4(cx, sv, &times, len, 1, &a) }
    })
}

/// `rep2`'s counts: `times` as integers (doubles truncated), each checked
/// ("invalid 'times' value" for NA, a negative or a too large one), and
/// their sum, the result's length.
// R: seq.c:rep2
fn rep2_counts(cx: &mut Cx, ncopy: Root<'_>) -> RResult<(Vec<usize>, usize)> {
    let r = if ncopy.bind(cx).sexptype() != INTSXP {
        // coerceVector(ncopy, REALSXP), with its warnings
        let t = cx.scope(|cx, s| {
            let t = s.capture(cx, |cx, o| crate::coerce::vector::coerce_vector(cx, ncopy, REALSXP, o))?;
            Ok(t.bind(cx).as_vector::<f64>().and_then(|t| t.as_slice().map(|s| s.to_vec())))
        })?;
        let Some(t) = t else {
            return Err(crate::util::not_yet(cx, None, "rep.int() with a package ALTREP 'times'"));
        };
        let mut times = Vec::with_capacity(t.len());
        let mut sna = 0.0f64;
        for x in t {
            if x.is_nan() || x <= -1.0 || x >= XLEN_MAX + 1.0 {
                return Err(cx.error("invalid 'times' value"));
            }
            times.push(x as usize);
            sna += (x as i64) as f64;
        }
        (times, sna)
    } else {
        let t = ncopy.bind(cx).as_ints().unwrap().to_vec();
        let mut times = Vec::with_capacity(t.len());
        let mut sna = 0.0f64;
        for x in t {
            if x == i32::MIN || x < 0 {
                return Err(cx.error("invalid 'times' value"));
            }
            times.push(x as usize);
            sna += x as f64;
        }
        (times, sna)
    };
    if r.1 > XLEN_MAX {
        return Err(cx.error("invalid 'times' value"));
    }
    Ok((r.0, r.1 as usize))
}

const REP_LEN: Generic = Generic::builtin("rep_len");

/// `.Internal(rep_len(x, length.out))`.
// R: seq.c:do_rep_len
pub fn do_rep_len(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("rep_len", true).check_arity(cx, call, args)?;
    let Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &REP_LEN, call, args, env, out)? else {
        return Ok(());
    };
    let s = args.root(0);
    if s.bind(cx).is_object() && !inherits(s.bind(cx), b"factor") {
        return Err(crate::util::not_yet(cx, Some(call), "rep_len() of an object (dispatch to rep)"));
    }
    let sv = s.bind(cx);
    if !attr::is_vector(sv) && sv.sexptype() != NILSXP {
        return Err(cx.error("attempt to replicate non-vector"));
    }
    let len = args.root(1);
    if crate::coerce::xlength(len.bind(cx)) != 1 {
        return Err(cx.error("invalid 'length.out' value"));
    }
    let na = if len.bind(cx).sexptype() != INTSXP {
        let (d, w) = as_real_w(cx, len.bind(cx));
        crate::coerce::vector::coercion_warning(cx, w)?;
        if d.is_nan() || d <= -1.0 || d >= 4503599627370496.0 + 1.0 {
            return Err(cx.error("invalid 'length.out' value"));
        }
        d as usize
    } else {
        let (n, _) = as_integer(cx, len.bind(cx));
        if n == i32::MIN || n < 0 {
            return Err(cx.error("invalid 'length.out' value"));
        }
        n as usize
    };
    if s.bind(cx).sexptype() == NILSXP && na > 0 {
        return Err(cx.error("cannot replicate NULL to a non-zero length"));
    }
    if s.bind(cx).is_package_altrep() {
        return Err(crate::util::not_yet(cx, Some(call), "rep_len() of a package ALTREP vector"));
    }
    let ns = crate::coerce::xlength(s.bind(cx));
    cx.build_with(out, |cx| {
        let sv = s.bind(cx);
        if ns == 0 {
            return Ok(Built::done(if na > 0 { xlengthgets(cx, sv, na)? } else { sv }));
        }
        let mut a = Attrs::new();
        if inherits(sv, b"factor") {
            let mut tmp = cx.alloc_vector::<Char<'_>>(if inherits(sv, b"ordered") { 2 } else { 1 })?;
            if inherits(sv, b"ordered") {
                tmp.set(0, cx.mk_char(b"ordered", robigo_rtcore::Encoding::Native)?);
                tmp.set(1, cx.mk_char(b"factor", robigo_rtcore::Encoding::Native)?);
            } else {
                tmp.set(0, cx.mk_char(b"factor", robigo_rtcore::Encoding::Native)?);
            }
            a.set_class(cx.sym(RSym::Class), Some(tmp.finish().into()));
            if let Some(l) = attr::get(sv, b"levels") {
                a.install(l);
            }
        }
        Ok(Built::done(replicate(cx, sv, na, |k| k % ns, &a)?))
    })
    .map(|_: ()| ())
}

// ---- seq.int (B5b) ----------------------------------------------------------

const SEQ: Generic = Generic::builtin("seq");

/// do_seq's `FEPS`, "to match seq.default".
// R: seq.c:FEPS
const FEPS: f64 = 1e-10;

/// `length(x)` as do_seq reads it (`Rf_length`, whose vector cases are the
/// `LENGTH` macro): `None` for an environment, whose `envlength` is not
/// ported; a package ALTREP's length is not read; C's "long vectors not
/// supported yet" past INT_MAX has no port here.
// R: Rinlinedfuns.h:length, Rinlinedfuns.h:LENGTH_EX, memory.c:R_BadLongVector
fn length_seq(cx: &mut Cx, call: Root<'_>, x: Root<'_>) -> RResult<usize> {
    let (alt, n) = {
        let v = x.bind(cx);
        (v.is_package_altrep(), length(v))
    };
    if alt {
        return Err(crate::util::not_yet(cx, None, "seq.int() of a package ALTREP argument"));
    }
    let Some(n) = n else {
        return Err(crate::util::not_yet(cx, Some(call), "seq.int() of an environment (envlength)"));
    };
    if n > i32::MAX as usize {
        return Err(crate::util::not_yet(cx, None, "seq.int() of a long vector argument"));
    }
    Ok(n)
}

/// `asReal(x)` with its `CoercionWarning`; a raw vector with an element is
/// `asReal`'s UNIMPLEMENTED_TYPE (a zero-length one is NA, as for every
/// type); a package ALTREP is not read yet.
// R: coerce.c:asReal
fn as_real_seq(cx: &mut Cx, x: Root<'_>) -> RResult<f64> {
    let (alt, raw, r) = {
        let v = x.bind(cx);
        (v.is_package_altrep(), v.sexptype() == RAWSXP && v.len().unwrap_or(0) > 0, as_real_w(cx, v))
    };
    if alt {
        return Err(crate::util::not_yet(cx, None, "seq.int() of a package ALTREP argument"));
    }
    if raw {
        return Err(cx.error("unimplemented type 'raw' in 'asReal'\n"));
    }
    crate::coerce::vector::coercion_warning(cx, r.1)?;
    Ok(r.0)
}

/// `seq.int(from, to, by, length.out, along.with, ...)`: a primitive with
/// internal argument matching, an internal generic (`seq`) for objects,
/// and the one-argument `seq.int(5)` and `along.with` forms.
// R: seq.c:do_seq
pub fn do_seq(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // `One`, counted before anything messes with the arguments
    let one = args.len() == 1;
    // DispatchOrEval(call, op, "seq", args, rho, &ans, 0, 1): a `seq`
    // method for an object
    let Dispatched::Default(out) = crate::coerce::dispatch_or_eval(cx, &SEQ, call, args, env, out)? else {
        return Ok(());
    };
    // a primitive: match seq.int(from, to, by, length.out, along.with, ...)
    // ourselves
    let m = crate::util::match_args(cx, call, &["from", "to", "by", "length.out", "along.with", "..."], args)?;
    cx.scope(|cx, s| seq_int(cx, s, call, one, &m, args, out))
}

/// The body of [`do_seq`] after the dispatch and the matching: each formal
/// rooted, `R_MissingArg` when unmatched or empty.
// R: seq.c:do_seq
fn seq_int(
    cx: &mut Cx,
    s: Scope<'_>,
    call: Root<'_>,
    one: bool,
    m: &crate::util::Matched,
    args: Args<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let absent = s.root(cx, cx.missing_arg());
    let from = m.formals[0].map_or(absent, |i| s.root(cx, args.get(cx, i)));
    let to = m.formals[1].map_or(absent, |i| s.root(cx, args.get(cx, i)));
    let by = m.formals[2].map_or(absent, |i| s.root(cx, args.get(cx, i)));
    let len = m.formals[3].map_or(absent, |i| s.root(cx, args.get(cx, i)));
    let along = m.formals[4].map_or(absent, |i| s.root(cx, args.get(cx, i)));
    let miss_from = from.bind(cx) == cx.missing_arg();
    let miss_to = to.bind(cx) == cx.missing_arg();
    let by_missing = by.bind(cx) == cx.missing_arg();
    let along_given = along.bind(cx) != cx.missing_arg();
    let len_given = len.bind(cx) != cx.missing_arg() && len.bind(cx).sexptype() != NILSXP;

    if one && !miss_from {
        // seq.int(5), seq.int(<vector>), seq.int(NULL), seq.int("2")
        let lf = length_seq(cx, call, from)?;
        if lf == 1 && matches!(from.bind(cx).sexptype(), INTSXP | REALSXP) {
            let rfrom = as_real_seq(cx, from)?;
            if !rfrom.is_finite() {
                return Err(cx.error_call(Some(call), "'from' must be a finite number"));
            }
            return seq_colon(cx, 1.0, rfrom, call, out);
        }
        if lf > 0 {
            return seq_colon(cx, 1.0, lf as f64, call, out);
        }
        return cx.build(out, |cx| Ok(cx.alloc_copy::<i32>(&[])?.finish().into()));
    }

    // C's `R_xlen_t lout = NA_INTEGER`: NA_INTEGER as the absent mark
    const NA_LEN: i64 = i32::MIN as i64;
    let mut lout: i64 = NA_LEN;
    if along_given {
        let (alt, env, xl) = {
            let a = along.bind(cx);
            (a.is_package_altrep(), a.sexptype() == ENVSXP, crate::coerce::xlength(a))
        };
        if alt {
            return Err(crate::util::not_yet(cx, None, "seq.int() of a package ALTREP argument"));
        }
        if env {
            return Err(crate::util::not_yet(cx, Some(call), "seq.int() of an environment (envxlength)"));
        }
        lout = xl as i64;
        if one {
            return if lout != 0 {
                seq_colon(cx, 1.0, lout as f64, call, out)
            } else {
                cx.build(out, |cx| Ok(cx.alloc_copy::<i32>(&[])?.finish().into()))
            };
        }
    } else if len_given {
        let rout = as_real_seq(cx, len)?;
        if !rout.is_finite() {
            return Err(cx.error_call(Some(call), "'length.out' must be a finite number"));
        }
        if rout.is_nan() || rout <= -0.5 {
            return Err(cx.error_call(Some(call), "'length.out' must be a non-negative number"));
        }
        if length_seq(cx, call, len)? != 1 {
            cx.warning_call(Some(call), "first element used of 'length.out' argument")?;
        }
        let rout = rout.ceil();
        if rout >= XLEN_MAX {
            return Err(cx.error_call(Some(call), "result would be too long a vector"));
        }
        lout = c_ftoi(rout);
    }

    if lout == NA_LEN {
        let mut rfrom = 1.0;
        if !miss_from {
            if length_seq(cx, call, from)? != 1 {
                return Err(cx.error_call(Some(call), "'from' must be of length 1"));
            }
            rfrom = as_real_seq(cx, from)?;
            if !rfrom.is_finite() {
                return Err(cx.error_call(Some(call), "'from' must be a finite number"));
            }
        }
        let mut rto = 1.0;
        if !miss_to {
            if length_seq(cx, call, to)? != 1 {
                return Err(cx.error_call(Some(call), "'to' must be of length 1"));
            }
            rto = as_real_seq(cx, to)?;
            if !rto.is_finite() {
                return Err(cx.error_call(Some(call), "'to' must be a finite number"));
            }
        }
        if by_missing {
            return seq_colon(cx, rfrom, rto, call, out);
        }
        // 'by' specified
        if length_seq(cx, call, by)? != 1 {
            return Err(cx.error_call(Some(call), "'by' must be of length 1"));
        }
        let del = rto - rfrom;
        if del == 0.0 && rto == 0.0 {
            // 'to' itself, which is *not* missing in this case (its default
            // is 1.0)
            return cx.build(out, |cx| Ok(to.bind(cx)));
        }
        let rby = as_real_seq(cx, by)?;
        if (rby == 1.0 && del > 0.0) || (rby == -1.0 && del < 0.0) {
            // treat as if missing: an integer sequence
            return seq_colon(cx, rfrom, rto, call, out);
        }
        let finite_del = del.is_finite();
        // overflow in (to - from) when both are finite
        let n = if finite_del { del / rby } else { rto / rby - rfrom / rby };
        if !n.is_finite() {
            if del == 0.0 && rby == 0.0 {
                return cx.build(out, |cx| {
                    if miss_from { Ok(cx.alloc_copy::<f64>(&[rfrom])?.finish().into()) } else { Ok(from.bind(cx)) }
                });
            }
            return Err(cx.error_call(Some(call), "invalid '(to - from)/by'"));
        }
        // inherited from seq.default() but "fudgy" (fmax2 of finite values,
        // which is f64::max)
        if finite_del && del.abs() / f64::max(rto.abs(), rfrom.abs()) < 100.0 * f64::EPSILON {
            return cx.build(out, |cx| {
                if miss_from { Ok(cx.alloc_copy::<f64>(&[rfrom])?.finish().into()) } else { Ok(from.bind(cx)) }
            });
        }
        if n > 100.0 * (i32::MAX as f64) {
            return Err(cx.error_call(Some(call), "'by' argument is much too small"));
        }
        if n < -FEPS {
            return Err(cx.error_call(Some(call), "wrong sign in 'by' argument"));
        }
        // as seq.default also returns integer for from + (0:n)*by
        let all_int_inputs = {
            let (f, t, b) = (from.bind(cx), to.bind(cx), by.bind(cx));
            (miss_from || f.sexptype() == INTSXP) && (miss_to || t.sexptype() == INTSXP) && b.sexptype() == INTSXP
        };
        if all_int_inputs {
            // asInteger would read the element of a package ALTREP: not read
            // here
            let alt = {
                let (f, b) = (from.bind(cx), by.bind(cx));
                f.is_package_altrep() || b.is_package_altrep()
            };
            if alt {
                return Err(crate::util::not_yet(cx, None, "seq.int() of a package ALTREP argument"));
            }
            let (ifrom, iby) = {
                let (f, b) = (from.bind(cx), by.bind(cx));
                (if miss_from { rfrom as i32 } else { as_integer(cx, f).0 }, as_integer(cx, b).0)
            };
            let nn = c_ftoi(n);
            if nn < 0 {
                // allocVector of the length C's conversion gave
                return Err(cx.error("negative length vectors are not allowed"));
            }
            let nn = nn as usize;
            return cx.build(out, |cx| {
                // `(int)(ifrom + i * iby)`: `i` is an R_xlen_t, so the
                // arithmetic is i64 and the int cast truncates
                Ok(cx.alloc_with::<i32>(nn + 1, |i| (ifrom as i64 + i as i64 * iby as i64) as i32)?.finish().into())
            });
        }
        let nn = c_ftoi32(n + FEPS) as i64;
        if nn < 0 {
            // allocVector(REALSXP, nn + 1) of the negative length C's
            // `(int)` conversion gave for n past INT_MAX
            return Err(cx.error("negative length vectors are not allowed"));
        }
        let nn = nn as usize;
        // C divides its copies before the loop when the difference is
        // infinite; the overshoot test then reads the divided `rby` (its
        // sign is unchanged)
        let (mut rfrom, mut rby) = (rfrom, rby);
        if !finite_del {
            // |from - to| is infinite, but n = (from-to)/by is not
            rfrom /= 4.0;
            rby /= 4.0;
        }
        return cx.build(out, |cx| {
            let mut a = cx.alloc_with::<f64>(nn + 1, |i| {
                let y = rfrom + i as f64 * rby;
                if finite_del { y } else { y * 4.0 } // ldexp(y, 2) := y * 2^2
            })?;
            // correct for possible overshot because of fuzz
            if nn > 0 {
                let last = &mut a.as_mut_slice()[nn];
                if (rby > 0.0 && *last > rto) || (rby < 0.0 && *last < rto) {
                    *last = rto;
                }
            }
            Ok(a.finish().into())
        });
    }
    if lout == 0 {
        cx.build(out, |cx| Ok(cx.alloc_copy::<i32>(&[])?.finish().into()))
    } else if one {
        seq_colon(cx, 1.0, lout as f64, call, out)
    } else if by_missing {
        // 'length.out' elements from 'from' to 'to'; either may be missing
        let mut rfrom = as_real_seq(cx, from)?;
        let mut rto = as_real_seq(cx, to)?;
        let mut rby = 0.0;
        if miss_to {
            rto = rfrom + lout as f64 - 1.0;
        }
        if miss_from {
            rfrom = rto - lout as f64 + 1.0;
        }
        if !rfrom.is_finite() {
            return Err(cx.error_call(Some(call), "'from' must be a finite number"));
        }
        if !rto.is_finite() {
            return Err(cx.error_call(Some(call), "'to' must be a finite number"));
        }
        let mut finite_del = false;
        if lout > 2 {
            // only then, use 'by'
            let nint = (lout - 1) as f64;
            rby = rto - rfrom;
            if rby.is_finite() {
                finite_del = true;
                rby /= nint;
            } else {
                // overflow in (to - from), nint >= 2 => finite 'by'
                rby = rto / nint - rfrom / nint;
            }
        }
        let all_int = rfrom <= i32::MAX as f64
            && rfrom >= i32::MIN as f64
            && rto <= i32::MAX as f64
            && rto >= i32::MIN as f64
            && rfrom == c_ftoi32(rfrom) as f64
            && (lout <= 1 || rto == c_ftoi32(rto) as f64)
            && (lout <= 2 || rby == c_ftoi32(rby) as f64);
        if all_int {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx
                    .alloc_with::<i32>(n, |i| {
                        if i == 0 {
                            rfrom as i32
                        } else if i == n - 1 {
                            rto as i32
                        } else {
                            (rfrom + i as f64 * rby) as i32
                        }
                    })?
                    .finish()
                    .into())
            })
        } else {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx
                    .alloc_with::<f64>(n, |i| {
                        if i == 0 {
                            rfrom
                        } else if i == n - 1 {
                            rto
                        } else if finite_del {
                            rfrom + i as f64 * rby
                        } else {
                            // |del| = from - to is infinite: ldexp(rfrom +
                            // i*rby, 2) after both quarters (* 2^2 is exact)
                            (rfrom / 4.0 + i as f64 * (rby / 4.0)) * 4.0
                        }
                    })?
                    .finish()
                    .into())
            })
        }
    } else if miss_to {
        // 'length.out' and 'by', no 'to'
        let mut rfrom = as_real_seq(cx, from)?;
        let rby = as_real_seq(cx, by)?;
        if miss_from {
            rfrom = 1.0;
        }
        if !rfrom.is_finite() {
            return Err(cx.error_call(Some(call), "'from' must be a finite number"));
        }
        if !rby.is_finite() {
            return Err(cx.error_call(Some(call), "'by' must be a finite number"));
        }
        let rto = rfrom + (lout - 1) as f64 * rby;
        let all_int = rfrom <= i32::MAX as f64
            && rfrom >= i32::MIN as f64
            && rto <= i32::MAX as f64
            && rto >= i32::MIN as f64
            && rby == c_ftoi32(rby) as f64
            && rfrom == c_ftoi32(rfrom) as f64;
        if all_int {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx.alloc_with::<i32>(n, |i| (rfrom + i as f64 * rby) as i32)?.finish().into())
            })
        } else {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx.alloc_with::<f64>(n, |i| rfrom + i as f64 * rby)?.finish().into())
            })
        }
    } else if miss_from {
        // 'length.out' and 'by', no 'from'
        let rto = as_real_seq(cx, to)?;
        let rby = as_real_seq(cx, by)?;
        let rfrom = rto - (lout - 1) as f64 * rby;
        if !rto.is_finite() {
            return Err(cx.error_call(Some(call), "'to' must be a finite number"));
        }
        if !rby.is_finite() {
            return Err(cx.error_call(Some(call), "'by' must be a finite number"));
        }
        let all_int = rby == c_ftoi32(rby) as f64
            && rto == c_ftoi32(rto) as f64
            && rfrom <= i32::MAX as f64
            && rfrom >= i32::MIN as f64
            && rto <= i32::MAX as f64
            && rto >= i32::MIN as f64;
        if all_int {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx.alloc_with::<i32>(n, |i| (rto - (n - 1 - i) as f64 * rby) as i32)?.finish().into())
            })
        } else {
            cx.build(out, |cx| {
                let n = lout as usize;
                Ok(cx.alloc_with::<f64>(n, |i| rto - (n - 1 - i) as f64 * rby)?.finish().into())
            })
        }
    } else {
        Err(cx.error_call(Some(call), "too many arguments"))
    }
}

/// `seq_len(length.out)`: `1:n`, a compact sequence.
// R: seq.c:do_seq_len
pub fn do_seq_len(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("seq_len").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "length.out")?;
    if crate::coerce::xlength(args.get(cx, 0)) != 1 {
        cx.warning_call(Some(call), "first element used of 'length.out' argument")?;
    }
    let (d, w) = as_real_w(cx, args.get(cx, 0));
    crate::coerce::vector::coercion_warning(cx, w)?;
    if !d.is_finite() || d < 0.0 {
        return Err(cx.error_call(Some(call), "argument must be coercible to non-negative integer"));
    }
    if d >= 4503599627370496.0 {
        return Err(cx.error_call(Some(call), "result would be too long a vector"));
    }
    seq_1_to(cx, call, d as usize, out)
}

/// `1:n` as GNU R's `R_compact_intrange(1, n)` makes it (a compact
/// sequence, double past the integer range), or `integer(0)`.
fn seq_1_to(cx: &mut Cx, _call: Root<'_>, n: usize, out: Out<'_>) -> RResult<()> {
    if n == 0 {
        return cx.build(out, |cx| Ok(cx.alloc_copy::<i32>(&[])?.finish().into()));
    }
    cx.compact_intrange(1, n as i64, out)
}

const LENGTH: Generic = Generic::builtin("length");

/// `seq_along(along.with)`: `1:length(along.with)`, the length from a
/// `length` method for an object.
// R: seq.c:do_seq_along
pub fn do_seq_along(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("seq_along").check_arity(cx, call, args)?;
    crate::util::check1arg(cx, args, call, "along.with")?;
    let len = if args.get(cx, 0).is_object() {
        let r = cx.scope(|cx, s| {
            let o = s.out(cx);
            let res = o.root();
            match cx.try_dispatch(&LENGTH, call, args, env, o)? {
                Dispatched::Done => {
                    let (n, w) = as_integer(cx, res.bind(cx));
                    crate::coerce::vector::coercion_warning(cx, w)?;
                    Ok(Some(n))
                }
                Dispatched::Default(_) => Ok(None),
            }
        })?;
        match r {
            Some(n) => n.max(0) as usize,
            None => crate::builtin::length_mut(cx, args.root(0))?.unwrap_or(0),
        }
    } else {
        match crate::builtin::length_mut(cx, args.root(0))? {
            Some(n) => n,
            None => return Err(crate::util::not_yet(cx, Some(call), "seq_along() of this environment")),
        }
    };
    seq_1_to(cx, call, len, out)
}
