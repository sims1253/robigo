//! A bytecode compiler for closure bodies and loops.
//!
//! The compiler specializes the constructs that dominate tight R code —
//! `{`, `(`, `if`, `for`, `while`, `repeat`, `break`, `next`, `return`,
//! assignment to a symbol, variable reads, arithmetic, comparison and `!` —
//! and leaves everything else to the AST interpreter through `Eval`
//! instructions, so any R expression can be compiled.
//!
//! Semantics are preserved by runtime guards: every specialized construct
//! first checks that its head symbol (`+`, `if`, `<-`, ...) is not bound
//! outside the base environment ([`crate::env::shadow_count`]); if it is,
//! the original call is evaluated by the interpreter instead. Arithmetic
//! and comparison only take the fast path for attribute-free scalars and
//! otherwise call the regular builtin, which handles recycling,
//! attributes, dispatch and warnings.

use rr_syntax::Span;

use crate::builtins::arith::{ArithOp, RelOp};
use crate::sym::{s, Sym};
use crate::value::*;

pub type Reg = u16;

#[derive(Clone, Debug)]
pub enum Op {
    Const { dst: Reg, k: u32 },
    GetVar { dst: Reg, sym: Sym },
    /// Assign to a symbol in the current frame (`<-`/`=`).
    SetVar { src: Reg, sym: Sym },
    Arith { op: ArithOp, dst: Reg, a: Reg, b: Reg, call: u32 },
    Neg { dst: Reg, a: Reg, call: u32 },
    Rel { op: RelOp, dst: Reg, a: Reg, b: Reg, call: u32 },
    Not { dst: Reg, a: Reg, call: u32 },
    /// Jump unless `cond` is TRUE, with `if`/`while` condition semantics.
    JumpIfNot { cond: Reg, target: u32, call: u32 },
    Jump { target: u32 },
    /// Prepare iteration over the value in `seq`.
    ForInit { seq: Reg, call: u32 },
    /// Assign the next element to `var`, or jump to `exit`.
    ForStep { seq: Reg, idx: Reg, var: Sym, exit: u32, call: u32 },
    /// Evaluate `consts[k]` with the AST interpreter.
    Eval { dst: Reg, k: u32 },
    /// If `sym` is rebound outside base, evaluate `consts[k]` instead and
    /// continue at `skip`.
    Guard { sym: Sym, dst: Reg, k: u32, skip: u32 },
    Null { dst: Reg },
    Return { src: Reg },
    /// Set result visibility.
    Visible { on: bool },
    /// Enter/leave a loop region (for `break`/`next` from the interpreter).
    LoopEnter { brk: u32, nxt: u32 },
    LoopExit,
}

pub struct Code {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    pub nregs: usize,
    /// The result register of the whole body.
    pub result: Reg,
    /// A closure body (`Return` ends the call) rather than a standalone
    /// loop (`Return` unwinds to the enclosing function).
    pub is_body: bool,
}

struct Loop {
    breaks: Vec<usize>,
    nexts: Vec<usize>,
}

struct Compiler {
    ops: Vec<Op>,
    consts: Vec<Value>,
    nregs: usize,
    loops: Vec<Loop>,
}

fn head(l: &Lang) -> Option<Sym> {
    l.fun.as_sym()
}

/// Positional arguments without tags, `...` or empty arguments.
fn plain_args(l: &Lang, n: usize) -> bool {
    l.args.len() == n
        && l.args.iter().all(|a| a.tag.is_none() && !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS))
}

fn arith_op(name: &str) -> Option<ArithOp> {
    Some(match name {
        "+" => ArithOp::Plus,
        "-" => ArithOp::Minus,
        "*" => ArithOp::Times,
        "/" => ArithOp::Div,
        "^" => ArithOp::Pow,
        "%%" => ArithOp::Mod,
        "%/%" => ArithOp::IDiv,
        _ => return None,
    })
}

fn rel_op(name: &str) -> Option<RelOp> {
    Some(match name {
        "==" => RelOp::Eq,
        "!=" => RelOp::Ne,
        "<" => RelOp::Lt,
        "<=" => RelOp::Le,
        ">" => RelOp::Gt,
        ">=" => RelOp::Ge,
        _ => return None,
    })
}

impl Compiler {
    fn reg(&mut self) -> Reg {
        self.nregs += 1;
        (self.nregs - 1) as Reg
    }

    fn konst(&mut self, v: Value) -> u32 {
        self.consts.push(v);
        (self.consts.len() - 1) as u32
    }

    fn emit(&mut self, op: Op) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    fn here(&self) -> u32 {
        self.ops.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        match &mut self.ops[at] {
            Op::Jump { target: t } | Op::JumpIfNot { target: t, .. } => *t = target,
            Op::ForStep { exit, .. } => *exit = target,
            Op::Guard { skip, .. } => *skip = target,
            Op::LoopEnter { brk, .. } => *brk = target,
            _ => unreachable!(),
        }
    }

