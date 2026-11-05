//! Whole closures as native code.
//!
//! `crate::jit` compiles hot loops; this compiles hot *closures*, together
//! with the closures they call: a call tree whose bodies compute with
//! scalars, read vectors, and use their own local variables. Such a tree
//! cannot observe anything outside itself nor be observed while it runs (no
//! R code runs, no frame of it can be seen), so it needs no environments,
//! promises or contexts: its closures become machine functions, specialized
//! on the types of their arguments ("contextual dispatch"), that pass values
//! in registers and call each other directly.
//!
//! The same fact makes leaving native code trivial. Whenever a body meets
//! something its fast paths do not cover (NA as a condition, integer
//! overflow, a value that needs a warning, the recursion limit, or an op
//! native code does not have, which is how a rare `stop()` stays out of the
//! way), nothing observable has happened yet, so the whole tree is
//! abandoned: native code unwinds, and the VM makes the original call from
//! scratch, with all of R's semantics. There is no deoptimization metadata
//! to get wrong; results, warnings and errors are the VM's by construction.
//!
//! Variables of enclosing environments are read into a block of *cells* when
//! the tree is entered (nothing can change them while it runs but the tree
//! itself). `<<-` writes its cell; cells are written back only when the tree
//! finishes, so abandoning it still leaves no trace: each run is a small
//! transaction.
//!
//! What keeps the code valid: that the call sites still call closures with
//! these bodies, and that the functions guarded in the bodies are still
//! base's. For closures on the search path both are checked once per search
//! epoch, otherwise at each entry. Closures run here only after GNU R's JIT
//! would have compiled them (see `vm::rjit`), as on the VM's inline path.
//! Code belongs to the bytecode, so a closure created anew by each call of
//! its enclosing function shares it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, InstBuilder, MemFlagsData, Signature, StackSlotData, StackSlotKind, Type,
    Value as CValue, types,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Linkage, Module};
use smallvec::SmallVec;

use super::{
    CallArg, Gen, HELPERS, Inl, InlineCall, JType, MODULE, RegState, Region, Reject, Vars, VecView, cl_type, defs,
    new_module, reg_flow, successors, trace, types_ok, uses,
};
use crate::builtins::arith::{ArithOp, RelOp};
use crate::compile::{Builtin1, Code, InlineArg, Leaf, NativeArg, Op, RangeKind, Reg, Slot};
use crate::env::{Env, bump_r_epoch, search_epoch};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::NA_LOGICAL;
use crate::value::{Arg, Closure, Value};
use crate::vm::{RVal, call_site_fun_fast, code_for, jit_type, plain_lookup, set_plain_in_place};

/// Calls of a body before it is compiled. (Compiling costs about as much
/// as a few thousand VM calls of a small closure.)
const THRESHOLD: u32 = 512;
const NEVER: u32 = u32::MAX;
/// Attempts that may fail before the VM keeps a body, unless failures are
/// rare among all attempts (1 in [`MISS_RATIO`]).
const MAX_MISSES: u32 = 128;
const MISS_RATIO: u32 = 8;
/// Arguments of a closure in a tree.
pub const MAX_ARGS: usize = 8;
/// Closures and specializations of a call tree.
const MAX_FUNS: usize = 16;
const MAX_SPECS: usize = 32;
/// Registers and variables of a body: with the frame bound below, what the
/// machine stack needs per call.
const MAX_LOCALS: usize = 48;
const FRAME_BOUND: usize = 4096;
/// Runs that may be abandoned before the VM keeps the tree, unless they are
/// rare among all runs (1 in [`BAIL_RATIO`]).
const MAX_BAILS: u64 = 256;
const BAIL_RATIO: u64 = 16;
/// Argument and cell type combinations of a root.
const MAX_VARIANTS: usize = 6;

/// The context block: word 0 tells whether the result is visible, then
/// the cells.
const CTX_VISIBLE: i32 = 0;
const CTX_CELLS: usize = 1;
const VIEW_WORDS: usize = std::mem::size_of::<VecView>() / 8;

/// A body's native code: the calls counted, and the code once it exists.
#[derive(Default)]
pub struct NativeSlot {
    calls: Cell<u32>,
    misses: Cell<u32>,
    hits: Cell<u32>,
    entry: RefCell<Option<Rc<Entry>>>,
}

impl NativeSlot {
    /// Known never to be compiled.
    #[inline(always)]
    pub fn never(&self) -> bool {
        self.calls.get() == NEVER
    }

    /// Count a call; whether the body is hot.
    #[inline(always)]
    pub fn counted(&self) -> bool {
        let n = self.calls.get();
        if n < THRESHOLD {
            self.calls.set(n + 1);
            return false;
        }
        true
    }

    /// Whether a run may need its caller's pending assignments in the frame.
    #[inline]
    pub fn reads_frames(&self) -> bool {
        self.entry.borrow().as_ref().is_none_or(|e| !e.cells.is_empty())
    }

    /// Note how an attempt went. A body that keeps failing to run (a type
    /// that does not fit, say) must not cost every call an attempt.
    #[inline(always)]
    pub fn outcome(&self, ok: bool) {
        if ok {
            self.hits.set(self.hits.get().saturating_add(1));
            return;
        }
        let m = self.misses.get() + 1;
        self.misses.set(m);
        if m >= MAX_MISSES && u64::from(m) * u64::from(MISS_RATIO) > u64::from(self.hits.get()) {
            self.give_up();
        }
    }

    fn give_up(&self) {
        self.calls.set(NEVER);
        self.entry.borrow_mut().take();
    }
}

/// The root of a call tree.
struct Entry {
    funs: Vec<Fun>,
    /// Some body calls builtins (which print when traced).
    builtins: bool,
    /// Some cell is assigned with `<<-`.
    writes: bool,
    cells: Vec<CellDef>,
    /// Words of the context block.
    ctx_words: usize,
    /// What the tree was last found valid for.
    valid: RefCell<Option<Valid>>,
    runs: Cell<u64>,
    bails: Cell<u64>,
    variants: RefCell<Vec<Variant>>,
    /// The variant the last run used.
    last: Cell<Option<Last>>,
}

/// A variant by the types of its arguments, packed (see [`pack`]).
#[derive(Clone, Copy)]
struct Last {
    types: u64,
    /// The types of the cells (none for the fast path, see `run`).
    cells: u64,
    thunk: Thunk,
    result: JType,
}

/// Types as one number, if they fit (`MAX_ARGS` of them do).
#[inline(always)]
fn pack(tys: &[JType]) -> Option<u64> {
    (tys.len() <= MAX_ARGS).then(|| tys.iter().fold(tys.len() as u64, |a, t| (a << 7) | (*t as u64 + 1)))
}

/// A validated tree: its root, the search epoch, the versions of the
/// function bindings of the frames between its closures and the search path,
/// and its closures. (Those may be frames of running calls: not kept alive.)
struct Valid {
    root: usize,
    epoch: u64,
    versions: u32,
    insts: SmallVec<[std::rc::Weak<Closure>; 4]>,
    /// All the closures are defined on the search path: the epoch is all
    /// that can change.
    search: bool,
}

/// Changes whenever a function binding changes between `env` and the
/// search path.
#[inline]
fn chain_version(env: &Env) -> u32 {
    let mut v = 0u32;
    let mut cur = env.clone();
    while !cur.is_search() {
        v = v.wrapping_mul(31).wrapping_add(cur.fn_version()).wrapping_add(cur.id() as u32);
        cur = match cur.parent() {
            Some(p) => p,
            None => break,
        };
    }
    v
}

struct Variant {
    args: Vec<JType>,
    cells: Vec<JType>,
    code: Option<(Thunk, JType)>,
}

/// A closure body of a call tree. (What it is called from is checked by
/// the bytecode and the formals: instances differ only in environment.)
struct Fun {
    code: Rc<Code>,
    formals: Rc<[Arg]>,
    /// By formal: its slot in the code (`None`: unused).
    formal_slots: Vec<Option<Slot>>,
    /// The closures called, by call site. (A site without an entry leaves
    /// native code, see [`leaves`].)
    calls: HashMap<u32, InlineCall>,
    /// (site, function name) of those.
    sites: Vec<(u32, Sym)>,
    /// The functions the body guards.
    guards: Vec<Sym>,
    /// Variables of enclosing environments: slot to [`Entry::cells`] index.
    cells: HashMap<Slot, usize>,
}

/// A variable of an enclosing environment.
struct CellDef {
    sym: Sym,
    /// The bodies reading or writing it: it has to be the same binding.
    users: Vec<usize>,
    /// Assigned with `<<-`.
    written: bool,
    /// Offset in the context block (in words).
    off: usize,
}

/// Runs a specialization: remaining depth, context block, arguments, and
/// where to put the result; nonzero if the tree was abandoned.
type Thunk = unsafe extern "C" fn(i32, *mut u64, *const u64, *mut u64) -> i32;

/// The specializations of a call tree and what is known of their results.
#[derive(Default)]
pub struct Specs {
    list: RefCell<Vec<Spec>>,
    changed: Cell<bool>,
    /// Superassigned variables of the body being looked at, by symbol.
    written: RefCell<HashMap<Sym, JType>>,
    /// The body being looked at.
    code: RefCell<Option<Rc<Code>>>,
}

struct Spec {
    fun: usize,
    args: Vec<JType>,
    /// `None`: no path to a result found yet.
    result: Option<JType>,
}

impl Specs {
    pub(super) fn code(&self) -> Option<Rc<Code>> {
        self.code.borrow().clone()
    }

    /// The result type of `fun` for these arguments, as far as known. A
    /// combination not seen before becomes a new specialization.
    pub(super) fn result(&self, fun: usize, args: Vec<JType>) -> Option<JType> {
        let mut list = self.list.borrow_mut();
        if let Some(s) = list.iter().find(|s| s.fun == fun && s.args == args) {
            return s.result;
        }
        list.push(Spec { fun, args, result: None });
        self.changed.set(true);
        None
    }
}

/// What code generation for a closure needs beyond the loop compiler's.
pub(super) struct Whole<'a> {
    /// Abandon the tree.
    pub(super) bail: Block,
    /// Calls left before the recursion limit.
    rem: CValue,
    ctx: CValue,
    refs: &'a HashMap<(usize, Vec<JType>), FuncRef>,
    /// Superassigned variables: slot to (offset, type).
    written: HashMap<Slot, (usize, JType)>,
    /// Vectors: the variable holding the `VecView`'s address.
    views: HashMap<Slot, (Variable, JType)>,
    /// The frame's `RVal` slots (values of any type), and how many.
    base: Option<CValue>,
    nframe: usize,
    /// Offsets in the frame: registers, the temporary slots.
    dyn_regs: HashMap<Reg, i32>,
    tmp: i32,
    /// Variables of any type: the variable holding their slot's address.
    dyn_slots: HashMap<Slot, Variable>,
    /// Room for the slot addresses of a builtin call's arguments.
    argv: Option<CValue>,
    /// The body's constants.
    consts: *const Value,
    /// The result's type and, for a value of any type, the caller's slot.
    result: JType,
    ret: Option<CValue>,
}

