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
use crate::sym::{Sym, s};
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
    Const {
        dst: Reg,
        k: u32,
    },
    /// Attribute-free scalar constants, unboxed.
    ConstD {
        dst: Reg,
        x: f64,
    },
    ConstI {
        dst: Reg,
        x: i32,
    },
    ConstL {
        dst: Reg,
        x: Lgl,
    },
    GetVar {
        dst: Reg,
        sym: Sym,
        slot: Slot,
    },
    /// `sym <<- src`: assign in the first enclosing frame that binds `sym`,
    /// or the global environment (`slot`: this code's slot for `sym`, whose
    /// cached value the assignment may make stale).
    SuperSet {
        src: Reg,
        sym: Sym,
        slot: Slot,
        /// As for `SetVar`.
        take: bool,
    },
    /// `x[i] <<- val` / `x[[i]] <<- val` (`two`; call `consts[k]`): in
    /// place where `sym <<- ` would assign a plain vector, otherwise the
    /// complex superassignment.
    SuperSetIndex {
        val: Reg,
        x: Sym,
        slot: Slot,
        i: Leaf,
        two: bool,
        k: u32,
        /// As for `SetVar`.
        take: bool,
    },
    /// Assign to a symbol in the current frame (`<-`/`=`).
    SetVar {
        src: Reg,
        sym: Sym,
        slot: Slot,
        /// In effect position: move the value out of the register (left
        /// dead) instead of copying it, so the register does not keep it
        /// alive (R: bcEval's SETVAR + POP; #357).
        take: bool,
    },
    Arith {
        op: ArithOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        call: u32,
    },
    Neg {
        dst: Reg,
        a: Reg,
        call: u32,
    },
    Rel {
        op: RelOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        call: u32,
    },
    Not {
        dst: Reg,
        a: Reg,
        call: u32,
    },
    /// Jump unless `cond` is TRUE, with `if`/`while` condition semantics.
    JumpIfNot {
        cond: Reg,
        target: u32,
        call: u32,
    },
    /// Jump if `cond` is TRUE (the test at the bottom of a `while` loop).
    JumpIf {
        cond: Reg,
        target: u32,
        call: u32,
    },
    Jump {
        target: u32,
    },
    /// Prepare iteration over the value in `seq`.
    ForInit {
        seq: Reg,
        call: u32,
    },
    /// Assign the next element to `var` and jump to `body`; fall through
    /// when the sequence is exhausted.
    ForStep {
        seq: Reg,
        idx: Reg,
        var: Sym,
        slot: Slot,
        body: u32,
        call: u32,
    },
    /// `x[i]` / `x[[i]]` (`two`) for a variable `x`: an in-range scalar
    /// index into a plain vector is read directly; anything else
    /// evaluates the call `consts[k]` with the interpreter.
    Index {
        dst: Reg,
        x: Sym,
        slot: Slot,
        i: Leaf,
        two: bool,
        k: u32,
    },
    /// The object case of a general `x[i]` / `x[[i]]` (call `consts[k]`),
    /// after `x` was evaluated into register `x` and before `i` is: if `x`
    /// is an object, call `[`/`[[` (which may dispatch) with `x`'s value and
    /// a promise of the index, and jump to `end`.
    SubsetObj {
        dst: Reg,
        x: Reg,
        two: bool,
        k: u32,
        end: u32,
    },
    /// The rest of a general `x[i]` / `x[[i]]` (`two`): an in-range scalar
    /// index into a plain vector is read directly, otherwise `[`/`[[` is
    /// called with both values.
    Subset {
        dst: Reg,
        x: Reg,
        i: Reg,
        two: bool,
        k: u32,
    },
    /// `x[i] <- val` / `x[[i]] <- val` with `val` already evaluated: an
    /// in-range scalar index into a plain local vector of a compatible type
    /// is updated in place; anything else runs the complex assignment
    /// `consts[k]` with that value.
    SetIndex {
        val: Reg,
        x: Sym,
        slot: Slot,
        i: Leaf,
        two: bool,
        k: u32,
        /// As for `SetVar`.
        take: bool,
    },
    /// A setter with arbitrary or missing indices. Delegate evaluation to
    /// the interpreter while retaining the compiled diagnostic source.
    SetComplex {
        val: Reg,
        slot: Slot,
        k: u32,
        /// As for `SetVar`.
        take: bool,
    },
    /// `x[i, j]` / `x[[i, j]]` for a variable `x`: an element of a plain
    /// atomic matrix by in-range scalar indices is read directly; anything
    /// else evaluates the call `consts[k]`.
    Index2 {
        dst: Reg,
        x: Sym,
        slot: Slot,
        i: Leaf,
        j: Leaf,
        two: bool,
        k: u32,
    },
    /// `x[i, j] <- val` / `x[[i, j]] <- val` (`two`): in place for a plain
    /// local matrix and in-range scalar indices, otherwise the complex
    /// assignment `consts[k]`.
    SetIndex2 {
        val: Reg,
        x: Sym,
        slot: Slot,
        i: Leaf,
        j: Leaf,
        two: bool,
        k: u32,
        /// As for `SetVar`.
        take: bool,
    },
    /// `x$name <- val` (name `consts[name]`): in place for an existing
    /// element of a plain local list, otherwise the complex assignment
    /// `consts[k]`.
    SetDollar {
        val: Reg,
        x: Sym,
        slot: Slot,
        name: u32,
        k: u32,
        /// As for `SetVar`.
        take: bool,
    },
    /// Call the base builtin `consts[f]` with the evaluated arguments
    /// `arglists[args]`; `consts[call]` is the call.
    CallBuiltin {
        dst: Reg,
        f: u32,
        args: u32,
        call: u32,
    },
    /// `sqrt(a)` etc. on an attribute-free scalar; otherwise the builtin.
    Math1 {
        f: Math1,
        dst: Reg,
        a: Reg,
        call: u32,
    },
    /// The `x` (`!is_y`) or `y` operand of `&&`/`||` as a logical(1),
    /// with R's checks.
    AndOrArg {
        dst: Reg,
        a: Reg,
        and: bool,
        is_y: bool,
        call: u32,
    },
    /// Short-circuit: for `&&` (`and`) jump to `target` with `dst` FALSE
    /// if `x` is FALSE; for `||` with TRUE if `x` is TRUE.
    AndOrShort {
        x: Reg,
        dst: Reg,
        and: bool,
        target: u32,
    },
    /// Combine the logical operands of `&&`/`||`.
    AndOr {
        dst: Reg,
        x: Reg,
        y: Reg,
        and: bool,
    },
    /// Set up `seq` for a `for` loop over `a:b`, `seq_len(a)` or
    /// `seq_along(a)`: an integer range when possible, else the vector the
    /// builtin returns.
    ForRange {
        kind: RangeKind,
        a: Reg,
        b: Reg,
        seq: Reg,
        call: u32,
    },
    /// Set up `seq` for a `for` loop over `seq(length.out = n)` (call
    /// `consts[call]`, `n` the leaf `len`): the integer range GNU R's
    /// `seq.default` returns when it can be read off without running code,
    /// else the value of the call (see `vm::seq_length_range`).
    ForSeqLength {
        len: Leaf,
        seq: Reg,
        call: u32,
    },
    /// Set up `seq` for a `for` loop over `seq(from, to, by)` (call
    /// `consts[call]`, the arguments `seq_by[k]`): the integer range GNU R's
    /// `seq.default` returns for integer arguments when it can be read off
    /// without running code, else the value of the call (see
    /// `vm::seq_by_range`).
    ForSeqBy {
        k: u32,
        seq: Reg,
        call: u32,
    },
    /// Evaluate `consts[k]` with the AST interpreter.
    Eval {
        dst: Reg,
        k: u32,
    },
    /// The call `consts[k]` to the special builtin `consts[f]`, evaluated
    /// as `Eval` would without looking the function up.
    CallSpecial {
        dst: Reg,
        f: u32,
        k: u32,
    },
    /// `x$name` (call `consts[call]`) with `x` evaluated into register `x`
    /// and the name as a string `consts[name]`.
    Dollar {
        dst: Reg,
        x: Reg,
        name: u32,
        call: u32,
    },
    /// `x@name` (call `consts[call]`) with `object` evaluated into register `object`
    /// and the slot name as a symbol `name`.
    SlotRead {
        dst: Reg,
        object: Reg,
        name: Sym,
        call: u32,
    },
    /// `missing(sym)` (call `consts[call]`).
    Missing {
        dst: Reg,
        sym: Sym,
        call: u32,
    },
    /// `switch(x, ...)` with `x` evaluated into register `x`: jump to the
    /// alternative `switches[table]` selects (call `consts[call]` for
    /// messages).
    Switch {
        x: Reg,
        table: u32,
        call: u32,
    },
    /// `function(formals) body`: a closure of `closures[t]` over the
    /// current frame (`consts[k]` is the call, for `trace()` of the
    /// primitive). R: eval.c, MAKECLOSURE.
    MakeClosure {
        dst: Reg,
        t: u32,
        k: u32,
    },
    /// If `sym` is rebound outside base, evaluate `consts[k]` instead and
    /// continue at `skip`.
    Guard {
        sym: Sym,
        dst: Reg,
        k: u32,
        skip: u32,
        g: u32,
    },
    /// `dst <- NULL`. `dead`: not a value but the clear of a register whose
    /// value is dead (#357; `Compiler::discard`): the VM puts a scalar
    /// there (`vm::DEAD`), native code nothing.
    Null {
        dst: Reg,
        dead: bool,
    },
    Return {
        src: Reg,
    },
    /// Call the function `f` (not a base primitive), with the arguments of
    /// call site `site` as promises; falls back to `eval` for builtins.
    Call {
        dst: Reg,
        f: Sym,
        site: u32,
    },
    /// `a op b` with operands read directly (constants, variables, or
    /// registers holding evaluated subexpressions), guarding `sym` itself
    /// unless `g` is [`NO_GUARD`] (then a `Guard` op precedes it).
    ArithL {
        op: ArithOp,
        dst: Reg,
        a: Leaf,
        b: Leaf,
        call: u32,
        sym: Sym,
        g: u32,
    },
    RelL {
        op: RelOp,
        dst: Reg,
        a: Leaf,
        b: Leaf,
        call: u32,
        sym: Sym,
        g: u32,
    },
    /// `a:b`: an unmaterialized `RVal::Range` for integer-valued scalar
    /// ends, otherwise the builtin.
    Colon {
        dst: Reg,
        a: Reg,
        b: Reg,
        call: u32,
    },
    /// `f(a)` for a [`Builtin1`]; the builtin itself (`call`) when the
    /// fast path does not apply.
    Builtin1 {
        f: Builtin1,
        dst: Reg,
        a: Reg,
        call: u32,
    },
    /// Set result visibility.
    Visible {
        on: bool,
    },
    /// R_Srcref = `consts[k]` (a srcref, or NULL): the statement of a `{`
    /// block that starts here (eval.c:do_begin). Only code with source
    /// references has these ops.
    Srcref {
        k: u32,
    },
    /// Save R_Srcref in `dst` where an outermost `{` with source references
    /// starts ([`Op::SrcrefRestore`] puts it back where it ends, as eval.c:
    /// eval does).
    SrcrefSave {
        dst: Reg,
    },
    SrcrefRestore {
        src: Reg,
    },
    /// Enter/leave a loop region (for `break`/`next` from the interpreter).
    LoopEnter {
        brk: u32,
        nxt: u32,
    },
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
    /// For a frame whose parent `outer` (with function bindings at
    /// `outer_version`) lies directly below the search path: other frames
    /// with that parent (calls of a closure defined in a call) share the
    /// result unless they bind the name themselves.
    pub outer: usize,
    pub outer_version: u32,
}

