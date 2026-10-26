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

use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::arith::{int_to_real, myfmod, r_pow, real_op, ArithOp, RelOp};
use crate::compile::{compile_body, compile_loop, CallSite, Code, GuardMemo, Reg, NO_GUARD, Leaf, Math1, Op, RangeKind, Slot};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{s, Sym};
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

#[inline(always)]
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

#[inline(always)]
fn as_f64(n: Num) -> f64 {
    match n {
        Num::D(x) => x,
        Num::I(x) | Num::L(x) => int_to_real(x),
    }
}

#[inline(always)]
fn as_i32(n: Num) -> i32 {
    match n {
        Num::I(x) | Num::L(x) => x,
        Num::D(_) => unreachable!(),
    }
}

/// Scalar arithmetic; `None` means "use the general builtin" (e.g. on
/// integer overflow, which needs a warning).
#[inline(always)]
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

#[inline(always)]
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
    /// `ROBIGO_NO_INLINE=1`: never run closure bodies inline (to compare).
    static NO_INLINE: bool = std::env::var_os("ROBIGO_NO_INLINE").is_some();
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
    if std::env::var_os("ROBIGO_VM_DUMP").is_some() {
        eprintln!("== {}", crate::deparse::deparse_one_line(e).chars().take(100).collect::<String>());
        for (pc, op) in code.ops.iter().enumerate() {
            eprintln!("{:4} {:?}", pc, op);
        }
    }
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

struct Vars<'a> {
    slots: &'a mut [SlotVal],
    /// Whether any slot is non-empty / dirty / caches a vector.
    any: bool,
    dirty: bool,
    vclean: bool,
    /// `env::r_epoch()` when the cached (clean) values were valid: R code
    /// that ran since may have changed the variables.
    epoch: u64,
}

impl Vars<'_> {
    /// Before R code may run: write pending assignments to `env`. Their
    /// values stay cached, as do other scalars (until R code actually
    /// runs, see `get`); cached vectors are dropped so that the binding
    /// holds the only reference and R code can modify it in place.
    fn flush(&mut self, interp: &mut Interp, env: &Env, syms: &[Sym]) -> RResult<()> {
        if !self.dirty && !self.vclean {
            return Ok(());
        }
        self.dirty = false;
        self.vclean = false;
        let mut res = Ok(());
        for (i, s) in self.slots.iter_mut().enumerate() {
            match std::mem::replace(s, SlotVal::Empty) {
                SlotVal::Dirty(v) => {
                    if res.is_ok() {
                        res = set_var(interp, env, syms[i], &v);
                    }
                    if res.is_ok() && !matches!(v, RVal::V(_)) {
                        *s = SlotVal::Clean(v);
                    }
                }
                SlotVal::Clean(v) if !matches!(v, RVal::V(_)) => *s = SlotVal::Clean(v),
                _ => {}
            }
        }
        res
    }

    /// Forget cached values if R code ran since they were read.
    fn sync(&mut self) {
        let now = crate::env::r_epoch();
        if self.epoch != now {
            self.epoch = now;
            if self.any {
                for s in self.slots.iter_mut() {
                    if matches!(s, SlotVal::Clean(_)) {
                        *s = SlotVal::Empty;
                    }
                }
                self.vclean = false;
            }
        }
    }

    fn get(&mut self, slot: usize) -> Option<&RVal> {
        let now = crate::env::r_epoch();
        if self.epoch != now {
            // R code ran: forget cached values (pending ones are newer).
            self.epoch = now;
            if self.any {
                for s in self.slots.iter_mut() {
                    if matches!(s, SlotVal::Clean(_)) {
                        *s = SlotVal::Empty;
                    }
                }
                self.vclean = false;
            }
        }
        match &self.slots[slot] {
            SlotVal::Clean(v) | SlotVal::Dirty(v) => Some(v),
            SlotVal::Empty => None,
        }
    }

    /// Cache the current value of a variable.
    fn cache(&mut self, slot: usize, v: RVal) {
        if matches!(v, RVal::V(_)) {
            self.vclean = true;
        }
        self.slots[slot] = SlotVal::Clean(v);
        self.any = true;
    }
}

