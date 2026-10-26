//! Executor for [`crate::compile::Code`].
//!
//! Registers hold either boxed R values or unboxed scalars (double,
//! integer, logical). Arithmetic and comparison on attribute-free scalars
//! never allocate; assigning a scalar to a variable whose current value is
//! an unshared scalar of the same type overwrites it in place.
//!
//! Variables live in slots while the code runs: a read caches the value
//! and an assignment only updates the slot. Code that could observe or
//! change the frame — anything evaluated by the interpreter, a builtin
//! call, a signal, a promise or active binding — runs only after
//! [`Vars::flush`] has written pending assignments back to the
//! environment and dropped all cached values. Between such operations no
//! other code runs, so the frame cannot tell the difference.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::arith::{int_to_real, myfmod, r_pow, real_op, ArithOp, RelOp};
use crate::compile::{compile_body, compile_loop, Code, Leaf, Math1, Op, RangeKind, Slot};
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
    /// The integer sequence `start, start + step, ...` of length `n` a
    /// `for` loop steps through (only ever in a sequence register).
    Range { start: i64, step: i64, n: u64 },
}

impl RVal {
    fn into_value(self) -> Value {
        match self {
            RVal::V(v) => v,
            RVal::D(x) => Value::real(x),
            RVal::I(x) => Value::int(x),
            RVal::L(x) => Value::lgl_raw(x),
            RVal::Range { start, step, n } => Value::ints((0..n as i64).map(|k| (start + k * step) as i32).collect()),
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
    static ENABLED: bool = std::env::var("ROBIGO_COMPILE").map(|v| v != "0").unwrap_or(false);
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

enum SlotVal {
    Empty,
    /// The variable's current value, also bound in the environment.
    Clean(RVal),
    /// Assigned but not yet written to the environment.
    Dirty(RVal),
}

struct Vars {
    slots: Vec<SlotVal>,
    /// Whether any slot is non-empty / dirty.
    any: bool,
    dirty: bool,
}

impl Vars {
    /// Write pending assignments to `env` and forget cached values.
    fn flush(&mut self, interp: &mut Interp, env: &Env, syms: &[Sym]) -> RResult<()> {
        if !self.any {
            return Ok(());
        }
        let dirty = self.dirty;
        self.any = false;
        self.dirty = false;
        let mut res = Ok(());
        for (i, s) in self.slots.iter_mut().enumerate() {
            if let SlotVal::Dirty(v) = std::mem::replace(s, SlotVal::Empty) {
                if dirty && res.is_ok() {
                    res = set_var(interp, env, syms[i], &v);
                }
            }
        }
        res
    }

    fn get(&self, slot: usize) -> Option<&RVal> {
        match &self.slots[slot] {
            SlotVal::Clean(v) | SlotVal::Dirty(v) => Some(v),
            SlotVal::Empty => None,
        }
    }
}

/// The value of `sym` if reading it cannot run code: a plain binding, or
/// an argument whose promise has been forced.
fn plain_lookup(env: &Env, sym: Sym) -> Option<RVal> {
    let mut cur = env.clone();
    loop {
        let found = {
            let frame = cur.0.frame.borrow();
            frame.get(sym).map(|b| (b.value.clone(), b.flags))
        };
        if let Some((v, flags)) = found {
            if flags & crate::env::BIND_ACTIVE != 0 {
                return None;
            }
            let v = match v {
                Value::Promise(p) => p.value()?,
                v => v,
            };
            if v.is_missing_arg() {
                return None;
            }
            return Some(unbox(v));
        }
        cur = cur.parent()?;
    }
}

fn unbox(v: Value) -> RVal {
    match &v {
        Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::D(x.data[0]),
        Value::Int(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::I(x.data[0]),
        Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::L(x.data[0]),
        _ => RVal::V(v),
    }
}

/// Whether an assignment of `sym` in `env` may be deferred: the binding is
/// plain (not locked or active) or can be created, and the value is not a
/// function (function bindings affect lookups and guards).
fn deferrable(env: &Env, sym: Sym, val: &RVal, pending: bool) -> bool {
    if matches!(val, RVal::V(Value::Closure(_) | Value::Builtin(_))) {
        return false;
    }
    if pending {
        return true;
    }
    match env.binding_flags(sym) {
        Some(flags) => flags == 0,
        None => !env.0.locked.get(),
    }
}

/// The value of a variable without running code, cached in its slot.
fn read_var<'a>(vars: &'a mut Vars, env: &Env, sym: Sym, slot: Slot) -> Option<&'a RVal> {
    let slot = slot as usize;
    if vars.get(slot).is_none() {
        let v = plain_lookup(env, sym)?;
        vars.slots[slot] = SlotVal::Clean(v);
        vars.any = true;
    }
    vars.get(slot)
}

/// A 0-based index from an in-range positive scalar index.
fn scalar_index(vars: &mut Vars, env: &Env, regs: &[RVal], i: &Leaf) -> Option<f64> {
    let num = |r: &RVal| match r {
        RVal::D(x) => Some(*x),
        RVal::I(x) if *x != NA_INTEGER => Some(*x as f64),
        RVal::V(Value::Real(v)) if v.data.len() == 1 && v.attrs.is_none() => Some(v.data[0]),
        RVal::V(Value::Int(v)) if v.data.len() == 1 && v.attrs.is_none() && v.data[0] != NA_INTEGER => {
            Some(v.data[0] as f64)
        }
        _ => None,
    };
    let x = match i {
        Leaf::D(x) => *x,
        Leaf::I(x) => *x as f64,
        Leaf::Var(sym, slot) => num(read_var(vars, env, *sym, *slot)?)?,
        Leaf::R(r) => num(&regs[*r as usize])?,
    };
    // NaN fails both comparisons.
    (x >= 1.0 && x < 4503599627370496.0).then_some(x.trunc() - 1.0)
}

/// Element `i` of a vector that `x[i]` (`[[` if `two`) reads without
/// dispatch, attribute handling or allocation beyond a string scalar.
fn element(x: &RVal, i: usize, two: bool) -> Option<RVal> {
    let v = match x {
        RVal::V(v) => v,
        _ if i == 0 => return Some(x.clone()),
        _ => return None,
    };
    // `x[i]` keeps names and other attributes; `x[[i]]` drops them, but
    // classed objects dispatch.
    let plain = |attrs: &AttrBox| attrs.is_none() || (two && !v.is_object());
    Some(match v {
        Value::Real(x) if i < x.data.len() && plain(&x.attrs) => RVal::D(x.data[i]),
        Value::Int(x) if i < x.data.len() && plain(&x.attrs) => RVal::I(x.data[i]),
        Value::Lgl(x) if i < x.data.len() && plain(&x.attrs) => RVal::L(x.data[i]),
        Value::Str(x) if i < x.data.len() && plain(&x.attrs) => RVal::V(Value::strs(vec![x.data[i].clone()])),
        Value::List(x) if two && i < x.data.len() && plain(&x.attrs) => unbox(x.data[i].clone()),
        _ => return None,
    })
}

/// Make slot `slot` hold the value of the local variable `sym` as a
/// pending assignment, so that it can be modified in place. Fails unless
/// `sym` is a plain binding of `env` itself (a variable found in an
/// enclosing frame is copied into this one by the interpreter path).
fn own_var(vars: &mut Vars, env: &Env, sym: Sym, slot: Slot) -> bool {
    let slot = slot as usize;
    match &vars.slots[slot] {
        SlotVal::Dirty(RVal::V(_)) => return true,
        SlotVal::Dirty(r) => {
            let v = RVal::V(r.clone().into_value());
            vars.slots[slot] = SlotVal::Dirty(v);
            return true;
        }
        _ => {}
    }
    if env.binding_flags(sym) != Some(0) {
        return false;
    }
    match env.get_local(sym) {
        Some(Value::Promise(p)) if p.value().is_none() => return false,
        Some(v) if v.is_missing_arg() => return false,
        _ => {}
    }
    vars.slots[slot] = SlotVal::Empty;
    let v = match env.take_local(sym) {
        Some(Value::Promise(p)) => p.value().unwrap_or(Value::Null),
        Some(v) => v,
        None => return false,
    };
    vars.slots[slot] = SlotVal::Dirty(RVal::V(v));
    vars.any = true;
    vars.dirty = true;
    true
}

/// `x[i] <- val` in place for an in-range index of a plain vector whose
/// type does not change; `false` leaves `x` untouched.
fn set_element(x: &mut Value, i: usize, val: &RVal, two: bool) -> bool {
    if x.is_object() || i >= x.len() {
        return false;
    }
    match (x, val) {
        (Value::Real(v), RVal::D(d)) => Rc::make_mut(v).data[i] = *d,
        (Value::Real(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data[i] = int_to_real(*n),
        (Value::Int(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data[i] = *n,
        (Value::Lgl(v), RVal::L(n)) => Rc::make_mut(v).data[i] = *n,
        (Value::Str(v), RVal::V(Value::Str(s))) if s.data.len() == 1 && s.attrs.is_none() => {
            Rc::make_mut(v).data[i] = s.data[0].clone()
        }
        (Value::List(v), val) if two && !matches!(val, RVal::V(Value::Null)) => {
            Rc::make_mut(v).data[i] = val.to_value()
        }
        _ => return false,
    }
    true
}

/// Unboxed `sqrt(x)` etc.; `None` where the builtin must run (it warns
/// about NaNs produced, or the input is not a plain number).
fn math1_fast(f: Math1, x: Num) -> Option<RVal> {
    if f == Math1::Abs {
        return Some(match x {
            Num::I(i) | Num::L(i) => RVal::I(if i == NA_INTEGER { i } else { i.abs() }),
            Num::D(d) => RVal::D(d.abs()),
        });
    }
    let d = as_f64(x);
    let r = match f {
        Math1::Sqrt => d.sqrt(),
        Math1::Exp => d.exp(),
        Math1::Floor => d.floor(),
        Math1::Ceiling => d.ceil(),
        Math1::Sin => d.sin(),
        Math1::Cos => d.cos(),
        Math1::Tan => d.tan(),
        Math1::Abs => unreachable!(),
    };
    if r.is_nan() && !d.is_nan() {
        return None;
    }
    Some(RVal::D(r))
}

/// `from:to` as an integer range, when R's `:` gives an integer
/// sequence (R: seq.c:seq_colon); `None` otherwise.
fn colon_range(n1: f64, n2: f64) -> Option<(i64, i64, u64)> {
    if n1.is_nan() || n2.is_nan() {
        return None;
    }
    let int_valued = |x: f64| x == x.trunc() && x > i32::MIN as f64 && x <= i32::MAX as f64;
    if !int_valued(n1) || !int_valued(n2) {
        return None;
    }
    let n = (n2 - n1).abs() as u64 + 1;
    Some((n1 as i64, if n1 <= n2 { 1 } else { -1 }, n))
}

/// An operand of `&&`/`||` as a logical(1).
// R: logic.c:do_logic2, coerce.c:asLogical2
fn andor_operand(v: &Value, and: bool, is_y: bool, call: &Value) -> RResult<Lgl> {
    let is_number = matches!(v, Value::Lgl(_) | Value::Real(_) | Value::Cplx(_))
        || (matches!(v, Value::Int(_)) && !v.inherits("factor"));
    if !is_number {
        return Err(Flow::error_call(
            call.clone(),
            format!("invalid '{}' type in 'x {} y'", if is_y { "y" } else { "x" }, if and { "&&" } else { "||" }),
        ));
    }
    crate::builtins::summary::as_logical2(v, true, call)
}

/// `x[i]` / `x[[i]]` with the index already evaluated: the subset builtin
/// gets forced promises, so nothing is evaluated twice and methods still
/// see the expressions.
fn subset_with_index(interp: &mut Interp, call: &Value, x: Sym, ival: Value, two: bool, env: &Env) -> RResult<Value> {
    let Value::Lang(l) = call else { unreachable!() };
    let xval = interp.eval_sym(x, env)?;
    let args = vec![
        Arg::new(l.args[0].tag, Value::Promise(Rc::new(Promise::forced(l.args[0].value.clone(), xval)))),
        Arg::new(l.args[1].tag, Value::Promise(Rc::new(Promise::forced(l.args[1].value.clone(), ival)))),
    ];
    let b = crate::builtins::lookup_primitive(if two { "[[" } else { "[" }).expect("subset builtin");
    interp.invoke_builtin(b, call, args, env)
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

/// Does `sym`, looked up as a function from `env`, still resolve to base?
/// Frames below the search path (fresh per call) are checked directly; the
/// lookup from the first search-path environment on is memoized per guard
/// against the search epoch, which changes with any function binding there.
fn guard_ok(interp: &mut Interp, sym: Sym, env: &Env, memo: &std::cell::Cell<(u64, usize, bool)>) -> bool {
    let mut cur = env.clone();
    while !cur.is_search() {
        if cur.local_may_bind_fun(sym) {
            return interp.resolves_to_base(sym, env);
        }
        cur = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
    }
    let epoch = crate::env::search_epoch();
    let (e, id, r) = memo.get();
    if e == epoch && id == cur.id() {
        return r;
    }
    let r = interp.resolves_to_base(sym, &cur);
    memo.set((epoch, cur.id(), r));
    r
}

thread_local! {
    /// Register and variable-slot buffers of finished runs, reused so that
    /// a call does not allocate them.
    static BUFFERS: RefCell<Vec<(Vec<RVal>, Vec<SlotVal>)>> = const { RefCell::new(Vec::new()) };
}

pub fn run(interp: &mut Interp, code: &Code, env: &Env) -> RResult<Value> {
    let (mut regs, mut slots) = BUFFERS.with(|b| b.borrow_mut().pop()).unwrap_or_default();
    regs.resize(code.nregs, RVal::V(Value::Null));
    slots.resize_with(code.vars.len(), || SlotVal::Empty);
    let mut vars = Vars { slots, any: false, dirty: false };
    let r = run_with(interp, code, env, &mut regs, &mut vars);
    regs.clear();
    vars.slots.clear();
    BUFFERS.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() < 64 {
            b.push((regs, vars.slots));
        }
    });
    r
}

fn run_with(interp: &mut Interp, code: &Code, env: &Env, regs: &mut [RVal], mut vars: &mut Vars) -> RResult<Value> {
    interp.visible = true;
    // (break target, next target) of active loops.
    let mut loops: Vec<(u32, u32)> = Vec::new();
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
    // Before anything that may run R code or signal.
    macro_rules! flush {
        () => {
            tryop!(vars.flush(interp, env, &code.vars))
        };
    }
    // Assign `$val` to variable `$sym` (slot `$slot`).
    macro_rules! assign {
        ($sym:expr, $slot:expr, $val:expr) => {{
            let val: RVal = $val;
            let slot = $slot as usize;
            if deferrable(env, $sym, &val, matches!(vars.slots[slot], SlotVal::Dirty(_))) {
                vars.slots[slot] = SlotVal::Dirty(val);
                vars.any = true;
                vars.dirty = true;
            } else {
                // Locked or active bindings, functions: assign now.
                flush!();
                tryop!(set_var(interp, env, $sym, &val));
            }
        }};
    }
    while pc < ops.len() {
        let op = &ops[pc];
        pc += 1;
        match op {
            Op::Const { dst, k } => regs[*dst as usize] = RVal::V(code.consts[*k as usize].clone()),
            Op::ConstD { dst, x } => regs[*dst as usize] = RVal::D(*x),
            Op::ConstI { dst, x } => regs[*dst as usize] = RVal::I(*x),
            Op::ConstL { dst, x } => regs[*dst as usize] = RVal::L(*x),
            Op::Null { dst } => regs[*dst as usize] = RVal::V(Value::Null),
            Op::Visible { on } => interp.visible = *on,
            Op::GetVar { dst, sym, slot } => {
                // R: eval() sets R_Visible before the lookup, so forcing a
                // promise (`expr` in withCallingHandlers) keeps its visibility.
                interp.visible = true;
                let slot = *slot as usize;
                if let Some(v) = vars.get(slot) {
                    regs[*dst as usize] = v.clone();
                } else if let Some(v) = plain_lookup(env, *sym) {
                    regs[*dst as usize] = v.clone();
                    vars.slots[slot] = SlotVal::Clean(v);
                    vars.any = true;
                } else {
                    flush!();
                    let r = interp.eval_sym(*sym, env);
                    let v = tryop!(signal(interp, r));
                    regs[*dst as usize] = RVal::V(v);
                }
            }
            Op::SetVar { src, sym, slot } => {
                let v = regs[*src as usize].clone();
                assign!(*sym, *slot, v);
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
                        flush!();
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
                        flush!();
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
                        flush!();
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
                        flush!();
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x], env)))
                    }
                };
            }
            Op::JumpIfNot { cond, target, call } | Op::JumpIf { cond, target, call } => {
                let truth = match &regs[*cond as usize] {
                    RVal::L(x) if *x != NA_LOGICAL => *x != 0,
                    other => {
                        let v = other.to_value();
                        flush!();
                        let c = code.consts[*call as usize].clone();
                        let r = crate::coerce::as_condition(interp, &v, &c).map(|b| Value::lgl(b));
                        let v = tryop!(signal(interp, r));
                        crate::coerce::as_logical(&v) != 0
                    }
                };
                if truth == matches!(op, Op::JumpIf { .. }) {
                    pc = *target as usize;
                }
            }
            Op::Jump { target } => pc = *target as usize,
            Op::Guard { sym, dst, k, skip } => {
                let ok = guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]);
                if !ok {
                    if std::env::var_os("ROBIGO_VM_TRACE").is_some() {
                        eprintln!("vm: guard failed for {}", sym);
                    }
                    flush!();
                    let v = tryop!(interp.eval(&code.consts[*k as usize], env));
                    regs[*dst as usize] = RVal::V(v);
                    pc = *skip as usize;
                }
            }
            Op::Index { dst, x, slot, i, two, k } => {
                interp.visible = true;
                let fast = match scalar_index(&mut vars, env, &regs, i) {
                    Some(ix) => read_var(&mut vars, env, *x, *slot).and_then(|xv| element(xv, ix as usize, *two)),
                    None => None,
                };
                regs[*dst as usize] = match (fast, i) {
                    (Some(v), _) => v,
                    (None, Leaf::R(r)) => {
                        // The index was evaluated already: call `[`/`[[`
                        // with forced promises instead of evaluating it again.
                        flush!();
                        let ival = regs[*r as usize].to_value();
                        let r = subset_with_index(interp, &code.consts[*k as usize], *x, ival, *two, env);
                        RVal::V(tryop!(signal(interp, r)))
                    }
                    (None, _) => {
                        flush!();
                        RVal::V(tryop!(interp.eval(&code.consts[*k as usize], env)))
                    }
                };
            }
            Op::SetIndex { val, x, slot, i, two, k } => {
                let done = match scalar_index(&mut vars, env, &regs, i) {
                    Some(ix) if own_var(&mut vars, env, *x, *slot) => match &mut vars.slots[*slot as usize] {
                        SlotVal::Dirty(RVal::V(xv)) => set_element(xv, ix as usize, &regs[*val as usize], *two),
                        _ => false,
                    },
                    _ => false,
                };
                if !done {
                    flush!();
                    let whole = &code.consts[*k as usize];
                    let Value::Lang(l) = whole else { unreachable!() };
                    let rhs = regs[*val as usize].to_value();
                    let pre = match i {
                        Leaf::R(r) => Some(vec![regs[*r as usize].to_value()]),
                        _ => None,
                    };
                    let r = crate::builtins::lang::complex_assign_pre(
                        interp,
                        whole,
                        &l.args[0].value,
                        &l.args[1].value,
                        rhs,
                        env,
                        false,
                        pre.as_deref(),
                    );
                    tryop!(signal(interp, r.map(|_| Value::Null)));
                }
            }
            Op::CallBuiltin { dst, f, args, call } => {
                flush!();
                let Value::Builtin(b) = &code.consts[*f as usize] else { unreachable!() };
                let argv: Vec<Arg> = code.arglists[*args as usize]
                    .iter()
                    .map(|(tag, r)| Arg::new(*tag, std::mem::replace(&mut regs[*r as usize], RVal::I(0)).into_value()))
                    .collect();
                let r = interp.invoke_builtin(b, &code.consts[*call as usize], argv, env);
                regs[*dst as usize] = RVal::V(tryop!(signal(interp, r)));
            }
            Op::Math1 { f, dst, a, call } => {
                regs[*dst as usize] = match scalar(&regs[*a as usize]).and_then(|x| math1_fast(*f, x)) {
                    Some(v) => {
                        regs[*a as usize] = RVal::I(0);
                        v
                    }
                    None => {
                        flush!();
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x], env)))
                    }
                };
            }
            Op::AndOrArg { dst, a, and, is_y, call } => {
                let x = match &regs[*a as usize] {
                    RVal::L(x) => Some(*x),
                    RVal::I(x) => Some(if *x == NA_INTEGER { NA_LOGICAL } else { (*x != 0) as Lgl }),
                    RVal::D(x) => Some(if x.is_nan() { NA_LOGICAL } else { (*x != 0.0) as Lgl }),
                    _ => None,
                };
                let x = match x {
                    Some(x) => x,
                    None => {
                        flush!();
                        let v = regs[*a as usize].to_value();
                        let r = andor_operand(&v, *and, *is_y, &code.consts[*call as usize]);
                        crate::coerce::as_logical(&tryop!(signal(interp, r.map(Value::lgl_raw))))
                    }
                };
                regs[*a as usize] = RVal::I(0);
                regs[*dst as usize] = RVal::L(x);
            }
            Op::AndOrShort { x, dst, and, target } => {
                let RVal::L(xv) = regs[*x as usize] else { unreachable!() };
                if *and && xv == FALSE {
                    regs[*dst as usize] = RVal::L(FALSE);
                    pc = *target as usize;
                } else if !*and && xv == TRUE {
                    regs[*dst as usize] = RVal::L(TRUE);
                    pc = *target as usize;
                }
            }
            Op::AndOr { dst, x, y, and } => {
                let (RVal::L(x1), RVal::L(x2)) = (&regs[*x as usize], &regs[*y as usize]) else { unreachable!() };
                let (x1, x2) = (*x1, *x2);
                // R: logic.c:do_logic2
                let ans = if *and {
                    if x1 == NA_LOGICAL {
                        if x2 == NA_LOGICAL || x2 != 0 { NA_LOGICAL } else { x2 }
                    } else {
                        x2
                    }
                } else if x1 == NA_LOGICAL {
                    if x2 == NA_LOGICAL || x2 == 0 { NA_LOGICAL } else { x2 }
                } else {
                    x2
                };
                regs[*dst as usize] = RVal::L(ans);
            }
            Op::ForRange { kind, a, b, seq, call } => {
                let range = match kind {
                    RangeKind::Colon => match (scalar(&regs[*a as usize]), scalar(&regs[*b as usize])) {
                        (Some(x), Some(y)) => colon_range(as_f64(x), as_f64(y)),
                        _ => None,
                    },
                    RangeKind::SeqLen => match scalar(&regs[*a as usize]) {
                        Some(Num::I(n)) if n != NA_INTEGER && n >= 0 => Some((1, 1, n as u64)),
                        Some(Num::D(d)) if d >= 0.0 && d < 2147483647.0 => Some((1, 1, d as u64)),
                        _ => None,
                    },
                    RangeKind::SeqAlong => match &regs[*a as usize] {
                        RVal::V(v) if !v.is_object() && v.len() < i32::MAX as usize => Some((1, 1, v.len() as u64)),
                        RVal::D(_) | RVal::I(_) | RVal::L(_) => Some((1, 1, 1)),
                        _ => None,
                    },
                };
                regs[*seq as usize] = match range {
                    Some((start, step, n)) => {
                        regs[*a as usize] = RVal::I(0);
                        regs[*b as usize] = RVal::I(0);
                        RVal::Range { start, step, n }
                    }
                    None => {
                        flush!();
                        let mut argv = vec![std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null))];
                        if *kind == RangeKind::Colon {
                            argv.push(std::mem::replace(&mut regs[*b as usize], RVal::V(Value::Null)));
                        }
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], argv, env)))
                    }
                };
            }
            Op::Eval { dst, k } => {
                flush!();
                let v = tryop!(interp.eval(&code.consts[*k as usize], env));
                regs[*dst as usize] = RVal::V(v);
            }
            Op::Return { src } => {
                flush!();
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
                if matches!(regs[*seq as usize], RVal::Range { .. }) {
                    regs[*seq as usize + 1] = RVal::I(0);
                    continue;
                }
                let mut v = regs[*seq as usize].to_value();
                if v.inherits("factor") {
                    v = crate::builtins::envir::as_character_factor(&v);
                }
                if !(v.is_vector() || v.is_null() || matches!(v, Value::Pairlist(_))) {
                    flush!();
                    let c = code.consts[*call as usize].clone();
                    return signal(interp, Err(Flow::error_call(c, "invalid for() loop sequence")));
                }
                regs[*seq as usize] = RVal::V(v);
                regs[*seq as usize + 1] = RVal::I(0);
            }
            Op::ForStep { seq, idx, var, slot, body, .. } => {
                let i = match regs[*idx as usize] {
                    RVal::I(i) => i as usize,
                    _ => 0,
                };
                if i == 0 {
                    // R binds the loop variable to NULL before iterating.
                    assign!(*var, *slot, RVal::V(Value::Null));
                }
                if let RVal::Range { start, step, n } = regs[*seq as usize] {
                    if i as u64 >= n {
                        continue;
                    }
                    regs[*idx as usize] = RVal::I(i as i32 + 1);
                    assign!(*var, *slot, RVal::I((start + i as i64 * step) as i32));
                    pc = *body as usize;
                    continue;
                }
                let RVal::V(sv) = &regs[*seq as usize] else { unreachable!() };
                if i >= sv.len() {
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
                assign!(*var, *slot, elt);
                pc = *body as usize;
            }
        }
    }
    vars.flush(interp, env, &code.vars)?;
    Ok(std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null)).into_value())
}
