//! Sequences and replication (GNU R's seq.c).

use std::rc::Rc;

use super::summary::check_arity_internal;
use super::{check_arity, internal, prim, special};
use crate::attrib::get_attrib;
use crate::coerce::{CoerceWarnings, as_integer};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim(":", do_colon, 2),
    prim("seq_len", do_seq_len, 1),
    prim("seq_along", do_seq_along, 1),
    prim("seq.int", do_seq, -1),
    special("rep", do_rep, -1),
    internal("rep.int", do_rep_int, 2),
    internal("rep_len", do_rep_len, 2),
    internal("sequence", do_sequence, 3),
];

/// R_XLEN_T_MAX
const XLEN_MAX: f64 = 4503599627370496.0;

// ----------------------------------------------------------------------
// vector helpers shared with sort.rs / unique.rs

/// Select elements `idx` (0-based) of a vector; `None` gives NA (NULL for
/// lists). Attributes are not copied.
pub fn gather(x: &Value, idx: &[Option<usize>]) -> Value {
    fn g<T: Elt>(d: &[T], idx: &[Option<usize>]) -> Vec<T> {
        idx.iter().map(|k| k.map(|k| d[k].clone()).unwrap_or(T::NA)).collect()
    }
    match x {
        Value::Lgl(v) => Value::lgls(g(&v.data, idx)),
        Value::Int(v) => Value::ints(g(&v.data, idx)),
        Value::Real(v) => Value::reals(g(&v.data, idx)),
        Value::Cplx(v) => Value::cplxs(g(&v.data, idx)),
        Value::Str(v) => Value::strs(g(&v.data, idx)),
        Value::Raw(v) => Value::raws(g(&v.data, idx)),
        Value::List(v) => Value::list(g(&v.data, idx)),
        Value::Expr(v) => Value::expr_vec(g(&v.data, idx)),
        _ => Value::Null,
    }
}

/// `gather` with all indices present.
pub fn gather_all(x: &Value, idx: &[usize]) -> Value {
    fn g<T: Clone>(d: &[T], idx: &[usize]) -> Vec<T> {
        idx.iter().map(|&k| d[k].clone()).collect()
    }
    match x {
        Value::Lgl(v) => Value::lgls(g(&v.data, idx)),
        Value::Int(v) => Value::ints(g(&v.data, idx)),
        Value::Real(v) => Value::reals(g(&v.data, idx)),
        Value::Cplx(v) => Value::cplxs(g(&v.data, idx)),
        Value::Str(v) => Value::strs(g(&v.data, idx)),
        Value::Raw(v) => Value::raws(g(&v.data, idx)),
        Value::List(v) => Value::list(g(&v.data, idx)),
        Value::Expr(v) => Value::expr_vec(g(&v.data, idx)),
        _ => Value::Null,
    }
}

/// R's `xlengthgets`: truncate or pad with NA, keeping (padded) names.
pub fn xlengthgets(x: &Value, len: usize) -> Value {
    let n = x.len();
    let idx: Vec<Option<usize>> = (0..len).map(|k| if k < n { Some(k) } else { None }).collect();
    let mut out = gather(x, &idx);
    if let Value::Str(nm) = get_attrib(x, s::NAMES) {
        let nn: Vec<RStr> = (0..len).map(|k| if k < n { nm.data[k].clone() } else { Some(Rc::from("")) }).collect();
        out.set_attr_raw(s::NAMES, Value::strs(nn));
    }
    out
}

/// `asReal` with R's coercion warnings.
pub fn as_real_warn(i: &mut Interp, v: &Value) -> RResult<f64> {
    if v.is_atomic() && !v.is_empty() {
        let mut w = CoerceWarnings::default();
        let first = crate::coerce::first_elt(v);
        let d = crate::coerce::to_real(&first, &mut w)[0];
        w.emit(i)?;
        return Ok(d);
    }
    Ok(NA_REAL)
}