impl Gen<'_, '_> {
    /// A direct call of the closure at `site`.
    pub(super) fn real_call(&mut self, site: u32, dst: Reg, args: &[(CValue, JType)]) -> CValue {
        let callee = self.inl.calls[&site].callee;
        let tys: Vec<JType> = args.iter().map(|a| a.1).collect();
        // (A value of any type is written to the caller's slot.)
        let ret = (self.inl.specs.unwrap().result(callee, tys.clone()) == Some(JType::Dyn)).then(|| self.dyn_reg(dst));
        let w = self.whole.as_ref().unwrap();
        let (bail, rem, ctx) = (w.bail, w.rem, w.ctx);
        let f = w.refs[&(callee, tys)];
        let left = self.b.ins().iadd_imm_s(rem, -1);
        let cont = self.b.create_block();
        self.b.ins().brif(left, cont, &[], bail, &[]);
        self.b.switch_to_block(cont);
        let mut a = Vec::with_capacity(args.len() + 2);
        a.push(left);
        a.push(ctx);
        a.extend(args.iter().map(|x| x.0));
        a.extend(ret);
        let c = self.b.ins().call(f, &a);
        let (res, failed) = {
            let r = self.b.inst_results(c);
            (r[0], r[1])
        };
        let cont = self.b.create_block();
        self.b.ins().brif(failed, bail, &[], cont, &[]);
        self.b.switch_to_block(cont);
        res
    }

    pub(super) fn return_value(&mut self, x: CValue) {
        let w = self.whole.as_ref().unwrap();
        let x = match (w.result, w.ret) {
            (JType::Dyn, Some(ret)) => {
                // (A copy: the value may be an argument, the caller's.)
                self.hcall("robigo_d_copy", &[ret, x]);
                self.b.ins().iconst(types::I64, 0)
            }
            _ => x,
        };
        self.release();
        let ok = self.i32c(0);
        self.b.ins().return_(&[x, ok]);
    }

    /// Variable `s`, if not an ordinary local: a superassigned cell is
    /// read from memory each time, a vector is its view.
    pub(super) fn whole_var(&mut self, s: Slot) -> Option<(CValue, JType)> {
        let w = self.whole.as_ref().unwrap();
        if let Some(var) = w.dyn_slots.get(&s).copied() {
            return Some((self.b.use_var(var), JType::Dyn));
        }
        if let Some((var, t)) = w.views.get(&s).copied() {
            return Some((self.b.use_var(var), t));
        }
        let (off, t) = w.written.get(&s).copied()?;
        let ctx = w.ctx;
        let x = self.b.ins().load(cl_type(t), MemFlagsData::trusted(), ctx, (off * 8) as i32);
        Some((x, t))
    }

    /// `sym <<- x`: into the cell, marked to be written back.
    pub(super) fn super_set(&mut self, slot: Slot, x: CValue) {
        let w = self.whole.as_ref().unwrap();
        let (off, _) = w.written[&slot];
        let ctx = w.ctx;
        let mf = MemFlagsData::trusted();
        self.b.ins().store(mf, x, ctx, (off * 8) as i32);
        let one = self.b.ins().iconst(types::I64, 1);
        self.b.ins().store(mf, one, ctx, ((off + 1) * 8) as i32);
    }

    pub(super) fn set_visible(&mut self, on: bool) {
        let ctx = self.whole.as_ref().unwrap().ctx;
        let v = self.b.ins().iconst(types::I64, on as i64);
        self.b.ins().store(MemFlagsData::trusted(), v, ctx, CTX_VISIBLE);
    }

    /// `length(x)`, `is.null(x)`.
    pub(super) fn builtin1(&mut self, f: Builtin1, x: CValue, t: JType, pc: usize) -> CValue {
        match f {
            Builtin1::IsNull => self.i32c(0),
            Builtin1::Length if t.is_scalar() => self.i32c(1),
            Builtin1::Length => {
                let len = self.b.ins().load(types::I64, MemFlagsData::trusted(), x, 8);
                let big = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, len, i32::MAX as i64);
                self.deopt_if(big, pc);
                self.b.ins().ireduce(types::I32, len)
            }
            _ => unreachable!(),
        }
    }
}

fn no_code(why: &str, body: &Value) -> Reject {
    if trace() {
        eprintln!("jit: closure cannot run natively: {} ({})", why, crate::deparse::deparse_one_line(body));
    }
    Reject::Never
}

/// The ops a body can have in native code.
fn native_op(op: &Op) -> bool {
    match op {
        Op::ConstD { .. }
        | Op::ConstI { .. }
        | Op::ConstL { .. }
        | Op::GetVar { .. }
        | Op::SetVar { .. }
        | Op::SuperSet { .. }
        | Op::Arith { .. }
        | Op::Rel { .. }
        | Op::ArithL { .. }
        | Op::RelL { .. }
        | Op::Neg { .. }
        | Op::Not { .. }
        | Op::Math1 { .. }
        | Op::AndOrArg { .. }
        | Op::AndOr { .. }
        | Op::AndOrShort { .. }
        | Op::ForInit { .. }
        | Op::ForStep { .. }
        | Op::Index { .. }
        | Op::Index2 { .. }
        | Op::Visible { .. }
        | Op::Guard { .. }
        | Op::LoopEnter { .. }
        | Op::LoopExit
        | Op::Jump { .. }
        | Op::JumpIf { .. }
        | Op::JumpIfNot { .. }
        | Op::Return { .. }
        | Op::Call { .. }
        | Op::Const { .. }
        | Op::Null { .. }
        | Op::Subset { .. }
        | Op::SubsetObj { .. }
        | Op::Builtin1 { .. }
        | Op::CallBuiltin { .. } => true,
        Op::ForRange { kind, .. } => *kind != RangeKind::SeqAlong,
        _ => false,
    }
}

/// Whether native code leaves the tree at `op` instead of executing it: an
/// op it does not have, or a call of something outside the tree.
pub(super) fn leaves(op: &Op, inl: &Inl) -> bool {
    match op {
        Op::Call { site, .. } => !inl.calls.contains_key(site),
        Op::SuperSet { sym, .. } => !inl.specs.is_some_and(|s| s.written.borrow().contains_key(sym)),
        // `list(...)` and `c(...)` of at most `MAX_ARGS` untagged arguments.
        Op::CallBuiltin { f, args, .. } => !inl.specs.and_then(|s| s.code()).is_some_and(|code| {
            let a = &code.arglists[*args as usize];
            matches!(&code.consts[*f as usize], Value::Builtin(b) if b.name == "list" || b.name == "c")
                && a.len() <= MAX_ARGS
                && a.iter().all(|(tag, _)| tag.is_none())
        }),
        _ => !native_op(op),
    }
}

/// The bytecode and formal slots of `clo`, if it can be in a tree at all.
fn shape(clo: &Closure) -> Option<(Rc<Code>, Vec<Option<Slot>>)> {
    if clo.jit.traced() || clo.formals.len() > MAX_ARGS {
        return None;
    }
    // (Default values would have to be evaluated in the frame; `...` would
    // need the frame's dots.)
    if clo.formals.iter().any(|f| f.tag == Some(crate::sym::s::DOTS) || f.tag.is_none() || !f.value.is_missing_arg()) {
        return None;
    }
    let code = clo.code.get_or_init(|| code_for(&clo.body, true)).clone()?;
    if !code.is_body || code.nregs + code.vars.len() > MAX_LOCALS {
        return None;
    }
    let formal_slots =
        clo.formals.iter().map(|f| code.vars.iter().position(|v| Some(*v) == f.tag).map(|p| p as Slot)).collect();
    Some((code, formal_slots))
}

/// The closures reachable from `root` through call sites that can be
/// direct calls.
fn call_tree(interp: &mut Interp, root: &Rc<Closure>) -> Result<(Vec<Fun>, Vec<CellDef>), Reject> {
    let mut clos: Vec<Rc<Closure>> = vec![root.clone()];
    let mut funs: Vec<Fun> = Vec::new();
    let mut cells: Vec<CellDef> = Vec::new();
    while funs.len() < clos.len() {
        let clo = clos[funs.len()].clone();
        let Some((code, formal_slots)) = shape(&clo) else { return Err(no_code("its shape", &clo.body)) };
        let formals: Vec<Sym> = clo.formals.iter().map(|f| f.tag.unwrap()).collect();
        let idx = funs.len();
        let mut guards: Vec<Sym> = code.guard_syms.clone();
        let mut calls = HashMap::new();
        let mut sites = Vec::new();
        for op in &code.ops {
            match op {
                Op::ArithL { sym, .. } | Op::RelL { sym, .. } | Op::Guard { sym, .. } => guards.push(*sym),
                Op::Call { f, site, .. } => {
                    let cs = &code.call_sites[*site as usize];
                    let n = cs.inline_args.as_ref().map(Vec::len).or(cs.native_args.as_ref().map(Vec::len));
                    let Some(nargs) = n else { continue };
                    if code.vars.contains(f) || cs.force != 0 {
                        continue;
                    }
                    let Some(Value::Closure(callee)) = call_site_fun_fast(interp, *f, cs, &clo.env) else { continue };
                    // (Not settled: the VM still counts its calls. This site
                    // leaves native code; it was hardly reached so far.)
                    if nargs != callee.formals.len() || callee.jit.inline_blocked() {
                        continue;
                    }
                    let Some((ccode, cslots)) = shape(&callee) else { continue };
                    let k = match clos.iter().position(|c| Rc::ptr_eq(c, &callee)) {
                        Some(i) => i,
                        None if clos.len() >= MAX_FUNS => continue,
                        None => {
                            clos.push(callee.clone());
                            clos.len() - 1
                        }
                    };
                    let args: Vec<CallArg> = match (&cs.inline_args, &cs.native_args) {
                        (Some(a), _) => a.iter().map(|a| call_arg(a, &mut guards)).collect(),
                        (None, Some(a)) => a.iter().map(|a| native_call_arg(a, &mut guards)).collect(),
                        (None, None) => unreachable!(),
                    };
                    sites.push((*site, *f));
                    calls.insert(
                        *site,
                        InlineCall {
                            code: ccode,
                            formal_slots: cslots,
                            guard_syms: Vec::new(),
                            args,
                            clo: callee,
                            callee: k,
                        },
                    );
                }
                _ => {}
            }
        }
        guards.sort();
        guards.dedup();
        // A variable named like a guarded function would be seen by the
        // VM's guard.
        if guards.iter().any(|g| code.vars.contains(g)) {
            return Err(no_code("variable named like a guarded function", &clo.body));
        }
        // Local variables, and what is left: variables of enclosing
        // environments.
        let inl = Inl::new(&calls);
        let mut assigned: Vec<Sym> = formals.clone();
        let mut read: Vec<Slot> = Vec::new();
        let mut supers: Vec<(Sym, Slot)> = Vec::new();
        for op in &code.ops {
            if !native_op(op) {
                continue;
            }
            let u = uses(op, &inl);
            assigned.extend(u.wslots.iter().map(|s| code.vars[*s as usize]));
            read.extend(u.rslots);
            if let Op::SuperSet { sym, slot, .. } = op {
                supers.push((*sym, *slot));
            }
        }
        let mut fcells = HashMap::new();
        let mut cell_for = |sym: Sym, written: bool| -> usize {
            let k = match cells.iter().position(|c| c.sym == sym) {
                Some(k) => k,
                None => {
                    cells.push(CellDef { sym, users: Vec::new(), written: false, off: 0 });
                    cells.len() - 1
                }
            };
            if !cells[k].users.contains(&idx) {
                cells[k].users.push(idx);
            }
            cells[k].written |= written;
            k
        };
        for s in read {
            let sym = code.vars[s as usize];
            if !assigned.contains(&sym) {
                fcells.insert(s, cell_for(sym, false));
            }
        }
        for (sym, slot) in supers {
            // (`<<-` of a name that is also a local stays in the VM.)
            if !assigned.contains(&sym) {
                fcells.insert(slot, cell_for(sym, true));
            }
        }
        funs.push(Fun {
            code: code.clone(),
            formals: clo.formals.clone(),
            formal_slots,
            calls,
            sites,
            guards,
            cells: fcells,
        });
    }
    // Arguments are read at the call. That is only unobservable if nothing
    // the callee does can come before: `<<-` can.
    for f in &funs {
        for ic in f.calls.values() {
            if ic.args.iter().any(|a| reads_written(a, &f.code, &cells)) {
                return Err(no_code("argument reads a variable assigned with <<-", &root.body));
            }
        }
    }
    let mut off = CTX_CELLS;
    for c in &mut cells {
        c.off = off;
        off += VIEW_WORDS.max(2);
    }
    Ok((funs, cells))
}

