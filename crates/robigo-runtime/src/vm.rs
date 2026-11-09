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

use crate::builtins::arith::{ArithOp, RelOp, int_to_real, myfmod, r_pow, real_op};
use crate::compile::{
    Builtin1, CallSite, Code, GuardMemo, InlineArg, Leaf, Math1, NO_GUARD, Op, RangeKind, Reg, SiArg, SiInline, Slot,
    compile_body, compile_loop,
};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::{Sym, s};
use crate::value::*;

mod rjit;
pub use rjit::{JitState, begin_default_force, frozen_fun, inlines_local, jit_check, jit_inherit};
mod slow;
use slow::{Kind, OTHER, Pure, Shape, Slow, arith_slow, dbl_arith, leaf_op_slow, math1_slow, pure_plan, shape};

#[derive(Clone)]
pub enum RVal {
    V(Value),
    D(f64),
    I(i32),
    L(Lgl),
    /// The integer sequence `start, start + step, ...` of length `n` a
    /// `for` loop steps through (only ever in a sequence register).
    Range {
        start: i64,
        step: i64,
        n: u64,
    },
}

impl RVal {
    pub(crate) fn into_value(self) -> Value {
        match self {
            RVal::V(v) => v,
            RVal::D(x) => Value::real(x),
            RVal::I(x) => Value::int(x),
            RVal::L(x) => Value::lgl_raw(x),
            // `:`, seq_len, seq_along: GNU R's compact sequences
            RVal::Range { start, step, n } => Value::compact_intseq(n as usize, start, step),
        }
    }

    fn to_value(&self) -> Value {
        self.clone().into_value()
    }

