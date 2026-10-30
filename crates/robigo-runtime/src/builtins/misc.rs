//! Assorted internals: complex numbers (complex.c), raw vectors and bit
//! operations (raw.c, relop.c bitwise), message translation (errors.c),
//! debugging and memory (debug.c, memory.c), version and quit (version.c,
//! main.c), tail calls and declarations (eval.c).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use super::{arg, check_arity, internal, prim, special};
use crate::coerce::{CoerceWarnings, as_integer, as_logical};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    internal("complex", do_complex, 3),
    prim("Re", do_re, 1),
    prim("Im", do_im, 1),
    prim("Mod", do_mod, 1),
    prim("Arg", do_arg, 1),
    prim("Conj", do_conj, 1),
    internal("polyroot", do_polyroot, 1),
    internal("rawShift", do_raw_shift, 2),
    internal("intToBits", do_int_to_bits, 1),
    internal("rawToBits", do_raw_to_bits, 1),
    internal("packBits", do_pack_bits, 2),
    internal("numToBits", do_num_to_bits, 1),
    internal("numToInts", do_num_to_ints, 1),
    internal("bitwiseAnd", do_bitwise_and, 2),
    internal("bitwiseOr", do_bitwise_or, 2),
    internal("bitwiseXor", do_bitwise_xor, 2),
    internal("bitwiseNot", do_bitwise_not, 1),
    internal("bitwiseShiftL", do_bitwise_shiftl, 2),
    internal("bitwiseShiftR", do_bitwise_shiftr, 2),
    internal("gettext", do_gettext, 3),
    internal("ngettext", do_ngettext, 4),
    internal("bindtextdomain", do_null, 2),
    internal("traceback", do_null1, 1),
    internal("debug", do_debug, 3).invisible(),
    internal("undebug", do_undebug, 1).invisible(),
    internal("isdebugged", do_isdebugged, 1),
    internal("debugonce", do_debug, 3).invisible(),
    prim(".primTrace", do_identity, 1).invisible(),
    prim(".primUntrace", do_identity, 1).invisible(),
    prim("browser", do_null_any, -1).invisible(),
    internal("browserText", do_null_any, 1),
    internal("browserCondition", do_null_any, 1),
    internal("browserSetDebug", do_null_any, 1).invisible(),
    internal("gc", do_gc, 3),
    prim("gc.time", do_gc_time, -1),
    internal("gcinfo", do_false, 1),
    internal("gctorture", do_false, 1).invisible(),
    internal("gctorture2", do_zero, 3),
    internal("mem.maxVSize", do_inf, 1),
    internal("mem.maxNSize", do_inf, 1),
    internal("reg.finalizer", do_reg_finalizer, 3),
    internal("Cstack_info", do_cstack_info, 0),
    internal("Version", do_version, 0),
    internal("internalsID", do_internals_id, 0),
    internal("quit", do_quit, 3).invisible(),
    special("Exec", do_exec, -1).custom_vis(),
    special("Tailcall", do_tailcall, -1).custom_vis(),
    special("declare", do_declare, -1).invisible(),
    internal("returnValue", do_return_value, 1),
    internal("asCharacterFactor", do_as_character_factor, 1),
    internal("interruptsSuspended", do_false_any, -1),
    internal("activeBindingFunction", do_active_binding_function, 2),
    internal("env.profile", do_null1, 1),
    internal("enableJIT", do_zero1, 1),
    internal("compilePKGS", do_false, 1),
    internal("is.builtin.internal", do_is_builtin_internal, 1),
    internal("setNumMathThreads", do_one, 1),
    internal("setMaxNumMathThreads", do_one, 1),
    internal("makeLazy", do_make_lazy, 5).invisible(),
    // options.c's getOption; `options()` itself lives with the I/O builtins.
    internal("getOption", do_get_option, 1),
];

fn do_get_option(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => s.data[0].clone().unwrap(),
        _ => return Err(Flow::error("'x' must be a character string")),
    };
    Ok(i.get_option(&name).unwrap_or(Value::Null))
}

// ----------------------------------------------------------------------
// complex.c

fn do_complex(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER || n < 0 {
        return Err(Flow::error("invalid length"));
    }
    // R: complex.c:do_complex — coerceVector of real/imaginary to real
    // (a complex imaginary warns about the discarded imaginary part).
    let re = match i.coerce(&arg(&args, 1), VType::Real)? {
        Value::Real(v) => v.data.clone(),
        _ => vec![],
    };
    let im = match i.coerce(&arg(&args, 2), VType::Real)? {
        Value::Real(v) => v.data.clone(),
        _ => vec![],
    };
    let n = (n as usize).max(re.len()).max(im.len());
    let out: Vec<Complex> = (0..n)
        .map(|k| {
            Complex::new(
                if re.is_empty() { 0.0 } else { re[k % re.len()] },
                if im.is_empty() { 0.0 } else { im[k % im.len()] },
            )
        })
        .collect();
    Ok(Value::cplxs(out))
}