impl GuardMemo {
    /// Valid for nothing.
    pub const NONE: GuardMemo =
        GuardMemo { epoch: u64::MAX, env: 0, version: 0, search: 0, ok: false, outer: 0, outer_version: 0 };
}

/// R: compiler/R/cmp.R:safeBaseInternals. Base closures that installed
/// bytecode inlines as direct `.Internal` calls (guard-free in locked
/// namespaces, `BASEGUARD`-checked in user code). The index is the
/// [`CallSite::si`] value and the position in the startup snapshot.
pub const SIMPLE_INTERNALS: [&str; 20] = [
    "atan2",
    "besselY",
    "beta",
    "choose",
    "drop",
    "inherits",
    "is.vector",
    "lbeta",
    "lchoose",
    "nchar",
    "polyroot",
    "typeof",
    "vector",
    "which.max",
    "which.min",
    "is.loaded",
    "identical",
    "match",
    "rep.int",
    "rep_len",
];

/// One substituted argument of an inlined `.Internal` call (R:
/// compiler/R/cmp.R:inlineSimpleInternalCall): an actual argument of the
/// call, or a constant formal default (`simpleFormals` allows only
/// constants and missing, and missing falls back to the generic call).
#[derive(Clone)]
pub enum SiArg {
    Actual(usize),
    Const(Value),
}