/// A site argument as native code evaluates it; the functions it guards go
/// to `guards`.
fn call_arg(a: &InlineArg, guards: &mut Vec<Sym>) -> CallArg {
    match a {
        InlineArg::Leaf(l) => CallArg::Leaf(*l),
        InlineArg::Arith { op, a, b, sym, .. } => {
            guards.push(*sym);
            CallArg::Arith(*op, *a, *b)
        }
        InlineArg::Index { slot, i, two, sym, .. } => {
            guards.push(*sym);
            CallArg::Index(*slot, *i, *two)
        }
    }
}

/// The same for an argument of `CallSite::native_args`.
fn native_call_arg(a: &NativeArg, guards: &mut Vec<Sym>) -> CallArg {
    match a {
        NativeArg::Plain(a) => call_arg(a, guards),
        NativeArg::Nested(n) => {
            guards.push(n.sym);
            let a = native_call_arg(&n.a, guards);
            let b = n.b.as_ref().map(|b| native_call_arg(b, guards));
            CallArg::Nested(Rc::new((n.op, a, b)))
        }
    }
}

fn written_leaf(l: &Leaf, cells: &[CellDef]) -> bool {
    matches!(l, Leaf::Var(sym, _) if cells.iter().any(|c| c.written && c.sym == *sym))
}

/// Whether an argument depends on a variable the tree assigns with `<<-`.
fn reads_written(a: &CallArg, code: &Code, cells: &[CellDef]) -> bool {
    match a {
        CallArg::Leaf(l) => written_leaf(l, cells),
        CallArg::Arith(_, x, y) => written_leaf(x, cells) || written_leaf(y, cells),
        CallArg::Index(x, i, _) => {
            written_leaf(&Leaf::Var(code.vars[*x as usize], *x), cells) || written_leaf(i, cells)
        }
        CallArg::Nested(n) => {
            reads_written(&n.1, code, cells) || n.2.as_ref().is_some_and(|b| reads_written(b, code, cells))
        }
    }
}

/// The same for an argument of the call entering the tree.
fn site_reads_written(a: &InlineArg, cells: &[CellDef]) -> bool {
    match a {
        InlineArg::Leaf(l) => written_leaf(l, cells),
        InlineArg::Arith { a, b, .. } => written_leaf(a, cells) || written_leaf(b, cells),
        InlineArg::Index { x, slot, i, .. } => written_leaf(&Leaf::Var(*x, *slot), cells) || written_leaf(i, cells),
    }
}

/// The closures of the tree of `root`, if they are what the code assumes.
fn resolve(interp: &mut Interp, e: &Entry, root: &Rc<Closure>) -> Option<Vec<Rc<Closure>>> {
    let n = e.funs.len();
    let mut insts: SmallVec<[Option<Rc<Closure>>; 4]> = SmallVec::from_elem(None, n);
    insts[0] = Some(root.clone());
    for (i, f) in e.funs.iter().enumerate() {
        let inst = insts[i].clone()?;
        if inst.jit.inline_blocked() {
            return None;
        }
        for (site, sym) in &f.sites {
            let cs = &f.code.call_sites[*site as usize];
            let Some(Value::Closure(c)) = call_site_fun_fast(interp, *sym, cs, &inst.env) else { return None };
            let k = f.calls[site].callee;
            let g = &e.funs[k];
            match c.code.get() {
                Some(Some(code)) if Rc::ptr_eq(code, &g.code) && Rc::ptr_eq(&c.formals, &g.formals) => {}
                _ => return None,
            }
            match &insts[k] {
                Some(x) if !Rc::ptr_eq(x, &c) => return None,
                Some(_) => {}
                None => insts[k] = Some(c),
            }
        }
        if !f.guards.iter().all(|s| interp.resolves_to_base(*s, &inst.env)) {
            return None;
        }
    }
    insts.into_iter().collect()
}