/// Evaluate the arguments of a special keeping empty arguments as the
/// missing marker (`evalListKeepMissing`), after offering the first
/// argument for dispatch (`DispatchOrEval` with unevaluated args).
/// Returns `Ok(Ok(result))` if a method was applied.
pub fn dispatch_or_eval_special(
    i: &mut Interp,
    generic: &str,
    call: &Value,
    args: &[Arg],
    env: &Env,
) -> RResult<Result<Value, Vec<Arg>>> {
    let pargs = i.promise_args(args, env)?;
    // With no arguments (after dropping empty `...`), DispatchOrEval
    // evaluates to list(NULL).
    if pargs.is_empty() {
        return Ok(Err(vec![Arg::pos(Value::Null)]));
    }
    if let Some(first) = pargs.first() {
        let v = match &first.value {
            Value::Promise(p) => i.force_promise(p)?,
            v => v.clone(),
        };
        if v.is_object() {
            if let Some(r) = i.dispatch_with_temp_context(generic, &v, call, pargs.clone(), env)? {
                return Ok(Ok(r));
            }
        }
    }
    let mut out = Vec::with_capacity(pargs.len());
    for a in pargs {
        let v = match &a.value {
            Value::Promise(p) => i.force_promise(p)?,
            v => v.clone(),
        };
        out.push(Arg::new(a.tag, v));
    }
    Ok(Err(out))
}

// ----------------------------------------------------------------------
// `:`

// R: seq.c:cross_colon
fn cross_colon(call: &Value, s1: &Value, t: &Value) -> RResult<Value> {
    if s1.len() != t.len() {
        return Err(Flow::error_call(call.clone(), "unequal factor lengths"));
    }
    let ls = get_attrib(s1, s::LEVELS);
    let lt = get_attrib(t, s::LEVELS);
    let nlt = lt.len() as i32;
    let rs = crate::coerce::coerce_quiet(s1, VType::Int);
    let rt = crate::coerce::coerce_quiet(t, VType::Int);
    let (rs, rt) = (int_view(&rs), int_view(&rt));
    let a: Vec<i32> = rs
        .iter()
        .zip(rt.iter())
        .map(|(&vs, &vt)| if vs == NA_INTEGER || vt == NA_INTEGER { NA_INTEGER } else { vt + (vs - 1) * nlt })
        .collect();
    let mut ans = Value::ints(a);
    if let (Value::Str(lsv), Value::Str(ltv)) = (&ls, &lt) {
        let mut la = Vec::with_capacity(lsv.data.len() * ltv.data.len());
        for vi in &lsv.data {
            for vj in &ltv.data {
                la.push(rstr(&format!("{}:{}", vi.as_deref().unwrap_or("NA"), vj.as_deref().unwrap_or("NA"))));
            }
        }
        ans.set_attr_raw(s::LEVELS, Value::strs(la));
    }
    ans.set_attr_raw(s::CLASS, Value::str("factor"));
    Ok(ans)
}

fn int_view(v: &Value) -> Vec<i32> {
    match v {
        Value::Int(x) | Value::Lgl(x) => x.data.clone(),
        _ => vec![],
    }
}

fn do_colon(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity(&args, 2, ":", call)?;
    let (x, y) = (&args[0].value, &args[1].value);
    if x.inherits("factor") && y.inherits("factor") {
        return cross_colon(call, x, y);
    }
    let (l1, l2) = (x.len(), y.len());
    if l1 != 1 || l2 != 1 {
        if l1 == 0 || l2 == 0 {
            return Err(Flow::error_call(call.clone(), "argument of length 0"));
        }
        i.warningcall(
            call,
            format!("numerical expression has {} elements: only the first used", if l1 > 1 { l1 } else { l2 }),
        )?;
    }
    let n1 = as_real_warn(i, x)?;
    let n2 = as_real_warn(i, y)?;
    if n1.is_nan() || n2.is_nan() {
        return Err(Flow::error_call(call.clone(), "NA/NaN argument"));
    }
    seq_colon(n1, n2, call)
}

/// `R_compact_intrange`.
fn compact_intrange(n1: i64, n2: i64, call: &Value) -> RResult<Value> {
    let n = if n1 <= n2 { n2 - n1 + 1 } else { n1 - n2 + 1 };
    if n as f64 >= XLEN_MAX {
        return Err(Flow::error_call(call.clone(), "result would be too long a vector"));
    }
    let up = n1 <= n2;
    if n1 <= i32::MIN as i64 || n1 > i32::MAX as i64 || n2 <= i32::MIN as i64 || n2 > i32::MAX as i64 {
        let start = n1 as f64;
        let v: Vec<f64> = (0..n).map(|k| if up { start + k as f64 } else { start - k as f64 }).collect();
        return Ok(Value::reals(v));
    }
    let v: Vec<i32> = (0..n).map(|k| if up { (n1 + k) as i32 } else { (n1 - k) as i32 }).collect();
    Ok(Value::ints(v))
}