    /// The value as an object of its own, never a shared small integer
    /// (`Value::for_binding`): what native code gets for a computed scalar
    /// (GNU R's is a fresh box).
    #[inline]
    pub(crate) fn to_binding(&self) -> Value {
        match self {
            RVal::I(x) => Value::int_fresh(*x),
            RVal::V(v) => v.clone().for_binding(),
            r => r.to_value(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Num {
    D(f64),
    I(i32),
    L(i32),
}

#[inline(always)]
pub(crate) fn scalar(r: &RVal) -> Option<Num> {
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
pub(crate) fn arith_fast(op: ArithOp, a: Num, b: Num) -> Option<RVal> {
    if matches!(a, Num::D(_)) || matches!(b, Num::D(_)) {
        if op == ArithOp::Mod && crate::builtins::arith::fmod_warns(as_f64(a), as_f64(b)) {
            return None;
        }
        // R: arithmetic.c:do_arith. With a scalar integer operand GNU R's
        // compiled +/* use the right operand as SSE destination; with two
        // doubles (or a logical coerced to double), the left payload wins.
        // Keep LLVM from commuting the mixed operations when both are NaN.
        let (x, y) = (as_f64(a), as_f64(b));
        let value = if matches!(a, Num::I(_)) || matches!(b, Num::I(_)) {
            match op {
                ArithOp::Plus => crate::builtins::complex::sse::add(y, x),
                ArithOp::Times => crate::builtins::complex::sse::mul(y, x),
                _ => real_op(op, x, y),
            }
        } else {
            real_op(op, x, y)
        };
        return Some(RVal::D(value));
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
pub(crate) fn rel_fast(op: RelOp, a: Num, b: Num) -> RVal {
    if matches!(a, Num::D(_)) || matches!(b, Num::D(_)) {
        let (x, y) = (as_f64(a), as_f64(b));
        return RVal::L(if x.is_nan() || y.is_nan() { NA_LOGICAL } else { op.test(x, y) as Lgl });
    }
    let (x, y) = (as_i32(a), as_i32(b));
    RVal::L(if x == NA_INTEGER || y == NA_INTEGER { NA_LOGICAL } else { op.test(x, y) as Lgl })
}

/// Bytecode for the default arguments of closures with these `formals`,
/// compiled on first use and shared by closures with the same formals.
pub fn default_codes(formals: &Rc<[Arg]>) -> Rc<[Option<Rc<Code>>]> {
    let key = Rc::as_ptr(formals) as *const Arg as usize;
    if let Some(c) = DEFAULTS.with(|c| c.borrow().get(&key).map(|(_, codes)| codes.clone())) {
        return c;
    }
    let codes: Rc<[Option<Rc<Code>>]> = formals
        .iter()
        .map(|f| if f.value.is_missing_arg() { None } else { crate::compile::compile_thunk(&f.value) })
        .collect();
    // Keep the formals alive so their address is not reused (and bound the
    // cache: formals made at run time are new each time).
    DEFAULTS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 100_000 {
            c.clear();
        }
        c.insert(key, (formals.clone(), codes.clone()))
    });
    codes
}

thread_local! {
    static CACHE: RefCell<HashMap<usize, (Value, Rc<Code>), crate::env::FastHash>> = RefCell::new(HashMap::default());
    static INLINES: RefCell<HashMap<usize, (Rc<[Arg]>, Option<Rc<Inline>>), crate::env::FastHash>> = RefCell::new(HashMap::default());
    static DEFAULTS: RefCell<HashMap<usize, (Rc<[Arg]>, Rc<[Option<Rc<Code>>]>), crate::env::FastHash>> =
        RefCell::new(HashMap::default());
    /// The VM runs closures and loops unless `ROBIGO_COMPILE=0` (then only
    /// the AST interpreter runs).
    static ENABLED: bool = std::env::var("ROBIGO_COMPILE").map(|v| v != "0").unwrap_or(true);
    /// `ROBIGO_NO_INLINE=1`: never run closure bodies inline (to compare).
    static NO_INLINE: bool = std::env::var_os("ROBIGO_NO_INLINE").is_some();
}

/// Discard implementation-only owners before testing weak-key reachability.
pub fn clear_gc_caches() {
    clear_gc_caches_running();
    rjit::clear_gc_caches();
}

/// [`clear_gc_caches`] without the JIT's, for an automatic collection:
/// those caches hold compiled loops' bodies and environments, and clearing
/// them at every collection would recompile the hot loops after each one
/// (what they hold waits for an explicit `gc()`).
pub fn clear_gc_caches_running() {
    CACHE.with(|c| c.borrow_mut().clear());
    DEFAULTS.with(|c| c.borrow_mut().clear());
}

pub fn enabled() -> bool {
    ENABLED.with(|e| *e)
}

/// Compiled code for a closure body or loop, compiled on first use.
pub fn code_for(e: &Value, body: bool) -> Option<Rc<Code>> {
    let Value::Lang(l) = e else {
        // Constant and symbol bodies (`function() 1`, `function(x) x`,
        // `function() NULL`) compile to a single op, making the call
        // inlinable or at least direct. Compiled per closure (`clo.code`
        // caches it); there is no shared body object to key the cache on.
        return Some(Rc::new(if body { compile_body(e) } else { compile_loop(e)? }));
    };
    let key = Rc::as_ptr(l) as usize;
    if let Some(c) = CACHE.with(|c| c.borrow().get(&key).map(|(_, code)| code.clone())) {
        return Some(c);
    }
    let code = Rc::new(if body { compile_body(e) } else { compile_loop(e)? });
    if std::env::var_os("ROBIGO_VM_DUMP").is_some() {
        eprintln!(
            "== {} (regs {}, vars {}, guards {})",
            crate::deparse::deparse_one_line(e).chars().take(100).collect::<String>(),
            code.nregs,
            code.vars.len(),
            code.nguards
        );
        for (pc, op) in code.ops.iter().enumerate() {
            match op {
                Op::Eval { k, .. } => eprintln!(
                    "{:4} {:?}  # {}",
                    pc,
                    op,
                    crate::deparse::deparse_one_line(&code.consts[*k as usize]).chars().take(80).collect::<String>()
                ),
                _ => eprintln!("{:4} {:?}", pc, op),
            }
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
    /// The slots that may be dirty or cache a vector (bit `i` for slot `i`;
    /// bit 63 for slots 63 and above): those `flush` visits.
    touched: u64,
    /// `env::r_epoch()` when the cached (clean) values were valid: R code
    /// that ran since may have changed the variables.
    epoch: u64,
    /// The compiled caller takes a scalar result unboxed (see
    /// `run_closure`). (Here rather than a `run_with` parameter, which
    /// costs the dispatch loop a register.)
    unboxed: bool,
    /// An error of a flush made for native code, for the next `flush` to
    /// report (it comes right after, at the same op).
    failed: Option<Box<Flow>>,
}

impl Vars<'_> {
    /// Before R code may run: write pending assignments to `env`. Their
    /// values stay cached, as do other scalars (until R code actually
    /// runs, see `get`); cached vectors are dropped so that the binding
    /// holds the only reference and R code can modify it in place.
    #[inline(always)]
    fn flush(&mut self, interp: &mut Interp, env: &Env, syms: &[Sym]) -> RResult<()> {
        if !self.dirty && !self.vclean {
            return Ok(());
        }
        self.flush_pending(interp, env, syms)
    }

    /// The work of [`Self::flush`], out of line: the dispatch loop has a
    /// flush before every op that may run R code.
    #[inline(never)]
    fn flush_pending(&mut self, interp: &mut Interp, env: &Env, syms: &[Sym]) -> RResult<()> {
        if let Some(e) = self.failed.take() {
            return Err(*e);
        }
        self.dirty = false;
        self.vclean = false;
        let mut res = Ok(());
        let mut touched = std::mem::take(&mut self.touched);
        while touched != 0 {
            let i = touched.trailing_zeros() as usize;
            touched &= touched - 1;
            let end = if i == 63 { self.slots.len() } else { i + 1 };
            for (j, s) in self.slots[i..end].iter_mut().enumerate() {
                match std::mem::replace(s, SlotVal::Empty) {
                    SlotVal::Dirty(v) => {
                        if res.is_ok() {
                            res = set_var(interp, env, syms[i + j], &v);
                        }
                        if res.is_ok() && !matches!(v, RVal::V(_)) {
                            *s = SlotVal::Clean(v);
                        }
                    }
                    SlotVal::Clean(v) if !matches!(v, RVal::V(_)) => *s = SlotVal::Clean(v),
                    _ => {}
                }
            }
        }
        res
    }

    /// Note that `slot` may now be dirty or cache a vector.
    #[inline(always)]
    /// Whether a slot of `mask` (bits as in `touched`) has an assignment not
    /// yet written to the frame.
    fn any_pending(&self, mask: u64) -> bool {
        let mut m = self.touched & mask;
        while m != 0 {
            let i = m.trailing_zeros() as usize;
            m &= m - 1;
            // (Bit 63 stands for slots 63 and above.)
            let range = if i == 63 { 63..self.slots.len() } else { i..i + 1 };
            if self.slots[range].iter().any(|s| matches!(s, SlotVal::Dirty(_))) {
                return true;
            }
        }
        false
    }

    fn touch(&mut self, slot: usize) {
        self.touched |= 1 << slot.min(63);
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
            self.touch(slot);
        }
        self.slots[slot] = SlotVal::Clean(v);
        self.any = true;
    }
}

/// The value of `sym` if reading it cannot run code: a plain binding, or
/// an argument whose promise has been forced.
pub(crate) fn plain_lookup(env: &Env, sym: Sym) -> Option<RVal> {
    #[inline(always)]
    fn read(v: &Value) -> Option<RVal> {
        Some(match v {
            Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::D(x.data[0]),
            Value::Int(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::I(x.data[0]),
            Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::L(x.data[0]),
            v if v.is_missing_arg() => return None,
            v => RVal::V(v.clone()),
        })
    }
    plain_lookup_impl(env, sym, read, RVal::I, RVal::D, RVal::L)
}

/// Builtin arguments are boxed already: return their object directly,
/// without wrapping it in an RVal that the caller must unpack again.
pub(crate) fn plain_lookup_boxed(env: &Env, sym: Sym) -> Option<Value> {
    plain_lookup_impl(env, sym, |v| (!v.is_missing_arg()).then(|| v.clone()), Value::int, Value::real, Value::lgl_raw)
}

fn plain_lookup_impl<T>(
    env: &Env,
    sym: Sym,
    read: impl Fn(&Value) -> Option<T>,
    int: impl Fn(i32) -> T,
    real: impl Fn(f64) -> T,
    lgl: impl Fn(Lgl) -> T,
) -> Option<T> {
    // `None`: not bound in `e`; `Some(None)`: bound, but reading may run code.
    #[inline(always)]
    fn in_frame<T>(
        e: &Env,
        sym: Sym,
        read: &impl Fn(&Value) -> Option<T>,
        int: &impl Fn(i32) -> T,
        real: &impl Fn(f64) -> T,
        lgl: &impl Fn(Lgl) -> T,
    ) -> Option<Option<T>> {
        let frame = e.0.frame().borrow();
        let b = frame.get(sym)?;
        if b.flags & crate::env::BIND_ACTIVE != 0 {
            return Some(None);
        }
        Some(match &b.value {
            Value::Promise(p) => match &*p.value.borrow() {
                crate::value::PromiseVal::Boxed(v) => read(v),
                crate::value::PromiseVal::ScalarInt(x) => Some(int(*x)),
                crate::value::PromiseVal::ScalarReal(x) => Some(real(*x)),
                crate::value::PromiseVal::ScalarLgl(x) => Some(lgl(*x)),
                crate::value::PromiseVal::Unforced => None,
            },
            v => read(v),
        })
    }
    if let Some(r) = in_frame(env, sym, &read, &int, &real, &lgl) {
        return r;
    }
    let mut cur = env.parent()?;
    loop {
        if let Some(r) = in_frame(&cur, sym, &read, &int, &real, &lgl) {
            return r;
        }
        cur = cur.parent()?;
    }
}

pub(crate) fn unbox(v: Value) -> RVal {
    match &v {
        Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::D(x.data[0]),
        Value::Int(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::I(x.data[0]),
        Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::L(x.data[0]),
        _ => RVal::V(v),
    }
}

/// [`unbox`] of a borrowed value.
pub(crate) fn unbox_ref(v: &Value) -> RVal {
    match v {
        Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::D(x.data[0]),
        Value::Int(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::I(x.data[0]),
        Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() && Rc::weak_count(x) == 0 => RVal::L(x.data[0]),
        _ => RVal::V(v.clone()),
    }
}

/// The object a read of variable `sym` gives to whatever it escapes to (a
/// call's argument, the value of a promise), when `v` is what the VM read:
/// an unboxed scalar stands for the binding's own object, which native code
/// may write into and R then reads (GNU R hands out the binding's value,
/// boxing an unboxed binding cell into it the first time:
/// `R_expand_binding_value`). Pending assignments must have been flushed.
/// An object of its own when the binding no longer holds that value.
// R: envir.c:BINDING_VALUE
fn escaping_read(env: &Env, sym: Sym, v: RVal) -> Value {
    if let Some(b) = binding_object(env, sym) {
        let same = match (&b, &v) {
            (Value::Int(x), RVal::I(i)) => x.data.len() == 1 && x.data[0] == *i && x.attrs.is_none(),
            (Value::Real(x), RVal::D(d)) => {
                x.data.len() == 1 && x.data[0].to_bits() == d.to_bits() && x.attrs.is_none()
            }
            (Value::Lgl(x), RVal::L(l)) => x.data.len() == 1 && x.data[0] == *l && x.attrs.is_none(),
            _ => false,
        };
        if same {
            return b;
        }
    }
    v.to_binding()
}

/// [`call_builtin_op`] of a builtin calling native code (`.Call` & co).
#[inline(never)]
fn native_builtin_op(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    b: &'static Builtin,
    args: u32,
    call: u32,
) -> RResult<Value> {
    let argv = native_args(code, args, regs, env);
    let r = interp.invoke_builtin(b, &code.consts[call as usize], argv, env);
    signal(interp, r)
}

/// The arguments of a `CallBuiltin` of a builtin calling native code
/// (`Code::argvars`): a variable passed as it is is its binding's own
/// object.
#[inline(never)]
fn native_args(code: &Code, args: u32, regs: &mut [RVal], env: &Env) -> Vec<Arg> {
    debug_assert_eq!(code.arglists[args as usize].len(), code.argvars[args as usize].len());
    code.arglists[args as usize]
        .iter()
        .zip(&code.argvars[args as usize])
        .map(|((tag, r), var)| {
            let v = std::mem::replace(&mut regs[*r as usize], RVal::I(0));
            Arg::new(
                *tag,
                match (var, v) {
                    (Some(sym), v @ (RVal::I(_) | RVal::D(_) | RVal::L(_))) => escaping_read(env, *sym, v),
                    (_, RVal::V(v)) => v.for_binding(),
                    // (a computed value: an object of its own, never a
                    // shared small integer C could write into)
                    (_, v) => v.to_binding(),
                },
            )
        })
        .collect()
}

/// The object variable `sym` is bound to (a plain binding, or a forced
/// promise's value), expanded into an object of its own the first time it
/// is read as one (`Env::binding_value`, `Promise::value`), if reading it
/// runs no code; `None` otherwise (`interp::plain_value`'s cases, for the
/// variables the VM has just read).
#[inline]
fn binding_object(env: &Env, sym: Sym) -> Option<Value> {
    // (a variable's object: Env::binding_value, Promise::value)
    #[inline(always)]
    fn in_frame(e: &Env, sym: Sym) -> Option<Option<Value>> {
        let (v, flags) = e.binding_value(sym)?;
        if flags & crate::env::BIND_ACTIVE != 0 {
            return Some(None);
        }
        Some(match v {
            Value::Promise(p) => p.value(),
            v if v.is_missing_arg() => None,
            v => Some(v),
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
fn force_var(interp: &mut Interp, env: &Env, sym: Sym) -> RResult<(RVal, bool)> {
    let p = match env.0.frame().borrow().get(sym) {
        Some(b) if b.flags == 0 => match &b.value {
            Value::Promise(p) => Some(p.clone()),
            _ => None,
        },
        _ => None,
    };
    match p {
        // (taken unboxed: no expansion, and a scalar it computes is never
        // boxed)
        Some(p) => interp.force_promise_unboxed(&p).map(|v| (v, true)),
        None => interp.eval_sym(sym, env).map(|v| (unbox(v), false)),
    }
}

fn signal_var(interp: &mut Interp, r: RResult<(RVal, bool)>) -> RResult<(RVal, bool)> {
    match r {
        Err(Flow::Error(e)) if !e.signaled => Err(interp.signal_error(e)),
        r => r,
    }
}

/// An index value as `scalar_index` reads it.
#[inline(always)]
fn index_num(r: &RVal) -> Option<f64> {
    match r {
        RVal::D(x) => Some(*x),
        RVal::I(x) if *x != NA_INTEGER => Some(*x as f64),
        RVal::V(Value::Real(v)) if v.data.len() == 1 && v.attrs.is_none() => Some(v.data[0]),
        RVal::V(Value::Int(v)) if v.data.len() == 1 && v.attrs.is_none() && v.data[0] != NA_INTEGER => {
            Some(v.data[0] as f64)
        }
        _ => None,
    }
}

/// `x[[i]]` / `x[i]` through `Op::Index`'s fast path: `None` where it would
/// take the slow one.
#[inline]
pub(crate) fn index_value(x: &RVal, i: &RVal, two: bool) -> Option<RVal> {
    let i = index_num(i)?;
    // NaN fails both comparisons.
    let ix = (1.0..4503599627370496.0).contains(&i).then_some(i.trunc() - 1.0)?;
    element(x, ix as usize, two)
}

/// A 0-based index from an in-range positive scalar index.
#[inline(always)]
fn scalar_index(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], i: &Leaf) -> Option<f64> {
    let num = index_num;
    let x = match i {
        Leaf::D(x) => *x,
        Leaf::I(x) => *x as f64,
        Leaf::Var(sym, slot) => num(read_var(vars, env, *sym, *slot)?)?,
        Leaf::R(r) => num(&regs[*r as usize])?,
    };
    // NaN fails both comparisons.
    (1.0..4503599627370496.0).contains(&x).then_some(x.trunc() - 1.0)
}

/// How a variable index operand of `x[i]` / `x[i, j]` stands before its
/// fast path: `Some(true)` when it is an unforced promise of `env`'s own
/// frame that `[`/`[[` would evaluate (after `x`, if `x` is not an object:
/// R's `evalListKeepMissing`) rather than pass on as missing, so that the
/// VM may force it; `Some(false)` when it needs no forcing; `None` when it
/// may be missing (the general path decides).
// R: eval.c:R_DispatchOrEvalSP, envir.c:R_isMissing
fn index_promise(vars: &mut Vars<'_>, env: &Env, l: &Leaf) -> Option<bool> {
    let Leaf::Var(sym, slot) = l else { return Some(false) };
    if vars.get(*slot as usize).is_some() {
        return Some(false);
    }
    let frame = env.0.frame().borrow();
    let Some(b) = frame.get(*sym) else { return Some(false) };
    let Value::Promise(p) = &b.value else { return (!b.value.is_missing_arg()).then_some(false) };
    if b.flags != 0 {
        return None;
    }
    if !matches!(*p.value.borrow(), crate::value::PromiseVal::Unforced) {
        return Some(false);
    }
    match &p.expr {
        Value::Promise(_) => None,
        // R_isMissing follows a symbol into the promise's environment
        // (its own frame only).
        Value::Sym(s2) => {
            if s2.is_empty() || s2.ddval().is_some() || *s2 == s::DOTS {
                return None;
            }
            let penv = p.env.borrow();
            let penv = penv.as_ref()?;
            let pf = penv.0.frame().borrow();
            match pf.get(*s2) {
                None => Some(true),
                Some(b2) if b2.flags != 0 || b2.value.is_missing_arg() => None,
                Some(b2) => match &b2.value {
                    Value::Promise(q) if matches!(*q.value.borrow(), crate::value::PromiseVal::Unforced) => None,
                    _ => Some(true),
                },
            }
        }
        _ => Some(true),
    }
}

/// Force the variable index operands of `x[...]` (in order, as R
/// evaluates them) that [`index_promise`] allowed (`idx`: each with its
/// result), stopping at one that may be missing.
fn force_indices(interp: &mut Interp, vars: &mut Vars<'_>, env: &Env, idx: &[(&Leaf, Option<bool>)]) -> RResult<()> {
    for (l, state) in idx {
        match state {
            Some(true) => {
                let Leaf::Var(sym, slot) = l else { unreachable!() };
                let r = force_var(interp, env, *sym);
                let (v, cacheable) = signal_var(interp, r)?;
                if cacheable {
                    vars.sync();
                    vars.cache(*slot as usize, v);
                }
            }
            Some(false) => {}
            None => break,
        }
    }
    Ok(())
}

/// What a dead register holds: a scalar, which (unlike `RVal::V(NULL)`)
/// the next write into the register drops without a call. Nothing reads
/// it (as in the arithmetic ops, which clear their operands the same way).
pub(crate) const DEAD: RVal = RVal::I(0);

/// Move a register's value out, leaving it dead.
#[inline(always)]
fn take_reg(regs: &mut [RVal], r: Reg) -> RVal {
    std::mem::replace(&mut regs[r as usize], DEAD)
}

/// The general case of `Op::SetIndex`/`Op::SetIndex2`/`Op::SetDollar`
/// (`x[i] <- val`, `x[i, j] <- val`, `x$name <- val`: no `idx`; call
/// `consts[k]`), after a flush: the complex
/// assignment with `val`'s value and, where an index was evaluated into a
/// register (or, for two indices, any is), the indices' values.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn set_index_slow(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    vars: &mut Vars<'_>,
    slot: Slot,
    idx: &[&Leaf],
    val: Reg,
    k: u32,
    sup: bool,
    take: bool,
) -> RResult<()> {
    let whole = &code.consts[k as usize];
    let Value::Lang(l) = whole else { unreachable!() };
    if let ([i], false, Value::Lang(target)) = (idx, sup, &l.args[0].value)
        && target.fun_sym() == Some(s::BRACKET)
        && scatter_fast(vars, env, regs, (code.vars[slot as usize], slot), i, &regs[val as usize])
    {
        if take {
            take_reg(regs, val);
        }
        return Ok(());
    }
    let rhs = if take { take_reg(regs, val).into_value() } else { regs[val as usize].to_value() };
    let pre = match idx {
        [Leaf::R(r)] => Some(vec![regs[*r as usize].to_value()]),
        [_] => None,
        [i, j] if matches!(i, Leaf::R(_)) || matches!(j, Leaf::R(_)) => {
            let iv = leaf_value(interp, vars, env, regs, i)?.to_value();
            let jv = leaf_value(interp, vars, env, regs, j)?.to_value();
            Some(vec![iv, jv])
        }
        _ => None,
    };
    // R: eval.c:R_CheckJIT. Eager VM execution does not imply that GNU R
    // has byte-compiled this closure yet. Its interpreted setter calls
    // display the forced RHS value until that transition actually occurs.
    let source_rhs = !code.is_body
        || interp.frames.last().is_none_or(|ctx| match &ctx.function {
            Value::Closure(clo) => clo.jit.is_bytecode(),
            _ => true,
        });
    // (After the flush the slot only caches the binding: holding on to it
    // would share the object with the replacement call, which then copies.)
    if matches!(vars.slots[slot as usize], SlotVal::Clean(_)) {
        vars.slots[slot as usize] = SlotVal::Empty;
    }
    let r = crate::builtins::lang::complex_assign_pre(
        interp,
        whole,
        &l.args[0].value,
        &l.args[1].value,
        rhs,
        env,
        sup,
        pre.as_deref(),
        source_rhs.then_some(&l.args[1].value),
    );
    // The assignment rebound `x` in the frame.
    vars.slots[slot as usize] = SlotVal::Empty;
    signal(interp, r.map(|_| Value::Null)).map(|_| ())
}

/// An index vector `x[i]` and `x[i] <- v` take element by element: integer
/// or double, without attributes, every element in `1..=len`. Anything else
/// (NA, zero, negative, out of range) is the general path's.
enum Positions<'a> {
    I(&'a [i32]),
    D(&'a [f64]),
}

impl Positions<'_> {
    fn of(i: &Value, len: usize) -> Option<Positions<'_>> {
        let lim = len as f64 + 1.0;
        match i {
            Value::Int(v) if v.attrs.is_none() && v.data.iter().all(|&k| k >= 1 && (k as usize) <= len) => {
                Some(Positions::I(&v.data))
            }
            // (NaN fails the comparison.)
            Value::Real(v) if v.attrs.is_none() && v.data.iter().all(|&k| k >= 1.0 && k < lim) => {
                Some(Positions::D(&v.data))
            }
            _ => None,
        }
    }

    fn len(&self) -> usize {
        match self {
            Positions::I(v) => v.len(),
            Positions::D(v) => v.len(),
        }
    }

    /// The 0-based position of element `n`.
    #[inline(always)]
    fn at(&self, n: usize) -> usize {
        match self {
            Positions::I(v) => v[n] as usize - 1,
            Positions::D(v) => v[n] as usize - 1,
        }
    }
}

/// The value of an index operand that is a vector.
fn index_vector(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], i: &Leaf) -> Option<Value> {
    match i {
        Leaf::R(r) => match &regs[*r as usize] {
            RVal::V(v) => Some(v.clone()),
            _ => None,
        },
        Leaf::Var(sym, s) => match read_var(vars, env, *sym, *s)? {
            RVal::V(v) => Some(v.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// `x[i]` for an index vector of [`Positions`] and an attribute-free vector
/// `x`: the elements, as `[` returns them (R: subset.c:VectorSubset,
/// ExtractSubset). `None`: the general path.
fn gather_fast(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], (x, slot): (Sym, Slot), i: &Leaf) -> Option<RVal> {
    let iv = index_vector(vars, env, regs, i)?;
    let RVal::V(xv) = read_var(vars, env, x, slot)? else { return None };
    macro_rules! gather {
        ($x:expr, $ctor:path) => {{
            if $x.attrs.is_some() {
                return None;
            }
            let pos = Positions::of(&iv, $x.data.len())?;
            let out: Vec<_> = (0..pos.len()).map(|n| $x.data[pos.at(n)].clone()).collect();
            Some(RVal::V($ctor(Rc::new(crate::value::Vector::new(out)))))
        }};
    }
    match xv {
        Value::Real(x) => gather!(x, Value::Real),
        Value::Int(x) => gather!(x, Value::Int),
        Value::Lgl(x) => gather!(x, Value::Lgl),
        Value::Str(x) => gather!(x, Value::Str),
        _ => None,
    }
}

/// `x[i] <- v` for an index vector of [`Positions`], `x` an attribute-free
/// vector of this frame (made unshared) and `v` an attribute-free vector
/// whose length divides the index's (recycled, as R does without a warning
/// then): in place, coercing an integer `x` to double for a double `v`
/// (R: subassign.c:VectorAssign, SubassignTypeFix). `false` (nothing
/// done): the general path.
fn scatter_fast(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], (x, slot): (Sym, Slot), i: &Leaf, val: &RVal) -> bool {
    let Some(iv) = index_vector(vars, env, regs, i) else { return false };
    let v = val.to_value();
    let m = v.len();
    if m == 0 || v.attrs().is_some() {
        return false;
    }
    let len = match read_var(vars, env, x, slot) {
        Some(RVal::V(xv @ (Value::Real(_) | Value::Int(_) | Value::Lgl(_) | Value::Str(_))))
            if xv.attrs().is_none() =>
        {
            xv.len()
        }
        _ => return false,
    };
    let Some(pos) = Positions::of(&iv, len) else { return false };
    let fits = matches!(
        (read_var(vars, env, x, slot), &v),
        (Some(RVal::V(Value::Real(_))), Value::Real(_) | Value::Int(_) | Value::Lgl(_))
            | (Some(RVal::V(Value::Int(_))), Value::Real(_) | Value::Int(_) | Value::Lgl(_))
            | (Some(RVal::V(Value::Lgl(_))), Value::Lgl(_))
            | (Some(RVal::V(Value::Str(_))), Value::Str(_))
    );
    if !fits || pos.len() % m != 0 || !own_var(vars, env, x, slot) {
        return false;
    }
    let SlotVal::Dirty(RVal::V(xv)) = &mut vars.slots[slot as usize] else { return false };
    if let (Value::Int(xi), Value::Real(_)) = (&*xv, &v) {
        let d: Vec<f64> = xi.data.iter().map(|&k| crate::builtins::arith::int_to_real(k)).collect();
        *xv = Value::Real(Rc::new(crate::value::Vector::new(d)));
    }
    macro_rules! scatter {
        ($x:expr, $y:expr, $f:expr) => {{
            let x = Rc::make_mut($x);
            for n in 0..pos.len() {
                x.data[pos.at(n)] = $f(&$y.data[n % m]);
            }
        }};
    }
    let real = |k: &i32| crate::builtins::arith::int_to_real(*k);
    match (xv, &v) {
        (Value::Real(x), Value::Real(y)) => scatter!(x, y, |d: &f64| *d),
        (Value::Real(x), Value::Int(y)) => scatter!(x, y, real),
        (Value::Real(x), Value::Lgl(y)) => scatter!(x, y, real),
        (Value::Int(x), Value::Int(y)) => scatter!(x, y, |k: &i32| *k),
        (Value::Int(x), Value::Lgl(y)) => scatter!(x, y, |k: &i32| *k),
        (Value::Lgl(x), Value::Lgl(y)) => scatter!(x, y, |k: &i32| *k),
        (Value::Str(x), Value::Str(y)) => scatter!(x, y, |s: &crate::value::RStr| s.clone()),
        _ => unreachable!("checked by `fits`"),
    }
    true
}

/// The general case of `Op::Index` (`x[i]` / `x[[i]]`, call `consts[k]`),
/// after a flush. With `x` not an object, `[`/`[[` evaluate the index next
/// (R: eval.c:R_DispatchOrEvalSP): an index promise is forced here,
/// holding on to `x` as it was before.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn index_slow(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    vars: &mut Vars<'_>,
    (x, slot): (Sym, Slot),
    i: &Leaf,
    two: bool,
    k: u32,
) -> RResult<RVal> {
    let call = &code.consts[k as usize];
    if !two && let Some(v) = gather_fast(vars, env, regs, (x, slot), i) {
        return Ok(v);
    }
    if index_promise(vars, env, i) == Some(true) {
        if let Some(xv) = read_var(vars, env, x, slot).filter(|xv| !matches!(xv, RVal::V(v) if v.is_object())).cloned()
        {
            force_indices(interp, vars, env, &[(i, Some(true))])?;
            if let Some(v) = scalar_index(vars, env, regs, i).and_then(|ix| element(&xv, ix as usize, two)) {
                return Ok(v);
            }
            let ival = leaf_value(interp, vars, env, regs, i)?.into_value();
            let r = subset_values(interp, call, xv.into_value(), vec![ival], two, env);
            return signal(interp, r).map(RVal::V);
        }
    }
    match i {
        // The index was evaluated already: call `[`/`[[` with forced
        // promises instead of evaluating it again.
        Leaf::R(r) => {
            let ival = regs[*r as usize].to_value();
            let r = subset_with_index(interp, call, x, ival, two, env);
            signal(interp, r).map(RVal::V)
        }
        _ => interp.eval(call, env).map(RVal::V),
    }
}

/// The general case of `Op::Index2` (`x[i, j]` / `x[[i, j]]`), after a
/// flush; index promises as in [`index_slow`].
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn index2_slow(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    vars: &mut Vars<'_>,
    (x, slot): (Sym, Slot),
    i: &Leaf,
    j: &Leaf,
    two: bool,
    k: u32,
) -> RResult<RVal> {
    let call = &code.consts[k as usize];
    let ip = index_promise(vars, env, i);
    let jp = index_promise(vars, env, j);
    if ip.is_some() && jp.is_some() && (ip == Some(true) || jp == Some(true)) {
        if let Some(xv) = read_var(vars, env, x, slot).filter(|xv| !matches!(xv, RVal::V(v) if v.is_object())).cloned()
        {
            force_indices(interp, vars, env, &[(i, ip), (j, jp)])?;
            if let Some(v) = matrix_element_at(vars, env, regs, (x, slot, Some(&xv)), i, j) {
                return Ok(v);
            }
            let iv = leaf_value(interp, vars, env, regs, i)?.into_value();
            let jv = leaf_value(interp, vars, env, regs, j)?.into_value();
            let r = subset_values(interp, call, xv.into_value(), vec![iv, jv], two, env);
            return signal(interp, r).map(RVal::V);
        }
    }
    interp.eval(call, env).map(RVal::V)
}

/// Element `i` of a vector that `x[i]` (`[[` if `two`) reads without
/// dispatch, attribute handling or allocation beyond a string scalar.
#[inline(always)]
fn element(x: &RVal, i: usize, two: bool) -> Option<RVal> {
    let v = match x {
        RVal::V(v) => v,
        // An unmaterialized `a:b` (say `x <- 1:n`) is an integer vector.
        RVal::Range { start, step, n } => {
            return ((i as u64) < *n).then(|| RVal::I((*start + i as i64 * *step) as i32));
        }
        _ if i == 0 => return Some(x.clone()),
        _ => return None,
    };
    // `x[i]` keeps names and other attributes; `x[[i]]` drops them, but
    // classed objects dispatch.
    let plain = |attrs: &AttrBox| attrs.is_none() || (two && !v.is_object());
    // a package ALTREP vector is read through its class (subset2_dflt)
    let alt = || crate::altrep::lookup(v).is_some();
    Some(match v {
        Value::Real(x) if i < x.data.len() && plain(&x.attrs) && !alt() => RVal::D(x.data[i]),
        Value::Int(x) if i < x.data.len() && plain(&x.attrs) && !alt() => RVal::I(x.data[i]),
        Value::Lgl(x) if i < x.data.len() && plain(&x.attrs) && !alt() => RVal::L(x.data[i]),
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
    // One lookup for a plain binding of a value that is not function-like
    // (taking one of those changes function bindings: `Env::take_local`).
    let taken = {
        let mut frame = env.0.frame().borrow_mut();
        let Some(b) = frame.get_mut(sym) else { return false };
        if b.flags != 0 {
            return false;
        }
        match &b.value {
            Value::Promise(p) if p.value().is_none() => return false,
            v if v.is_missing_arg() => return false,
            Value::Closure(_) | Value::Builtin(_) | Value::Promise(_) => None,
            _ => Some(std::mem::replace(&mut b.value, Value::Null)),
        }
    };
    vars.slots[slot] = SlotVal::Empty;
    let v = match taken {
        Some(v) => v,
        None => match env.take_local(sym) {
            Some(Value::Promise(p)) => p.value().unwrap_or(Value::Null),
            Some(v) => v,
            None => return false,
        },
    };
    vars.slots[slot] = SlotVal::Dirty(RVal::V(v));
    vars.any = true;
    vars.dirty = true;
    vars.touch(slot);
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
    if step == 1 { d[first..first + n].to_vec() } else { (0..n).map(|k| d[first - k].clone()).collect() }
}

/// `x[a:b]` for an attribute-free vector and an in-range range.
fn range_subset(x: &RVal, start: i64, step: i64, n: u64) -> Option<RVal> {
    let RVal::V(x) = x else { return None };
    if crate::altrep::lookup(x).is_some() {
        return None;
    }
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
fn set_range(x: &mut Value, start: i64, step: i64, n: u64, val: &mut RVal, take: bool) -> bool {
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
    match (x, &*val) {
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
    // (The elements are copied: in effect position the value is dead.)
    if take {
        *val = DEAD;
    }
    true
}

pub(crate) fn set_element(x: &mut Value, i: usize, val: &mut RVal, two: bool, take: bool) -> bool {
    if i == x.len() && x.attrs().is_none() {
        return append_element(x, val, two, take);
    }
    if x.is_object() || i >= x.len() {
        return false;
    }
    match (x, &*val) {
        (Value::Real(v), RVal::D(d)) => Rc::make_mut(v).data[i] = *d,
        // `x[[i]] <- y` takes the element of a length-one `y` whatever its
        // attributes (a 1 x 1 matrix from `%*%`, say). R: subassign.c:SubAssign2
        (Value::Real(v), RVal::V(yv @ Value::Real(y))) if two && y.data.len() == 1 && !yv.is_object() => {
            Rc::make_mut(v).data[i] = y.data[0]
        }
        (Value::Real(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data[i] = int_to_real(*n),
        (Value::Int(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data[i] = *n,
        (Value::Lgl(v), RVal::L(n)) => Rc::make_mut(v).data[i] = *n,
        (Value::Str(v), RVal::V(Value::Str(s))) if s.data.len() == 1 && s.attrs.is_none() => {
            Rc::make_mut(v).data[i] = s.data[0].clone()
        }
        (Value::List(v), x) if two && !matches!(x, RVal::V(Value::Null)) => Rc::make_mut(v).data[i] = stored(val, take),
        _ => return false,
    }
    true
}

// The fast paths of the store ops, out of line (but `x[i] <- v`'s and
// `x[i, j] <- v`'s): they
// are large, and the dispatch loop's register allocation is shared by all
// its ops. Each
// returns whether it did the store; if not, the op flushes and takes the
// general path (`set_index_slow`). `take`: in effect position, the value
// register must not keep the value alive (#357; see `stored`).

/// `Op::SetIndex`: an in-range scalar or range index into a plain local
/// vector. (Inline: the commonest store, in hot loops.)
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn set_index_fast(
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    i: &Leaf,
    two: bool,
    take: bool,
) -> bool {
    let range = match (i, two) {
        (Leaf::R(r), false) => match regs[*r as usize] {
            RVal::Range { start, step, n } => Some((start, step, n)),
            _ => None,
        },
        _ => None,
    };
    match (range, scalar_index(vars, env, regs, i)) {
        (Some((start, step, n)), _) if own_var(vars, env, x, slot) => match &mut vars.slots[slot as usize] {
            SlotVal::Dirty(RVal::V(xv)) => set_range(xv, start, step, n, &mut regs[val as usize], take),
            _ => false,
        },
        (None, Some(ix)) if own_var(vars, env, x, slot) => match &mut vars.slots[slot as usize] {
            SlotVal::Dirty(RVal::V(xv)) => set_element(xv, ix as usize, &mut regs[val as usize], two, take),
            _ => false,
        },
        _ => false,
    }
}

/// `Op::SetIndex2`: in-range scalar indices into a plain local matrix.
/// (Inline: matrix stores are hot in numeric loops.)
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn set_index2_fast(
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    i: &Leaf,
    j: &Leaf,
    two: bool,
    take: bool,
) -> bool {
    match (scalar_index(vars, env, regs, i), scalar_index(vars, env, regs, j)) {
        (Some(a), Some(b)) if own_var(vars, env, x, slot) => match &mut vars.slots[slot as usize] {
            SlotVal::Dirty(RVal::V(xv)) => match matrix_offset(xv, a as usize, b as usize) {
                Some(off) => set_element(xv, off, &mut regs[val as usize], two, take),
                None => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// `Op::SetDollar`: an existing element of a plain local list.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn set_dollar_fast(
    code: &Code,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    name: u32,
    take: bool,
) -> bool {
    let Value::Str(nm) = &code.consts[name as usize] else { unreachable!() };
    own_var(vars, env, x, slot)
        && match &mut vars.slots[slot as usize] {
            SlotVal::Dirty(RVal::V(xv)) => set_list_field(xv, &nm.data[0], &mut regs[val as usize], take),
            _ => false,
        }
}

/// `Op::SuperSetIndex`: an element of the vector `x <<- ` would assign.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn super_set_index_fast(
    interp: &Interp,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    i: &Leaf,
    two: bool,
    take: bool,
) -> bool {
    // (The current frame's cached copy of `x` would make the vector look
    // shared, and may be stale afterwards.)
    if !matches!(vars.slots[slot as usize], SlotVal::Dirty(_)) {
        vars.slots[slot as usize] = SlotVal::Empty;
    }
    match scalar_index(vars, env, regs, i) {
        Some(ix) => super_set_element(interp, env, x, ix as usize, &mut regs[val as usize], two, take),
        None => false,
    }
}

/// The value a store puts into a list: in effect position (`take`) moved
/// out of its register, which must not keep it alive (#357).
#[inline(always)]
fn stored(val: &mut RVal, take: bool) -> Value {
    if take && let RVal::V(_) = val { std::mem::replace(val, DEAD).into_value() } else { val.to_value() }
}

/// `Op::IndexN`'s fast path (out of line, as the store ops' are): the slice
/// of a plain array by in-range scalar and empty indices.
#[inline(never)]
fn index_n_fast(
    code: &Code,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
    x: Sym,
    slot: Slot,
    idx: u32,
) -> Option<RVal> {
    let ix = slice_indices(vars, env, regs, &code.index_lists[idx as usize])?;
    read_var(vars, env, x, slot).and_then(|xv| array_slice(xv, &ix))
}

/// `Op::SetIndexN`'s fast path: in place on a plain local array. In effect
/// position (`take`) the value is dead once its elements are copied (#357).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn set_index_n_fast(
    code: &Code,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    idx: u32,
    take: bool,
) -> bool {
    let done = match slice_indices(vars, env, regs, &code.index_lists[idx as usize]) {
        Some(ix) if own_var(vars, env, x, slot) => match &mut vars.slots[slot as usize] {
            SlotVal::Dirty(RVal::V(xv)) => array_assign(xv, &ix, &regs[val as usize]),
            _ => false,
        },
        _ => false,
    };
    if done && take {
        regs[val as usize] = DEAD;
    }
    done
}

/// `Op::SuperSetIndexN`'s fast path: in place on the binding `<<-` assigns
/// (the frame's cached copy dropped first, as for `Op::SuperSetIndex`).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn super_set_index_n_fast(
    interp: &Interp,
    code: &Code,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
    val: Reg,
    x: Sym,
    slot: Slot,
    idx: u32,
    take: bool,
) -> bool {
    if !matches!(vars.slots[slot as usize], SlotVal::Dirty(_)) {
        vars.slots[slot as usize] = SlotVal::Empty;
    }
    let done = match slice_indices(vars, env, regs, &code.index_lists[idx as usize]) {
        Some(ix) => super_set_slice(interp, env, x, &ix, &regs[val as usize]),
        None => false,
    };
    if done && take {
        regs[val as usize] = DEAD;
    }
    done
}

/// The geometry of `x[i, , k]` on a plain array: the offset of the first
/// element, and (extent, stride) of each empty index in order.
struct Slice {
    base: usize,
    ext: SmallVec<[(usize, usize); 4]>,
    len: usize,
}

impl Slice {
    /// The elements are one block, `base..base + len` (`x[, , k]`: the
    /// empty indices come first): copied as a slice, not offset by offset.
    fn contiguous(&self) -> bool {
        let mut stride = 1;
        for (n, st) in &self.ext {
            if *st != stride {
                return false;
            }
            stride *= n;
        }
        true
    }
}

/// The extents of an atomic vector whose only attribute is `dim`.
fn plain_dims(attrs: &AttrBox) -> Option<&[i32]> {
    let a = attrs.as_ref()?;
    if a.1 || a.0.len() != 1 || a.0[0].0 != s::DIM {
        return None;
    }
    // (Extents with names, say from a deserialized array, name a slice's.)
    let Value::Int(d) = &a.0[0].1 else { return None };
    d.attrs.is_none().then_some(&d.data[..])
}

/// The scalar indices of an [`Op::IndexN`] (`None` for an empty one), all
/// within `[1, 2^52)` as 0-based offsets; `None` if one is not such a
/// scalar (or is a promise, a logical, NA, ...).
fn slice_indices(
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
    idx: &[Option<Leaf>],
) -> Option<SmallVec<[Option<usize>; 8]>> {
    let mut out = SmallVec::new();
    for l in idx {
        out.push(match l {
            None => None,
            Some(l) => Some(scalar_index(vars, env, regs, l)? as usize),
        });
    }
    Some(out)
}

/// The slice of an array with extents `dims` that in-range scalar and
/// empty indices select; `None` for another rank, an index out of range or
/// an empty result (the general path handles those).
// R: subset.c:ArraySubset, for indices that select one element or all.
fn slice_geom(dims: &[i32], idx: &[Option<usize>], n: usize) -> Option<Slice> {
    if dims.len() != idx.len() {
        return None;
    }
    let (mut base, mut stride, mut len) = (0usize, 1usize, 1usize);
    let mut ext = SmallVec::new();
    for (d, i) in dims.iter().zip(idx) {
        let d = usize::try_from(*d).ok().filter(|d| *d > 0)?;
        match i {
            None => {
                ext.push((d, stride));
                len *= d;
            }
            Some(i) if *i < d => base += i * stride,
            Some(_) => return None,
        }
        stride = stride.checked_mul(d)?;
    }
    (stride == n).then_some(Slice { base, ext, len })
}

/// Call `f` with the offset of each element of the slice, in R's order
/// (the first empty index varies fastest).
#[inline]
fn slice_offsets(g: &Slice, mut f: impl FnMut(usize)) {
    match g.ext.as_slice() {
        [] => f(g.base),
        [(n, st)] => (0..*n).for_each(|t| f(g.base + t * st)),
        ext => {
            let mut ctr: SmallVec<[usize; 8]> = SmallVec::from_elem(0, ext.len());
            for _ in 0..g.len {
                f(g.base + ctr.iter().zip(ext).map(|(c, (_, st))| c * st).sum::<usize>());
                for (c, (n, _)) in ctr.iter_mut().zip(ext) {
                    *c += 1;
                    if *c < *n {
                        break;
                    }
                    *c = 0;
                }
            }
        }
    }
}

/// `x[i, , k]` of a plain atomic array: a vector of the selected elements,
/// with the extents of the empty indices other than 1 as its `dim` when
/// two or more remain (R's `drop = TRUE`: subset.c:DropDims).
fn array_slice(x: &RVal, idx: &[Option<usize>]) -> Option<RVal> {
    let RVal::V(v) = x else { return None };
    macro_rules! slice {
        ($x:expr, $ctor:path, $one:path) => {{
            let g = slice_geom(plain_dims(&$x.attrs)?, idx, $x.data.len())?;
            if g.len == 1 {
                return Some($one($x.data[g.base]));
            }
            let mut data = Vec::with_capacity(g.len);
            if g.contiguous() {
                data.extend_from_slice(&$x.data[g.base..g.base + g.len]);
            } else {
                slice_offsets(&g, |o| data.push($x.data[o]));
            }
            let mut out = Vector::new(data);
            let keep: Vec<i32> = g.ext.iter().filter(|(n, _)| *n != 1).map(|(n, _)| *n as i32).collect();
            if keep.len() >= 2 {
                out.attrs = Some(Box::new(Attrs(vec![(s::DIM, Value::Int(Rc::new(Vector::new(keep))))], false, false)));
            }
            Some(RVal::V($ctor(Rc::new(out))))
        }};
    }
    match v {
        Value::Real(x) => slice!(x, Value::Real, RVal::D),
        Value::Int(x) => slice!(x, Value::Int, RVal::I),
        Value::Lgl(x) => slice!(x, Value::Lgl, RVal::L),
        _ => None,
    }
}

/// `x[i, , k] <- val` in place on a plain atomic array: a value of the
/// same or a lower type (logical < integer < double) without a class,
/// whose length divides the slice's (recycled). `false` (nothing done)
/// otherwise.
// R: subassign.c:ArrayAssign, for indices that select one element or all.
fn array_assign(x: &mut Value, idx: &[Option<usize>], val: &RVal) -> bool {
    macro_rules! assign {
        ($x:expr, $src:expr, $conv:expr) => {{
            let Some(g) = plain_dims(&$x.attrs).and_then(|d| slice_geom(d, idx, $x.data.len())) else { return false };
            let src = $src;
            if src.is_empty() || g.len % src.len() != 0 {
                return false;
            }
            let data = &mut Rc::make_mut($x).data;
            if g.contiguous() && src.len() == g.len {
                for (d, v) in data[g.base..g.base + g.len].iter_mut().zip(src) {
                    *d = $conv(*v);
                }
                return true;
            }
            let mut t = 0;
            slice_offsets(&g, |o| {
                data[o] = $conv(src[t]);
                t += 1;
                if t == src.len() {
                    t = 0;
                }
            });
            true
        }};
    }
    if matches!(val, RVal::V(v) if v.is_object()) {
        return false;
    }
    let one_i;
    let one_d;
    match (x, val) {
        (Value::Real(x), RVal::D(d)) => {
            one_d = [*d];
            assign!(x, &one_d[..], |v: f64| v)
        }
        (Value::Real(x), RVal::I(n) | RVal::L(n)) => {
            one_i = [*n];
            assign!(x, &one_i[..], int_to_real)
        }
        (Value::Real(x), RVal::V(Value::Real(y))) => assign!(x, &y.data[..], |v: f64| v),
        (Value::Real(x), RVal::V(Value::Int(y) | Value::Lgl(y))) => {
            assign!(x, &y.data[..], int_to_real)
        }
        (Value::Int(x), RVal::I(n) | RVal::L(n)) => {
            one_i = [*n];
            assign!(x, &one_i[..], |v: i32| v)
        }
        (Value::Int(x), RVal::V(Value::Int(y) | Value::Lgl(y))) => assign!(x, &y.data[..], |v: i32| v),
        (Value::Lgl(x), RVal::L(n)) => {
            one_i = [*n];
            assign!(x, &one_i[..], |v: i32| v)
        }
        (Value::Lgl(x), RVal::V(Value::Lgl(y))) => assign!(x, &y.data[..], |v: i32| v),
        _ => false,
    }
}

/// `x[i, , k] <<- val` in place on the binding `<<-` assigns (see
/// [`super_set_element`]).
fn super_set_slice(interp: &Interp, env: &Env, sym: Sym, idx: &[Option<usize>], val: &RVal) -> bool {
    let mut cur = env.parent();
    let target = loop {
        match cur {
            Some(e) if e.has_local(sym) => break e,
            Some(e) => cur = e.parent(),
            None => break interp.global.clone(),
        }
    };
    let mut frame = target.0.frame().borrow_mut();
    let Some(b) = frame.get_mut(sym) else { return false };
    if b.flags != 0 || matches!(b.value, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_)) {
        return false;
    }
    array_assign(&mut b.value, idx, val)
}

/// A leaf operand as it stands, without running code (a constant, a
/// register, a variable's cached or plain value); `None` otherwise.
#[inline]
fn leaf_peek<'a>(vars: &'a mut Vars<'_>, env: &Env, regs: &'a [RVal], l: &Leaf) -> Option<&'a RVal> {
    match l {
        Leaf::R(r) => Some(&regs[*r as usize]),
        Leaf::Var(sym, slot) => read_var(vars, env, *sym, *slot),
        Leaf::D(_) | Leaf::I(_) => None,
    }
}

/// The [`Shape`] of a leaf operand as it stands (see [`leaf_peek`]).
#[inline]
fn leaf_shape(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], l: &Leaf) -> Shape {
    match l {
        Leaf::D(d) => shape(&RVal::D(*d)),
        Leaf::I(i) => shape(&RVal::I(*i)),
        _ => leaf_peek(vars, env, regs, l).map_or(OTHER, shape),
    }
}

/// Whether a first operand may qualify for [`pure_plan`] (inline: a cheap
/// test that spares complex vectors and other values a call).
#[inline(always)]
fn may_be_pure(v: &RVal) -> bool {
    match v {
        RVal::D(_) | RVal::I(_) | RVal::L(_) | RVal::V(Value::Real(_)) => true,
        RVal::V(Value::Cplx(z)) => z.data.len() == 1,
        _ => false,
    }
}

/// [`may_be_pure`] for a leaf, from what is at hand (a variable's cached
/// value; one not cached may be).
#[inline(always)]
fn leaf_may_be_pure(vars: &Vars<'_>, regs: &[RVal], l: &Leaf) -> bool {
    match l {
        Leaf::D(_) | Leaf::I(_) => true,
        Leaf::R(r) => may_be_pure(&regs[*r as usize]),
        Leaf::Var(_, s) => match &vars.slots[*s as usize] {
            SlotVal::Clean(v) | SlotVal::Dirty(v) => may_be_pure(v),
            _ => true,
        },
    }
}

/// `Op::Arith` on registers without a flush where [`pure_plan`] allows it;
/// `None` (nothing done) otherwise. Out of line: the dispatch loop's
/// register allocation is shared by all its ops.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn reg_arith_pure(
    code: &Code,
    pc: usize,
    dst: Reg,
    op: ArithOp,
    a: Reg,
    b: Reg,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> Option<Slow> {
    match pure_plan(op, shape(&regs[a as usize]), shape(&regs[b as usize])) {
        Pure::Val(v) => {
            regs[a as usize] = RVal::I(0);
            regs[b as usize] = RVal::I(0);
            Some(Slow::Val(v))
        }
        Pure::Dbl => {
            let x = std::mem::replace(&mut regs[a as usize], RVal::I(0));
            let y = std::mem::replace(&mut regs[b as usize], RVal::I(0));
            Some(dbl_arith(code, pc, dst, op, x, y, vars, env, regs))
        }
        Pure::No => None,
    }
}

/// `Op::ArithL` without a flush where [`pure_plan`] allows it (see
/// [`reg_arith_pure`]).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn leaf_arith_pure(
    code: &Code,
    pc: usize,
    dst: Reg,
    op: ArithOp,
    a: &Leaf,
    b: &Leaf,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &mut [RVal],
) -> Option<Slow> {
    let sa = leaf_shape(vars, env, regs, a);
    if !sa.any() {
        return None;
    }
    match pure_plan(op, sa, leaf_shape(vars, env, regs, b)) {
        Pure::Val(v) => {
            for l in [a, b] {
                if let Leaf::R(r) = l {
                    regs[*r as usize] = RVal::I(0);
                }
            }
            Some(Slow::Val(v))
        }
        Pure::Dbl => {
            let x = leaf_take(vars, env, regs, a);
            let y = leaf_take(vars, env, regs, b);
            Some(dbl_arith(code, pc, dst, op, x, y, vars, env, regs))
        }
        Pure::No => None,
    }
}

/// The value of a leaf [`pure_plan`] accepted as a double (a register is
/// taken, so that an unshared temporary can hold the result).
fn leaf_take(vars: &mut Vars<'_>, env: &Env, regs: &mut [RVal], l: &Leaf) -> RVal {
    match l {
        Leaf::D(d) => RVal::D(*d),
        Leaf::R(r) => std::mem::replace(&mut regs[*r as usize], RVal::I(0)),
        _ => leaf_peek(vars, env, regs, l).cloned().expect("checked by pure_plan"),
    }
}

/// `forceAndCall`: force the first `n` promise arguments.
// R: eval.c:R_forceAndCall
fn force_args(interp: &mut Interp, promargs: &[Arg], n: u32) -> RResult<()> {
    for a in promargs.iter().take(n as usize) {
        if let Value::Promise(p) = &a.value {
            // Already forced: forcing again only re-boxes the scalar.
            if !matches!(&*p.value.borrow(), crate::value::PromiseVal::Unforced) {
                continue;
            }
            interp.force_promise_raw(p)?;
        }
    }
    Ok(())
}

/// `x$name` for a list or expression vector by exact name, without
/// warnings; `None` where `R_subset3_dflt` must run (partial matches,
/// environments, other types).
fn dollar_fast(x: &Value, input: &RStr) -> Option<Value> {
    let (Value::List(v) | Value::Expr(v)) = x else { return None };
    let input: &str = input.as_deref().unwrap_or("NA");
    if input.is_empty() {
        return None;
    }
    let names = v.attrs.as_ref().and_then(|_| x.get_attr(s::NAMES));
    let Some(Value::Str(n)) = names else { return Some(Value::Null) };
    let mut partial = false;
    for (i, nm) in n.data.iter().enumerate() {
        let nm: &str = nm.as_deref().unwrap_or("NA");
        if nm.starts_with(input) {
            if nm.len() == input.len() {
                return Some(v.data[i].clone());
            }
            partial = true;
        }
    }
    (!partial).then_some(Value::Null)
}

/// The fast case of `@` for an S4 object with an existing slot (other than `.Data`).
#[inline]
fn slot_read_fast(x: &Value, name: Sym) -> Option<Value> {
    if name == crate::s4::syms::data() || !x.is_s4() {
        return None;
    }
    match x.get_attr(name) {
        Some(Value::Sym(p)) if p == crate::s4::syms::pseudo_null() => Some(Value::Null),
        Some(v) => Some(v),
        None => None,
    }
}

/// The offset of element `[i, j]` (0-based) of a matrix that is not an
/// object, if in range.
fn matrix_offset(x: &Value, i: usize, j: usize) -> Option<usize> {
    if x.is_object() {
        return None;
    }
    let Some(Value::Int(d)) = x.get_attr(s::DIM) else { return None };
    let [nr, nc] = d.data[..] else { return None };
    let (nr, nc) = (nr as usize, nc as usize);
    (i < nr && j < nc).then_some(i + j * nr)
}

/// `x[i, j]` (0-based, in range) of a plain matrix, as a scalar.
fn matrix_element(x: &RVal, i: usize, j: usize) -> Option<RVal> {
    let RVal::V(v) = x else { return None };
    let k = matrix_offset(v, i, j)?;
    Some(match v {
        Value::Real(x) => RVal::D(x.data[k]),
        Value::Int(x) => RVal::I(x.data[k]),
        Value::Lgl(x) => RVal::L(x.data[k]),
        Value::Str(x) => RVal::V(Value::strs(vec![x.data[k].clone()])),
        _ => return None,
    })
}

/// [`matrix_element`] of the variable `x` (`held`: of that value) at
/// scalar indices `i`, `j`, without running code.
#[inline(always)]
fn matrix_element_at(
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
    (x, slot, held): (Sym, Slot, Option<&RVal>),
    i: &Leaf,
    j: &Leaf,
) -> Option<RVal> {
    let (a, b) = (scalar_index(vars, env, regs, i)?, scalar_index(vars, env, regs, j)?);
    match held {
        Some(xv) => matrix_element(xv, a as usize, b as usize),
        None => matrix_element(read_var(vars, env, x, slot)?, a as usize, b as usize),
    }
}

/// `x$name <- val` in place: an existing element (first exact match of a
/// non-NA name, as R's `$<-` finds it) of a list that is not an object,
/// and a value other than NULL (which deletes).
fn set_list_field(x: &mut Value, name: &RStr, val: &mut RVal, take: bool) -> bool {
    if x.is_object() || matches!(val, RVal::V(Value::Null)) {
        return false;
    }
    let Some(name) = name.as_deref() else { return false };
    let Some(Value::Str(names)) = x.get_attr(s::NAMES) else { return false };
    let Some(i) = names.data.iter().position(|n| n.as_deref() == Some(name)) else { return false };
    let Value::List(v) = x else { return false };
    Rc::make_mut(v).data[i] = stored(val, take);
    true
}

/// `x[length(x) + 1] <- val` for a vector without attributes and a value
/// of its type: append (the vector's buffer grows geometrically, as R's
/// growable vectors do), instead of reallocating it through `[<-`.
fn append_element(x: &mut Value, val: &mut RVal, two: bool, take: bool) -> bool {
    if x.len() >= i32::MAX as usize {
        return false;
    }
    match (x, &*val) {
        (Value::Real(v), RVal::D(d)) => Rc::make_mut(v).data.push(*d),
        (Value::Real(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data.push(int_to_real(*n)),
        (Value::Int(v), RVal::I(n) | RVal::L(n)) => Rc::make_mut(v).data.push(*n),
        (Value::Lgl(v), RVal::L(n)) => Rc::make_mut(v).data.push(*n),
        (Value::Str(v), RVal::V(Value::Str(s))) if s.data.len() == 1 && s.attrs.is_none() => {
            Rc::make_mut(v).data.push(s.data[0].clone())
        }
        (Value::List(v), x) if two && !matches!(x, RVal::V(Value::Null)) => {
            Rc::make_mut(v).data.push(stored(val, take))
        }
        _ => return false,
    }
    true
}

thread_local! {
    /// `typeof` results, shared.
    static TYPE_NAMES: RefCell<HashMap<&'static str, Value>> = RefCell::new(HashMap::default());
}

fn type_name_value(name: &'static str) -> Value {
    TYPE_NAMES.with(|t| t.borrow_mut().entry(name).or_insert_with(|| Value::str(name)).clone())
}

/// [`Builtin1`] without a call: for values that are not objects, or
/// whatever the builtin does not dispatch on. `None`: call the builtin.
pub(crate) fn builtin1_fast(f: Builtin1, x: &RVal) -> Option<RVal> {
    let v = match x {
        RVal::V(v) => v,
        RVal::D(d) => {
            return Some(match f {
                Builtin1::Length => RVal::I(1),
                Builtin1::IsNa => RVal::L(d.is_nan() as Lgl),
                Builtin1::IsAtomic | Builtin1::IsNumeric => RVal::L(TRUE),
                Builtin1::Attributes => RVal::V(Value::Null),
                Builtin1::Typeof => RVal::V(type_name_value("double")),
                _ => RVal::L(FALSE),
            });
        }
        RVal::I(i) | RVal::L(i) => {
            let lgl = matches!(x, RVal::L(_));
            return Some(match f {
                Builtin1::Length => RVal::I(1),
                Builtin1::IsNa => RVal::L((*i == NA_INTEGER) as Lgl),
                Builtin1::IsAtomic => RVal::L(TRUE),
                Builtin1::IsNumeric => RVal::L(!lgl as Lgl),
                Builtin1::IsLogical => RVal::L(lgl as Lgl),
                Builtin1::Attributes => RVal::V(Value::Null),
                Builtin1::Typeof => RVal::V(type_name_value(if lgl { "logical" } else { "integer" })),
                _ => RVal::L(FALSE),
            });
        }
        // An integer vector without attributes or NAs. Always answered for
        // `typeof`: its builtin is an .Internal, not a base binding.
        RVal::Range { n, .. } => {
            return Some(match f {
                Builtin1::Length => RVal::I(i32::try_from(*n).ok()?),
                Builtin1::IsNa if *n == 1 => RVal::L(FALSE),
                Builtin1::IsNa => return None,
                Builtin1::IsAtomic | Builtin1::IsNumeric => RVal::L(TRUE),
                Builtin1::Attributes => RVal::V(Value::Null),
                Builtin1::Typeof => RVal::V(type_name_value("integer")),
                _ => RVal::L(FALSE),
            });
        }
    };
    let l = |b: bool| Some(RVal::L(b as Lgl));
    match f {
        Builtin1::IsNull => l(v.is_null()),
        Builtin1::IsAtomic => l(v.is_atomic()),
        Builtin1::IsCharacter => l(matches!(v, Value::Str(_))),
        Builtin1::IsLogical => l(matches!(v, Value::Lgl(_))),
        Builtin1::IsFunction => l(v.is_function()),
        Builtin1::IsList => l(matches!(v, Value::List(_) | Value::Pairlist(_))),
        Builtin1::Typeof => Some(RVal::V(type_name_value(v.type_name()))),
        _ if v.is_object() => None,
        Builtin1::Length => {
            let n = v.len();
            (n <= i32::MAX as usize).then_some(RVal::I(n as i32))
        }
        Builtin1::IsNumeric => l(matches!(v, Value::Int(_) | Value::Real(_))),
        Builtin1::IsNa => {
            let na = match v {
                Value::Real(x) if x.data.len() == 1 && x.attrs.is_none() => x.data[0].is_nan(),
                Value::Int(x) | Value::Lgl(x) if x.data.len() == 1 && x.attrs.is_none() => x.data[0] == NA_INTEGER,
                Value::Str(x) if x.data.len() == 1 && x.attrs.is_none() => x.data[0].is_none(),
                _ => return None,
            };
            l(na)
        }
        Builtin1::Attributes => match v {
            Value::Null | Value::Lgl(_) | Value::Int(_) | Value::Real(_) | Value::Str(_) | Value::List(_)
                if v.attrs().is_none() =>
            {
                Some(RVal::V(Value::Null))
            }
            _ => None,
        },
    }
}

/// Scalar string `==`/`!=` (R compares strings by bytes for equality).
fn str_eq_fast(op: RelOp, a: &RVal, b: &RVal) -> Option<RVal> {
    if !matches!(op, RelOp::Eq | RelOp::Ne) {
        return None;
    }
    let (RVal::V(Value::Str(x)), RVal::V(Value::Str(y))) = (a, b) else { return None };
    if x.data.len() != 1 || y.data.len() != 1 || x.attrs.is_some() || y.attrs.is_some() {
        return None;
    }
    Some(RVal::L(match (&x.data[0], &y.data[0]) {
        (Some(p), Some(q)) => ((p == q) == (op == RelOp::Eq)) as Lgl,
        _ => NA_LOGICAL,
    }))
}

/// Unboxed `sqrt(x)` etc.; `None` where the builtin must run (it warns
/// about NaNs produced, or the input is not a plain number).
pub(crate) fn math1_fast(f: Math1, x: Num) -> Option<RVal> {
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
fn andor_operand(interp: &mut Interp, v: &Value, and: bool, is_y: bool, call: &Value) -> RResult<Lgl> {
    // a deferred package ALTREP value is read now (crate::altrep)
    if let Some(f) = crate::altrep::force(interp, v)? {
        return andor_operand(interp, &f, and, is_y, call);
    }
    let is_number = matches!(v, Value::Lgl(_) | Value::Real(_) | Value::Cplx(_))
        || (matches!(v, Value::Int(_)) && !v.inherits("factor"));
    if !is_number {
        return Err(Flow::error_call(
            call.clone(),
            format!("invalid '{}' type in 'x {} y'", if is_y { "y" } else { "x" }, if and { "&&" } else { "||" }),
        ));
    }
    // asLogical2 reads the scalar through an ALTREP class (LOGICAL_ELT etc.)
    if v.len() == 1
        && let Some(h) = crate::altrep::lookup(v)
    {
        let e = crate::altrep::elt(interp, h, 0)?;
        return andor_operand(interp, &e, and, is_y, call);
    }
    crate::builtins::summary::as_logical2(v, true, call)
}

/// `x[i]` / `x[[i]]` with the index already evaluated: the subset builtin
/// gets forced promises, so nothing is evaluated twice and methods still
/// see the expressions.
fn subset_with_index(interp: &mut Interp, call: &Value, x: Sym, ival: Value, two: bool, env: &Env) -> RResult<Value> {
    let xval = interp.eval_sym(x, env)?;
    subset_values(interp, call, xval, vec![ival], two, env)
}

/// Call `[`/`[[` for `call` (`x[i]`, `x[i, j]`) with the value of `x` and
/// those of the indices, or (none given, `x[i]` only) a promise to
/// evaluate the index.
fn subset_values(
    interp: &mut Interp,
    call: &Value,
    xval: Value,
    ivals: Vec<Value>,
    two: bool,
    env: &Env,
) -> RResult<Value> {
    let Value::Lang(l) = call else { unreachable!() };
    let mut args = Vec::with_capacity(l.args.len());
    args.push(Arg::new(l.args[0].tag, Value::Promise(Rc::new(Promise::forced(l.args[0].value.clone(), xval)))));
    if ivals.is_empty() {
        args.push(Arg::new(
            l.args[1].tag,
            Value::Promise(Rc::new(Promise::arg(l.args[1].value.clone(), env.clone(), None, interp.bc))),
        ));
    }
    for (a, v) in l.args[1..].iter().zip(ivals) {
        args.push(Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced(a.value.clone(), v)))));
    }
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
                // An unshared attribute-free scalar of the same type is
                // updated in place.
                macro_rules! scalar {
                    ($rc:expr, $x:expr) => {
                        match Rc::get_mut($rc) {
                            Some(v) if v.data.len() == 1 && v.attrs.is_none() => {
                                v.data[0] = *$x;
                                true
                            }
                            _ => false,
                        }
                    };
                }
                let done = match (&mut b.value, val) {
                    (Value::Real(rc), RVal::D(x)) => scalar!(rc, x),
                    (Value::Int(rc), RVal::I(x)) => scalar!(rc, x),
                    (Value::Lgl(rc), RVal::L(x)) => scalar!(rc, x),
                    // (a shared small integer like an unboxed one)
                    (Value::Int(rc), RVal::V(v)) => match v.shared_int() {
                        Some(x) => scalar!(rc, &x),
                        None => false,
                    },
                    _ => false,
                };
                if done {
                    return Ok(());
                }
                // Any other plain binding of a value that is not
                // function-like (those change function bindings: see
                // `Env::set_local_flags`) is overwritten in place.
                if !matches!(b.value, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_))
                    && !matches!(val, RVal::V(Value::Closure(_) | Value::Builtin(_) | Value::Promise(_)))
                {
                    let prev = std::mem::replace(&mut b.value, val.to_value());
                    drop(frame);
                    drop(prev);
                    return Ok(());
                }
            }
        }
    }
    let r = interp.define(sym, val.to_value(), env);
    signal(interp, r.map(|_| Value::Null)).map(|_| ())
}

/// `x[i] <<- val` / `x[[i]] <<- val` (`two`) in place, without running
/// code: the binding `x <<- ` would assign (see [`super_set_fast`]) is a
/// plain binding of a vector [`set_element`] can update. `false` (nothing
/// done) otherwise.
fn super_set_element(interp: &Interp, env: &Env, sym: Sym, i: usize, val: &mut RVal, two: bool, take: bool) -> bool {
    let mut cur = env.parent();
    let target = loop {
        match cur {
            Some(e) if e.has_local(sym) => break e,
            Some(e) => cur = e.parent(),
            None => break interp.global.clone(),
        }
    };
    let mut frame = target.0.frame().borrow_mut();
    let Some(b) = frame.get_mut(sym) else { return false };
    if b.flags != 0 || matches!(b.value, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_)) {
        return false;
    }
    set_element(&mut b.value, i, val, two, take)
}

/// `sym <<- val` without running code, as `Interp::super_assign` would do
/// it: the first enclosing frame of `env` binding `sym` (or the global
/// environment) holds a plain binding of a value that is not function-like,
/// and `val` is not either (those change function bindings, see
/// `Env::set_local_flags`): an unshared scalar of `val`'s type is updated
/// in place, anything else replaced. `false` (nothing done) otherwise.
fn super_set_fast(interp: &Interp, env: &Env, sym: Sym, val: &mut RVal, take: bool) -> bool {
    if matches!(val, RVal::V(Value::Closure(_) | Value::Builtin(_) | Value::Promise(_)) | RVal::Range { .. }) {
        return false;
    }
    let mut cur = env.parent();
    while let Some(e) = cur {
        if e.has_local(sym) {
            return set_plain_in_place(&e, sym, val, take);
        }
        cur = e.parent();
    }
    set_plain_in_place(&interp.global, sym, val, take)
}

/// Whether [`super_set_fast`] of a scalar to `sym` from `env` cannot fail:
/// the binding it assigns exists and is plain.
fn super_target_plain(interp: &Interp, env: &Env, sym: Sym) -> bool {
    let mut cur = env.parent();
    while let Some(e) = cur {
        if e.has_local(sym) {
            return e.plain_binding(sym);
        }
        cur = e.parent();
    }
    interp.global.plain_binding(sym)
}

/// A writable view of the vector of the binding `x[[i]] <<- v` assigns
/// from `env` (see [`super_set_element`]), unshared first: a plain binding
/// of a vector of native type `ty` that is not an object.
fn super_vec_view(interp: &Interp, env: &Env, sym: Sym, ty: crate::jit::JType) -> Option<crate::jit::VecView> {
    use crate::jit::JType;
    let mut cur = env.parent();
    let target = loop {
        match cur {
            Some(e) if e.has_local(sym) => break e,
            Some(e) => cur = e.parent(),
            None => break interp.global.clone(),
        }
    };
    let mut frame = target.0.frame().borrow_mut();
    let b = frame.get_mut(sym)?;
    if b.flags != 0 || b.value.is_object() || jit_type(&RVal::V(b.value.clone())) != Some(ty) {
        return None;
    }
    macro_rules! view {
        ($x:expr) => {{
            let (nrow, ncol) = jit_matrix_dims($x).unwrap_or((0, 0));
            let v = Rc::make_mut($x);
            Some(crate::jit::VecView { ptr: v.data.as_mut_ptr() as *mut u8, len: v.data.len() as u64, nrow, ncol })
        }};
    }
    match (&mut b.value, ty) {
        (Value::Real(x), JType::VecD | JType::MatD) => view!(x),
        (Value::Int(x), JType::VecI | JType::MatI) => view!(x),
        _ => None,
    }
}

/// Overwrite the existing plain binding `sym` of `env`'s own frame with
/// `val` (see [`super_set_fast`]; `take`: moved out, see [`stored`]).
pub(crate) fn set_plain_in_place(env: &Env, sym: Sym, val: &mut RVal, take: bool) -> bool {
    let mut frame = env.0.frame().borrow_mut();
    let Some(b) = frame.get_mut(sym) else { return false };
    if b.flags != 0 || matches!(b.value, Value::Closure(_) | Value::Builtin(_) | Value::Promise(_)) {
        return false;
    }
    macro_rules! scalar {
        ($rc:expr, $x:expr) => {
            match Rc::get_mut($rc) {
                Some(v) if v.data.len() == 1 && v.attrs.is_none() => {
                    v.data[0] = *$x;
                    true
                }
                _ => false,
            }
        };
    }
    let done = match (&mut b.value, &*val) {
        (Value::Real(rc), RVal::D(x)) => scalar!(rc, x),
        (Value::Int(rc), RVal::I(x)) => scalar!(rc, x),
        (Value::Lgl(rc), RVal::L(x)) => scalar!(rc, x),
        (Value::Int(rc), RVal::V(v)) => match v.shared_int() {
            Some(x) => scalar!(rc, &x),
            None => false,
        },
        _ => false,
    };
    if !done {
        let prev = std::mem::replace(&mut b.value, stored(val, take));
        drop(frame);
        drop(prev);
    }
    true
}

/// The length of the integer sequence `seq(length.out = n)` returns, when
/// the call would reach base's `seq.default` (no `seq.integer`,
/// `seq.numeric`, `seq.double` or `seq.logical` method visible from `env`
/// or registered, and `seq.default` itself base's) and `n` passes its
/// checks: logical and integer `n` as they are, doubles rounded up, and
/// then finite, non-negative and within integer range, the result being
/// `seq_len(n)` (or `integer()` for 0). `None`: the call is made.
// R: seq.R:seq.default (`One <- nargs() == 1L`, `is.logint`, the
// `length.out` checks)
fn seq_length_range(interp: &mut Interp, env: &Env, n: Num) -> Option<u64> {
    let (len, classes): (f64, &[&str]) = match n {
        Num::I(x) if x != NA_INTEGER => (x as f64, &["seq.integer", "seq.numeric"]),
        Num::L(x) if x != NA_LOGICAL => (x as f64, &["seq.logical"]),
        Num::D(x) => (x.ceil(), &["seq.double", "seq.numeric"]),
        _ => return None,
    };
    if !len.is_finite() || len < 0.0 || len > i32::MAX as f64 {
        return None;
    }
    reaches_seq_default(interp, env, classes).then_some(len as u64)
}

/// Whether `seq` called from `env` on an object of implicit class `classes`
/// reaches base's `seq.default` (no method of those classes visible from
/// `env` or registered, and `seq.default` itself base's).
fn reaches_seq_default(interp: &mut Interp, env: &Env, classes: &[&str]) -> bool {
    let defenv = interp.base_ns.clone();
    for m in classes {
        match interp.lookup_method(Sym::new(m), env, &defenv) {
            Ok(None) => {}
            _ => return false,
        }
    }
    let dflt = Sym::new("seq.default");
    let Some(base) = interp.base_binding(dflt) else { return false };
    // base's S3 methods table registers its own seq.default: loaded from
    // base.rdb, a separately unserialized copy of the binding's closure
    let registered = match interp.base_binding(crate::sym::s::DOT_S3_METHODS_TABLE) {
        Some(Value::Env(t)) => t.get_local(dflt).and_then(|v| interp.force(v).ok()),
        _ => None,
    };
    matches!(interp.lookup_method(dflt, env, &defenv),
        Ok(Some(f)) if f.ptr_eq(&base) || registered.as_ref().is_some_and(|r| f.ptr_eq(r) && registered_is_base(r, &base)))
}

thread_local! {
    /// The `seq.default` last found in base's methods table, and whether it
    /// is base's own.
    static SEQ_DEFAULT_SEEN: RefCell<Option<(Value, bool)>> = const { RefCell::new(None) };
}

/// Whether `f`, the `seq.default` of base's methods table, is base's own
/// (a copy of the binding's closure, unserialized separately), not one a
/// later `registerS3method` put there. Remembered for the value last seen
/// (held, so its address is not reused).
fn registered_is_base(f: &Value, base: &Value) -> bool {
    SEQ_DEFAULT_SEEN.with(|seen| {
        if let Some((v, ok)) = &*seen.borrow()
            && v.ptr_eq(f)
        {
            return *ok;
        }
        let ok = crate::builtins::identical::identical(f, base);
        *seen.borrow_mut() = Some((f.clone(), ok));
        ok
    })
}

thread_local! {
    /// `seq.integer`, `seq.numeric`, `seq.default`.
    static SEQ_INT_METHODS: [Sym; 3] = [Sym::new("seq.integer"), Sym::new("seq.numeric"), Sym::new("seq.default")];
    /// The search epoch in which `seq` on an integer, called from frames
    /// below the global environment that bind none of those, was last seen
    /// to reach base's `seq.default`.
    static SEQ_INT_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
}

/// [`reaches_seq_default`] for an integer, as `seq` called from `env`
/// dispatches on it. Method lookup searches `env` up to its top environment,
/// the registered methods, then base: for function frames below the global
/// environment, what depends on the global environment and base is
/// memoized by the search epoch (which their function bindings bump), and
/// the frames and base's methods table are checked each time.
fn seq_integer_default(interp: &mut Interp, env: &Env) -> bool {
    let names = SEQ_INT_METHODS.with(|n| *n);
    let epoch = crate::env::search_epoch();
    let below_global = || {
        let mut cur = env.clone();
        while !cur.ptr_eq(&interp.global) {
            if cur.is_search() || names.iter().any(|m| cur.local_may_bind_fun(*m)) {
                return false;
            }
            match cur.parent() {
                Some(p) => cur = p,
                None => return false,
            }
        }
        true
    };
    let plain = interp.topenv(env).ptr_eq(&interp.global)
        && below_global()
        && match interp.base_binding(crate::sym::s::DOT_S3_METHODS_TABLE) {
            Some(Value::Env(t)) => {
                !t.has_local(names[0])
                    && !t.has_local(names[1])
                    && t.get_local(names[2])
                        .is_none_or(|d| interp.base_binding(names[2]).is_some_and(|b| registered_is_base(&d, &b)))
            }
            _ => false,
        };
    if plain && SEQ_INT_EPOCH.with(|e| e.get()) == epoch {
        return true;
    }
    let ok = reaches_seq_default(interp, env, &["seq.integer", "seq.numeric"]);
    if plain && ok {
        SEQ_INT_EPOCH.with(|e| e.set(epoch));
    }
    ok
}

/// The arguments of `Op::ForSeqBy` when they are integers (not NA)
/// computed without warnings or dispatch (the guards hold).
#[inline(never)]
fn seq_by_args(args: &[crate::compile::SeqArg; 3], vars: &mut Vars<'_>, env: &Env, regs: &[RVal]) -> Option<[i32; 3]> {
    let mut v = [0i32; 3];
    for (j, a) in args.iter().enumerate() {
        let x = leaf_num(vars, env, regs, &a.a);
        let n = match a.op {
            None => x,
            Some(op) => arith_fast(op, x?, leaf_num(vars, env, regs, &a.b)?).as_ref().and_then(scalar),
        };
        match n {
            Some(Num::I(i)) if i != NA_INTEGER => v[j] = i,
            _ => return None,
        }
    }
    Some(v)
}

/// The integer range `seq(from, to, by)` returns for integer (not NA)
/// arguments when the call reaches base's `seq.default`: `(start, step,
/// length)`. `None` where it would signal an error (a zero or wrong-signed
/// `by`, too many elements) or dispatch: the call is made.
// R: seq.R:seq.default (the `by` branch with `int` true: `del == 0`
// returns `from` or `to`, both `from`; else `n <- as.integer(del/by)` and
// `from + (0L:n) * by`, or the same values by `cumsum`)
#[inline(never)]
fn seq_by_range(interp: &mut Interp, env: &Env, from: i32, to: i32, by: i32) -> Option<(i64, i64, u64)> {
    if !seq_integer_default(interp, env) {
        return None;
    }
    let del = to as f64 - from as f64;
    if del == 0.0 {
        return Some((from as i64, 1, 1));
    }
    if by == 0 {
        return None;
    }
    let n = del / by as f64;
    if n < 0.0 || n > i32::MAX as f64 {
        return None;
    }
    Some((from as i64, by as i64, n as u64 + 1))
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
#[inline(always)]
pub(crate) fn guard_ok(interp: &mut Interp, sym: Sym, env: &Env, memo: &std::cell::Cell<GuardMemo>) -> bool {
    let epoch = crate::env::search_epoch();
    let m = memo.get();
    if m.epoch == epoch && m.env == env.id() && m.version == env.fn_version() {
        return m.ok;
    }
    guard_ok_slow(interp, sym, env, memo)
}

#[inline(never)]
fn guard_ok_slow(interp: &mut Interp, sym: Sym, env: &Env, memo: &std::cell::Cell<GuardMemo>) -> bool {
    let epoch = crate::env::search_epoch();
    let m = memo.get();
    if m.epoch == epoch {
        if m.env == env.id() && m.version == env.fn_version() {
            return m.ok;
        }
        // A new frame (another call) directly below the memoized search-path
        // environment: only its own bindings can differ.
        if m.search != 0
            && env.parent_ref().as_ref().is_some_and(|p| p.id() == m.search)
            && !env.local_may_bind_fun(sym)
        {
            // (This frame's next checks are the fast path's.)
            memo.set(GuardMemo { env: env.id(), version: env.fn_version(), ..m });
            return m.ok;
        }
        // A new frame of a closure defined in the memoized call frame.
        if m.outer != 0
            && env.parent_ref().as_ref().is_some_and(|p| p.id() == m.outer && p.fn_version() == m.outer_version)
            && !env.local_may_bind_fun(sym)
        {
            return m.ok;
        }
    }
    // The common cases without reference count traffic: `env` on the search
    // path, or a closure frame whose parent is.
    if env.is_search() {
        let ok = if m.epoch == epoch && m.search == env.id() { m.ok } else { interp.resolves_to_base(sym, env) };
        memo.set(GuardMemo {
            epoch,
            env: env.id(),
            version: env.fn_version(),
            search: env.id(),
            ok,
            outer: 0,
            outer_version: 0,
        });
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
                memo.set(GuardMemo {
                    epoch,
                    env: env.id(),
                    version: env.fn_version(),
                    search: p.id(),
                    ok,
                    outer: 0,
                    outer_version: 0,
                });
                return ok;
            }
            Some(_) => {}
            None => return false,
        }
    }
    let Some(mut cur) = env.parent() else { return false };
    let mut depth = 1;
    // Whether every environment past `env`'s parent, up to where the lookup
    // stops, is locked (namespaces, their imports, the base namespace): it
    // cannot gain a binding, so the outcome depends only on the parent's
    // function bindings and `env`'s own. (GNU R's namespace code is
    // compiled with base functions inlined anyway.)
    let mut locked_above = true;
    while !cur.is_search() {
        if cur.local_may_bind_fun(sym) {
            let ok = interp.resolves_to_base(sym, env);
            if locked_above {
                // Frames of namespace closures: memoize the namespace.
                let (outer, outer_version) = match &*env.parent_ref() {
                    Some(p) => (p.id(), p.fn_version()),
                    None => (0, 0),
                };
                memo.set(GuardMemo { epoch, env: 0, version: 0, search: 0, ok, outer, outer_version });
            }
            return ok;
        }
        cur = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
        depth += 1;
        if !cur.is_search() && !cur.0.locked.get() {
            locked_above = false;
        }
    }
    let ok = if m.epoch == epoch && m.search == cur.id() { m.ok } else { interp.resolves_to_base(sym, &cur) };
    // The per-environment memo is only sound when no frame other than
    // `env` lies between it and the search path.
    let (env_id, version) = if depth <= 1 { (env.id(), env.fn_version()) } else { (0, 0) };
    // Two frames below the search path, or more with locked ones between:
    // memoize the outer one.
    let (outer, outer_version) = match &*env.parent_ref() {
        Some(p) if depth == 2 || locked_above => (p.id(), p.fn_version()),
        _ => (0, 0),
    };
    memo.set(GuardMemo { epoch, env: env_id, version, search: cur.id(), ok, outer, outer_version });
    ok
}

/// A closure whose calls can run inline: see [`inline_info`].
pub struct Inline {
    pub(crate) code: Rc<Code>,
    /// The slot of each formal in `code.vars` (`None`: unused).
    pub(crate) formal_slots: Vec<Option<Slot>>,
    /// Every function the body guards, and the (search epoch, environment
    /// id, its function-binding version) for which all of them were last
    /// seen to resolve to base from the closure's environment (one on the
    /// search path or directly below it, where that depends on nothing
    /// else).
    pub(crate) guard_syms: Vec<Sym>,
    guards_key: std::cell::Cell<(u64, usize, u32)>,
    /// The search epoch in which a nested call inside the body was last
    /// seen not to be inlinable (a builtin target, an unreadable
    /// argument, a callee that is not itself inlinable): while it holds,
    /// the body's calls go straight to the real path, as they did before
    /// nested inlining. `u64::MAX` never holds. Depth-budget exhaustion
    /// does not set it (recursion reaches the budget on purpose).
    no_nested: std::cell::Cell<u64>,
    /// The body contains a call (only such bodies pay for the nested
    /// inline machinery and its failure memo).
    pub(crate) has_calls: bool,
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
/// takes its fast path; [`run_inline_with`] gives up before anything else.
#[cold]
#[inline(never)]
pub fn inline_info(clo: &Closure) -> Option<Rc<Inline>> {
    if crate::typeprof::enabled() || NO_INLINE.with(|n| *n) {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    // Closures created by one `function` expression (the same body, so the
    // same bytecode) share the analysis, which depends on the formals only
    // through their names (it requires them to have no defaults).
    // (Constant and symbol bodies get a fresh `Code` per closure instance,
    // so the shared cache cannot key on it; `clo.inline` memoizes those.)
    let key = Rc::as_ptr(&code) as usize;
    let shared = matches!(&clo.body, Value::Lang(_));
    let same_formals = |f: &[Arg]| {
        f.len() == clo.formals.len()
            && f.iter()
                .zip(clo.formals.iter())
                .all(|(a, b)| a.tag == b.tag && a.value.is_missing_arg() == b.value.is_missing_arg())
    };
    if shared {
        if let Some(r) = INLINES.with(|c| c.borrow().get(&key).filter(|(f, _)| same_formals(f)).map(|(_, i)| i.clone()))
        {
            return r;
        }
    }
    let r = inline_analysis(clo, code.clone(), false);
    if shared {
        // (The code is kept alive by the bytecode cache, so its address is
        // not reused.)
        INLINES.with(|c| c.borrow_mut().insert(key, (clo.formals.clone(), r.clone())));
    }
    r
}

/// [`inline_info`] for a closure that native loops may inline although the
/// VM does not: its body may also read elements (`x[[i]]`, `x[i, j]`) of
/// free variables, which native code views at the loop's entry.
fn loop_inline_info(clo: &Closure) -> Option<Rc<Inline>> {
    if crate::typeprof::enabled() || NO_INLINE.with(|n| *n) {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    inline_analysis(clo, code, true)
}

#[cold]
fn inline_analysis(clo: &Closure, code: Rc<Code>, loops: bool) -> Option<Rc<Inline>> {
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
            | Op::Builtin1 { .. }
            | Op::Call { .. }
            | Op::JumpIfNot { .. }
            | Op::JumpIf { .. }
            | Op::Jump { .. }
            | Op::Visible { .. }
            | Op::Return { .. } => {}
            Op::Guard { sym, .. } | Op::ArithL { sym, .. } | Op::RelL { sym, .. } if !formals.contains(sym) => {}
            Op::Index { .. } | Op::Index2 { .. } if loops => {}
            _ => return None,
        }
    }
    let formal_slots = formals.iter().map(|f| code.vars.iter().position(|v| v == f).map(|p| p as Slot)).collect();
    let free_vars = code.vars.iter().any(|v| !formals.contains(v));
    let has_calls = code.ops.iter().any(|o| matches!(o, Op::Call { .. }));
    Some(Rc::new(Inline {
        code,
        formal_slots,
        guard_syms,
        guards_key: std::cell::Cell::new((u64::MAX, 0, 0)),
        no_nested: std::cell::Cell::new(u64::MAX),
        has_calls,
        free_vars,
    }))
}

/// Run an inlinable closure body (see [`inline_info`]) with argument values,
/// in place of a call: `env` is the closure's environment (free
/// variables and guards resolve there, as from the call's frame, which
/// would hold only the formals). `None` as soon as a step would need its
/// slow path (a guard failing, a non-scalar, NA in a condition, an
/// unforced variable): nothing observable has happened, so the caller
/// then makes the real call.
///
/// The value of argument `k` is `arg(interp, k)`, asked for (after the
/// guards) only if the body uses that formal.
#[inline(never)]
/// [`run_inline_with`] with the arguments as values. The recursion of
/// nested calls goes through here so every level instantiates the same
/// closure type (a direct recursion would grow one per nesting level).
fn run_inline_args(interp: &mut Interp, info: &Inline, env: &Env, args: &[RVal], depth: u32) -> Option<RVal> {
    // The recursion of nested calls goes through here so every level
    // instantiates the same closure type (a direct recursion would grow
    // one per nesting level).
    if info.has_calls {
        run_inline_nested(interp, info, env, |_, k| Some(args[k].clone()), depth)
    } else {
        run_inline_with(interp, info, env, |_, k| Some(args[k].clone()), depth)
    }
}

/// The `Op::Call` of an inline run ([`run_inline_with`]): the target is
/// resolved and, when it is itself pure-inlinable, run inline too;
/// anything else gives up before anything observable has happened, and
/// the caller re-runs the whole call for real. Structural failures
/// (a builtin target, an unreadable argument, a callee that is not
/// itself inlinable) are remembered per body (`Inline::no_nested`) so
/// they cost nothing on later calls; depth-budget exhaustion is not
/// (recursion reaches it on purpose).
#[cold]
#[inline(never)]
fn nested_call_inline(
    interp: &mut Interp,
    info: &Inline,
    code: &Code,
    env: &Env,
    slots: &mut [Option<RVal>],
    regs: &mut [RVal],
    dst: Reg,
    f: Sym,
    site: u32,
    depth: u32,
) -> Option<()> {
    fn read(slots: &mut [Option<RVal>], env: &Env, sym: Sym, slot: Slot) -> Option<RVal> {
        let slot = slot as usize;
        if slots[slot].is_none() {
            slots[slot] = Some(plain_lookup(env, sym)?);
        }
        slots[slot].clone()
    }
    if depth == 0 || !interp.call_depth_available() {
        return None;
    }
    let no_nested = |info: &Inline| {
        info.no_nested.set(crate::env::search_epoch());
    };
    let cs = &code.call_sites[site as usize];
    let Some(leaves) = cs.inline_args.as_ref() else {
        no_nested(info);
        return None;
    };
    let Some(Value::Closure(clo)) = call_site_fun_fast(interp, f, cs, env) else {
        no_nested(info);
        return None;
    };
    if leaves.len() != clo.formals.len() || clo.jit.inline_blocked() {
        no_nested(info);
        return None;
    }
    let Some(info2) = clo.inline.get_or_init(|| inline_info(&clo)).as_ref() else {
        no_nested(info);
        return None;
    };
    if info2.no_nested.get() == crate::env::search_epoch() {
        no_nested(info);
        return None;
    }
    // The arguments read this level's slots and registers; they are
    // evaluated in call order before the nested run.
    let mut args: SmallVec<[RVal; 4]> = SmallVec::new();
    for a in leaves {
        let v = match a {
            InlineArg::Leaf(Leaf::D(x)) => RVal::D(*x),
            InlineArg::Leaf(Leaf::I(x)) => RVal::I(*x),
            InlineArg::Leaf(Leaf::Var(sym, slot)) => read(slots, env, *sym, *slot)?,
            InlineArg::Leaf(Leaf::R(r)) => regs[*r as usize].clone(),
            InlineArg::Arith { op, a, b, sym, memo } => {
                if !guard_ok(interp, *sym, env, memo) {
                    return None;
                }
                let mut num = |l: &Leaf| match l {
                    Leaf::D(x) => Some(Num::D(*x)),
                    Leaf::I(x) => Some(Num::I(*x)),
                    Leaf::Var(sym, slot) => scalar(&read(slots, env, *sym, *slot)?),
                    Leaf::R(r) => scalar(&regs[*r as usize]),
                };
                arith_fast(*op, num(a)?, num(b)?)?
            }
            InlineArg::Index { x, slot, i, two, sym, memo } => {
                if !guard_ok(interp, *sym, env, memo) {
                    return None;
                }
                let iv = match i {
                    Leaf::D(v) => RVal::D(*v),
                    Leaf::I(v) => RVal::I(*v),
                    Leaf::Var(sym, slot) => read(slots, env, *sym, *slot)?,
                    Leaf::R(r) => regs[*r as usize].clone(),
                };
                index_value(&read(slots, env, *x, *slot)?, &iv, *two)?
            }
        };
        args.push(v);
    }
    interp.depth += 1;
    let r = run_inline_args(interp, info2, &clo.env, &args, depth - 1);
    interp.depth -= 1;
    regs[dst as usize] = r?;
    Some(())
}

macro_rules! inline_runner {
    ($name:ident, $nested:literal $(, $attr:meta)?) => {
        $(#[$attr])?
        fn $name(
            interp: &mut Interp,
            info: &Inline,
            env: &Env,
            mut arg: impl FnMut(&mut Interp, usize) -> Option<RVal>,
            depth: u32,
        ) -> Option<RVal> {
            let code = &*info.code;
            // All guards at once, once per search epoch, when that suffices.
            let mut guarded = false;
            if env.is_search() || env.parent_ref().as_ref().is_some_and(|p| p.is_search()) || locked_to_search(env) {
                let key = (crate::env::search_epoch(), env.id(), env.fn_version());
                if info.guards_key.get() != key && info.guard_syms.iter().all(|s| interp.resolves_to_base(*s, env)) {
                    info.guards_key.set(key);
                }
                guarded = info.guards_key.get() == key;
            }
            let mut regs_buf: SmallVec<[RVal; 16]> = SmallVec::new();
            let mut slots_buf: SmallVec<[Option<RVal>; 8]> = SmallVec::new();
            fill_empty(&mut regs_buf, code.nregs, || RVal::V(Value::Null));
            fill_empty(&mut slots_buf, code.vars.len(), || None);
            // Plain slices: indexing a SmallVec tests where its data is each time.
            let regs: &mut [RVal] = &mut regs_buf;
            let slots: &mut [Option<RVal>] = &mut slots_buf;
            for (k, slot) in info.formal_slots.iter().enumerate() {
                if let Some(s) = slot {
                    slots[*s as usize] = Some(arg(interp, k)?);
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
                    Op::Null { dst, .. } => regs[*dst as usize] = RVal::V(Value::Null),
                    Op::GetVar { dst, sym, slot } => regs[*dst as usize] = read(slots, env, *sym, *slot)?,
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
                        let x = leaf(slots, regs, a)?;
                        let y = leaf(slots, regs, b)?;
                        regs[*dst as usize] = arith_fast(*op, x, y)?;
                    }
                    Op::RelL { op, dst, a, b, sym, g, .. } => {
                        if !guarded && *g != NO_GUARD && !guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]) {
                            return None;
                        }
                        let x = leaf(slots, regs, a)?;
                        let y = leaf(slots, regs, b)?;
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
                            Num::I(x) | Num::L(x) => {
                                RVal::L(if x == NA_INTEGER { NA_LOGICAL } else { (x == 0) as Lgl })
                            }
                        };
                    }
                    Op::Math1 { f, dst, a, .. } => regs[*dst as usize] = math1_fast(*f, scalar(&regs[*a as usize])?)?,
                    Op::Builtin1 { f, dst, a, .. } => {
                        // A traced builtin must print where GNU R's GETBUILTIN
                        // would: leave this to the interpreter.
                        if crate::builtins::debug::any_prim_traced() {
                            return None;
                        }
                        regs[*dst as usize] = builtin1_fast(*f, &regs[*a as usize])?;
                    }
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
                    // A call whose target is itself pure-inlinable runs inline
                    // too; anything else gives up before anything observable
                    // has happened, and the caller re-runs the call for real.
                    // Out of line: the op loop's codegen is hot (the call
                    // machinery lives in `nested_call_inline`).
                    Op::Call { dst, f, site } => {
                        if $nested {
                            nested_call_inline(interp, info, code, env, slots, regs, *dst, *f, *site, depth)?;
                        } else {
                            return None;
                        }
                    }
                    Op::Return { src } => {
                        return Some(std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null)));
                    }
                    _ => return None,
                }
            }
            Some(std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null)))
        }
    };
}

