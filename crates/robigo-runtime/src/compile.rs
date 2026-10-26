//! A bytecode compiler for closure bodies and loops.
//!
//! The compiler specializes the constructs that dominate tight R code —
//! `{`, `(`, `if`, `for`, `while`, `repeat`, `break`, `next`, `return`,
//! assignment to a symbol, variable reads, arithmetic, comparison and `!` —
//! and leaves everything else to the AST interpreter through `Eval`
//! instructions, so any R expression can be compiled.
//!
//! Semantics are preserved by runtime guards: every specialized construct
//! first checks that its head symbol (`+`, `if`, `<-`, ...) still resolves
//! to the base function; if not, the original call is evaluated by the
//! interpreter instead. Variables read or assigned by the code get a slot
//! (see [`Code::vars`]) that lets the VM keep their values in registers
//! between operations that could observe the frame. Arithmetic
//! and comparison only take the fast path for attribute-free scalars and
//! otherwise call the regular builtin, which handles recycling,
//! attributes, dispatch and warnings.

use std::rc::Rc;

use robigo_syntax::Span;

use crate::builtins::arith::{ArithOp, RelOp};
use crate::sym::{s, Sym};
use crate::value::*;

pub type Reg = u16;
/// Index into [`Code::vars`].
pub type Slot = u16;

/// An index operand: a numeric constant, a variable (read without running
/// code), or a register holding an already evaluated pure expression.
#[derive(Clone, Copy, Debug)]
pub enum Leaf {
    D(f64),
    I(i32),
    Var(Sym, Slot),
    R(Reg),
}

/// One-argument builtins answered without a call for values that are not
/// objects (so no dispatch can happen); see `vm::builtin1_fast`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin1 {
    Length,
    IsNull,
    IsNa,
    IsAtomic,
    IsCharacter,
    IsLogical,
    IsNumeric,
    IsFunction,
    IsList,
    Attributes,
    /// `.Internal(typeof(x))`
    Typeof,
}

fn builtin1(name: &str) -> Option<Builtin1> {
    Some(match name {
        "length" => Builtin1::Length,
        "is.null" => Builtin1::IsNull,
        "is.na" => Builtin1::IsNa,
        "is.atomic" => Builtin1::IsAtomic,
        "is.character" => Builtin1::IsCharacter,
        "is.logical" => Builtin1::IsLogical,
        "is.numeric" => Builtin1::IsNumeric,
        "is.function" => Builtin1::IsFunction,
        "is.list" => Builtin1::IsList,
        "attributes" => Builtin1::Attributes,
        _ => return None,
    })
}

/// The one-argument math functions with an unboxed fast path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Math1 {
    Sqrt,
    Abs,
    Exp,
    Floor,
    Ceiling,
    Sin,
    Cos,
    Tan,
}

/// The sequences a `for` loop can step through without allocating them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    /// `a:b`
    Colon,
    /// `seq_len(n)`
    SeqLen,
    /// `seq_along(x)`
    SeqAlong,
}