/// R's `seq_colon`.
pub fn seq_colon(n1: f64, n2: f64, call: &Value) -> RResult<Value> {
    let r = (n2 - n1).abs();
    if r >= XLEN_MAX {
        return Err(Flow::error_call(call.clone(), "result would be too long a vector"));
    }
    if n1 == (n1 as i64) as f64 && n2 == (n2 as i64) as f64 {
        return compact_intrange(n1 as i64, n2 as i64, call);
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
        let s1 = n1 as i64;
        if n1 <= n2 { compact_intrange(s1, s1 + n - 1, call) } else { compact_intrange(s1, s1 - n + 1, call) }
    } else {
        let v: Vec<f64> = (0..n).map(|k| if n1 <= n2 { n1 + k as f64 } else { n1 - k as f64 }).collect();
        Ok(Value::reals(v))
    }
}

// ----------------------------------------------------------------------
// seq_len, seq_along

/// R's `check1arg`.
pub fn check1arg(args: &[Arg], call: &Value, formal: &str) -> RResult<()> {
    if let Some(t) = args.first().and_then(|a| a.tag) {
        let supplied = t.name();
        if supplied.len() > formal.len() || !formal.starts_with(&*supplied) {
            return Err(Flow::error_call(
                call.clone(),
                format!("supplied argument name '{}' does not match '{}'", supplied, formal),
            ));
        }
    }
    Ok(())
}

fn do_seq_len(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    check_arity(&args, 1, "seq_len", call)?;
    check1arg(&args, call, "length.out")?;
    let v = &args[0].value;
    if v.len() != 1 {
        i.warningcall(call, "first element used of 'length.out' argument")?;
    }
    let d = as_real_warn(i, v)?;
    if !d.is_finite() || d < 0.0 {
        return Err(Flow::error_call(call.clone(), "argument must be coercible to non-negative integer"));
    }
    if d >= XLEN_MAX {
        return Err(Flow::error_call(call.clone(), "result would be too long a vector"));
    }
    let n = d as i64;
    if n == 0 {
        return Ok(Value::ints(vec![]));
    }
    compact_intrange(1, n, call)
}

fn do_seq_along(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity(&args, 1, "seq_along", call)?;
    check1arg(&args, call, "along.with")?;
    let mut len = args[0].value.len() as i64;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch("length", call, &args, env)? {
            len = as_integer(&r) as i64;
        }
    }
    if len == 0 {
        return Ok(Value::ints(vec![]));
    }
    compact_intrange(1, len, call)
}

// ----------------------------------------------------------------------
// seq.int

const FEPS: f64 = 1e-10;

fn is_finite(x: f64) -> bool {
    x.is_finite()
}