inline_runner!(run_inline_with, false, inline);
inline_runner!(run_inline_nested, true);

/// How deep inline runs may nest calls of further pure-inlinable
/// closures (each level is one Rust frame; beyond this the real call is
/// made). Deep enough for the recursion of benchmarks, small enough that
/// the stack cost of an abandoned attempt stays bounded.
const INLINE_DEPTH: u32 = 12;

/// Run a call of the pure closure `clo` (see [`inline_info`]) with the
/// arguments `leaves` of its call site inline, if they can be read without
/// running code: `None` when the call has to be made.
#[inline(never)]
fn inline_site_call(
    interp: &mut Interp,
    clo: &Closure,
    leaves: &[InlineArg],
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
) -> Option<RVal> {
    // (Until GNU R's JIT would have decided about the closure, its calls
    // are counted: see `rjit::jit_check`. A body whose nested calls were
    // seen not to be inlinable is declined by the caller's check.)
    if leaves.len() != clo.formals.len() || clo.jit.inline_blocked() {
        return None;
    }
    let info = clo.inline.get_or_init(|| inline_info(clo)).as_ref()?;
    if info.free_vars && vars.dirty {
        return None;
    }
    // (The call made if this gives up sets it too.)
    interp.visible = true;
    if !info.has_calls {
        return run_inline_with(interp, info, &clo.env, |interp, k| site_arg(interp, &leaves[k], vars, env, regs), 0);
    }
    // As GNU R counts every call, an inline run over the limit gives up
    // and lets the real call raise the error at the same depth.
    if !interp.call_depth_available() {
        return None;
    }
    interp.depth += 1;
    let r = run_inline_nested(
        interp,
        info,
        &clo.env,
        |interp, k| site_arg(interp, &leaves[k], vars, env, regs),
        INLINE_DEPTH,
    );
    interp.depth -= 1;
    r
}