fn cmath(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env, name: &str, which: u8) -> RResult<Value> {
    check_arity(&args, 1, name, call)?;
    if args[0].value.is_object() {
        if let Some(r) = i.try_dispatch_group("Complex", name, call, &args, env)? {
            return Ok(r);
        }
    }
    let x = &args[0].value;
    let mut out = match x {
        Value::Cplx(v) => {
            if which == 5 {
                Value::cplxs(v.data.iter().map(|z| Complex::new(z.re, -z.im)).collect())
            } else {
                Value::reals(
                    v.data
                        .iter()
                        .map(|z| match which {
                            1 => z.re,
                            2 => z.im,
                            3 => z.re.hypot(z.im),
                            _ => z.im.atan2(z.re),
                        })
                        .collect(),
                )
            }
        }
        Value::Lgl(_) | Value::Int(_) | Value::Real(_) => {
            let mut w = CoerceWarnings::default();
            let r = crate::coerce::to_real(x, &mut w);
            match which {
                5 => {
                    let mut v = x.clone();
                    if matches!(v, Value::Lgl(_)) {
                        v = crate::coerce::coerce_quiet(&v, VType::Int);
                    }
                    return Ok(v);
                }
                1 => Value::reals(r),
                2 => Value::reals(r.iter().map(|d| if d.is_nan() { *d } else { 0.0 }).collect()),
                3 => Value::reals(r.iter().map(|d| d.abs()).collect()),
                _ => Value::reals(
                    r.iter()
                        .map(|d| {
                            if d.is_nan() {
                                *d
                            } else if *d >= 0.0 {
                                0.0
                            } else {
                                std::f64::consts::PI
                            }
                        })
                        .collect(),
                ),
            }
        }
        _ => return Err(Flow::error_call(call.clone(), "non-numeric argument to function")),
    };
    // Keep names, dims and dimnames.
    for a in [s::NAMES, s::DIM, s::DIMNAMES] {
        if let Some(v) = x.get_attr(a) {
            out.set_attr_raw(a, v);
        }
    }
    Ok(out)
}

fn do_re(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    cmath(i, c, a, e, "Re", 1)
}
fn do_im(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    cmath(i, c, a, e, "Im", 2)
}
fn do_mod(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    cmath(i, c, a, e, "Mod", 3)
}
fn do_arg(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    cmath(i, c, a, e, "Arg", 4)
}
fn do_conj(i: &mut Interp, c: &Value, a: Vec<Arg>, e: &Env) -> RResult<Value> {
    cmath(i, c, a, e, "Conj", 5)
}

fn do_polyroot(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Err(Flow::error("robigo: 'polyroot' is not implemented yet"))
}

// ----------------------------------------------------------------------
// raw.c

fn raw_arg(v: &Value, what: &str) -> RResult<Vec<u8>> {
    match v {
        Value::Raw(r) => Ok(r.data.clone()),
        _ => Err(Flow::error(format!("argument '{}' must be a raw vector", what))),
    }
}

fn do_raw_shift(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = raw_arg(&arg(&args, 0), "x")?;
    let n = as_integer(&arg(&args, 1));
    if n == NA_INTEGER || !(-8..=8).contains(&n) {
        return Err(Flow::error("argument 'n' must be a small integer"));
    }
    // R: raw.c:do_rawShift — the shift happens on the promoted int and
    // is truncated back to Rbyte on assignment, so shift 8 zeroes the
    // byte (Rust's u8 shift would mask the count instead).
    Ok(Value::raws(
        x.iter().map(|&b| if n >= 0 { ((b as u32) << n) as u8 } else { ((b as u32) >> -n) as u8 }).collect(),
    ))
}