/// The frame binding `sym`, from `env` outwards.
fn owner(env: &Env, sym: Sym) -> Option<Env> {
    let mut cur = env.clone();
    loop {
        if cur.has_local(sym) {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Register types of a specialization of `fun`, and its variables' types.
struct Flow {
    states: Vec<Option<RegState>>,
    slot_ty: HashMap<Slot, JType>,
    result: Option<JType>,
    /// Ops that leave native code because of the types found here.
    bail: Vec<bool>,
}

fn set_written(fun: &Fun, cells: &[CellDef], cell_ty: &[JType], specs: &Specs) {
    *specs.code.borrow_mut() = Some(fun.code.clone());
    let mut written = specs.written.borrow_mut();
    written.clear();
    for k in fun.cells.values() {
        if cells[*k].written {
            written.insert(cells[*k].sym, cell_ty[*k]);
        }
    }
}

fn flow(fun: &Fun, spec_args: &[JType], cell_ty: &[JType], cells: &[CellDef], specs: &Specs) -> Result<Flow, Reject> {
    let code = &*fun.code;
    let mut inl = Inl::new(&fun.calls);
    inl.specs = Some(specs);
    let mut slot_ty: HashMap<Slot, JType> = HashMap::new();
    for (k, s) in fun.formal_slots.iter().enumerate() {
        if let Some(s) = s {
            slot_ty.insert(*s, spec_args[k]);
        }
    }
    for (s, k) in &fun.cells {
        slot_ty.insert(*s, cell_ty[*k]);
    }
    set_written(fun, cells, cell_ty, specs);
    let r = Region { start: 0, end: code.ops.len() };
    let entry: RegState = vec![None; code.nregs];
    let mut rounds = 0;
    let states = loop {
        rounds += 1;
        if rounds > 20 {
            return Err(Reject::Never);
        }
        if let Some(st) = reg_flow(&code.ops, &r, &entry, &mut slot_ty, &inl)? {
            break st;
        }
    };
    let bail = bail_ops(fun, &states, &slot_ty, &inl);
    let mut result = None;
    let mut merge = |t: Option<JType>| -> Result<(), Reject> {
        match (result, t) {
            (Some(a), Some(b)) if a != b => Err(Reject::Never),
            (None, t) => {
                result = t;
                Ok(())
            }
            _ => Ok(()),
        }
    };
    for (pc, st) in states.iter().enumerate() {
        let Some(st) = st else { continue };
        if let Some(Op::Return { src }) = code.ops.get(pc)
            && !bail[pc]
        {
            merge(st[*src as usize])?;
        }
    }
    if let Some(Some(st)) = states.last() {
        merge(st[code.result as usize])?;
    }
    if let Some(t) = result
        && !t.is_scalar()
        && t != JType::Dyn
    {
        return Err(Reject::Never);
    }
    Ok(Flow { states, slot_ty, result, bail })
}

/// Which reachable ops native code cannot run with the types found.
fn bail_ops(fun: &Fun, states: &[Option<RegState>], slot_ty: &HashMap<Slot, JType>, inl: &Inl) -> Vec<bool> {
    let code = &*fun.code;
    let mut bail = vec![false; code.ops.len()];
    for (pc, op) in code.ops.iter().enumerate() {
        let Some(st) = &states[pc] else { continue };
        if leaves(op, inl) {
            bail[pc] = true;
            continue;
        }
        let u = uses(op, inl);
        bail[pc] = u.rslots.iter().chain(&u.wslots).any(|s| !slot_ty.contains_key(s))
            || u.rregs.iter().any(|r| st[*r as usize].is_none())
            // (A cell holds one type: another means the binding changes type.)
            || matches!(op, Op::SuperSet { src, slot, .. } if st[*src as usize] != slot_ty.get(slot).copied())
            || matches!(op, Op::Index { slot, .. } if !slot_ty[slot].is_vector() && slot_ty[slot] != JType::Dyn)
            || matches!(op, Op::Index2 { slot, .. } if !slot_ty[slot].is_vector())
            // (Typed operands: these have native code only for values of any
            // type, see `dyn_op`.)
            || matches!(op, Op::Subset { x, i, .. }
                if st[*x as usize] != Some(JType::Dyn)
                    || !matches!(st[*i as usize], Some(JType::D | JType::I | JType::Dyn)))
            || matches!(op, Op::Const { dst, .. } | Op::Null { dst, dead: false } | Op::CallBuiltin { dst, .. }
                if !defs(op, st, slot_ty, inl).iter().any(|(r, t)| *r == *dst as usize && *t == Some(JType::Dyn)))
            // (Typed operands: only these builtins have native code.)
            || matches!(op, Op::Builtin1 { f, a, .. }
                if st[*a as usize] != Some(JType::Dyn) && !matches!(f, Builtin1::Length | Builtin1::IsNull))
            || matches!(op, Op::CallBuiltin { .. }
                if u.rregs.iter().any(|r| !matches!(st[*r as usize], Some(JType::D | JType::I | JType::L | JType::Dyn))))
            || !types_ok(op, st, slot_ty, inl);
    }
    // Variables are read only where they are certainly set (anything else
    // would look the name up in the enclosing environments).
    let nslots = code.vars.len();
    let mut set: Vec<Option<Vec<bool>>> = vec![None; code.ops.len() + 1];
    let mut first = vec![false; nslots];
    for s in fun.formal_slots.iter().flatten().chain(fun.cells.keys()) {
        first[*s as usize] = true;
    }
    set[0] = Some(first);
    let mut work = vec![0];
    let mut succ = Vec::new();
    while let Some(pc) = work.pop() {
        if pc >= code.ops.len() || bail[pc] {
            continue;
        }
        let mut sd = set[pc].clone().unwrap();
        let op = &code.ops[pc];
        let u = uses(op, inl);
        if u.rslots.iter().any(|s| !sd[*s as usize]) {
            bail[pc] = true;
            continue;
        }
        for s in &u.wslots {
            sd[*s as usize] = true;
        }
        if matches!(op, Op::Return { .. }) {
            continue;
        }
        successors(op, pc, &mut succ);
        for &t in &succ {
            let next = match &set[t] {
                None => Some(sd.clone()),
                Some(s0) => {
                    let s1: Vec<bool> = s0.iter().zip(&sd).map(|(a, b)| *a && *b).collect();
                    (s1 != *s0).then_some(s1)
                }
            };
            if let Some(s) = next {
                set[t] = Some(s);
                work.push(t);
            }
        }
    }
    bail
}

/// A specialization with its analysis.
struct Done {
    fun: usize,
    args: Vec<JType>,
    fl: Flow,
}

/// Find the specializations a call of `funs[0]` with `args` needs, each
/// with its types, to a fixed point (a recursive call's result is unknown
/// until a path through the body without it gave one).
fn analyse(e: &Entry, args: &[JType], cell_ty: &[JType]) -> Result<(Vec<Done>, Specs), Reject> {
    let funs = &e.funs;
    let specs = Specs::default();
    specs.list.borrow_mut().push(Spec { fun: 0, args: args.to_vec(), result: None });
    let mut rounds = 0;
    loop {
        rounds += 1;
        if rounds > 16 {
            return Err(Reject::Never);
        }
        specs.changed.set(false);
        let mut flows = Vec::new();
        let mut i = 0;
        while i < specs.list.borrow().len() {
            if i >= MAX_SPECS {
                return Err(Reject::Never);
            }
            let (fun, sargs) = {
                let l = specs.list.borrow();
                (l[i].fun, l[i].args.clone())
            };
            let fl = flow(&funs[fun], &sargs, cell_ty, &e.cells, &specs)?;
            let mut l = specs.list.borrow_mut();
            if l[i].result != fl.result {
                l[i].result = fl.result;
                specs.changed.set(true);
            }
            drop(l);
            flows.push(Done { fun, args: sargs, fl });
            i += 1;
        }
        if !specs.changed.get() {
            // A specialization without a result only ever leaves native
            // code: useless at the root, unreachable otherwise (its call
            // sites end their paths: see `reg_flow`).
            if specs.list.borrow()[0].result.is_none() {
                return Err(Reject::Never);
            }
            // Nor is code that leaves inside a loop, where the time goes.
            for d in &flows {
                let ops = &funs[d.fun].code.ops;
                for (pc, op) in ops.iter().enumerate() {
                    let Op::LoopEnter { brk, .. } = op else { continue };
                    if (pc + 1..*brk as usize).any(|q| d.fl.states[q].is_some() && d.fl.bail[q]) {
                        if trace() {
                            eprintln!("jit: closure leaves native code inside a loop");
                        }
                        return Err(Reject::Never);
                    }
                }
            }
            return Ok((flows, specs));
        }
    }
}

/// The signature of a specialization.
fn signature(module: &cranelift_jit::JITModule, args: &[JType], result: JType) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(ptr));
    for t in args {
        sig.params.push(AbiParam::new(cl_type(*t)));
    }
    // (A value of any type goes to the caller's slot.)
    if result == JType::Dyn {
        sig.params.push(AbiParam::new(ptr));
    }
    sig.returns.push(AbiParam::new(cl_type(result)));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

fn zero(b: &mut FunctionBuilder, t: JType) -> CValue {
    match t {
        JType::D => b.ins().f64const(0.0),
        t => b.ins().iconst(cl_type(t), 0),
    }
}

/// Machine code of one specialization.
#[allow(clippy::too_many_arguments)]
fn define(
    module: &mut cranelift_jit::JITModule,
    id: FuncId,
    sig: Signature,
    fun: &Fun,
    d: &Done,
    cells: &[CellDef],
    cell_ty: &[JType],
    specs: &Specs,
    ids: &HashMap<(usize, Vec<JType>), FuncId>,
    helper_ids: &[(&'static str, FuncId)],
) -> Result<(), Reject> {
    let code = &*fun.code;
    let fl = &d.fl;
    let result = fl.result.unwrap_or(JType::D);
    let tcfg = module.target_config();
    let ptr = tcfg.pointer_type();
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fbc = FunctionBuilderContext::new();
    let mut inl = Inl::new(&fun.calls);
    inl.specs = Some(specs);
    set_written(fun, cells, cell_ty, specs);
    let empty_regs = HashMap::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let mut helpers = HashMap::new();
        for (name, hid) in helper_ids {
            helpers.insert(*name, module.declare_func_in_func(*hid, b.func));
        }
        let mut refs = HashMap::new();
        for (key, fid) in ids {
            refs.insert(key.clone(), module.declare_func_in_func(*fid, b.func));
        }
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params: Vec<CValue> = b.block_params(entry).to_vec();
        let (rem, cx) = (params[0], params[1]);
        let mf = MemFlagsData::trusted();
        let mut v = Vars {
            slot: HashMap::new(),
            vec: HashMap::new(),
            dims: HashMap::new(),
            flag: HashMap::new(),
            reg: HashMap::new(),
            range: HashMap::new(),
        };
        let mut views = HashMap::new();
        let mut written = HashMap::new();
        // A vector is known by the address of its view.
        let mut add_vector = |b: &mut FunctionBuilder, v: &mut Vars, s: Slot, view: CValue, t: JType| {
            let var = b.declare_var(ptr);
            b.def_var(var, view);
            views.insert(s, (var, t));
            let pv = b.declare_var(ptr);
            let lv = b.declare_var(types::I64);
            let p = b.ins().load(ptr, mf, view, 0);
            let l = b.ins().load(types::I64, mf, view, 8);
            b.def_var(pv, p);
            b.def_var(lv, l);
            v.vec.insert(s, (pv, lv, t));
            if matches!(t, JType::MatD | JType::MatI) {
                let nr = b.declare_var(types::I64);
                let nc = b.declare_var(types::I64);
                let r = b.ins().load(types::I64, mf, view, 16);
                let c = b.ins().load(types::I64, mf, view, 24);
                b.def_var(nr, r);
                b.def_var(nc, c);
                v.dims.insert(s, (nr, nc));
            }
        };
        // Values of any type: formals (their address is the caller's slot's),
        // cells (the entry's), and the frame's own.
        let mut dyn_formals: Vec<(Slot, CValue)> = Vec::new();
        let mut dyn_cells: Vec<(Slot, usize)> = Vec::new();
        let mut dyn_locals: Vec<Slot> = Vec::new();
        let mut dyn_reg_list: Vec<Reg> = Vec::new();
        for (k, s) in fun.formal_slots.iter().enumerate() {
            if let Some(s) = s {
                let t = d.args[k];
                if t == JType::Dyn {
                    dyn_formals.push((*s, params[k + 2]));
                } else if t.is_vector() {
                    add_vector(&mut b, &mut v, *s, params[k + 2], t);
                } else {
                    let var = b.declare_var(cl_type(t));
                    b.def_var(var, params[k + 2]);
                    v.slot.insert(*s, (var, t));
                }
            }
        }
        for (s, k) in &fun.cells {
            let (t, off) = (cell_ty[*k], cells[*k].off);
            if t == JType::Dyn {
                dyn_cells.push((*s, off));
            } else if t.is_vector() {
                let view = b.ins().iadd_imm_s(cx, (off * 8) as i64);
                add_vector(&mut b, &mut v, *s, view, t);
            } else if cells[*k].written {
                written.insert(*s, (off, t));
            } else {
                // (Nothing in the tree writes it: one load per call.)
                let var = b.declare_var(cl_type(t));
                let x = b.ins().load(cl_type(t), mf, cx, (off * 8) as i32);
                b.def_var(var, x);
                v.slot.insert(*s, (var, t));
            }
        }
        for (s, t) in &fl.slot_ty {
            let known = dyn_formals.iter().any(|(f, _)| f == s) || dyn_cells.iter().any(|(c, _)| c == s);
            if *t == JType::Dyn {
                if !known {
                    dyn_locals.push(*s);
                }
                continue;
            }
            if !v.slot.contains_key(s) && !v.vec.contains_key(s) && !written.contains_key(s) {
                let var = b.declare_var(cl_type(*t));
                let z = zero(&mut b, *t);
                b.def_var(var, z);
                v.slot.insert(*s, (var, *t));
            }
        }
        let mut pairs: Vec<(Reg, JType)> = Vec::new();
        for (pc, st) in fl.states.iter().enumerate() {
            let Some(st) = st else { continue };
            for (reg, t) in st.iter().enumerate() {
                if let Some(t) = t {
                    pairs.push((reg as Reg, *t));
                }
            }
            if pc < code.ops.len() && !fl.bail[pc] {
                for (reg, t) in defs(&code.ops[pc], st, &fl.slot_ty, &inl) {
                    if let Some(t) = t {
                        pairs.push((reg as Reg, t));
                    }
                }
            }
        }
        pairs.sort_by_key(|(r, t)| (*r, *t as u8));
        pairs.dedup();
        for (reg, t) in pairs {
            if t == JType::Dyn {
                dyn_reg_list.push(reg);
                continue;
            }
            if t == JType::Range {
                if v.range.contains_key(&reg) {
                    continue;
                }
                let vs: Vec<_> = (0..3).map(|_| b.declare_var(types::I64)).collect();
                for var in &vs {
                    let z = b.ins().iconst(types::I64, 0);
                    b.def_var(*var, z);
                }
                v.range.insert(reg, (vs[0], vs[1], vs[2]));
            } else {
                let var = b.declare_var(cl_type(t));
                let z = zero(&mut b, t);
                b.def_var(var, z);
                v.reg.insert((reg, t), var);
            }
        }
        // The frame's slots: registers, locals, assigned formals, temporaries.
        let uses_dyn = !dyn_reg_list.is_empty()
            || !dyn_locals.is_empty()
            || !dyn_formals.is_empty()
            || !dyn_cells.is_empty()
            || result == JType::Dyn;
        let assigned: Vec<Slot> = code.ops.iter().flat_map(|op| uses(op, &inl).wslots).collect();
        let mut nframe = 0usize;
        let mut dyn_regs = HashMap::new();
        for r in &dyn_reg_list {
            dyn_regs.insert(*r, (nframe * RVAL) as i32);
            nframe += 1;
        }
        let mut own: Vec<(Slot, i32, Option<CValue>)> = Vec::new();
        for s in &dyn_locals {
            own.push((*s, (nframe * RVAL) as i32, None));
            nframe += 1;
        }
        let mut dyn_slots = HashMap::new();
        for (s, p) in &dyn_formals {
            if assigned.contains(s) {
                own.push((*s, (nframe * RVAL) as i32, Some(*p)));
                nframe += 1;
            } else {
                let var = b.declare_var(ptr);
                b.def_var(var, *p);
                dyn_slots.insert(*s, var);
            }
        }
        for (s, off) in &dyn_cells {
            let var = b.declare_var(ptr);
            let p = b.ins().load(ptr, mf, cx, (*off * 8) as i32);
            b.def_var(var, p);
            dyn_slots.insert(*s, var);
        }
        let tmp = (nframe * RVAL) as i32;
        if uses_dyn {
            nframe += TMP;
        }
        let (base, argv) = if uses_dyn {
            let ss =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (nframe * RVAL) as u32, 3));
            let base = b.ins().stack_addr(ptr, ss, 0);
            let n = b.ins().iconst(types::I64, nframe as i64);
            // Scalars defined so far (arguments, cells) go around the call
            // through memory, explicitly: live across it, they would be
            // spilled, and the register allocator then stores and reloads
            // them in every iteration of the loops that read them.
            let mut around: Vec<(Variable, Type)> =
                v.slot.values().filter(|(_, t)| t.is_scalar()).map(|(var, t)| (*var, cl_type(*t))).collect();
            around.sort_by_key(|(var, _)| var.as_u32());
            let keep = (!around.is_empty()).then(|| {
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (around.len() * 8) as u32, 3))
            });
            for (k, (var, _)) in around.iter().enumerate() {
                let x = b.use_var(*var);
                b.ins().stack_store(ptr, x, keep.unwrap(), (k * 8) as i32);
            }
            b.ins().call(helpers["robigo_d_init"], &[base, n]);
            for (k, (var, ty)) in around.iter().enumerate() {
                let x = b.ins().stack_load(ptr, *ty, keep.unwrap(), (k * 8) as i32);
                b.def_var(*var, x);
            }
            let sa =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (MAX_ARGS * 8) as u32, 3));
            (Some(base), Some(b.ins().stack_addr(ptr, sa, 0)))
        } else {
            (None, None)
        };
        for (s, off, from) in own {
            let p = b.ins().iadd_imm_s(base.unwrap(), off as i64);
            let var = b.declare_var(ptr);
            b.def_var(var, p);
            dyn_slots.insert(s, var);
            if let Some(from) = from {
                b.ins().call(helpers["robigo_d_copy"], &[p, from]);
            }
        }
        let ret = (result == JType::Dyn).then(|| *params.last().unwrap());
        let len = code.ops.len();
        let blocks: Vec<Block> = (0..=len).map(|_| b.create_block()).collect();
        let exit = b.create_block();
        let bail = b.create_block();
        b.set_cold_block(bail);
        b.ins().jump(blocks[0], &[]);
        let mut g = Gen {
            b,
            v,
            slots: &[],
            states: fl.states.clone(),
            inl: &inl,
            deopt_at: None,
            ret: None,
            reg_idx: &empty_regs,
            vals: rem,
            blocks,
            start: 0,
            cur: 0,
            cur_defs: Vec::new(),
            deopt: HashMap::new(),
            exit,
            helpers,
            whole: Some(Whole {
                bail,
                rem,
                ctx: cx,
                refs: &refs,
                written,
                views,
                base,
                nframe,
                dyn_regs,
                tmp,
                dyn_slots,
                argv,
                consts: code.consts.as_ptr(),
                result,
                ret,
            }),
        };
        for pc in 0..len {
            let blk = g.block(pc);
            g.b.switch_to_block(blk);
            if g.states[pc].is_none() || fl.bail[pc] {
                g.b.ins().jump(bail, &[]);
                continue;
            }
            g.cur = pc;
            g.cur_defs = defs(&code.ops[pc], g.state(pc), &fl.slot_ty, &inl);
            g.op(pc, &code.ops[pc]);
        }
        let last = g.block(len);
        g.b.switch_to_block(last);
        g.b.ins().jump(exit, &[]);
        g.b.switch_to_block(exit);
        let done = g.states[len].as_ref().and_then(|st| st[code.result as usize]);
        if done.is_some() {
            g.cur = len;
            let (x, _) = g.reg(code.result);
            g.return_value(x);
        } else {
            g.b.ins().jump(bail, &[]);
        }
        g.b.switch_to_block(bail);
        g.release();
        let z = zero(&mut g.b, result);
        let failed = g.i32c(1);
        g.b.ins().return_(&[z, failed]);
        g.b.seal_all_blocks();
        g.b.finalize(tcfg);
    }
    if let Err(e) = module.define_function(id, &mut ctx) {
        if trace() {
            eprintln!("jit: closure: codegen failed: {:?}", e);
        }
        return Err(Reject::Never);
    }
    module.clear_context(&mut ctx);
    Ok(())
}