/// The value of an argument of a call site, without running code.
#[inline(always)]
fn site_arg(interp: &mut Interp, a: &InlineArg, vars: &mut Vars<'_>, env: &Env, regs: &[RVal]) -> Option<RVal> {
    Some(match a {
        InlineArg::Leaf(Leaf::D(x)) => RVal::D(*x),
        InlineArg::Leaf(Leaf::I(x)) => RVal::I(*x),
        InlineArg::Leaf(Leaf::Var(sym, slot)) => read_var(vars, env, *sym, *slot)?.clone(),
        InlineArg::Leaf(Leaf::R(_)) => return None,
        // Evaluated where the call would force its promise, but only
        // by a scalar fast path, which has no effects.
        InlineArg::Arith { op, a, b, sym, memo } => {
            if !guard_ok(interp, *sym, env, memo) {
                return None;
            }
            let x = leaf_num(vars, env, regs, a)?;
            let y = leaf_num(vars, env, regs, b)?;
            arith_fast(*op, x, y)?
        }
        InlineArg::Index { x, slot, i, two, sym, memo } => {
            if !guard_ok(interp, *sym, env, memo) {
                return None;
            }
            let iv = leaf_rval(vars, env, regs, i)?;
            index_value(read_var(vars, env, *x, *slot)?, &iv, *two)?
        }
    })
}

/// A leaf's value, if reading it cannot run code.
#[inline(always)]
fn leaf_rval(vars: &mut Vars<'_>, env: &Env, regs: &[RVal], l: &Leaf) -> Option<RVal> {
    Some(match l {
        Leaf::D(x) => RVal::D(*x),
        Leaf::I(x) => RVal::I(*x),
        Leaf::Var(sym, slot) => read_var(vars, env, *sym, *slot)?.clone(),
        Leaf::R(r) => regs[*r as usize].clone(),
    })
}

/// Whether a call of `clo` may run inline: not for a closure already known
/// not to be pure, or one whose nested calls were seen not to be inlinable,
/// nor for a traced one, see `inline_site_call`.
#[inline(always)]
fn inlineable(clo: &Closure) -> bool {
    match clo.inline.get() {
        None => true,
        Some(None) => false,
        Some(Some(info)) => info.no_nested.get() != crate::env::search_epoch(),
    }
}

/// Make the call of `clo` at `site` without a frame, if it can be: in native
/// code, or inline (see [`inline_info`]). Out of line: what `run_with` has
/// around a call costs its dispatch loop.
#[inline(never)]
fn frameless_call(
    interp: &mut Interp,
    caller: &Code,
    site: &CallSite,
    clo: &Rc<Closure>,
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
) -> Option<RVal> {
    let leaves = site.inline_args.as_ref()?;
    if site.no_native.get() != Rc::as_ptr(&clo.formals) as *const Arg as usize
        && let Some(v) = native_site_call(interp, caller, site, clo, leaves, vars, env, regs)
    {
        return Some(v);
    }
    if !inlineable(clo) {
        return None;
    }
    inline_site_call(interp, clo, leaves, vars, env, regs)
}

/// Run a call of `clo` at a call site as native code ([`crate::jit::closure`]),
/// if it has (or now gets) such code for these argument types: `None` when
/// the call has to be made.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn native_site_call(
    interp: &mut Interp,
    caller: &Code,
    site: &CallSite,
    clo: &Rc<Closure>,
    leaves: &[InlineArg],
    vars: &mut Vars<'_>,
    env: &Env,
    regs: &[RVal],
) -> Option<RVal> {
    let Some(Some(code)) = clo.code.get() else { return None };
    if code.native.never() || leaves.len() != clo.formals.len() || leaves.len() > crate::jit::closure::MAX_ARGS {
        // (Later calls from here skip this attempt.)
        site.no_native.set(Rc::as_ptr(&clo.formals) as *const Arg as usize);
        return None;
    }
    if !code.native.counted() {
        return None;
    }
    let mut args: SmallVec<[RVal; crate::jit::closure::MAX_ARGS]> = SmallVec::new();
    for a in leaves {
        match site_arg(interp, a, vars, env, regs) {
            Some(v) => args.push(v),
            None => {
                code.native.outcome(false);
                return None;
            }
        }
    }
    use crate::jit::closure::Run;
    // Pending assignments of this frame matter only if the tree reads one of
    // those variables (as a top-level loop's variable `for (j in ...) f()`
    // is not read by `f`).
    let dirty = vars.dirty
        && code.native.reads_frames()
        && vars.any_pending(code.native.reads_slots(&site.native_reads, &caller.vars));
    // A closure defined in this frame, as usual for one reading its
    // variables, sees its pending assignments: they go first. (Other
    // closures of the tree are checked by `call`.)
    if dirty
        && crate::jit::closure::on_chain(&clo.env, env)
        && let Err(e) = vars.flush(interp, env, &caller.vars)
    {
        vars.failed = Some(Box::new(e));
        vars.dirty = true;
        return None;
    }
    let mut r = crate::jit::closure::call(interp, clo, code, &args, leaves, dirty.then_some(env));
    if let Run::Flush = r {
        // The tree reads variables of this frame: pending assignments go
        // first. (The VM does the same before any call, which this may be.)
        if let Err(e) = vars.flush(interp, env, &caller.vars) {
            vars.failed = Some(Box::new(e));
            vars.dirty = true;
            return None;
        }
        r = crate::jit::closure::call(interp, clo, code, &args, leaves, None);
    }
    let v = match r {
        Run::Done(v) => Some(v),
        _ => None,
    };
    code.native.outcome(v.is_some());
    v
}

/// R: compiler/R/cmp.R:safeBaseInternals, cmpSimpleInternal,
/// inlineSimpleInternalCall, cmpBuiltinArgs. Installed base bytecode
/// inlines these simple `.Internal` wrappers as direct internal calls:
/// guard-free in locked namespaces, `BASEGUARD`-checked in user code.
/// The definition is resolved once per call site (GNU R compiles once);
/// the call's actuals substitute the formals in the inner call and are
/// evaluated eagerly in formal order (R: MAKEPROM on a builtin frame).
///
/// Snapshot the shipped wrapper bindings at the end of base loading
/// (R: installed bytecode is fixed at install) WITHOUT forcing them:
/// forcing lazy promises at startup perturbs the heap for the whole
/// run. Namespace callers inline the forced snapshot definition (see
/// [`resolve_simple_site`]); forcing happens on first use, where the
/// generic call would force them anyway.
///
/// Kept out of line and cold: only candidate call sites call it.
#[cold]
#[inline(never)]
pub fn snapshot_simple_internals(interp: &mut Interp) {
    // The raw bindings, promises unforced; `None` entries (or no
    // snapshot on paths that skip base loading) fall back to generic.
    for (i, name) in crate::compile::SIMPLE_INTERNALS.iter().enumerate() {
        let Some(sym) = Sym::lookup(name) else { continue };
        interp.si_snaps[i] = interp.base.get_local(sym);
    }
    interp.si_ready = true;
}

/// The caller of a call that may inline: top-level calls are never
/// compiled in GNU R and stay generic. Otherwise the caller closure
/// (cloned; the memo holds it weakly, which keeps its allocation, and so
/// its address, from being reused),
/// whether it is namespace-scoped (R: getInlineInfo finds a locked
/// namespace above it, so the call inlines without a guard), and whether
/// it runs as compiled code (R: R_CheckJIT timing; first calls of small
/// closures stay interpreted and generic).
#[cold]
#[inline(never)]
fn simple_caller(interp: &Interp) -> Option<(Rc<Closure>, bool, bool)> {
    let frame = interp.current_frame()?;
    let Value::Closure(caller) = &interp.frames[frame].function else { return None };
    let ns = caller.env.ptr_eq(&interp.base_ns) || (!caller.env.is_search() && locked_to_search(&caller.env));
    Some((caller.clone(), ns, caller.jit.is_bytecode()))
}

/// R: compiler/R/cmp.R:is.simpleInternal. The definition is a closure
/// with simple formals (no `...`, constant defaults) whose body is
/// `.Internal(ifun(...))` for a builtin internal with formal-symbol or
/// constant arguments. Returns the internal name with the formals and
/// the inner template.
#[cold]
#[inline(never)]
fn simple_internal_shape(clo: &Closure) -> Option<(Sym, &[Arg], &[Arg])> {
    if clo.formals.iter().any(|f| f.tag == Some(s::DOTS)) {
        return None;
    }
    for f in clo.formals.iter() {
        if !f.value.is_missing_arg() {
            match &f.value {
                Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => return None,
                _ => {}
            }
        }
    }
    // The body, unwrapping `{`.
    let mut b = &clo.body;
    if let Value::Lang(l) = b {
        if l.fun_sym() == Some(s::BRACE) && l.args.len() == 1 && l.args[0].tag.is_none() {
            b = &l.args[0].value;
        }
    }
    let Value::Lang(l) = b else { return None };
    if l.fun_sym() != Some(s::DOT_INTERNAL) || l.args.len() != 1 || l.args[0].tag.is_some() {
        return None;
    }
    let Value::Lang(icall) = &l.args[0].value else { return None };
    let fsym = icall.fun_sym()?;
    // R: is.builtin.internal.
    crate::builtins::lookup_internal(&fsym.name())?;
    // R: simpleArgs (tags never appear in the shipped templates).
    for a in &icall.args {
        if a.tag.is_some() || a.value.is_missing_arg() {
            return None;
        }
        match &a.value {
            Value::Sym(sym) => {
                if !clo.formals.iter().any(|f| f.tag == Some(*sym)) {
                    return None;
                }
            }
            Value::Lang(_) | Value::Promise(_) => return None,
            _ => {}
        }
    }
    Some((fsym, &clo.formals, &icall.args))
}

/// Align this call's actuals to the wrapper formals (R: match.call) and
/// substitute them into the inner template (R: inlineSimpleInternalCall).
/// `None` is the generic call (R: cmpBuiltin bails on substituted
/// missing, and checkCall fails where match_args does). Marker values
/// recover each formal's actual index; match failures reproduce their
/// error on the generic path.
#[cold]
#[inline(never)]
fn simple_arg_map(
    interp: &mut Interp,
    formals: &[Arg],
    template: &[Arg],
    args: &[Arg],
    call: &Value,
) -> Option<Vec<SiArg>> {
    // R: dots.or.missing on the outer call.
    for a in args {
        if a.value.is_missing_arg() || a.value.as_sym() == Some(s::DOTS) {
            return None;
        }
    }
    let skeleton: Vec<Arg> = args.iter().enumerate().map(|(j, a)| Arg::new(a.tag, Value::int(j as i32))).collect();
    let actuals = interp.match_args(formals, &skeleton, call).ok()?;
    let mut map = Vec::with_capacity(template.len());
    for t in template {
        let Value::Sym(sym) = &t.value else {
            map.push(SiArg::Const(t.value.clone()));
            continue;
        };
        let fi = formals.iter().position(|f| f.tag == Some(*sym))?;
        match &actuals[fi] {
            Some(Value::Int(m)) if m.data.len() == 1 => map.push(SiArg::Actual(m.data[0] as usize)),
            Some(_) => return None,
            None => {
                let d = &formals[fi].value;
                if d.is_missing_arg() {
                    return None;
                }
                map.push(SiArg::Const(d.clone()));
            }
        }
    }
    Some(map)
}

/// Run the substituted inner call: actuals evaluate eagerly in formal
/// order (R: cmpBuiltinArgs; evalList does the same for interpreted
/// calls), then the internal runs with the user's call as its error
/// context (R: getCurrentCall over the bytecode expressionsIndex).
/// Out of line (so callers stay lean) but normally optimized: this runs
/// once per inlined call.
#[inline(never)]
fn run_simple_internal(
    interp: &mut Interp,
    fsym: Sym,
    map: &[SiArg],
    args: &[Arg],
    call: &Value,
    env: &Env,
) -> RResult<Value> {
    // The inner call keeps the actuals' expressions: error calls deparse
    // them, as in GNU R's `rep.int(v, -1)`.
    let mut cargs = Vec::with_capacity(map.len());
    for m in map {
        match m {
            SiArg::Actual(j) => cargs.push(args[*j].clone()),
            SiArg::Const(v) => cargs.push(Arg::pos(v.clone())),
        }
    }
    let argv = match interp.eval_args(&cargs, env, call) {
        Ok(argv) => argv,
        Err(e) => return signal(interp, Err(e)),
    };
    let inner = Value::lang(Value::Sym(fsym), cargs);
    let Some(b) = crate::builtins::lookup_internal(&fsym.name()) else {
        return Err(Flow::error(format!("robigo: no .Internal function '{fsym}'")));
    };
    // R: names.c:do_internal evaluates the arguments, then runs the
    // primitive with the inner call.
    interp.visible = true;
    let argv = match b.kind {
        BuiltinKind::Special => {
            let Value::Lang(l) = &inner else { unreachable!() };
            l.args.clone()
        }
        BuiltinKind::Builtin => argv,
    };
    // R: eval.c:getCurrentCall via the bytecode expressionsIndex: plain
    // errors and warnings from the inlined internal report the user's
    // call, as the wrapper frame does when interpreted.
    interp.ccode_calls.push((interp.frames.len(), call.clone()));
    let r = interp.invoke_builtin(b, &inner, argv, env);
    let r = signal(interp, r);
    interp.ccode_calls.pop();
    r
}

/// Resolve a `si` call site for this caller (R: compiling the call).
/// `None` is the generic call. The resolution is write-once per caller:
/// the definition and argument map, or never-inline.
#[cold]
#[inline(never)]
fn resolve_simple_site(
    interp: &mut Interp,
    si: usize,
    ns: bool,
    l: &crate::value::Lang,
    call: &Value,
    env: &Env,
) -> Option<(Sym, Rc<Closure>, Vec<SiArg>)> {
    let head = l.fun_sym()?;
    let def = if ns {
        // Installed code: the shipped definition, used unconditionally
        // (R: no guard in namespaces). The snapshot holds the raw
        // binding; forcing happens here, where the generic call would
        // force it anyway. A failed force falls back to the generic
        // call, which reproduces the failure.
        let raw = interp.si_snaps[si].clone()?;
        let Value::Closure(d) = interp.force(raw).ok()? else { return None };
        // The live lookup must still reach base, else the generic call
        // reports the lookup failure.
        if !interp.resolves_to_base(head, env) {
            return None;
        }
        d
    } else {
        // User code: the current base definition (R: BASEGUARD compares
        // against the base namespace; the recorded definition is what
        // the call was compiled against).
        let Value::Closure(d) = interp.base_binding(head)? else { return None };
        d
    };
    let (fsym, formals, template) = simple_internal_shape(&def)?;
    let map = simple_arg_map(interp, formals, template, &l.args, call)?;
    Some((fsym, def, map))
}

/// A `si` call site taking the inlined path: the memoized definition
/// and argument map, guarded for user callers (R: DO_BASEGUARD compares
/// the visible binding with the base namespace). `None` is the generic
/// call.
#[cold]
#[inline(never)]
fn try_inline_site(
    interp: &mut Interp,
    site: &CallSite,
    l: &crate::value::Lang,
    call: &Value,
    env: &Env,
) -> Option<RResult<RVal>> {
    let si = site.si? as usize;
    if site.force != 0 {
        return None;
    }
    // No resolution while base is still loading (the snapshot is
    // partial); those calls stay generic and unmemoized.
    if !interp.si_ready {
        return None;
    }
    let head = l.fun_sym()?;
    let (caller, ns, compiled) = simple_caller(interp)?;
    // User code inlines only once compiled (R_CheckJIT timing); first
    // calls stay interpreted and generic.
    if !ns && !compiled {
        return None;
    }
    // The memoized resolution, if it was resolved for this caller. The
    // borrow ends here: running the call may re-enter this site through
    // a handler.
    let hit = {
        let memo = site.si_memo.borrow();
        match memo.as_ref() {
            Some((c, n, r)) if std::ptr::eq(c.as_ptr(), Rc::as_ptr(&caller)) && *n == ns => Some(r.clone()),
            _ => None,
        }
    };
    match hit {
        Some(inline) => {
            let inline = inline?;
            if !ns && !interp.resolves_to_base(head, env) {
                return None;
            }
            // C code can rewrite the call in place through its cells (a
            // spliced spine, sims1253/robigo#387): a resolution made for
            // other arguments no longer applies, and the call runs generic
            if inline.args.iter().any(|m| matches!(m, SiArg::Actual(j) if *j >= l.args.len())) {
                return None;
            }
            let r = run_simple_internal(interp, inline.fun, &inline.args, &l.args, call, env);
            Some(r.map(RVal::V))
        }
        None => {
            // Write-once either way (GNU R compiles once): a failed
            // resolution stays generic, matching code compiled against a
            // replacement definition.
            let resolved = resolve_simple_site(interp, si, ns, l, call, env).map(|(fsym, def, map)| SiInline {
                def,
                fun: fsym,
                args: map,
            });
            *site.si_memo.borrow_mut() = Some((Rc::downgrade(&caller), ns, resolved.clone()));
            let inline = resolved?;
            let r = run_simple_internal(interp, inline.fun, &inline.args, &l.args, call, env);
            Some(r.map(RVal::V))
        }
    }
}