fn do_int_to_bits(i: &mut Interp, _call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: raw.c:do_intToBits — coerceVector to integer (with its coercion
    // warnings and errors) then the low 32 bits of each element.
    let x = i.coerce(&arg(&args, 0), VType::Int)?;
    let x = match &x {
        Value::Int(v) => &v.data,
        _ => unreachable!("coerce to integer yields an integer vector"),
    };
    let mut out = Vec::with_capacity(32 * x.len());
    for v in x {
        let u = *v as u32;
        for k in 0..32 {
            out.push(((u >> k) & 1) as u8);
        }
    }
    Ok(Value::raws(out))
}

fn do_raw_to_bits(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = raw_arg(&arg(&args, 0), "x")?;
    let mut out = Vec::with_capacity(8 * x.len());
    for b in x {
        for k in 0..8 {
            out.push((b >> k) & 1);
        }
    }
    Ok(Value::raws(out))
}

fn do_num_to_bits(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Value::Real(x) = arg(&args, 0) else { return Err(Flow::error("argument 'x' must be a numeric vector")) };
    let mut out = Vec::with_capacity(64 * x.data.len());
    for v in &x.data {
        let u = v.to_bits();
        for k in 0..64 {
            out.push(((u >> k) & 1) as u8);
        }
    }
    Ok(Value::raws(out))
}

fn do_num_to_ints(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Value::Real(x) = arg(&args, 0) else { return Err(Flow::error("argument 'x' must be a numeric vector")) };
    let mut out = Vec::with_capacity(2 * x.data.len());
    for v in &x.data {
        let u = v.to_bits();
        out.push(u as u32 as i32);
        out.push((u >> 32) as u32 as i32);
    }
    Ok(Value::ints(out))
}

fn do_pack_bits(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    let t = crate::coerce::as_string(&arg(&args, 1)).map(|s| s.to_string()).unwrap_or_default();
    let bits: Vec<u8> = match &x {
        Value::Raw(r) => r.data.iter().map(|b| b & 1).collect(),
        Value::Lgl(v) | Value::Int(v) => {
            if v.data.contains(&NA_INTEGER) {
                return Err(Flow::error("argument 'x' must not contain NAs"));
            }
            v.data.iter().map(|b| (b & 1) as u8).collect()
        }
        _ => return Err(Flow::error("argument 'x' must be raw, integer or logical")),
    };
    let width = match t.as_str() {
        "raw" => 8,
        "integer" => 32,
        "double" => 64,
        _ => return Err(Flow::error("invalid 'type' argument")),
    };
    if bits.len() % width != 0 {
        return Err(Flow::error(format!("argument 'x' must be a multiple of {} long", width)));
    }
    Ok(match width {
        8 => Value::raws(bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (k, b)| a | (b << k))).collect()),
        32 => Value::ints(
            bits.chunks(32)
                .map(|c| c.iter().enumerate().fold(0u32, |a, (k, b)| a | ((*b as u32) << k)) as i32)
                .collect(),
        ),
        _ => Value::reals(
            bits.chunks(64)
                .map(|c| f64::from_bits(c.iter().enumerate().fold(0u64, |a, (k, b)| a | ((*b as u64) << k))))
                .collect(),
        ),
    })
}

fn bitwise2(args: &[Arg], f: fn(i32, i32) -> Option<i32>) -> RResult<Value> {
    let a = arg(args, 0);
    let b = arg(args, 1);
    let mut w = CoerceWarnings::default();
    let x = crate::coerce::to_int(&a, &mut w);
    let y = crate::coerce::to_int(&b, &mut w);
    if !(a.is_numeric_type() || matches!(a, Value::Raw(_))) || !(b.is_numeric_type() || matches!(b, Value::Raw(_))) {
        return Err(Flow::error("'a' and 'b' must have the same type"));
    }
    let n = if x.is_empty() || y.is_empty() { 0 } else { x.len().max(y.len()) };
    let out: Vec<i32> = (0..n)
        .map(|k| {
            let (p, q) = (x[k % x.len()], y[k % y.len()]);
            if p == NA_INTEGER || q == NA_INTEGER { NA_INTEGER } else { f(p, q).unwrap_or(NA_INTEGER) }
        })
        .collect();
    Ok(Value::ints(out))
}

fn do_bitwise_and(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    bitwise2(&args, |a, b| Some(a & b))
}
fn do_bitwise_or(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    bitwise2(&args, |a, b| Some(a | b))
}
fn do_bitwise_xor(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    bitwise2(&args, |a, b| Some(a ^ b))
}
fn do_bitwise_shiftl(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    bitwise2(&args, |a, b| if b > 31 { None } else { Some(((a as u32) << b) as i32) })
}
fn do_bitwise_shiftr(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    bitwise2(&args, |a, b| if b > 31 { None } else { Some(((a as u32) >> b) as i32) })
}
fn do_bitwise_not(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut w = CoerceWarnings::default();
    let x = crate::coerce::to_int(&arg(&args, 0), &mut w);
    Ok(Value::ints(x.into_iter().map(|v| if v == NA_INTEGER { v } else { !v }).collect()))
}