/// The value of `sym` if reading it cannot run code: a plain binding, or
/// an argument whose promise has been forced.
fn plain_lookup(env: &Env, sym: Sym) -> Option<RVal> {
    // A plain value (or forced promise's value), unboxed without cloning
    // the vector for scalars.
    #[inline(always)]
    fn read(v: &Value) -> Option<RVal> {
        Some(match v {
            Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::D(x.data[0]),
            Value::Int(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::I(x.data[0]),
            Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() => RVal::L(x.data[0]),
            v if v.is_missing_arg() => return None,
            v => RVal::V(v.clone()),
        })
    }
    // `None`: not bound in `e`; `Some(None)`: bound, but reading may run code.
    #[inline(always)]
    fn in_frame(e: &Env, sym: Sym) -> Option<Option<RVal>> {
        let frame = e.0.frame().borrow();
        let b = frame.get(sym)?;
        if b.flags & crate::env::BIND_ACTIVE != 0 {
            return Some(None);
        }
        Some(match &b.value {
            Value::Promise(p) => p.value.borrow().as_ref().and_then(read),
            v => read(v),
        })
    }
    if let Some(r) = in_frame(env, sym) {
        return r;
    }
    let mut cur = env.parent()?;
    loop {
        if let Some(r) = in_frame(&cur, sym) {
            return r;
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
#[inline(always)]
fn read_var<'a>(vars: &'a mut Vars, env: &Env, sym: Sym, slot: Slot) -> Option<&'a RVal> {
    let slot = slot as usize;
    if vars.get(slot).is_none() {
        let v = plain_lookup(env, sym)?;
        vars.cache(slot, v);
    }
    vars.get(slot)
}

/// A leaf operand as a number, without running code.
#[inline(always)]
fn leaf_num(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], l: &Leaf) -> Option<Num> {
    match l {
        Leaf::D(x) => Some(Num::D(*x)),
        Leaf::I(x) => Some(Num::I(*x)),
        Leaf::Var(sym, slot) => read_var(vars, env, *sym, *slot).and_then(scalar),
        Leaf::R(r) => scalar(&regs[*r as usize]),
    }
}

/// A leaf operand's value, evaluating a variable (forcing a promise,
/// signaling a missing one) when it cannot be read in place.
fn leaf_value(interp: &mut Interp, vars: &mut Vars<'_>, env: &Env, regs: &mut [RVal], l: &Leaf) -> RResult<RVal> {
    match l {
        Leaf::D(x) => Ok(RVal::D(*x)),
        Leaf::I(x) => Ok(RVal::I(*x)),
        Leaf::Var(sym, slot) => match read_var(vars, env, *sym, *slot) {
            Some(v) => Ok(v.clone()),
            None => {
                let r = force_var(interp, env, *sym);
                let (v, cacheable) = signal_var(interp, r)?;
                let v = unbox(v);
                if cacheable {
                    vars.sync();
                    vars.cache(*slot as usize, v.clone());
                }
                Ok(v)
            }
        },
        Leaf::R(r) => Ok(std::mem::replace(&mut regs[*r as usize], RVal::V(Value::Null))),
    }
}

/// The value of a variable that cannot be read in place: an argument of
/// this frame is forced directly, anything else evaluated. Also whether the
/// value may be cached (it is now a plain or forced binding).
fn force_var(interp: &mut Interp, env: &Env, sym: Sym) -> RResult<(Value, bool)> {
    let p = match env.0.frame().borrow().get(sym) {
        Some(b) if b.flags == 0 => match &b.value {
            Value::Promise(p) => Some(p.clone()),
            _ => None,
        },
        _ => None,
    };
    match p {
        Some(p) => interp.force_promise(&p).map(|v| (v, true)),
        None => interp.eval_sym(sym, env).map(|v| (v, false)),
    }
}

fn signal_var(interp: &mut Interp, r: RResult<(Value, bool)>) -> RResult<(Value, bool)> {
    match r {
        Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
        r => r,
    }
}

/// A 0-based index from an in-range positive scalar index.
#[inline(always)]
fn scalar_index(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], i: &Leaf) -> Option<f64> {
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
#[inline(always)]
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
fn own_var(vars: &mut Vars<'_>, env: &Env, sym: Sym, slot: Slot) -> bool {
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
/// The 0-based position of the first element of the 1-based range
/// `start, start + step, ...` (length `n`, `step` = ±1) when all of it
/// lies within a vector of length `len`.
fn range_first(start: i64, step: i64, n: u64, len: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let last = start + (n as i64 - 1) * step;
    let ok = |p: i64| p >= 1 && p <= len as i64;
    (ok(start) && ok(last)).then(|| (start - 1) as usize)
}

fn gather_range<T: Clone>(d: &[T], first: usize, step: i64, n: usize) -> Vec<T> {
    if step == 1 {
        d[first..first + n].to_vec()
    } else {
        (0..n).map(|k| d[first - k].clone()).collect()
    }
}

/// `x[a:b]` for an attribute-free vector and an in-range range.
fn range_subset(x: &RVal, start: i64, step: i64, n: u64) -> Option<RVal> {
    let RVal::V(x) = x else { return None };
    if x.attrs().is_some() {
        return None;
    }
    let first = range_first(start, step, n, x.len())?;
    let n = n as usize;
    Some(RVal::V(match x {
        Value::Real(v) => Value::reals(gather_range(&v.data, first, step, n)),
        Value::Int(v) => Value::ints(gather_range(&v.data, first, step, n)),
        Value::Lgl(v) => Value::lgls(gather_range(&v.data, first, step, n)),
        Value::Str(v) => Value::strs(gather_range(&v.data, first, step, n)),
        _ => return None,
    }))
}

/// `x[a:b] <- val` in place: `x` a non-object vector, the range within
/// it, `val` of x's type (or an integer into a double vector) and of
/// length 1 or the range's length. Otherwise false (use `[<-`).
fn set_range(x: &mut Value, start: i64, step: i64, n: u64, val: &RVal) -> bool {
    if x.is_object() {
        return false;
    }
    let Some(first) = range_first(start, step, n, x.len()) else { return false };
    let n = n as usize;
    let pos = |k: usize| if step == 1 { first + k } else { first - k };
    fn fill<T: Clone>(d: &mut [T], n: usize, pos: impl Fn(usize) -> usize, src: &[T]) {
        for k in 0..n {
            d[pos(k)] = src[if src.len() == 1 { 0 } else { k }].clone();
        }
    }
    let fits = |len: usize| len == n || len == 1;
    match (x, val) {
        (Value::Real(v), RVal::D(d)) => fill(&mut Rc::make_mut(v).data, n, pos, &[*d]),
        (Value::Real(v), RVal::I(i)) => fill(&mut Rc::make_mut(v).data, n, pos, &[int_to_real(*i)]),
        (Value::Int(v), RVal::I(i)) => fill(&mut Rc::make_mut(v).data, n, pos, &[*i]),
        (Value::Lgl(v), RVal::L(l)) => fill(&mut Rc::make_mut(v).data, n, pos, &[*l]),
        (Value::Real(v), RVal::V(Value::Real(s))) if fits(s.data.len()) => {
            fill(&mut Rc::make_mut(v).data, n, pos, &s.data)
        }
        (Value::Real(v), RVal::V(Value::Int(s))) if fits(s.data.len()) => {
            let src: Vec<f64> = s.data.iter().map(|&i| int_to_real(i)).collect();
            fill(&mut Rc::make_mut(v).data, n, pos, &src)
        }
        (Value::Int(v), RVal::V(Value::Int(s))) if fits(s.data.len()) => {
            fill(&mut Rc::make_mut(v).data, n, pos, &s.data)
        }
        (Value::Lgl(v), RVal::V(Value::Lgl(s))) if fits(s.data.len()) => {
            fill(&mut Rc::make_mut(v).data, n, pos, &s.data)
        }
        (Value::Str(v), RVal::V(Value::Str(s))) if fits(s.data.len()) => {
            fill(&mut Rc::make_mut(v).data, n, pos, &s.data)
        }
        _ => return false,
    }
    true
}

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
        let mut frame = env.0.frame().borrow_mut();
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
/// Memoized per guard in two ways:
/// - for `env` itself, while neither the search path (epoch) nor `env`'s
///   function bindings (version) change, when `env`'s parent is on the
///   search path: repeated guards in a loop cost no lookup;
/// - for the first search-path environment, after checking that the frames
///   below it (fresh per call) bind no function of that name.
fn guard_ok(interp: &mut Interp, sym: Sym, env: &Env, memo: &std::cell::Cell<GuardMemo>) -> bool {
    let epoch = crate::env::search_epoch();
    let m = memo.get();
    if m.epoch == epoch && m.env == env.id() && m.version == env.fn_version() {
        return m.ok;
    }
    // The common cases without reference count traffic: `env` on the search
    // path, or a closure frame whose parent is.
    if env.is_search() {
        let ok = if m.epoch == epoch && m.search == env.id() { m.ok } else { interp.resolves_to_base(sym, env) };
        memo.set(GuardMemo { epoch, env: env.id(), version: env.fn_version(), search: env.id(), ok });
        return ok;
    }
    if env.local_may_bind_fun(sym) {
        return interp.resolves_to_base(sym, env);
    }
    {
        let parent = env.parent_ref();
        match &*parent {
            Some(p) if p.is_search() => {
                let ok = if m.epoch == epoch && m.search == p.id() { m.ok } else { interp.resolves_to_base(sym, p) };
                memo.set(GuardMemo { epoch, env: env.id(), version: env.fn_version(), search: p.id(), ok });
                return ok;
            }
            Some(_) => {}
            None => return false,
        }
    }
    let Some(mut cur) = env.parent() else { return false };
    let mut depth = 1;
    while !cur.is_search() {
        if cur.local_may_bind_fun(sym) {
            return interp.resolves_to_base(sym, env);
        }
        cur = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
        depth += 1;
    }
    let ok = if m.epoch == epoch && m.search == cur.id() { m.ok } else { interp.resolves_to_base(sym, &cur) };
    // The per-environment memo is only sound when no frame other than
    // `env` lies between it and the search path.
    let (env_id, version) = if depth <= 1 { (env.id(), env.fn_version()) } else { (0, 0) };
    memo.set(GuardMemo { epoch, env: env_id, version, search: cur.id(), ok });
    ok
}

/// A closure whose calls can run inline: see [`inline_info`].
pub struct Inline {
    pub(crate) code: Rc<Code>,
    /// The slot of each formal in `code.vars` (`None`: unused).
    pub(crate) formal_slots: Vec<Option<Slot>>,
    /// Every function the body guards, and the search epoch in which all of
    /// them were last seen to resolve to base (from a search-path closure
    /// environment, where that depends on nothing else).
    pub(crate) guard_syms: Vec<Sym>,
    guards_epoch: std::cell::Cell<u64>,
    /// The body reads variables other than its formals (through the
    /// closure's environment, which may be the caller's frame).
    pub(crate) free_vars: bool,
}

/// Whether calls of `clo` can skip the call machinery: the body only reads
/// variables and constants and does arithmetic, comparisons, `!`, `if` and
/// unboxed math functions (no assignments, no calls, so it cannot observe
/// or change any frame); the formals are plain (no `...`, no defaults); and
/// no formal is named like a function the body guards. Such a body run
/// with its arguments' values behaves like the call as long as every step
/// takes its fast path; [`run_inline`] gives up before anything else.
#[cold]
#[inline(never)]
pub fn inline_info(clo: &Closure) -> Option<Rc<Inline>> {
    if crate::typeprof::enabled() || NO_INLINE.with(|n| *n) {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    if clo.formals.iter().any(|f| f.tag == Some(s::DOTS) || !f.value.is_missing_arg()) {
        return None;
    }
    let formals: Vec<Sym> = clo.formals.iter().map(|f| f.tag.unwrap()).collect();
    let mut guard_syms = Vec::new();
    for op in &code.ops {
        if let Op::Guard { sym, .. } | Op::ArithL { sym, .. } | Op::RelL { sym, .. } = op {
            if !guard_syms.contains(sym) {
                guard_syms.push(*sym);
            }
        }
        match op {
            Op::Const { .. }
            | Op::ConstD { .. }
            | Op::ConstI { .. }
            | Op::ConstL { .. }
            | Op::Null { .. }
            | Op::GetVar { .. }
            | Op::Arith { .. }
            | Op::Rel { .. }
            | Op::Neg { .. }
            | Op::Not { .. }
            | Op::Math1 { .. }
            | Op::JumpIfNot { .. }
            | Op::JumpIf { .. }
            | Op::Jump { .. }
            | Op::Visible { .. }
            | Op::Return { .. } => {}
            Op::Guard { sym, .. } | Op::ArithL { sym, .. } | Op::RelL { sym, .. } if !formals.contains(sym) => {}
            _ => return None,
        }
    }
    let formal_slots = formals.iter().map(|f| code.vars.iter().position(|v| v == f).map(|p| p as Slot)).collect();
    let free_vars = code.vars.iter().any(|v| !formals.contains(v));
    Some(Rc::new(Inline { code, formal_slots, guard_syms, guards_epoch: std::cell::Cell::new(u64::MAX), free_vars }))
}

/// Run an inlinable closure body (see [`inline_info`]) with argument values
/// `args`, in place of a call: `env` is the closure's environment (free
/// variables and guards resolve there, as from the call's frame, which
/// would hold only the formals). `None` as soon as a step would need its
/// slow path (a guard failing, a non-scalar, NA in a condition, an
/// unforced variable): nothing observable has happened, so the caller
/// then makes the real call.
#[inline(never)]
fn run_inline(interp: &mut Interp, info: &Inline, args: &[RVal], env: &Env) -> Option<RVal> {
    let code = &*info.code;
    // All guards at once, once per search epoch, when that suffices.
    let mut guarded = false;
    if env.is_search() {
        let epoch = crate::env::search_epoch();
        if info.guards_epoch.get() != epoch && info.guard_syms.iter().all(|s| interp.resolves_to_base(*s, env)) {
            info.guards_epoch.set(epoch);
        }
        guarded = info.guards_epoch.get() == epoch;
    }
    let mut regs: SmallVec<[RVal; 16]> = SmallVec::new();
    regs.resize_with(code.nregs, || RVal::V(Value::Null));
    let mut slots: SmallVec<[Option<RVal>; 8]> = SmallVec::new();
    slots.resize_with(code.vars.len(), || None);
    for (arg, slot) in args.iter().zip(&info.formal_slots) {
        if let Some(s) = slot {
            slots[*s as usize] = Some(arg.clone());
        }
    }
    fn read(slots: &mut [Option<RVal>], env: &Env, sym: Sym, slot: Slot) -> Option<RVal> {
        let slot = slot as usize;
        if slots[slot].is_none() {
            slots[slot] = Some(plain_lookup(env, sym)?);
        }
        slots[slot].clone()
    }
    let leaf = |slots: &mut [Option<RVal>], regs: &[RVal], l: &Leaf| -> Option<Num> {
        match l {
            Leaf::D(x) => Some(Num::D(*x)),
            Leaf::I(x) => Some(Num::I(*x)),
            Leaf::Var(sym, slot) => scalar(&read(slots, env, *sym, *slot)?),
            Leaf::R(r) => scalar(&regs[*r as usize]),
        }
    };
    let ops = &code.ops;
    let mut pc = 0;
    while pc < ops.len() {
        let op = &ops[pc];
        pc += 1;
        match op {
            Op::Const { dst, k } => regs[*dst as usize] = unbox(code.consts[*k as usize].clone()),
            Op::ConstD { dst, x } => regs[*dst as usize] = RVal::D(*x),
            Op::ConstI { dst, x } => regs[*dst as usize] = RVal::I(*x),
            Op::ConstL { dst, x } => regs[*dst as usize] = RVal::L(*x),
            Op::Null { dst } => regs[*dst as usize] = RVal::V(Value::Null),
            Op::GetVar { dst, sym, slot } => regs[*dst as usize] = read(&mut slots, env, *sym, *slot)?,
            Op::Guard { sym, .. } => {
                if !guarded && !guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]) {
                    return None;
                }
            }
            Op::Arith { op, dst, a, b, .. } => {
                let (x, y) = (scalar(&regs[*a as usize])?, scalar(&regs[*b as usize])?);
                regs[*dst as usize] = arith_fast(*op, x, y)?;
            }
            Op::Rel { op, dst, a, b, .. } => {
                let (x, y) = (scalar(&regs[*a as usize])?, scalar(&regs[*b as usize])?);
                regs[*dst as usize] = rel_fast(*op, x, y);
            }
            Op::ArithL { op, dst, a, b, sym, g, .. } => {
                if !guarded && *g != NO_GUARD && !guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]) {
                    return None;
                }
                let x = leaf(&mut slots, &regs, a)?;
                let y = leaf(&mut slots, &regs, b)?;
                regs[*dst as usize] = arith_fast(*op, x, y)?;
            }
            Op::RelL { op, dst, a, b, sym, g, .. } => {
                if !guarded && *g != NO_GUARD && !guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]) {
                    return None;
                }
                let x = leaf(&mut slots, &regs, a)?;
                let y = leaf(&mut slots, &regs, b)?;
                regs[*dst as usize] = rel_fast(*op, x, y);
            }
            Op::Neg { dst, a, .. } => {
                regs[*dst as usize] = match scalar(&regs[*a as usize])? {
                    Num::D(x) => RVal::D(-x),
                    Num::I(x) | Num::L(x) => RVal::I(if x == NA_INTEGER { x } else { -x }),
                };
            }
            Op::Not { dst, a, .. } => {
                regs[*dst as usize] = match scalar(&regs[*a as usize])? {
                    Num::D(x) => RVal::L(if x.is_nan() { NA_LOGICAL } else { (x == 0.0) as Lgl }),
                    Num::I(x) | Num::L(x) => RVal::L(if x == NA_INTEGER { NA_LOGICAL } else { (x == 0) as Lgl }),
                };
            }
            Op::Math1 { f, dst, a, .. } => regs[*dst as usize] = math1_fast(*f, scalar(&regs[*a as usize])?)?,
            Op::JumpIfNot { cond, target, .. } | Op::JumpIf { cond, target, .. } => {
                let truth = match regs[*cond as usize] {
                    RVal::L(x) if x != NA_LOGICAL => x != 0,
                    _ => return None,
                };
                if truth == matches!(op, Op::JumpIf { .. }) {
                    pc = *target as usize;
                }
            }
            Op::Jump { target } => pc = *target as usize,
            Op::Visible { .. } => {}
            Op::Return { src } => return Some(std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null))),
            _ => return None,
        }
    }
    Some(std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null)))
}