/// The inlined form of a [`CallSite::si`] call site for one caller
/// closure: the definition it was resolved against, the internal name,
/// and the argument map.
#[derive(Clone)]
pub struct SiInline {
    pub def: Rc<Closure>,
    pub fun: Sym,
    pub args: Vec<SiArg>,
}

/// The write-once resolution of a [`CallSite::si`] call site: the caller
/// closure it was resolved for, whether that caller is namespace-scoped,
/// and the inline (`None`: never inline here). The caller is held weakly:
/// GNU R inlines at compile time and keeps no reference to the caller, so
/// this cache must not keep it, its environment or what that reaches
/// alive (R6's clone() left every cloned object unfinalizable). The weak
/// reference still keeps the allocation, so its address is not reused
/// while the memo names it.
pub type SiMemo = Option<(std::rc::Weak<Closure>, bool, Option<SiInline>)>;

/// A call compiled to `Op::Call`.
pub struct CallSite {
    pub call: Value,
    /// Bytecode for each argument's promise (`None`: use the interpreter).
    pub codes: Vec<Option<Rc<Code>>>,
    /// The function found from the first search-path environment: (search
    /// epoch, that environment's id, function).
    pub memo: std::cell::RefCell<Option<(u64, usize, Value)>>,
    /// Index into [`SIMPLE_INTERNALS`] when the call head names one.
    pub si: Option<u8>,
    /// The resolution of [`CallSite::si`] (see [`SiMemo`]).
    pub si_memo: std::cell::RefCell<SiMemo>,
    /// The arguments, when all are positional constants or variables (for
    /// running pure closures inline).
    pub leaf_args: Option<Vec<Leaf>>,
    /// The arguments, when all are positional leaves or single arithmetic
    /// operations on leaves (for running pure closures inline: see
    /// `vm::inline_arg`).
    pub inline_args: Option<Vec<InlineArg>>,
    /// Where `inline_args` is `None` because some argument is arithmetic of
    /// arithmetic: the arguments as native code evaluates them. Only native
    /// call trees take such calls (`jit::closure`); the VM makes them.
    pub native_args: Option<Vec<NativeArg>>,
    /// `inline_args` has one to four arguments, one of them not a constant:
    /// the caller may evaluate some itself (`vm::caller_promargs`).
    pub eager: bool,
    /// The formals (address) of the last closure for which that is
    /// impossible for reasons of the call site and the closure's
    /// definition alone (so later calls of it skip the attempt).
    pub no_eager: std::cell::Cell<usize>,
    /// The same for native code (`jit::closure`): the formals (address) of
    /// the last closure that has none here.
    pub no_native: std::cell::Cell<usize>,
    /// For the native tree called here (`jit::closure`): its entry (id)
    /// and the slots of this code whose variables it may read, as bits
    /// (bit 63: any slot from 63 on).
    pub native_reads: std::cell::Cell<(usize, u64)>,
    /// The slot of the function's name, if the code has one (filled in once
    /// compilation is done).
    pub fslot: Option<Slot>,
    /// `forceAndCall(force, f, ...)`: the number of promise arguments of a
    /// closure forced before the call.
    pub force: u32,
    /// In effect position: the VM leaves the destination register dead
    /// instead of storing the value (the call's own `POP`, #357; see
    /// `Compiler::discard`), saving an `Op::Null` per call.
    pub effect: bool,
    /// A base closure the call may answer without running it (see
    /// [`crate::intrinsic`]).
    pub intrinsic: Option<Box<crate::intrinsic::Site>>,
}

/// The variables `ops` read, in order, before the first operation that may
/// run R code, assuming every guard holds: guards, constants and variable
/// reads come first; an arithmetic or comparison op reads its operands and
/// may then run code (its slow path). Reading a variable may force a
/// promise, which is why callers walk this list in order and stop where
/// they cannot supply the value themselves.
fn strict_prefix(ops: &[Op]) -> Vec<Sym> {
    let mut out = Vec::new();
    for op in ops {
        match op {
            Op::Guard { .. }
            | Op::Const { .. }
            | Op::ConstD { .. }
            | Op::ConstI { .. }
            | Op::ConstL { .. }
            | Op::Null { .. }
            | Op::Visible { .. } => {}
            Op::GetVar { sym, .. } => out.push(*sym),
            Op::ArithL { a, b, .. } | Op::RelL { a, b, .. } => {
                for l in [a, b] {
                    if let Leaf::Var(sym, _) = l {
                        out.push(*sym);
                    }
                }
                break;
            }
            _ => break,
        }
    }
    out
}