// R: seq.c:do_seq
fn do_seq(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let one = args.len() == 1;
    if let Some(r) = i.try_dispatch("seq", call, &args, env)? {
        return Ok(r);
    }
    let m = super::match_args(args, &["from", "to", "by", "length.out", "along.with", "..."], call)?;
    let from = m.get(0).cloned();
    let to = m.get(1).cloned();
    let by = m.get(2).cloned();
    let len = m.get(3).cloned();
    let along = m.get(4).cloned();
    let miss_from = from.is_none();
    let miss_to = to.is_none();
    let err = |msg: String| Flow::error_call(call.clone(), msg);
    let finite_err = |what: &str| Flow::error_call(call.clone(), format!("'{}' must be a finite number", what));

    if one && !miss_from {
        let f = from.as_ref().unwrap();
        let lf = f.len();
        if lf == 1 && matches!(f, Value::Int(_) | Value::Real(_)) {
            let rfrom = as_real_warn(i, f)?;
            if !rfrom.is_finite() {
                return Err(finite_err("from"));
            }
            return seq_colon(1.0, rfrom, call);
        } else if lf > 0 {
            return seq_colon(1.0, lf as f64, call);
        } else {
            return Ok(Value::ints(vec![]));
        }
    }
    let mut lout: Option<i64> = None;
    if let Some(al) = &along {
        let n = al.len() as i64;
        if one {
            return if n > 0 { seq_colon(1.0, n as f64, call) } else { Ok(Value::ints(vec![])) };
        }
        lout = Some(n);
    } else if let Some(l) = len.as_ref().filter(|l| !l.is_null()) {
        let rout = as_real_warn(i, l)?;
        if !rout.is_finite() {
            return Err(finite_err("length.out"));
        }
        if rout.is_nan() || rout <= -0.5 {
            return Err(err("'length.out' must be a non-negative number".into()));
        }
        if l.len() != 1 {
            i.warningcall(call, "first element used of 'length.out' argument")?;
        }
        let rout = rout.ceil();
        if rout >= XLEN_MAX {
            return Err(err("result would be too long a vector".into()));
        }
        lout = Some(rout as i64);
    }

    let Some(lout) = lout else {
        let rfrom = match &from {
            None => 1.0,
            Some(f) => {
                if f.len() != 1 {
                    return Err(err("'from' must be of length 1".into()));
                }
                let r = as_real_warn(i, f)?;
                if !r.is_finite() {
                    return Err(finite_err("from"));
                }
                r
            }
        };
        let rto = match &to {
            None => 1.0,
            Some(t) => {
                if t.len() != 1 {
                    return Err(err("'to' must be of length 1".into()));
                }
                let r = as_real_warn(i, t)?;
                if !r.is_finite() {
                    return Err(finite_err("to"));
                }
                r
            }
        };
        let Some(by) = by else { return seq_colon(rfrom, rto, call) };
        if by.len() != 1 {
            return Err(err("'by' must be of length 1".into()));
        }
        let del = rto - rfrom;
        if del == 0.0 && rto == 0.0 {
            return Ok(to.unwrap());
        }
        let rby = as_real_warn(i, &by)?;
        if (rby == 1.0 && del > 0.0) || (rby == -1.0 && del < 0.0) {
            return seq_colon(rfrom, rto, call);
        }
        let finite_del = is_finite(del);
        let n = if finite_del { del / rby } else { rto / rby - rfrom / rby };
        if !n.is_finite() {
            if del == 0.0 && rby == 0.0 {
                return Ok(if miss_from { Value::real(rfrom) } else { from.unwrap() });
            }
            return Err(err("invalid '(to - from)/by'".into()));
        }
        if finite_del && del.abs() / rto.abs().max(rfrom.abs()) < 100.0 * f64::EPSILON {
            return Ok(if miss_from { Value::real(rfrom) } else { from.unwrap() });
        }
        if n > 100.0 * i32::MAX as f64 {
            return Err(err("'by' argument is much too small".into()));
        }
        if n < -FEPS {
            return Err(err("wrong sign in 'by' argument".into()));
        }
        let is_int = |v: &Option<Value>| v.as_ref().is_none_or(|x| matches!(x, Value::Int(_)));
        if is_int(&from) && is_int(&to) && matches!(by, Value::Int(_)) {
            let ifrom = if miss_from { rfrom as i32 } else { as_integer(from.as_ref().unwrap()) } as i64;
            let iby = as_integer(&by) as i64;
            let nn = n as i64;
            let v: Vec<i32> = (0..=nn).map(|k| (ifrom + k * iby) as i32).collect();
            return Ok(Value::ints(v));
        }
        let nn = (n + FEPS) as i32 as i64;
        let mut ra: Vec<f64> = Vec::with_capacity(nn as usize + 1);
        if finite_del {
            for k in 0..=nn {
                ra.push(rfrom + k as f64 * rby);
            }
        } else {
            let (f4, b4) = (rfrom / 4.0, rby / 4.0);
            for k in 0..=nn {
                ra.push((f4 + k as f64 * b4) * 4.0);
            }
        }
        if nn > 0 {
            let last = ra[nn as usize];
            if (rby > 0.0 && last > rto) || (rby < 0.0 && last < rto) {
                ra[nn as usize] = rto;
            }
        }
        return Ok(Value::reals(ra));
    };

    if lout == 0 {
        return Ok(Value::ints(vec![]));
    }
    if one {
        return seq_colon(1.0, lout as f64, call);
    }
    let real_of = |i: &mut Interp, v: &Option<Value>| -> RResult<f64> {
        match v {
            Some(v) => as_real_warn(i, v),
            None => Ok(NA_REAL),
        }
    };
    if by.is_none() {
        let mut rfrom = real_of(i, &from)?;
        let mut rto = real_of(i, &to)?;
        let mut rby = 0.0;
        if miss_to {
            rto = rfrom + lout as f64 - 1.0;
        }
        if miss_from {
            rfrom = rto - lout as f64 + 1.0;
        }
        if !rfrom.is_finite() {
            return Err(finite_err("from"));
        }
        if !rto.is_finite() {
            return Err(finite_err("to"));
        }
        let mut finite_del = false;
        if lout > 2 {
            let nint = (lout - 1) as f64;
            rby = rto - rfrom;
            finite_del = rby.is_finite();
            if finite_del {
                rby /= nint;
            } else {
                rby = rto / nint - rfrom / nint;
            }
        }
        let fits = |x: f64| x <= i32::MAX as f64 && x >= i32::MIN as f64;
        if fits(rfrom)
            && fits(rto)
            && rfrom == (rfrom as i32) as f64
            && (lout <= 1 || rto == (rto as i32) as f64)
            && (lout <= 2 || rby == (rby as i32) as f64)
        {
            let mut v = vec![0i32; lout as usize];
            v[0] = rfrom as i32;
            if lout > 1 {
                v[lout as usize - 1] = rto as i32;
            }
            for k in 1..(lout - 1).max(1) {
                if lout > 2 {
                    v[k as usize] = (rfrom + k as f64 * rby) as i32;
                }
            }
            return Ok(Value::ints(v));
        }
        let mut v = vec![0.0f64; lout as usize];
        v[0] = rfrom;
        if lout > 1 {
            v[lout as usize - 1] = rto;
        }
        if lout > 2 {
            if finite_del {
                for k in 1..lout - 1 {
                    v[k as usize] = rfrom + k as f64 * rby;
                }
            } else {
                let (f4, b4) = (rfrom / 4.0, rby / 4.0);
                for k in 1..lout - 1 {
                    v[k as usize] = (f4 + k as f64 * b4) * 4.0;
                }
            }
        }
        return Ok(Value::reals(v));
    }
    let fits = |x: f64| x <= i32::MAX as f64 && x >= i32::MIN as f64;
    if miss_to {
        let mut rfrom = real_of(i, &from)?;
        let rby = real_of(i, &by)?;
        if miss_from {
            rfrom = 1.0;
        }
        if !rfrom.is_finite() {
            return Err(finite_err("from"));
        }
        if !rby.is_finite() {
            return Err(finite_err("by"));
        }
        let rto = rfrom + (lout - 1) as f64 * rby;
        if fits(rfrom) && fits(rto) && rby == (rby as i32) as f64 && rfrom == (rfrom as i32) as f64 {
            let v: Vec<i32> = (0..lout).map(|k| (rfrom + k as f64 * rby) as i32).collect();
            return Ok(Value::ints(v));
        }
        let v: Vec<f64> = (0..lout).map(|k| rfrom + k as f64 * rby).collect();
        return Ok(Value::reals(v));
    }
    if miss_from {
        let rto = real_of(i, &to)?;
        let rby = real_of(i, &by)?;
        let rfrom = rto - (lout - 1) as f64 * rby;
        if !rto.is_finite() {
            return Err(finite_err("to"));
        }
        if !rby.is_finite() {
            return Err(finite_err("by"));
        }
        if rby == (rby as i32) as f64 && rto == (rto as i32) as f64 && fits(rfrom) && fits(rto) {
            let v: Vec<i32> = (0..lout).map(|k| (rto - (lout - 1 - k) as f64 * rby) as i32).collect();
            return Ok(Value::ints(v));
        }
        let v: Vec<f64> = (0..lout).map(|k| rto - (lout - 1 - k) as f64 * rby).collect();
        return Ok(Value::reals(v));
    }
    Err(err("too many arguments".into()))
}