/// Evaluate code that is a single guarded operation on constants and
/// variables (see `Code::simple`) without a VM run; `None` when any part
/// needs the general path (which then runs the code normally).
pub fn run_simple(interp: &mut Interp, code: &Code, env: &Env) -> Option<Value> {
    let i = code.simple?;
    let (sym, a, b) = match &code.ops[i] {
        Op::ArithL { sym, a, b, .. } | Op::RelL { sym, a, b, .. } => (*sym, a, b),
        _ => return None,
    };
    if !guard_ok(interp, sym, env, &code.guard_memo[i]) {
        return None;
    }
    let num = |l: &Leaf| match l {
        Leaf::D(x) => Some(Num::D(*x)),
        Leaf::I(x) => Some(Num::I(*x)),
        Leaf::Var(s, _) => plain_lookup(env, *s).as_ref().and_then(scalar),
        Leaf::R(_) => None,
    };
    let (x, y) = (num(a)?, num(b)?);
    let r = match &code.ops[i] {
        Op::ArithL { op, .. } => arith_fast(*op, x, y)?,
        Op::RelL { op, .. } => rel_fast(*op, x, y),
        _ => return None,
    };
    interp.visible = true;
    Some(r.into_value())
}

/// Whether a closure's frame `env` is unreachable once its body returns:
/// referenced only by the call (`exec_closure`) and its context, with no
/// `on.exit` code to run in it. Pending assignments then need not be
/// written: nothing can read them.
fn frame_dies(interp: &Interp, env: &Env) -> bool {
    Rc::strong_count(&env.0) == 2
        && Rc::weak_count(&env.0) == 0
        && interp.frames.last().is_some_and(|c| c.on_exit.is_empty() && c.cloenv.ptr_eq(env))
        && !crate::typeprof::enabled()
}

