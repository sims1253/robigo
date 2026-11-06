//! Loops summarized in closed form.
//!
//! A native loop runs with nothing observing it: no R code, no warnings, no
//! interrupts; what it leaves behind is the variables' values at its exit
//! (or, where it leaves early, at that op, for the VM to continue). So a
//! loop whose iterations only count and copy (`for (i in 1:n) x <- i`,
//! `while (i < n) { x <- i; i <- i + 1 }`) can be replaced by its final
//! state, computed in constant time, whenever it is certain that the native
//! code would have run to the end without leaving.
//!
//! The analysis executes one iteration symbolically. Every variable the
//! loop assigns must then be
//! - an induction variable, `x <- x + c` or `x <- x - c` for an integer
//!   constant `c`: after `T` iterations `x0 + T*c`, exactly, as long as
//!   every value on the way is an integer that a double represents exactly
//!   (or an integer without overflow);
//! - or a function of induction variables, the `for` element and variables
//!   the loop does not assign, made of operations that never leave native
//!   code: its final value is that function at the last iteration.
//!
//! The trip count `T` is the `for` sequence's remaining length, or for a
//! `while` loop whose condition compares an induction variable with a
//! loop-invariant bound, the first `k` at which the comparison fails. Where
//! an assumption does not hold for the values at entry (an NA, a bound past
//! 2^53, a loop that would not end, an empty `for` sequence), the summary
//! declines and the native loop runs.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{Block, InstBuilder, Value as CValue, types};
use cranelift_frontend::Variable;

use super::{Inl, JType, SlotInfo};
use crate::builtins::arith::{ArithOp, RelOp, r_pow};
use crate::compile::{Code, Leaf, Op, Reg, Slot};
use crate::value::{NA_INTEGER, NA_LOGICAL, NA_REAL};

/// A symbolic value: an index into [`Summary::nodes`].
type N = u32;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Node {
    /// A variable's value at the start of the iteration.
    Entry(Slot),
    /// The `for` element of the iteration (integer).
    Elem,
    D(f64),
    I(i32),
    L(i32),
    Arith(ArithOp, N, N),
    Rel(RelOp, N, N),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Num {
    D(f64),
    I(i32),
    L(i32),
}

impl Num {
    fn bits(self) -> u64 {
        match self {
            Num::D(x) => x.to_bits(),
            Num::I(x) | Num::L(x) => x as i64 as u64,
        }
    }

    fn f64(self) -> f64 {
        match self {
            Num::D(x) => x,
            Num::I(x) | Num::L(x) => {
                if x == NA_INTEGER {
                    NA_REAL
                } else {
                    x as f64
                }
            }
        }
    }

    fn is_na(self) -> bool {
        match self {
            Num::D(x) => x.is_nan(),
            Num::I(x) | Num::L(x) => x == NA_INTEGER,
        }
    }
}

/// A variable the loop assigns that is not an induction variable (those
/// end as `x0 + T * step`): it ends as the value of `node` at the last
/// iteration (unchanged if there was none); `flag` is set when it was
/// assigned.
#[derive(Debug)]
struct Fate {
    idx: usize,
    node: N,
    flag: Option<usize>,
}

#[derive(Debug)]
enum Trip {
    /// A `for` loop: the sequence registers' and the index's places.
    For { seq: usize, idx: usize },
    /// `while (iv OP bound)`, with the induction variable on the left
    /// (`flip`: it was on the right).
    While { op: RelOp, iv: Slot, bound: N, flip: bool },
}

/// An induction variable: its place in the value buffer, type and step.
#[derive(Debug)]
struct Iv {
    slot: Slot,
    idx: usize,
    ty: JType,
    step: i64,
}

/// At most this many induction variables (their values at the last
/// iteration live on the stack).
const MAX_IVS: usize = 8;
const NO_IV: u8 = u8::MAX;

/// A loop's closed form (see the module documentation).
#[derive(Debug)]
pub struct Summary {
    nodes: Vec<Node>,
    /// Per node: for `Entry`, the variable's place in the value buffer and
    /// its type, and which induction variable it is (`NO_IV`: none).
    place: Vec<(usize, JType, u8)>,
    trip: Trip,
    fates: Vec<Fate>,
    ivs: Vec<Iv>,
    /// The induction variable of a `while` condition.
    cond_iv: usize,
    /// Per node: its type.
    tys: Vec<JType>,
}

const EXACT: i128 = 1 << 53;

struct Builder<'a> {
    nodes: Vec<Node>,
    slot_ty: &'a HashMap<Slot, JType>,
    inl: &'a Inl<'a>,
}