// ----------------------------------------------------------------------
// messages (no translations: the C locale)

fn do_gettext(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let msg = arg(&args, 1);
    Ok(match msg {
        Value::Str(_) => msg,
        Value::Null => Value::strs(vec![]),
        _ => return Err(Flow::error("invalid 'string' value")),
    })
}

fn do_ngettext(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER || n < 0 {
        return Err(Flow::error("invalid 'n'"));
    }
    let (m1, m2) = (arg(&args, 1), arg(&args, 2));
    for (v, w) in [(&m1, "msg1"), (&m2, "msg2")] {
        if !matches!(v, Value::Str(s) if s.data.len() == 1) {
            return Err(Flow::error(format!("'{}' must be a character string", w)));
        }
    }
    Ok(if n == 1 { m1 } else { m2 })
}

// ----------------------------------------------------------------------
// debug.c / memory.c

thread_local! {
    static DEBUGGED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

fn closure_key(v: &Value) -> Option<usize> {
    match v {
        Value::Closure(c) => Some(Rc::as_ptr(c) as usize),
        _ => None,
    }
}

fn do_debug(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match closure_key(&arg(&args, 0)) {
        Some(k) => DEBUGGED.with(|d| d.borrow_mut().insert(k)),
        None => return Err(Flow::error("argument must be a function")),
    };
    Ok(Value::Null)
}

fn do_undebug(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if let Some(k) = closure_key(&arg(&args, 0)) {
        let was = DEBUGGED.with(|d| d.borrow_mut().remove(&k));
        if !was {
            i.warning("argument is not being debugged")?;
        }
    }
    Ok(Value::Null)
}

fn do_isdebugged(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let k = closure_key(&arg(&args, 0));
    Ok(Value::lgl(k.is_some_and(|k| DEBUGGED.with(|d| d.borrow().contains(&k)))))
}

/// robigo does not track allocations by R's cell classes; report NA rather
/// than invented numbers.
fn do_gc(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    // Implementation caches are weak in R: they must not retain otherwise
    // unreachable environments through closure bodies or call-site memos.
    // the C API's heap collects, sweeps unreachable runtime cycles and runs
    // pending finalizers (docs/core.md §3)
    i.fun_cache.clear();
    crate::vm::clear_gc_caches();
    if let Some(b) = crate::native::bridge() {
        b.gc(i);
    }
    Ok(Value::reals(vec![NA_REAL; 14]))
}

fn do_gc_time(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::reals(vec![0.0; 5]))
}

// R: memory.c:do_regFinaliz
fn do_reg_finalizer(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let e = arg(&args, 0);
    if !matches!(e, Value::Env(_) | Value::ExtPtr(_))
        || crate::native::bridge().is_some_and(|bridge| !bridge.is_finalizer_key(&e))
    {
        return Err(Flow::error("first argument must be environment or external pointer"));
    }
    if !matches!(arg(&args, 1), Value::Closure(_)) {
        return Err(Flow::error("second argument must be a function"));
    }
    let onexit = as_logical(&arg(&args, 2));
    if onexit == NA_LOGICAL {
        return Err(Flow::error("third argument must be 'TRUE' or 'FALSE'"));
    }
    let bridge = crate::native::bridge()
        .ok_or_else(|| Flow::error("robigo: environment finalizers require the native bridge"))?;
    bridge.register_finalizer(i, &e, &arg(&args, 1), onexit != 0)?;
    Ok(Value::Null)
}

fn do_cstack_info(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut v = Value::ints(vec![NA_INTEGER, NA_INTEGER, 1, NA_INTEGER]);
    v.set_attr_raw(s::NAMES, Value::strs_from(["size", "current", "direction", "eval_depth"]));
    Ok(v)
}

// ----------------------------------------------------------------------
// version.c / main.c

fn do_version(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let fields: Vec<(&str, &str)> = vec![
        ("platform", "x86_64-pc-linux-gnu"),
        ("arch", "x86_64"),
        ("os", "linux-gnu"),
        ("system", "x86_64, linux-gnu"),
        ("status", ""),
        ("major", "4"),
        ("minor", "5.0"),
        ("year", "2025"),
        ("month", "04"),
        ("day", "11"),
        ("svn rev", "88135"),
        ("language", "R"),
        ("version.string", "R version 4.5.0 (2025-04-11)"),
        ("nickname", "How About a Twenty-Six"),
    ];
    Ok(Value::named_list(fields.into_iter().map(|(k, v)| (k, Value::str(v))).collect()))
}