/// Whether `sym` is bound in `env`'s own frame to a promise not yet forced.
fn unforced_promise(env: &Env, sym: Sym) -> bool {
    let frame = env.0.frame().borrow();
    matches!(frame.get(sym), Some(b) if b.flags == 0 && matches!(&b.value, Value::Promise(p) if p.value.borrow().is_none()))
}

/// An active loop: break and next targets, its `LoopEnter` index and how
/// often its back edge ran in this entry.
#[derive(Clone, Copy)]
struct LoopRec {
    brk: u32,
    nxt: u32,
    enter: u32,
    count: u32,
}

/// Iterations of a loop in the VM before it is compiled to native code.
const JIT_THRESHOLD: u32 = 16;

/// The scalar or vector type of a variable's current value for native code.
fn jit_type(v: &RVal) -> Option<crate::jit::JType> {
    use crate::jit::JType;
    Some(match v {
        RVal::D(_) => JType::D,
        RVal::I(_) => JType::I,
        RVal::L(_) => JType::L,
        RVal::V(Value::Real(x)) if x.attrs.is_none() => {
            if x.data.len() == 1 {
                JType::D
            } else {
                JType::VecD
            }
        }
        RVal::V(Value::Int(x)) if x.attrs.is_none() => {
            if x.data.len() == 1 {
                JType::I
            } else {
                JType::VecI
            }
        }
        _ => return None,
    })
}

/// Whether calls of `clo` can run inline (see [`inline_info`]).
pub(crate) fn inlinable(clo: &Closure) -> bool {
    enabled() && clo.inline.get_or_init(|| inline_info(clo)).is_some()
}

/// Call an inlinable closure (see [`inlinable`]) with argument values,
/// without the call machinery; `None` where its body gives up (then make
/// the real call).
pub(crate) fn call_inline(interp: &mut Interp, clo: &Closure, args: &[Value]) -> Option<Value> {
    let info = clo.inline.get()?.clone()?;
    let vals: SmallVec<[RVal; 4]> = args.iter().map(|a| unbox(a.clone())).collect();
    let v = run_inline(interp, &info, &vals, &clo.env)?;
    interp.visible = true;
    Some(v.into_value())
}