/// The entry from Rust: unpack the arguments, call `target`, store the
/// result.
fn define_thunk(
    module: &mut cranelift_jit::JITModule,
    id: FuncId,
    sig: Signature,
    target: FuncId,
    args: &[JType],
    result: JType,
) -> Result<(), Reject> {
    let tcfg = module.target_config();
    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut fbc = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let f = module.declare_func_in_func(target, b.func);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let (rem, cx, argv, out) = {
            let p = b.block_params(entry);
            (p[0], p[1], p[2], p[3])
        };
        let mf = MemFlagsData::trusted();
        let mut a = vec![rem, cx];
        for (k, t) in args.iter().enumerate() {
            a.push(b.ins().load(cl_type(*t), mf, argv, (k * 8) as i32));
        }
        // (A value of any type is written to `out`, an `RVal`.)
        if result == JType::Dyn {
            a.push(out);
        }
        let c = b.ins().call(f, &a);
        let (res, failed) = {
            let r = b.inst_results(c);
            (r[0], r[1])
        };
        if result != JType::Dyn {
            b.ins().store(mf, res, out, 0);
        }
        b.ins().return_(&[failed]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    module.define_function(id, &mut ctx).map_err(|_| Reject::Never)?;
    module.clear_context(&mut ctx);
    Ok(())
}

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(0) };
}

fn compile(e: &Entry, args: &[JType], cell_ty: &[JType]) -> Result<(Thunk, JType), Reject> {
    let (done, specs) = analyse(e, args, cell_ty)?;
    let root_result = specs.list.borrow()[0].result.unwrap();
    MODULE.with(|m| {
        let mut m = m.borrow_mut();
        let module = m.get_or_insert_with(new_module);
        let mut helper_ids: Vec<(&'static str, FuncId)> = Vec::new();
        let all = HELPERS
            .iter()
            .map(|(n, _, p, r)| (*n, *p, Some(*r)))
            .chain(DYN_HELPERS.iter().map(|(n, _, p, r)| (*n, *p, *r)));
        for (name, params, ret) in all {
            let mut sig = module.make_signature();
            for p in params {
                sig.params.push(AbiParam::new(*p));
            }
            if let Some(ret) = ret {
                sig.returns.push(AbiParam::new(ret));
            }
            let id = module.declare_function(name, Linkage::Import, &sig).map_err(|_| Reject::Never)?;
            helper_ids.push((name, id));
        }
        let n = NEXT.with(|c| {
            let v = c.get();
            c.set(v + 1);
            v
        });
        let mut ids: HashMap<(usize, Vec<JType>), FuncId> = HashMap::new();
        let mut sigs = Vec::new();
        for (k, d) in done.iter().enumerate() {
            let sig = signature(module, &d.args, d.fl.result.unwrap_or(JType::D));
            let id = module
                .declare_function(&format!("robigo_clo_{}_{}", n, k), Linkage::Local, &sig)
                .map_err(|_| Reject::Never)?;
            ids.insert((d.fun, d.args.clone()), id);
            sigs.push(sig);
        }
        for (d, sig) in done.iter().zip(sigs) {
            let id = ids[&(d.fun, d.args.clone())];
            define(module, id, sig, &e.funs[d.fun], d, &e.cells, cell_ty, &specs, &ids, &helper_ids)?;
        }
        let ptr = module.target_config().pointer_type();
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I32));
        for _ in 0..3 {
            sig.params.push(AbiParam::new(ptr));
        }
        sig.returns.push(AbiParam::new(types::I32));
        let tid = module
            .declare_function(&format!("robigo_clo_entry_{}", n), Linkage::Local, &sig)
            .map_err(|_| Reject::Never)?;
        define_thunk(module, tid, sig, ids[&(0, args.to_vec())], args, root_result)?;
        module.finalize_definitions().map_err(|_| Reject::Never)?;
        let f = module.get_finalized_function(tid);
        // SAFETY: the function was built with the `Thunk` signature.
        Ok((unsafe { std::mem::transmute::<*const u8, Thunk>(f) }, root_result))
    })
}

/// Build the entry of the hot body `code` of `clo`.
#[cold]
#[inline(never)]
fn new_entry(interp: &mut Interp, clo: &Rc<Closure>, code: &Code) -> Option<Rc<Entry>> {
    if !super::enabled() || !crate::vm::enabled() || !crate::vm::may_inline() {
        code.native.give_up();
        return None;
    }
    if clo.jit.inline_blocked() {
        return None;
    }
    // The VM's inline path costs less than entering native code from it
    // for a body that small. (It can still be called from native code.)
    if clo.inline.get_or_init(|| crate::vm::inline_info(clo)).as_ref().is_some_and(|i| !i.has_calls) {
        code.native.give_up();
        return None;
    }
    let Ok((funs, cells)) = call_tree(interp, clo) else {
        code.native.give_up();
        return None;
    };
    let ctx_words = CTX_CELLS + cells.len() * VIEW_WORDS.max(2);
    let builtins =
        funs.iter().any(|f| f.code.ops.iter().any(|o| matches!(o, Op::CallBuiltin { .. } | Op::Builtin1 { .. })));
    let writes = cells.iter().any(|c| c.written);
    let e = Rc::new(Entry {
        funs,
        builtins,
        writes,
        cells,
        ctx_words,
        valid: RefCell::new(None),
        runs: Cell::new(0),
        bails: Cell::new(0),
        variants: RefCell::new(Vec::new()),
        last: Cell::new(None),
    });
    *code.native.entry.borrow_mut() = Some(e.clone());
    Some(e)
}

fn view_of(v: &RVal) -> Option<VecView> {
    let (ptr, len, dims) = match v {
        RVal::V(Value::Real(x)) => (x.data.as_ptr() as *mut u8, x.data.len(), crate::vm::jit_matrix_dims(x)),
        RVal::V(Value::Int(x)) => (x.data.as_ptr() as *mut u8, x.data.len(), crate::vm::jit_matrix_dims(x)),
        _ => return None,
    };
    let (nrow, ncol) = dims.unwrap_or((0, 0));
    Some(VecView { ptr, len: len as u64, nrow, ncol })
}

fn scalar_bits(v: &RVal) -> Option<u64> {
    Some(match crate::vm::scalar(v)? {
        crate::vm::Num::D(x) => x.to_bits(),
        crate::vm::Num::I(x) | crate::vm::Num::L(x) => x as u32 as u64,
    })
}

fn from_bits(t: JType, x: u64) -> RVal {
    match t {
        JType::D => RVal::D(f64::from_bits(x)),
        JType::I => RVal::I(x as u32 as i32),
        _ => RVal::L(x as u32 as i32),
    }
}