// ----------------------------------------------------------------------
// rep

/// R's `rep4` (and `rep3` for each == 1, nt == 1) on the data of a
/// non-empty vector.
fn rep4<T: Clone>(d: &[T], times: &[i64], len: usize, each: usize, nt: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(len);
    if nt == 1 && each == 1 {
        while out.len() < len {
            let k = (len - out.len()).min(d.len());
            out.extend_from_slice(&d[..k]);
        }
        return out;
    }
    if nt == 1 {
        'cycle: loop {
            for v in d {
                for _ in 0..each {
                    if out.len() == len {
                        break 'cycle;
                    }
                    out.push(v.clone());
                }
            }
        }
        return out;
    }
    let mut kk = 0;
    'outer: for v in d {
        let mut sum: i64 = 0;
        for _ in 0..each {
            sum += times[kk];
            kk += 1;
        }
        for _ in 0..sum {
            if out.len() == len {
                break 'outer;
            }
            out.push(v.clone());
        }
    }
    out
}

fn rep4_value(x: &Value, times: &[i64], len: usize, each: usize, nt: usize) -> Value {
    match x {
        Value::Lgl(v) => Value::lgls(rep4(&v.data, times, len, each, nt)),
        Value::Int(v) => Value::ints(rep4(&v.data, times, len, each, nt)),
        Value::Real(v) => Value::reals(rep4(&v.data, times, len, each, nt)),
        Value::Cplx(v) => Value::cplxs(rep4(&v.data, times, len, each, nt)),
        Value::Str(v) => Value::strs(rep4(&v.data, times, len, each, nt)),
        Value::Raw(v) => Value::raws(rep4(&v.data, times, len, each, nt)),
        Value::List(v) => Value::list(rep4(&v.data, times, len, each, nt)),
        Value::Expr(v) => Value::expr_vec(rep4(&v.data, times, len, each, nt)),
        _ => Value::Null,
    }
}

