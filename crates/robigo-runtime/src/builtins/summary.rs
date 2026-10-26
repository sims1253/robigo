//! Summaries and cumulative operations (GNU R's summary.c, cum.c, the
//! `all`/`any` part of logic.c and `anyNA` from coerce.c).

pub mod ldouble;

use std::rc::Rc;

use self::ldouble::F80;
use super::{internal, prim};
use crate::attrib::get_attrib;
use crate::coerce::int_to_str;
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::format::string_from_real;
use crate::interp::Interp;
use crate::sym::{s, Sym};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("sum", do_sum, -1),
    prim("prod", do_prod, -1),
    prim("min", do_min, -1),
    prim("max", do_max, -1),
    prim("range", do_range, -1),
    prim("all", do_all, -1),
    prim("any", do_any, -1),
    internal("mean", do_mean, 1),
    internal("pmin", do_pmin, -1),
    internal("pmax", do_pmax, -1),
    internal("which", do_which, 1),
    internal("which.min", do_which_min, 1),
    internal("which.max", do_which_max, 1),
    prim("cumsum", do_cumsum, 1),
    prim("cumprod", do_cumprod, 1),
    prim("cummax", do_cummax, 1),
    prim("cummin", do_cummin, 1),
    prim("anyNA", do_any_na, -1),
];

// ----------------------------------------------------------------------
// shared helpers

/// R's SEXPTYPE codes, for the places where C code compares types
/// numerically.
pub fn sexptype(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Sym(_) => 1,
        Value::Pairlist(_) => 2,
        Value::Closure(_) => 3,
        Value::Env(_) => 4,
        Value::Promise(_) => 5,
        Value::Lang(_) => 6,
        Value::Builtin(b) => {
            if b.kind == BuiltinKind::Special {
                7
            } else {
                8
            }
        }
        Value::Lgl(_) => 10,
        Value::Int(_) => 13,
        Value::Real(_) => 14,
        Value::Cplx(_) => 15,
        Value::Str(_) => 16,
        Value::Dots(_) => 17,
        Value::List(_) => 19,
        Value::Expr(_) => 20,
        Value::ExtPtr(_) => 22,
        Value::Raw(_) => 24,
    }
}

/// `R_typeToChar`.
pub fn type_to_char(v: &Value) -> &'static str {
    v.type_name()
}

/// R's `asLogical2(x, checking, call)`.
pub fn as_logical2(v: &Value, checking: bool, call: &Value) -> RResult<Lgl> {
    if v.is_atomic() {
        if v.is_empty() {
            return Ok(NA_LOGICAL);
        }
        if checking && v.len() > 1 {
            return Err(Flow::error_call(
                call.clone(),
                format!("'length = {}' in coercion to 'logical(1)'", v.len()),
            ));
        }
        return Ok(crate::coerce::as_logical(v));
    }
    Ok(NA_LOGICAL)
}

/// R's `asBool2(x, call)`.
pub fn as_bool2(v: &Value, call: &Value) -> RResult<bool> {
    let l = as_logical2(v, true, call)?;
    if l == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    Ok(l != 0)
}

/// Double addition with SSE NaN propagation (the first NaN operand wins),
/// as GNU R's compiled C code behaves on x86_64.
#[inline]
pub fn add_sse(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        quiet(a)
    } else if b.is_nan() {
        quiet(b)
    } else {
        a + b
    }
}

#[inline]
pub fn mul_sse(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        quiet(a)
    } else if b.is_nan() {
        quiet(b)
    } else {
        a * b
    }
}

#[inline]
fn quiet(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | (1u64 << 51))
}

fn int2real(i: i32) -> f64 {
    if i == NA_INTEGER {
        NA_REAL
    } else {
        i as f64
    }
}

fn int_slice(v: &Value) -> &[i32] {
    match v {
        Value::Int(x) | Value::Lgl(x) => &x.data,
        _ => &[],
    }
}

/// `fixup_NaRm`: make sure `na.rm` is present (default FALSE) and last.
pub fn fixup_na_rm(mut args: Vec<Arg>) -> RResult<Vec<Arg>> {
    let mut na_value = Value::lgl(false);
    let mut seen = false;
    let mut i = 0;
    while i < args.len() {
        if args[i].tag == Some(s::NA_RM) {
            if seen {
                return Err(Flow::error("formal argument \"na.rm\" matched by multiple actual arguments"));
            }
            seen = true;
            if i == args.len() - 1 {
                return Ok(args);
            }
            na_value = args.remove(i).value;
            continue;
        }
        i += 1;
    }
    args.push(Arg::new(Some(s::NA_RM), na_value));
    Ok(args)
}

/// Group dispatch for the Summary group. As in R, the call seen by the
/// method has the evaluated arguments (with `na.rm`) as its arguments.
fn summary_dispatch(i: &mut Interp, name: &str, call: &Value, args: &[Arg], env: &Env) -> RResult<Option<Value>> {
    if !args.first().is_some_and(|a| a.value.is_object()) {
        return Ok(None);
    }
    let fun = match call {
        Value::Lang(l) => l.fun.clone(),
        _ => Value::sym(name),
    };
    let call2 = Value::lang(fun, args.to_vec());
    i.try_dispatch_group("Summary", name, &call2, args, env)
}

/// Remove the trailing `na.rm` argument added by `fixup_na_rm`.
fn take_na_rm(args: &mut Vec<Arg>) -> Value {
    match args.iter().position(|a| a.tag == Some(s::NA_RM)) {
        Some(p) => args.remove(p).value,
        None => Value::lgl(false),
    }
}

