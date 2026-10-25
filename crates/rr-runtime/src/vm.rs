//! Executor for [`crate::compile::Code`].
//!
//! Registers hold either boxed R values or unboxed scalars (double,
//! integer, logical). Arithmetic and comparison on attribute-free scalars
//! never allocate; assigning a scalar to a variable whose current value is
//! an unshared scalar of the same type overwrites it in place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::arith::{int_to_real, myfmod, r_pow, real_op, ArithOp, RelOp};
use crate::compile::{compile_body, compile_loop, Code, Op};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

#[derive(Clone)]
pub enum RVal {
    V(Value),
    D(f64),
    I(i32),
    L(Lgl),
}

impl RVal {
    fn into_value(self) -> Value {
        match self {
            RVal::V(v) => v,
            RVal::D(x) => Value::real(x),
            RVal::I(x) => Value::int(x),
            RVal::L(x) => Value::lgl_raw(x),
        }
    }

    fn to_value(&self) -> Value {
        self.clone().into_value()
    }
}

#[derive(Clone, Copy)]
enum Num {
    D(f64),
    I(i32),
    L(i32),
}

#[inline]
fn scalar(r: &RVal) -> Option<Num> {
    match r {
        RVal::D(x) => Some(Num::D(*x)),
        RVal::I(x) => Some(Num::I(*x)),
        RVal::L(x) => Some(Num::L(*x)),
        RVal::V(Value::Real(v)) if v.data.len() == 1 && v.attrs.is_none() => Some(Num::D(v.data[0])),
        RVal::V(Value::Int(v)) if v.data.len() == 1 && v.attrs.is_none() => Some(Num::I(v.data[0])),
        RVal::V(Value::Lgl(v)) if v.data.len() == 1 && v.attrs.is_none() => Some(Num::L(v.data[0])),
        _ => None,
    }
}

fn as_f64(n: Num) -> f64 {
    match n {
        Num::D(x) => x,
        Num::I(x) | Num::L(x) => int_to_real(x),
    }
}

fn as_i32(n: Num) -> i32 {
    match n {
        Num::I(x) | Num::L(x) => x,
        Num::D(_) => unreachable!(),
    }
}

/// Scalar arithmetic; `None` means "use the general builtin" (e.g. on
/// integer overflow, which needs a warning).
#[inline]
fn arith_fast(op: ArithOp, a: Num, b: Num) -> Option<RVal> {
    if matches!(a, Num::D(_)) || matches!(b, Num::D(_)) {
        return Some(RVal::D(real_op(op, as_f64(a), as_f64(b))));
    }
    let (x, y) = (as_i32(a), as_i32(b));
    Some(match op {
        ArithOp::Plus | ArithOp::Minus | ArithOp::Times => {
            if x == NA_INTEGER || y == NA_INTEGER {
                RVal::I(NA_INTEGER)
            } else {
                let r = match op {
                    ArithOp::Plus => x as i64 + y as i64,
                    ArithOp::Minus => x as i64 - y as i64,
                    _ => x as i64 * y as i64,
                };
                if r > i32::MAX as i64 || r <= i32::MIN as i64 {
                    return None;
                }
                RVal::I(r as i32)
            }
        }
        ArithOp::Div => RVal::D(if x == NA_INTEGER || y == NA_INTEGER { NA_REAL } else { x as f64 / y as f64 }),
        ArithOp::Pow => RVal::D(if x == 1 || y == 0 {
            1.0
        } else if x == NA_INTEGER || y == NA_INTEGER {
            NA_REAL
        } else {
            r_pow(x as f64, y as f64)
        }),
        ArithOp::Mod => RVal::I(if x == NA_INTEGER || y == NA_INTEGER || y == 0 {
            NA_INTEGER
        } else if x >= 0 && y > 0 {
            x % y
        } else {
            myfmod(x as f64, y as f64) as i32
        }),
        ArithOp::IDiv => RVal::I(if x == NA_INTEGER || y == NA_INTEGER || y == 0 {
            NA_INTEGER
        } else {
            (x as f64 / y as f64).floor() as i32
        }),
    })
}