/// The call at call site `site` (of `f`) as a call native code can run
/// inline: a pure closure without free variables (see [`inline_info`])
/// called with plain arguments.
fn inline_call(interp: &mut Interp, code: &Code, f: Sym, site: u32, env: &Env, vars: &Vars<'_>) -> Option<crate::jit::InlineCall> {
    let cs = &code.call_sites[site as usize];
    if cs.fslot.is_some_and(|s| matches!(vars.slots[s as usize], SlotVal::Dirty(_))) {
        return None;
    }
    let leaves = cs.leaf_args.as_ref()?;
    let Value::Closure(clo) = call_site_fun_fast(interp, f, cs, env)? else { return None };
    if leaves.len() != clo.formals.len() {
        return None;
    }
    let info = clo.inline.get_or_init(|| inline_info(&clo)).clone()?;
    if info.free_vars {
        return None;
    }
    Some(crate::jit::InlineCall {
        code: info.code.clone(),
        formal_slots: info.formal_slots.clone(),
        guard_syms: info.guard_syms.clone(),
        args: leaves.clone(),
        clo: clo.clone(),
    })
}

/// Run the loop entered at op `enter` natively (compiling it first if
/// needed): see `crate::jit`. `None`: it cannot run natively now (the VM
/// runs it). Otherwise the outcome and, after a deopt, the loops inside it
/// that are active at the op where the VM continues.
fn jit_enter(
    interp: &mut Interp,
    code: &Code,
    enter: usize,
    env: &Env,
    regs: &mut [RVal],
    vars: &mut Vars<'_>,
) -> Option<(crate::jit::Outcome, Vec<(u32, u32, u32)>)> {
    use crate::jit::{JType, JitSlot, Outcome, Reject};
    let key = enter as u32;
    let state = match code.jit.borrow().get(&key) {
        Some(JitSlot::Ready(l)) => Some(l.clone()),
        Some(JitSlot::Never) => return None,
        Some(JitSlot::Failed(n)) if *n >= 3 => return None,
        _ => None,
    };
    let jl = match state {
        Some(l) => l,
        None => {
            // Types of the variables' current values, and of the for
            // sequences set up before the loop.
            let mut types: Vec<Option<JType>> = Vec::with_capacity(code.vars.len());
            for (s, sym) in code.vars.iter().enumerate() {
                types.push(read_var(vars, env, *sym, s as Slot).and_then(jit_type));
            }
            let Op::LoopEnter { brk, .. } = code.ops[enter] else { return None };
            let mut live: HashMap<Reg, JType> = HashMap::default();
            for op in &code.ops[enter + 1..brk as usize] {
                if let Op::ForStep { seq, idx, .. } = op {
                    let set_inside = code.ops[enter + 1..brk as usize]
                        .iter()
                        .any(|o| matches!(o, Op::ForRange { seq: s2, .. } if s2 == seq));
                    if !set_inside {
                        if let (RVal::Range { .. }, RVal::I(_)) = (&regs[*seq as usize], &regs[*idx as usize]) {
                            live.insert(*seq, JType::Range);
                            live.insert(*idx, JType::I);
                        }
                    }
                }
            }
            // Calls of pure closures without free variables run inline.
            let mut calls = HashMap::default();
            for op in &code.ops[enter + 1..brk as usize] {
                if let Op::Call { f, site, .. } = op {
                    if let Some(c) = inline_call(interp, code, *f, *site, env, vars) {
                        calls.insert(*site, c);
                    }
                }
            }
            let r = crate::jit::compile(code, enter, &|s| types[s as usize], &|r| live.get(&r).copied(), calls);
            let mut cache = code.jit.borrow_mut();
            match r {
                Ok(l) => {
                    if crate::jit::trace() {
                        eprintln!("jit: compiled loop at {} ({} variables, {} registers)", enter, l.slots.len(), l.reg_idx.len());
                    }
                    let l = Rc::new(l);
                    cache.insert(key, JitSlot::Ready(l.clone()));
                    l
                }
                Err(Reject::Never) => {
                    if crate::jit::trace() {
                        eprintln!("jit: loop at {} cannot be compiled", enter);
                    }
                    cache.insert(key, JitSlot::Never);
                    return None;
                }
                Err(Reject::NotNow) => {
                    let n = match cache.get(&key) {
                        Some(JitSlot::Failed(n)) => *n,
                        _ => 0,
                    };
                    cache.insert(key, JitSlot::Failed(n + 1));
                    return None;
                }
            }
        }
    };
    // Guards: native code assumes them, and that its inlined calls still
    // call the same closures (whose guards hold).
    for (sym, at) in &jl.guards {
        if !guard_ok(interp, *sym, env, &code.guard_memo[*at]) {
            return None;
        }
    }
    for (site, clo, syms) in &jl.calls {
        let cs = &code.call_sites[*site as usize];
        if cs.fslot.is_some_and(|s| matches!(vars.slots[s as usize], SlotVal::Dirty(_))) {
            return None;
        }
        let Op::Call { f, .. } = code.ops.iter().find(|o| matches!(o, Op::Call { site: s2, .. } if s2 == site))? else {
            return None;
        };
        match call_site_fun_fast(interp, *f, cs, env) {
            Some(Value::Closure(c)) if Rc::ptr_eq(&c, clo) => {}
            _ => return None,
        }
        if !syms.iter().all(|s| interp.resolves_to_base(*s, &clo.env)) {
            return None;
        }
    }
    let mut vals: SmallVec<[u64; 32]> = SmallVec::new();
    vals.resize(jl.nvals, 0);
    let mut vecs: SmallVec<[crate::jit::VecView; 4]> = SmallVec::new();
    for s in &jl.slots {
        if s.ty.is_vector() {
            let ok = match read_var(vars, env, s.sym, s.slot) {
                Some(RVal::V(v)) => !v.is_object() && jit_type(&RVal::V(v.clone())) == Some(s.ty),
                _ => false,
            };
            if !ok || (s.writable && !own_var(vars, env, s.sym, s.slot)) {
                return None;
            }
            let view = match &mut vars.slots[s.slot as usize] {
                SlotVal::Dirty(RVal::V(Value::Real(x))) | SlotVal::Clean(RVal::V(Value::Real(x))) if s.ty == JType::VecD => {
                    let d = if s.writable { Rc::make_mut(x).data.as_mut_ptr() } else { x.data.as_ptr() as *mut f64 };
                    crate::jit::VecView { ptr: d as *mut u8, len: x.data.len() as u64 }
                }
                SlotVal::Dirty(RVal::V(Value::Int(x))) | SlotVal::Clean(RVal::V(Value::Int(x))) if s.ty == JType::VecI => {
                    let d = if s.writable { Rc::make_mut(x).data.as_mut_ptr() } else { x.data.as_ptr() as *mut i32 };
                    crate::jit::VecView { ptr: d as *mut u8, len: x.data.len() as u64 }
                }
                _ => return None,
            };
            debug_assert_eq!(vecs.len(), s.idx);
            vecs.push(view);
            continue;
        }
        let cur = read_var(vars, env, s.sym, s.slot).cloned();
        if s.written {
            let pending = matches!(vars.slots[s.slot as usize], SlotVal::Dirty(_));
            if !deferrable(env, s.sym, &RVal::I(0), pending) {
                return None;
            }
        }
        let bits = match (s.ty, cur.as_ref().and_then(scalar)) {
            (JType::D, Some(Num::D(x))) => Some(x.to_bits()),
            (JType::I, Some(Num::I(x))) | (JType::L, Some(Num::L(x))) => Some(x as i64 as u64),
            _ => None,
        };
        match bits {
            Some(b) => vals[s.idx] = b,
            None if s.needs_entry => return None,
            None => {}
        }
    }
    for (reg, ty) in &jl.live_in {
        let idx = jl.reg_idx[reg];
        match (&regs[*reg as usize], ty) {
            (RVal::Range { start, step, n }, JType::Range) => {
                vals[idx] = *start as u64;
                vals[idx + 1] = *step as u64;
                vals[idx + 2] = *n;
            }
            (RVal::I(i), JType::I) => vals[idx] = *i as i64 as u64,
            _ => return None,
        }
    }
    let ret = (jl.func)(vals.as_mut_ptr(), vecs.as_ptr());
    // Write the variables back as pending assignments.
    for s in &jl.slots {
        if !s.written || !s.ty.is_scalar() {
            continue;
        }
        let assigned = s.needs_entry || s.flag.is_some_and(|f| vals[f] != 0);
        if !assigned {
            continue;
        }
        let b = vals[s.idx];
        let v = match s.ty {
            JType::D => RVal::D(f64::from_bits(b)),
            JType::I => RVal::I(b as i32),
            _ => RVal::L(b as i32),
        };
        vars.slots[s.slot as usize] = SlotVal::Dirty(v);
        vars.any = true;
        vars.dirty = true;
    }
    if ret == crate::jit::EXIT {
        if crate::jit::trace() {
            eprintln!("jit: loop at {} finished", enter);
        }
        return Some((Outcome::Exited, Vec::new()));
    }
    let p = ret as usize;
    for (reg, ty) in jl.deopt_regs.get(&ret).map(|v| v.as_slice()).unwrap_or(&[]) {
        let idx = jl.reg_idx[reg];
        let b = vals[idx];
        regs[*reg as usize] = match ty {
            JType::D => RVal::D(f64::from_bits(b)),
            JType::I => RVal::I(b as i32),
            JType::L => RVal::L(b as i32),
            JType::Range => RVal::Range { start: b as i64, step: vals[idx + 1] as i64, n: vals[idx + 2] },
            _ => continue,
        };
    }
    if crate::jit::trace() {
        eprintln!("jit: loop at {} continues in the VM at {}", enter, p);
    }
    let nested = jl.nested.iter().copied().filter(|(e, b, _)| (*e as usize) < p && p <= *b as usize).collect();
    Some((Outcome::Deopt(p), nested))
}