/// An interpreted call taking the inlined path (see [`try_inline_site`]).
/// Interpreted callers outside locked namespaces stay generic: only
/// installed code observes the inline. Resolutions are memoized per
/// caller and call expression, as for compiled call sites.
#[cold]
#[inline(never)]
pub(crate) fn try_inline_expr(
    interp: &mut Interp,
    si: u8,
    l: &Rc<crate::value::Lang>,
    call: &Value,
    env: &Env,
) -> Option<RResult<Value>> {
    // R: only installed — or, after R_CheckJIT's timing, warmed — code
    // observes the inline, and only for its OWN body: the expression must
    // evaluate in the caller closure's frame. A promise forced from that
    // body keeps the promise's env (where the call was written), and GNU R
    // never compiles promise expressions: the wrapper's closure runs,
    // frame and all (issue #157). `cat(identical(x, y))` forces its
    // promise in the global env although cat is base code. The frame
    // check is the common rejection and only reads, so it runs before
    // the caller lookup and its clone.
    let frame = interp.current_frame()?;
    if !env.ptr_eq(&interp.frames[frame].cloenv) {
        return None;
    }
    let (caller, ns, compiled) = simple_caller(interp)?;
    if !ns && !compiled {
        return None;
    }
    // No resolution while base is still loading (the snapshot is
    // partial); those calls stay generic and unmemoized.
    if !interp.si_ready {
        return None;
    }
    // Record-first per caller, as for compiled call sites (GNU R
    // compiles once): the definition and argument map, or never-inline.
    // The value holds caller and expression weakly (see SiMemo): their
    // allocations stay, so a key hit is always the same objects (freed
    // addresses could otherwise be reused by unrelated calls), but not
    // the objects; gc() drops the entries of dead callers.
    let key = (Rc::as_ptr(&caller) as usize, Rc::as_ptr(l) as usize);
    if let Some((_, _, hit)) = interp.si_expr_memo.get(&key) {
        let inline = hit.clone()?;
        if !ns && !interp.resolves_to_base(l.fun_sym()?, env) {
            return None;
        }
        // (a call C rewrote in place, as above)
        if inline.args.iter().any(|m| matches!(m, SiArg::Actual(j) if *j >= l.args.len())) {
            return None;
        }
        return Some(run_simple_internal(interp, inline.fun, &inline.args, &l.args, call, env));
    }
    let resolved = resolve_simple_site(interp, si as usize, ns, l, call, env).map(|(fsym, def, map)| SiInline {
        def,
        fun: fsym,
        args: map,
    });
    interp.si_expr_memo.insert(key, (Rc::downgrade(&caller), Rc::downgrade(l), resolved.clone()));
    let inline = resolved?;
    Some(run_simple_internal(interp, inline.fun, &inline.args, &l.args, call, env))
}

/// `Op::Call` of the closure `clo` (call site `site`, call `l`): promised
/// arguments (some forced by the caller, see [`caller_promargs`]), then
/// the call, whose compiled body may leave a scalar result unboxed.
#[inline(never)]
fn closure_call(
    interp: &mut Interp,
    site: &CallSite,
    clo: &Rc<Closure>,
    l: &crate::value::Lang,
    env: &Env,
    vars: &mut Vars<'_>,
    regs: &[RVal],
) -> RResult<RVal> {
    if site.si.is_some()
        && let Some(r) = try_inline_site(interp, site, l, &site.call, env)
    {
        return r;
    }
    if let Some(isite) = &site.intrinsic
        && let Some(v) = crate::intrinsic::match_arg(interp, isite, clo, env)
    {
        interp.visible = true;
        return Ok(RVal::V(v));
    }
    // (Interp::eval_inlined_local: the compiler's local() inline.)
    if interp.bc && inlines_local(interp, clo, l, env) {
        return interp.eval_inlined_local(l, env).map(RVal::V);
    }
    let promargs = if site.eager && site.no_eager.get() != Rc::as_ptr(&clo.formals) as *const Arg as usize {
        caller_promargs(interp, site, clo, l, env, vars, regs)
    } else {
        interp.promise_args_with(&l.args, env, &site.codes)
    };
    let r = match promargs {
        Ok(promargs) => match force_args(interp, &promargs, site.force) {
            Ok(()) => match direct_code(clo, &promargs) {
                Some(code) => direct_call(interp, &site.call, clo, &code, promargs, env),
                None => tagged_call(interp, &site.call, clo, promargs, env),
            },
            Err(e) => signal(interp, Err(e)),
        },
        Err(e) => signal(interp, Err(e)),
    };
    let v = r?;
    Ok(match interp.unboxed_ret.take() {
        Some(r) => r,
        None => RVal::V(v),
    })
}

/// The compiled body of `clo` when a call with these arguments can take
/// [`direct_call`]: untagged, no more than the formals, none of which is
/// `...` (R's matching is then positional: `interp::bind_positional`).
#[inline]
fn direct_code(clo: &Closure, promargs: &[Arg]) -> Option<Rc<Code>> {
    if promargs.len() > clo.formals.len()
        || promargs.iter().any(|a| a.tag.is_some())
        || clo.formals.iter().any(|f| f.tag == Some(s::DOTS))
        || crate::typeprof::enabled()
    {
        return None;
    }
    clo.code.get_or_init(|| code_for(&clo.body, true)).clone()
}

/// A call of `clo` from compiled code, as `call_closure_ex` makes it
/// (through `apply_closure_ex` and `exec_closure_ex`) for positional
/// arguments and a compiled body, without their general cases: nesting
/// limits, the new frame, the context (pushed as `exec_closure` does, see
/// the hook contract in docs/ledger/methods.md), the body, a `return()` to
/// it, `end_context`, signaling. A scalar result may stay unboxed
/// (`Interp::unboxed_ret`).
#[inline(never)]
fn direct_call(
    interp: &mut Interp,
    call: &Value,
    clo: &Rc<Closure>,
    code: &Code,
    promargs: Vec<Arg>,
    env: &Env,
) -> RResult<Value> {
    interp.enter_call()?;
    let newenv = crate::interp::bind_positional(clo, &promargs);
    run_direct(interp, call, clo, code, promargs, newenv, env)
}

/// A call of `clo` from compiled code that [`direct_code`] declined: when
/// the arguments match without partial matching (`interp::bind_exact`:
/// tags, `...`), still a [`direct_call`]; otherwise the general call.
#[inline(never)]
fn tagged_call(interp: &mut Interp, call: &Value, clo: &Rc<Closure>, promargs: Vec<Arg>, env: &Env) -> RResult<Value> {
    if !crate::typeprof::enabled()
        && let Some(code) = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()
        && let Some(newenv) = crate::interp::bind_exact(clo, &promargs)
    {
        interp.enter_call()?;
        return run_direct(interp, call, clo, &code, promargs, newenv, env);
    }
    interp.call_closure_ex(call, clo, promargs, env, true)
}

/// The rest of [`direct_call`] once the frame is made.
#[inline(always)]
fn run_direct(
    interp: &mut Interp,
    call: &Value,
    clo: &Rc<Closure>,
    code: &Code,
    promargs: Vec<Arg>,
    newenv: Env,
    env: &Env,
) -> RResult<Value> {
    let sysparent = interp.call_sysparent(env);
    interp.push_closure_context(call, clo, promargs, &newenv, sysparent);
    // R: eval.c:START_BCFRAME_CALL's BC_CHECK_SIGINT
    if crate::autogc::due() {
        crate::autogc::collect(interp);
    }
    if clo.jit.inline_blocked() {
        jit_check(interp, clo);
    }
    // R: eval.c:R_execClosure — bcEval for a compiled body, else eval.
    interp.bc = !clo.jit.interpreted();
    let r = match run_closure(interp, code, &newenv, clo, true) {
        Err(Flow::Return(v, target)) if target.ptr_eq(&newenv) => Ok(v),
        r => r,
    };
    let r = interp.end_context(r);
    interp.leave_call(r)
}

/// The promised arguments of a call of `clo` at `site`. Where the body
/// forces a formal before anything that may run R code (`Code::strict`,
/// with all its guards known to hold), evaluating that argument here
/// instead is unobservable if it needs no code either: a constant, a
/// variable readable without forcing, or an arithmetic operation through
/// its scalar fast path. Such arguments get promises already forced (as
/// the callee would leave them), so the callee reads their values; the
/// others are promised as usual.
fn caller_promargs(
    interp: &mut Interp,
    site: &CallSite,
    clo: &Closure,
    l: &crate::value::Lang,
    env: &Env,
    vars: &mut Vars<'_>,
    regs: &[RVal],
) -> RResult<Vec<Arg>> {
    if let Some(eager) = eager_args(interp, site, clo, env, vars, regs) {
        // A variable's promise has the binding's own object as its value
        // (pending assignments were flushed for the call), when the body
        // can hand it to native code.
        let escapes = matches!(clo.code.get(), Some(Some(c)) if c.escapes);
        let mut out = interp.arg_vec(l.args.len());
        for ((a, c), v) in l.args.iter().zip(&site.codes).zip(eager) {
            let v = match (v, &a.value) {
                (Some(v @ (RVal::I(_) | RVal::D(_) | RVal::L(_))), Value::Sym(sym)) if escapes => {
                    Some(RVal::V(escaping_read(env, *sym, v)))
                }
                (v, _) => v,
            };
            out.push(match (v, &a.value) {
                (Some(RVal::I(i)), _) => {
                    Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced_int(a.value.clone(), i))))
                }
                (Some(RVal::D(d)), _) => {
                    Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced_real(a.value.clone(), d))))
                }
                (Some(RVal::L(l)), _) => {
                    Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced_lgl(a.value.clone(), l))))
                }
                (Some(v), _) => {
                    Arg::new(a.tag, Value::Promise(Rc::new(Promise::forced(a.value.clone(), v.into_value()))))
                }
                (None, Value::Sym(_) | Value::Lang(_)) => {
                    let p = Promise::arg(a.value.clone(), env.clone(), c.clone(), interp.bc);
                    Arg::new(a.tag, Value::Promise(Rc::new(p)))
                }
                (None, Value::Promise(_)) => a.clone(),
                // (Interp::promise_args_with: GNU R's AST interpreter
                // promises a constant, bytecode passes its value.)
                (None, _) if interp.bc => a.clone(),
                (None, _) => crate::interp::const_promise(a, env),
            });
        }
        return Ok(out);
    }
    interp.promise_args_with(&l.args, env, &site.codes)
}

/// The values [`caller_promargs`] may supply (by argument), if any.
fn eager_args(
    interp: &mut Interp,
    site: &CallSite,
    clo: &Closure,
    env: &Env,
    vars: &mut Vars<'_>,
    regs: &[RVal],
) -> Option<[Option<RVal>; 4]> {
    let args = site.inline_args.as_ref()?;
    let code = clo.code.get()?.as_ref()?;
    // Reasons that depend on the call site and the closure's definition
    // only are remembered. (Up to four arguments, the common case.) The
    // body must read a formal first, whose argument is not a constant
    // (constants are passed as values anyway).
    let first =
        (args.len() <= 4 && args.len() == clo.formals.len() && !clo.formals.iter().any(|f| f.tag == Some(s::DOTS)))
            .then(|| clo.formals.iter().position(|f| Some(f.tag) == code.strict.first().map(|s| Some(*s))))
            .flatten()
            .filter(|k| !matches!(args[*k], InlineArg::Leaf(Leaf::D(_) | Leaf::I(_))));
    if first.is_none() {
        site.no_eager.set(Rc::as_ptr(&clo.formals) as *const Arg as usize);
        return None;
    }
    // No guard of the body may fail before it forces these formals (a
    // rebound `if` or `<` could run code first).
    if code.nguards > 0 {
        let held = if clo.env.is_search() {
            fresh_guards_ok(interp, code, clo)
        } else {
            let key = (
                crate::env::search_epoch(),
                clo.env.id(),
                clo.env.fn_version(),
                Rc::as_ptr(&clo.formals) as *const Arg as usize,
            );
            clo.env.parent_ref().as_ref().is_some_and(|p| p.is_search()) && code.fresh_nested.get() == key
        };
        if !held {
            return None;
        }
    }
    let mut out: [Option<RVal>; 4] = [None, None, None, None];
    let mut any = false;
    for sym in &code.strict {
        // (In order: stop at the first read that would need the callee.)
        let Some(k) = clo.formals.iter().position(|f| f.tag == Some(*sym)) else { break };
        if out[k].is_some() {
            continue;
        }
        let v = match &args[k] {
            InlineArg::Leaf(Leaf::D(_) | Leaf::I(_)) => continue,
            InlineArg::Leaf(Leaf::Var(s, slot)) => match read_var(vars, env, *s, *slot) {
                Some(v) => v.clone(),
                None => break,
            },
            InlineArg::Leaf(Leaf::R(_)) => break,
            InlineArg::Arith { op, a, b, sym, memo } => {
                if !guard_ok(interp, *sym, env, memo) {
                    break;
                }
                let (Some(x), Some(y)) = (leaf_num(vars, env, regs, a), leaf_num(vars, env, regs, b)) else { break };
                match arith_fast(*op, x, y) {
                    Some(v) => v,
                    None => break,
                }
            }
            InlineArg::Index { x, slot, i, two, sym, memo } => {
                if !guard_ok(interp, *sym, env, memo) {
                    break;
                }
                let Some(iv) = leaf_rval(vars, env, regs, i) else { break };
                let Some(xv) = read_var(vars, env, *x, *slot) else { break };
                match index_value(xv, &iv, *two) {
                    Some(v) => v,
                    None => break,
                }
            }
        };
        out[k] = Some(v);
        any = true;
    }
    any.then_some(out)
}

/// The value a closure body returns: a scalar stays unboxed when the compiled caller asked (`unboxed`) and the
/// frame has no `on.exit` code (which would run before the caller takes
/// it), with `NULL` as the placeholder; otherwise boxed.
#[inline]
fn body_result(interp: &mut Interp, r: RVal, unboxed: bool) -> Value {
    if unboxed
        && matches!(r, RVal::D(_) | RVal::I(_) | RVal::L(_))
        && interp.frames.last().is_some_and(|c| c.on_exit.is_empty())
        && !crate::typeprof::enabled()
    {
        interp.unboxed_ret = Some(r);
        return Value::Null;
    }
    r.into_value()
}

/// Evaluate code that is a single guarded operation on constants and
/// variables (see `Code::simple`) without a VM run; `None` when any part
/// needs the general path (which then runs the code normally).
pub fn run_simple(interp: &mut Interp, code: &Code, env: &Env) -> Option<Value> {
    run_simple_unboxed(interp, code, env).map(RVal::into_value)
}

/// [`run_simple`], its result unboxed.
#[inline]
pub(crate) fn run_simple_unboxed(interp: &mut Interp, code: &Code, env: &Env) -> Option<RVal> {
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
    Some(r)
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
    matches!(frame.get(sym), Some(b) if b.flags == 0 && matches!(&b.value, Value::Promise(p) if matches!(*p.value.borrow(), crate::value::PromiseVal::Unforced)))
}

/// An active loop: break and next targets, its `LoopEnter` index and how
/// often its back edge ran in this entry.
#[derive(Clone, Copy)]
struct LoopRec {
    brk: u32,
    nxt: u32,
    /// Its `LoopEnter` index, with [`HEAT_BIT`] set when its back edges
    /// count toward `JitSlot::Heat` as it ends (a flag of its own made
    /// every loop entry copy 20 bytes instead of 16).
    enter: u32,
    count: u32,
}

/// See [`LoopRec::enter`] (op indices stay far below it).
const HEAT_BIT: u32 = 1 << 31;

/// The state of `run_with`'s guard memo (`checked`) and native loops. Kept
/// in memory (its re-check is out of line, so it has an address): the
/// dispatch loop has more loop-carried values than registers, and which
/// get spilled otherwise depends on the whole function, so that a change
/// to one op moved the cost of all others (#357's calibration).
struct GuardState {
    /// The `flush!()` generation: a guard that held in it needs no new
    /// check (only R code, always run after a flush, rebinds functions).
    generation: u32,
    /// Where `env`, its parent or the parent's parent is on the search
    /// path, whether guards hold depends only on the search epoch and the
    /// function bindings of `env` and its parent: R code that ran (after a
    /// flush) invalidates them only if one changed. Elsewhere (more deeply
    /// nested closures) every flush does.
    stable: bool,
    /// A flush happened since `token` was taken (`stable` only).
    recheck: bool,
    token: (u64, u32),
    /// Native loops are on.
    jit_on: bool,
}

impl GuardState {
    fn new(env: &Env) -> Self {
        GuardState {
            generation: 1,
            stable: env.is_search() || env.parent_ref().as_ref().is_some_and(|p| p.is_search()) || frame_of_locked(env),
            recheck: false,
            token: (crate::env::search_epoch(), env.fn_version()),
            jit_on: crate::jit::enabled(),
        }
    }

    /// Before anything that may run R code.
    #[inline(always)]
    fn flushed(&mut self) {
        if self.stable {
            self.recheck = true;
        } else {
            self.generation = self.generation.wrapping_add(1).max(1);
        }
    }

    /// Before relying on `checked`: start a new generation if R code that
    /// ran since the last flush changed a function binding.
    #[inline(never)]
    fn revalidate(&mut self, env: &Env, code: &Code) {
        self.recheck = false;
        let now = (crate::env::search_epoch(), env.fn_version());
        if now != self.token {
            // A new local function binding (say a helper defined in the
            // body) matters only if it binds a guarded name.
            let local_only = now.0 == self.token.0;
            self.token = now;
            if !local_only || code.guard_syms.iter().any(|s| env.local_may_bind_fun(*s)) {
                self.generation = self.generation.wrapping_add(1).max(1);
            }
        }
    }
}

/// Where an error of an op leaves the dispatch loop: `break`/`next` of
/// this frame's innermost loop continue at its exit or next target;
/// anything else propagates.
#[cold]
#[inline(never)]
fn unwind_to(f: Flow, env: &Env, loops: &[LoopRec]) -> Result<usize, Flow> {
    match (&f, loops.last()) {
        (Flow::Break(e), Some(l)) if e.ptr_eq(env) => Ok(l.brk as usize),
        (Flow::Next(e), Some(l)) if e.ptr_eq(env) => Ok(l.nxt as usize),
        _ => Err(f),
    }
}

/// Iterations of a loop in the VM before it is compiled to native code.
const JIT_THRESHOLD: u32 = 16;

/// Back edges, summed over entries that each ended below
/// [`JIT_THRESHOLD`], before a loop is compiled at its next entry. Higher:
/// compiling costs as much as thousands of iterations in the VM, so a short
/// loop must be run often to repay it (at 16, `robigo/nbody` was +25%,
/// compiling its inner loops before the outer one).
const JIT_HEAT: u32 = 1024;

/// Ops per entry (its back edges per entry times the length of its body)
/// a loop compiled by heat must average: entering native code costs more
/// than a few ops in the VM (`robigo/fannkuch` +0.7% entering two loops of
/// eight ops and one or two iterations natively 108k times).
const JIT_HEAT_WORK: u64 = 64;

/// Whether the loop entered at op `enter` (ending at `brk`) runs natively
/// now, and whether this entry counts toward its heat. Out of line: the
/// dispatch loop's register allocation is shared by all its ops.
#[inline(never)]
fn loop_enter_state(code: &Code, enter: u32, brk: u32) -> (bool, bool) {
    match code.jit.borrow().get(&enter) {
        Some(crate::jit::JitSlot::Ready(_)) => (true, false),
        // (Hot but too little work per entry: no more counting.)
        Some(crate::jit::JitSlot::Heat { back, entries }) if *back >= JIT_HEAT => {
            let body = (brk as u64).saturating_sub(enter as u64);
            (*back as u64 * body >= JIT_HEAT_WORK * *entries as u64, false)
        }
        Some(crate::jit::JitSlot::Heat { .. }) | None => (false, true),
        Some(_) => (false, false),
    }
}

/// Add the back edges of an entry of the loop at `enter` that ended below
/// the threshold to its heat.
#[cold]
#[inline(never)]
fn loop_heat(code: &Code, enter: u32, count: u32) {
    use crate::jit::JitSlot;
    if count == 0 || count >= JIT_THRESHOLD {
        return;
    }
    let mut cache = code.jit.borrow_mut();
    match cache.get_mut(&enter) {
        None => {
            cache.insert(enter, JitSlot::Heat { back: count, entries: 1 });
        }
        Some(JitSlot::Heat { back, entries }) => {
            *back = back.saturating_add(count);
            *entries = entries.saturating_add(1);
        }
        Some(_) => {}
    }
}

/// Native matrix views accept only a dimension attribute, with validated
/// storage bounds. No native op can change the dimensions during a loop.
pub(crate) fn jit_matrix_dims<T>(x: &crate::value::Vector<T>) -> Option<(u64, u64)> {
    let attrs = x.attrs.as_ref()?;
    if attrs.1 || attrs.0.len() != 1 || attrs.0[0].0 != s::DIM {
        return None;
    }
    let Value::Int(d) = &attrs.0[0].1 else { return None };
    let [nr, nc] = d.data[..] else { return None };
    if nr < 0 || nc < 0 || (nr as usize).checked_mul(nc as usize)? != x.data.len() {
        return None;
    }
    Some((nr as u64, nc as u64))
}

/// The value of a number of native code (`crate::jit::JType::N`) by its tag.
fn number(x: f64, tag: u64) -> RVal {
    if tag as u32 != 0 { RVal::I(x as i32) } else { RVal::D(x) }
}

/// A scalar complex (`JType::C`) from its two value-buffer lanes: real part
/// first, imaginary part at the next index.
fn cplx_of(re: u64, im: u64) -> RVal {
    RVal::V(Value::cplx(crate::value::Complex::new(f64::from_bits(re), f64::from_bits(im))))
}

/// The type of a variable's current value for a loop: [`jit_type`], a
/// range, or a character vector or matrix (whose elements native loops
/// read and assign).
fn loop_type(v: &RVal) -> Option<crate::jit::JType> {
    if range_of(v).is_some() {
        return Some(crate::jit::JType::Range);
    }
    loop_vec_type(v)
}

/// [`loop_type`] of a vector variable's value: a compact integer sequence
/// is an integer vector here, as native code views it.
fn loop_vec_type(v: &RVal) -> Option<crate::jit::JType> {
    use crate::jit::JType;
    jit_type(v).or(match v {
        RVal::V(Value::Str(x)) if x.attrs.is_none() => Some(JType::VecS),
        RVal::V(Value::Str(x)) if jit_matrix_dims(x).is_some() => Some(JType::MatS),
        _ => None,
    })
}

/// The scalar or vector type of a variable's current value for native code.
pub(crate) fn jit_type(v: &RVal) -> Option<crate::jit::JType> {
    use crate::jit::JType;
    // native kernels read data directly: never a package ALTREP vector's
    if let RVal::V(x) = v {
        if crate::altrep::lookup(x).is_some() {
            return None;
        }
    }
    Some(match v {
        RVal::V(Value::Cplx(z)) if z.attrs.is_none() && z.data.len() == 1 => JType::C,
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
        RVal::V(Value::Real(x)) if jit_matrix_dims(x).is_some() => JType::MatD,
        RVal::V(Value::Int(x)) if jit_matrix_dims(x).is_some() => JType::MatI,
        _ => return None,
    })
}

/// Whether calls of `clo` can run inline (see [`inline_info`]).
/// Whether closure bodies may run outside the call machinery at all.
pub(crate) fn may_inline() -> bool {
    !crate::typeprof::enabled() && !NO_INLINE.with(|n| *n)
}

pub(crate) fn inlinable(clo: &Closure) -> bool {
    !clo.jit.traced() && enabled() && clo.inline.get_or_init(|| inline_info(clo)).is_some()
}

/// Call an inlinable closure with pre-unboxed or fast `RVal` argument values,
/// returning the unboxed `RVal` result directly.
pub(crate) fn call_inline_rval(interp: &mut Interp, clo: &Closure, args: &[RVal]) -> Option<RVal> {
    let info = clo.inline.get()?.as_ref()?;
    if args.len() != info.formal_slots.len() || clo.jit.inline_blocked() {
        return None;
    }
    let v = run_inline_with(interp, info, &clo.env, |_, k| Some(args[k].clone()), INLINE_DEPTH)?;
    interp.visible = true;
    Some(v)
}

/// A call of `clo` with the values `args` by its native code, for a builtin
/// that calls a closure per element (`lapply`), as a call site's
/// [`native_site_call`] would make it. The VM's frames are flushed (the
/// builtin's call was a real call). `None`: the call has to be made, and
/// nothing has happened yet.
pub(crate) fn call_native_rval(interp: &mut Interp, clo: &Rc<Closure>, args: &[RVal]) -> Option<RVal> {
    let Some(Some(code)) = clo.code.get() else { return None };
    if code.native.never() || args.len() != clo.formals.len() || clo.jit.inline_blocked() {
        return None;
    }
    if !code.native.counted() {
        return None;
    }
    let v = match crate::jit::closure::call(interp, clo, code, args, &[], None) {
        crate::jit::closure::Run::Done(v) => Some(v),
        _ => None,
    };
    code.native.outcome(v.is_some());
    v
}

/// Whether `clo` is known never to run as native code (see
/// [`call_native_rval`]).
pub(crate) fn native_never(clo: &Closure) -> bool {
    matches!(clo.code.get(), Some(Some(code)) if code.native.never()) || matches!(clo.code.get(), Some(None))
}

/// The call at call site `site` (of `f`) as a call native code can run
/// inline: a pure closure (see [`inline_info`], [`native_inline`] or
/// [`loop_callee`]) called with plain arguments or element reads (`x[i]`,
/// `x[a:b]`, which native code evaluates where it has them); a
/// [`loop_callee`] may also read scalar variables of its enclosing
/// environments and assign them with `<<-` (see `crate::jit::LoopCell`).
fn inline_call(
    interp: &mut Interp,
    code: &Code,
    f: Sym,
    site: u32,
    env: &Env,
    vars: &Vars<'_>,
) -> Option<crate::jit::InlineCall> {
    use crate::jit::CallArg;
    let cs = &code.call_sites[site as usize];
    if cs.fslot.is_some_and(|s| matches!(vars.slots[s as usize], SlotVal::Dirty(_))) {
        return None;
    }
    // Arithmetic and element reads of the caller's variables are evaluated
    // by native code where they take their fast paths (else it leaves at
    // the call); `jit_enter` checks their functions are base's (memoized
    // per argument).
    let arg_syms = Vec::new();
    let args: Vec<CallArg> = match (&cs.leaf_args, &cs.inline_args) {
        (Some(leaves), _) => leaves.iter().map(|l| CallArg::Leaf(*l)).collect(),
        (None, Some(ia)) => ia
            .iter()
            .map(|a| match a {
                InlineArg::Leaf(l) => CallArg::Leaf(*l),
                InlineArg::Arith { op, a, b, .. } => CallArg::Arith(*op, *a, *b),
                InlineArg::Index { slot, i, two, .. } => CallArg::Index(*slot, *i, *two),
            })
            .collect(),
        _ => return None,
    };
    let Value::Closure(clo) = call_site_fun_fast(interp, f, cs, env)? else { return None };
    if clo.jit.traced() || args.len() != clo.formals.len() {
        return None;
    }
    let info = clo.inline.get_or_init(|| inline_info(&clo)).clone();
    let native = if info.is_none() { native_inline(&clo) } else { None };
    let (body, formal_slots, guard_syms, cells, free) = match (info, native) {
        (Some(info), _) if !info.free_vars => {
            (info.code.clone(), info.formal_slots.clone(), info.guard_syms.clone(), Vec::new(), Vec::new())
        }
        (None, Some((c, f, g))) => (c, f, g, Vec::new(), Vec::new()),
        (info, _) => match loop_callee(&clo) {
            // Free variables bound to scalars, read or `<<-`-assigned: cells
            // of the loop (#312).
            Some((c, f, g, cells)) => (c, f, g, cells, Vec::new()),
            None => {
                // Otherwise read-only free variables, scalars or vectors (and
                // a body that reads their elements), read once at the loop's
                // entry from the closure's environment: one that does not
                // reach the loop's own frame (whose variables may be pending
                // in the VM).
                let info = match info {
                    Some(i) => i,
                    None => loop_inline_info(&clo)?,
                };
                let mut free = Vec::new();
                if info.free_vars {
                    if crate::jit::closure::on_chain(&clo.env, env) {
                        return None;
                    }
                    for (s, sym) in info.code.vars.iter().enumerate() {
                        if info.formal_slots.contains(&Some(s as Slot)) {
                            continue;
                        }
                        let v = plain_lookup(&clo.env, *sym)?;
                        if matches!(&v, RVal::V(x) if x.is_object()) {
                            return None;
                        }
                        free.push((s as Slot, *sym, jit_type(&v)?));
                    }
                }
                (info.code.clone(), info.formal_slots.clone(), info.guard_syms.clone(), Vec::new(), free)
            }
        },
    };
    // Native code does not count calls: take the loop to call the closure
    // at least twice (GNU R's JIT decides by the second call).
    while clo.jit.inline_blocked() {
        jit_check(interp, &clo);
    }
    Some(crate::jit::InlineCall {
        code: body,
        formal_slots,
        guard_syms,
        args,
        clo: clo.clone(),
        callee: 0,
        arg_syms,
        cells,
        free,
    })
}