#[inline]
fn rel_fast(op: RelOp, a: Num, b: Num) -> RVal {
    if matches!(a, Num::D(_)) || matches!(b, Num::D(_)) {
        let (x, y) = (as_f64(a), as_f64(b));
        return RVal::L(if x.is_nan() || y.is_nan() { NA_LOGICAL } else { op.test(x, y) as Lgl });
    }
    let (x, y) = (as_i32(a), as_i32(b));
    RVal::L(if x == NA_INTEGER || y == NA_INTEGER { NA_LOGICAL } else { op.test(x, y) as Lgl })
}

thread_local! {
    static CACHE: RefCell<HashMap<usize, (Value, Rc<Code>), crate::env::FastHash>> = RefCell::new(HashMap::default());
    static ENABLED: bool = std::env::var("RR_COMPILE").map(|v| v != "0").unwrap_or(false);
}

pub fn enabled() -> bool {
    ENABLED.with(|e| *e)
}

/// Compiled code for a closure body or loop, compiled on first use.
pub fn code_for(e: &Value, body: bool) -> Option<Rc<Code>> {
    let Value::Lang(l) = e else { return None };
    let key = Rc::as_ptr(l) as usize;
    if let Some(c) = CACHE.with(|c| c.borrow().get(&key).map(|(_, code)| code.clone())) {
        return Some(c);
    }
    let code = Rc::new(if body { compile_body(e) } else { compile_loop(e)? });
    // Keep the expression alive so its address is not reused.
    CACHE.with(|c| c.borrow_mut().insert(key, (e.clone(), code.clone())));
    Some(code)
}

fn signal(interp: &mut Interp, r: RResult<Value>) -> RResult<Value> {
    match r {
        Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
        r => r,
    }
}