/// An argument of a call that may run a pure closure inline.
pub enum InlineArg {
    Leaf(Leaf),
    /// `a op b`, evaluated eagerly only where it takes its scalar fast path
    /// (no warning, error or other observable effect) and `sym` resolves to
    /// base's (memoized as for a `Guard`).
    Arith {
        op: ArithOp,
        a: Leaf,
        b: Leaf,
        sym: Sym,
        memo: std::cell::Cell<GuardMemo>,
    },
    /// `x[[i]]` (`two`) or `x[i]` of a variable `x`, evaluated eagerly only
    /// where it takes the scalar fast path of `Op::Index` and `sym` (`[[` or
    /// `[`) resolves to base's.
    Index {
        x: Sym,
        slot: Slot,
        i: Leaf,
        two: bool,
        sym: Sym,
        memo: std::cell::Cell<GuardMemo>,
    },
}

/// An argument of a call in a native call tree: an [`InlineArg`], or
/// arithmetic of arithmetic (`2L * item - 1L`, `-i`), evaluated eagerly on
/// the terms of `InlineArg::Arith`, operator by operator.
pub enum NativeArg {
    Plain(InlineArg),
    Nested(Box<NestedArith>),
}

/// `a op b`, or `-a` when `b` is absent, with operands that are leaves or
/// arithmetic themselves (not element reads), at most [`NESTED_DEPTH`]
/// operators deep.
pub struct NestedArith {
    pub op: ArithOp,
    pub a: NativeArg,
    pub b: Option<NativeArg>,
    pub sym: Sym,
}

/// How deep a [`NestedArith`] argument may be.
const NESTED_DEPTH: usize = 3;

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
    /// For a closure defined in a frame directly below the search path:
    /// (search epoch, closure environment id, its function-binding version,
    /// formals address) for which all guards hold in a fresh call frame,
    /// and the key of the last call whose guards were not checked yet
    /// (they are, from its second call with the same key on).
    pub fresh_nested: std::cell::Cell<(u64, usize, u32, usize)>,
    /// The variables the code reads, in order, before its first operation
    /// that may run R code (see [`strict_prefix`]): a caller may evaluate
    /// arguments for formals among them itself (`vm::caller_promargs`).
    pub strict: Vec<Sym>,
    pub fresh_seen: std::cell::Cell<(u64, usize, u32, usize)>,
    /// For a closure made in a call frame of a namespace function: (search
    /// epoch, the namespace's id and function-binding version, formals
    /// address) for which all guards hold in a fresh call frame, when that
    /// frame binds none of their names (see `vm::fresh_nested_ok`).
    pub fresh_outer: std::cell::Cell<(u64, usize, u32, usize)>,
    /// Native code for loops, by `LoopEnter` index (see `crate::jit`).
    pub jit: std::cell::RefCell<std::collections::HashMap<u32, crate::jit::JitSlot>>,
    /// Native code for the whole closure (see `crate::jit::closure`).
    pub native: crate::jit::closure::NativeSlot,
    /// Per-op memo of `Guard` results (see `vm::guard_ok`).
    pub guard_memo: Box<[std::cell::Cell<GuardMemo>]>,
    /// Tables of `Switch` ops.
    pub switches: Vec<SwitchTable>,
    /// What `MakeClosure` ops make closures of.
    pub closures: Vec<ClosureTemplate>,
    /// Arguments of `ForSeqBy` ops.
    pub seq_by: Vec<[SeqArg; 3]>,
}

/// An argument of `seq(from, to, by)` in a `for` loop: a constant, a
/// variable, or the arithmetic of two of them (`x_start - 1L`), computed by
/// the VM only where that cannot be observed (no warning, no dispatch).
#[derive(Clone, Copy, Debug)]
pub struct SeqArg {
    pub op: Option<ArithOp>,
    pub a: Leaf,
    pub b: Leaf,
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
        let strict = if is_body { strict_prefix(&c.ops) } else { Vec::new() };
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
        let empty = GuardMemo { epoch: u64::MAX, env: 0, version: 0, search: 0, ok: false, outer: 0, outer_version: 0 };
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
            native: Default::default(),
            fresh_guards: std::cell::Cell::new((u64::MAX, 0, 0)),
            fresh_nested: std::cell::Cell::new((u64::MAX, 0, 0, 0)),
            strict,
            fresh_seen: std::cell::Cell::new((u64::MAX, 0, 0, 0)),
            fresh_outer: std::cell::Cell::new((u64::MAX, 0, 0, 0)),
            result,
            is_body,
            guard_memo,
            switches: c.switches,
            closures: c.closures,
            seq_by: c.seq_by,
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

/// The formals, body and source reference of a `function` expression,
/// converted once when its code is compiled.
pub struct ClosureTemplate {
    pub formals: Rc<[Arg]>,
    pub body: Value,
    pub srcref: Option<Value>,
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
    /// The `consts` index of the srcref of the `{` statement being compiled,
    /// inside a block with source references (see [`Op::Srcref`]).
    srcref_k: Option<u32>,
    nguards: usize,
    loops: Vec<Loop>,
    switches: Vec<SwitchTable>,
    closures: Vec<ClosureTemplate>,
    seq_by: Vec<[SeqArg; 3]>,
    /// A promise thunk (an argument or default expression). GNU R never
    /// compiles promise expressions: forcing evaluates the AST, so a
    /// call runs the callee's closure — an inlined simple `.Internal`
    /// wrapper here would drop the closure's frame from error
    /// `Calls:` traces and `sys.call()` where GNU R shows it (issue
    /// #157). Inlining stays for calls in direct position.
    thunk: bool,
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
            ok_head
                && !l.args.is_empty()
                && l.args.len() <= 2
                && plain_args(l, l.args.len())
                && l.args.iter().all(|a| is_pure(&a.value, depth + 1))
        }
        _ => false,
    }
}