    /// Emit a guard for `sym`; returns the guard's index to patch with the
    /// end of the construct.
    fn guard(&mut self, sym: Sym, whole: &Value, dst: Reg) -> usize {
        let k = self.konst(whole.clone());
        self.emit(Op::Guard { sym, dst, k, skip: 0 })
    }

    fn fallback(&mut self, e: &Value, dst: Reg) {
        let k = self.konst(e.clone());
        self.emit(Op::Eval { dst, k });
    }

    fn expr(&mut self, e: &Value, dst: Reg) {
        match e {
            Value::Sym(sym) if !sym.is_empty() && *sym != s::DOTS && sym.ddval().is_none() => {
                self.emit(Op::GetVar { dst, sym: *sym });
            }
            Value::Lang(l) => self.call(l, e, dst),
            Value::Sym(_) | Value::Promise(_) => self.fallback(e, dst),
            v => {
                let k = self.konst(v.clone());
                self.emit(Op::Const { dst, k });
                self.emit(Op::Visible { on: true });
            }
        }
    }

    fn call(&mut self, l: &Lang, whole: &Value, dst: Reg) {
        let Some(h) = head(l) else { return self.fallback(whole, dst) };
        let name = h.name();
        let name = &*name;
        match name {
            "{" => {
                let g = self.guard(h, whole, dst);
                if l.args.is_empty() {
                    self.emit(Op::Null { dst });
                    self.emit(Op::Visible { on: true });
                }
                for a in &l.args {
                    if a.value.is_missing_arg() {
                        return self.fallback(whole, dst);
                    }
                    self.expr(&a.value, dst);
                }
                let end = self.here();
                self.patch(g, end);
            }
            "(" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[0].value, dst);
                self.emit(Op::Visible { on: true });
                let end = self.here();
                self.patch(g, end);
            }
            "<-" | "=" if l.args.len() == 2 && matches!(l.args[0].value, Value::Sym(_)) && l.args[0].tag.is_none() => {
                let target = l.args[0].value.as_sym().unwrap();
                if target.is_empty() || target == s::DOTS {
                    return self.fallback(whole, dst);
                }
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[1].value, dst);
                self.emit(Op::SetVar { src: dst, sym: target });
                self.emit(Op::Visible { on: false });
                let end = self.here();
                self.patch(g, end);
            }
            "if" if (l.args.len() == 2 || l.args.len() == 3) && l.args.iter().all(|a| a.tag.is_none()) => {
                let g = self.guard(h, whole, dst);
                let c = self.reg();
                self.expr(&l.args[0].value, c);
                let kc = self.konst(whole.clone());
                let j = self.emit(Op::JumpIfNot { cond: c, target: 0, call: kc });
                self.expr(&l.args[1].value, dst);
                let jend = self.emit(Op::Jump { target: 0 });
                let else_start = self.here();
                self.patch(j, else_start);
                if l.args.len() == 3 {
                    self.expr(&l.args[2].value, dst);
                } else {
                    self.emit(Op::Null { dst });
                    self.emit(Op::Visible { on: false });
                }
                let end = self.here();
                self.patch(jend, end);
                self.patch(g, end);
            }
            "for" if l.args.len() == 3 && matches!(l.args[0].value, Value::Sym(_)) => {
                let var = l.args[0].value.as_sym().unwrap();
                let g = self.guard(h, whole, dst);
                let seq = self.reg();
                let idx = self.reg();
                self.expr(&l.args[1].value, seq);
                let kc = self.konst(whole.clone());
                self.emit(Op::ForInit { seq, call: kc });
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let top = self.here();
                let step = self.emit(Op::ForStep { seq, idx, var, exit: 0, call: kc });
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[2].value, body);
                self.emit(Op::Jump { target: top });
                let lp = self.loops.pop().unwrap();
                let exit = self.here();
                self.emit(Op::LoopExit);
                self.patch(step, exit);
                for b in lp.breaks {
                    self.patch(b, exit);
                }
                for n in lp.nexts {
                    self.patch(n, top);
                }
                if let Op::LoopEnter { brk, nxt } = &mut self.ops[enter] {
                    *brk = exit;
                    *nxt = top;
                }
                self.emit(Op::Null { dst });
                self.emit(Op::Visible { on: false });
                let end = self.here();
                self.patch(g, end);
            }
            "while" if plain_args(l, 2) => {
                let g = self.guard(h, whole, dst);
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let top = self.here();
                let c = self.reg();
                self.expr(&l.args[0].value, c);
                let kc = self.konst(whole.clone());
                let j = self.emit(Op::JumpIfNot { cond: c, target: 0, call: kc });
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[1].value, body);
                self.emit(Op::Jump { target: top });
                let lp = self.loops.pop().unwrap();
                let exit = self.here();
                self.emit(Op::LoopExit);
                self.patch(j, exit);
                for b in lp.breaks {
                    self.patch(b, exit);
                }
                for n in lp.nexts {
                    self.patch(n, top);
                }
                if let Op::LoopEnter { brk, nxt } = &mut self.ops[enter] {
                    *brk = exit;
                    *nxt = top;
                }
                self.emit(Op::Null { dst });
                self.emit(Op::Visible { on: false });
                let end = self.here();
                self.patch(g, end);
            }
            "repeat" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let top = self.here();
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[0].value, body);
                self.emit(Op::Jump { target: top });
                let lp = self.loops.pop().unwrap();
                let exit = self.here();
                self.emit(Op::LoopExit);
                for b in lp.breaks {
                    self.patch(b, exit);
                }
                for n in lp.nexts {
                    self.patch(n, top);
                }
                if let Op::LoopEnter { brk, nxt } = &mut self.ops[enter] {
                    *brk = exit;
                    *nxt = top;
                }
                self.emit(Op::Null { dst });
                self.emit(Op::Visible { on: false });
                let end = self.here();
                self.patch(g, end);
            }
            "break" | "next" if l.args.is_empty() && !self.loops.is_empty() => {
                let g = self.guard(h, whole, dst);
                let j = self.emit(Op::Jump { target: 0 });
                let lp = self.loops.last_mut().unwrap();
                if name == "break" {
                    lp.breaks.push(j);
                } else {
                    lp.nexts.push(j);
                }
                let end = self.here();
                self.patch(g, end);
            }
            "return" if l.args.len() <= 1 && l.args.iter().all(|a| a.tag.is_none() && !a.value.is_missing_arg()) => {
                // `return` from a loop at top level is not a function return;
                // only compile it inside closure bodies (see `compile_body`).
                let g = self.guard(h, whole, dst);
                if l.args.is_empty() {
                    self.emit(Op::Null { dst });
                } else {
                    self.expr(&l.args[0].value, dst);
                }
                self.emit(Op::Return { src: dst });
                let end = self.here();
                self.patch(g, end);
            }
            "!" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a);
                let k = self.konst(whole.clone());
                self.emit(Op::Not { dst, a, call: k });
                self.emit(Op::Visible { on: true });
                let end = self.here();
                self.patch(g, end);
            }
            "-" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a);
                let k = self.konst(whole.clone());
                self.emit(Op::Neg { dst, a, call: k });
                self.emit(Op::Visible { on: true });
                let end = self.here();
                self.patch(g, end);
            }
            n if plain_args(l, 2) && (arith_op(n).is_some() || rel_op(n).is_some()) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                let b = self.reg();
                self.expr(&l.args[0].value, a);
                self.expr(&l.args[1].value, b);
                let k = self.konst(whole.clone());
                if let Some(op) = arith_op(n) {
                    self.emit(Op::Arith { op, dst, a, b, call: k });
                } else {
                    self.emit(Op::Rel { op: rel_op(n).unwrap(), dst, a, b, call: k });
                }
                self.emit(Op::Visible { on: true });
                let end = self.here();
                self.patch(g, end);
            }
            _ => self.fallback(whole, dst),
        }
    }
}