/// Assign a register value to `sym` in `env`, in place when possible.
fn set_var(interp: &mut Interp, env: &Env, sym: Sym, val: &RVal) -> RResult<()> {
    {
        let mut frame = env.0.frame.borrow_mut();
        if let Some(b) = frame.get_mut(sym) {
            if b.flags == 0 {
                match (&mut b.value, val) {
                    (Value::Real(rc), RVal::D(x)) => {
                        if let Some(v) = Rc::get_mut(rc) {
                            if v.data.len() == 1 && v.attrs.is_none() {
                                v.data[0] = *x;
                                return Ok(());
                            }
                        }
                    }
                    (Value::Int(rc), RVal::I(x)) => {
                        if let Some(v) = Rc::get_mut(rc) {
                            if v.data.len() == 1 && v.attrs.is_none() {
                                v.data[0] = *x;
                                return Ok(());
                            }
                        }
                    }
                    (Value::Lgl(rc), RVal::L(x)) => {
                        if let Some(v) = Rc::get_mut(rc) {
                            if v.data.len() == 1 && v.attrs.is_none() {
                                v.data[0] = *x;
                                return Ok(());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let r = interp.define(sym, val.to_value(), env);
    signal(interp, r.map(|_| Value::Null)).map(|_| ())
}

/// Call the base builtin named by the call's head with boxed arguments.
fn call_builtin(interp: &mut Interp, call: &Value, args: Vec<RVal>, env: &Env) -> RResult<Value> {
    let Value::Lang(l) = call else { unreachable!() };
    let sym = l.fun_sym().unwrap();
    let f = interp.base.get_local(sym);
    let Some(Value::Builtin(b)) = f else {
        return signal(interp, Err(Flow::error("internal error: builtin not found")));
    };
    let argv: Vec<Arg> = args.into_iter().map(|r| Arg::pos(r.into_value())).collect();
    let r = interp.invoke_builtin(b, call, argv, env);
    signal(interp, r)
}

pub fn run(interp: &mut Interp, code: &Code, env: &Env) -> RResult<Value> {
    interp.visible = true;

    let mut regs: Vec<RVal> = vec![RVal::V(Value::Null); code.nregs];
    // (break target, next target) of active loops.
    let mut loops: Vec<(u32, u32)> = Vec::new();
    // Guard results memoized for this run, valid while neither the search
    // path nor the frame's function bindings change. Only used when the
    // frame's parent is on the search path (the common case).
    let cacheable = code.has_guards && (env.is_search() || env.parent().is_some_and(|p| p.is_search()));
    let mut guards: Vec<(u64, u32, bool)> = if cacheable { vec![(u64::MAX, 0, false); code.ops.len()] } else { Vec::new() };
    let ops = &code.ops;
    let mut pc: usize = 0;
    macro_rules! tryop {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(Flow::Break(e)) if e.ptr_eq(env) && !loops.is_empty() => {
                    pc = loops.last().unwrap().0 as usize;
                    continue;
                }
                Err(Flow::Next(e)) if e.ptr_eq(env) && !loops.is_empty() => {
                    pc = loops.last().unwrap().1 as usize;
                    continue;
                }
                Err(f) => return Err(f),
            }
        };
    }
    while pc < ops.len() {
        let op = &ops[pc];
        pc += 1;
        match op {
            Op::Const { dst, k } => regs[*dst as usize] = RVal::V(code.consts[*k as usize].clone()),
            Op::Null { dst } => regs[*dst as usize] = RVal::V(Value::Null),
            Op::Visible { on } => interp.visible = *on,
            Op::GetVar { dst, sym } => {
                let r = interp.eval_sym(*sym, env);
                let v = tryop!(signal(interp, r));
                interp.visible = true;
                regs[*dst as usize] = RVal::V(v);
            }
            Op::SetVar { src, sym } => {
                let v = std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null));
                tryop!(set_var(interp, env, *sym, &v));
                regs[*src as usize] = v;
            }
            Op::Arith { op, dst, a, b, call } => {
                let fast = match (scalar(&regs[*a as usize]), scalar(&regs[*b as usize])) {
                    (Some(x), Some(y)) => arith_fast(*op, x, y),
                    _ => None,
                };
                regs[*dst as usize] = match fast {
                    Some(v) => {
                        // Drop operand references so bindings stay unshared.
                        regs[*a as usize] = RVal::I(0);
                        regs[*b as usize] = RVal::I(0);
                        v
                    }
                    None => {
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        let y = std::mem::replace(&mut regs[*b as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x, y], env)))
                    }
                };
            }
            Op::Rel { op, dst, a, b, call } => {
                let fast = match (scalar(&regs[*a as usize]), scalar(&regs[*b as usize])) {
                    (Some(x), Some(y)) => Some(rel_fast(*op, x, y)),
                    _ => None,
                };
                regs[*dst as usize] = match fast {
                    Some(v) => {
                        regs[*a as usize] = RVal::I(0);
                        regs[*b as usize] = RVal::I(0);
                        v
                    }
                    None => {
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        let y = std::mem::replace(&mut regs[*b as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x, y], env)))
                    }
                };
            }
            Op::Neg { dst, a, call } => {
                regs[*dst as usize] = match scalar(&regs[*a as usize]) {
                    Some(Num::D(x)) => RVal::D(-x),
                    Some(Num::I(x)) | Some(Num::L(x)) => RVal::I(if x == NA_INTEGER { x } else { -x }),
                    None => {
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x], env)))
                    }
                };
            }
            Op::Not { dst, a, call } => {
                regs[*dst as usize] = match scalar(&regs[*a as usize]) {
                    Some(Num::D(x)) => RVal::L(if x.is_nan() { NA_LOGICAL } else { (x == 0.0) as Lgl }),
                    Some(Num::I(x)) | Some(Num::L(x)) => RVal::L(if x == NA_INTEGER { NA_LOGICAL } else { (x == 0) as Lgl }),
                    None => {
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x], env)))
                    }
                };
            }
            Op::JumpIfNot { cond, target, call } => {
                let truth = match &regs[*cond as usize] {
                    RVal::L(x) if *x != NA_LOGICAL => *x != 0,
                    other => {
                        let v = other.to_value();
                        let c = code.consts[*call as usize].clone();
                        let r = crate::coerce::as_condition(interp, &v, &c).map(|b| Value::lgl(b));
                        let v = tryop!(signal(interp, r));
                        crate::coerce::as_logical(&v) != 0
                    }
                };
                if !truth {
                    pc = *target as usize;
                }
            }
            Op::Jump { target } => pc = *target as usize,
            Op::Guard { sym, dst, k, skip } => {
                let ok = if cacheable {
                    let (epoch, ver) = (crate::env::search_epoch(), env.fn_version());
                    let slot = &mut guards[pc - 1];
                    if slot.0 == epoch && slot.1 == ver {
                        slot.2
                    } else {
                        let r = interp.resolves_to_base(*sym, env);
                        guards[pc - 1] = (epoch, ver, r);
                        r
                    }
                } else {
                    interp.resolves_to_base(*sym, env)
                };
                if !ok {
                    let v = tryop!(interp.eval(&code.consts[*k as usize], env));
                    regs[*dst as usize] = RVal::V(v);
                    pc = *skip as usize;
                }
            }
            Op::Eval { dst, k } => {
                let v = tryop!(interp.eval(&code.consts[*k as usize], env));
                regs[*dst as usize] = RVal::V(v);
            }
            Op::Return { src } => {
                let v = std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null)).into_value();
                if code.is_body {
                    return Ok(v);
                }
                return Err(Flow::Return(v, env.clone()));
            }
            Op::LoopEnter { brk, nxt } => loops.push((*brk, *nxt)),
            Op::LoopExit => {
                loops.pop();
            }
            Op::ForInit { seq, call } => {
                let mut v = regs[*seq as usize].to_value();
                if v.inherits("factor") {
                    v = crate::builtins::envir::as_character_factor(&v);
                }
                if !(v.is_vector() || v.is_null() || matches!(v, Value::Pairlist(_))) {
                    let c = code.consts[*call as usize].clone();
                    return signal(interp, Err(Flow::error_call(c, "invalid for() loop sequence")));
                }
                regs[*seq as usize] = RVal::V(v);
                regs[*seq as usize + 1] = RVal::I(0);
            }
            Op::ForStep { seq, idx, var, exit, .. } => {
                let i = match regs[*idx as usize] {
                    RVal::I(i) => i as usize,
                    _ => 0,
                };
                if i == 0 {
                    // R binds the loop variable to NULL before iterating.
                    tryop!(set_var(interp, env, *var, &RVal::V(Value::Null)));
                }
                let RVal::V(sv) = &regs[*seq as usize] else { unreachable!() };
                if i >= sv.len() {
                    pc = *exit as usize;
                    continue;
                }
                let elt = match sv {
                    Value::Int(x) => RVal::I(x.data[i]),
                    Value::Real(x) => RVal::D(x.data[i]),
                    Value::Lgl(x) => RVal::L(x.data[i]),
                    Value::Str(x) => RVal::V(Value::strs(vec![x.data[i].clone()])),
                    Value::Cplx(x) => RVal::V(Value::cplx(x.data[i])),
                    Value::Raw(x) => RVal::V(Value::raws(vec![x.data[i]])),
                    Value::List(x) | Value::Expr(x) => RVal::V(x.data[i].clone()),
                    Value::Pairlist(p) => RVal::V(p.items[i].value.clone()),
                    _ => unreachable!(),
                };
                regs[*idx as usize] = RVal::I(i as i32 + 1);
                tryop!(set_var(interp, env, *var, &elt));
            }
        }
    }
    Ok(std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null)).into_value())
}