#[derive(Clone, Debug)]
pub enum Op {
    Const { dst: Reg, k: u32 },
    /// Attribute-free scalar constants, unboxed.
    ConstD { dst: Reg, x: f64 },
    ConstI { dst: Reg, x: i32 },
    ConstL { dst: Reg, x: Lgl },
    GetVar { dst: Reg, sym: Sym, slot: Slot },
    /// Assign to a symbol in the current frame (`<-`/`=`).
    SetVar { src: Reg, sym: Sym, slot: Slot },
    Arith { op: ArithOp, dst: Reg, a: Reg, b: Reg, call: u32 },
    Neg { dst: Reg, a: Reg, call: u32 },
    Rel { op: RelOp, dst: Reg, a: Reg, b: Reg, call: u32 },
    Not { dst: Reg, a: Reg, call: u32 },
    /// Jump unless `cond` is TRUE, with `if`/`while` condition semantics.
    JumpIfNot { cond: Reg, target: u32, call: u32 },
    /// Jump if `cond` is TRUE (the test at the bottom of a `while` loop).
    JumpIf { cond: Reg, target: u32, call: u32 },
    Jump { target: u32 },
    /// Prepare iteration over the value in `seq`.
    ForInit { seq: Reg, call: u32 },
    /// Assign the next element to `var` and jump to `body`; fall through
    /// when the sequence is exhausted.
    ForStep { seq: Reg, idx: Reg, var: Sym, slot: Slot, body: u32, call: u32 },
    /// `x[i]` / `x[[i]]` (`two`) for a variable `x`: an in-range scalar
    /// index into a plain vector is read directly; anything else
    /// evaluates the call `consts[k]` with the interpreter.
    Index { dst: Reg, x: Sym, slot: Slot, i: Leaf, two: bool, k: u32 },
    /// The object case of a general `x[i]` / `x[[i]]` (call `consts[k]`),
    /// after `x` was evaluated into register `x` and before `i` is: if `x`
    /// is an object, call `[`/`[[` (which may dispatch) with `x`'s value and
    /// a promise of the index, and jump to `end`.
    SubsetObj { dst: Reg, x: Reg, two: bool, k: u32, end: u32 },
    /// The rest of a general `x[i]` / `x[[i]]` (`two`): an in-range scalar
    /// index into a plain vector is read directly, otherwise `[`/`[[` is
    /// called with both values.
    Subset { dst: Reg, x: Reg, i: Reg, two: bool, k: u32 },
    /// `x[i] <- val` / `x[[i]] <- val` with `val` already evaluated: an
    /// in-range scalar index into a plain local vector of a compatible type
    /// is updated in place; anything else runs the complex assignment
    /// `consts[k]` with that value.
    SetIndex { val: Reg, x: Sym, slot: Slot, i: Leaf, two: bool, k: u32 },
    /// `x[i, j]` for a variable `x`: an element of a plain matrix by
    /// in-range scalar indices is read directly; anything else evaluates
    /// the call `consts[k]`.
    Index2 { dst: Reg, x: Sym, slot: Slot, i: Leaf, j: Leaf, k: u32 },
    /// `x[i, j] <- val`: in place for a plain local matrix and in-range
    /// scalar indices, otherwise the complex assignment `consts[k]`.
    SetIndex2 { val: Reg, x: Sym, slot: Slot, i: Leaf, j: Leaf, k: u32 },
    /// `x$name <- val` (name `consts[name]`): in place for an existing
    /// element of a plain local list, otherwise the complex assignment
    /// `consts[k]`.
    SetDollar { val: Reg, x: Sym, slot: Slot, name: u32, k: u32 },
    /// Call the base builtin `consts[f]` with the evaluated arguments
    /// `arglists[args]`; `consts[call]` is the call.
    CallBuiltin { dst: Reg, f: u32, args: u32, call: u32 },
    /// `sqrt(a)` etc. on an attribute-free scalar; otherwise the builtin.
    Math1 { f: Math1, dst: Reg, a: Reg, call: u32 },
    /// The `x` (`!is_y`) or `y` operand of `&&`/`||` as a logical(1),
    /// with R's checks.
    AndOrArg { dst: Reg, a: Reg, and: bool, is_y: bool, call: u32 },
    /// Short-circuit: for `&&` (`and`) jump to `target` with `dst` FALSE
    /// if `x` is FALSE; for `||` with TRUE if `x` is TRUE.
    AndOrShort { x: Reg, dst: Reg, and: bool, target: u32 },
    /// Combine the logical operands of `&&`/`||`.
    AndOr { dst: Reg, x: Reg, y: Reg, and: bool },
    /// Set up `seq` for a `for` loop over `a:b`, `seq_len(a)` or
    /// `seq_along(a)`: an integer range when possible, else the vector the
    /// builtin returns.
    ForRange { kind: RangeKind, a: Reg, b: Reg, seq: Reg, call: u32 },
    /// Evaluate `consts[k]` with the AST interpreter.
    Eval { dst: Reg, k: u32 },
    /// `x$name` (call `consts[call]`) with `x` evaluated into register `x`
    /// and the name as a string `consts[name]`.
    Dollar { dst: Reg, x: Reg, name: u32, call: u32 },
    /// `missing(sym)` (call `consts[call]`).
    Missing { dst: Reg, sym: Sym, call: u32 },
    /// `switch(x, ...)` with `x` evaluated into register `x`: jump to the
    /// alternative `switches[table]` selects (call `consts[call]` for
    /// messages).
    Switch { x: Reg, table: u32, call: u32 },
    /// If `sym` is rebound outside base, evaluate `consts[k]` instead and
    /// continue at `skip`.
    Guard { sym: Sym, dst: Reg, k: u32, skip: u32, g: u32 },
    Null { dst: Reg },
    Return { src: Reg },
    /// Call the function `f` (not a base primitive), with the arguments of
    /// call site `site` as promises; falls back to `eval` for builtins.
    Call { dst: Reg, f: Sym, site: u32 },
    /// `a op b` with operands read directly (constants, variables, or
    /// registers holding evaluated subexpressions), guarding `sym` itself
    /// unless `g` is [`NO_GUARD`] (then a `Guard` op precedes it).
    ArithL { op: ArithOp, dst: Reg, a: Leaf, b: Leaf, call: u32, sym: Sym, g: u32 },
    RelL { op: RelOp, dst: Reg, a: Leaf, b: Leaf, call: u32, sym: Sym, g: u32 },
    /// `a:b`: an unmaterialized `RVal::Range` for integer-valued scalar
    /// ends, otherwise the builtin.
    Colon { dst: Reg, a: Reg, b: Reg, call: u32 },
    /// `f(a)` for a [`Builtin1`]; the builtin itself (`call`) when the
    /// fast path does not apply.
    Builtin1 { f: Builtin1, dst: Reg, a: Reg, call: u32 },
    /// Set result visibility.
    Visible { on: bool },
    /// Enter/leave a loop region (for `break`/`next` from the interpreter).
    LoopEnter { brk: u32, nxt: u32 },
    LoopExit,
}

/// `g` of an `ArithL`/`RelL` without its own guard.
pub const NO_GUARD: u32 = u32::MAX;

/// A memoized `Guard` result, valid for search epoch `epoch` when the
/// lookup starts in the environment with id `env` at function version
/// `version`, or (after checking the frames below it) starts from the
/// search-path environment with id `search`.
#[derive(Clone, Copy)]
pub struct GuardMemo {
    pub epoch: u64,
    pub env: usize,
    pub version: u32,
    pub search: usize,
    pub ok: bool,
}

/// A call compiled to `Op::Call`.
pub struct CallSite {
    pub call: Value,
    /// Bytecode for each argument's promise (`None`: use the interpreter).
    pub codes: Vec<Option<Rc<Code>>>,
    /// The function found from the first search-path environment: (search
    /// epoch, that environment's id, function).
    pub memo: std::cell::RefCell<Option<(u64, usize, Value)>>,
    /// The arguments, when all are positional constants or variables (for
    /// running pure closures inline).
    pub leaf_args: Option<Vec<Leaf>>,
    /// The slot of the function's name, if the code has one (filled in once
    /// compilation is done).
    pub fslot: Option<Slot>,
}