impl Builder<'_> {
    fn add(&mut self, n: Node) -> N {
        if let Some(k) = self.nodes.iter().position(|m| *m == n) {
            return k as N;
        }
        self.nodes.push(n);
        (self.nodes.len() - 1) as N
    }

    fn ty(&self, n: N) -> Option<JType> {
        Some(match self.nodes[n as usize] {
            Node::Entry(s) => *self.slot_ty.get(&s)?,
            Node::Elem | Node::I(_) => JType::I,
            Node::D(_) => JType::D,
            Node::L(_) => JType::L,
            Node::Rel(..) => JType::L,
            Node::Arith(op, a, b) => {
                let (x, y) = (self.ty(a)?, self.ty(b)?);
                if x == JType::D || y == JType::D {
                    JType::D
                } else if op == ArithOp::Div {
                    JType::D
                } else {
                    JType::I
                }
            }
        })
    }

    fn leaf(&mut self, l: &Leaf, slots: &HashMap<Slot, N>, regs: &HashMap<Reg, N>) -> Option<N> {
        Some(match l {
            Leaf::D(x) => self.add(Node::D(*x)),
            Leaf::I(x) => self.add(Node::I(*x)),
            Leaf::Var(_, s) => self.var(*s, slots),
            Leaf::R(r) => *regs.get(r)?,
        })
    }

    fn var(&mut self, s: Slot, slots: &HashMap<Slot, N>) -> N {
        match slots.get(&s) {
            Some(n) => *n,
            None => self.add(Node::Entry(s)),
        }
    }

    /// Execute `ops[from..to]` (straight-line) symbolically.
    fn run(
        &mut self,
        ops: &[Op],
        from: usize,
        to: usize,
        slots: &mut HashMap<Slot, N>,
        regs: &mut HashMap<Reg, N>,
    ) -> Option<()> {
        for op in &ops[from..to] {
            match op {
                Op::Guard { .. } | Op::Visible { .. } => {}
                Op::ConstD { dst, x } => {
                    let n = self.add(Node::D(*x));
                    regs.insert(*dst, n);
                }
                Op::ConstI { dst, x } => {
                    let n = self.add(Node::I(*x));
                    regs.insert(*dst, n);
                }
                Op::ConstL { dst, x } => {
                    let n = self.add(Node::L(*x));
                    regs.insert(*dst, n);
                }
                Op::GetVar { dst, slot, .. } => {
                    let n = self.var(*slot, slots);
                    regs.insert(*dst, n);
                }
                Op::SetVar { slot, .. } if self.inl.is_dead(*slot) => {}
                Op::SetVar { src, slot, .. } => {
                    let n = *regs.get(src)?;
                    slots.insert(*slot, n);
                }
                Op::Arith { op, dst, a, b, .. } => {
                    let n = self.add(Node::Arith(*op, *regs.get(a)?, *regs.get(b)?));
                    regs.insert(*dst, n);
                }
                Op::ArithL { op, dst, a, b, .. } => {
                    let (x, y) = (self.leaf(a, slots, regs)?, self.leaf(b, slots, regs)?);
                    let n = self.add(Node::Arith(*op, x, y));
                    regs.insert(*dst, n);
                }
                Op::Rel { op, dst, a, b, .. } => {
                    let n = self.add(Node::Rel(*op, *regs.get(a)?, *regs.get(b)?));
                    regs.insert(*dst, n);
                }
                Op::RelL { op, dst, a, b, .. } => {
                    let (x, y) = (self.leaf(a, slots, regs)?, self.leaf(b, slots, regs)?);
                    let n = self.add(Node::Rel(*op, x, y));
                    regs.insert(*dst, n);
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Whether native code evaluates `n` without ever leaving, whatever the
    /// operands (an integer `+` can overflow, which leaves).
    fn total(&self, n: N) -> bool {
        match self.nodes[n as usize] {
            Node::Entry(_) | Node::Elem | Node::D(_) | Node::I(_) | Node::L(_) => true,
            Node::Rel(_, a, b) => self.total(a) && self.total(b),
            Node::Arith(op, a, b) => {
                let ok = match self.ty(n) {
                    Some(JType::D) => {
                        matches!(op, ArithOp::Plus | ArithOp::Minus | ArithOp::Times | ArithOp::Div | ArithOp::Pow)
                    }
                    _ => false,
                };
                ok && self.total(a) && self.total(b)
            }
        }
    }

    /// The variables `n` reads, and whether it reads the `for` element.
    fn reads(&self, n: N, out: &mut Vec<Slot>, elem: &mut bool) {
        match self.nodes[n as usize] {
            Node::Entry(s) => {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            Node::Elem => *elem = true,
            Node::D(_) | Node::I(_) | Node::L(_) => {}
            Node::Arith(_, a, b) | Node::Rel(_, a, b) => {
                self.reads(a, out, elem);
                self.reads(b, out, elem);
            }
        }
    }

    /// `x <- x + c` / `x <- x - c` / `x <- c + x`: the step.
    fn step(&self, s: Slot, n: N, ty: JType) -> Option<i64> {
        let Node::Arith(op, a, b) = self.nodes[n as usize] else { return None };
        if self.ty(n)? != ty {
            return None;
        }
        let (k, neg) = match (op, self.nodes[a as usize], self.nodes[b as usize]) {
            (ArithOp::Plus, Node::Entry(x), k) | (ArithOp::Plus, k, Node::Entry(x)) if x == s => (k, false),
            (ArithOp::Minus, Node::Entry(x), k) if x == s => (k, true),
            _ => return None,
        };
        let c = match (k, ty) {
            (Node::I(c), _) if c != NA_INTEGER => c as i64,
            (Node::D(c), JType::D) if c.fract() == 0.0 && c.abs() <= (1u64 << 31) as f64 => c as i64,
            _ => return None,
        };
        if c == 0 {
            return None;
        }
        Some(if neg { -c } else { c })
    }
}

/// The closed form of the loop `ops[start..end]` (`end` its `LoopExit`),
/// whose variables have the types `slot_ty` and the places `slots`, if it
/// has one. `regs` gives the places of registers carried in.
pub fn analyze(
    code: &Code,
    start: usize,
    end: usize,
    slots: &[SlotInfo],
    slot_ty: &HashMap<Slot, JType>,
    reg_idx: &HashMap<Reg, usize>,
    live: &[(Reg, JType)],
    inl: &Inl,
) -> Option<Summary> {
    let ops = &code.ops[..];
    // Shape: `Jump nxt; body...; <test at nxt>`, the test being a
    // `ForStep` or condition ops ending in `JumpIf` back to the body.
    let Op::Jump { target } = ops[start] else { return None };
    let nxt = target as usize;
    let body = start + 1;
    if nxt < body || nxt >= end {
        return None;
    }
    let mut b = Builder { nodes: Vec::new(), slot_ty, inl };
    let mut sv: HashMap<Slot, N> = HashMap::new();
    let mut rv: HashMap<Reg, N> = HashMap::new();
    let (trip, for_var) = match ops[end - 1] {
        Op::ForStep { seq, idx, slot, body: tb, .. } if end - 1 == nxt && tb as usize == body => {
            let has = |r: Reg, t: JType| live.iter().any(|(r2, t2)| *r2 == r && *t2 == t);
            if !has(seq, JType::Range) || !has(idx, JType::I) {
                return None;
            }
            let elem = b.add(Node::Elem);
            sv.insert(slot, elem);
            (Some(Trip::For { seq: *reg_idx.get(&seq)?, idx: *reg_idx.get(&idx)? }), Some(slot))
        }
        Op::JumpIf { cond, target: tb, .. } if tb as usize == body => {
            // The condition, as a function of the variables before an
            // iteration (it must not assign any).
            let mut cs = HashMap::new();
            let mut cr = HashMap::new();
            b.run(ops, nxt, end - 1, &mut cs, &mut cr)?;
            if !cs.is_empty() {
                return None;
            }
            let c = *cr.get(&cond)?;
            let Node::Rel(op, x, y) = b.nodes[c as usize] else { return None };
            // (Which side is the induction variable is settled below.)
            match (b.nodes[x as usize], b.nodes[y as usize]) {
                (Node::Entry(iv), _) => (Some(Trip::While { op, iv, bound: y, flip: false }), None),
                _ => match b.nodes[y as usize] {
                    Node::Entry(iv) => (Some(Trip::While { op, iv, bound: x, flip: true }), None),
                    _ => return None,
                },
            }
        }
        _ => return None,
    };
    let mut trip = trip?;
    b.run(ops, body, nxt, &mut sv, &mut rv)?;

    // Classify what the loop assigns.
    let mut ivs: HashMap<Slot, i64> = HashMap::new();
    for (s, n) in &sv {
        if Some(*s) == for_var {
            continue;
        }
        if let Some(c) = b.step(*s, *n, *slot_ty.get(s)?) {
            ivs.insert(*s, c);
        }
    }
    let info = |s: Slot| slots.iter().find(|x| x.slot == s);
    let mut fates = Vec::new();
    let mut assigned: Vec<Slot> = sv.keys().copied().collect();
    assigned.sort();
    for s in assigned {
        let si = info(s)?;
        if !si.ty.is_scalar() {
            return None;
        }
        let n = sv[&s];
        if ivs.contains_key(&s) {
            if !si.needs_entry {
                return None;
            }
            continue;
        }
        if b.nodes[n as usize] == Node::Entry(s) {
            // `x <- x`: unchanged.
            continue;
        }
        if !b.total(n) || b.ty(n)? != si.ty {
            return None;
        }
        let mut rs = Vec::new();
        let mut elem = false;
        b.reads(n, &mut rs, &mut elem);
        if rs.iter().any(|r| sv.contains_key(r) && !ivs.contains_key(r)) {
            return None;
        }
        fates.push(Fate { idx: si.idx, node: n, flag: si.flag });
    }
    if let Trip::While { op, iv, bound, flip } = &mut trip {
        if !ivs.contains_key(iv) {
            // The induction variable is on the other side, or there is none.
            let Node::Entry(other) = b.nodes[*bound as usize] else { return None };
            if !*flip && ivs.contains_key(&other) {
                let old = *iv;
                *iv = other;
                *bound = b.add(Node::Entry(old));
                *flip = true;
            } else {
                return None;
            }
        }
        if *flip {
            *op = match *op {
                RelOp::Lt => RelOp::Gt,
                RelOp::Le => RelOp::Ge,
                RelOp::Gt => RelOp::Lt,
                RelOp::Ge => RelOp::Le,
                o => o,
            };
            *flip = false;
        }
        // The bound is invariant and evaluates without leaving.
        let mut rs = Vec::new();
        let mut elem = false;
        b.reads(*bound, &mut rs, &mut elem);
        if elem || rs.iter().any(|r| sv.contains_key(r)) || !b.total(*bound) {
            return None;
        }
    }
    // Every operation of an iteration, not only those whose values last
    // (`x <- i + 2147483600L; x <- i` overflows, which leaves native code),
    // never leaves, but the induction steps, whose values `run` bounds.
    for (k, n) in b.nodes.iter().enumerate() {
        let k = k as N;
        if matches!(n, Node::Arith(..)) && !b.total(k) && !ivs.keys().any(|s| sv.get(s) == Some(&k)) {
            return None;
        }
    }
    // Everything read has its value at entry.
    let mut iv_list: Vec<Iv> = Vec::new();
    let mut ks: Vec<(&Slot, &i64)> = ivs.iter().collect();
    ks.sort();
    for (s, c) in ks {
        let si = info(*s)?;
        iv_list.push(Iv { slot: *s, idx: si.idx, ty: si.ty, step: *c });
    }
    if iv_list.len() > MAX_IVS {
        return None;
    }
    let mut place = Vec::with_capacity(b.nodes.len());
    for n in &b.nodes {
        place.push(match n {
            Node::Entry(s) => {
                let si = info(*s)?;
                if !si.ty.is_scalar() || !si.needs_entry {
                    return None;
                }
                let k = iv_list.iter().position(|v| v.slot == *s).map_or(NO_IV, |k| k as u8);
                (si.idx, si.ty, k)
            }
            _ => (0, JType::D, NO_IV),
        });
    }
    let cond_iv = match &trip {
        Trip::While { iv, .. } => iv_list.iter().position(|v| v.slot == *iv)?,
        Trip::For { .. } => 0,
    };
    let tys = (0..b.nodes.len()).map(|n| b.ty(n as N)).collect::<Option<Vec<_>>>()?;
    Some(Summary { nodes: b.nodes, place, trip, fates, ivs: iv_list, cond_iv, tys })
}

fn ceil_div(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0);
    if a >= 0 { (a + b - 1) / b } else { -((-a) / b) }
}

fn get(vals: &[u64], idx: usize, ty: JType) -> Num {
    let b = vals[idx];
    match ty {
        JType::D => Num::D(f64::from_bits(b)),
        JType::I => Num::I(b as i32),
        _ => Num::L(b as i32),
    }
}

/// An induction variable's value, as an exact integer.
fn int_of(x: Num) -> Option<i128> {
    match x {
        Num::D(d) if d.fract() == 0.0 && (d.abs() as i128) <= EXACT => Some(d as i128),
        Num::I(i) if i != NA_INTEGER => Some(i as i128),
        _ => None,
    }
}

fn compare(op: RelOp, x: f64, y: f64) -> bool {
    match op {
        RelOp::Eq => x == y,
        RelOp::Ne => x != y,
        RelOp::Lt => x < y,
        RelOp::Le => x <= y,
        RelOp::Gt => x > y,
        RelOp::Ge => x >= y,
    }
}

impl Summary {
    /// Node `n`'s value, with the variables of `vals`, the induction
    /// variables' values `ivs` if given, and the `for` element `elem`.
    fn eval(&self, n: N, vals: &[u64], ivs: Option<&[Num]>, elem: i32) -> Num {
        match self.nodes[n as usize] {
            Node::Entry(_) => {
                let (idx, ty, k) = self.place[n as usize];
                match ivs {
                    Some(v) if k != NO_IV => v[k as usize],
                    _ => get(vals, idx, ty),
                }
            }
            Node::Elem => Num::I(elem),
            Node::D(x) => Num::D(x),
            Node::I(x) => Num::I(x),
            Node::L(x) => Num::L(x),
            Node::Arith(op, a, b) => {
                let (x, y) = (self.eval(a, vals, ivs, elem), self.eval(b, vals, ivs, elem));
                if matches!(x, Num::D(_)) || matches!(y, Num::D(_)) || op == ArithOp::Div {
                    // (Integer division of NA: NA_REAL, as `to_f64` gives.)
                    let (x, y) = (x.f64(), y.f64());
                    Num::D(match op {
                        ArithOp::Plus => x + y,
                        ArithOp::Minus => x - y,
                        ArithOp::Times => x * y,
                        ArithOp::Div => x / y,
                        ArithOp::Pow => r_pow(x, y),
                        // (Not admitted by `total`.)
                        _ => NA_REAL,
                    })
                } else {
                    // (Integer arithmetic is only an induction step, which
                    // `run` computes itself.)
                    Num::I(NA_INTEGER)
                }
            }
            Node::Rel(op, a, b) => {
                let (x, y) = (self.eval(a, vals, ivs, elem), self.eval(b, vals, ivs, elem));
                if x.is_na() || y.is_na() {
                    return Num::L(NA_LOGICAL);
                }
                let r = match (x, y) {
                    (Num::I(x) | Num::L(x), Num::I(y) | Num::L(y)) => compare(op, x as f64, y as f64),
                    _ => compare(op, x.f64(), y.f64()),
                };
                Num::L(r as i32)
            }
        }
    }

    /// The trip count from the values at entry, and the first `for`
    /// element's index and the sequence (start, step); `None` if the native
    /// code would leave or the closed form is not exact.
    fn trip_count(&self, vals: &[u64]) -> Option<(i128, i64, i64, i64)> {
        match self.trip {
            Trip::For { seq, idx } => {
                let (start, step, n) = (vals[seq] as i64, vals[seq + 1] as i64, vals[seq + 2] as i64);
                let i0 = vals[idx] as i32 as i64;
                if i0 >= n {
                    // An empty sequence: the VM sets the variable.
                    return if i0 == 0 { None } else { Some((0, 0, 0, 0)) };
                }
                Some(((n - i0) as i128, i0, start, step))
            }
            Trip::While { op, bound, .. } => {
                let iv = &self.ivs[self.cond_iv];
                let x0 = int_of(get(vals, iv.idx, iv.ty))?;
                let c = iv.step as i128;
                let bv = self.eval(bound, vals, None, 0);
                if bv.is_na() {
                    return None;
                }
                let bf = bv.f64();
                // (Exact while every value is: checked by `run`.)
                let cont = |k: i128| compare(op, (x0 + k * c) as f64, bf);
                let t = if !cont(0) {
                    0
                } else {
                    if !bf.is_finite() || bf.abs() > EXACT as f64 {
                        return None;
                    }
                    match op {
                        RelOp::Lt if c > 0 => ceil_div(bf.ceil() as i128 - x0, c),
                        RelOp::Le if c > 0 => ceil_div(bf.floor() as i128 + 1 - x0, c),
                        RelOp::Gt if c < 0 => ceil_div(x0 - bf.floor() as i128, -c),
                        RelOp::Ge if c < 0 => ceil_div(x0 - (bf.ceil() as i128 - 1), -c),
                        RelOp::Ne if bf.fract() == 0.0 && (bf as i128 - x0) % c == 0 && (bf as i128 - x0) / c > 0 => {
                            (bf as i128 - x0) / c
                        }
                        // It would not end, or not before overflowing.
                        _ => return None,
                    }
                };
                debug_assert!(t == 0 || (!cont(t) && cont(t - 1)));
                Some((t, 0, 0, 0))
            }
        }
    }

    /// Run the loop on the value buffer `vals` (laid out as for its native
    /// code, with the values at entry), leaving it as the native code would
    /// at the loop's exit. `false`, with `vals` untouched, if the native code
    /// would leave the loop early or the closed form is not exact for these
    /// values.
    pub fn run(&self, vals: &mut [u64]) -> bool {
        let Some((t, i0, start, step)) = self.trip_count(vals) else { return false };
        // Every induction variable stays exact (and an integer one without
        // overflow) all the way: they are monotonic. Their values at the
        // start of the last iteration and at the end:
        let mut last = [Num::I(0); MAX_IVS];
        let mut fin = [0u64; MAX_IVS];
        for (k, iv) in self.ivs.iter().enumerate() {
            let x = get(vals, iv.idx, iv.ty);
            let Some(x0) = int_of(x) else { return false };
            let xt = x0 + t * iv.step as i128;
            let num = |v: i128| if iv.ty == JType::D { Num::D(v as f64) } else { Num::I(v as i32) };
            let ok = match iv.ty {
                JType::D => xt.abs() <= EXACT,
                _ => xt > i32::MIN as i128 && xt <= i32::MAX as i128,
            };
            if !ok {
                return false;
            }
            // (At the first iteration the value itself, which may be -0;
            // after that a sum, whose zero is +0.)
            last[k] = if t <= 1 { x } else { num(x0 + (t - 1) * iv.step as i128) };
            fin[k] = if t == 0 { vals[iv.idx] } else { num(xt).bits() };
        }
        if t == 0 {
            return true;
        }
        let elem = (start as i128 + (i0 as i128 + t - 1) * step as i128) as i32;
        // Values at the last iteration first (they read the entry values),
        // then the induction variables.
        let ivs = &last[..self.ivs.len()];
        for f in &self.fates {
            let v = self.eval(f.node, vals, Some(ivs), elem);
            vals[f.idx] = v.bits();
            if let Some(fi) = f.flag {
                vals[fi] = 1;
            }
        }
        for (k, iv) in self.ivs.iter().enumerate() {
            vals[iv.idx] = fin[k];
        }
        true
    }
}

impl Summary {
    /// Whether [`Summary::emit_for`] applies: a `for` loop.
    pub(super) fn is_for(&self) -> bool {
        matches!(self.trip, Trip::For { .. })
    }

    /// The closed form of a `for` loop as native code, computing what
    /// [`Summary::run`] does: `vars[idx]` is the variable at value-buffer
    /// place `idx`, `range` the sequence's (start, step, length) and `index`
    /// its index variable. Continues at `exit` with the variables set, or at
    /// `fallback`, with nothing changed, where `run` would decline.
    pub(super) fn emit_for(
        &self,
        g: &mut super::Gen,
        vars: &[Variable],
        (vs, vt, vn): (Variable, Variable, Variable),
        index: Variable,
        exit: Block,
        fallback: Block,
    ) {
        let start = g.b.use_var(vs);
        let step = g.b.use_var(vt);
        let n = g.b.use_var(vn);
        let i0 = g.b.use_var(index);
        let i0 = g.b.ins().sextend(types::I64, i0);
        let done = g.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, i0, n);
        let at_end = g.b.create_block();
        let run = g.b.create_block();
        g.b.ins().brif(done, at_end, &[], run, &[]);
        // No iteration left: nothing changes, but an empty sequence, whose
        // variable the VM sets.
        g.b.switch_to_block(at_end);
        let empty = g.b.ins().icmp_imm_s(IntCC::Equal, i0, 0);
        g.b.ins().brif(empty, fallback, &[], exit, &[]);
        g.b.switch_to_block(run);
        let t = g.b.ins().isub(n, i0);
        // The induction variables: exact all the way, their values at the
        // last iteration and at the end.
        let mut ok = g.b.ins().iconst(types::I8, 1);
        let mut last = Vec::new();
        let mut fin = Vec::new();
        for iv in &self.ivs {
            let x = g.b.use_var(vars[iv.idx]);
            // (`t * step` and the sums below cannot overflow: beyond this
            // many iterations no value stays exact anyway.)
            let fits = g.b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, t, (1i64 << 54) / iv.step.abs());
            ok = g.b.ins().band(ok, fits);
            let tc = g.b.ins().imul_imm_s(t, iv.step);
            if iv.ty == JType::D {
                let xi = g.b.ins().fcvt_to_sint_sat(types::I64, x);
                let back = g.b.ins().fcvt_from_sint(types::F64, xi);
                let same = g.b.ins().fcmp(FloatCC::Equal, back, x);
                let xt = g.b.ins().iadd(xi, tc);
                for v in [xi, xt] {
                    let lo = g.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, v, -(EXACT as i64));
                    let hi = g.b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, v, EXACT as i64);
                    ok = g.b.ins().band(ok, lo);
                    ok = g.b.ins().band(ok, hi);
                }
                ok = g.b.ins().band(ok, same);
                // (At the first iteration the value itself, which may be -0;
                // after that a sum, whose zero is +0.)
                let xl = g.b.ins().iadd_imm_s(xt, -iv.step);
                let fl = g.b.ins().fcvt_from_sint(types::F64, xl);
                let first = g.b.ins().icmp_imm_s(IntCC::Equal, t, 1);
                last.push((g.b.ins().select(first, x, fl), JType::D));
                fin.push(g.b.ins().fcvt_from_sint(types::F64, xt));
            } else {
                let notna = g.b.ins().icmp_imm_s(IntCC::NotEqual, x, NA_INTEGER as i64);
                let xi = g.b.ins().sextend(types::I64, x);
                let xt = g.b.ins().iadd(xi, tc);
                let lo = g.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, xt, i32::MIN as i64);
                let hi = g.b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, xt, i32::MAX as i64);
                for c in [notna, lo, hi] {
                    ok = g.b.ins().band(ok, c);
                }
                let xl = g.b.ins().iadd_imm_s(xt, -iv.step);
                last.push((g.b.ins().ireduce(types::I32, xl), iv.ty));
                fin.push(g.b.ins().ireduce(types::I32, xt));
            }
        }
        let go = g.b.create_block();
        g.b.ins().brif(ok, go, &[], fallback, &[]);
        g.b.switch_to_block(go);
        // The `for` element of the last iteration.
        let k = g.b.ins().iadd(i0, t);
        let k = g.b.ins().iadd_imm_s(k, -1);
        let off = g.b.ins().imul(k, step);
        let e = g.b.ins().iadd(start, off);
        let elem = g.b.ins().ireduce(types::I32, e);
        let mut memo = vec![None; self.nodes.len()];
        let values: Vec<CValue> =
            self.fates.iter().map(|f| self.emit_node(g, f.node, vars, &last, elem, &mut memo).0).collect();
        for (f, v) in self.fates.iter().zip(values) {
            g.b.def_var(vars[f.idx], v);
        }
        for (iv, v) in self.ivs.iter().zip(fin) {
            g.b.def_var(vars[iv.idx], v);
        }
        g.b.ins().jump(exit, &[]);
    }

    /// Node `n` as native code (as [`Summary::eval`]).
    fn emit_node(
        &self,
        g: &mut super::Gen,
        n: N,
        vars: &[Variable],
        last: &[(CValue, JType)],
        elem: CValue,
        memo: &mut Vec<Option<(CValue, JType)>>,
    ) -> (CValue, JType) {
        if let Some(v) = memo[n as usize] {
            return v;
        }
        let v = match self.nodes[n as usize] {
            Node::Entry(_) => {
                let (idx, ty, k) = self.place[n as usize];
                if k != NO_IV { last[k as usize] } else { (g.b.use_var(vars[idx]), ty) }
            }
            Node::Elem => (elem, JType::I),
            Node::D(x) => (g.b.ins().f64const(x), JType::D),
            Node::I(x) => (g.b.ins().iconst(types::I32, x as i64), JType::I),
            Node::L(x) => (g.b.ins().iconst(types::I32, x as i64), JType::L),
            Node::Arith(op, a, b) => {
                let x = self.emit_node(g, a, vars, last, elem, memo);
                let y = self.emit_node(g, b, vars, last, elem, memo);
                // (Only double results are admitted, see `total`.)
                let (x, y) = (g.to_f64(x.0, x.1), g.to_f64(y.0, y.1));
                let r = match op {
                    ArithOp::Plus => g.b.ins().fadd(x, y),
                    ArithOp::Minus => g.b.ins().fsub(x, y),
                    ArithOp::Times => g.b.ins().fmul(x, y),
                    ArithOp::Div => g.b.ins().fdiv(x, y),
                    _ => g.call("robigo_pow", &[x, y]),
                };
                (r, JType::D)
            }
            Node::Rel(op, a, b) => {
                let x = self.emit_node(g, a, vars, last, elem, memo);
                let y = self.emit_node(g, b, vars, last, elem, memo);
                (g.rel(op, x, y), JType::L)
            }
        };
        debug_assert_eq!(v.1, self.tys[n as usize]);
        memo[n as usize] = Some(v);
        v
    }
}