/// Read a `times`-like vector as truncated counts, validating as R does.
/// `Err(())` means "invalid".
fn counts_of(v: &Value) -> Result<Vec<i64>, ()> {
    match v {
        Value::Int(x) => {
            x.data.iter().map(|&t| if t == NA_INTEGER || t < 0 { Err(()) } else { Ok(t as i64) }).collect()
        }
        _ => {
            let r = match crate::coerce::coerce_quiet(v, VType::Real) {
                Value::Real(x) => x.data.clone(),
                _ => vec![],
            };
            r.iter()
                .map(|&t| if t.is_nan() || t <= -1.0 || t >= XLEN_MAX + 1.0 { Err(()) } else { Ok(t as i64) })
                .collect()
        }
    }
}

// R: seq.c:do_rep
fn do_rep(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let ev = match dispatch_or_eval_special(i, "rep", call, &args, env)? {
        Ok(r) => return Ok(r),
        Err(ev) => ev,
    };
    let m = super::match_args(ev, &["x", "times", "length.out", "each", "..."], call)?;
    let err = |what: &str| Flow::error_call(call.clone(), format!("invalid '{}' argument", what));
    let x = match m.get(0) {
        Some(x) => x.clone(),
        None => {
            return Err(Flow::error_call(call.clone(), "attempt to replicate an object of type 'symbol'"));
        }
    };
    if matches!(x, Value::Pairlist(_)) {
        return Err(Flow::error_call(call.clone(), "replication of pairlists is defunct"));
    }
    let lx = x.len();
    // length.out
    let lo = m.get(2).cloned();
    let mut len: Option<usize> = None;
    match &lo {
        Some(Value::Int(_)) => {
            let l = as_integer(lo.as_ref().unwrap());
            if l != NA_INTEGER && l < 0 {
                return Err(err("length.out"));
            }
            if l != NA_INTEGER {
                len = Some(l as usize);
            }
        }
        Some(v) => {
            let slen = as_real_warn(i, v)?;
            if slen.is_finite() {
                if slen <= -1.0 || slen >= XLEN_MAX + 1.0 {
                    return Err(err("length.out"));
                }
                len = Some(slen as i64 as usize);
            }
        }
        None => {}
    }
    if lo.as_ref().is_some_and(|v| v.len() != 1) {
        i.warningcall(call, "first element used of 'length.out' argument")?;
    }
    // each
    let ea = m.get(3).cloned();
    let mut each: Option<usize> = None;
    match &ea {
        Some(Value::Int(_)) => {
            let e = as_integer(ea.as_ref().unwrap());
            if e != NA_INTEGER && e < 0 {
                return Err(err("each"));
            }
            if e != NA_INTEGER {
                each = Some(e as usize);
            }
        }
        Some(v) => {
            let seach = as_real_warn(i, v)?;
            if seach.is_finite() {
                if seach <= -1.0 || (lx > 0 && seach >= XLEN_MAX + 1.0) {
                    return Err(err("each"));
                }
                each = if lx == 0 { None } else { Some(seach as i64 as usize) };
            }
        }
        None => {}
    }
    if ea.as_ref().is_some_and(|v| v.len() != 1) {
        i.warningcall(call, "first element used of 'each' argument")?;
    }
    let each = each.unwrap_or(1);

    if lx == 0 {
        if len.is_some_and(|l| l > 0) && x.is_null() {
            i.warningcall(call, "'x' is NULL so the result will be NULL")?;
        }
        if let Some(l) = len {
            if l > 0 && !x.is_null() {
                return Ok(xlengthgets(&x, l));
            }
        }
        return Ok(x);
    }
    if !x.is_vector() {
        return Err(Flow::error_call(
            call.clone(),
            format!("attempt to replicate an object of type '{}'", x.type_name()),
        ));
    }
    let mut times: Vec<i64> = vec![1];
    let nt;
    let len = match len {
        Some(l) => {
            nt = 1;
            l
        }
        None => {
            let tv = m.get(1).cloned().unwrap_or_else(|| Value::int(1));
            // R: seq.c:do_rep coerces non-integer `times` to double first,
            // which warns (e.g. "NAs introduced by coercion").
            let tv = if matches!(tv, Value::Int(_)) { tv } else { i.coerce(&tv, VType::Real)? };
            let t = counts_of(&tv).map_err(|_| err("times"))?;
            nt = t.len();
            if nt == 1 {
                let it = t[0];
                if (lx as f64) * (it as f64) * (each as f64) > XLEN_MAX {
                    return Err(err("times"));
                }
                times = t;
                lx * it as usize * each
            } else {
                if nt as f64 != (lx as f64) * (each as f64) {
                    return Err(err("times"));
                }
                let sum: i64 = t.iter().sum();
                times = t;
                sum as usize
            }
        }
    };
    if len > 0 && each == 0 {
        return Err(err("each"));
    }
    let len = if each == 0 { 0 } else { len };
    let each = each.max(1);
    let mut ans = rep4_value(&x, &times, len, each, nt);
    if let Value::Str(xn) = get_attrib(&x, s::NAMES) {
        if !xn.data.is_empty() {
            ans.set_attr_raw(s::NAMES, Value::strs(rep4(&xn.data, &times, len, each, nt)));
        }
    }
    Ok(ans)
}