/// A closure body only native code runs inline: as [`inline_analysis`]
/// requires, but also with `a:b` and `x[i]`, which the VM's inline path does
/// not have (a closure like `rev.default`). Its code, formal slots and
/// guarded functions.
fn native_inline(clo: &Closure) -> Option<(Rc<Code>, Vec<Option<Slot>>, Vec<Sym>)> {
    if !may_inline() || clo.formals.iter().any(|f| f.tag == Some(s::DOTS) || !f.value.is_missing_arg()) {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    let formals: Vec<Sym> = clo.formals.iter().map(|f| f.tag.unwrap()).collect();
    if code.vars.iter().any(|v| !formals.contains(v)) {
        return None;
    }
    let mut guard_syms = Vec::new();
    for op in &code.ops {
        match op {
            Op::CallBuiltin { .. } if crate::jit::bitwise_call(&code, op) => {}
            Op::Guard { sym, .. } | Op::ArithL { sym, .. } | Op::RelL { sym, .. } => {
                if formals.contains(sym) {
                    return None;
                }
                if !guard_syms.contains(sym) {
                    guard_syms.push(*sym);
                }
            }
            Op::ConstD { .. }
            | Op::ConstI { .. }
            | Op::ConstL { .. }
            | Op::GetVar { .. }
            | Op::Arith { .. }
            | Op::Rel { .. }
            | Op::Neg { .. }
            | Op::Not { .. }
            | Op::Math1 { .. }
            | Op::Builtin1 { .. }
            | Op::JumpIfNot { .. }
            | Op::JumpIf { .. }
            | Op::Jump { .. }
            | Op::Visible { .. }
            | Op::Return { .. }
            | Op::Colon { .. }
            | Op::Index { two: false, .. } => {}
            _ => return None,
        }
    }
    let formal_slots = formals.iter().map(|f| code.vars.iter().position(|v| v == f).map(|p| p as Slot)).collect();
    Some((code, formal_slots, guard_syms))
}

/// A closure that reads or `<<-`-assigns variables of its enclosing
/// environments, as [`inline_call`] takes it: the body is pure but for the
/// `<<-` (as [`inline_info`] has it, which keeps such bodies in the VM),
/// and every such variable is bound to a scalar now. (The native loop
/// checks the bindings again whenever it is entered.)
#[cold]
#[allow(clippy::type_complexity)]
fn loop_callee(clo: &Closure) -> Option<(Rc<Code>, Vec<Option<Slot>>, Vec<Sym>, Vec<crate::jit::CalleeCell>)> {
    // (Untagged formals come from the C API's `R_mkClosure`.)
    if !may_inline()
        || clo.formals.iter().any(|f| f.tag == Some(s::DOTS) || f.tag.is_none() || !f.value.is_missing_arg())
    {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    let formals: Vec<Sym> = clo.formals.iter().map(|f| f.tag.unwrap()).collect();
    let mut guard_syms = Vec::new();
    let mut written: Vec<Slot> = Vec::new();
    for op in &code.ops {
        match op {
            Op::Guard { sym, .. } | Op::ArithL { sym, .. } | Op::RelL { sym, .. } => {
                if formals.contains(sym) {
                    return None;
                }
                if !guard_syms.contains(sym) {
                    guard_syms.push(*sym);
                }
            }
            // (`<<-` of a formal's name assigns the enclosing binding while
            // the body reads the formal: not a cell.)
            Op::SuperSet { sym, slot, .. } if !formals.contains(sym) => written.push(*slot),
            Op::ConstD { .. }
            | Op::ConstI { .. }
            | Op::ConstL { .. }
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
            _ => return None,
        }
    }
    let formal_slots = formals.iter().map(|f| code.vars.iter().position(|v| v == f).map(|p| p as Slot)).collect();
    let mut cells = Vec::new();
    for (k, sym) in code.vars.iter().enumerate() {
        if formals.contains(sym) {
            continue;
        }
        let t = plain_lookup(&clo.env, *sym).as_ref().and_then(jit_type).filter(|t| t.is_scalar())?;
        cells.push(crate::jit::CalleeCell {
            slot: k as Slot,
            sym: *sym,
            ty: t,
            written: written.contains(&(k as Slot)),
        });
    }
    Some((code, formal_slots, guard_syms, cells))
}

/// Search epoch and frame identities and function-binding versions, including
/// the loop frame. Environment ids are monotonic, so the cache owns no frames.
pub(crate) type LoopGuardKey = Option<(u64, usize, [(usize, u32); 3])>;

fn loop_guard_key(env: &Env) -> LoopGuardKey {
    let mut frames = [(0, 0); 3];
    let mut cur = env.clone();
    for frame in &mut frames {
        if cur.is_search() {
            return Some((crate::env::search_epoch(), cur.id(), frames));
        }
        *frame = (cur.id(), cur.fn_version());
        cur = cur.parent()?;
    }
    cur.is_search().then(|| (crate::env::search_epoch(), cur.id(), frames))
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
    let (state, entry_nonna) = match code.jit.borrow().get(&key) {
        Some(JitSlot::Ready(l)) => (Some(l.clone()), true),
        Some(JitSlot::Never) => return None,
        Some(JitSlot::Failed { attempts, .. }) if *attempts >= 3 => return None,
        Some(JitSlot::Failed { entry_nonna, .. }) => (None, *entry_nonna),
        _ => (None, true),
    };
    let jl = match state {
        Some(l) => l,
        None => {
            // Types of the variables' current values, and of the for
            // sequences set up before the loop.
            let mut types: Vec<Option<JType>> = Vec::with_capacity(code.vars.len());
            let mut nonna = Vec::with_capacity(code.vars.len());
            for (s, sym) in code.vars.iter().enumerate() {
                let value = read_var(vars, env, *sym, s as Slot);
                types.push(value.and_then(loop_type));
                nonna.push(
                    entry_nonna && matches!(value.and_then(scalar), Some(Num::I(x) | Num::L(x)) if x != NA_INTEGER),
                );
            }
            let Op::LoopEnter { brk, .. } = code.ops[enter] else { return None };
            let mut live: HashMap<Reg, JType> = HashMap::default();
            for op in &code.ops[enter + 1..brk as usize] {
                if let Op::ForStep { seq, idx, .. } = op {
                    let set_inside = code.ops[enter + 1..brk as usize].iter().any(
                        |o| matches!(o, Op::ForRange { seq: s2, .. } | Op::ForSeqLength { seq: s2, .. } if s2 == seq),
                    );
                    if !set_inside {
                        if range_of(&regs[*seq as usize]).is_some() && matches!(regs[*idx as usize], RVal::I(_)) {
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
            let r = crate::jit::compile(
                code,
                enter,
                &|s| types[s as usize],
                &|s| nonna[s as usize],
                &|r| live.get(&r).copied(),
                calls,
            );
            let mut cache = code.jit.borrow_mut();
            match r {
                Ok(l) => {
                    if crate::jit::trace() {
                        eprintln!(
                            "jit: compiled loop at {} ({} variables, {} registers)",
                            enter,
                            l.slots.len(),
                            l.reg_idx.len()
                        );
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
                        Some(JitSlot::Failed { attempts, .. }) => *attempts,
                        _ => 0,
                    };
                    cache.insert(key, JitSlot::Failed { attempts: n + 1, entry_nonna });
                    return None;
                }
            }
        }
    };
    // The closures the inlined calls call now: the same ones, or other
    // instances of them (one made by each call of an enclosing function,
    // say): native code depends only on their bodies and formals, the
    // guards and cells on their environments.
    let mut called: SmallVec<[Rc<crate::value::Closure>; 2]> = SmallVec::new();
    for c in &jl.calls {
        let cs = &code.call_sites[c.site as usize];
        if cs.fslot.is_some_and(|s| matches!(vars.slots[s as usize], SlotVal::Dirty(_))) {
            return None;
        }
        let k = match call_site_fun_fast(interp, c.f, cs, env) {
            Some(Value::Closure(k)) if Rc::ptr_eq(&k, &c.clo) && !k.jit.traced() => k,
            Some(Value::Closure(k))
                if Rc::ptr_eq(&k.formals, &c.clo.formals)
                    && matches!((k.code.get(), c.clo.code.get()), (Some(Some(a)), Some(Some(b))) if Rc::ptr_eq(a, b))
                    && !k.jit.traced() =>
            {
                // (Native code does not count calls: see `inline_call`.)
                while k.jit.unsettled() {
                    jit_check(interp, &k);
                }
                k
            }
            _ => return None,
        };
        called.push(k);
    }
    // A closure called inline that is defined in this frame sees its
    // pending assignments, for its variables and for the functions it
    // calls: they go first. seq.default dispatch must see pending
    // assignments before any guard result is reused. Failed entry checks
    // leave execution to the VM.
    if ((!jl.cells.is_empty() && vars.dirty && called.iter().any(|k| crate::jit::closure::on_chain(&k.env, env)))
        || !jl.seq_lengths.is_empty())
        && let Err(e) = vars.flush(interp, env, &code.vars)
    {
        vars.failed = Some(Box::new(e));
        vars.dirty = true;
        return None;
    }
    // Guards: native code assumes them, and that its inlined calls' guards
    // hold (memoized per guard: see `guard_ok`).
    // Group the loop's own guards. Callees can have different environments
    // on each entry, so their guards and targets are still checked below.
    let env_key = loop_guard_key(env);
    if env_key.is_none() || jl.checked_guards.get() != env_key {
        for (sym, at) in &jl.guards {
            // Reuse the individual memos on a group miss: fresh call frames
            // can share a search-path or enclosing-frame guard result.
            // Locked-frame mutations invalidate their epoch in Env.
            if !guard_ok(interp, *sym, env, &code.guard_memo[*at]) {
                return None;
            }
        }
        jl.checked_guards.set(env_key);
    }
    for (c, k) in jl.calls.iter().zip(&called) {
        let cs = &code.call_sites[c.site as usize];
        if !c.guards.iter().all(|(s, memo)| guard_ok(interp, *s, &k.env, memo)) {
            return None;
        }
        for a in cs.inline_args.as_deref().unwrap_or(&[]) {
            if let InlineArg::Arith { sym, memo, .. } | InlineArg::Index { sym, memo, .. } = a {
                if !guard_ok(interp, *sym, env, memo) {
                    return None;
                }
            }
        }
    }
    if jl.builtins && crate::builtins::debug::any_prim_traced() {
        return None;
    }
    if !jl.arg_syms.iter().all(|s| interp.resolves_to_base(*s, env)) {
        return None;
    }
    let mut vals: SmallVec<[u64; 32]> = SmallVec::new();
    vals.resize(jl.nvals, 0);
    for (_, len, idx) in &jl.seq_lengths {
        let n = leaf_num(vars, env, regs, len)?;
        vals[*idx] = seq_length_range(interp, env, n)?;
    }
    // The cells: the bindings the closures called inline see (pending
    // assignments flushed above).
    let mut owners: SmallVec<[Env; 2]> = SmallVec::new();
    if !jl.cells.is_empty() {
        // (The closures called now: their cells are their environments'.)
        let clo_at = |site: u32| jl.calls.iter().position(|c| c.site == site).map(|k| &called[k]);
        for c in &jl.cells {
            let first = clo_at(c.sites[0])?;
            let o = crate::jit::closure::owner(&first.env, c.sym)?;
            for s in &c.sites[1..] {
                if !crate::jit::closure::owner(&clo_at(*s)?.env, c.sym)?.ptr_eq(&o) {
                    return None;
                }
            }
            if c.written && !o.plain_binding(c.sym) {
                return None;
            }
            vals[c.idx] = match (c.ty, plain_lookup(&o, c.sym).as_ref().and_then(scalar)) {
                (JType::D, Some(Num::D(x))) => x.to_bits(),
                (JType::I, Some(Num::I(x))) | (JType::L, Some(Num::L(x))) => x as i64 as u64,
                _ => return None,
            };
            owners.push(o);
        }
    }
    let mut vecs: SmallVec<[crate::jit::VecView; 4]> = SmallVec::new();
    let mut ctx = crate::jit::LoopCtx::default();
    for s in &jl.slots {
        if s.ty.is_vector() && s.superset {
            // `x[[i]] <<- v`: the binding `<<-` assigns, viewed in place (the
            // frame has none of its own, and its cached copy is dropped so
            // that the binding's vector is not shared through it).
            if matches!(vars.slots[s.slot as usize], SlotVal::Dirty(_)) || env.has_local(s.sym) {
                return None;
            }
            vars.slots[s.slot as usize] = SlotVal::Empty;
            let view = super_vec_view(interp, env, s.sym, s.ty)?;
            debug_assert_eq!(vecs.len(), s.idx);
            vecs.push(view);
            continue;
        }
        if s.superset {
            // The variable is the binding `<<-` assigns: the frame must not
            // have one of its own, and writing it back must not fail.
            if matches!(vars.slots[s.slot as usize], SlotVal::Dirty(_))
                || env.has_local(s.sym)
                || !super_target_plain(interp, env, s.sym)
            {
                return None;
            }
        } else if s.written && !s.ty.is_vector() || s.owned {
            let pending = matches!(vars.slots[s.slot as usize], SlotVal::Dirty(_));
            if !deferrable(env, s.sym, &RVal::I(0), pending) {
                return None;
            }
        }
        if s.owned {
            // A buffer of the run's, with the value's elements if needed.
            use crate::jit::Buf;
            let buf = match (s.needs_entry, read_var(vars, env, s.sym, s.slot)) {
                (false, _) => match s.ty {
                    JType::VecD => Buf::D(Vec::new()),
                    _ => Buf::I(Vec::new()),
                },
                (true, Some(RVal::V(Value::Real(x)))) if x.attrs.is_none() && s.ty == JType::VecD => {
                    Buf::D(x.data.clone())
                }
                (true, Some(RVal::V(Value::Int(x)))) if x.attrs.is_none() && s.ty == JType::VecI => {
                    Buf::I(x.data.clone())
                }
                (true, Some(RVal::Range { start, step, n })) if s.ty == JType::VecI => {
                    Buf::I((0..*n as i64).map(|k| (start + k * step) as i32).collect())
                }
                _ => return None,
            };
            let k = ctx.bufs.len() as u64;
            ctx.bufs.push(buf);
            let view = match ctx.bufs.last_mut().unwrap() {
                Buf::I(v) => crate::jit::VecView {
                    ptr: v.as_mut_ptr() as *mut u8,
                    len: v.len() as u64,
                    nrow: v.len() as u64,
                    ncol: k,
                },
                Buf::D(v) => crate::jit::VecView {
                    ptr: v.as_mut_ptr() as *mut u8,
                    len: v.len() as u64,
                    nrow: v.len() as u64,
                    ncol: k,
                },
            };
            debug_assert_eq!(vecs.len(), s.idx);
            vecs.push(view);
            continue;
        }
        if s.ty.is_vector() {
            // (A range the loop indexes is a vector.)
            if let SlotVal::Dirty(r @ RVal::Range { .. }) = &vars.slots[s.slot as usize] {
                vars.slots[s.slot as usize] = SlotVal::Dirty(RVal::V(r.clone().into_value()));
            }
            let ok = match read_var(vars, env, s.sym, s.slot) {
                Some(RVal::V(v)) => !v.is_object() && loop_vec_type(&RVal::V(v.clone())) == Some(s.ty),
                _ => false,
            };
            if !ok || (s.writable && !own_var(vars, env, s.sym, s.slot)) {
                return None;
            }
            let view = match &mut vars.slots[s.slot as usize] {
                SlotVal::Dirty(RVal::V(Value::Real(x))) | SlotVal::Clean(RVal::V(Value::Real(x)))
                    if matches!(s.ty, JType::VecD | JType::MatD) =>
                {
                    let d = if s.writable { Rc::make_mut(x).data.as_mut_ptr() } else { x.data.as_ptr() as *mut f64 };
                    let (nrow, ncol) = jit_matrix_dims(x).unwrap_or((0, 0));
                    crate::jit::VecView { ptr: d as *mut u8, len: x.data.len() as u64, nrow, ncol }
                }
                SlotVal::Dirty(RVal::V(Value::Int(x))) | SlotVal::Clean(RVal::V(Value::Int(x)))
                    if matches!(s.ty, JType::VecI | JType::MatI) =>
                {
                    let d = if s.writable { Rc::make_mut(x).data.as_mut_ptr() } else { x.data.as_ptr() as *mut i32 };
                    let (nrow, ncol) = jit_matrix_dims(x).unwrap_or((0, 0));
                    crate::jit::VecView { ptr: d as *mut u8, len: x.data.len() as u64, nrow, ncol }
                }
                SlotVal::Dirty(RVal::V(Value::Str(x))) | SlotVal::Clean(RVal::V(Value::Str(x)))
                    if matches!(s.ty, JType::VecS | JType::MatS) =>
                {
                    let d = if s.writable {
                        Rc::make_mut(x).data.as_mut_ptr()
                    } else {
                        x.data.as_ptr() as *mut crate::value::RStr
                    };
                    let (nrow, ncol) = jit_matrix_dims(x).unwrap_or((0, 0));
                    crate::jit::VecView { ptr: d as *mut u8, len: x.data.len() as u64, nrow, ncol }
                }
                _ => return None,
            };
            debug_assert_eq!(vecs.len(), s.idx);
            vecs.push(view);
            continue;
        }
        if s.ty == JType::Range {
            if s.needs_entry {
                let (start, step, n) = read_var(vars, env, s.sym, s.slot).and_then(range_of)?;
                vals[s.idx] = start as u64;
                vals[s.idx + 1] = step as u64;
                vals[s.idx + 2] = n;
            }
            continue;
        }
        if s.ty == JType::C {
            if s.needs_entry {
                let RVal::V(Value::Cplx(z)) = read_var(vars, env, s.sym, s.slot)? else { return None };
                if z.attrs.is_some() || z.data.len() != 1 {
                    return None;
                }
                vals[s.idx] = z.data[0].re.to_bits();
                vals[s.idx + 1] = z.data[0].im.to_bits();
            }
            continue;
        }
        let cur = read_var(vars, env, s.sym, s.slot).cloned();
        let bits = match (s.ty, cur.as_ref().and_then(scalar)) {
            (JType::I, Some(Num::I(x))) | (JType::L, Some(Num::L(x))) if s.nonna && x == NA_INTEGER => {
                // Relax all entry assumptions on the retry. Widening once
                // avoids recompilation when different slots alternate NA.
                // Constants and facts derived in the loop remain available.
                code.jit.borrow_mut().insert(key, JitSlot::Failed { attempts: 0, entry_nonna: false });
                return None;
            }
            (JType::D, Some(Num::D(x))) => Some(x.to_bits()),
            (JType::I, Some(Num::I(x))) | (JType::L, Some(Num::L(x))) => Some(x as i64 as u64),
            // A number: an integer (never NA, see `JType::N`) or a double.
            (JType::N, Some(Num::I(x))) if x != NA_INTEGER => {
                vals[s.tag.unwrap()] = 1;
                Some((x as f64).to_bits())
            }
            (JType::N, Some(Num::D(x))) => {
                vals[s.tag.unwrap()] = 0;
                Some(x.to_bits())
            }
            _ => None,
        };
        match bits {
            Some(b) => vals[s.idx] = b,
            None if s.needs_entry => return None,
            None => {}
        }
    }
    // The inlined callees' free variables, from the instance each site
    // calls now (held in `held` for the call): values, and views after the
    // variables'.
    let mut held: SmallVec<[RVal; 4]> = SmallVec::new();
    for (site, sym, ty, idx) in &jl.free {
        let c = &called[jl.calls.iter().position(|c| c.site == *site)?];
        if crate::jit::closure::on_chain(&c.env, env) {
            return None;
        }
        let v = plain_lookup(&c.env, *sym)?;
        if jit_type(&v) != Some(*ty) || matches!(&v, RVal::V(x) if x.is_object()) {
            return None;
        }
        match (&v, ty) {
            (RVal::D(x), JType::D) => vals[*idx] = x.to_bits(),
            (RVal::I(x), JType::I) | (RVal::L(x), JType::L) => vals[*idx] = *x as i64 as u64,
            (RVal::V(Value::Real(x)), JType::VecD | JType::MatD) => {
                let (nrow, ncol) = jit_matrix_dims(x).unwrap_or((0, 0));
                vecs.push(crate::jit::VecView {
                    ptr: x.data.as_ptr() as *mut u8,
                    len: x.data.len() as u64,
                    nrow,
                    ncol,
                });
                debug_assert_eq!(vecs.len(), *idx + 1);
            }
            (RVal::V(Value::Int(x)), JType::VecI | JType::MatI) => {
                let (nrow, ncol) = jit_matrix_dims(x).unwrap_or((0, 0));
                vecs.push(crate::jit::VecView {
                    ptr: x.data.as_ptr() as *mut u8,
                    len: x.data.len() as u64,
                    nrow,
                    ncol,
                });
                debug_assert_eq!(vecs.len(), *idx + 1);
            }
            _ => return None,
        }
        held.push(v);
    }
    // The temporary vectors' buffers, after the variables' views.
    for t in &jl.temps {
        let k = ctx.bufs.len() as u64;
        let buf = if *t == JType::VecD { crate::jit::Buf::D(Vec::new()) } else { crate::jit::Buf::I(Vec::new()) };
        ctx.bufs.push(buf);
        vecs.push(crate::jit::VecView {
            ptr: std::ptr::NonNull::<f64>::dangling().as_ptr() as *mut u8,
            len: 0,
            nrow: 0,
            ncol: k,
        });
    }
    for (reg, ty) in &jl.live_in {
        let idx = jl.reg_idx[reg];
        match (&regs[*reg as usize], ty) {
            (v, JType::Range) => {
                let (start, step, n) = range_of(v)?;
                // The native ForStep index is an i32, including when the
                // interpreter supplied a compact sequence.
                if n > i32::MAX as u64 {
                    return None;
                }
                vals[idx] = start as u64;
                vals[idx + 1] = step as u64;
                vals[idx + 2] = n;
            }
            (RVal::I(i), JType::I) => vals[idx] = *i as i64 as u64,
            _ => return None,
        }
    }
    vals[jl.vis] = interp.visible as u64;
    let entry_cells: SmallVec<[u64; 4]> = jl.cells.iter().map(|c| vals[c.idx]).collect();
    // A closed form, where it applies, leaves `vals` as the native code would
    // at its exit (the visibility flag included: a closed form only ever
    // exits, and the VM sets visibility after a loop where it is used).
    let ret = match &jl.summary {
        Some(sm) if sm.run(&mut vals) => crate::jit::EXIT,
        // SAFETY: `vals` and `vecs` are laid out as `jl` was compiled for, and
        // the vectors they view are held by `regs`, `vars`, `held` and `ctx` for the
        // whole call.
        _ => unsafe { (jl.func)(vals.as_mut_ptr(), vecs.as_mut_ptr(), &mut ctx) },
    };
    interp.visible = vals[jl.vis] != 0;
    // The cells the calls assigned, back to their bindings (checked above to
    // be plain); this frame's cached copies of them are stale.
    for ((c, o), was) in jl.cells.iter().zip(&owners).zip(&entry_cells) {
        let b = vals[c.idx];
        if !c.written || b == *was {
            continue;
        }
        let mut v = match c.ty {
            JType::D => RVal::D(f64::from_bits(b)),
            JType::I => RVal::I(b as i32),
            _ => RVal::L(b as i32),
        };
        let done = set_plain_in_place(o, c.sym, &mut v, false);
        debug_assert!(done);
        for (k, sym) in code.vars.iter().enumerate() {
            if *sym == c.sym && matches!(vars.slots[k], SlotVal::Clean(_)) {
                vars.slots[k] = SlotVal::Empty;
            }
        }
    }
    // Registers the VM continues with (vector ones are copied out of what
    // they view, which the write-back below may take).
    if ret != crate::jit::EXIT {
        for (reg, ty) in jl.deopt_regs.get(&ret).map(|v| v.as_slice()).unwrap_or(&[]) {
            let idx = jl.reg_idx[reg];
            let b = vals[idx];
            regs[*reg as usize] = match ty {
                JType::D => RVal::D(f64::from_bits(b)),
                JType::I => RVal::I(b as i32),
                JType::L => RVal::L(b as i32),
                JType::Range => RVal::Range { start: b as i64, step: vals[idx + 1] as i64, n: vals[idx + 2] },
                JType::C => cplx_of(b, vals[idx + 1]),
                JType::N => number(f64::from_bits(b), vals[idx + 1]),
                JType::S => {
                    // SAFETY: a string register holds the address of an element
                    // of a character vector the loop was entered with, which its
                    // variable still holds (character vectors are not owned
                    // buffers, and nothing has been written back yet).
                    let e = unsafe { (*(b as usize as *const crate::value::RStr)).clone() };
                    RVal::V(Value::Str(Rc::new(crate::value::Vector::new(vec![e]))))
                }
                JType::VecD | JType::VecI => {
                    let (p, n) = (b as usize, vals[idx + 1] as usize);
                    // SAFETY: a live vector register views `n` elements of a
                    // vector held by `vars` or a buffer of `ctx`, both alive.
                    RVal::V(unsafe {
                        if *ty == JType::VecD {
                            Value::reals(std::slice::from_raw_parts(p as *const f64, n).to_vec())
                        } else {
                            Value::Int(Rc::new(crate::value::Vector::new(
                                std::slice::from_raw_parts(p as *const i32, n).to_vec(),
                            )))
                        }
                    })
                }
                _ => continue,
            };
        }
    }
    // Write the variables back as pending assignments.
    for s in &jl.slots {
        if !s.written || (s.ty.is_vector() && !s.owned) {
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
            JType::Range => RVal::Range { start: b as i64, step: vals[s.idx + 1] as i64, n: vals[s.idx + 2] },
            JType::VecD | JType::VecI => {
                let view = &vecs[s.idx];
                let n = view.len as usize;
                let buf = std::mem::replace(&mut ctx.bufs[view.ncol as usize], crate::jit::Buf::I(Vec::new()));
                RVal::V(match buf {
                    crate::jit::Buf::I(mut v) => {
                        v.truncate(n);
                        Value::Int(Rc::new(crate::value::Vector::new(v)))
                    }
                    crate::jit::Buf::D(mut v) => {
                        v.truncate(n);
                        Value::reals(v)
                    }
                })
            }
            JType::C => cplx_of(b, vals[s.idx + 1]),
            JType::N => number(f64::from_bits(b), vals[s.tag.unwrap()]),
            _ => RVal::L(b as i32),
        };
        if s.superset {
            // As `Op::SuperSet` does (checked above not to fail).
            vars.slots[s.slot as usize] = SlotVal::Empty;
            let mut v = v;
            let done = super_set_fast(interp, env, s.sym, &mut v, true);
            debug_assert!(done);
            continue;
        }
        vars.slots[s.slot as usize] = SlotVal::Dirty(v);
        vars.any = true;
        vars.dirty = true;
        vars.touch(s.slot as usize);
    }
    // A register native code cleared is dead; the VM's copy may still hold
    // a value from before the loop (#357).
    let live = if ret == crate::jit::EXIT { None } else { jl.deopt_regs.get(&ret) };
    for reg in &jl.nulled {
        if !live.is_some_and(|l| l.iter().any(|(r, _)| r == reg)) {
            regs[*reg as usize] = DEAD;
        }
    }
    if let Some(c) = jl.counter {
        // A loop that keeps leaving at an op native code does not have
        // without running much of itself costs more than it saves.
        let left = jl.leaves.binary_search(&ret).is_ok() as u64;
        let (n, iters) = jl.stats.get();
        let (n, iters) = (n + left, iters + vals[c]);
        jl.stats.set((n, iters));
        if n >= 4 && iters < 16 * n {
            code.jit.borrow_mut().insert(key, JitSlot::Never);
        }
    }
    if ret == crate::jit::EXIT {
        if crate::jit::trace() {
            eprintln!("jit: loop at {} finished", enter);
        }
        return Some((Outcome::Exited, Vec::new()));
    }
    let p = ret as usize;
    if crate::jit::trace() {
        eprintln!("jit: loop at {} continues in the VM at {}", enter, p);
    }
    let nested = jl.nested.iter().copied().filter(|(e, b, _)| (*e as usize) < p && p <= *b as usize).collect();
    Some((Outcome::Deopt(p), nested))
}

/// A range variable's value: an unmaterialized range, or a compact integer
/// sequence.
fn range_of(v: &RVal) -> Option<(i64, i64, u64)> {
    match v {
        RVal::Range { start, step, n } => Some((*start, *step, *n)),
        RVal::V(Value::Int(x)) => {
            let (n, first, inc) = crate::value::compact_intseq_of(x)?;
            Some((first as i64, inc as i64, n as u64))
        }
        _ => None,
    }
}

/// The head symbol of a call constant: what GNU R prints when tracing in
/// compiled code (`PrintValue(symbol)` in eval.c's GETFUN/GETBUILTIN ops).
fn call_sym(call: &Value) -> Value {
    match call {
        Value::Lang(l) => l.fun_sym().map(Value::Sym).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// A traced function's report: the whole call from the AST interpreter
/// (eval.c:1224), the function's symbol from bytecode (eval.c:7894).
/// Out of line, as are the other trace helpers: `run_with` must not grow.
#[cold]
#[inline(never)]
fn trace_report(interp: &mut Interp, call: &Value) {
    let shown = if crate::builtins::debug::in_compiled_frame(interp) { call_sym(call) } else { call.clone() };
    crate::builtins::debug::print_trace(interp, &shown);
}

/// [`trace_report`] for a call of builtin `b`, if it is traced.
#[cold]
#[inline(never)]
fn trace_builtin(interp: &mut Interp, b: &'static Builtin, call: &Value) {
    if crate::builtins::debug::is_traced(&Value::Builtin(b)) {
        trace_report(interp, call);
    }
}

/// [`trace_builtin`] for a `Builtin1` op, whose builtin is named by `call`.
#[cold]
#[inline(never)]
fn trace_builtin1(interp: &mut Interp, call: &Value) {
    if let Value::Lang(l) = call
        && let Some(b) = l.fun_sym().and_then(|s| crate::builtins::lookup_primitive(&s.name()))
    {
        trace_builtin(interp, b, call);
    }
}

/// [`builtin1_fast`], off while any builtin is traced (see `Op::Builtin1`).
#[inline(always)]
fn builtin1_untraced(f: Builtin1, x: &RVal) -> Option<RVal> {
    if crate::builtins::debug::any_prim_traced() { None } else { builtin1_fast(f, x) }
}

/// The function a call site calls, memoized like [`guard_ok`]: frames below
/// the search path are checked directly, the search-path lookup is cached
/// against the search epoch (only when it needs no forcing or active
/// bindings, i.e. when [`Interp::search_fun`] answers it).
/// `Op::Switch` on the value `xv`: the code to continue at, `None` where
/// `switch_slow` must signal an error or a warning.
// R: eval.c:do_switch
#[inline(never)]
fn switch_target(xv: &Value, t: &crate::compile::SwitchTable) -> Option<u32> {
    if !xv.is_vector() || xv.len() != 1 || xv.inherits("factor") {
        return None;
    }
    Some(match xv {
        Value::Str(sv) => {
            // pmatch() compares CHAR(): NA_STRING is "NA".
            let key: &str = sv.data[0].as_deref().unwrap_or("NA");
            match t.names.iter().position(|n| n.is_some_and(|n| n.with_name(|m| m == key))) {
                Some(k) => t.by_name[k],
                None => t.default.unwrap_or(t.null),
            }
        }
        _ => {
            let k = crate::coerce::as_integer(xv);
            if k != NA_INTEGER && k >= 1 && k as usize <= t.by_number.len() {
                t.by_number[k as usize - 1]?
            } else {
                t.null
            }
        }
    })
}

/// [`switch_target`] where it signals (after a flush): the errors, and the
/// warning for a factor, which then switches on its codes. The outer error
/// leaves the dispatch loop at once (as `return signal(...)` there); the
/// inner one is the warning's, which may unwind to a loop of the frame.
#[cold]
#[inline(never)]
fn switch_slow(
    interp: &mut Interp,
    call: &Value,
    t: &crate::compile::SwitchTable,
    xv: &Value,
) -> Result<RResult<u32>, Flow> {
    if !xv.is_vector() || xv.len() != 1 {
        signal(interp, Err(Flow::error_call(call.clone(), "EXPR must be a length 1 vector")))?;
        unreachable!("an error signals");
    }
    if xv.inherits("factor") {
        let r = interp.warningcall(
            call,
            "EXPR is a \"factor\", treated as integer.\n Consider using 'switch(as.character( * ), ...)' instead.",
        );
        if let Err(e) = signal(interp, r.map(|_| Value::Null)) {
            return Ok(Err(e));
        }
    }
    let k = crate::coerce::as_integer(xv);
    if k != NA_INTEGER && k >= 1 && k as usize <= t.by_number.len() {
        return match t.by_number[k as usize - 1] {
            Some(c) => Ok(Ok(c)),
            None => {
                signal(interp, Err(Flow::error("empty alternative in numeric switch")))?;
                unreachable!("an error signals");
            }
        };
    }
    Ok(Ok(t.null))
}

/// `x$name` where `dollar_fast` does not do (after a flush): the default
/// method, or for an object, `$` dispatching on `x`'s value.
#[inline(never)]
fn dollar_slow(interp: &mut Interp, xv: Value, input: &RStr, call: &Value, env: &Env) -> RResult<RVal> {
    if !xv.is_object() {
        let r = crate::builtins::subset::subset3_dflt(interp, &xv, input, call);
        return Ok(unbox(signal(interp, r)?));
    }
    // R_DispatchOrEvalSP with `x` evaluated.
    let Value::Lang(l) = call else { unreachable!() };
    let args = vec![
        Arg::new(l.args[0].tag, Value::Promise(Rc::new(Promise::forced(l.args[0].value.clone(), xv)))),
        Arg::new(l.args[1].tag, Value::strs(vec![input.clone()])),
    ];
    let b = crate::builtins::lookup_primitive("$").expect("$ builtin");
    let r = interp.invoke_builtin(b, call, args, env);
    Ok(RVal::V(signal(interp, r)?))
}

/// `object@name` where `slot_read_fast` does not do (after a flush): `@`
/// with the object's value.
#[inline(never)]
fn slot_read_slow(interp: &mut Interp, xv: Value, call: &Value, env: &Env) -> RResult<RVal> {
    let Value::Lang(l) = call else { unreachable!() };
    let args = vec![
        Arg::new(l.args[0].tag, Value::Promise(Rc::new(Promise::forced(l.args[0].value.clone(), xv)))),
        l.args[1].clone(),
    ];
    let b = crate::builtins::lookup_primitive("@").expect("@ builtin");
    let r = interp.invoke_builtin(b, call, args, env);
    Ok(unbox(signal(interp, r)?))
}

/// The sequence `Op::ForInit` iterates over (not a range): a factor's
/// levels as strings; an error (to signal) unless it is a vector, NULL or
/// a pairlist.
#[inline(never)]
fn for_seq(seq: &RVal, call: &Value) -> RResult<Value> {
    let mut v = seq.to_value();
    if v.inherits("factor") {
        v = crate::builtins::envir::as_character_factor(&v)?;
    }
    if !(v.is_vector() || v.is_null() || matches!(v, Value::Pairlist(_))) {
        return Err(Flow::error_call(call.clone(), "invalid for() loop sequence"));
    }
    Ok(v)
}

/// Element `i` of a `for` sequence (`None` past its end).
#[inline(never)]
fn for_element(sv: &Value, i: usize) -> Option<RVal> {
    if i >= sv.len() {
        return None;
    }
    Some(match sv {
        Value::Int(x) => RVal::I(x.data[i]),
        Value::Real(x) => RVal::D(x.data[i]),
        Value::Lgl(x) => RVal::L(x.data[i]),
        Value::Str(x) => RVal::V(Value::strs(vec![x.data[i].clone()])),
        Value::Cplx(x) => RVal::V(Value::cplx(x.data[i])),
        Value::Raw(x) => RVal::V(Value::raws(vec![x.data[i]])),
        Value::List(x) | Value::Expr(x) => RVal::V(x.data[i].clone()),
        Value::Pairlist(p) => RVal::V(p.items[i].value.clone()),
        _ => unreachable!(),
    })
}

/// The `for` index of a package ALTREP sequence's element 0 (Op::ForInit):
/// negative, so that ordinary element access finds no element.
const ALT_FOR: i32 = i32::MIN;

/// Element `i` of a package ALTREP `for` sequence, from its class's `Elt`
/// (crate::altrep). Before element 0 the loop variable is bound to NULL,
/// as for any sequence (the frame is flushed: `define` writes it).
#[cold]
#[inline(never)]
fn for_altrep_element(interp: &mut Interp, env: &Env, var: Sym, sv: &Value, i: usize) -> RResult<Value> {
    if i == 0 {
        interp.define(var, Value::Null, env)?;
    }
    let Some(h) = crate::altrep::lookup(sv) else { unreachable!() };
    crate::altrep::elt(interp, h, i)
}

/// The general case of `Op::Call` (after a flush): find the function
/// (`fast_fun` if already found) and call it.
#[inline(never)]
fn call_op(
    interp: &mut Interp,
    site: &CallSite,
    f: Sym,
    fast_fun: Option<Value>,
    env: &Env,
    vars: &mut Vars<'_>,
    regs: &[RVal],
) -> RResult<RVal> {
    let fun = match fast_fun {
        Some(f) => f,
        None => call_site_fun(interp, f, site, env)?,
    };
    if crate::builtins::debug::is_traced(&fun) {
        trace_report(interp, &site.call);
    }
    Ok(match (&fun, &site.call) {
        (Value::Closure(clo), Value::Lang(l)) => closure_call(interp, site, clo, l, env, vars, regs)?,
        // A builtin (say the value of a variable): its arguments through
        // their bytecode, left to right, as evalList.
        (Value::Builtin(b), Value::Lang(l))
            if b.kind == crate::value::BuiltinKind::Builtin
                && l.args.iter().all(|a| !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS)) =>
        {
            // (As `eval` of the call would: cached variables are stale if an
            // argument assigns one.)
            crate::env::bump_r_epoch();
            let mut argv = Vec::with_capacity(l.args.len());
            for (a, c) in l.args.iter().zip(&site.codes) {
                let v = match c {
                    Some(c) => match run_simple(interp, c, env) {
                        Some(v) => v,
                        None => run(interp, c, env)?,
                    },
                    // (a variable: its value, unboxed scalars as they are,
                    // except for native code, which gets the binding's
                    // object as `CallBuiltin` hands it over)
                    None => match a.value.as_sym().filter(|_| !crate::compile::native_call(b)) {
                        Some(sym) => match plain_lookup(env, sym) {
                            Some(r) => r.into_value(),
                            None => interp.eval(&a.value, env)?,
                        },
                        None => interp.eval(&a.value, env)?,
                    },
                };
                argv.push(Arg::new(a.tag, v));
            }
            let r = interp.invoke_builtin(b, &site.call, argv, env);
            RVal::V(signal(interp, r)?)
        }
        _ => RVal::V(interp.eval(&site.call, env)?),
    })
}

/// `Op::CallBuiltin` (after a flush): the builtin `consts[f]` with the
/// values of `arglists[args]`, which are moved out of their registers (for
/// a `native` call, a variable's binding object: [`native_args`]).
#[inline(never)]
fn call_builtin_op(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    f: u32,
    args: u32,
    call: u32,
    native: bool,
) -> RResult<Value> {
    let Value::Builtin(b) = &code.consts[f as usize] else { unreachable!() };
    if crate::builtins::debug::any_prim_traced() {
        trace_builtin(interp, b, &code.consts[call as usize]);
    }
    if native {
        return native_builtin_op(interp, code, env, regs, b, args, call);
    }
    let argv: Vec<Arg> = code.arglists[args as usize]
        .iter()
        .map(|(tag, r)| Arg::new(*tag, std::mem::replace(&mut regs[*r as usize], RVal::I(0)).into_value()))
        .collect();
    let r = interp.invoke_builtin(b, &code.consts[call as usize], argv, env);
    signal(interp, r)
}

fn call_site_fun(interp: &mut Interp, sym: Sym, site: &CallSite, env: &Env) -> RResult<Value> {
    match call_site_fun_fast(interp, sym, site, env) {
        Some(f) => Ok(f),
        None => interp.find_fun_for_call(sym, &site.call, env),
    }
}

/// [`call_site_fun`] when it needs no forcing, active bindings or errors
/// (so it cannot run R code); `None` otherwise.
pub(crate) fn call_site_fun_fast(interp: &mut Interp, sym: Sym, site: &CallSite, env: &Env) -> Option<Value> {
    let mut cur = env.clone();
    while !cur.is_search() {
        if cur.local_may_bind_fun(sym) {
            // A function bound in a local frame (say a helper defined in
            // the enclosing function, or an argument already forced), as
            // `findFun` finds it; anything that would need forcing, an
            // active binding or an error takes the slow path.
            let frame = cur.0.frame().borrow();
            let b = frame.get(sym)?;
            if b.flags & crate::env::BIND_ACTIVE != 0 {
                return None;
            }
            match &b.value {
                Value::Closure(_) | Value::Builtin(_) => return Some(b.value.clone()),
                Value::Promise(p) => match &*p.value.borrow() {
                    crate::value::PromiseVal::Boxed(v @ (Value::Closure(_) | Value::Builtin(_))) => {
                        return Some(v.clone());
                    }
                    crate::value::PromiseVal::Boxed(v) if !v.is_missing_arg() => {}
                    _ => return None,
                },
                _ => return None,
            }
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
    run_pre(interp, code, env, false, false)
}

/// Run a closure body in its fresh call frame `env`. For a closure defined
/// on the search path, such a frame holds only the formals: if none is
/// named like a guarded function, every guard's result depends only on the
/// search path, so all are checked once per search epoch (and closure)
/// rather than once per call, and hold until R code first runs. The same
/// holds, per version of its function bindings, for a closure defined in a
/// frame directly below the search path (say a helper defined in a function
/// called from the global environment) that is called repeatedly.
///
/// With `unboxed` (the compiled caller asked, `Interp::call_closure_ex`), a
/// scalar result of a frame without `on.exit`
/// code is left unboxed (see [`body_result`]) and a placeholder returned:
/// nothing runs between the return and the caller taking it.
pub fn run_closure(interp: &mut Interp, code: &Code, env: &Env, clo: &Closure, unboxed: bool) -> RResult<Value> {
    let pre = code.nguards > 0
        && if clo.env.is_search() { fresh_guards_ok(interp, code, clo) } else { fresh_nested_ok(interp, code, clo) };
    run_pre(interp, code, env, pre, unboxed)
}

#[inline]
fn fresh_guards_ok(interp: &mut Interp, code: &Code, clo: &Closure) -> bool {
    let key = (crate::env::search_epoch(), clo.env.id(), Rc::as_ptr(&clo.formals) as *const Arg as usize);
    if code.fresh_guards.get() == key {
        return true;
    }
    if !guards_hold_fresh(interp, code, clo) {
        return false;
    }
    code.fresh_guards.set(key);
    true
}

#[inline(never)]
fn fresh_nested_ok(interp: &mut Interp, code: &Code, clo: &Closure) -> bool {
    let key = (
        crate::env::search_epoch(),
        clo.env.id(),
        clo.env.fn_version(),
        Rc::as_ptr(&clo.formals) as *const Arg as usize,
    );
    // (Only ever set for an environment that qualifies, which it keeps:
    // locked environments stay locked.)
    if code.fresh_nested.get() == key {
        return true;
    }
    if !clo.env.parent_ref().as_ref().is_some_and(|p| p.is_search() || locked_to_search(p)) {
        return false;
    }
    // A closure made in a call of a function of the search path or a
    // namespace (`function(x) ...` passed to lapply): each call of the
    // outer function makes a new one, in a new frame. Its guards depend on
    // that frame's own function bindings and the enclosure's (locked, or on
    // the search path), so the enclosure is the memo.
    let outer = match &*clo.env.parent_ref() {
        Some(p) if p.is_search() || p.0.locked.get() => Some((key.0, p.id(), p.fn_version(), key.3)),
        _ => None,
    };
    if let Some(okey) = outer {
        if code.guard_syms.iter().any(|s| clo.env.local_may_bind_fun(*s)) {
            return false;
        }
        if code.fresh_outer.get() != okey {
            if !guards_hold_fresh(interp, code, clo) {
                return false;
            }
            code.fresh_outer.set(okey);
        }
        // (As below: callers read it too, see `eager_args`.)
        code.fresh_nested.set(key);
        return true;
    }
    // Environments below the search path come and go with calls: check
    // only for a closure called again with the same environment (checking
    // every guard costs more than one call saves).
    if code.fresh_seen.replace(key) != key || !guards_hold_fresh(interp, code, clo) {
        return false;
    }
    code.fresh_nested.set(key);
    true
}

/// A call frame whose enclosure is a namespace (see [`locked_to_search`]):
/// its guards, like those of frames directly below the search path,
/// depend only on the search epoch and its own function bindings.
#[inline(never)]
fn frame_of_locked(env: &Env) -> bool {
    env.parent_ref().as_ref().is_some_and(|p| p.0.locked.get() && locked_to_search(p))
}

/// Whether `env` and its enclosures up to the search path are all locked
/// (a namespace, its imports, the base namespace): no binding can appear
/// in them, so function lookups through them depend only on their
/// function bindings and the search path.
#[inline(never)]
fn locked_to_search(env: &Env) -> bool {
    let mut cur = env.clone();
    loop {
        if cur.is_search() {
            return true;
        }
        if !cur.0.locked.get() {
            return false;
        }
        cur = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
    }
}

/// Whether every guard of `code` holds in a fresh call frame of `clo`.
fn guards_hold_fresh(interp: &mut Interp, code: &Code, clo: &Closure) -> bool {
    !clo.formals.iter().any(|f| f.tag.is_some_and(|t| code.guard_syms.contains(&t)))
        && code.guard_syms.iter().all(|s| interp.resolves_to_base(*s, &clo.env))
}

/// Give the empty `v` `n` elements of `value()`, written in place
/// (`resize_with` costs ~25 instructions per element). It reserves only when
/// the inline buffer is too small: the call costs more than a small
/// closure's frame setup otherwise.
#[inline(always)]
fn fill_empty<A: smallvec::Array>(v: &mut SmallVec<A>, n: usize, value: impl Fn() -> A::Item) {
    debug_assert!(v.is_empty());
    if n > v.inline_size() {
        v.reserve_exact(n);
    }
    // SAFETY: the capacity is at least n, and all n elements are written
    // before the length covers them.
    unsafe {
        let p = v.as_mut_ptr();
        for k in 0..n {
            p.add(k).write(value());
        }
        v.set_len(n);
    }
}

fn run_pre(interp: &mut Interp, code: &Code, env: &Env, pre: bool, unboxed: bool) -> RResult<Value> {
    // Registers and slots live on the Rust stack for typical code, so a
    // call does not allocate them.
    // (Sized for typical base-R functions, which a call should not have
    // to allocate for.)
    let mut regs: SmallVec<[RVal; 32]> = SmallVec::new();
    let mut slots: SmallVec<[SlotVal; 24]> = SmallVec::new();
    fill_empty(&mut regs, code.nregs, || RVal::V(Value::Null));
    fill_empty(&mut slots, code.vars.len(), || SlotVal::Empty);
    let mut vars = Vars {
        slots: &mut slots,
        any: false,
        dirty: false,
        vclean: false,
        touched: 0,
        epoch: crate::env::r_epoch(),
        unboxed,
        failed: None,
    };
    // Valid in the first flush generation (see `run_with`).
    let init = pre as u32;
    let mut small = std::mem::MaybeUninit::<[u32; 128]>::uninit();
    let mut big = Vec::new();
    let checked: &mut [u32] = if code.nguards <= 128 {
        // Only the guards the code has are initialized.
        let p = small.as_mut_ptr() as *mut u32;
        // SAFETY: `nguards <= 128`, the array's length, and each of the
        // `nguards` elements in the slice is written first.
        unsafe {
            for k in 0..code.nguards {
                p.add(k).write(init);
            }
            std::slice::from_raw_parts_mut(p, code.nguards)
        }
    } else {
        big.resize(code.nguards, init);
        &mut big
    };
    run_with(interp, code, env, &mut regs, &mut vars, checked)
}

/// `Op::Srcref`: R_Srcref is a statement's srcref (`v`), or NULL.
#[cold]
#[inline(never)]
fn set_srcref(interp: &mut Interp, v: &Value) {
    interp.srcref = match v {
        Value::Int(v) => Some(v.clone()),
        _ => crate::interp::null_srcref(),
    };
}

/// `Op::SrcrefSave`: R_Srcref as a register value (`NULL` for `None`).
#[cold]
#[inline(never)]
fn saved_srcref(interp: &Interp) -> Value {
    match &interp.srcref {
        Some(v) => Value::Int(v.clone()),
        None => Value::Null,
    }
}

/// `Op::SrcrefRestore`.
#[cold]
#[inline(never)]
fn set_srcref_saved(interp: &mut Interp, r: &RVal) {
    interp.srcref = match r {
        RVal::V(Value::Int(v)) => Some(v.clone()),
        _ => None,
    };
}

fn run_with(
    interp: &mut Interp,
    code: &Code,
    env: &Env,
    regs: &mut [RVal],
    mut vars: &mut Vars<'_>,
    // The `flush!()` generation in which each guard last held: only R code
    // (always run after a flush) can rebind functions, so a guard that held
    // in the current generation needs no new check.
    checked: &mut [u32],
) -> RResult<Value> {
    // (In memory, not registers: the dispatch loop needs those for its hot
    // values; see `GuardState`.)
    let mut gs = GuardState::new(env);
    interp.visible = true;
    // Active loops.
    let mut loops: Vec<LoopRec> = Vec::new();
    let ops = &code.ops;
    let mut pc: usize = 0;
    // (The error path is one out-of-line function: the dispatch loop has
    // dozens of these.)
    macro_rules! tryop {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(f) => match unwind_to(f, env, &loops) {
                    Ok(p) => {
                        pc = p;
                        continue;
                    }
                    Err(f) => return Err(f),
                },
            }
        };
    }
    // Before anything that may run R code or signal.
    macro_rules! flush {
        () => {{
            gs.flushed();
            tryop!(vars.flush(interp, env, &code.vars))
        }};
    }
    // Before relying on `checked`: start a new generation if R code that
    // ran since the last flush changed a function binding.
    macro_rules! revalidate {
        () => {{
            if gs.recheck {
                gs.revalidate(env, code);
            }
        }};
    }
    // `ArithL`/`RelL`: check the folded guard (if any), read the operands
    // in place, take `$fast` for scalars, else call the builtin.
    macro_rules! leaf_op {
        ($dst:expr, $a:expr, $b:expr, $call:expr, $sym:expr, $g:expr, $fast:expr, $kind:expr) => {
            leaf_op!($dst, $a, $b, $call, $sym, $g, $fast, $kind, None::<ArithOp>)
        };
        ($dst:expr, $a:expr, $b:expr, $call:expr, $sym:expr, $g:expr, $fast:expr, $kind:expr, $dbl:expr) => {{
            let (dst, call, g) = (*$dst as usize, *$call as usize, *$g);
            if g != NO_GUARD {
                revalidate!();
            }
            if g != NO_GUARD && checked[g as usize] != gs.generation {
                if guard_ok(interp, *$sym, env, &code.guard_memo[pc - 1]) {
                    checked[g as usize] = gs.generation;
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
                    let pure = match $dbl {
                        Some(op) if leaf_may_be_pure(&vars, &regs, $a) => {
                            leaf_arith_pure(code, pc, dst as Reg, op, $a, $b, &mut vars, env, regs)
                        }
                        _ => None,
                    };
                    let r = match pure {
                        Some(r) => r,
                        None => {
                            flush!();
                            tryop!(leaf_op_slow(interp, code, pc, dst as Reg, $kind, $a, $b, &mut vars, env, regs))
                        }
                    };
                    match r {
                        Slow::Val(v) => v,
                        Slow::Next(r, v) => {
                            pc += 1;
                            regs[r as usize] = v;
                            continue;
                        }
                    }
                }
            };
        }};
    }
    // Run the innermost loop natively from its head (the op being executed
    // is at the head or the back edge, so the VM state is that of the loop
    // between iterations) if it compiles; continue after it or where the
    // native code gave up.
    macro_rules! jit_loop {
        () => {{
            let rec = *loops.last().unwrap();
            match jit_enter(interp, code, (rec.enter & !HEAT_BIT) as usize, env, regs, &mut vars) {
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
            // R: eval.c:BC_CHECK_SIGINT at GOTO/BRIFNOT/STEPFOR: a due
            // collection, with this frame's assignments written out first
            // (the finalizers it runs are R code)
            if crate::autogc::due() {
                flush!();
                crate::autogc::collect(interp);
            }
            if gs.jit_on {
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
                vars.touch(slot);
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
        #[cfg(feature = "op-stats")]
        op_stats::count(op);
        pc += 1;
        match op {
            Op::Const { dst, k } => regs[*dst as usize] = RVal::V(code.consts[*k as usize].clone()),
            Op::ConstD { dst, x } => regs[*dst as usize] = RVal::D(*x),
            Op::ConstI { dst, x } => regs[*dst as usize] = RVal::I(*x),
            Op::ConstL { dst, x } => regs[*dst as usize] = RVal::L(*x),
            Op::Null { dst, dead: false } => regs[*dst as usize] = RVal::V(Value::Null),
            Op::Null { dst, dead: true } => regs[*dst as usize] = DEAD,
            Op::Visible { on } => interp.visible = *on,
            // R: eval.c:do_begin (only code with source references)
            Op::Srcref { k } => set_srcref(interp, &code.consts[*k as usize]),
            Op::SrcrefSave { dst } => regs[*dst as usize] = RVal::V(saved_srcref(interp)),
            Op::SrcrefRestore { src } => set_srcref_saved(interp, &regs[*src as usize]),
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
                    if cacheable {
                        vars.sync();
                        vars.cache(slot, v.clone());
                    }
                    regs[*dst as usize] = v;
                }
            }
            Op::SetVar { src, sym, slot, take } => {
                // R: bcEval SETVAR then POP: in effect position the value
                // lives on in the binding only (#357).
                let v = if *take { take_reg(regs, *src) } else { regs[*src as usize].clone() };
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
                        // Doubles and complex scalars without a flush (out of
                        // line, behind a cheap test of the first operand).
                        let pure = if may_be_pure(&regs[*a as usize]) {
                            reg_arith_pure(code, pc, *dst, *op, *a, *b, vars, env, regs)
                        } else {
                            None
                        };
                        let r = match pure {
                            Some(r) => r,
                            None => {
                                flush!();
                                tryop!(arith_slow(interp, code, pc, *dst, *op, *a, *b, *call, vars, env, regs))
                            }
                        };
                        match r {
                            Slow::Val(v) => v,
                            Slow::Next(r, v) => {
                                pc += 1;
                                regs[r as usize] = v;
                                continue;
                            }
                        }
                    }
                };
            }
            Op::ArithL { op, dst, a, b, call, sym, g } => {
                leaf_op!(dst, a, b, call, sym, g, |x, y| arith_fast(*op, x, y), Kind::Arith(*op, *call), Some(*op));
            }
            Op::RelL { op, dst, a, b, call, sym, g } => {
                leaf_op!(dst, a, b, call, sym, g, |x, y| Some(rel_fast(*op, x, y)), Kind::Rel(*op, *call));
            }
            Op::Rel { op, dst, a, b, call } => {
                let fast = match (scalar(&regs[*a as usize]), scalar(&regs[*b as usize])) {
                    (Some(x), Some(y)) => Some(rel_fast(*op, x, y)),
                    _ => str_eq_fast(*op, &regs[*a as usize], &regs[*b as usize]),
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
                    Some(Num::I(x)) | Some(Num::L(x)) => {
                        RVal::L(if x == NA_INTEGER { NA_LOGICAL } else { (x == 0) as Lgl })
                    }
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
                    _ => {
                        // R: bcEval BRIFNOT pops the condition: the
                        // register does not keep it (#357).
                        let v = take_reg(regs, *cond).into_value();
                        flush!();
                        let c = code.consts[*call as usize].clone();
                        let r = crate::coerce::as_condition(interp, &v, &c).map(Value::lgl);
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
                if checked[*g as usize] == gs.generation {
                    continue;
                }
                let ok = guard_ok(interp, *sym, env, &code.guard_memo[pc - 1]);
                if ok {
                    checked[*g as usize] = gs.generation;
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
                let mut fast = match scalar_index(vars, env, regs, i) {
                    Some(ix) => read_var(vars, env, *x, *slot).and_then(|xv| element(xv, ix as usize, *two)),
                    None => None,
                };
                if let (None, Leaf::R(r), false) = (&fast, i, *two) {
                    if let RVal::Range { start, step, n } = regs[*r as usize] {
                        fast = read_var(vars, env, *x, *slot).and_then(|xv| range_subset(xv, start, step, n));
                    }
                }
                if fast.is_none() && vars.get(*slot as usize).is_none() && unforced_promise(env, *x) {
                    // `x` is an argument not yet forced: force it (`[`/`[[`
                    // evaluate it first anyway), then retry the fast path.
                    flush!();
                    let r = interp.eval_sym(*x, env);
                    tryop!(signal(interp, r));
                    fast = match scalar_index(vars, env, regs, i) {
                        Some(ix) => read_var(vars, env, *x, *slot).and_then(|xv| element(xv, ix as usize, *two)),
                        None => None,
                    };
                }
                regs[*dst as usize] = match fast {
                    Some(v) => v,
                    None => {
                        flush!();
                        tryop!(index_slow(interp, code, env, regs, vars, (*x, *slot), i, *two, *k))
                    }
                };
            }
            Op::Dollar { dst, x, name, call } => {
                interp.visible = true;
                let xv = std::mem::replace(&mut regs[*x as usize], RVal::I(0)).into_value();
                let Value::Str(input) = &code.consts[*name as usize] else { unreachable!() };
                let input = &input.data[0];
                let call = &code.consts[*call as usize];
                regs[*dst as usize] = match (xv.is_object(), dollar_fast(&xv, input)) {
                    (false, Some(v)) => unbox(v),
                    _ => {
                        flush!();
                        tryop!(dollar_slow(interp, xv, input, call, env))
                    }
                };
            }
            Op::SlotRead { dst, object, name, call } => {
                interp.visible = true;
                let fast = if interp.methods_dispatch_on() {
                    match &regs[*object as usize] {
                        RVal::V(xv) => slot_read_fast(xv, *name),
                        _ => None,
                    }
                } else {
                    None
                };
                let call = &code.consts[*call as usize];
                regs[*dst as usize] = match fast {
                    Some(v) => unbox(v),
                    None => {
                        flush!();
                        let xv = std::mem::replace(&mut regs[*object as usize], RVal::I(0)).into_value();
                        tryop!(slot_read_slow(interp, xv, call, env))
                    }
                };
            }
            Op::Missing { dst, sym, call } => {
                // Pending assignments first: assigning clears the missing bit.
                if vars.dirty {
                    tryop!(vars.flush(interp, env, &code.vars));
                }
                regs[*dst as usize] =
                    match crate::builtins::lang::missing_sym(interp, &code.consts[*call as usize], *sym, env) {
                        Ok(b) => RVal::L(b as Lgl),
                        Err(e) => return signal(interp, Err(e)),
                    };
                interp.visible = true;
            }
            Op::Switch { x, table, call } => {
                let xv = std::mem::replace(&mut regs[*x as usize], RVal::I(0)).into_value();
                pc = match switch_target(&xv, &code.switches[*table as usize]) {
                    Some(p) => p,
                    None => {
                        // An error or a warning to signal.
                        flush!();
                        // (`?`: its errors leave at once; the warning's may unwind.)
                        tryop!(switch_slow(interp, &code.consts[*call as usize], &code.switches[*table as usize], &xv)?)
                    }
                } as usize;
                interp.visible = true;
            }
            Op::SubsetObj { dst, x, two, k, end } => {
                if matches!(&regs[*x as usize], RVal::V(v) if v.is_object()) {
                    flush!();
                    let xv = std::mem::replace(&mut regs[*x as usize], RVal::I(0)).into_value();
                    let r = subset_values(interp, &code.consts[*k as usize], xv, vec![], *two, env);
                    regs[*dst as usize] = RVal::V(tryop!(signal(interp, r)));
                    interp.visible = true;
                    pc = *end as usize;
                }
            }
            Op::Subset { dst, x, i, two, k } => {
                interp.visible = true;
                let fast = match &regs[*i as usize] {
                    RVal::Range { start, step, n } if !*two => range_subset(&regs[*x as usize], *start, *step, *n),
                    _ => match scalar_index(vars, env, regs, &Leaf::R(*i)) {
                        Some(ix) => element(&regs[*x as usize], ix as usize, *two),
                        None => None,
                    },
                };
                regs[*dst as usize] = match fast {
                    Some(v) => {
                        regs[*x as usize] = RVal::I(0);
                        regs[*i as usize] = RVal::I(0);
                        v
                    }
                    None => {
                        flush!();
                        let xv = std::mem::replace(&mut regs[*x as usize], RVal::I(0)).into_value();
                        let iv = std::mem::replace(&mut regs[*i as usize], RVal::I(0)).into_value();
                        let r = subset_values(interp, &code.consts[*k as usize], xv, vec![iv], *two, env);
                        RVal::V(tryop!(signal(interp, r)))
                    }
                };
            }
            Op::Index2 { dst, x, slot, i, j, two, k } => {
                interp.visible = true;
                let mut fast = matrix_element_at(vars, env, regs, (*x, *slot, None), i, j);
                if fast.is_none() && vars.get(*slot as usize).is_none() && unforced_promise(env, *x) {
                    // `x` is an argument not yet forced: force it (`[`
                    // evaluates it first anyway), then retry.
                    flush!();
                    let r = interp.eval_sym(*x, env);
                    tryop!(signal(interp, r));
                    fast = matrix_element_at(vars, env, regs, (*x, *slot, None), i, j);
                }
                regs[*dst as usize] = match fast {
                    Some(v) => v,
                    None => {
                        flush!();
                        tryop!(index2_slow(interp, code, env, regs, vars, (*x, *slot), i, j, *two, *k))
                    }
                };
            }
            Op::IndexN { dst, x, slot, idx, k } => {
                interp.visible = true;
                regs[*dst as usize] = match index_n_fast(code, vars, env, regs, *x, *slot, *idx) {
                    Some(v) => v,
                    None => {
                        flush!();
                        RVal::V(tryop!(interp.eval(&code.consts[*k as usize], env)))
                    }
                };
            }
            Op::SetIndexN { val, x, slot, idx, k, take } => {
                if !set_index_n_fast(code, vars, env, regs, *val, *x, *slot, *idx, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[], *val, *k, false, *take));
                }
            }
            Op::SuperSetIndexN { val, x, slot, idx, k, take } => {
                if !super_set_index_n_fast(interp, code, vars, env, regs, *val, *x, *slot, *idx, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[], *val, *k, true, *take));
                }
            }
            Op::SetComplex { val, slot, k, take } => {
                flush!();
                let Value::Lang(l) = &code.consts[*k as usize] else { unreachable!() };
                regs[*val as usize] = RVal::V(tryop!(interp.eval(&l.args[1].value, env)));
                tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[], *val, *k, false, *take));
            }
            Op::SetIndex2 { val, x, slot, i, j, two, k, take } => {
                if !set_index2_fast(vars, env, regs, *val, *x, *slot, i, j, *two, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[i, j], *val, *k, false, *take));
                }
            }
            Op::SuperSet { src, sym, slot, take } => {
                // The current frame's cached value of `sym` may be the old
                // value of the binding assigned here.
                if !matches!(vars.slots[*slot as usize], SlotVal::Dirty(_)) {
                    vars.slots[*slot as usize] = SlotVal::Empty;
                }
                if !super_set_fast(interp, env, *sym, &mut regs[*src as usize], *take) {
                    flush!();
                    let v = if *take { take_reg(regs, *src).into_value() } else { regs[*src as usize].to_value() };
                    let r = interp.super_assign(*sym, v, env);
                    tryop!(signal(interp, r.map(|_| Value::Null)));
                }
            }
            Op::SuperSetIndex { val, x, slot, i, two, k, take } => {
                if !super_set_index_fast(interp, vars, env, regs, *val, *x, *slot, i, *two, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[i], *val, *k, true, *take));
                }
            }
            Op::SetDollar { val, x, slot, name, k, take } => {
                if !set_dollar_fast(code, vars, env, regs, *val, *x, *slot, *name, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[], *val, *k, false, *take));
                }
            }
            Op::SetIndex { val, x, slot, i, two, k, take } => {
                if !set_index_fast(vars, env, regs, *val, *x, *slot, i, *two, *take) {
                    flush!();
                    tryop!(set_index_slow(interp, code, env, regs, vars, *slot, &[i], *val, *k, false, *take));
                }
            }
            Op::CallBuiltin { dst, f, args, call, native } => {
                flush!();
                regs[*dst as usize] =
                    RVal::V(tryop!(call_builtin_op(interp, code, env, regs, *f, *args, *call, *native)));
            }
            Op::Builtin1 { f, dst, a, call } => {
                interp.visible = true;
                // R: eval.c:7894 (GETBUILTIN) — a traced builtin prints its
                // symbol, so the no-call fast path is off while one is traced.
                regs[*dst as usize] = match builtin1_untraced(*f, &regs[*a as usize]) {
                    Some(v) => {
                        regs[*a as usize] = RVal::I(0);
                        v
                    }
                    None => {
                        flush!();
                        if crate::builtins::debug::any_prim_traced() {
                            trace_builtin1(interp, &code.consts[*call as usize]);
                        }
                        let x = std::mem::replace(&mut regs[*a as usize], RVal::V(Value::Null));
                        RVal::V(tryop!(call_builtin(interp, &code.consts[*call as usize], vec![x], env)))
                    }
                };
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
                        tryop!(math1_slow(interp, *f, x, &code.consts[*call as usize], env))
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
                        let r = andor_operand(interp, &v, *and, *is_y, &code.consts[*call as usize]);
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
                    if x1 == NA_LOGICAL { if x2 == NA_LOGICAL || x2 != 0 { NA_LOGICAL } else { x2 } } else { x2 }
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
                        Some(Num::D(d)) if (0.0..2147483647.0).contains(&d) => Some((1, 1, d as u64)),
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
                let fast_fun = if pending_f || site.inline_args.is_none() {
                    None
                } else {
                    call_site_fun_fast(interp, *f, site, env)
                };
                // A call that needs no frame: native code, or the body of a
                // pure closure inline (it cannot observe the frame, so
                // nothing needs flushing, unless it reads variables that
                // pending writes may affect).
                if let (Some(Value::Closure(clo)), Some(leaves)) = (&fast_fun, &site.inline_args)
                    && let Some(v) = if matches!(clo.inline.get(), Some(Some(i)) if !i.has_calls) {
                        // (A body that small is not worth entering native
                        // code for: see `jit::closure::new_entry`.)
                        inline_site_call(interp, clo, leaves, vars, env, regs)
                    } else if inlineable(clo) || site.no_native.get() != Rc::as_ptr(&clo.formals) as *const Arg as usize
                    {
                        frameless_call(interp, code, site, clo, vars, env, regs)
                    } else {
                        None
                    }
                {
                    regs[*dst as usize] = if site.effect { DEAD } else { v };
                    continue;
                }
                flush!();
                let v = tryop!(call_op(interp, site, *f, fast_fun, env, vars, regs));
                // (In effect position the value is dead: #357.)
                regs[*dst as usize] = if site.effect { DEAD } else { v };
            }
            Op::ForSeqBy { k, seq, call } => {
                let r = match seq_by_args(&code.seq_by[*k as usize], vars, env, regs) {
                    Some([from, to, by]) => {
                        flush!();
                        seq_by_range(interp, env, from, to, by)
                    }
                    None => None,
                };
                regs[*seq as usize] = match r {
                    Some((start, step, n)) => RVal::Range { start, step, n },
                    None => {
                        flush!();
                        RVal::V(tryop!(interp.eval(&code.consts[*call as usize], env)))
                    }
                };
            }
            Op::ForSeqLength { len, seq, call } => {
                let n = match leaf_num(vars, env, regs, len) {
                    Some(n) => {
                        flush!();
                        seq_length_range(interp, env, n)
                    }
                    None => None,
                };
                regs[*seq as usize] = match n {
                    Some(n) => RVal::Range { start: 1, step: 1, n },
                    None => {
                        flush!();
                        RVal::V(tryop!(interp.eval(&code.consts[*call as usize], env)))
                    }
                };
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
            Op::CallSpecial { dst, f, k } => {
                flush!();
                let Value::Builtin(b) = &code.consts[*f as usize] else { unreachable!() };
                if crate::builtins::debug::any_prim_traced() {
                    trace_builtin(interp, b, &code.consts[*k as usize]);
                }
                let v = tryop!(interp.eval_special(b, &code.consts[*k as usize], env));
                regs[*dst as usize] = RVal::V(v);
            }
            // No flush: making a closure runs no R code, and the closure's
            // first chance to read this frame is a call, which flushes.
            Op::MakeClosure { dst, t, k } => {
                if crate::builtins::debug::any_prim_traced()
                    && let Some(b) = crate::builtins::lookup_primitive("function")
                {
                    flush!();
                    trace_builtin(interp, b, &code.consts[*k as usize]);
                }
                let tpl = &code.closures[*t as usize];
                regs[*dst as usize] = RVal::V(crate::builtins::lang::make_closure(
                    interp,
                    tpl.formals.clone(),
                    tpl.body.clone(),
                    tpl.srcref.as_ref(),
                    env,
                ));
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
                let r = std::mem::replace(&mut regs[*src as usize], RVal::V(Value::Null));
                if code.is_body {
                    return Ok(body_result(interp, r, vars.unboxed));
                }
                let v = r.into_value();
                // R: context.c:findcontext — a `return` with no target
                // context is a catchable error at the jump site.
                if !interp.has_return_target(env) {
                    flush!();
                    return Err(Flow::error("no function to return from, jumping to top level"));
                }
                return Err(Flow::Return(v, env.clone()));
            }
            Op::LoopEnter { brk, nxt } => {
                // A loop compiled on an earlier entry runs natively at once,
                // and so does one whose short entries add up to the threshold.
                let enter = (pc - 1) as u32;
                let (go, heat) = if gs.jit_on { loop_enter_state(code, enter, *brk) } else { (false, false) };
                loops.push(LoopRec {
                    brk: *brk,
                    nxt: *nxt,
                    enter: if heat { enter | HEAT_BIT } else { enter },
                    count: 0,
                });
                if go {
                    jit_loop!();
                }
            }
            Op::LoopExit => {
                if let Some(rec) = loops.pop()
                    && rec.enter & HEAT_BIT != 0
                {
                    loop_heat(code, rec.enter & !HEAT_BIT, rec.count);
                }
            }
            Op::ForInit { seq, call } => {
                if matches!(regs[*seq as usize], RVal::Range { .. }) {
                    regs[*seq as usize + 1] = RVal::I(0);
                    continue;
                }
                // a deferred package ALTREP sequence is read now (crate::altrep)
                if crate::altrep::any_deferred() && matches!(regs[*seq as usize], RVal::V(Value::ExtPtr(_))) {
                    flush!();
                    let v = regs[*seq as usize].to_value();
                    let r = crate::altrep::forced(interp, v);
                    regs[*seq as usize] = RVal::V(tryop!(signal(interp, r)));
                }
                let v = match for_seq(&regs[*seq as usize], &code.consts[*call as usize]) {
                    Ok(v) => v,
                    Err(e) => {
                        flush!();
                        return signal(interp, Err(e));
                    }
                };
                // a package ALTREP sequence is read through its class
                // (crate::altrep): decided once per loop, marked by an index
                // counting from ALT_FOR (see Op::ForStep)
                // (an empty one takes the ordinary path: no element to read)
                let alt = !v.is_empty() && crate::altrep::lookup(&v).is_some();
                regs[*seq as usize] = RVal::V(v);
                regs[*seq as usize + 1] = RVal::I(if alt { ALT_FOR } else { 0 });
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
                let elt = match for_element(sv, i) {
                    Some(elt) => elt,
                    // past the end, or a package ALTREP sequence: its index
                    // counts from ALT_FOR (negative, so for_element has no
                    // element), and its class's Elt gives the element. The
                    // check is off the path of ordinary loops' iterations.
                    None => {
                        let RVal::I(raw) = regs[*idx as usize] else { continue };
                        // (a negative index of an ordinary sequence is a
                        // wrapped count past i32::MAX iterations: not ALTREP)
                        if raw >= 0 || (raw - ALT_FOR) as usize >= sv.len() || crate::altrep::lookup(sv).is_none() {
                            continue;
                        }
                        flush!();
                        let r = for_altrep_element(interp, env, *var, sv, (raw - ALT_FOR) as usize);
                        unbox(tryop!(signal(interp, r)))
                    }
                };
                // (an ALTREP index: ALT_FOR + k + 1, as `i` is `raw`)
                regs[*idx as usize] = RVal::I(i as i32 + 1);
                assign!(*var, *slot, elt);
                pc = *body as usize;
            }
        }
    }
    if !(code.is_body && frame_dies(interp, env)) {
        vars.flush(interp, env, &code.vars)?;
    }
    let r = std::mem::replace(&mut regs[code.result as usize], RVal::V(Value::Null));
    Ok(if code.is_body { body_result(interp, r, vars.unboxed) } else { r.into_value() })
}