/// GNU R 4.5.0's `R_INTERNALS_UUID` (Defn.h). loadNamespace refuses
/// packages whose compiled code was built against other internals. robigo
/// never runs packages' compiled code (their native routines are ported),
/// so the packages it can use are those installed for R 4.5.0.
fn do_internals_id(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::str("2fdf6c18-697a-4ba7-b8ef-11c0d92f1327"))
}

fn do_quit(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let status = as_integer(&arg(&args, 1));
    i.flush();
    // R: unix/sys-std.c:R_CleanUp -- exit finalizers run before exit.
    if let Some(b) = crate::native::bridge() {
        b.run_exit_finalizers(i);
    }
    i.flush();
    std::process::exit(if status == NA_INTEGER { 0 } else { status });
}

// ----------------------------------------------------------------------
// eval.c odds and ends

fn do_exec(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let Some(first) = args.first() else { return Err(Flow::error_call(call.clone(), "'expr' is missing")) };
    let e = i.eval(&first.value, env)?;
    let target = match args.get(1) {
        Some(a) => i.eval(&a.value, env)?,
        None => Value::Env(env.clone()),
    };
    let Value::Env(tenv) = target else { return Err(Flow::error_call(call.clone(), "invalid environment")) };
    let v = i.eval(&e, &tenv)?;
    Err(Flow::Return(v, env.clone()))
}

fn do_tailcall(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let Some(first) = args.first() else { return Err(Flow::error_call(call.clone(), "'FUN' is missing")) };
    let fun = match &first.value {
        Value::Str(s) if s.data.len() == 1 => Value::Sym(Sym::new(s.data[0].as_deref().unwrap_or(""))),
        other => other.clone(),
    };
    let c = Value::lang(fun, args[1..].to_vec());
    let v = i.eval(&c, env)?;
    Err(Flow::Return(v, env.clone()))
}

fn do_declare(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}

fn do_return_value(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(arg(&args, 0))
}

fn do_as_character_factor(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    crate::builtins::envir::as_character_factor(&x)
}

fn do_active_binding_function(_: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = match arg(&args, 0) {
        Value::Sym(s) => s,
        Value::Str(v) if v.data.len() == 1 => Sym::new(v.data[0].as_deref().unwrap_or("")),
        _ => return Err(Flow::error_call(call.clone(), "not a symbol")),
    };
    let Value::Env(e) = arg(&args, 1) else { return Err(Flow::error_call(call.clone(), "not an environment")) };
    match e.binding_flags(sym) {
        Some(f) if f & crate::env::BIND_ACTIVE != 0 => Ok(e.get_local(sym).unwrap()),
        Some(_) => Err(Flow::error_call(call.clone(), format!("no active binding for \"{}\"", sym))),
        None => Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym))),
    }
}

fn do_is_builtin_internal(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Sym(s) => s.name().to_string(),
        _ => return Err(Flow::error("invalid symbol")),
    };
    Ok(Value::lgl(super::lookup_internal(&name).is_some_and(|b| b.kind == BuiltinKind::Builtin)))
}

fn do_make_lazy(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let vars = arg(&args, 0);
    let vals = crate::coerce::to_list(&arg(&args, 1));
    let expr = arg(&args, 2);
    let (Value::Env(eenv), Value::Env(aenv)) = (arg(&args, 3), arg(&args, 4)) else {
        return Err(Flow::error("invalid environment"));
    };
    let Value::Str(names) = vars else { return Err(Flow::error("invalid 'vars'")) };
    for (k, n) in names.data.iter().enumerate() {
        let e = match &expr {
            Value::Lang(l) => {
                let mut l2 = (**l).clone();
                if let Some(a) = l2.args.first_mut() {
                    a.value = vals.get(k).cloned().unwrap_or(Value::Null);
                }
                Value::Lang(Rc::new(l2))
            }
            other => other.clone(),
        };
        let p = Value::Promise(Rc::new(Promise::new(e, eenv.clone())));
        i.define(Sym::new(n.as_deref().unwrap_or("NA")), p, &aenv)?;
    }
    Ok(Value::Null)
}

// ----------------------------------------------------------------------
// trivial results

fn do_null(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}
fn do_null1(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}
fn do_null_any(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Null)
}
fn do_false(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(false))
}
fn do_false_any(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(false))
}
fn do_zero(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::int(0))
}
fn do_zero1(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::int(0))
}
fn do_one(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::int(1))
}
fn do_inf(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::real(f64::INFINITY))
}
fn do_identity(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(arg(&args, 0))
}