/// The jump targets of a compiled `switch` (R: eval.c:do_switch).
#[derive(Debug)]
pub struct SwitchTable {
    /// The tag of each alternative.
    pub names: Vec<Option<Sym>>,
    /// For a string matching alternative `k`: the code of the first
    /// non-missing alternative from `k` on (or `null`).
    pub by_name: Vec<u32>,
    /// The untagged alternative, if any.
    pub default: Option<u32>,
    /// For a number `k`: alternative `k`'s code, `None` if it is empty.
    pub by_number: Vec<Option<u32>>,
    /// Code that gives an invisible NULL.
    pub null: u32,
}

pub struct Code {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    pub nregs: usize,
    /// The variables the code reads or assigns, indexed by [`Slot`].
    pub vars: Vec<Sym>,
    /// Argument lists of `CallBuiltin`: (tag, register) pairs.
    pub arglists: Vec<Vec<(Option<Sym>, Reg)>>,
    /// Call sites of `Call`.
    pub call_sites: Vec<CallSite>,
    /// The result register of the whole body.
    pub result: Reg,
    /// A closure body (`Return` ends the call) rather than a standalone
    /// loop (`Return` unwinds to the enclosing function).
    pub is_body: bool,
    /// Number of `Guard` ops (numbered by their `g`).
    pub nguards: usize,
    /// Every function the code guards.
    pub guard_syms: Vec<Sym>,
    /// The code is a single guarded `ArithL`/`RelL` on constants and
    /// variables (a promise like `n - 1`): its index in `ops`.
    pub simple: Option<usize>,
    /// (search epoch, closure environment id, formals address) for which
    /// all guards are known to hold in a fresh call frame (see
    /// `vm::run_closure`).
    pub fresh_guards: std::cell::Cell<(u64, usize, usize)>,
    /// Native code for loops, by `LoopEnter` index (see `crate::jit`).
    pub jit: std::cell::RefCell<std::collections::HashMap<u32, crate::jit::JitSlot>>,
    /// Per-op memo of `Guard` results (see `vm::guard_ok`).
    pub guard_memo: Box<[std::cell::Cell<GuardMemo>]>,
    /// Tables of `Switch` ops.
    pub switches: Vec<SwitchTable>,
}

impl Code {
    fn new(c: Compiler, result: Reg, is_body: bool) -> Code {
        let has_guards = c.ops.iter().any(|o| {
            matches!(o, Op::Guard { .. }) || matches!(o, Op::ArithL { g, .. } | Op::RelL { g, .. } if *g != NO_GUARD)
        });
        let memo = if has_guards { c.ops.len() } else { 0 };
        let mut c = c;
        for op in &c.ops {
            if let Op::Call { f, site, .. } = op {
                c.call_sites[*site as usize].fslot = c.vars.iter().position(|v| v == f).map(|p| p as Slot);
            }
        }
        let mut guard_syms: Vec<Sym> = Vec::new();
        for op in &c.ops {
            match op {
                Op::Guard { sym, .. } => guard_syms.push(*sym),
                Op::ArithL { sym, g, .. } | Op::RelL { sym, g, .. } if *g != NO_GUARD => guard_syms.push(*sym),
                _ => {}
            }
        }
        guard_syms.sort();
        guard_syms.dedup();
        let plain = |l: &Leaf| !matches!(l, Leaf::R(_));
        let mut real = c.ops.iter().enumerate().filter(|(_, o)| !matches!(o, Op::Visible { .. }));
        let simple = match (real.next(), real.next()) {
            (Some((i, Op::ArithL { a, b, g, dst, .. } | Op::RelL { a, b, g, dst, .. })), None)
                if *g != NO_GUARD && plain(a) && plain(b) && *dst == result =>
            {
                Some(i)
            }
            _ => None,
        };
        let empty = GuardMemo { epoch: u64::MAX, env: 0, version: 0, search: 0, ok: false };
        let guard_memo = (0..memo).map(|_| std::cell::Cell::new(empty)).collect();
        Code {
            ops: c.ops,
            consts: c.consts,
            nregs: c.nregs,
            vars: c.vars,
            arglists: c.arglists,
            call_sites: c.call_sites,
            nguards: c.nguards,
            guard_syms,
            simple,
            jit: Default::default(),
            fresh_guards: std::cell::Cell::new((u64::MAX, 0, 0)),
            result,
            is_body,
            guard_memo,
            switches: c.switches,
        }
    }
}

/// How the result of an expression is used.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    /// Value and visibility (the result of a body or top-level loop).
    All,
    /// Only the value (operands, conditions, right-hand sides).
    Value,
    /// Neither (loop bodies, all but the last expression of `{`).
    Effect,
}

impl Want {
    fn value(self) -> Want {
        if self == Want::Effect { Want::Effect } else { Want::Value }
    }
}

struct Loop {
    breaks: Vec<usize>,
    nexts: Vec<usize>,
}

#[derive(Default)]
struct Compiler {
    ops: Vec<Op>,
    consts: Vec<Value>,
    /// Registers needed (the most live at once).
    nregs: usize,
    /// The next free register: registers are allocated as a stack, and a
    /// statement's temporaries are free again once it has run.
    top: usize,
    vars: Vec<Sym>,
    arglists: Vec<Vec<(Option<Sym>, Reg)>>,
    call_sites: Vec<CallSite>,
    nguards: usize,
    loops: Vec<Loop>,
    switches: Vec<SwitchTable>,
}

fn head(l: &Lang) -> Option<Sym> {
    l.fun.as_sym()
}

/// Positional arguments without tags, `...` or empty arguments.
fn plain_args(l: &Lang, n: usize) -> bool {
    l.args.len() == n
        && l.args.iter().all(|a| a.tag.is_none() && !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS))
}