fn factor_class_levels(s1: &Value, a: &mut Value) {
    if s1.inherits("factor") {
        let cls = if s1.inherits("ordered") { Value::strs_from(["ordered", "factor"]) } else { Value::str("factor") };
        a.set_attr_raw(s::CLASS, cls);
        a.set_attr_raw(s::LEVELS, get_attrib(s1, s::LEVELS));
    }
}

// R: seq.c:do_rep_int
fn do_rep_int(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "rep.int", call)?;
    if let Some(r) = i.try_dispatch("rep.int", call, &args, env)? {
        return Ok(r);
    }
    if let Some(r) = i.try_dispatch("rep", call, &args, env)? {
        return Ok(r);
    }
    let s1 = &args[0].value;
    let ncopy = &args[1].value;
    if !ncopy.is_vector() {
        return Err(Flow::error(format!("invalid type ({}) for '{}' (must be a vector)", ncopy.type_name(), "times")));
    }
    if !s1.is_vector() && !s1.is_null() {
        return Err(Flow::error(format!("attempt to replicate an object of type '{}'", s1.type_name())));
    }
    let bad = || Flow::error("invalid 'times' value");
    let nc = ncopy.len();
    let ns = s1.len();
    let mut a = if nc == ns {
        let t = counts_of(ncopy).map_err(|_| bad())?;
        let total: i64 = t.iter().sum();
        if total as f64 > XLEN_MAX {
            return Err(bad());
        }
        if s1.is_null() {
            return Err(Flow::error("unimplemented type 'NULL' in 'rep2'\n"));
        } else {
            let idx: Vec<usize> = t.iter().enumerate().flat_map(|(k, &c)| std::iter::repeat_n(k, c as usize)).collect();
            gather_all(s1, &idx)
        }
    } else {
        if nc != 1 {
            return Err(bad());
        }
        let n: usize = if !matches!(ncopy, Value::Int(_)) {
            let snc = as_real_warn(i, ncopy)?;
            if !snc.is_finite() || snc <= -1.0 || (ns > 0 && snc >= XLEN_MAX + 1.0) {
                return Err(bad());
            }
            if ns == 0 { 1 } else { snc as i64 as usize }
        } else {
            let k = as_integer(ncopy);
            if k == NA_INTEGER || k < 0 {
                return Err(bad());
            }
            k as usize
        };
        if (n as f64) * (ns as f64) > XLEN_MAX {
            return Err(bad());
        }
        if s1.is_null() {
            return Err(Flow::error("unimplemented type 'NULL' in 'rep3'\n"));
        } else {
            let idx: Vec<usize> = (0..n * ns).map(|k| k % ns).collect();
            gather_all(s1, &idx)
        }
    };
    factor_class_levels(s1, &mut a);
    Ok(a)
}