/// Executed ops by kind (`--features op-stats`).
#[cfg(feature = "op-stats")]
mod op_stats {
    use super::Op;
    use std::collections::HashMap;
    use std::mem::Discriminant;

    struct Stats(HashMap<Discriminant<Op>, (String, u64)>);

    impl Drop for Stats {
        fn drop(&mut self) {
            let mut v: Vec<_> = self.0.values().collect();
            v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            let total: u64 = v.iter().map(|(_, n)| n).sum();
            eprintln!("op-stats: {} ops", total);
            for (name, n) in v {
                eprintln!("{:>12} {:5.1}% {}", n, 100.0 * *n as f64 / total as f64, name);
            }
        }
    }

    thread_local! {
        static STATS: std::cell::RefCell<Stats> = std::cell::RefCell::new(Stats(HashMap::new()));
    }

    pub fn count(op: &Op) {
        STATS.with(|s| {
            let mut s = s.borrow_mut();
            let e = s.0.entry(std::mem::discriminant(op)).or_insert_with(|| {
                let d = format!("{:?}", op);
                (d.split([' ', '{', '(']).next().unwrap_or("").to_string(), 0)
            });
            e.1 += 1;
        });
    }
}

#[cfg(test)]
mod arithmetic_tests {
    use super::*;
    use std::io::Read;