/// The function a call site calls, memoized like [`guard_ok`]: frames below
/// the search path are checked directly, the search-path lookup is cached
/// against the search epoch (only when it needs no forcing or active
/// bindings, i.e. when [`Interp::search_fun`] answers it).
fn call_site_fun(interp: &mut Interp, sym: Sym, site: &CallSite, env: &Env) -> RResult<Value> {
    match call_site_fun_fast(interp, sym, site, env) {
        Some(f) => Ok(f),
        None => interp.find_fun_for_call(sym, &site.call, env),
    }
}

/// [`call_site_fun`] when it needs no forcing, active bindings or errors
/// (so it cannot run R code); `None` otherwise.
fn call_site_fun_fast(interp: &mut Interp, sym: Sym, site: &CallSite, env: &Env) -> Option<Value> {
    let mut cur = env.clone();
    while !cur.is_search() {
        if cur.local_may_bind_fun(sym) {
            return None;
        }
        cur = cur.parent()?;
    }
    let epoch = crate::env::search_epoch();
    if let Some((e, id, f)) = &*site.memo.borrow() {
        if *e == epoch && *id == cur.id() {
            return Some(f.clone());
        }
    }
    let f = interp.search_fun(sym, &cur)?;
    *site.memo.borrow_mut() = Some((epoch, cur.id(), f.clone()));
    Some(f)
}

pub fn run(interp: &mut Interp, code: &Code, env: &Env) -> RResult<Value> {
    run_pre(interp, code, env, false)
}

/// Run a closure body in its fresh call frame `env`. For a closure defined
/// on the search path, such a frame holds only the formals: if none is
/// named like a guarded function, every guard's result depends only on the
/// search path, so all are checked once per search epoch (and closure)
/// rather than once per call, and hold until R code first runs.
pub fn run_closure(interp: &mut Interp, code: &Code, env: &Env, clo: &Closure) -> RResult<Value> {
    let pre = code.nguards > 0 && clo.env.is_search() && fresh_guards_ok(interp, code, clo);
    run_pre(interp, code, env, pre)
}

fn fresh_guards_ok(interp: &mut Interp, code: &Code, clo: &Closure) -> bool {
    let key = (crate::env::search_epoch(), clo.env.id(), Rc::as_ptr(&clo.formals) as *const Arg as usize);
    if code.fresh_guards.get() == key {
        return true;
    }
    if clo.formals.iter().any(|f| f.tag.is_some_and(|t| code.guard_syms.contains(&t))) {
        return false;
    }
    if !code.guard_syms.iter().all(|s| interp.resolves_to_base(*s, &clo.env)) {
        return false;
    }
    code.fresh_guards.set(key);
    true
}