// R: seq.c:do_rep_len
fn do_rep_len(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 2, "rep_len", call)?;
    if let Some(r) = i.try_dispatch("rep_len", call, &args, env)? {
        return Ok(r);
    }
    let s1 = &args[0].value;
    if s1.is_object() {
        let rep_call = match call {
            Value::Lang(l) => {
                let mut nl = (**l).clone();
                nl.fun = Value::sym("rep");
                if let Some(a) = nl.args.get_mut(1) {
                    a.tag = Some(Sym::new("length.out"));
                }
                Value::Lang(Rc::new(nl))
            }
            c => c.clone(),
        };
        let mut a2 = args.clone();
        a2[1].tag = Some(Sym::new("length.out"));
        if let Some(r) = i.try_dispatch("rep", &rep_call, &a2, env)? {
            return Ok(r);
        }
    }
    if !s1.is_vector() && !s1.is_null() {
        return Err(Flow::error("attempt to replicate non-vector"));
    }
    let lenv = &args[1].value;
    let bad = || Flow::error("invalid 'length.out' value");
    if lenv.len() != 1 {
        return Err(bad());
    }
    let na: usize = if !matches!(lenv, Value::Int(_)) {
        let sna = as_real_warn(i, lenv)?;
        if sna.is_nan() || sna <= -1.0 || sna >= XLEN_MAX + 1.0 {
            return Err(bad());
        }
        sna as i64 as usize
    } else {
        let k = as_integer(lenv);
        if k == NA_INTEGER || k < 0 {
            return Err(bad());
        }
        k as usize
    };
    if s1.is_null() && na > 0 {
        return Err(Flow::error("cannot replicate NULL to a non-zero length"));
    }
    let ns = s1.len();
    if ns == 0 {
        if na > 0 {
            return Ok(xlengthgets(s1, na));
        }
        return Ok(s1.clone());
    }
    let idx: Vec<usize> = (0..na).map(|k| k % ns).collect();
    let mut a = gather_all(s1, &idx);
    factor_class_levels(s1, &mut a);
    Ok(a)
}

// R: seq.c:do_sequence
fn do_sequence(_i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    check_arity_internal(&args, 3, "sequence", call)?;
    let (lengths, from, by) = (&args[0].value, &args[1].value, &args[2].value);
    let Value::Int(lengths) = lengths else { return Err(Flow::error("'lengths' is not of mode integer")) };
    let Value::Int(from) = from else { return Err(Flow::error("'from' is not of mode integer")) };
    let Value::Int(by) = by else { return Err(Flow::error("'by' is not of mode integer")) };
    let (nl, nf, nb) = (lengths.data.len(), from.data.len(), by.data.len());
    if nl != 0 {
        if nf == 0 {
            return Err(Flow::error("'from' has length 0, but not 'lengths'"));
        }
        if nb == 0 {
            return Err(Flow::error("'by' has length 0, but not 'lengths'"));
        }
    }
    for &l in &lengths.data {
        if l == NA_INTEGER || l < 0 {
            return Err(Flow::error("'lengths' must be a vector of non-negative integers"));
        }
    }
    let mut out = Vec::new();
    for (k, &l) in lengths.data.iter().enumerate() {
        let f = from.data[k % nf.max(1)];
        if l != 0 && f == NA_INTEGER {
            return Err(Flow::error("'from' contains NAs"));
        }
        let b = by.data[k % nb.max(1)];
        if l >= 2 && b == NA_INTEGER {
            return Err(Flow::error("'by' contains NAs"));
        }
        let mut j = f;
        for _ in 0..l {
            out.push(j);
            j = j.wrapping_add(b);
        }
    }
    Ok(Value::ints(out))
}