/// A symbol that names an ordinary variable.
fn is_var(sym: Sym) -> bool {
    !sym.is_empty() && sym != s::DOTS && sym.ddval().is_none()
}

/// An index expression that can be evaluated before the vector is read
/// and before any dispatch without observable differences: constants,
/// variables, `length()` of a variable, and arithmetic, comparisons, `(`,
/// unary minus and `:` over such expressions (all guarded, so a rebound
/// `+` falls back).
fn is_pure(e: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match e {
        Value::Sym(s) => is_var(*s),
        Value::Real(_) | Value::Lgl(_) | Value::Int(_) => e.len() == 1 && e.attrs().is_none(),
        Value::Lang(l) => {
            let Some(h) = head(l) else { return false };
            let n = h.name();
            // `length(x)` of a variable (the `x[[length(x) + 1]] <- v` idiom).
            if &*n == "length" && plain_args(l, 1) {
                return matches!(&l.args[0].value, Value::Sym(s) if is_var(*s));
            }
            let ok_head = arith_op(&n).is_some() || rel_op(&n).is_some() || matches!(&*n, "(" | ":");
            ok_head && !l.args.is_empty() && l.args.len() <= 2 && plain_args(l, l.args.len()) && l.args.iter().all(|a| is_pure(&a.value, depth + 1))
        }
        _ => false,
    }
}

/// `x[i]` or `x[[i]]` with a variable `x` and a pure index: the
/// variable, whether it is `[[`, and the index expression.
fn index_form(e: &Value) -> Option<(Sym, bool, &Value)> {
    let Value::Lang(l) = e else { return None };
    let two = match head(l)?.name().as_ref() {
        "[" => false,
        "[[" => true,
        _ => return None,
    };
    if !plain_args(l, 2) {
        return None;
    }
    let x = l.args[0].value.as_sym().filter(|s| is_var(*s))?;
    let i = &l.args[1].value;
    is_pure(i, 0).then_some((x, two, i))
}

/// `x[i, j]` with a variable `x` and pure indices (see [`is_pure`]).
fn index2_form(e: &Value) -> Option<(Sym, &Value, &Value)> {
    let Value::Lang(l) = e else { return None };
    if &*head(l)?.name() != "[" || !plain_args(l, 3) {
        return None;
    }
    let x = l.args[0].value.as_sym().filter(|s| is_var(*s))?;
    let (i, j) = (&l.args[1].value, &l.args[2].value);
    (is_pure(i, 0) && is_pure(j, 0)).then_some((x, i, j))
}

/// `x$name` with a variable `x`: the variable and the name.
fn dollar_form(e: &Value) -> Option<(Sym, RStr)> {
    let Value::Lang(l) = e else { return None };
    if &*head(l)?.name() != "$" || !plain_args(l, 2) {
        return None;
    }
    let x = l.args[0].value.as_sym().filter(|s| is_var(*s))?;
    let name = match &l.args[1].value {
        Value::Sym(s) if is_var(*s) => Some(s.name()),
        Value::Str(v) if v.data.len() == 1 && v.attrs.is_none() => v.data[0].clone(),
        _ => return None,
    };
    Some((x, name))
}

fn math1(name: &str) -> Option<Math1> {
    Some(match name {
        "sqrt" => Math1::Sqrt,
        "abs" => Math1::Abs,
        "exp" => Math1::Exp,
        "floor" => Math1::Floor,
        "ceiling" => Math1::Ceiling,
        "sin" => Math1::Sin,
        "cos" => Math1::Cos,
        "tan" => Math1::Tan,
        _ => return None,
    })
}

/// The `.Internal` builtin of an inner call `f(args)` when `f` evaluates its
/// arguments and none is empty or `...`.
fn internal_call(e: &Value) -> Option<(&'static Builtin, &Lang)> {
    let Value::Lang(l) = e else { return None };
    let name = head(l)?.name();
    let b = crate::builtins::lookup_internal(&name).filter(|b| b.kind == BuiltinKind::Builtin)?;
    l.args.iter().all(|a| !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS)).then_some((b, &**l))
}

/// A base primitive evaluating its arguments (a BUILTINSXP), by name.
fn base_builtin(name: &str) -> Option<&'static Builtin> {
    crate::builtins::lookup_primitive(name).filter(|b| b.kind == BuiltinKind::Builtin)
}