fn run_pre(interp: &mut Interp, code: &Code, env: &Env, pre: bool) -> RResult<Value> {
    // Registers and slots live on the Rust stack for typical code, so a
    // call does not allocate them.
    // (Sized for typical base-R functions, which a call should not have
    // to allocate for.)
    let mut regs: SmallVec<[RVal; 32]> = SmallVec::new();
    regs.resize_with(code.nregs, || RVal::V(Value::Null));
    let mut slots: SmallVec<[SlotVal; 16]> = SmallVec::new();
    slots.resize_with(code.vars.len(), || SlotVal::Empty);
    let mut vars = Vars { slots: &mut slots, any: false, dirty: false, vclean: false, epoch: crate::env::r_epoch() };
    let mut small = [0u32; 64];
    let mut big = Vec::new();
    let checked: &mut [u32] = if code.nguards <= small.len() {
        &mut small[..code.nguards]
    } else {
        big.resize(code.nguards, 0);
        &mut big
    };
    if pre {
        // Valid in the first flush generation (see `run_with`).
        checked.fill(1);
    }
    run_with(interp, code, env, &mut regs, &mut vars, checked)
}

fn run_with(interp: &mut Interp, code: &Code, env: &Env, regs: &mut [RVal], mut vars: &mut Vars<'_>,
    // The `flush!()` generation in which each guard last held: only R code
    // (always run after a flush) can rebind functions, so a guard that held
    // in the current generation needs no new check.
    checked: &mut [u32],
) -> RResult<Value> {
    let mut generation: u32 = 1;
    // Where `env` or its parent is on the search path, whether guards hold
    // depends only on the search epoch and `env`'s function bindings: R
    // code that ran (after a flush) invalidates them only if one changed.
    // Elsewhere (nested closures) every flush does.
    let stable = env.is_search() || env.parent_ref().as_ref().is_some_and(|p| p.is_search());
    let mut token = (crate::env::search_epoch(), env.fn_version());
    let mut recheck = false;
    interp.visible = true;
    // Active loops.
    let mut loops: Vec<LoopRec> = Vec::new();
    let ops = &code.ops;
    let mut pc: usize = 0;
    macro_rules! tryop {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(Flow::Break(e)) if e.ptr_eq(env) && !loops.is_empty() => {
                    pc = loops.last().unwrap().brk as usize;
                    continue;
                }
                Err(Flow::Next(e)) if e.ptr_eq(env) && !loops.is_empty() => {
                    pc = loops.last().unwrap().nxt as usize;
                    continue;
                }
                Err(f) => return Err(f),
            }
        };
    }
    // Before anything that may run R code or signal.
    macro_rules! flush {
        () => {{
            if stable {
                recheck = true;
            } else {
                generation = generation.wrapping_add(1).max(1);
            }
            tryop!(vars.flush(interp, env, &code.vars))
        }};
    }
    // Before relying on `checked`: start a new generation if R code that
    // ran since the last flush changed a function binding.
    macro_rules! revalidate {
        () => {{
            if recheck {
                recheck = false;
                let now = (crate::env::search_epoch(), env.fn_version());
                if now != token {
                    token = now;
                    generation = generation.wrapping_add(1).max(1);
                }
            }
        }};
    }
    // `ArithL`/`RelL`: check the folded guard (if any), read the operands
    // in place, take `$fast` for scalars, else call the builtin.
    macro_rules! leaf_op {
        ($dst:expr, $a:expr, $b:expr, $call:expr, $sym:expr, $g:expr, $fast:expr) => {{
            let (dst, call, g) = (*$dst as usize, *$call as usize, *$g);
            if g != NO_GUARD {
                revalidate!();
            }
            if g != NO_GUARD && checked[g as usize] != generation {
                if guard_ok(interp, *$sym, env, &code.guard_memo[pc - 1]) {
                    checked[g as usize] = generation;
                } else {
                    if std::env::var_os("ROBIGO_VM_TRACE").is_some() {
                        eprintln!("vm: guard failed for {}", $sym);
                    }
                    flush!();
                    regs[dst] = RVal::V(tryop!(interp.eval(&code.consts[call], env)));
                    continue;
                }
            }
            let x = leaf_num(&mut vars, env, &regs, $a);
            let y = leaf_num(&mut vars, env, &regs, $b);
            let fast = match (x, y) {
                (Some(x), Some(y)) => $fast(x, y),
                _ => None,
            };
            regs[dst] = match fast {
                Some(v) => {
                    // Drop operand references so bindings stay unshared.
                    for l in [$a, $b] {
                        if let Leaf::R(r) = l {
                            regs[*r as usize] = RVal::I(0);
                        }
                    }
                    v
                }
                None => {
                    flush!();
                    let x = tryop!(leaf_value(interp, &mut vars, env, regs, $a));
                    let y = tryop!(leaf_value(interp, &mut vars, env, regs, $b));
                    // Operands that needed evaluating (forced promises) are
                    // usually scalars.
                    match (scalar(&x), scalar(&y)) {
                        (Some(p), Some(q)) => match $fast(p, q) {
                            Some(v) => v,
                            None => RVal::V(tryop!(call_builtin(interp, &code.consts[call], vec![x, y], env))),
                        },
                        _ => RVal::V(tryop!(call_builtin(interp, &code.consts[call], vec![x, y], env))),
                    }
                }
            };
        }};
    }
    let jit_on = crate::jit::enabled();
    // Run the innermost loop natively from its head (the op being executed
    // is at the head or the back edge, so the VM state is that of the loop
    // between iterations) if it compiles; continue after it or where the
    // native code gave up.
    macro_rules! jit_loop {
        () => {{
            let rec = *loops.last().unwrap();
            match jit_enter(interp, code, rec.enter as usize, env, regs, &mut vars) {
                Some((crate::jit::Outcome::Exited, _)) => {
                    loops.pop();
                    pc = rec.brk as usize + 1;
                    continue;
                }
                Some((crate::jit::Outcome::Deopt(p), nested)) => {
                    for (e, b, n) in nested {
                        loops.push(LoopRec { brk: b, nxt: n, enter: e, count: JIT_THRESHOLD + 1 });
                    }
                    loops.last_mut().unwrap().count = JIT_THRESHOLD + 1;
                    pc = p;
                    continue;
                }
                None => {}
            }
        }};
    }
    // Count iterations of the innermost loop; at the threshold, try native.
    macro_rules! back_edge {
        () => {{
            if jit_on {
                if let Some(rec) = loops.last_mut() {
                    rec.count += 1;
                    if rec.count == JIT_THRESHOLD {
                        // (Native code starts at the loop head, re-executing
                        // this op.)
                        jit_loop!();
                    }
                }
            }
        }};
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
                vars.slots[slot] = SlotVal::Empty;
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
                    vars.cache(slot, v);
                } else {
                    flush!();
                    let r = force_var(interp, env, *sym);
                    let (v, cacheable) = tryop!(signal_var(interp, r));
                    let v = unbox(v);
                    if cacheable {
                        vars.sync();
                        vars.cache(slot, v.clone());
                    }
                    regs[*dst as usize] = v;
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
            Op::ArithL { op, dst, a, b, call, sym, g } => {
                leaf_op!(dst, a, b, call, sym, g, |x, y| arith_fast(*op, x, y));
            }
            Op::RelL { op, dst, a, b, call, sym, g } => {
                leaf_op!(dst, a, b, call, sym, g, |x, y| Some(rel_fast(*op, x, y)));
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
                if (*target as usize) < pc {
                    back_edge!();
                }
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
            Op::Jump { target } => {
                if (*target as usize) < pc {
                    back_edge!();
                }
                pc = *target as usize
            }
            Op::Guard { sym, dst, k, skip, g } => {
                revalidate!();
                if checked[*g as usize] == generation {
                    continue;
                }
                let ok = guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]);
                if ok {
                    checked[*g as usize] = generation;
                } else {
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
                let mut fast = match scalar_index(&mut vars, env, &regs, i) {
                    Some(ix) => read_var(&mut vars, env, *x, *slot).and_then(|xv| element(xv, ix as usize, *two)),
                    None => None,
                };
                if let (None, Leaf::R(r), false) = (&fast, i, *two) {
                    if let RVal::Range { start, step, n } = regs[*r as usize] {
                        fast = read_var(&mut vars, env, *x, *slot).and_then(|xv| range_subset(xv, start, step, n));
                    }
                }
                if fast.is_none() && vars.get(*slot as usize).is_none() && unforced_promise(env, *x) {
                    // `x` is an argument not yet forced: force it (`[`/`[[`
                    // evaluate it first anyway), then retry the fast path.
                    flush!();
                    let r = interp.eval_sym(*x, env);
                    tryop!(signal(interp, r));
                    fast = match scalar_index(&mut vars, env, &regs, i) {
                        Some(ix) => read_var(&mut vars, env, *x, *slot).and_then(|xv| element(xv, ix as usize, *two)),
                        None => None,
                    };
                }
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
                let range = match (i, *two) {
                    (Leaf::R(r), false) => match regs[*r as usize] {
                        RVal::Range { start, step, n } => Some((start, step, n)),
                        _ => None,
                    },
                    _ => None,
                };
                let done = match (range, scalar_index(&mut vars, env, &regs, i)) {
                    (Some((start, step, n)), _) if own_var(&mut vars, env, *x, *slot) => {
                        match &mut vars.slots[*slot as usize] {
                            SlotVal::Dirty(RVal::V(xv)) => set_range(xv, start, step, n, &regs[*val as usize]),
                            _ => false,
                        }
                    }
                    (None, Some(ix)) if own_var(&mut vars, env, *x, *slot) => match &mut vars.slots[*slot as usize] {
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
                    // The assignment rebound `x` in the frame.
                    vars.slots[*slot as usize] = SlotVal::Empty;
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
            Op::Call { dst, f, site } => {
                let site = &code.call_sites[*site as usize];
                // Look the function up before flushing when that cannot run
                // code and no pending assignment to its name is outstanding.
                let pending_f = site.fslot.is_some_and(|s| matches!(vars.slots[s as usize], SlotVal::Dirty(_)));
                let fast_fun = if pending_f || site.leaf_args.is_none() {
                    None
                } else {
                    call_site_fun_fast(interp, *f, site, env)
                };
                // A pure closure with plain arguments: run its body inline
                // (it cannot observe the frame, so nothing needs flushing,
                // unless it reads variables that pending writes may affect).
                if let (Some(Value::Closure(clo)), Some(leaves)) = (&fast_fun, &site.leaf_args) {
                    if leaves.len() == clo.formals.len() {
                        if let Some(info) = clo.inline.get_or_init(|| inline_info(clo)).clone().filter(|i| !(i.free_vars && vars.dirty)) {
                            let mut args: SmallVec<[RVal; 4]> = SmallVec::new();
                            for l in leaves {
                                args.push(match l {
                                    Leaf::D(x) => RVal::D(*x),
                                    Leaf::I(x) => RVal::I(*x),
                                    Leaf::Var(sym, slot) => match read_var(&mut vars, env, *sym, *slot) {
                                        Some(v) => v.clone(),
                                        None => break,
                                    },
                                    Leaf::R(_) => break,
                                });
                            }
                            if args.len() == leaves.len() {
                                if let Some(v) = run_inline(interp, &info, &args, &clo.env) {
                                    interp.visible = true;
                                    regs[*dst as usize] = v;
                                    continue;
                                }
                            }
                        }
                    }
                }
                flush!();
                let fun = match fast_fun {
                    Some(f) => f,
                    None => tryop!(call_site_fun(interp, *f, site, env)),
                };
                let v = match (&fun, &site.call) {
                    (Value::Closure(clo), Value::Lang(l)) => {
                        let r = match interp.promise_args_with(&l.args, env, &site.codes) {
                            Ok(promargs) => interp.call_closure(&site.call, clo, promargs, env),
                            Err(e) => signal(interp, Err(e)),
                        };
                        tryop!(r)
                    }
                    _ => tryop!(interp.eval(&site.call, env)),
                };
                regs[*dst as usize] = RVal::V(v);
            }
            Op::Colon { dst, a, b, call } => {
                let range = match (scalar(&regs[*a as usize]), scalar(&regs[*b as usize])) {
                    (Some(x @ (Num::D(_) | Num::I(_))), Some(y @ (Num::D(_) | Num::I(_)))) => {
                        colon_range(as_f64(x), as_f64(y))
                    }
                    _ => None,
                };
                regs[*dst as usize] = match range {
                    Some((start, step, n)) => RVal::Range { start, step, n },
                    None => {
                        flush!();
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::I(0));
                        let y = std::mem::replace(&mut regs[*b as usize], RVal::I(0));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x, y], env)))
                    }
                };
            }
            Op::Eval { dst, k } => {
                flush!();
                let v = tryop!(interp.eval(&code.consts[*k as usize], env));
                regs[*dst as usize] = RVal::V(v);
            }
            Op::Return { src } => {
                if !(code.is_body && frame_dies(interp, env)) {
                    flush!();
                }
                let v = std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null)).into_value();
                if code.is_body {
                    return Ok(v);
                }
                return Err(Flow::Return(v, env.clone()));
            }
            Op::LoopEnter { brk, nxt } => {
                loops.push(LoopRec { brk: *brk, nxt: *nxt, enter: (pc - 1) as u32, count: 0 });
                // A loop compiled on an earlier entry runs natively at once.
                if jit_on && matches!(code.jit.borrow().get(&((pc - 1) as u32)), Some(crate::jit::JitSlot::Ready(_))) {
                    jit_loop!();
                }
            }
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
                back_edge!();
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
    if !(code.is_body && frame_dies(interp, env)) {
        vars.flush(interp, env, &code.vars)?;
    }
    Ok(std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null)).into_value())
}