// ----------------------------------------------------------------------
// sum, prod, min, max (do_summary)

#[derive(Clone, Copy, PartialEq, Eq)]
enum SumOp {
    Sum,
    Min,
    Max,
    Prod,
}

fn do_sum(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_summary(i, call, args, env, SumOp::Sum)
}
fn do_prod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_summary(i, call, args, env, SumOp::Prod)
}
fn do_min(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_summary(i, call, args, env, SumOp::Min)
}
fn do_max(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_summary(i, call, args, env, SumOp::Max)
}

/// `isum`: integer sum in 64 bits. `Err(())` signals an NA result.
fn isum(x: &[i32], narm: bool) -> Result<Option<i64>, ()> {
    let mut s: i64 = 0;
    let mut updated = false;
    for &v in x {
        if v != NA_INTEGER {
            updated = true;
            s += v as i64;
        } else if !narm {
            return Err(());
        }
    }
    Ok(if updated { Some(s) } else { None })
}

fn rsum(x: &[f64], narm: bool) -> Option<f64> {
    let (s, updated) = if !narm {
        (F80::ZERO.sum_slice(x), !x.is_empty())
    } else {
        // Sum the runs between NaNs, continuing the same accumulator.
        let mut s = F80::ZERO;
        let mut updated = false;
        for run in x.split(|v| v.is_nan()) {
            if !run.is_empty() {
                updated = true;
                s = s.sum_slice(run);
            }
        }
        (s, updated)
    };
    if updated {
        Some(s.to_f64_clamped())
    } else {
        None
    }
}

fn csum(x: &[Complex], narm: bool) -> Option<Complex> {
    let (mut sr, mut si) = (F80::ZERO, F80::ZERO);
    let mut updated = false;
    for z in x {
        if !narm || (!z.re.is_nan() && !z.im.is_nan()) {
            updated = true;
            sr = sr.add(F80::from_f64(z.re));
            si = si.add(F80::from_f64(z.im));
        }
    }
    if updated {
        Some(Complex::new(sr.to_f64(), si.to_f64()))
    } else {
        None
    }
}

/// imin/imax: `Some(NA)` if an NA is seen with !narm.
fn iminmax(x: &[i32], narm: bool, max: bool) -> Option<i32> {
    let mut s = 0;
    let mut updated = false;
    for &v in x {
        if v != NA_INTEGER {
            if !updated || (max && s < v) || (!max && s > v) {
                s = v;
                updated = true;
            }
        } else if !narm {
            return Some(NA_INTEGER);
        }
    }
    if updated {
        Some(s)
    } else {
        None
    }
}

fn rminmax(x: &[f64], narm: bool, max: bool) -> Option<f64> {
    let mut s = 0.0;
    let mut updated = false;
    for &v in x {
        if v.is_nan() {
            if !narm {
                if !is_na_real(s) {
                    s = v;
                }
                updated = true;
            }
        } else if !updated || (max && v > s) || (!max && v < s) {
            s = v;
            updated = true;
        }
    }
    if updated {
        Some(s)
    } else {
        None
    }
}

/// Byte-wise string comparison (the C locale's strcoll).
pub fn scollate(a: &str, b: &str) -> std::cmp::Ordering {
    a.as_bytes().cmp(b.as_bytes())
}

fn sminmax(x: &[RStr], narm: bool, max: bool) -> Option<RStr> {
    let mut s: RStr = None;
    let mut updated = false;
    for v in x {
        match v {
            Some(sv) => {
                let better = match &s {
                    _ if !updated => true,
                    Some(cur) => {
                        let o = scollate(cur, sv);
                        if max {
                            o == std::cmp::Ordering::Less
                        } else {
                            o == std::cmp::Ordering::Greater
                        }
                    }
                    None => false,
                };
                if better {
                    s = Some(sv.clone());
                    updated = true;
                }
            }
            None => {
                if !narm {
                    return Some(None);
                }
            }
        }
    }
    if updated {
        Some(s)
    } else {
        None
    }
}

fn iprod(x: &[i32], narm: bool) -> Option<f64> {
    let mut s = F80::ONE;
    let mut updated = false;
    for &v in x {
        if v != NA_INTEGER {
            s = s.mul(F80::from_i64(v as i64));
            updated = true;
        } else if !narm {
            return Some(NA_REAL);
        }
        if s.is_nan() {
            return Some(NA_REAL);
        }
    }
    if updated {
        Some(s.to_f64_clamped())
    } else {
        None
    }
}

fn rprod(x: &[f64], narm: bool) -> Option<f64> {
    let (s, updated) = if !narm {
        (F80::ONE.prod_slice(x), !x.is_empty())
    } else {
        let mut s = F80::ONE;
        let mut updated = false;
        for run in x.split(|v| v.is_nan()) {
            if !run.is_empty() {
                updated = true;
                s = s.prod_slice(run);
            }
        }
        (s, updated)
    };
    if updated {
        Some(s.to_f64_clamped())
    } else {
        None
    }
}

fn cprod(x: &[Complex], narm: bool) -> Option<Complex> {
    let (mut sr, mut si) = (F80::ONE, F80::ZERO);
    let mut updated = false;
    for z in x {
        if !narm || (!z.re.is_nan() && !z.im.is_nan()) {
            updated = true;
            let (tr, ti) = (sr, si);
            let (xr, xi) = (F80::from_f64(z.re), F80::from_f64(z.im));
            sr = tr.mul(xr).sub(ti.mul(xi));
            si = tr.mul(xi).add(ti.mul(xr));
        }
    }
    if updated {
        Some(Complex::new(sr.to_f64(), si.to_f64()))
    } else {
        None
    }
}