/// A call of the hot closure `clo`, with body `code`, with `args`. `None`
/// when native code did not produce the result: the call has to be made,
/// and nothing has happened yet.
#[inline(always)]
pub(crate) fn call(
    interp: &mut Interp,
    clo: &Rc<Closure>,
    code: &Code,
    args: &[RVal],
    sources: &[InlineArg],
    pending: Option<&Env>,
) -> Run {
    let mut flush = false;
    match run(interp, clo, code, args, sources, pending, &mut flush) {
        Some(v) => Run::Done(v),
        None if flush => Run::Flush,
        None => Run::No,
    }
}

/// How a call in native code went.
pub(crate) enum Run {
    Done(RVal),
    /// The call has to be made; nothing has happened yet.
    No,
    /// The caller's pending assignments have to be in its frame first.
    Flush,
}

/// Whether `target` is `env` or one of its enclosures below the search path
/// (or the global environment).
#[inline]
pub(crate) fn on_chain(env: &Env, target: &Env) -> bool {
    let mut cur = env.clone();
    loop {
        if cur.ptr_eq(target) {
            return true;
        }
        if cur.is_search() {
            return false;
        }
        cur = match cur.parent() {
            Some(p) => p,
            None => return false,
        };
    }
}

/// [`call`]; `None` with `flush` set where the caller has to flush first.
#[inline(always)]
fn run(
    interp: &mut Interp,
    clo: &Rc<Closure>,
    code: &Code,
    args: &[RVal],
    sources: &[InlineArg],
    pending: Option<&Env>,
    flush: &mut bool,
) -> Option<RVal> {
    let existing = code.native.entry.borrow().clone();
    let e = match existing {
        Some(e) => e,
        None => new_entry(interp, clo, code)?,
    };
    if e.writes && sources.iter().any(|a| site_reads_written(a, &e.cells)) {
        return None;
    }
    // The closures, cheaply if nothing changed since the last run.
    let epoch = search_epoch();
    let key = Rc::as_ptr(clo) as usize;
    let memo: Option<SmallVec<[Rc<Closure>; 4]>> = e.valid.borrow().as_ref().and_then(|v| {
        if v.root != key || v.epoch != epoch {
            return None;
        }
        // (Without cells nothing below needs the closures themselves.)
        if v.search && e.cells.is_empty() {
            return Some(SmallVec::new());
        }
        let i: Option<SmallVec<[Rc<Closure>; 4]>> = v.insts.iter().map(|w| w.upgrade()).collect();
        let i = i?;
        let versions = i.iter().fold(0u32, |a, c| a.wrapping_mul(31).wrapping_add(chain_version(&c.env)));
        (versions == v.versions).then_some(i)
    });
    let insts = match memo {
        Some(i) => i,
        None => {
            let i = resolve(interp, &e, clo)?;
            let versions = i.iter().fold(0u32, |a, c| a.wrapping_mul(31).wrapping_add(chain_version(&c.env)));
            let insts = i.iter().map(Rc::downgrade).collect();
            let search = i.iter().all(|c| c.env.is_search());
            *e.valid.borrow_mut() = Some(Valid { root: key, epoch, versions, insts, search });
            i.into_iter().collect()
        }
    };
    // The arguments and cells, and their types.
    // (A traced primitive prints when called: the VM must make the calls.)
    if e.builtins && crate::builtins::debug::any_prim_traced() {
        return None;
    }
    let mut buf = [0u64; MAX_ARGS];
    let mut views = [const { std::mem::MaybeUninit::<VecView>::uninit() }; MAX_ARGS];
    let mut tys_buf = [JType::D; MAX_ARGS];
    let tys = &mut tys_buf[..args.len()];
    for (k, v) in args.iter().enumerate() {
        let t = jit_type(v).unwrap_or(JType::Dyn);
        tys[k] = t;
        if t == JType::Dyn {
            // (`args` outlives the run.)
            buf[k] = v as *const RVal as u64;
        } else if t.is_vector() {
            buf[k] = views[k].write(view_of(v)?) as *const VecView as u64;
        } else {
            buf[k] = scalar_bits(v)?;
        }
    }
    let tys = &tys[..];
    // The common case: no cells, the same types as last time.
    if e.cells.is_empty()
        && let Some(l) = e.last.get()
        && Some(l.types) == pack(tys)
    {
        let rem = interp.native_depth();
        if rem < 2 {
            return None;
        }
        let mut ctx = [1u64];
        let mut out = 0u64;
        let mut out_any = RVal::I(0);
        let out_p: *mut u64 = if l.result == JType::Dyn { &mut out_any as *mut RVal as *mut u64 } else { &mut out };
        let runs = e.runs.get() + 1;
        e.runs.set(runs);
        // SAFETY: as for the general case below (`ctx` is the block of a tree
        // without cells).
        let failed = unsafe { (l.thunk)(rem, ctx.as_mut_ptr(), buf.as_ptr(), out_p) };
        if failed != 0 {
            bailed(&e, code, clo, runs);
            return None;
        }
        interp.visible = ctx[0] != 0;
        return Some(if l.result == JType::Dyn { out_any } else { from_bits(l.result, out) });
    }
    let mut ctx: SmallVec<[u64; 24]> = SmallVec::from_elem(0, e.ctx_words);
    ctx[0] = 1;
    // (Pending assignments of the caller's frame matter only if a cell may be
    // one of its variables.)
    if let Some(caller) = pending
        && insts.iter().any(|c| on_chain(&c.env, caller))
    {
        *flush = true;
        return None;
    }
    let mut cell_ty: SmallVec<[JType; 8]> = SmallVec::new();
    let mut held: SmallVec<[RVal; 8]> = SmallVec::new();
    // (Cells of any type are known by their address in `held`: it must not
    // grow while they are taken.)
    if e.cells.len() > held.inline_size() {
        held.reserve(e.cells.len());
    }
    for c in &e.cells {
        let env = &insts[c.users[0]].env;
        for u in &c.users[1..] {
            if !owner(env, c.sym)?.ptr_eq(&owner(&insts[*u].env, c.sym)?) {
                return None;
            }
        }
        let v = plain_lookup(env, c.sym)?;
        let t = jit_type(&v).unwrap_or(JType::Dyn);
        cell_ty.push(t);
        if t == JType::Dyn {
            // (Only read: a `<<-` of one stays in the VM.)
            if c.written {
                return None;
            }
            held.push(v);
            ctx[c.off] = held.last().unwrap() as *const RVal as u64;
        } else if t.is_vector() {
            let view = view_of(&v)?;
            // SAFETY: the block has `VIEW_WORDS.max(2)` words per cell.
            unsafe { (ctx.as_mut_ptr().add(c.off) as *mut VecView).write(view) };
            held.push(v);
        } else {
            ctx[c.off] = scalar_bits(&v)?;
        }
        if c.written {
            // Writing it back must not be able to fail.
            let o = owner(env, c.sym)?;
            if t.is_vector() || !o.plain_binding(c.sym) {
                return None;
            }
        }
    }
    let key = pack(tys).zip(pack(&cell_ty));
    let found = match e.last.get() {
        Some(l) if key == Some((l.types, l.cells)) => Some(Some((l.thunk, l.result))),
        _ => e.variants.borrow().iter().find(|v| v.args[..] == tys[..] && v.cells[..] == cell_ty[..]).map(|v| v.code),
    };
    let thunk = match found {
        Some(f) => f,
        None => {
            if e.variants.borrow().len() >= MAX_VARIANTS {
                return None;
            }
            let r = compile(&e, tys, &cell_ty).ok();
            if trace() {
                eprintln!(
                    "jit: closure {} for {:?} {:?}: {}",
                    crate::deparse::deparse_one_line(&clo.body),
                    tys,
                    cell_ty,
                    if r.is_some() { "compiled" } else { "cannot be compiled" }
                );
            }
            e.variants.borrow_mut().push(Variant { args: tys.to_vec(), cells: cell_ty.to_vec(), code: r });
            r
        }
    };
    let Some((f, result)) = thunk else {
        if e.variants.borrow().iter().all(|v| v.code.is_none()) {
            code.native.give_up();
        }
        return None;
    };
    // (Only types that fit a key: more cells than that are looked up.)
    e.last.set(key.map(|(types, cells)| Last { types, cells, thunk: f, result }));
    let rem = interp.native_depth();
    if rem < 2 {
        return None;
    }
    let mut out = 0u64;
    // (Initialized: native code assigns it, dropping what it held.)
    let mut out_any = RVal::I(0);
    let out_p: *mut u64 = if result == JType::Dyn { &mut out_any as *mut RVal as *mut u64 } else { &mut out };
    let runs = e.runs.get() + 1;
    e.runs.set(runs);
    // SAFETY: `f` was compiled for these types of arguments and cells, which
    // `buf` and `ctx` hold in the layout it expects, and it writes one value
    // to `out`. What views point to is kept alive by `args` and `held`, and
    // nothing in native code modifies it. The tree's assumptions were
    // checked above, and nothing it does can change them.
    let failed = unsafe { f(rem, ctx.as_mut_ptr(), buf.as_ptr(), out_p) };
    if failed != 0 {
        bailed(&e, code, clo, runs);
        return None;
    }
    // The transaction's end.
    let mut wrote = false;
    for (c, t) in e.cells.iter().zip(&cell_ty) {
        if c.written && ctx[c.off + 1] != 0 {
            let o = owner(&insts[c.users[0]].env, c.sym)?;
            set_plain_in_place(&o, c.sym, &mut from_bits(*t, ctx[c.off]), false);
            wrote = true;
        }
    }
    if wrote {
        bump_r_epoch();
    }
    interp.visible = ctx[0] != 0;
    Some(if result == JType::Dyn { out_any } else { from_bits(result, out) })
}

/// Count a run that was abandoned; a tree whose runs keep being abandoned
/// goes back to the VM.
#[cold]
fn bailed(e: &Entry, code: &Code, clo: &Closure, runs: u64) {
    let n = e.bails.get() + 1;
    e.bails.set(n);
    if n >= MAX_BAILS && n * BAIL_RATIO > runs {
        if trace() {
            eprintln!("jit: closure left too often: {}", crate::deparse::deparse_one_line(&clo.body));
        }
        code.native.give_up();
    }
}

/// How many nested calls native code may make from here: the recursion
/// limit, and what the machine stack has room for (each body's frame is
/// below [`FRAME_BOUND`], see [`MAX_LOCALS`]).
pub(crate) fn depth_budget(limit_left: usize, stack_left: usize) -> i32 {
    limit_left.min(stack_left / FRAME_BOUND).min(i32::MAX as usize) as i32
}

// ----------------------------------------------------------------------
// Values of any type (`JType::Dyn`). They live in `RVal` slots of the
// frame (a stack slot of each function that has some), initialized at entry
// and released at every exit, abandoning included; native code passes their
// addresses. The helpers below are the VM's fast paths for them and fail
// (nonzero) wherever the VM would take a slow one.

const ARITH: [ArithOp; 7] =
    [ArithOp::Plus, ArithOp::Minus, ArithOp::Times, ArithOp::Div, ArithOp::Pow, ArithOp::Mod, ArithOp::IDiv];