    /// Replay the recorded GNU R 4.5.0 arithmetic bits through compiled
    /// expressions, including the VM's scalar and simple-body paths.
    #[test]
    fn gnu_r_arithmetic_oracle() {
        let raw = include_bytes!("builtins/arith_r_oracle.txt.gz");
        let mut oracle = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut oracle).unwrap();
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let env = Env::new(Some(interp.global.clone()));
        let x = Sym::new("x");
        let y = Sym::new("y");
        let mut codes = HashMap::new();
        for name in ["+", "-", "*", "/", "^", "%%", "%/%"] {
            let expr = Value::lang(Value::Sym(Sym::new(name)), vec![Arg::pos(Value::Sym(x)), Arg::pos(Value::Sym(y))]);
            codes.insert(name, compile_body(&expr));
        }
        let real = |s: &str| f64::from_bits(u64::from_str_radix(s, 16).unwrap());
        let shape = |v: Value, n: usize| match v {
            Value::Real(v) => Value::reals(vec![v.data[0]; n]),
            Value::Int(v) => Value::ints(vec![v.data[0]; n]),
            _ => unreachable!(),
        };
        let mut failures = Vec::new();
        let mut checked = 0;
        for line in oracle.lines() {
            let t: Vec<_> = line.split_whitespace().collect();
            let (op, sh, a, b) = if matches!(t[0], "+" | "-" | "*" | "/") {
                let (sh, order) = t[3].split_once('_').unwrap();
                let a = Value::int(if t[1] == "NAi" { NA_INTEGER } else { 3 });
                let b = Value::real(real(t[2]));
                let (a, b) = if order == "id" { (a, b) } else { (b, a) };
                (t[0], sh, a, b)
            } else {
                let op = match t[0] {
                    "add" => "+",
                    "sub" => "-",
                    "mul" => "*",
                    "div" => "/",
                    "pow" => "^",
                    "mod" => "%%",
                    "idiv" => "%/%",
                    _ => unreachable!(),
                };
                (op, t[1], Value::real(real(t[2])), Value::real(real(t[3])))
            };
            let (nx, ny, k) = match sh {
                "ss" => (1, 1, 0),
                "vv" => (2, 2, 1),
                "sv" => (1, 3, 1),
                "vs" => (3, 1, 1),
                "rc" => (4, 2, 2),
                _ => unreachable!(),
            };
            env.set_local(x, shape(a, nx));
            env.set_local(y, shape(b, ny));
            let code = &codes[op];
            let want = real(t[4]).to_bits();
            let result = run(&mut interp, code, &env).unwrap_or_else(|_| panic!("VM error: {line}"));
            let Value::Real(v) = result else { panic!("{line}: expected double") };
            if v.data[k].to_bits() != want {
                failures.push(format!("VM {line}: got {:016x}", v.data[k].to_bits()));
            }
            if sh == "ss"
                && let Some(result) = run_simple(&mut interp, code, &env)
            {
                let Value::Real(v) = result else { panic!("{line}: expected double") };
                if v.data[0].to_bits() != want {
                    failures.push(format!("simple {line}: got {:016x}", v.data[0].to_bits()));
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 9440);
        assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
    }
}

#[cfg(test)]
mod setter_tests {
    use super::*;

    #[test]
    fn gnu_r_setter_jit_transition() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let env = interp.global.clone();
        let definition = crate::lang::parse_to_values("f <- function(b, i, v) { b[[i]] <- v; b }").unwrap().remove(0);
        assert!(interp.eval(&definition, &env).is_ok());
        let call = crate::lang::parse_to_values("f(c(1, 2, b = c(x = 3)), c(\"b\", \"x\"), 10)").unwrap().remove(0);
        for line in include_str!("vm_setter_jit_oracle.txt").lines() {
            let observed = Rc::new(RefCell::new(None));
            let captured = observed.clone();
            let handler: crate::conditions::RustHandler = Rc::new(move |_, cond| {
                let Value::List(v) = cond else { panic!("error condition") };
                *captured.borrow_mut() = Some((v.data[0].clone(), v.data[1].clone()));
                Ok(())
            });
            assert!(interp.with_calling_error_handler(handler, |i| i.eval(&call, &env)).is_err());
            let (message, call) = observed.borrow().clone().expect("setter condition");
            let (want_call, want_message) = line.split_once('\t').unwrap();
            assert_eq!(crate::deparse::deparse(&call).join(" "), want_call);
            assert_eq!(crate::coerce::as_string(&message).as_deref(), Some(want_message));
        }
    }

    // Recorded by vm_setter_oracle.R with Debian GNU R 4.5.0-3, JIT=0.
    // The script explicitly compiles the VM cases. GNU R deliberately
    // reports different calls for interpreted and compiled assignments.
    #[test]
    fn gnu_r_setter_calls() {
        for line in include_str!("vm_setter_oracle.txt").lines() {
            let fields: Vec<_> = line.split('\t').collect();
            let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
            let env = Env::new(Some(interp.global.clone()));
            let mut m = Value::real(0.0);
            m.set_attr_raw(s::DIM, Value::ints(vec![1, 1]));
            env.set_local(Sym::new("m"), m);
            env.set_local(Sym::new("n"), Value::int(0));
            let expr = crate::lang::parse_to_values(fields[1]).unwrap().remove(0);
            let observed = Rc::new(RefCell::new(None));
            let captured = observed.clone();
            let handler: crate::conditions::RustHandler = Rc::new(move |_, cond| {
                let Value::List(v) = cond else { panic!("error condition") };
                *captured.borrow_mut() = Some(v.data[1].clone());
                Ok(())
            });
            let result = interp.with_calling_error_handler(handler, |interp| {
                if fields[0] == "vm" { run(interp, &compile_body(&expr), &env) } else { interp.eval(&expr, &env) }
            });
            let Err(Flow::Error(err)) = result else { panic!("expected error: {line}") };
            assert_eq!(err.message, fields[3], "{line}");
            if fields[1].starts_with("m[[") {
                assert_eq!(err.call.as_ref().and_then(Value::as_sym), Some(Sym::new("<current-expression>")));
            }
            let call = observed.borrow().clone().expect("setter condition call");
            assert_eq!(crate::deparse::deparse(&call).join(" "), fields[2], "{line}");
            if fields[1].contains("n <-") {
                assert_eq!(crate::coerce::as_integer(&env.get_local(Sym::new("n")).unwrap()), 1);
            }
        }
    }
}

#[cfg(test)]
mod review_regression_tests {
    use super::*;

    fn ready_loop(code: &Code) -> Rc<crate::jit::JitLoop> {
        let cache = code.jit.borrow();
        let mut ready = cache.values().filter_map(|slot| match slot {
            crate::jit::JitSlot::Ready(loop_) => Some(loop_.clone()),
            _ => None,
        });
        let loop_ = ready.next().expect("loop compiled natively");
        assert!(ready.next().is_none());
        loop_
    }

    #[test]
    fn complex_and_bitwise_loops_compile_natively() {
        for source in [
            "{ z <- 0i; for (i in 1:600) z <- z + (1 + 2i); z }",
            "{ x <- 0L; for (i in 1:600) x <- .Internal(bitwiseXor(x, i)); x }",
            "{ x <- 0L; for (i in 1:600) x <- .Internal(bitwiseNot(x)); x }",
        ] {
            let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
            let env = Env::new(Some(interp.global.clone()));
            let expr = crate::lang::parse_to_values(source).unwrap().remove(0);
            let code = compile_body(&expr);
            let result = run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("VM loop failed: {source}"));
            // The computed values are VM semantics: they hold with or
            // without the JIT. `z` is `600+1200i`, the xor fold is over
            // `1:600`, and 600 `bitwiseNot` applications leave `0L`.
            if source.contains("z <-") {
                let Value::Cplx(z) = &result else { panic!("expected a complex: {source}") };
                assert_eq!((z.data[0].re, z.data[0].im), (600.0, 1200.0), "{source}");
            } else if source.contains("bitwiseXor") {
                let expected = (1..=600).fold(0i32, |acc, i| acc ^ i);
                assert_eq!(crate::coerce::as_integer(&result), expected, "{source}");
            } else {
                assert_eq!(crate::coerce::as_integer(&result), 0, "{source}");
            }
            if crate::jit::enabled() {
                let native = ready_loop(&code);
                assert!(native.leaves.is_empty(), "loop must execute every operation natively");
            }
        }
    }

    #[test]
    fn seq_length_native_entry_checks() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        crate::bootstrap::load_base(&mut interp).unwrap();
        let env = Env::new(Some(interp.global.clone()));
        let expr =
            crate::lang::parse_to_values("{ s <- 0; for (i in 1:600) for (j in seq(length = n)) s <- s + j; s }")
                .unwrap()
                .remove(0);
        let code = compile_body(&expr);
        let n = Sym::new("n");
        for (value, expected) in
            [(Value::int(3), 3600.0), (Value::real(2.5), 3600.0), (Value::lgl(true), 600.0), (Value::int(0), 0.0)]
        {
            env.set_local(n, value);
            let result = run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("seq loop failed"));
            assert_eq!(crate::coerce::as_real(&result), expected);
            if crate::jit::enabled() {
                assert!(code.jit.borrow().values().any(|s| matches!(s,
                    crate::jit::JitSlot::Ready(l) if !l.seq_lengths.is_empty() && l.leaves.is_empty())));
            }
        }
        // A method installed after warmup must invalidate the entry, even
        // though seq itself still resolves to base.
        let method = crate::lang::parse_to_values("function(...) 7L").unwrap().remove(0);
        let method = interp.eval(&method, &env).unwrap_or_else(|_| panic!("method"));
        env.set_local(Sym::new("seq.numeric"), method);
        env.set_local(n, Value::real(3.0));
        let result = run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("dispatch"));
        assert_eq!(crate::coerce::as_real(&result), 4200.0);
    }

    #[test]
    fn seq_length_compact_outer_range_compiles() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        crate::bootstrap::load_base(&mut interp).unwrap();
        let env = Env::new(Some(interp.global.clone()));
        let expr =
            crate::lang::parse_to_values("{ s <- 0; for (i in outer) for (j in seq(length = 3L)) s <- s + j; s }")
                .unwrap()
                .remove(0);
        let outer = crate::lang::parse_to_values("seq_len(600L)").unwrap().remove(0);
        let outer = interp.eval(&outer, &env).unwrap_or_else(|_| panic!("compact sequence"));
        env.set_local(Sym::new("outer"), outer);
        let code = compile_body(&expr);
        let result = run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("compact outer range"));
        assert_eq!(crate::coerce::as_real(&result), 3600.0);
        if crate::jit::enabled() {
            assert!(code.jit.borrow().values().any(|s| matches!(s,
                crate::jit::JitSlot::Ready(l) if !l.seq_lengths.is_empty() && l.leaves.is_empty())));
        }
    }

    #[test]
    fn seq_length_changing_input_stays_in_vm() {
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        crate::bootstrap::load_base(&mut interp).unwrap();
        let env = Env::new(Some(interp.global.clone()));
        let expr = crate::lang::parse_to_values(
            "{ s <- 0; n <- 0L; for (i in 1:600) { n <- 3L - n; for (j in seq(length = n)) s <- s + j }; s }",
        )
        .unwrap()
        .remove(0);
        let code = compile_body(&expr);
        let result = run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("changing length"));
        assert_eq!(crate::coerce::as_real(&result), 1800.0);
        assert!(!code.jit.borrow().values().any(|s| matches!(s,
            crate::jit::JitSlot::Ready(l) if !l.seq_lengths.is_empty())));
    }

    #[test]
    fn alternating_na_slots_converge_to_one_compilation() {
        if !crate::jit::enabled() {
            return;
        }
        let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
        let env = Env::new(Some(interp.global.clone()));
        let expr = crate::lang::parse_to_values(
            "{ s <- 0; for (i in 1:600) { u <- a + 1L; v <- b + 1L; s <- s + u + v }; s }",
        )
        .unwrap()
        .remove(0);
        let code = compile_body(&expr);
        let a = Sym::new("a");
        let b = Sym::new("b");
        env.set_local(a, Value::int(1));
        env.set_local(b, Value::int(2));
        assert_eq!(
            crate::coerce::as_real(&run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("VM loop failed"))),
            3000.0
        );
        let initial = ready_loop(&code);
        env.set_local(a, Value::int(NA_INTEGER));
        assert!(crate::value::is_na_real(crate::coerce::as_real(
            &run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("VM loop failed"))
        )));
        let relaxed = ready_loop(&code);
        assert!(!Rc::ptr_eq(&initial, &relaxed));
        for (av, bv) in [(1, 2), (NA_INTEGER, 2), (1, NA_INTEGER), (1, 2), (NA_INTEGER, 2), (1, NA_INTEGER)] {
            env.set_local(a, Value::int(av));
            env.set_local(b, Value::int(bv));
            let value =
                crate::coerce::as_real(&run(&mut interp, &code, &env).unwrap_or_else(|_| panic!("VM loop failed")));
            assert!(if av == NA_INTEGER || bv == NA_INTEGER {
                crate::value::is_na_real(value)
            } else {
                value == 3000.0
            });
            assert!(Rc::ptr_eq(&relaxed, &ready_loop(&code)), "alternating slots must not recompile");
        }
    }
}