fn op_name(op: SumOp) -> &'static str {
    match op {
        SumOp::Sum => "sum",
        SumOp::Min => "min",
        SumOp::Max => "max",
        SumOp::Prod => "prod",
    }
}

// R: summary.c:do_summary
fn do_summary(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, op: SumOp) -> RResult<Value> {
    let mut args = fixup_na_rm(args)?;
    if let Some(r) = summary_dispatch(i, op_name(op), call, &args, env)? {
        return Ok(r);
    }
    let narm_v = take_na_rm(&mut args);
    let narm = as_bool2(&narm_v, call)?;

    let invalid = |a: &Value| -> Flow {
        Flow::error_call(call.clone(), format!("invalid 'type' ({}) of argument", type_to_char(a)))
    };

    let mut empty = true;
    let mut zcum = Complex::new(0.0, 0.0);
    let mut icum: i32 = 0;
    let mut il_cum: i64 = 0;
    let mut scum: RStr = None;
    let mut ans_type: VType;
    match op {
        SumOp::Sum => {
            let mut complex_a = false;
            let mut real_a = false;
            for a in &args {
                match &a.value {
                    Value::Int(_) | Value::Lgl(_) | Value::Null => {}
                    Value::Real(_) => real_a = true,
                    Value::Cplx(_) => complex_a = true,
                    other => return Err(invalid(other)),
                }
            }
            ans_type = if complex_a {
                VType::Cplx
            } else if real_a {
                VType::Real
            } else {
                VType::Int
            };
        }
        SumOp::Min => {
            ans_type = VType::Int;
            zcum.re = f64::INFINITY;
            icum = i32::MAX;
        }
        SumOp::Max => {
            ans_type = VType::Int;
            zcum.re = f64::NEG_INFINITY;
            icum = NA_INTEGER + 1;
        }
        SumOp::Prod => {
            ans_type = VType::Real;
            zcum.re = 1.0;
            zcum.im = 0.0;
        }
    }
    let is_minmax = matches!(op, SumOp::Min | SumOp::Max);
    let max = op == SumOp::Max;

    for arg in &args {
        let a = &arg.value;
        let mut int_a = false;
        let mut real_a = false;
        let mut updated = false;
        if !a.is_empty() && !matches!(a, Value::Null) {
            match op {
                SumOp::Min | SumOp::Max => {
                    let mut itmp = 0;
                    let mut tmp = 0.0;
                    let mut stmp: RStr = None;
                    match a {
                        Value::Lgl(_) | Value::Int(_) => {
                            int_a = true;
                            if let Some(v) = iminmax(int_slice(a), narm, max) {
                                itmp = v;
                                updated = true;
                            }
                        }
                        Value::Real(x) => {
                            real_a = true;
                            if ans_type == VType::Int {
                                ans_type = VType::Real;
                                if !empty {
                                    zcum.re = int2real(icum);
                                }
                            }
                            if let Some(v) = rminmax(&x.data, narm, max) {
                                tmp = v;
                                updated = true;
                            }
                        }
                        Value::Str(x) => {
                            if !empty && ans_type == VType::Int {
                                scum = int_to_str(icum);
                            } else if !empty && ans_type == VType::Real {
                                scum = string_from_real(zcum.re);
                            }
                            ans_type = VType::Str;
                            if let Some(v) = sminmax(&x.data, narm, max) {
                                stmp = v;
                                updated = true;
                            }
                        }
                        other => return Err(invalid(other)),
                    }
                    if updated {
                        match ans_type {
                            VType::Int => {
                                if icum == NA_INTEGER {
                                } else if itmp == NA_INTEGER || (!max && itmp < icum) || (max && itmp > icum) {
                                    icum = itmp;
                                }
                            }
                            VType::Real => {
                                if int_a {
                                    tmp = int2real(itmp);
                                }
                                if is_na_real(zcum.re) {
                                } else if tmp.is_nan() {
                                    if is_na_real(tmp) {
                                        zcum.re = tmp;
                                    } else {
                                        zcum.re = add_sse(zcum.re, tmp);
                                    }
                                } else if (!max && tmp < zcum.re) || (max && tmp > zcum.re) {
                                    zcum.re = tmp;
                                }
                            }
                            VType::Str => {
                                if int_a {
                                    stmp = int_to_str(itmp);
                                } else if real_a {
                                    stmp = string_from_real(tmp);
                                }
                                if empty {
                                    scum = stmp;
                                } else if let Some(cur) = &scum {
                                    let take = match &stmp {
                                        None => true,
                                        Some(st) => {
                                            st != cur && {
                                                let o = scollate(st, cur);
                                                if max {
                                                    o == std::cmp::Ordering::Greater
                                                } else {
                                                    o == std::cmp::Ordering::Less
                                                }
                                            }
                                        }
                                    };
                                    if take {
                                        scum = stmp;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                SumOp::Sum => match a {
                    Value::Lgl(_) | Value::Int(_) => match isum(int_slice(a), narm) {
                        Err(()) => {
                            return Ok(na_answer(ans_type));
                        }
                        Ok(Some(il_tmp)) => {
                            updated = true;
                            if ans_type == VType::Int {
                                let sd = il_cum as f64 + il_tmp as f64;
                                if sd > i32::MAX as f64 || sd < (NA_INTEGER + 1) as f64 {
                                    ans_type = VType::Real;
                                    zcum.re = sd;
                                } else {
                                    il_cum += il_tmp;
                                }
                            } else {
                                zcum.re = add_sse(zcum.re, il_tmp as f64);
                            }
                        }
                        Ok(None) => {}
                    },
                    Value::Real(x) => {
                        if ans_type == VType::Int {
                            ans_type = VType::Real;
                            if !empty {
                                zcum.re = il_cum as f64;
                            }
                        }
                        if let Some(t) = rsum(&x.data, narm) {
                            updated = true;
                            zcum.re = add_sse(zcum.re, t);
                        }
                    }
                    Value::Cplx(x) => {
                        if ans_type == VType::Int {
                            ans_type = VType::Cplx;
                            if !empty {
                                zcum.re = il_cum as f64;
                            }
                        } else if ans_type == VType::Real {
                            ans_type = VType::Cplx;
                        }
                        if let Some(z) = csum(&x.data, narm) {
                            updated = true;
                            zcum.re = add_sse(zcum.re, z.re);
                            zcum.im = add_sse(zcum.im, z.im);
                        }
                    }
                    other => return Err(invalid(other)),
                },
                SumOp::Prod => match a {
                    Value::Lgl(_) | Value::Int(_) | Value::Real(_) => {
                        let t = match a {
                            Value::Real(x) => rprod(&x.data, narm),
                            _ => iprod(int_slice(a), narm),
                        };
                        if let Some(t) = t {
                            updated = true;
                            zcum.re = mul_sse(zcum.re, t);
                            zcum.im = mul_sse(zcum.im, t);
                        }
                    }
                    Value::Cplx(x) => {
                        ans_type = VType::Cplx;
                        if let Some(z) = cprod(&x.data, narm) {
                            updated = true;
                            let (zr, zi) = (zcum.re, zcum.im);
                            zcum.re = zr * z.re - zi * z.im;
                            zcum.im = zr * z.im + zi * z.re;
                        }
                    }
                    other => return Err(invalid(other)),
                },
            }
        } else {
            // zero-length argument: may still be invalid
            match a {
                Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Null => {}
                Value::Cplx(_) => {
                    if is_minmax {
                        return Err(invalid(a));
                    }
                }
                Value::Str(_) if is_minmax => {
                    if !empty && ans_type == VType::Int {
                        scum = int_to_str(icum);
                    } else if !empty && ans_type == VType::Real {
                        scum = string_from_real(zcum.re);
                    }
                    ans_type = VType::Str;
                }
                other => return Err(invalid(other)),
            }
            let ans_code = vtype_code(ans_type);
            if ans_code < sexptype(a) && ans_type != VType::Cplx {
                if !empty && ans_type == VType::Int {
                    zcum.re = int2real(icum);
                }
                ans_type = a.vtype().unwrap_or(ans_type);
            }
        }
        if empty && updated {
            empty = false;
        }
    }

    if empty && is_minmax {
        if ans_type == VType::Str {
            i.warningcall(call, "no non-missing arguments, returning NA")?;
        } else {
            if op == SumOp::Min {
                i.warningcall(call, "no non-missing arguments to min; returning Inf")?;
            } else {
                i.warningcall(call, "no non-missing arguments to max; returning -Inf")?;
            }
            ans_type = VType::Real;
        }
    }
    Ok(match ans_type {
        VType::Int => Value::int(if op == SumOp::Sum { il_cum as i32 } else { icum }),
        VType::Real => Value::real(zcum.re),
        VType::Cplx => Value::cplx(zcum),
        VType::Str => Value::strs(vec![scum]),
        // Only reachable via zero-length logical args, which never
        // change the answer type.
        _ => Value::int(if op == SumOp::Sum { il_cum as i32 } else { icum }),
    })
}

fn vtype_code(t: VType) -> u8 {
    match t {
        VType::Null => 0,
        VType::Lgl => 10,
        VType::Int => 13,
        VType::Real => 14,
        VType::Cplx => 15,
        VType::Str => 16,
        VType::List => 19,
        VType::Expr => 20,
        VType::Raw => 24,
    }
}

fn na_answer(t: VType) -> Value {
    match t {
        VType::Int => Value::int(NA_INTEGER),
        VType::Real => Value::real(NA_REAL),
        VType::Cplx => Value::cplx(Complex::NA),
        VType::Str => Value::str_na(),
        _ => Value::int(NA_INTEGER),
    }
}

// ----------------------------------------------------------------------
// range

// R: summary.c:do_range
fn do_range(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let args = fixup_na_rm(args)?;
    if let Some(r) = summary_dispatch(i, "range", call, &args, env)? {
        return Ok(r);
    }
    let f = i.find_fun(Sym::new("range.default"), env)?;
    let pargs = i.forced_promargs(&Value::Null, &args, env);
    match &f {
        Value::Closure(c) => i.apply_closure(call, c, pargs, env, None),
        _ => i.apply_function(&f, pargs, env, call),
    }
}

// ----------------------------------------------------------------------
// all, any (logic.c:do_logic3)

fn do_all(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_logic3(i, call, args, env, true)
}
fn do_any(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_logic3(i, call, args, env, false)
}

fn do_logic3(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, all: bool) -> RResult<Value> {
    let mut args = fixup_na_rm(args)?;
    if let Some(r) = summary_dispatch(i, if all { "all" } else { "any" }, call, &args, env)? {
        return Ok(r);
    }
    let narm_v = take_na_rm(&mut args);
    let narm = as_logical2(&narm_v, true, call)?;
    let mut val = if all { TRUE } else { FALSE };
    let mut has_na = false;
    for a in &args {
        let t = &a.value;
        if t.is_empty() {
            continue;
        }
        let lv: Vec<Lgl> = match t {
            Value::Lgl(x) => x.data.clone(),
            _ => {
                if !matches!(t, Value::Int(_)) {
                    i.warningcall(call, format!("coercing argument of type '{}' to logical", type_to_char(t)))?;
                }
                match t {
                    Value::Int(_) | Value::Real(_) | Value::Cplx(_) | Value::Str(_) | Value::Raw(_) | Value::List(_) => {
                        match i.coerce(t, VType::Lgl)? {
                            Value::Lgl(x) => x.data.clone(),
                            _ => vec![],
                        }
                    }
                    other => {
                        return Err(Flow::error(format!(
                            "cannot coerce type '{}' to vector of type 'logical'",
                            type_to_char(other)
                        )))
                    }
                }
            }
        };
        val = check_values(all, narm, &lv);
        if val != NA_LOGICAL {
            if (!all && val != 0) || (all && val == 0) {
                has_na = false;
                break;
            }
        } else {
            has_na = true;
        }
    }
    Ok(Value::lgl_raw(if has_na { NA_LOGICAL } else { val }))
}

fn check_values(all: bool, na_rm: Lgl, x: &[Lgl]) -> Lgl {
    let mut has_na = false;
    for &xi in x {
        if na_rm == 0 && xi == NA_LOGICAL {
            has_na = true;
        } else {
            if xi == TRUE && !all {
                return TRUE;
            }
            if xi == FALSE && all {
                return FALSE;
            }
        }
    }
    if has_na {
        NA_LOGICAL
    } else if all {
        TRUE
    } else {
        FALSE
    }
}

// ----------------------------------------------------------------------
// mean

// R: summary.c:do_summary (mean branch)
fn do_mean(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "mean", call)?;
    let x = &args[0].value;
    Ok(match x {
        Value::Lgl(v) | Value::Int(v) => {
            let n = v.data.len();
            let mut s = F80::ZERO;
            for &xi in &v.data {
                if xi == NA_INTEGER {
                    return Ok(Value::real(NA_REAL));
                }
                s = s.add(F80::from_i64(xi as i64));
            }
            Value::real(s.div(F80::from_i64(n as i64)).to_f64())
        }
        Value::Real(v) => Value::real(real_mean(&v.data)),
        Value::Cplx(v) => {
            let n = F80::from_i64(v.data.len() as i64);
            let (mut s, mut si) = (F80::ZERO, F80::ZERO);
            for z in &v.data {
                s = s.add(F80::from_f64(z.re));
                si = si.add(F80::from_f64(z.im));
            }
            s = s.div(n);
            si = si.div(n);
            if s.to_f64().is_finite() && si.to_f64().is_finite() {
                let (mut t, mut ti) = (F80::ZERO, F80::ZERO);
                for z in &v.data {
                    t = t.add(F80::from_f64(z.re).sub(s));
                    ti = ti.add(F80::from_f64(z.im).sub(si));
                }
                s = s.add(t.div(n));
                si = si.add(ti.div(n));
            }
            Value::cplx(Complex::new(s.to_f64(), si.to_f64()))
        }
        other => return Err(Flow::error(format!("invalid 'type' ({}) of argument", type_to_char(other)))),
    })
}

// R: summary.c:real_mean
fn real_mean(x: &[f64]) -> f64 {
    let n = F80::from_i64(x.len() as i64);
    let mut s = F80::ZERO.sum_slice(x);
    let finite_s = s.to_f64().is_finite();
    if finite_s {
        s = s.div(n);
    } else {
        // `s += dx[k]/n`: the division is in double precision.
        let nd = x.len() as f64;
        s = F80::ZERO;
        for &v in x {
            let q = if v.is_nan() { f64::from_bits(v.to_bits() | (1u64 << 51)) } else { v / nd };
            s = s.add(F80::from_f64(q));
        }
    }
    if finite_s && s.to_f64().is_finite() {
        let t = s.sum_dev(x);
        s = s.add(t.div(n));
    } else if s.to_f64().is_finite() {
        let mut t = F80::ZERO;
        for &v in x {
            t = t.add(F80::from_f64(v).sub(s).div(n));
        }
        s = s.add(t);
    }
    s.to_f64()
}

/// checkArity for `.Internal`s.
pub fn check_arity_internal(args: &[Arg], n: usize, name: &str, _call: &Value) -> RResult<()> {
    if args.len() != n {
        return Err(Flow::error(format!(
            "{} argument{} passed to .Internal({}) which requires {}",
            args.len(),
            if args.len() == 1 { "" } else { "s" },
            name,
            n
        )));
    }
    Ok(())
}

// ----------------------------------------------------------------------
// pmin, pmax

fn do_pmin(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_pminmax(i, call, args, env, false)
}
fn do_pmax(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_pminmax(i, call, args, env, true)
}

// R: summary.c:do_pmin
fn do_pminmax(i: &mut Interp, _call: &Value, args: Vec<Arg>, _env: &Env, max: bool) -> RResult<Value> {
    let narm = crate::coerce::as_logical(&args.first().map(|a| a.value.clone()).unwrap_or(Value::Null));
    if narm == NA_LOGICAL {
        return Err(Flow::error("invalid 'na.rm' value"));
    }
    let narm = narm != 0;
    let xs: Vec<&Value> = args[1..].iter().map(|a| &a.value).collect();
    if xs.is_empty() {
        return Err(Flow::error("no arguments"));
    }
    let ok_type = |v: &Value| matches!(v, Value::Null | Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Str(_));
    let x0 = xs[0];
    if !ok_type(x0) {
        return Err(Flow::error("invalid input type"));
    }
    let mut anstype = x0.vtype().unwrap();
    if xs.len() == 1 {
        return Ok(x0.clone());
    }
    let mut len = x0.len();
    for x in &xs[1..] {
        if !ok_type(x) {
            return Err(Flow::error("invalid input type"));
        }
        let t = x.vtype().unwrap();
        if t > anstype {
            anstype = t;
        }
        let n = x.len();
        if (len > 0) ^ (n > 0) {
            len = 0;
            break;
        }
        len = len.max(n);
    }
    if anstype < VType::Int {
        anstype = VType::Int;
    }
    if len == 0 {
        return Ok(Value::empty_of(anstype));
    }
    for x in &xs {
        let n = x.len();
        if len % n != 0 {
            i.warning("an argument will be fractionally recycled")?;
            break;
        }
    }
    let quiet = |v: &Value| crate::coerce::coerce_quiet(v, anstype);
    Ok(match anstype {
        VType::Int => {
            let first = quiet(xs[0]);
            let r = int_slice(&first);
            let mut ra: Vec<i32> = (0..len).map(|k| r[k % r.len()]).collect();
            for x in &xs[1..] {
                let xv = quiet(x);
                let r = int_slice(&xv);
                let n = r.len();
                for (k, slot) in ra.iter_mut().enumerate() {
                    let tmp = r[k % n];
                    let better = if max { tmp > *slot } else { tmp < *slot };
                    if (narm && *slot == NA_INTEGER)
                        || (*slot != NA_INTEGER && tmp != NA_INTEGER && better)
                        || (!narm && tmp == NA_INTEGER)
                    {
                        *slot = tmp;
                    }
                }
            }
            Value::ints(ra)
        }
        VType::Real => {
            // Doubles borrowed when they are doubles already.
            fn get(v: &Value) -> std::borrow::Cow<'_, [f64]> {
                match v {
                    Value::Real(x) => std::borrow::Cow::Borrowed(&x.data[..]),
                    _ => match crate::coerce::coerce_quiet(v, VType::Real) {
                        Value::Real(x) => std::borrow::Cow::Owned(x.data.clone()),
                        _ => std::borrow::Cow::Owned(vec![]),
                    },
                }
            }
            let r = get(xs[0]);
            let mut ra: Vec<f64> = if r.len() == len { r.to_vec() } else { r.iter().copied().cycle().take(len).collect() };
            let update = |slot: &mut f64, tmp: f64| {
                let better = if max { tmp > *slot } else { tmp < *slot };
                if (narm && slot.is_nan()) || (!slot.is_nan() && !tmp.is_nan() && better) || (!narm && tmp.is_nan()) {
                    *slot = tmp;
                }
            };
            for x in &xs[1..] {
                let r = get(x);
                let n = r.len();
                if n == len {
                    for (slot, &tmp) in ra.iter_mut().zip(r.iter()) {
                        update(slot, tmp);
                    }
                } else if n == 1 {
                    let tmp = r[0];
                    for slot in ra.iter_mut() {
                        update(slot, tmp);
                    }
                } else {
                    for (slot, &tmp) in ra.iter_mut().zip(r.iter().cycle()) {
                        update(slot, tmp);
                    }
                }
            }
            Value::reals(ra)
        }
        _ => {
            let get = |v: &Value| -> Vec<RStr> {
                match quiet(v) {
                    Value::Str(x) => x.data.clone(),
                    _ => vec![],
                }
            };
            let r = get(xs[0]);
            let mut ra: Vec<RStr> = (0..len).map(|k| r[k % r.len()].clone()).collect();
            for x in &xs[1..] {
                let r = get(x);
                let n = r.len();
                for (k, slot) in ra.iter_mut().enumerate() {
                    let tmp = &r[k % n];
                    let take = match (&*slot, tmp) {
                        (None, _) if narm => true,
                        (_, None) if !narm => true,
                        (Some(t2), Some(tv)) => {
                            tv != t2 && {
                                let o = scollate(tv, t2);
                                if max {
                                    o == std::cmp::Ordering::Greater
                                } else {
                                    o == std::cmp::Ordering::Less
                                }
                            }
                        }
                        _ => false,
                    };
                    if take {
                        *slot = tmp.clone();
                    }
                }
            }
            Value::strs(ra)
        }
    })
}

// ----------------------------------------------------------------------
// which, which.min, which.max

// R: summary.c:do_which
fn do_which(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 1, "which", call)?;
    let v = &args[0].value;
    let Value::Lgl(lv) = v else {
        return Err(Flow::error("argument to 'which' is not logical"));
    };
    let idx: Vec<i32> = lv.data.iter().enumerate().filter(|(_, x)| **x == TRUE).map(|(k, _)| k as i32 + 1).collect();
    let names = get_attrib(v, s::NAMES);
    let mut ans = Value::ints(idx.clone());
    if let Value::Str(nm) = names {
        let nn: Vec<RStr> = idx.iter().map(|&k| nm.data[k as usize - 1].clone()).collect();
        ans.set_attr_raw(s::NAMES, Value::strs(nn));
    }
    Ok(ans)
}

fn do_which_min(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    first_min(i, call, args, env, false)
}
fn do_which_max(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    first_min(i, call, args, env, true)
}

// R: summary.c:do_first_min
fn first_min(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env, max: bool) -> RResult<Value> {
    check_arity_internal(&args, 1, if max { "which.max" } else { "which.min" }, call)?;
    let mut sx = args[0].value.clone();
    if !matches!(sx, Value::Lgl(_) | Value::Int(_) | Value::Real(_)) || sx.inherits("factor") {
        sx = i.coerce(&sx, VType::Real)?;
    }
    let mut indx: Option<usize> = None;
    match &sx {
        Value::Lgl(x) => {
            for (k, &r) in x.data.iter().enumerate() {
                let target = if max { TRUE } else { FALSE };
                if r == target {
                    indx = Some(k);
                    break;
                } else if indx.is_none() && r != NA_LOGICAL {
                    indx = Some(k);
                }
            }
        }
        Value::Int(x) => {
            let mut s = if max { i32::MIN } else { i32::MAX };
            for (k, &r) in x.data.iter().enumerate() {
                if r != NA_INTEGER && ((if max { r > s } else { r < s }) || indx.is_none()) {
                    s = r;
                    indx = Some(k);
                }
            }
        }
        Value::Real(x) => {
            let mut s = if max { f64::NEG_INFINITY } else { f64::INFINITY };
            for (k, &r) in x.data.iter().enumerate() {
                if !r.is_nan() && ((if max { r > s } else { r < s }) || indx.is_none()) {
                    s = r;
                    indx = Some(k);
                }
            }
        }
        _ => {}
    }
    let Some(k) = indx else { return Ok(Value::ints(vec![])) };
    let mut ans = Value::int(k as i32 + 1);
    if let Value::Str(nm) = get_attrib(&sx, s::NAMES) {
        ans.set_attr_raw(s::NAMES, Value::strs(vec![nm.data[k].clone()]));
    }
    Ok(ans)
}

// ----------------------------------------------------------------------
// cumsum, cumprod, cummax, cummin (cum.c)

#[derive(Clone, Copy, PartialEq, Eq)]
enum CumOp {
    Sum,
    Prod,
    Max,
    Min,
}

fn do_cumsum(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_cum(i, call, args, env, CumOp::Sum)
}
fn do_cumprod(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_cum(i, call, args, env, CumOp::Prod)
}
fn do_cummax(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_cum(i, call, args, env, CumOp::Max)
}
fn do_cummin(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    do_cum(i, call, args, env, CumOp::Min)
}

/// `handleNaN`: once a NaN is seen, every later result is NaN, or NA
/// once an NA has been seen.
fn handle_nan(x: &[f64], s: &mut [f64]) {
    let mut has_na = false;
    let mut has_nan = false;
    for (k, &v) in x.iter().enumerate() {
        has_nan = has_nan || v.is_nan();
        has_na = has_na || (has_nan && is_na_real(v));
        if has_na {
            s[k] = NA_REAL;
        } else if has_nan {
            s[k] = f64::NAN;
        }
    }
}

fn chandle_nan(x: &[Complex], s: &mut [Complex], r_isn: bool, i_isn: bool) {
    let mut has_na = false;
    let mut has_nan = false;
    for (k, z) in x.iter().enumerate() {
        has_nan = has_nan || z.re.is_nan() || z.im.is_nan();
        has_na = has_na || (has_nan && (is_na_real(z.re) || is_na_real(z.im)));
        if has_na {
            if r_isn {
                s[k].re = NA_REAL;
            }
            if i_isn {
                s[k].im = NA_REAL;
            }
        } else if has_nan {
            if r_isn {
                s[k].re = f64::NAN;
            }
            if i_isn {
                s[k].im = f64::NAN;
            }
        }
    }
}

// R: cum.c:do_cum
fn do_cum(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, op: CumOp) -> RResult<Value> {
    let name = match op {
        CumOp::Sum => "cumsum",
        CumOp::Prod => "cumprod",
        CumOp::Max => "cummax",
        CumOp::Min => "cummin",
    };
    super::check_arity(&args, 1, name, call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch_group("Math", name, call, &args, env)? {
            return Ok(r);
        }
    }
    let t = &args[0].value;
    let names = get_attrib(t, s::NAMES);
    let set_names = |mut v: Value| -> Value {
        if !names.is_null() {
            v.set_attr_raw(s::NAMES, names.clone());
        }
        v
    };
    if let Value::Cplx(x) = t {
        let n = x.data.len();
        let mut out = vec![Complex::default(); n];
        if n == 0 {
            return Ok(set_names(Value::cplxs(out)));
        }
        match op {
            CumOp::Sum => {
                let (mut sr, mut si) = (0.0f64, 0.0f64);
                for (k, z) in x.data.iter().enumerate() {
                    sr = add_sse(sr, z.re);
                    si = add_sse(si, z.im);
                    out[k] = Complex::new(sr, si);
                }
                if sr.is_nan() || si.is_nan() {
                    chandle_nan(&x.data, &mut out, sr.is_nan(), si.is_nan());
                }
            }
            CumOp::Prod => {
                let (mut pr, mut pi) = (1.0f64, 0.0f64);
                for (k, z) in x.data.iter().enumerate() {
                    let (tr, ti) = (pr, pi);
                    pr = z.re * tr - z.im * ti;
                    pi = z.re * ti + z.im * tr;
                    out[k] = Complex::new(pr, pi);
                }
                if pr.is_nan() || pi.is_nan() {
                    chandle_nan(&x.data, &mut out, pr.is_nan(), pi.is_nan());
                }
            }
            CumOp::Max => return Err(Flow::error_call(call.clone(), "'cummax' not defined for complex numbers")),
            CumOp::Min => return Err(Flow::error_call(call.clone(), "'cummin' not defined for complex numbers")),
        }
        return Ok(set_names(Value::cplxs(out)));
    }
    if matches!(t, Value::Int(_) | Value::Lgl(_)) && op != CumOp::Prod {
        let x = int_slice(t);
        let n = x.len();
        let mut s = vec![NA_INTEGER; n];
        if n == 0 {
            return Ok(set_names(Value::ints(s)));
        }
        match op {
            CumOp::Sum => {
                let mut sum = 0.0f64;
                for k in 0..n {
                    if x[k] == NA_INTEGER {
                        break;
                    }
                    sum += x[k] as f64;
                    if sum > i32::MAX as f64 || sum < (1 + i32::MIN) as f64 {
                        i.warning("integer overflow in 'cumsum'; use 'cumsum(as.numeric(.))'")?;
                        break;
                    }
                    s[k] = sum as i32;
                }
            }
            CumOp::Max | CumOp::Min => {
                if x[0] != NA_INTEGER || op == CumOp::Min {
                    let mut m = x[0];
                    s[0] = m;
                    for k in 1..n {
                        if x[k] == NA_INTEGER {
                            break;
                        }
                        m = if op == CumOp::Max { m.max(x[k]) } else { m.min(x[k]) };
                        s[k] = m;
                    }
                }
            }
            CumOp::Prod => unreachable!(),
        }
        return Ok(set_names(Value::ints(s)));
    }
    let tv = i.coerce(t, VType::Real)?;
    let x: &[f64] = match &tv {
        Value::Real(v) => &v.data,
        _ => &[],
    };
    let n = x.len();
    let mut s = vec![0.0f64; n];
    if n == 0 {
        return Ok(set_names(Value::reals(s)));
    }
    match op {
        CumOp::Sum => {
            let sum = F80::ZERO.cumsum_slice(&x[..n], &mut s[..n]);
            if sum.is_nan() {
                handle_nan(x, &mut s);
            }
        }
        CumOp::Prod => {
            let prod = F80::ONE.cumprod_slice(&x[..n], &mut s[..n]);
            if prod.is_nan() {
                handle_nan(x, &mut s);
            }
        }
        CumOp::Max | CumOp::Min => {
            let mut m = if op == CumOp::Max { f64::NEG_INFINITY } else { f64::INFINITY };
            for k in 0..n {
                if x[k].is_nan() {
                    handle_nan(x, &mut s);
                    break;
                }
                m = if op == CumOp::Max {
                    if m > x[k] {
                        m
                    } else {
                        x[k]
                    }
                } else if m < x[k] {
                    m
                } else {
                    x[k]
                };
                s[k] = m;
            }
        }
    }
    Ok(set_names(Value::reals(s)))
}

// ----------------------------------------------------------------------
// anyNA (coerce.c)

// R: coerce.c:do_anyNA
fn do_any_na(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err(Flow::error_call(call.clone(), "anyNA takes 1 or 2 arguments"));
    }
    if let Some(r) = i.try_dispatch("anyNA", call, &args, env)? {
        return Ok(r);
    }
    let (x, recursive) = if args.len() == 1 {
        if let Some(t) = args[0].tag {
            if !"x".starts_with(&*t.name()) {
                return Err(Flow::error_call(
                    call.clone(),
                    format!("supplied argument name '{}' does not match '{}'", t, "x"),
                ));
            }
        }
        (args[0].value.clone(), Value::lgl(false))
    } else {
        let mut m = super::match_args(args, &["x", "recursive"], call)?;
        (m.or_null(0), m.take(1).unwrap_or_else(|| Value::lgl(false)))
    };
    let r = any_na(i, call, &x, &recursive, env)?;
    Ok(Value::lgl(r))
}

fn any_na(i: &mut Interp, call: &Value, x: &Value, recursive: &Value, env: &Env) -> RResult<bool> {
    let is_list = matches!(x, Value::List(_) | Value::Pairlist(_));
    let rec = if is_list { as_rbool(recursive, call)? } else { false };
    if x.is_object() || (is_list && !rec) {
        let isna = i.find_fun(Sym::new("is.na"), env)?;
        let e0 = i.call_value(&isna, vec![Arg::pos(x.clone())], env)?;
        let any = i.find_fun(Sym::new("any"), env)?;
        let res = i.call_value(&any, vec![Arg::pos(e0)], env)?;
        return Ok(crate::coerce::as_logical(&res) == 1);
    }
    Ok(match x {
        Value::Real(v) => v.data.iter().any(|d| d.is_nan()),
        Value::Int(v) | Value::Lgl(v) => v.data.iter().any(|&d| d == NA_INTEGER),
        Value::Cplx(v) => v.data.iter().any(|z| z.re.is_nan() || z.im.is_nan()),
        Value::Str(v) => v.data.iter().any(|s| s.is_none()),
        Value::Raw(_) | Value::Null => false,
        Value::List(_) | Value::Pairlist(_) => {
            let items = crate::coerce::to_list(x);
            for el in items {
                let elcall = match call {
                    Value::Lang(l) => {
                        let mut nl = (**l).clone();
                        if let Some(a) = nl.args.first_mut() {
                            a.value = el.clone();
                        }
                        Value::Lang(Rc::new(nl))
                    }
                    c => c.clone(),
                };
                let a2 = vec![Arg::pos(el.clone()), Arg::pos(recursive.clone())];
                if let Some(r) = i.try_dispatch("anyNA", &elcall, &a2, env)? {
                    if crate::coerce::as_logical(&r) == TRUE {
                        return Ok(true);
                    }
                }
                if any_na(i, &elcall, &el, recursive, env)? {
                    return Ok(true);
                }
            }
            false
        }
        other => {
            return Err(Flow::error(format!(
                "anyNA() applied to non-(list or vector) of type '{}'",
                type_to_char(other)
            )))
        }
    })
}

/// R's `asRbool(x, call)`.
fn as_rbool(v: &Value, call: &Value) -> RResult<bool> {
    let l = as_logical2(v, true, call)?;
    if l == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    Ok(l != 0)
}