const REL: [RelOp; 6] = [RelOp::Eq, RelOp::Ne, RelOp::Lt, RelOp::Le, RelOp::Gt, RelOp::Ge];
const BUILTIN1: [Builtin1; 11] = [
    Builtin1::Length,
    Builtin1::IsNull,
    Builtin1::IsNa,
    Builtin1::IsAtomic,
    Builtin1::IsCharacter,
    Builtin1::IsLogical,
    Builtin1::IsNumeric,
    Builtin1::IsFunction,
    Builtin1::IsList,
    Builtin1::Attributes,
    Builtin1::Typeof,
];
/// Slots of a frame for values being put together (operands, arguments).
const TMP: usize = MAX_ARGS + 1;
const RVAL: usize = std::mem::size_of::<RVal>();

// SAFETY (all `h_d_*`): native code passes addresses of initialized slots of
// a live frame (or of the caller's, or the entry's arguments), each slot
// written by at most one of the pointers a helper gets.
unsafe extern "C" fn h_d_init(p: *mut RVal, n: i64) {
    for k in 0..n as usize {
        // SAFETY: see above; the frame has `n` slots.
        unsafe { p.add(k).write(RVal::I(0)) };
    }
}
unsafe extern "C" fn h_d_release(p: *mut RVal, n: i64) {
    for k in 0..n as usize {
        // SAFETY: see above; each slot is dropped once, at exit.
        unsafe { std::ptr::drop_in_place(p.add(k)) };
    }
}
unsafe extern "C" fn h_d_copy(dst: *mut RVal, src: *const RVal) {
    // SAFETY: see above.
    unsafe {
        if !std::ptr::eq(dst, src) {
            *dst = (*src).clone();
        }
    }
}
unsafe extern "C" fn h_d_move(dst: *mut RVal, src: *mut RVal) {
    // SAFETY: see above; this one writes both slots, which the check makes
    // distinct, so the two borrows do not overlap.
    unsafe {
        if !std::ptr::eq(dst, src) {
            *dst = std::mem::replace(&mut *src, crate::vm::DEAD);
        }
    }
}
unsafe extern "C" fn h_d_set_d(dst: *mut RVal, x: f64) {
    // SAFETY: see above.
    unsafe { *dst = RVal::D(x) };
}
unsafe extern "C" fn h_d_set_i(dst: *mut RVal, x: i32) {
    // SAFETY: see above.
    unsafe { *dst = RVal::I(x) };
}
unsafe extern "C" fn h_d_set_l(dst: *mut RVal, x: i32) {
    // SAFETY: see above.
    unsafe { *dst = RVal::L(x) };
}
unsafe extern "C" fn h_d_const(dst: *mut RVal, v: *const Value) {
    // SAFETY: see above; `v` is a constant of a body the entry keeps alive.
    unsafe { *dst = crate::vm::unbox((*v).clone()) };
}
unsafe extern "C" fn h_d_null(dst: *mut RVal) {
    // SAFETY: see above.
    unsafe { *dst = RVal::V(Value::Null) };
}
unsafe extern "C" fn h_d_arith(op: i32, a: *const RVal, b: *const RVal, dst: *mut RVal) -> i32 {
    // SAFETY: see above.
    let (a, b) = unsafe { (&*a, &*b) };
    let (Some(x), Some(y)) = (crate::vm::scalar(a), crate::vm::scalar(b)) else { return 1 };
    match crate::vm::arith_fast(ARITH[op as usize], x, y) {
        Some(v) => {
            // SAFETY: see above.
            unsafe { *dst = v };
            0
        }
        None => 1,
    }
}
unsafe extern "C" fn h_d_rel(op: i32, a: *const RVal, b: *const RVal, dst: *mut RVal) -> i32 {
    // SAFETY: see above.
    let (a, b) = unsafe { (&*a, &*b) };
    let (Some(x), Some(y)) = (crate::vm::scalar(a), crate::vm::scalar(b)) else { return 1 };
    let v = crate::vm::rel_fast(REL[op as usize], x, y);
    // SAFETY: see above.
    unsafe { *dst = v };
    0
}
/// A condition: 0 or 1, 2 where the VM decides (NA, a value not logical).
unsafe extern "C" fn h_d_truth(x: *const RVal) -> i32 {
    // SAFETY: see above.
    match unsafe { &*x } {
        RVal::L(v) if *v != NA_LOGICAL => (*v != 0) as i32,
        _ => 2,
    }
}
unsafe extern "C" fn h_d_index(x: *const RVal, i: f64, two: i32, dst: *mut RVal) -> i32 {
    // SAFETY: see above.
    match crate::vm::index_value(unsafe { &*x }, &RVal::D(i), two != 0) {
        Some(v) => {
            // SAFETY: see above.
            unsafe { *dst = v };
            0
        }
        None => 1,
    }
}
unsafe extern "C" fn h_d_index_d(x: *const RVal, i: *const RVal, two: i32, dst: *mut RVal) -> i32 {
    // SAFETY: see above.
    match crate::vm::index_value(unsafe { &*x }, unsafe { &*i }, two != 0) {
        Some(v) => {
            // SAFETY: see above.
            unsafe { *dst = v };
            0
        }
        None => 1,
    }
}
unsafe extern "C" fn h_d_is_object(x: *const RVal) -> i32 {
    // SAFETY: see above.
    matches!(unsafe { &*x }, RVal::V(v) if v.is_object()) as i32
}
unsafe extern "C" fn h_d_builtin1(f: i32, x: *const RVal, dst: *mut RVal) -> i32 {
    // SAFETY: see above.
    match crate::vm::builtin1_fast(BUILTIN1[f as usize], unsafe { &*x }) {
        Some(v) => {
            // SAFETY: see above.
            unsafe { *dst = v };
            0
        }
        None => 1,
    }
}
/// `list(...)` of untagged arguments: takes them, as the VM does.
unsafe extern "C" fn h_d_list(argv: *const *mut RVal, n: i64, dst: *mut RVal) {
    let mut v = Vec::with_capacity(n as usize);
    for k in 0..n as usize {
        // SAFETY: see above; `argv` holds `n` slot addresses.
        let p = unsafe { *argv.add(k) };
        // SAFETY: see above.
        v.push(std::mem::replace(unsafe { &mut *p }, RVal::I(0)).into_value());
    }
    // SAFETY: see above.
    unsafe { *dst = RVal::V(Value::list(v)) };
}
/// `c(...)` of untagged logical, integer and double scalars without
/// attributes: as bind.c combines them. Anything else fails.
unsafe extern "C" fn h_d_c(argv: *const *mut RVal, n: i64, dst: *mut RVal) -> i32 {
    let mut vals: SmallVec<[crate::vm::Num; 8]> = SmallVec::new();
    for k in 0..n as usize {
        // SAFETY: see above; `argv` holds `n` slot addresses.
        let p = unsafe { *argv.add(k) };
        // SAFETY: see above.
        match crate::vm::scalar(unsafe { &*p }) {
            Some(x) => vals.push(x),
            None => return 1,
        }
    }
    use crate::vm::Num;
    if vals.is_empty() {
        // SAFETY: see above.
        unsafe { *dst = RVal::V(Value::Null) };
        return 0;
    }
    let real = vals.iter().any(|x| matches!(x, Num::D(_)));
    let int = vals.iter().any(|x| matches!(x, Num::I(_)));
    let v = if real {
        Value::reals(
            vals.iter()
                .map(|x| match x {
                    Num::D(d) => *d,
                    Num::I(i) | Num::L(i) => crate::coerce::int_to_real(*i),
                })
                .collect(),
        )
    } else if int {
        Value::ints(
            vals.iter()
                .map(|x| match x {
                    Num::I(i) | Num::L(i) => *i,
                    Num::D(_) => unreachable!(),
                })
                .collect(),
        )
    } else {
        Value::lgls(
            vals.iter()
                .map(|x| match x {
                    Num::L(i) | Num::I(i) => *i,
                    Num::D(_) => unreachable!(),
                })
                .collect(),
        )
    };
    // SAFETY: see above.
    unsafe { *dst = RVal::V(v) };
    0
}

/// Native-code helpers for values of any type, as `crate::jit::HELPERS`.
pub(super) const DYN_HELPERS: &[(&str, *const u8, &[Type], Option<Type>)] = &[
    ("robigo_d_init", h_d_init as *const u8, &[types::I64, types::I64], None),
    ("robigo_d_release", h_d_release as *const u8, &[types::I64, types::I64], None),
    ("robigo_d_copy", h_d_copy as *const u8, &[types::I64, types::I64], None),
    ("robigo_d_move", h_d_move as *const u8, &[types::I64, types::I64], None),
    ("robigo_d_set_d", h_d_set_d as *const u8, &[types::I64, types::F64], None),
    ("robigo_d_set_i", h_d_set_i as *const u8, &[types::I64, types::I32], None),
    ("robigo_d_set_l", h_d_set_l as *const u8, &[types::I64, types::I32], None),
    ("robigo_d_const", h_d_const as *const u8, &[types::I64, types::I64], None),
    ("robigo_d_null", h_d_null as *const u8, &[types::I64], None),
    ("robigo_d_arith", h_d_arith as *const u8, &[types::I32, types::I64, types::I64, types::I64], Some(types::I32)),
    ("robigo_d_rel", h_d_rel as *const u8, &[types::I32, types::I64, types::I64, types::I64], Some(types::I32)),
    ("robigo_d_truth", h_d_truth as *const u8, &[types::I64], Some(types::I32)),
    ("robigo_d_index", h_d_index as *const u8, &[types::I64, types::F64, types::I32, types::I64], Some(types::I32)),
    ("robigo_d_index_d", h_d_index_d as *const u8, &[types::I64, types::I64, types::I32, types::I64], Some(types::I32)),
    ("robigo_d_is_object", h_d_is_object as *const u8, &[types::I64], Some(types::I32)),
    ("robigo_d_builtin1", h_d_builtin1 as *const u8, &[types::I32, types::I64, types::I64], Some(types::I32)),
    ("robigo_d_list", h_d_list as *const u8, &[types::I64, types::I64, types::I64], None),
    ("robigo_d_c", h_d_c as *const u8, &[types::I64, types::I64, types::I64], Some(types::I32)),
];

/// Register types of the ops that compute with values of any type.
pub(super) fn dyn_defs(
    op: &Op,
    st: &RegState,
    slot_ty: &HashMap<Slot, JType>,
    _inl: &Inl,
) -> Option<Vec<(usize, Option<JType>)>> {
    let rt = |r: &Reg| st[*r as usize];
    let lt = |l: &Leaf| super::leaf_type(l, st, slot_ty);
    let num = |t: Option<JType>| matches!(t, Some(JType::D | JType::I | JType::L | JType::Dyn));
    let some_dyn =
        |a: Option<JType>, b: Option<JType>| (a == Some(JType::Dyn) || b == Some(JType::Dyn)) && num(a) && num(b);
    let d = |dst: &Reg| Some(vec![(*dst as usize, Some(JType::Dyn))]);
    match op {
        Op::GetVar { dst, slot, .. } if slot_ty.get(slot) == Some(&JType::Dyn) => d(dst),
        Op::Const { dst, .. } | Op::Null { dst, dead: false } | Op::CallBuiltin { dst, .. } => d(dst),
        // The clear of a dead register: it has no value (see `dyn_op`).
        Op::Null { dst, dead: true } => Some(vec![(*dst as usize, None)]),
        Op::Arith { dst, a, b, .. } | Op::Rel { dst, a, b, .. } if some_dyn(rt(a), rt(b)) => d(dst),
        Op::ArithL { dst, a, b, .. } | Op::RelL { dst, a, b, .. } if some_dyn(lt(a), lt(b)) => d(dst),
        Op::Index { dst, slot, i, .. }
            if slot_ty.get(slot) == Some(&JType::Dyn) && matches!(lt(i), Some(JType::D | JType::I)) =>
        {
            d(dst)
        }
        // (A logical subscript selects: `index_value` has no fast path for it.)
        Op::Subset { dst, x, i, .. }
            if rt(x) == Some(JType::Dyn) && matches!(rt(i), Some(JType::D | JType::I | JType::Dyn)) =>
        {
            d(dst)
        }
        Op::Builtin1 { dst, a, .. } if rt(a) == Some(JType::Dyn) => d(dst),
        _ => None,
    }
}