/// An expression the VM evaluates without calling out: [`is_pure`] ones,
/// and `x[i]` / `x[[i]]` reads (see [`index_form`]) within arithmetic.
fn is_simple(e: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if is_pure(e, depth) || index_form(e).is_some() {
        return true;
    }
    let Value::Lang(l) = e else { return false };
    let Some(h) = head(l) else { return false };
    let n = h.name();
    (arith_op(&n).is_some() || rel_op(&n).is_some() || &*n == "(")
        && !l.args.is_empty()
        && l.args.len() <= 2
        && plain_args(l, l.args.len())
        && l.args.iter().all(|a| is_simple(&a.value, depth + 1))
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

/// `x[i, j]` / `x[[i, j]]` (`two`) with a variable `x` and pure indices
/// (see [`is_pure`]).
fn index2_form(e: &Value) -> Option<(Sym, bool, &Value, &Value)> {
    let Value::Lang(l) = e else { return None };
    let two = match head(l)?.name().as_ref() {
        "[" => false,
        "[[" => true,
        _ => return None,
    };
    if !plain_args(l, 3) {
        return None;
    }
    let x = l.args[0].value.as_sym().filter(|s| is_var(*s))?;
    let (i, j) = (&l.args[1].value, &l.args[2].value);
    (is_pure(i, 0) && is_pure(j, 0)).then_some((x, two, i, j))
}

/// The `n` of `forceAndCall(n, ...)` when it is a small constant.
fn force_count(e: &Value) -> Option<u32> {
    let n = match e {
        Value::Real(v) if v.data.len() == 1 && v.attrs.is_none() => v.data[0],
        Value::Int(v) if v.data.len() == 1 && v.attrs.is_none() && v.data[0] != NA_INTEGER => v.data[0] as f64,
        _ => return None,
    };
    ((0.0..=64.0).contains(&n) && n == n.trunc()).then_some(n as u32)
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

/// `seq(length.out = n)` with the one argument named by a prefix of
/// `length.out` (as `seq.default`'s formals match it) and `n` a variable or
/// a numeric constant: `seq` and `n`.
fn seq_length_form(e: &Value) -> Option<(Sym, &Value)> {
    let Value::Lang(l) = e else { return None };
    let h = head(l)?;
    if &*h.name() != "seq" || l.args.len() != 1 {
        return None;
    }
    let tag = l.args[0].tag?;
    let tag = tag.name();
    if tag.is_empty() || !"length.out".starts_with(&*tag) {
        return None;
    }
    let v = &l.args[0].value;
    leaf_shape(v).then_some((h, v))
}

/// What [`Compiler::leaf`] converts: a variable, or a length-1 numeric
/// constant without attributes. (The forms above match only these, so the
/// compiler's conversions of their operands cannot fail.)
fn leaf_shape(v: &Value) -> bool {
    match v {
        Value::Sym(s) => is_var(*s),
        Value::Real(_) | Value::Int(_) => v.len() == 1 && v.attrs().is_none(),
        _ => false,
    }
}

/// `seq(from, to, by)` with three positional arguments, each a variable, a
/// numeric constant, or `+`/`-` of two of those: `seq`, the arguments, and
/// the operators they use.
fn seq_by_form(e: &Value) -> Option<(Sym, [&Value; 3], Vec<Sym>)> {
    let Value::Lang(l) = e else { return None };
    let h = head(l)?;
    if &*h.name() != "seq" || !plain_args(l, 3) {
        return None;
    }
    let leaf = leaf_shape;
    let mut ops = Vec::new();
    for a in &l.args {
        let v = &a.value;
        if leaf(v) {
            continue;
        }
        let Value::Lang(al) = v else { return None };
        let op = head(al)?;
        let unary_minus = &*op.name() == "-" && plain_args(al, 1);
        if !matches!(&*op.name(), "+" | "-")
            || !(plain_args(al, 2) || unary_minus)
            || !al.args.iter().all(|x| leaf(&x.value))
        {
            return None;
        }
        if !ops.contains(&op) {
            ops.push(op);
        }
    }
    Some((h, [&l.args[0].value, &l.args[1].value, &l.args[2].value], ops))
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

    /// `e` as an [`InlineArg`], if it can be one.
    fn inline_arg(&mut self, e: &Value) -> Option<InlineArg> {
        if let Some(l) = self.leaf(e) {
            return Some(InlineArg::Leaf(l));
        }
        let Value::Lang(l) = e else { return None };
        let h = head(l)?;
        let memo = || {
            std::cell::Cell::new(GuardMemo {
                epoch: u64::MAX,
                env: 0,
                version: 0,
                search: 0,
                ok: false,
                outer: 0,
                outer_version: 0,
            })
        };
        let two = h.with_name(|n| n == "[[");
        if (two || h.with_name(|n| n == "[")) && plain_args(l, 2) {
            let (Some(Leaf::Var(x, slot)), Some(i @ (Leaf::D(_) | Leaf::I(_) | Leaf::Var(..)))) =
                (self.leaf(&l.args[0].value), self.leaf(&l.args[1].value))
            else {
                return None;
            };
            return Some(InlineArg::Index { x, slot, i, two, sym: h, memo: memo() });
        }
        let op = arith_op(&h.name())?;
        if !plain_args(l, 2) {
            return None;
        }
        let (a, b) = (self.leaf(&l.args[0].value)?, self.leaf(&l.args[1].value)?);
        let memo = GuardMemo { epoch: u64::MAX, env: 0, version: 0, search: 0, ok: false, outer: 0, outer_version: 0 };
        Some(InlineArg::Arith { op, a, b, sym: h, memo: std::cell::Cell::new(memo) })
    }

    /// `e` as a [`NativeArg`], if it can be one, `depth` operators down.
    fn native_arg(&mut self, e: &Value, depth: usize) -> Option<NativeArg> {
        if let Some(a) = self.inline_arg(e) {
            return match a {
                InlineArg::Index { .. } if depth > 0 => None,
                a => Some(NativeArg::Plain(a)),
            };
        }
        if depth + 1 >= NESTED_DEPTH {
            return None;
        }
        let Value::Lang(l) = e else { return None };
        let h = head(l)?;
        let op = arith_op(&h.name())?;
        let (a, b) = if op == ArithOp::Minus && plain_args(l, 1) {
            (self.native_arg(&l.args[0].value, depth + 1)?, None)
        } else if plain_args(l, 2) {
            let a = self.native_arg(&l.args[0].value, depth + 1)?;
            (a, Some(self.native_arg(&l.args[1].value, depth + 1)?))
        } else {
            return None;
        };
        Some(NativeArg::Nested(Box::new(NestedArith { op, a, b, sym: h })))
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

    /// `Op::Call` of the function named `f` for the call `whole`.
    fn call_site(&mut self, l: &Lang, whole: &Value, dst: Reg, f: Sym, force: u32) {
        let codes = l.args.iter().map(|a| compile_thunk(&a.value)).collect();
        let inline_args = if l.args.iter().all(|a| a.tag.is_none()) {
            l.args.iter().map(|a| self.inline_arg(&a.value)).collect::<Option<Vec<_>>>()
        } else {
            None
        };
        let native_args = if inline_args.is_none() && l.args.iter().all(|a| a.tag.is_none()) {
            l.args.iter().map(|a| self.native_arg(&a.value, 0)).collect::<Option<Vec<_>>>()
        } else {
            None
        };
        let leaf_args = inline_args.as_ref().and_then(|a| {
            a.iter()
                .map(|a| match a {
                    InlineArg::Leaf(l) => Some(*l),
                    InlineArg::Arith { .. } | InlineArg::Index { .. } => None,
                })
                .collect::<Option<Vec<_>>>()
        });
        self.call_sites.push(CallSite {
            si: if self.thunk {
                // Load-bearing: try_inline_site decides on `site.si` alone
                // (it has no env check), so a thunk whose head names a
                // simple internal must carry None — restoring Some here
                // regresses issue #157 through the compiled path.
                None
            } else {
                f.with_name(|n| SIMPLE_INTERNALS.iter().position(|m| *m == n).map(|i| i as u8))
            },
            si_memo: std::cell::RefCell::new(None),
            call: whole.clone(),
            codes,
            memo: Default::default(),
            leaf_args,
            // (Not for element reads: `vm::caller_promargs` gains nothing by
            // them, they are for calls that run without a frame.)
            eager: inline_args.as_ref().is_some_and(|a| {
                (1..=4).contains(&a.len())
                    && a.iter().any(|x| !matches!(x, InlineArg::Leaf(Leaf::D(_) | Leaf::I(_))))
                    && !a.iter().any(|x| matches!(x, InlineArg::Index { .. }))
            }),
            inline_args,
            native_args,
            no_eager: std::cell::Cell::new(0),
            no_native: std::cell::Cell::new(0),
            native_reads: std::cell::Cell::new((0, 0)),
            fslot: None,
            force,
            effect: false,
            intrinsic: crate::intrinsic::site(f, l),
        });
        let site = (self.call_sites.len() - 1) as u32;
        self.emit(Op::Call { dst, f, site });
    }

    fn fallback(&mut self, e: &Value, dst: Reg) {
        let k = self.konst(e.clone());
        self.emit(Op::Eval { dst, k });
    }

    /// In effect position, a value left in `dst` is dead: clear it, or the
    /// register keeps it alive until the next write (a `gc()` in a later
    /// statement could not collect it). R: bcEval's POP after each
    /// non-last statement of `{` and after a loop body (cmp.R cmpBrace,
    /// cmpFor...), and memory.c scans the node stack only up to
    /// R_BCNodeStackTop. Stores take their value themselves (`take`), and
    /// control flow (`{`, `if`, loops) leaves nothing of its own: only an
    /// op that last wrote a value to `dst` needs the clear. Jumps to the
    /// end of the expression (a guard's fallback, `&&`'s short cut, ...)
    /// land on it too. A closure call (`Op::Call`) clears it itself
    /// (`CallSite::effect`): there an extra op would cost more than the
    /// clear.
    fn discard(&mut self, dst: Reg, w: Want) {
        if w != Want::Effect {
            return;
        }
        let value = match self.ops.last() {
            Some(
                Op::Const { dst: d, .. }
                | Op::ConstD { dst: d, .. }
                | Op::ConstI { dst: d, .. }
                | Op::ConstL { dst: d, .. }
                | Op::GetVar { dst: d, .. }
                | Op::Arith { dst: d, .. }
                | Op::ArithL { dst: d, .. }
                | Op::Rel { dst: d, .. }
                | Op::RelL { dst: d, .. }
                | Op::Neg { dst: d, .. }
                | Op::Not { dst: d, .. }
                | Op::Math1 { dst: d, .. }
                | Op::Builtin1 { dst: d, .. }
                | Op::Colon { dst: d, .. }
                | Op::AndOr { dst: d, .. }
                | Op::Index { dst: d, .. }
                | Op::Index2 { dst: d, .. }
                | Op::Subset { dst: d, .. }
                | Op::Dollar { dst: d, .. }
                | Op::SlotRead { dst: d, .. }
                | Op::Missing { dst: d, .. }
                | Op::CallBuiltin { dst: d, .. }
                | Op::CallSpecial { dst: d, .. }
                | Op::Call { dst: d, .. }
                | Op::Eval { dst: d, .. },
            ) => *d == dst,
            _ => false,
        };
        if let Some(Op::Call { site, dst: d, .. }) = self.ops.last()
            && *d == dst
        {
            let site = *site as usize;
            self.call_sites[site].effect = true;
        } else if value {
            self.emit(Op::Null { dst, dead: true });
        }
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
                self.discard(dst, w);
            }
            Value::Lang(l) => {
                // The temporaries of a call are dead once its value is in `dst`.
                let top = self.top;
                self.call(l, e, dst, w);
                self.top = top;
                self.discard(dst, w);
            }
            Value::Sym(_) | Value::Promise(_) => {
                self.fallback(e, dst);
                self.discard(dst, w);
            }
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
                    self.emit(Op::Null { dst, dead: false });
                    self.vis(w, true);
                }
                // R: eval.c:do_begin -- with source references each
                // statement runs under its srcref (a block without them,
                // inside one with them, under NULL), and the value before
                // the block comes back after it (eval.c:eval). Code without
                // source references gets none of these ops.
                let refs = whole.get_attr(s::SRCREF).filter(|r| matches!(r, Value::List(_)));
                let outer = self.srcref_k;
                let tracked = refs.is_some() || outer.is_some();
                let save = (tracked && outer.is_none()).then(|| {
                    let r = self.reg();
                    self.emit(Op::SrcrefSave { dst: r });
                    r
                });
                let n = l.args.len();
                for (i, a) in l.args.iter().enumerate() {
                    if tracked {
                        let v = match refs.as_ref().map(|r| crate::interp::statement_srcref(r, i + 1)) {
                            Some(Some(v)) if v.data.len() >= 6 => Value::Int(v),
                            _ => Value::Null,
                        };
                        let k = self.konst(v);
                        self.srcref_k = Some(k);
                        self.emit(Op::Srcref { k });
                    }
                    self.expr(&a.value, dst, if i + 1 == n { w } else { Want::Effect });
                }
                self.srcref_k = outer;
                match (save, outer) {
                    (Some(r), _) => {
                        self.emit(Op::SrcrefRestore { src: r });
                    }
                    (None, Some(k)) => {
                        self.emit(Op::Srcref { k });
                    }
                    (None, None) => {}
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
            // (Only for a right-hand side without calls: one with calls runs
            // about as fast through the interpreter, without flushes.)
            "<<-"
                if l.args.len() == 2
                    && matches!(l.args[0].value, Value::Sym(_))
                    && l.args[0].tag.is_none()
                    && is_simple(&l.args[1].value, 0) =>
            {
                let target = l.args[0].value.as_sym().unwrap();
                if target.is_empty() || target == s::DOTS {
                    return self.fallback(whole, dst);
                }
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let slot = self.slot(target);
                let take = w == Want::Effect;
                self.emit(Op::SuperSet { src: dst, sym: target, slot, take });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
            }
            "<<-"
                if l.args.len() == 2
                    && l.args[0].tag.is_none()
                    && index_form(&l.args[0].value).is_some()
                    && is_simple(&l.args[1].value, 0) =>
            {
                let (x, two, i) = index_form(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let g2 = self.guard(Sym::new(if two { "[[<-" } else { "[<-" }), whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let i = self.index_operand(i);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                let take = w == Want::Effect;
                self.emit(Op::SuperSetIndex { val: dst, x, slot, i, two, k, take });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "<-" | "=" if l.args.len() == 2 && matches!(l.args[0].value, Value::Sym(_)) && l.args[0].tag.is_none() => {
                let target = l.args[0].value.as_sym().unwrap();
                if target.is_empty() || target == s::DOTS {
                    return self.fallback(whole, dst);
                }
                let g = self.guard(h, whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let slot = self.slot(target);
                let take = w == Want::Effect;
                self.emit(Op::SetVar { src: dst, sym: target, slot, take });
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
                let take = w == Want::Effect;
                self.emit(Op::SetIndex { val: dst, x, slot, i, two, k, take });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "<-" | "=" if l.args.len() == 2 && l.args[0].tag.is_none() && index2_form(&l.args[0].value).is_some() => {
                let (x, two, i, j) = index2_form(&l.args[0].value).unwrap();
                let g = self.guard(h, whole, dst);
                let g2 = self.guard(Sym::new(if two { "[[<-" } else { "[<-" }), whole, dst);
                self.expr(&l.args[1].value, dst, Want::Value);
                let i = self.index_operand(i);
                let j = self.index_operand(j);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                let take = w == Want::Effect;
                self.emit(Op::SetIndex2 { val: dst, x, slot, i, j, two, k, take });
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
                let take = w == Want::Effect;
                self.emit(Op::SetDollar { val: dst, x, slot, name, k, take });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
                self.patch(g2, end);
            }
            "<-" | "="
                if l.args.len() == 2
                    && l.args[0].tag.is_none()
                    && matches!(&l.args[0].value, Value::Lang(lhs)
                        if matches!(head(lhs).map(|s| s.name()).as_deref(), Some("[" | "[[" | "$"))
                        && lhs.args.first().and_then(|a| a.value.as_sym()).is_some_and(is_var)) =>
            {
                let Value::Lang(lhs) = &l.args[0].value else { unreachable!() };
                let target = lhs.args[0].value.as_sym().unwrap();
                let g = self.guard(h, whole, dst);
                // The helper resolves and dispatches the setter at runtime.
                let slot = self.slot(target);
                let k = self.konst(whole.clone());
                let take = w == Want::Effect;
                self.emit(Op::SetComplex { val: dst, slot, k, take });
                self.vis(w, false);
                let end = self.here();
                self.patch(g, end);
            }
            "[" | "[[" if index2_form(whole).is_some() => {
                let (x, two, i, j) = index2_form(whole).unwrap();
                let g = self.guard(h, whole, dst);
                let i = self.index_operand(i);
                let j = self.index_operand(j);
                let slot = self.slot(x);
                let k = self.konst(whole.clone());
                self.emit(Op::Index2 { dst, x, slot, i, j, two, k });
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
            "@" if l.args.len() == 2
                && l.args.iter().all(|a| a.tag.is_none())
                && !l.args[0].value.is_missing_arg()
                && l.args[0].value.as_sym() != Some(s::DOTS)
                && match &l.args[1].value {
                    Value::Sym(s) => is_var(*s),
                    Value::Str(v) => v.data.len() == 1 && v.attrs.is_none() && v.data[0].is_some(),
                    _ => false,
                } =>
            {
                let g = self.guard(h, whole, dst);
                let object = self.reg();
                self.expr(&l.args[0].value, object, Want::Value);
                let name = match &l.args[1].value {
                    Value::Sym(s) => *s,
                    Value::Str(v) => Sym::new(v.data[0].as_deref().unwrap()),
                    _ => unreachable!(),
                };
                let call = self.konst(whole.clone());
                self.emit(Op::SlotRead { dst, object, name, call });
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
                self.emit(Op::Null { dst, dead: false });
                self.vis(w, false);
                let end = self.here();
                for j in ends {
                    self.patch(j, end);
                }
                self.patch(g, end);
                let t = &mut self.switches[table as usize];
                t.names = alts.iter().map(|a| a.tag).collect();
                t.by_name =
                    (0..alts.len()).map(|k| code[k..].iter().flatten().next().copied().unwrap_or(null)).collect();
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
                    self.emit(Op::Null { dst, dead: false });
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
                let generic = if let Some((rh, args, ops)) = seq_by_form(&l.args[1].value) {
                    // (The guards evaluate the whole call when one fails.)
                    let mut gs = vec![self.guard(rh, &l.args[1].value, seq)];
                    for op in ops {
                        gs.push(self.guard(op, &l.args[1].value, seq));
                    }
                    let mut sa = [SeqArg { op: None, a: Leaf::I(0), b: Leaf::I(0) }; 3];
                    for (k, a) in args.iter().enumerate() {
                        sa[k] = match a {
                            // (`-x` as `0L - x`: the same for the integers
                            // the VM takes, anything else makes the call.)
                            Value::Lang(al) if al.args.len() == 1 => SeqArg {
                                op: Some(ArithOp::Minus),
                                a: Leaf::I(0),
                                b: self.leaf(&al.args[0].value).unwrap(),
                            },
                            Value::Lang(al) => SeqArg {
                                op: arith_op(&head(al).unwrap().name()),
                                a: self.leaf(&al.args[0].value).unwrap(),
                                b: self.leaf(&al.args[1].value).unwrap(),
                            },
                            v => SeqArg { op: None, a: self.leaf(v).unwrap(), b: Leaf::I(0) },
                        };
                    }
                    let k = self.seq_by.len() as u32;
                    self.seq_by.push(sa);
                    let kseq = self.konst(l.args[1].value.clone());
                    self.emit(Op::ForSeqBy { k, seq, call: kseq });
                    let here = self.here();
                    for g in gs {
                        self.patch(g, here);
                    }
                    false
                } else if let Some((rh, len)) = seq_length_form(&l.args[1].value) {
                    let gs = self.guard(rh, &l.args[1].value, seq);
                    let len = self.leaf(len).unwrap();
                    let kseq = self.konst(l.args[1].value.clone());
                    self.emit(Op::ForSeqLength { len, seq, call: kseq });
                    let here = self.here();
                    self.patch(gs, here);
                    false
                } else if let Some((rh, kind, a_e, b_e)) = range_form(&l.args[1].value) {
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
                    false
                } else {
                    self.expr(&l.args[1].value, seq, Want::Value);
                    true
                };
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
                // R: eval.c:bcEval ENDFOR pops the sequence: a generic one
                // (a list, a vector) is dead once the loop is done. (A
                // range or `seq_len` sequence holds no object.)
                if generic {
                    self.emit(Op::Null { dst: seq, dead: true });
                }
                self.close_loop(enter, lp, exit, step);
                if w != Want::Effect {
                    self.emit(Op::Null { dst, dead: false });
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
                    self.emit(Op::Null { dst, dead: false });
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
                    self.emit(Op::Null { dst, dead: false });
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
                    self.emit(Op::Null { dst, dead: false });
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
            // R: eval.c:R_forceAndCall: `f(...)` forcing the first `n`
            // promises of a closure first; the call is `f(...)`.
            "forceAndCall"
                if l.args.len() >= 2
                    && l.args[..2].iter().all(|a| a.tag.is_none())
                    && force_count(&l.args[0].value).is_some()
                    && matches!(&l.args[1].value, Value::Sym(f) if is_var(*f))
                    && l.args[2..].iter().all(|a| !a.value.is_missing_arg() && a.value.as_sym() != Some(s::DOTS)) =>
            {
                let g = self.guard(h, whole, dst);
                let f = l.args[1].value.as_sym().unwrap();
                let inner = Value::lang(Value::Sym(f), l.args[2..].to_vec());
                let Value::Lang(il) = &inner else { unreachable!() };
                self.call_site(il, &inner, dst, f, force_count(&l.args[0].value).unwrap());
                let end = self.here();
                self.patch(g, end);
            }
            "function"
                if (l.args.len() == 2 || l.args.len() == 3)
                    && l.args.iter().all(|a| a.tag.is_none())
                    && matches!(l.args[0].value, Value::Pairlist(_) | Value::Null) =>
            {
                let g = self.guard(h, whole, dst);
                let formals: Rc<[Arg]> = match &l.args[0].value {
                    Value::Pairlist(p) => p.items.iter().cloned().collect(),
                    _ => Rc::from(Vec::new()),
                };
                let srcref = l.args.get(2).map(|a| a.value.clone()).filter(|v| !v.is_null());
                let t = self.closures.len() as u32;
                self.closures.push(ClosureTemplate { formals, body: l.args[1].value.clone(), srcref });
                let k = self.konst(whole.clone());
                self.emit(Op::MakeClosure { dst, t, k });
                self.vis(w, true);
                let end = self.here();
                self.patch(g, end);
            }
            n if crate::builtins::lookup_primitive(n).is_some_and(|b| b.kind == BuiltinKind::Special) => {
                let b = crate::builtins::lookup_primitive(n).unwrap();
                let g = self.guard(h, whole, dst);
                let f = self.konst(Value::Builtin(b));
                let k = self.konst(whole.clone());
                self.emit(Op::CallSpecial { dst, f, k });
                let end = self.here();
                self.patch(g, end);
            }
            n if crate::builtins::lookup_primitive(n).is_none() => self.call_site(l, whole, dst, h, 0),
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
    let mut c = Compiler { thunk: true, ..Compiler::default() };
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