/// A for-loop sequence the VM can step through without allocating it.
fn range_form(e: &Value) -> Option<(Sym, RangeKind, &Value, Option<&Value>)> {
    let Value::Lang(l) = e else { return None };
    let h = head(l)?;
    let kind = match h.name().as_ref() {
        ":" if plain_args(l, 2) => RangeKind::Colon,
        "seq_len" if plain_args(l, 1) => RangeKind::SeqLen,
        "seq_along" if plain_args(l, 1) => RangeKind::SeqAlong,
        _ => return None,
    };
    Some((h, kind, &l.args[0].value, l.args.get(1).map(|a| &a.value)))
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
        self.top += 1;
        self.nregs = self.nregs.max(self.top);
        (self.top - 1) as Reg
    }

    fn slot(&mut self, sym: Sym) -> Slot {
        match self.vars.iter().position(|s| *s == sym) {
            Some(i) => i as Slot,
            None => {
                self.vars.push(sym);
                (self.vars.len() - 1) as Slot
            }
        }
    }

    /// An index operand: a leaf, or the register `e` is evaluated into.
    fn index_operand(&mut self, e: &Value) -> Leaf {
        match self.leaf(e) {
            Some(l) => l,
            None => {
                let r = self.reg();
                self.expr(e, r, Want::Value);
                Leaf::R(r)
            }
        }
    }

    /// `e` as a [`Leaf`], if it is one.
    fn leaf(&mut self, e: &Value) -> Option<Leaf> {
        match e {
            Value::Sym(sym) if is_var(*sym) => Some(Leaf::Var(*sym, self.slot(*sym))),
            Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => Some(Leaf::D(v.data[0])),
            Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() => Some(Leaf::I(v.data[0])),
            _ => None,
        }
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
            Op::Jump { target: t } | Op::JumpIfNot { target: t, .. } | Op::JumpIf { target: t, .. } => *t = target,
            Op::Guard { skip, .. } => *skip = target,
            Op::LoopEnter { brk, .. } => *brk = target,
            Op::SubsetObj { end, .. } => *end = target,
            _ => unreachable!(),
        }
    }

    /// Emit a guard for `sym`; returns the guard's index to patch with the
    /// end of the construct.
    fn guard(&mut self, sym: Sym, whole: &Value, dst: Reg) -> usize {
        let k = self.konst(whole.clone());
        let g = self.nguards as u32;
        self.nguards += 1;
        self.emit(Op::Guard { sym, dst, k, skip: 0, g })
    }

    fn fallback(&mut self, e: &Value, dst: Reg) {
        let k = self.konst(e.clone());
        self.emit(Op::Eval { dst, k });
    }

    fn vis(&mut self, w: Want, on: bool) {
        if w == Want::All {
            self.emit(Op::Visible { on });
        }
    }

    /// Compile `e` into `dst`. `w` says whether the value and its
    /// visibility are used; anything with possible effects is compiled
    /// regardless.
    fn expr(&mut self, e: &Value, dst: Reg, w: Want) {
        match e {
            Value::Sym(sym) if is_var(*sym) => {
                let slot = self.slot(*sym);
                self.emit(Op::GetVar { dst, sym: *sym, slot });
            }
            Value::Lang(l) => {
                // The temporaries of a call are dead once its value is in `dst`.
                let top = self.top;
                self.call(l, e, dst, w);
                self.top = top;
            }
            Value::Sym(_) | Value::Promise(_) => self.fallback(e, dst),
            _ if w == Want::Effect => {}
            Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => {
                self.emit(Op::ConstD { dst, x: v.data[0] });
                self.vis(w, true);
            }
            Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() => {
                self.emit(Op::ConstI { dst, x: v.data[0] });
                self.vis(w, true);
            }
            Value::Lgl(v) if v.data.len() == 1 && v.attrs.is_none() => {
                self.emit(Op::ConstL { dst, x: v.data[0] });
                self.vis(w, true);
            }
            v => {
                let k = self.konst(v.clone());
                self.emit(Op::Const { dst, k });
                self.vis(w, true);
            }
        }
    }

    /// Patch the jumps of a finished loop body and set its LoopEnter.
    fn close_loop(&mut self, enter: usize, lp: Loop, exit: u32, next: u32) {
        for b in lp.breaks {
            self.patch(b, exit);
        }
        for n in lp.nexts {
            self.patch(n, next);
        }
        if let Op::LoopEnter { brk, nxt } = &mut self.ops[enter] {
            *brk = exit;
            *nxt = next;
        }
    }

    fn call(&mut self, l: &Lang, whole: &Value, dst: Reg, w: Want) {
        let Some(h) = head(l) else { return self.fallback(whole, dst) };
        let name = h.name();
        let name = &*name;
        match name {
            "{" => {
                let g = self.guard(h, whole, dst);
                if l.args.iter().any(|a| a.value.is_missing_arg()) {
                    return self.fallback(whole, dst);
                }
                if l.args.is_empty() && w != Want::Effect {
                    self.emit(Op::Null { dst });
                    self.vis(w, true);
                }
                let n = l.args.len();
                for (i, a) in l.args.iter().enumerate() {
                    self.expr(&a.value, dst, if i + 1 == n { w } else { Want::Effect });
                }
                let end = self.here();
                self.patch(g, end);
            }
            "(" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[0].value, dst, w.value());
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            "<-" | "=" if l.args.len() == 2 && matches!(l.args[0].value, Value::Sym(_)) && l.args[0].tag.is_none() => {
                let target = l.args[0].value.as_sym().unwrap();
                if target.is_empty() || target == s::DOTS {
                    return self.fallback(whole, dst);
                }
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let slot = self.slot(target);
                self.emit(Op::SetVar { src: dst, sym: target, slot });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
            }
            "<-" | "=" if l.args.len() == 2 && l.args[0].tag.is_none() && index_form(&l.args[0].value).is_some() => {
                let (x, two, i) = index_form(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let g2 = self.guard(Sym::new(if two { "[[<-" } else { "[<-" }), whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let i = self.index_operand(i);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                self.emit(Op::SetIndex { val: dst, x, slot, i, two, k });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "<-" | "=" if l.args.len() == 2 && l.args[0].tag.is_none() && index2_form(&l.args[0].value).is_some() => {
                let (x, i, j) = index2_form(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let g2 = self.guard(Sym::new("[<-"), whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let i = self.index_operand(i);
                let j = self.index_operand(j);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                self.emit(Op::SetIndex2 { val: dst, x, slot, i, j, k });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "<-" | "=" if l.args.len() == 2 && l.args[0].tag.is_none() && dollar_form(&l.args[0].value).is_some() => {
                let (x, name) = dollar_form(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let g2 = self.guard(Sym::new("$<-"), whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let slot = self.slot(x);
                let name = self.konst(Value::strs(vec![name]));
                let k = self.konst(whole.clone());
                self.emit(Op::SetDollar { val: dst, x, slot, name, k });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "[" if index2_form(whole).is_some() => {
                let (x, i, j) = index2_form(whole).unwrap();
                let g = self.guard(h, whole, dst);
                let i = self.index_operand(i);
                let j = self.index_operand(j);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                self.emit(Op::Index2 { dst, x, slot, i, j, k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            "[" | "[[" if index_form(whole).is_some() => {
                let (x, two, i) = index_form(whole).unwrap();
                let g = self.guard(h, whole, dst);
                let i = self.index_operand(i);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                self.emit(Op::Index { dst, x, slot, i, two, k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            // R: subset.c:do_subset3: `x` is evaluated, the name is not.
            "$" if l.args.len() == 2
                && l.args.iter().all(|a| a.tag.is_none())
                && !l.args[0].value.is_missing_arg()
                && l.args[0].value.as_sym() != Some(s::DOTS)
                && match &l.args[1].value {
                    Value::Sym(s) => is_var(*s),
                    Value::Str(v) => v.data.len() == 1 && v.attrs.is_none(),
                    _ => false,
                } =>
            {
                let g = self.guard(h, whole, dst);
                let x = self.reg();
                self.expr(&l.args[0].value, x, Want::Value);
                let name = match &l.args[1].value {
                    Value::Sym(s) => Value::str(&s.name()),
                    v => v.clone(),
                };
                let name = self.konst(name);
                let call = self.konst(whole.clone());
                self.emit(Op::Dollar { dst, x, name, call });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            "missing" if plain_args(l, 1) && matches!(&l.args[0].value, Value::Sym(s) if is_var(*s)) => {
                let g = self.guard(h, whole, dst);
                let sym = l.args[0].value.as_sym().unwrap();
                let call = self.konst(whole.clone());
                self.emit(Op::Missing { dst, sym, call });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            // R: eval.c:do_switch, for alternatives without `...` and at
            // most one default (more is an error R reports while matching).
            "switch"
                if l.args.len() >= 2
                    && l.args[0].tag.is_none()
                    && !l.args[0].value.is_missing_arg()
                    && l.args.iter().all(|a| a.value.as_sym() != Some(s::DOTS))
                    && l.args[1..].iter().filter(|a| a.tag.is_none()).count() <= 1
                    && !l.args[1..].iter().any(|a| a.tag.is_none() && a.value.is_missing_arg()) =>
            {
                let g = self.guard(h, whole, dst);
                let x = self.reg();
                self.expr(&l.args[0].value, x, Want::Value);
                let call = self.konst(whole.clone());
                let table = self.switches.len() as u32;
                self.switches.push(SwitchTable {
                    names: Vec::new(),
                    by_name: Vec::new(),
                    default: None,
                    by_number: Vec::new(),
                    null: 0,
                });
                self.emit(Op::Switch { x, table, call });
                let alts = &l.args[1..];
                let mut code: Vec<Option<u32>> = Vec::with_capacity(alts.len());
                let mut ends = Vec::new();
                for a in alts {
                    if a.value.is_missing_arg() {
                        code.push(None);
                        continue;
                    }
                    code.push(Some(self.here()));
                    self.expr(&a.value, dst, w);
                    ends.push(self.emit(Op::Jump { target: 0 }));
                }
                let null = self.here();
                self.emit(Op::Null { dst });
                self.vis(w, false);
                let end = self.here();
                for j in ends {
                    self.patch(j, end);
                }
                self.patch(g, end);
                let t = &mut self.switches[table as usize];
                t.names = alts.iter().map(|a| a.tag).collect();
                t.by_name = (0..alts.len()).map(|k| code[k..].iter().flatten().next().copied().unwrap_or(null)).collect();
                t.default = alts.iter().zip(&code).find(|(a, _)| a.tag.is_none()).and_then(|(_, c)| *c);
                t.by_number = code;
                t.null = null;
            }
            // R: subset.c:do_subset2 (DispatchOrEval): `x` is evaluated
            // first; only for non-objects is the index evaluated here. Not
            // for a symbol index, which may be a missing argument (an empty
            // subscript).
            "[" | "[[" if plain_args(l, 2) && !matches!(l.args[1].value, Value::Sym(_)) => {
                let two = &*h.name() == "[[";
                let g = self.guard(h, whole, dst);
                let x = self.reg();
                self.expr(&l.args[0].value, x, Want::Value);
                let k = self.konst(whole.clone());
                let obj = self.emit(Op::SubsetObj { dst, x, two, k, end: 0 });
                let i = self.reg();
                self.expr(&l.args[1].value, i, Want::Value);
                self.emit(Op::Subset { dst, x, i, two, k });
                self.vis(w, true);
                let end = self.here();
                self.patch(obj, end);
                self.patch(g, end);
            }
            "if" if (l.args.len() == 2 || l.args.len() == 3) && l.args.iter().all(|a| a.tag.is_none()) => {
                let g = self.guard(h, whole, dst);
                let c = self.reg();
                self.expr(&l.args[0].value, c, Want::Value);
                let kc = self.konst(whole.clone());
                let j = self.emit(Op::JumpIfNot { cond: c, target: 0, call: kc });
                self.expr(&l.args[1].value, dst, w);
                if l.args.len() == 3 {
                    let jend = self.emit(Op::Jump { target: 0 });
                    let else_start = self.here();
                    self.patch(j, else_start);
                    self.expr(&l.args[2].value, dst, w);
                    let end = self.here();
                    self.patch(jend, end);
                } else if w != Want::Effect {
                    let jend = self.emit(Op::Jump { target: 0 });
                    let else_start = self.here();
                    self.patch(j, else_start);
                    self.emit(Op::Null { dst });
                    self.vis(w, false);
                    let end = self.here();
                    self.patch(jend, end);
                } else {
                    let end = self.here();
                    self.patch(j, end);
                }
                let end = self.here();
                self.patch(g, end);
            }
            "for" if l.args.len() == 3 && matches!(l.args[0].value, Value::Sym(_)) => {
                // ForInit; LoopEnter; Jump step; body: ...; step: ForStep
                // (back to body while elements remain); exit: LoopExit.
                let var = l.args[0].value.as_sym().unwrap();
                let g = self.guard(h, whole, dst);
                let seq = self.reg();
                let idx = self.reg();
                if let Some((rh, kind, a_e, b_e)) = range_form(&l.args[1].value) {
                    // Guard the sequence function; on failure the
                    // interpreter evaluates the sequence expression.
                    let gs = self.guard(rh, &l.args[1].value, seq);
                    let a = self.reg();
                    let b = self.reg();
                    self.expr(a_e, a, Want::Value);
                    if let Some(b_e) = b_e {
                        self.expr(b_e, b, Want::Value);
                    }
                    let kseq = self.konst(l.args[1].value.clone());
                    self.emit(Op::ForRange { kind, a, b, seq, call: kseq });
                    let here = self.here();
                    self.patch(gs, here);
                } else {
                    self.expr(&l.args[1].value, seq, Want::Value);
                }
                let kc = self.konst(whole.clone());
                self.emit(Op::ForInit { seq, call: kc });
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let jstep = self.emit(Op::Jump { target: 0 });
                let body_top = self.here();
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[2].value, body, Want::Effect);
                let lp = self.loops.pop().unwrap();
                let step = self.here();
                self.patch(jstep, step);
                let slot = self.slot(var);
                self.emit(Op::ForStep { seq, idx, var, slot, body: body_top, call: kc });
                let exit = self.here();
                self.emit(Op::LoopExit);
                self.close_loop(enter, lp, exit, step);
                if w != Want::Effect {
                    self.emit(Op::Null { dst });
                }
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
            }
            "while" if plain_args(l, 2) => {
                // LoopEnter; Jump test; body: ...; test: cond; JumpIf body;
                // exit: LoopExit.
                let g = self.guard(h, whole, dst);
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let jtest = self.emit(Op::Jump { target: 0 });
                let body_top = self.here();
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[1].value, body, Want::Effect);
                let lp = self.loops.pop().unwrap();
                let test = self.here();
                self.patch(jtest, test);
                let c = self.reg();
                self.expr(&l.args[0].value, c, Want::Value);
                let kc = self.konst(whole.clone());
                self.emit(Op::JumpIf { cond: c, target: body_top, call: kc });
                let exit = self.here();
                self.emit(Op::LoopExit);
                self.close_loop(enter, lp, exit, test);
                if w != Want::Effect {
                    self.emit(Op::Null { dst });
                }
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
            }
            "repeat" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let enter = self.emit(Op::LoopEnter { brk: 0, nxt: 0 });
                let top = self.here();
                self.loops.push(Loop { breaks: vec![], nexts: vec![] });
                let body = self.reg();
                self.expr(&l.args[0].value, body, Want::Effect);
                self.emit(Op::Jump { target: top });
                let lp = self.loops.pop().unwrap();
                let exit = self.here();
                self.emit(Op::LoopExit);
                self.close_loop(enter, lp, exit, top);
                if w != Want::Effect {
                    self.emit(Op::Null { dst });
                }
                self.vis(w, false);
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
                // The value keeps its visibility: return(invisible(x)).
                let g = self.guard(h, whole, dst);
                if l.args.is_empty() {
                    self.emit(Op::Null { dst });
                    self.emit(Op::Visible { on: true });
                } else {
                    self.expr(&l.args[0].value, dst, Want::All);
                }
                self.emit(Op::Return { src: dst });
                let end = self.here();
                self.patch(g, end);
            }
            "!" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                let k = self.konst(whole.clone());
                self.emit(Op::Not { dst, a, call: k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            "-" if plain_args(l, 1) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                let k = self.konst(whole.clone());
                self.emit(Op::Neg { dst, a, call: k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            ":" if plain_args(l, 2) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                let b = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                self.expr(&l.args[1].value, b, Want::Value);
                let call = self.konst(whole.clone());
                self.emit(Op::Colon { dst, a, b, call });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            n if plain_args(l, 2)
                && (arith_op(n).is_some() || rel_op(n).is_some())
                && self.leaf(&l.args[1].value).is_some() =>
            {
                // Operands read in place. A variable on the left is only read
                // in place when the right is a leaf too: reading it may force
                // a promise or fail, which must happen before evaluating the
                // right operand. The guard is folded in when no operand needs
                // evaluating first (a failed guard re-evaluates everything).
                let b = self.leaf(&l.args[1].value).unwrap();
                let a_leaf = self.leaf(&l.args[0].value);
                let (a, g, guard) = match a_leaf {
                    Some(a) => {
                        let g = self.nguards as u32;
                        self.nguards += 1;
                        (a, g, None)
                    }
                    None => {
                        let gop = self.guard(h, whole, dst);
                        let r = self.reg();
                        self.expr(&l.args[0].value, r, Want::Value);
                        (Leaf::R(r), NO_GUARD, Some(gop))
                    }
                };
                let call = self.konst(whole.clone());
                if let Some(op) = arith_op(n) {
                    self.emit(Op::ArithL { op, dst, a, b, call, sym: h, g });
                } else {
                    self.emit(Op::RelL { op: rel_op(n).unwrap(), dst, a, b, call, sym: h, g });
                }
                self.vis(w, true);
                if let Some(gop) = guard {
                    let end = self.here();
                    self.patch(gop, end);
                }
            }
            n if plain_args(l, 2) && (arith_op(n).is_some() || rel_op(n).is_some()) => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                let b = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                self.expr(&l.args[1].value, b, Want::Value);
                let k = self.konst(whole.clone());
                if let Some(op) = arith_op(n) {
                    self.emit(Op::Arith { op, dst, a, b, call: k });
                } else {
                    self.emit(Op::Rel { op: rel_op(n).unwrap(), dst, a, b, call: k });
                }
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            "&&" | "||" if plain_args(l, 2) => {
                let and = name == "&&";
                let g = self.guard(h, whole, dst);
                let k = self.konst(whole.clone());
                let a = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                let x = self.reg();
                self.emit(Op::AndOrArg { dst: x, a, and, is_y: false, call: k });
                let short = self.emit(Op::AndOrShort { x, dst, and, target: 0 });
                let b = self.reg();
                self.expr(&l.args[1].value, b, Want::Value);
                let y = self.reg();
                self.emit(Op::AndOrArg { dst: y, a: b, and, is_y: true, call: k });
                self.emit(Op::AndOr { dst, x, y, and });
                let end = self.here();
                if let Op::AndOrShort { target, .. } = &mut self.ops[short] {
                    *target = end;
                }
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            n if plain_args(l, 1) && builtin1(n).is_some() => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                let call = self.konst(whole.clone());
                self.emit(Op::Builtin1 { f: builtin1(n).unwrap(), dst, a, call });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            n if plain_args(l, 1) && math1(n).is_some() => {
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&l.args[0].value, a, Want::Value);
                let k = self.konst(whole.clone());
                self.emit(Op::Math1 { f: math1(n).unwrap(), dst, a, call: k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            // `.Internal(f(...))` with `f` evaluating its arguments: call it
            // directly (do_internal does nothing else), reporting against
            // the inner call as R does.
            ".Internal"
                if plain_args(l, 1)
                    && matches!(&l.args[0].value, Value::Lang(il) if head(il).is_some_and(|f| &*f.name() == "typeof") && plain_args(il, 1)) =>
            {
                let Value::Lang(il) = &l.args[0].value else { unreachable!() };
                let g = self.guard(h, whole, dst);
                let a = self.reg();
                self.expr(&il.args[0].value, a, Want::Value);
                let call = self.konst(l.args[0].value.clone());
                self.emit(Op::Builtin1 { f: Builtin1::Typeof, dst, a, call });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            ".Internal" if plain_args(l, 1) && internal_call(&l.args[0].value).is_some() => {
                let (b, inner) = internal_call(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let mut list = Vec::with_capacity(inner.args.len());
                for a in &inner.args {
                    let r = self.reg();
                    self.expr(&a.value, r, Want::Value);
                    list.push((a.tag, r));
                }
                self.arglists.push(list);
                let args = (self.arglists.len() - 1) as u32;
                let f = self.konst(Value::Builtin(b));
                let call = self.konst(l.args[0].value.clone());
                self.emit(Op::CallBuiltin { dst, f, args, call });
                let end = self.here();
                self.patch(g, end);
            }
            n if base_builtin(n).is_some()
                && l.args.iter().all(|a| !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS)) =>
            {
                let b = base_builtin(n).unwrap();
                let g = self.guard(h, whole, dst);
                let mut list = Vec::with_capacity(l.args.len());
                for a in &l.args {
                    let r = self.reg();
                    self.expr(&a.value, r, Want::Value);
                    list.push((a.tag, r));
                }
                self.arglists.push(list);
                let args = (self.arglists.len() - 1) as u32;
                let f = self.konst(Value::Builtin(b));
                let call = self.konst(whole.clone());
                self.emit(Op::CallBuiltin { dst, f, args, call });
                let end = self.here();
                self.patch(g, end);
            }
            n if crate::builtins::lookup_primitive(n).is_none() => {
                let codes = l.args.iter().map(|a| compile_thunk(&a.value)).collect();
                let leaf_args = if l.args.iter().all(|a| a.tag.is_none()) {
                    l.args.iter().map(|a| self.leaf(&a.value)).collect::<Option<Vec<_>>>()
                } else {
                    None
                };
                self.call_sites.push(CallSite { call: whole.clone(), codes, memo: Default::default(), leaf_args, fslot: None });
                let site = (self.call_sites.len() - 1) as u32;
                self.emit(Op::Call { dst, f: h, site });
            }
            _ => self.fallback(whole, dst),
        }
    }
}

/// Compile a closure body.
pub fn compile_body(body: &Value) -> Code {
    let mut c = Compiler::default();
    let r = c.reg();
    c.expr(body, r, Want::All);
    Code::new(c, r, true)
}

/// Bytecode for a promise of `e`, when the compiler handles `e` itself
/// (not only by evaluating it in the interpreter). `Return` unwinds to the
/// function whose frame the promise is evaluated in.
pub fn compile_thunk(e: &Value) -> Option<Rc<Code>> {
    if !matches!(e, Value::Lang(_)) {
        return None;
    }
    let mut c = Compiler::default();
    let r = c.reg();
    c.expr(e, r, Want::All);
    let useful = c.ops.iter().any(|o| !matches!(o, Op::Eval { .. } | Op::Guard { .. } | Op::Visible { .. }));
    useful.then(|| Rc::new(Code::new(c, r, false)))
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
    let mut c = Compiler::default();
    let r = c.reg();
    c.call(l, e, r, Want::All);
    Some(Code::new(c, r, false))
}

#[allow(dead_code)]
fn unused(_: Span) {}