impl Gen<'_, '_> {
    fn hcall(&mut self, name: &'static str, args: &[CValue]) -> Option<CValue> {
        let f = self.helpers[name];
        let c = self.b.ins().call(f, args);
        self.b.inst_results(c).first().copied()
    }

    /// The slot of a register holding a value of any type.
    pub(super) fn dyn_reg(&mut self, r: Reg) -> CValue {
        let w = self.whole.as_ref().unwrap();
        let (base, off) = (w.base.unwrap(), w.dyn_regs[&r]);
        self.b.ins().iadd_imm_s(base, off as i64)
    }

    fn tmp(&mut self, k: usize) -> CValue {
        let w = self.whole.as_ref().unwrap();
        let (base, off) = (w.base.unwrap(), w.tmp + (k * RVAL) as i32);
        self.b.ins().iadd_imm_s(base, off as i64)
    }

    /// A value as the address of a slot holding it: a typed scalar is put
    /// into temporary slot `k`.
    fn as_slot(&mut self, (x, t): (CValue, JType), k: usize) -> CValue {
        if t == JType::Dyn {
            return x;
        }
        let p = self.tmp(k);
        let name = match t {
            JType::D => "robigo_d_set_d",
            JType::I => "robigo_d_set_i",
            _ => "robigo_d_set_l",
        };
        self.hcall(name, &[p, x]);
        p
    }

    /// Release the frame's values (before any exit).
    pub(super) fn release(&mut self) {
        let w = self.whole.as_ref().unwrap();
        if let Some(base) = w.base {
            let n = self.b.ins().iconst(types::I64, w.nframe as i64);
            self.hcall("robigo_d_release", &[base, n]);
        }
    }

    fn failed(&mut self, r: CValue, pc: usize) {
        let bad = self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
        self.deopt_if(bad, pc);
    }

    /// An argument `x[[i]]` / `x[i]` of call argument `k`.
    pub(super) fn index_arg(&mut self, k: usize, x: Slot, i: &Leaf, two: bool, pc: usize) -> (CValue, JType) {
        let iv = self.leaf(i);
        let (xv, t) = self.var(x);
        if t == JType::Dyn {
            let f = self.to_f64(iv.0, iv.1);
            let dst = self.tmp(k);
            let two = self.i32c(two as i64);
            let r = self.hcall("robigo_d_index", &[xv, f, two, dst]).unwrap();
            self.failed(r, pc);
            return (dst, JType::Dyn);
        }
        let (pv, lv, vt) = self.v.vec[&x];
        let len = self.b.use_var(lv);
        let k0 = self.index(iv, len, pc);
        let p = self.b.use_var(pv);
        let (sz, ty, rt) = if vt == JType::VecD { (8, types::F64, JType::D) } else { (4, types::I32, JType::I) };
        let off = self.b.ins().imul_imm_s(k0, sz);
        let addr = self.b.ins().iadd(p, off);
        (self.b.ins().load(ty, MemFlagsData::trusted(), addr, 0), rt)
    }

    /// The ops that compute with values of any type. `None`: not one;
    /// otherwise whether control falls through to the next op.
    pub(super) fn dyn_op(&mut self, pc: usize, op: &Op) -> Option<bool> {
        let st = self.state(pc).clone();
        let rt = |r: &Reg| st[*r as usize];
        let dy = Some(JType::Dyn);
        let slot_dyn = |g: &Self, s: &Slot| g.whole.as_ref().is_some_and(|w| w.dyn_slots.contains_key(s));
        match op {
            Op::GetVar { dst, slot, .. } if slot_dyn(self, slot) => {
                let (src, _) = self.var(*slot);
                let d = self.dyn_reg(*dst);
                self.hcall("robigo_d_copy", &[d, src]);
            }
            Op::SetVar { src, slot, take, .. } if rt(src) == dy => {
                let (s, _) = self.reg(*src);
                let (d, _) = self.var(*slot);
                // (In effect position the register gives its value up, as
                // the VM's SetVar does: #357.)
                self.hcall(if *take { "robigo_d_move" } else { "robigo_d_copy" }, &[d, s]);
            }
            Op::Const { dst, k } => {
                let consts = self.whole.as_ref().unwrap().consts;
                let p = self
                    .b
                    .ins()
                    .iconst(types::I64, consts as i64 + (*k as usize * std::mem::size_of::<Value>()) as i64);
                let d = self.dyn_reg(*dst);
                self.hcall("robigo_d_const", &[d, p]);
            }
            Op::Null { dst, dead: false } => {
                let d = self.dyn_reg(*dst);
                self.hcall("robigo_d_null", &[d]);
            }
            // The clear of a dead register (#357): nothing to do, as native
            // code runs no R code (no gc() can see the frame: native code
            // leaves first, and leaving releases the frame).
            Op::Null { dead: true, .. } => {}
            Op::Arith { op: o, dst, a, b, .. } if rt(a) == dy || rt(b) == dy => {
                let (x, y) = (self.reg(*a), self.reg(*b));
                self.dyn_arith(true, ARITH.iter().position(|v| v == o).unwrap(), x, y, *dst, pc);
            }
            Op::Rel { op: o, dst, a, b, .. } if rt(a) == dy || rt(b) == dy => {
                let (x, y) = (self.reg(*a), self.reg(*b));
                self.dyn_arith(false, REL.iter().position(|v| v == o).unwrap(), x, y, *dst, pc);
            }
            Op::ArithL { op: o, dst, a, b, .. } => {
                let (x, y) = (self.leaf(a), self.leaf(b));
                if x.1 != JType::Dyn && y.1 != JType::Dyn {
                    return None;
                }
                self.dyn_arith(true, ARITH.iter().position(|v| v == o).unwrap(), x, y, *dst, pc);
            }
            Op::RelL { op: o, dst, a, b, .. } => {
                let (x, y) = (self.leaf(a), self.leaf(b));
                if x.1 != JType::Dyn && y.1 != JType::Dyn {
                    return None;
                }
                self.dyn_arith(false, REL.iter().position(|v| v == o).unwrap(), x, y, *dst, pc);
            }
            Op::JumpIf { cond, target, .. } | Op::JumpIfNot { cond, target, .. } if rt(cond) == dy => {
                let (c, _) = self.reg(*cond);
                let v = self.hcall("robigo_d_truth", &[c]).unwrap();
                let na = self.b.ins().icmp_imm_s(IntCC::Equal, v, 2);
                self.deopt_if(na, pc);
                let t = self.target(*target as usize);
                let n = self.target(pc + 1);
                if matches!(op, Op::JumpIf { .. }) {
                    self.b.ins().brif(v, t, &[], n, &[]);
                } else {
                    self.b.ins().brif(v, n, &[], t, &[]);
                }
                return Some(false);
            }
            Op::Index { dst, slot, i, two, .. } if slot_dyn(self, slot) => {
                let iv = self.leaf(i);
                let f = self.to_f64(iv.0, iv.1);
                let (x, _) = self.var(*slot);
                let d = self.dyn_reg(*dst);
                let two = self.i32c(*two as i64);
                let r = self.hcall("robigo_d_index", &[x, f, two, d]).unwrap();
                self.failed(r, pc);
            }
            Op::Subset { dst, x, i, two, .. } if rt(x) == dy => {
                let (xv, _) = self.reg(*x);
                let iv = self.reg(*i);
                let d = self.dyn_reg(*dst);
                let two = self.i32c(*two as i64);
                let r = if iv.1 == JType::Dyn {
                    self.hcall("robigo_d_index_d", &[xv, iv.0, two, d]).unwrap()
                } else {
                    let f = self.to_f64(iv.0, iv.1);
                    self.hcall("robigo_d_index", &[xv, f, two, d]).unwrap()
                };
                self.failed(r, pc);
            }
            Op::SubsetObj { x, .. } => {
                // (Values of other types are never objects.)
                if rt(x) == dy {
                    let (xv, _) = self.reg(*x);
                    let r = self.hcall("robigo_d_is_object", &[xv]).unwrap();
                    self.failed(r, pc);
                }
            }
            Op::Builtin1 { f, dst, a, .. } if rt(a) == dy => {
                let (x, _) = self.reg(*a);
                let d = self.dyn_reg(*dst);
                let fi = self.i32c(BUILTIN1.iter().position(|v| v == f).unwrap() as i64);
                let r = self.hcall("robigo_d_builtin1", &[fi, x, d]).unwrap();
                self.failed(r, pc);
            }
            Op::CallBuiltin { dst, f, args, .. } => {
                let code = self.inl.specs.and_then(|s| s.code()).unwrap();
                let is_list = matches!(&code.consts[*f as usize], Value::Builtin(b) if b.name == "list");
                let regs: Vec<Reg> = code.arglists[*args as usize].iter().map(|(_, r)| *r).collect();
                let argv = self.whole.as_ref().unwrap().argv.unwrap();
                for (k, r) in regs.iter().enumerate() {
                    let v = self.reg(*r);
                    let p = self.as_slot(v, k);
                    self.b.ins().store(MemFlagsData::trusted(), p, argv, (k * 8) as i32);
                }
                let n = self.b.ins().iconst(types::I64, regs.len() as i64);
                let d = self.dyn_reg(*dst);
                if is_list {
                    self.hcall("robigo_d_list", &[argv, n, d]);
                } else {
                    let r = self.hcall("robigo_d_c", &[argv, n, d]).unwrap();
                    self.failed(r, pc);
                }
            }
            Op::Return { src } if rt(src) == dy => {
                let (x, _) = self.reg(*src);
                self.return_value(x);
                return Some(false);
            }
            _ => return None,
        }
        Some(true)
    }

    fn dyn_arith(&mut self, arith: bool, k: usize, a: (CValue, JType), b: (CValue, JType), dst: Reg, pc: usize) {
        let pa = self.as_slot(a, 0);
        let pb = self.as_slot(b, 1);
        let d = self.dyn_reg(dst);
        let o = self.i32c(k as i64);
        let r = self.hcall(if arith { "robigo_d_arith" } else { "robigo_d_rel" }, &[o, pa, pb, d]).unwrap();
        self.failed(r, pc);
    }
}