/// Compile a closure body.
pub fn compile_body(body: &Value) -> Code {
    let mut c = Compiler { ops: Vec::new(), consts: Vec::new(), nregs: 0, loops: Vec::new() };
    let r = c.reg();
    c.expr(body, r);
    Code { ops: c.ops, consts: c.consts, nregs: c.nregs, result: r, is_body: true }
}

/// Compile a loop evaluated outside a compiled body (e.g. at top level).
/// Returns `None` unless `e` is a loop of the form the compiler
/// specializes (otherwise the compiled code would just call back into the
/// interpreter's loop).
pub fn compile_loop(e: &Value) -> Option<Code> {
    let Value::Lang(l) = e else { return None };
    let ok = match head(l).map(|s| s.name()).as_deref() {
        Some("for") => l.args.len() == 3 && matches!(l.args[0].value, Value::Sym(_)),
        Some("while") => plain_args(l, 2),
        Some("repeat") => plain_args(l, 1),
        _ => false,
    };
    if !ok {
        return None;
    }
    let mut c = Compiler { ops: Vec::new(), consts: Vec::new(), nregs: 0, loops: Vec::new() };
    let r = c.reg();
    c.call(l, e, r);
    Some(Code { ops: c.ops, consts: c.consts, nregs: c.nregs, result: r, is_body: false })
}

#[allow(dead_code)]
fn unused(_: Span) {}
