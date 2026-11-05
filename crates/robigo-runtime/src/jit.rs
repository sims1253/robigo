//! Native code for hot loops.
//!
//! A loop of compiled code (the ops between a `LoopEnter` and its
//! `LoopExit`) whose ops all have scalar fast paths (arithmetic,
//! comparisons, `&&`/`||`, `if`, `for` over ranges, element reads and
//! writes of double and integer vectors, ...) is compiled by Cranelift with
//! its variables and registers in machine registers. The VM enters the
//! native code at the loop head (see `vm::jit_enter`) after checking that
//! every guarded function still resolves to base and that the variables
//! have the types the code was compiled for; nothing in the loop can run R
//! code, so these hold throughout.
//!
//! Whenever an op meets a case its fast path does not cover (integer
//! overflow, an NA condition, an index out of range, ...), the native code
//! stores the variables and registers back and returns that op's index: the
//! VM then executes it, with all of R's semantics, and continues. Results
//! are therefore those of the VM, which are those of the interpreter.

use std::cell::RefCell;
use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, MemFlagsData, Type, Value as CValue, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};

use crate::builtins::arith::{ArithOp, RelOp, myfloor, myfmod, r_pow};
use crate::compile::{Builtin1, Code, Leaf, Math1, NO_GUARD, Op, RangeKind, Reg, Slot};
use crate::sym::Sym;
use crate::value::{NA_INTEGER, NA_LOGICAL, NA_REAL, Value};

pub mod closure;

/// The type of a variable or register in native code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum JType {
    /// A double scalar (f64).
    D,
    /// An integer scalar (i32, `NA_INTEGER` for NA).
    I,
    /// A logical scalar (i32: 0, 1 or `NA_LOGICAL`).
    L,
    /// A `for` sequence `start, start + step, ...` of length `n` (three i64).
    Range,
    /// A double vector without attributes, read and written in place.
    VecD,
    /// An integer vector without attributes, read and written in place.
    VecI,
    /// A matrix with only a dim attribute.
    MatD,
    MatI,
    /// Any value, held in an `RVal` slot of the frame (in a closure compiled
    /// whole): native code knows it by the slot's address.
    Dyn,
}

impl JType {
    pub fn is_scalar(self) -> bool {
        matches!(self, JType::D | JType::I | JType::L)
    }

    pub fn is_vector(self) -> bool {
        matches!(self, JType::VecD | JType::VecI | JType::MatD | JType::MatI)
    }
}

/// A variable the native code reads or writes.
pub struct SlotInfo {
    pub slot: Slot,
    pub sym: Sym,
    pub ty: JType,
    /// Index into the value buffer (scalars) or the vector buffer.
    pub idx: usize,
    /// Read before it is written: its value at entry is needed.
    pub needs_entry: bool,
    /// Assigned by the loop.
    pub written: bool,
    /// For written scalars without an entry value: index of a flag set once
    /// the loop has assigned it (only then is it written back).
    pub flag: Option<usize>,
    /// For vectors: written in place.
    pub writable: bool,
    /// For vectors: assigned by the loop, so held in a buffer of the run's
    /// [`LoopCtx`] (copied from the value at entry if `needs_entry`) and
    /// written back as a new vector.
    pub owned: bool,
}

/// A view of a vector's data passed to native code.
#[repr(C)]
pub struct VecView {
    pub ptr: *mut u8,
    pub len: u64,
    pub nrow: u64,
    pub ncol: u64,
}

/// Returned by native code when the loop finished normally.
pub const EXIT: u32 = u32::MAX;

/// A compiled loop. The caller passes the register and slot buffer of
/// [`JitLoop`]'s layout, one view per vector the loop was compiled for (the
/// machine code indexes both without checks) and the [`LoopCtx`].
type LoopFn = unsafe extern "C" fn(*mut u64, *mut VecView, *mut LoopCtx) -> u32;

/// What a running loop owns: the storage of the vectors it assigns to
/// variables (see [`SlotInfo::owned`]). Native code holds a view of each
/// buffer, whose `nrow` is the capacity and `ncol` the buffer's index.
#[derive(Default)]
pub struct LoopCtx {
    pub bufs: Vec<Buf>,
    /// Buffers replaced by larger ones, kept until the run ends: a register
    /// that viewed one stays readable (only a dead one can).
    old: Vec<Buf>,
}

pub enum Buf {
    I(Vec<i32>),
    D(Vec<f64>),
}

/// Room for `n` elements in the buffer `view` stands for: updates the view's
/// pointer and capacity, and returns the pointer.
unsafe extern "C" fn h_reserve(ctx: *mut LoopCtx, view: *mut VecView, n: i64) -> *mut u8 {
    // SAFETY: native code passes the context of its run and one of the
    // views of that run's owned buffers, which nothing else references.
    let (ctx, view) = unsafe { (&mut *ctx, &mut *view) };
    let n = n.max(0) as usize;
    // (The callers write all `n` elements: nothing is copied over.)
    let (ptr, cap) = match &mut ctx.bufs[view.ncol as usize] {
        Buf::I(v) => {
            if v.len() < n {
                let old = std::mem::replace(v, vec![0; n.max(2 * v.len())]);
                ctx.old.push(Buf::I(old));
            }
            (v.as_mut_ptr() as *mut u8, v.len())
        }
        Buf::D(v) => {
            if v.len() < n {
                let old = std::mem::replace(v, vec![0.0; n.max(2 * v.len())]);
                ctx.old.push(Buf::D(old));
            }
            (v.as_mut_ptr() as *mut u8, v.len())
        }
    };
    view.ptr = ptr;
    view.nrow = cap as u64;
    ptr
}

/// A compiled loop.
pub struct JitLoop {
    pub func: LoopFn,
    pub slots: Vec<SlotInfo>,
    /// Index of each register in the value buffer (three consecutive
    /// values for a range).
    pub reg_idx: HashMap<Reg, usize>,
    /// Registers set before the loop (a `for` sequence and its index).
    pub live_in: Vec<(Reg, JType)>,
    /// The registers live at each op where native code leaves, and their
    /// types, to restore for the VM.
    pub deopt_regs: HashMap<u32, Vec<(Reg, JType)>>,
    /// The registers the loop clears (`Op::Null`, which has no native
    /// code): the VM clears those not live where native code leaves, so
    /// they do not keep a value from before the loop alive (#357).
    pub nulled: Vec<Reg>,
    pub nvals: usize,
    pub nvecs: usize,
    /// (LoopEnter index, break target, next target) of loops inside the
    /// region, outermost first (not the region's own loop).
    pub nested: Vec<(u32, u32, u32)>,
    /// The functions the loop's guards check, with the index of a guarding
    /// op (for its memo).
    pub guards: Vec<(Sym, usize)>,
    /// The closures run inline (to check that their call sites still call
    /// them) and the functions their bodies guard.
    pub calls: Vec<(u32, std::rc::Rc<crate::value::Closure>, Vec<Sym>)>,
    /// The ops native code does not have, where it always leaves (sorted).
    pub leaves: Vec<u32>,
    /// With `leaves`: the value-buffer index where native code counts the
    /// back edges it took, and the leaves and back edges of all runs (a
    /// loop that keeps leaving early is given up: see `vm::jit_enter`).
    pub counter: Option<usize>,
    pub stats: std::cell::Cell<(u64, u64)>,
    /// Computes builtins natively (`max`, `min`): not while one is traced.
    pub builtins: bool,
    /// The value-buffer index of the visibility flag: in at entry, out at every
    /// exit (a `return` the VM makes after leaving returns a value as visible
    /// as the VM would have left it).
    pub vis: usize,
}

/// A call of a pure closure (see `vm::inline_info`) without free
/// variables, run inline in native code.
pub struct InlineCall {
    pub code: std::rc::Rc<Code>,
    /// The slot of each formal in the callee's code.
    pub formal_slots: Vec<Option<Slot>>,
    pub guard_syms: Vec<Sym>,
    /// The arguments.
    pub args: Vec<CallArg>,
    pub clo: std::rc::Rc<crate::value::Closure>,
    /// In a closure compiled whole ([`closure`]): the callee's index in its
    /// call tree.
    pub callee: usize,
}

/// An argument of a call in native code: evaluated by the caller, which is
/// only unobservable because of what it is (it has no effects and a bounded
/// cost; whatever would be observable leaves native code instead).
#[derive(Clone)]
pub enum CallArg {
    Leaf(Leaf),
    Arith(ArithOp, Leaf, Leaf),
    /// `x[[i]]` (`two`) or `x[i]` of variable slot `x`.
    Index(Slot, Leaf, bool),
    /// `compile::NestedArith`: `a op b`, or `-a` without `b`.
    Nested(std::rc::Rc<(ArithOp, CallArg, Option<CallArg>)>),
}

impl CallArg {
    fn ty(&self, st: &RegState, slot_ty: &HashMap<Slot, JType>) -> Option<JType> {
        match self {
            CallArg::Leaf(l) => leaf_type(l, st, slot_ty),
            CallArg::Arith(op, a, b) => arith_type(*op, leaf_type(a, st, slot_ty)?, leaf_type(b, st, slot_ty)?),
            CallArg::Nested(n) => {
                let a = n.1.ty(st, slot_ty)?;
                match &n.2 {
                    Some(b) => arith_type(n.0, a, b.ty(st, slot_ty)?),
                    None => match a {
                        JType::D => Some(JType::D),
                        JType::I | JType::L => Some(JType::I),
                        _ => None,
                    },
                }
            }
            CallArg::Index(x, i, _) => {
                if !matches!(leaf_type(i, st, slot_ty)?, JType::D | JType::I) {
                    return None;
                }
                match slot_ty.get(x)? {
                    JType::VecD => Some(JType::D),
                    JType::VecI => Some(JType::I),
                    JType::Dyn => Some(JType::Dyn),
                    _ => None,
                }
            }
        }
    }
}

/// Register types of an inlined callee for given argument types.
struct CalleeTy {
    states: Vec<Option<RegState>>,
    slot_ty: HashMap<Slot, JType>,
    result: JType,
}

/// The calls run inline (by call site) and their analyses.
pub struct Inl<'a> {
    calls: &'a HashMap<u32, InlineCall>,
    memo: RefCell<HashMap<(u32, Vec<JType>), Option<std::rc::Rc<CalleeTy>>>>,
    /// Compiling a closure whole: calls are real calls, whose result types
    /// come from here.
    specs: Option<&'a closure::Specs>,
    /// In a loop, by op index: the ops native code leaves at.
    leave: Vec<bool>,
    /// In a loop, by argument list: the `max` (`true`) and `min` calls of
    /// scalars native code computes, and their argument registers.
    minmax: HashMap<u32, (bool, Vec<Reg>)>,
}

impl<'a> Inl<'a> {
    pub fn new(calls: &'a HashMap<u32, InlineCall>) -> Inl<'a> {
        Inl { calls, memo: RefCell::new(HashMap::new()), specs: None, leave: Vec::new(), minmax: HashMap::new() }
    }

    /// Whether native code leaves at op `pc` of the loop.
    fn leaves_at(&self, pc: usize) -> bool {
        self.leave.get(pc).copied().unwrap_or(false)
    }

    /// The type of the result of the call at `site` with these arguments.
    fn result(&self, site: u32, args: Vec<JType>) -> Option<JType> {
        match self.specs {
            Some(s) => s.result(self.calls.get(&site)?.callee, args),
            None => self.callee(site, args).map(|c| c.result),
        }
    }

    fn callee(&self, site: u32, args: Vec<JType>) -> Option<std::rc::Rc<CalleeTy>> {
        let key = (site, args);
        if let Some(r) = self.memo.borrow().get(&key) {
            return r.clone();
        }
        let r = self.calls.get(&site).and_then(|ic| callee_flow(ic, &key.1)).map(std::rc::Rc::new);
        self.memo.borrow_mut().insert(key, r.clone());
        r
    }
}

/// Register types through an inlined callee's body, whose formals have the
/// argument types; `None` if native code cannot run it.
fn callee_flow(ic: &InlineCall, args: &[JType]) -> Option<CalleeTy> {
    let code = &*ic.code;
    let mut slot_ty = HashMap::new();
    for (k, s) in ic.formal_slots.iter().enumerate() {
        if let Some(s) = s {
            slot_ty.insert(*s, *args.get(k)?);
        }
    }
    let empty = HashMap::new();
    let inl = Inl::new(&empty);
    let len = code.ops.len();
    let mut states: Vec<Option<RegState>> = vec![None; len + 1];
    states[0] = Some(vec![None; code.nregs]);
    let mut result: Option<JType> = None;
    let mut merge_result = |t: Option<JType>| -> Option<()> {
        let t = t?;
        match result {
            Some(r) if r != t => None,
            _ => {
                result = Some(t);
                Some(())
            }
        }
    };
    let mut work = vec![0usize];
    let mut succ = Vec::new();
    let mut steps = 0;
    while let Some(pc) = work.pop() {
        steps += 1;
        if steps > 10_000 {
            return None;
        }
        let st = states[pc].clone().unwrap();
        if pc == len {
            merge_result(st[code.result as usize])?;
            continue;
        }
        let op = &code.ops[pc];
        match op {
            Op::ConstD { .. }
            | Op::ConstI { .. }
            | Op::ConstL { .. }
            | Op::GetVar { .. }
            | Op::Arith { .. }
            | Op::Rel { .. }
            | Op::ArithL { .. }
            | Op::RelL { .. }
            | Op::Neg { .. }
            | Op::Not { .. }
            | Op::Math1 { .. }
            | Op::Jump { .. }
            | Op::Visible { .. }
            | Op::Guard { .. } => {}
            Op::JumpIf { cond, .. } | Op::JumpIfNot { cond, .. } => {
                if st[*cond as usize] != Some(JType::L) {
                    return None;
                }
            }
            Op::Return { src } => {
                merge_result(st[*src as usize])?;
                continue;
            }
            _ => return None,
        }
        for reg in uses(op, &inl).rregs {
            st[reg as usize]?;
        }
        let mut out = st.clone();
        for (reg, t) in defs(op, &st, &slot_ty, &inl) {
            // (Inline bodies compute with scalars.)
            out[reg] = Some(t.filter(|t| t.is_scalar())?);
        }
        successors(op, pc, &mut succ);
        for &t in &succ {
            if t > len || t <= pc {
                return None;
            }
            let merged = match &states[t] {
                None => Some(out.clone()),
                Some(old) => {
                    let m: RegState = old.iter().zip(&out).map(|(a, b)| if a == b { *a } else { None }).collect();
                    (m != *old).then_some(m)
                }
            };
            if let Some(m) = merged {
                states[t] = Some(m);
                work.push(t);
            }
        }
    }
    Some(CalleeTy { states, slot_ty, result: result? })
}

/// The compilation state of a loop (per `LoopEnter` index, in `Code::jit`).
pub enum JitSlot {
    Ready(std::rc::Rc<JitLoop>),
    /// Not compiled yet: this many attempts failed for the values seen.
    Failed(u8),
    Never,
}

/// Why a loop was not compiled.
pub enum Reject {
    /// It uses something native code does not support.
    Never,
    /// The variables' current values do not allow it (try a later entry).
    NotNow,
}

fn not_now(site: u32) -> Reject {
    if trace() {
        eprintln!("jit: not compiled now (reason {})", site);
    }
    Reject::NotNow
}

/// How native code ended.
pub enum Outcome {
    /// The loop finished: continue after its `LoopExit`.
    Exited,
    /// Continue in the VM at this op.
    Deopt(usize),
}

thread_local! {
    static MODULE: RefCell<Option<JITModule>> = const { RefCell::new(None) };
    static ENABLED: bool = std::env::var("ROBIGO_JIT").map(|v| v != "0").unwrap_or(true);
    static TRACE: bool = std::env::var_os("ROBIGO_JIT_TRACE").is_some();
}

/// Whether hot loops are compiled to native code (`ROBIGO_JIT=0`: no).
pub fn enabled() -> bool {
    ENABLED.with(|e| *e)
}

pub fn trace() -> bool {
    TRACE.with(|t| *t)
}

// ----------------------------------------------------------------------
// Helpers called from native code. They compute exactly what the VM's
// fast paths compute (`vm::arith_fast`, `vm::math1_fast`).

extern "C" fn h_pow(x: f64, y: f64) -> f64 {
    r_pow(x, y)
}
extern "C" fn h_fmod(x: f64, y: f64) -> f64 {
    myfmod(x, y)
}
extern "C" fn h_fmod_warns(x: f64, y: f64) -> i32 {
    crate::builtins::arith::fmod_warns(x, y) as i32
}
extern "C" fn h_fidiv(x: f64, y: f64) -> f64 {
    myfloor(x, y)
}
extern "C" fn h_ipow(x: i32, y: i32) -> f64 {
    if x == 1 || y == 0 {
        1.0
    } else if x == NA_INTEGER || y == NA_INTEGER {
        NA_REAL
    } else {
        r_pow(x as f64, y as f64)
    }
}
extern "C" fn h_imod(x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER || y == 0 {
        NA_INTEGER
    } else if x >= 0 && y > 0 {
        x % y
    } else {
        myfmod(x as f64, y as f64) as i32
    }
}
extern "C" fn h_iidiv(x: i32, y: i32) -> i32 {
    if x == NA_INTEGER || y == NA_INTEGER || y == 0 { NA_INTEGER } else { (x as f64 / y as f64).floor() as i32 }
}
extern "C" fn h_exp(x: f64) -> f64 {
    x.exp()
}
extern "C" fn h_sin(x: f64) -> f64 {
    x.sin()
}
extern "C" fn h_cos(x: f64) -> f64 {
    x.cos()
}
extern "C" fn h_tan(x: f64) -> f64 {
    x.tan()
}

const HELPERS: &[(&str, *const u8, &[Type], Type)] = &[
    ("robigo_pow", h_pow as *const u8, &[types::F64, types::F64], types::F64),
    ("robigo_fmod", h_fmod as *const u8, &[types::F64, types::F64], types::F64),
    ("robigo_fmod_warns", h_fmod_warns as *const u8, &[types::F64, types::F64], types::I32),
    ("robigo_fidiv", h_fidiv as *const u8, &[types::F64, types::F64], types::F64),
    ("robigo_ipow", h_ipow as *const u8, &[types::I32, types::I32], types::F64),
    ("robigo_imod", h_imod as *const u8, &[types::I32, types::I32], types::I32),
    ("robigo_iidiv", h_iidiv as *const u8, &[types::I32, types::I32], types::I32),
    ("robigo_exp", h_exp as *const u8, &[types::F64], types::F64),
    ("robigo_sin", h_sin as *const u8, &[types::F64], types::F64),
    ("robigo_cos", h_cos as *const u8, &[types::F64], types::F64),
    ("robigo_tan", h_tan as *const u8, &[types::F64], types::F64),
    ("robigo_reserve", h_reserve as *const u8, &[types::I64, types::I64, types::I64], types::I64),
];

fn new_module() -> JITModule {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    // Cranelift's development verifier runs after each compilation pass.
    // Keep it in debug builds; release loops pay only for code generation.
    flags.set("enable_verifier", if cfg!(debug_assertions) { "true" } else { "false" }).unwrap();
    let isa = cranelift_native::builder().unwrap().finish(settings::Flags::new(flags)).unwrap();
    let mut jb = JITBuilder::with_isa(isa, default_libcall_names());
    for (name, f, _, _) in HELPERS {
        jb.symbol(*name, *f);
    }
    for (name, f, _, _) in closure::DYN_HELPERS {
        jb.symbol(*name, *f);
    }
    JITModule::new(jb)
}

// ----------------------------------------------------------------------
// Analysis

/// The loop region: ops `start..end`, `end` being the `LoopExit`.
struct Region {
    start: usize,
    end: usize,
}

fn successors(op: &Op, pc: usize, out: &mut Vec<usize>) {
    out.clear();
    match op {
        Op::Jump { target } => out.push(*target as usize),
        Op::JumpIf { target, .. } | Op::JumpIfNot { target, .. } | Op::AndOrShort { target, .. } => {
            out.push(pc + 1);
            out.push(*target as usize);
        }
        Op::ForStep { body, .. } => {
            out.push(pc + 1);
            out.push(*body as usize);
        }
        _ => out.push(pc + 1),
    }
}

/// Registers an op reads and writes; slots it reads and writes.
#[derive(Default)]
struct Uses {
    rregs: Vec<Reg>,
    wregs: Vec<Reg>,
    rslots: Vec<Slot>,
    wslots: Vec<Slot>,
}

fn leaf_uses(l: &Leaf, u: &mut Uses) {
    match l {
        Leaf::Var(_, s) => u.rslots.push(*s),
        Leaf::R(r) => u.rregs.push(*r),
        _ => {}
    }
}

fn arg_uses(a: &CallArg, u: &mut Uses) {
    match a {
        CallArg::Leaf(l) => leaf_uses(l, u),
        CallArg::Arith(_, x, y) => {
            leaf_uses(x, u);
            leaf_uses(y, u);
        }
        CallArg::Index(x, i, _) => {
            u.rslots.push(*x);
            leaf_uses(i, u);
        }
        CallArg::Nested(n) => {
            arg_uses(&n.1, u);
            if let Some(b) = &n.2 {
                arg_uses(b, u);
            }
        }
    }
}

fn uses(op: &Op, inl: &Inl) -> Uses {
    let mut u = Uses::default();
    match op {
        Op::Call { dst, site, .. } => {
            if let Some(ic) = inl.calls.get(site) {
                for a in &ic.args {
                    arg_uses(a, &mut u);
                }
            }
            u.wregs.push(*dst);
        }
        Op::ConstD { dst, .. }
        | Op::ConstI { dst, .. }
        | Op::ConstL { dst, .. }
        | Op::Const { dst, .. }
        | Op::Null { dst, .. } => u.wregs.push(*dst),
        Op::Subset { dst, x, i, .. } => {
            u.rregs.extend([*x, *i]);
            u.wregs.push(*dst);
        }
        Op::SubsetObj { x, .. } => u.rregs.push(*x),
        Op::CallBuiltin { dst, args, .. } => {
            if let Some(code) = inl.specs.and_then(|s| s.code()) {
                u.rregs.extend(code.arglists[*args as usize].iter().map(|(_, r)| *r));
            } else if let Some((_, regs)) = inl.minmax.get(args) {
                u.rregs.extend(regs);
            }
            u.wregs.push(*dst);
        }
        Op::GetVar { dst, slot, .. } => {
            u.rslots.push(*slot);
            u.wregs.push(*dst);
        }
        Op::SetVar { src, slot, .. } => {
            u.rregs.push(*src);
            u.wslots.push(*slot);
        }
        Op::Arith { dst, a, b, .. } | Op::Rel { dst, a, b, .. } => {
            u.rregs.extend([*a, *b]);
            u.wregs.push(*dst);
        }
        Op::ArithL { dst, a, b, .. } | Op::RelL { dst, a, b, .. } => {
            leaf_uses(a, &mut u);
            leaf_uses(b, &mut u);
            u.wregs.push(*dst);
        }
        Op::Neg { dst, a, .. } | Op::Not { dst, a, .. } | Op::Math1 { dst, a, .. } | Op::AndOrArg { dst, a, .. } => {
            u.rregs.push(*a);
            u.wregs.push(*dst);
        }
        Op::Builtin1 { dst, a, .. } => {
            u.rregs.push(*a);
            u.wregs.push(*dst);
        }
        Op::SuperSet { src, .. } => u.rregs.push(*src),
        Op::JumpIf { cond, .. } | Op::JumpIfNot { cond, .. } => u.rregs.push(*cond),
        Op::AndOrShort { x, dst, .. } => {
            u.rregs.push(*x);
            u.wregs.push(*dst);
        }
        Op::AndOr { dst, x, y, .. } => {
            u.rregs.extend([*x, *y]);
            u.wregs.push(*dst);
        }
        Op::ForRange { a, b, seq, kind, .. } => {
            u.rregs.push(*a);
            if *kind == RangeKind::Colon {
                u.rregs.push(*b);
            }
            u.wregs.push(*seq);
        }
        Op::ForInit { seq, .. } => {
            u.rregs.push(*seq);
            u.wregs.push(*seq + 1);
        }
        Op::ForStep { seq, idx, slot, .. } => {
            u.rregs.extend([*seq, *idx]);
            u.wregs.push(*idx);
            u.wslots.push(*slot);
        }
        Op::Index2 { dst, slot, i, j, .. } => {
            u.rslots.push(*slot);
            leaf_uses(i, &mut u);
            leaf_uses(j, &mut u);
            u.wregs.push(*dst);
        }
        Op::SetIndex2 { val, slot, i, j, .. } => {
            u.rslots.push(*slot);
            leaf_uses(i, &mut u);
            leaf_uses(j, &mut u);
            u.rregs.push(*val);
        }
        Op::Index { dst, slot, i, .. } => {
            u.rslots.push(*slot);
            leaf_uses(i, &mut u);
            u.wregs.push(*dst);
        }
        Op::SetIndex { val, slot, i, .. } => {
            u.rregs.push(*val);
            leaf_uses(i, &mut u);
            // The vector is read and modified in place, not reassigned.
            u.rslots.push(*slot);
        }
        _ => {}
    }
    u
}

/// The registers op `op` of `code` reads when the VM executes it, for an op
/// native code leaves at (`None`: one it cannot leave at).
fn leave_reads(op: &Op, code: &Code) -> Option<Vec<Reg>> {
    let leaf = |l: &Leaf, out: &mut Vec<Reg>| {
        if let Leaf::R(r) = l {
            out.push(*r)
        }
    };
    let mut out = Vec::new();
    match op {
        Op::Const { .. }
        | Op::ConstD { .. }
        | Op::ConstI { .. }
        | Op::ConstL { .. }
        | Op::GetVar { .. }
        | Op::Jump { .. }
        | Op::Eval { .. }
        | Op::CallSpecial { .. }
        | Op::Missing { .. }
        | Op::Guard { .. }
        | Op::Null { .. }
        | Op::Call { .. }
        | Op::Visible { .. } => {}
        Op::SuperSet { src, .. } | Op::SetVar { src, .. } | Op::Return { src } => out.push(*src),
        Op::SuperSetIndex { val, i, .. } | Op::SetIndex { val, i, .. } => {
            out.push(*val);
            leaf(i, &mut out);
        }
        Op::SetComplex { val, .. } | Op::SetDollar { val, .. } => out.push(*val),
        Op::Arith { a, b, .. } | Op::Rel { a, b, .. } | Op::Colon { a, b, .. } => out.extend([*a, *b]),
        Op::Neg { a, .. }
        | Op::Not { a, .. }
        | Op::Math1 { a, .. }
        | Op::AndOrArg { a, .. }
        | Op::Builtin1 { a, .. } => out.push(*a),
        Op::JumpIf { cond, .. } | Op::JumpIfNot { cond, .. } => out.push(*cond),
        Op::Index { i, .. } => leaf(i, &mut out),
        Op::Index2 { i, j, .. } => {
            leaf(i, &mut out);
            leaf(j, &mut out);
        }
        Op::SetIndex2 { val, i, j, .. } => {
            out.push(*val);
            leaf(i, &mut out);
            leaf(j, &mut out);
        }
        Op::SubsetObj { x, .. } | Op::Dollar { x, .. } | Op::Switch { x, .. } => out.push(*x),
        Op::Subset { x, i, .. } => out.extend([*x, *i]),
        Op::SlotRead { object, .. } => out.push(*object),
        Op::CallBuiltin { args, .. } => out.extend(code.arglists[*args as usize].iter().map(|(_, r)| *r)),
        Op::AndOrShort { x, .. } => out.push(*x),
        Op::AndOr { x, y, .. } => out.extend([*x, *y]),
        Op::ArithL { a, b, .. } | Op::RelL { a, b, .. } => {
            leaf(a, &mut out);
            leaf(b, &mut out);
        }
        Op::ForSeqLength { len, .. } => leaf(len, &mut out),
        // (Structure: these stay in the VM with the whole loop.)
        Op::ForRange { .. } | Op::ForInit { .. } | Op::ForStep { .. } | Op::LoopEnter { .. } | Op::LoopExit => {
            return None;
        }
    }
    Some(out)
}

/// The `max(a, b, ...)` and `min(...)` calls of the loop `r` of `code` with
/// two to four untagged arguments, by argument list.
fn minmax_calls(code: &Code, r: &Region) -> HashMap<u32, (bool, Vec<Reg>)> {
    let mut out = HashMap::new();
    for op in &code.ops[r.start..r.end] {
        let Op::CallBuiltin { f, args, .. } = op else { continue };
        let Value::Builtin(b) = &code.consts[*f as usize] else { continue };
        let a = &code.arglists[*args as usize];
        if (b.name == "max" || b.name == "min") && (2..=4).contains(&a.len()) && a.iter().all(|(t, _)| t.is_none()) {
            out.insert(*args, (b.name == "max", a.iter().map(|(_, r)| *r).collect()));
        }
    }
    out
}

/// Whether native code supports `op` (given the region bounds for jumps).
fn supported(op: &Op, r: &Region) -> bool {
    let inside = |t: u32| (t as usize) >= r.start && (t as usize) <= r.end;
    match op {
        Op::ConstD { .. }
        | Op::ConstI { .. }
        | Op::ConstL { .. }
        | Op::GetVar { .. }
        | Op::SetVar { .. }
        | Op::Arith { .. }
        | Op::Rel { .. }
        | Op::ArithL { .. }
        | Op::RelL { .. }
        | Op::Neg { .. }
        | Op::Not { .. }
        | Op::Math1 { .. }
        | Op::AndOrArg { .. }
        | Op::AndOr { .. }
        | Op::ForInit { .. }
        | Op::Visible { .. }
        | Op::LoopExit
        | Op::Index { .. }
        | Op::Index2 { .. }
        | Op::SetIndex2 { .. }
        // (No code: the register is dead, see `defs`. A NULL value is
        // not supported.)
        | Op::Null { dead: true, .. } => true,
        Op::Guard { skip, .. } => inside(*skip),
        Op::LoopEnter { brk, nxt } => inside(*brk) && inside(*nxt),
        Op::Jump { target }
        | Op::JumpIf { target, .. }
        | Op::JumpIfNot { target, .. }
        | Op::AndOrShort { target, .. } => inside(*target),
        Op::ForStep { body, .. } => inside(*body),
        Op::ForRange { kind, .. } => *kind != RangeKind::SeqAlong,
        // (`x[[i]] <- v` too: on the attribute-free double and integer
        // vectors and in-range scalar indices native code handles, it does
        // what `x[i] <- v` does; anything else deoptimizes.)
        Op::SetIndex { .. } => true,
        _ => false,
    }
}

fn arith_type(op: ArithOp, a: JType, b: JType) -> Option<JType> {
    if !a.is_scalar() || !b.is_scalar() {
        return None;
    }
    Some(if a == JType::D || b == JType::D {
        JType::D
    } else {
        match op {
            ArithOp::Div | ArithOp::Pow => JType::D,
            _ => JType::I,
        }
    })
}

/// Compile the loop starting at the `LoopEnter` op `enter`. `entry(slot)`
/// gives the type of a variable's current value (`None`: unbound or not
/// representable); `live_in(reg)` that of a register set before the loop.
pub fn compile(
    code: &Code,
    enter: usize,
    entry: &dyn Fn(Slot) -> Option<JType>,
    live_in: &dyn Fn(Reg) -> Option<JType>,
    calls: HashMap<u32, InlineCall>,
) -> Result<JitLoop, Reject> {
    let mut inl = Inl::new(&calls);
    let Op::LoopEnter { brk, .. } = code.ops[enter] else { return Err(Reject::Never) };
    let r = Region { start: enter + 1, end: brk as usize };
    if r.end > code.ops.len() || r.start >= r.end {
        return Err(Reject::Never);
    }
    let ops = &code.ops[..];
    inl.minmax = minmax_calls(code, &r);
    // Ops native code does not have: it leaves there, and the VM executes
    // them and goes on (a `cat()` at the end of a loop, a `return`).
    let mut leave = vec![false; ops.len()];
    let mut nested = Vec::new();
    let mut guards = Vec::new();
    for pc in r.start..r.end {
        let op = &ops[pc];
        let native = match op {
            Op::Call { site, .. } => calls.contains_key(site),
            Op::CallBuiltin { args, .. } => inl.minmax.contains_key(args),
            _ => supported(op, &r),
        };
        if !native {
            // Only calls and `return`, and not in a loop inside the loop:
            // anything else, or there, is likely to run in every iteration.
            let rare = matches!(
                op,
                Op::Call { .. } | Op::CallBuiltin { .. } | Op::CallSpecial { .. } | Op::Eval { .. } | Op::Return { .. }
            ) && !nested.iter().any(|(e, b, _)| (*e as usize) < pc && pc < *b as usize);
            if !rare || leave_reads(op, code).is_none() {
                if trace() {
                    eprintln!("jit: unsupported at {}: {:?}", pc, op);
                }
                return Err(Reject::Never);
            }
            leave[pc] = true;
            continue;
        }
        match op {
            Op::LoopEnter { brk, nxt } => nested.push((pc as u32, *brk, *nxt)),
            Op::Guard { sym, .. } => guards.push((*sym, pc)),
            Op::ArithL { sym, g, .. } | Op::RelL { sym, g, .. } if *g != NO_GUARD => guards.push((*sym, pc)),
            _ => {}
        }
    }
    inl.leave = leave;
    let inl = inl;
    let native_ops = || (r.start..r.end).filter(|pc| !inl.leaves_at(*pc));

    // Variables have one type each; registers have a type per op (the VM
    // reuses them across statements), found by a forward analysis.
    let mut slot_ty: HashMap<Slot, JType> = HashMap::new();
    for pc in native_ops() {
        if let Op::Index { slot, .. }
        | Op::SetIndex { slot, .. }
        | Op::Index2 { slot, .. }
        | Op::SetIndex2 { slot, .. } = &ops[pc]
        {
            match entry(*slot) {
                Some(t) if t.is_vector() => {
                    slot_ty.insert(*slot, t);
                }
                // (`vm::jit_enter` materializes it.)
                Some(JType::Range) => {
                    slot_ty.insert(*slot, JType::VecI);
                }
                _ => return Err(not_now(1)),
            }
        }
    }
    for pc in native_ops() {
        for s in uses(&ops[pc], &inl).rslots {
            if let std::collections::hash_map::Entry::Vacant(e) = slot_ty.entry(s) {
                if let Some(t) = entry(s) {
                    e.insert(t);
                }
            }
        }
    }
    // Registers read before any write in the loop (in op order) carry a
    // value in. (Registers are reused, so others may hold leftovers of
    // earlier statements; leaving such a register out is safe, as reading
    // a register without a type rejects the loop.)
    let mut entry_state: RegState = vec![None; code.nregs];
    let mut written = vec![false; code.nregs];
    let mut live = Vec::new();
    for pc in native_ops() {
        let u = uses(&ops[pc], &inl);
        for reg in u.rregs {
            if written[reg as usize] {
                continue;
            }
            if let Some(t) = live_in(reg) {
                if entry_state[reg as usize].is_none() {
                    entry_state[reg as usize] = Some(t);
                    live.push((reg, t));
                }
            }
        }
        for reg in u.wregs {
            written[reg as usize] = true;
        }
    }
    let reg_written = written.clone();
    let mut rounds = 0;
    let states = loop {
        rounds += 1;
        if rounds > 20 {
            return Err(Reject::Never);
        }
        if let Some(st) = reg_flow(ops, &r, &entry_state, &mut slot_ty, &inl)? {
            break st;
        }
    };
    // A loop that leaves before it can run a whole iteration natively would
    // only cost its compilation.
    // (A back edge on a cycle of ops native code has.)
    let back_edge = native_ops().any(|pc| {
        let t = match &ops[pc] {
            Op::Jump { target } | Op::JumpIf { target, .. } | Op::JumpIfNot { target, .. } => *target as usize,
            Op::ForStep { body, .. } => *body as usize,
            _ => return false,
        };
        if t > pc || states[pc - r.start].is_none() {
            return false;
        }
        let mut seen = vec![false; r.end - r.start];
        let mut work = vec![t];
        let mut succ = Vec::new();
        while let Some(q) = work.pop() {
            if q == pc {
                return true;
            }
            if q < r.start || q >= r.end || seen[q - r.start] || inl.leaves_at(q) {
                continue;
            }
            seen[q - r.start] = true;
            successors(&ops[q], q, &mut succ);
            work.extend(succ.iter().copied());
        }
        false
    });
    if !back_edge {
        if trace() {
            eprintln!("jit: loop at {} leaves native code in every iteration", enter);
        }
        return Err(Reject::Never);
    }
    // Check every reachable op's operands.
    for pc in r.start..r.end {
        let Some(st) = &states[pc - r.start] else { continue };
        let op = &ops[pc];
        if inl.leaves_at(pc) {
            // The VM reads its registers: they must be ones native code
            // restores or does not touch.
            for reg in leave_reads(op, code).unwrap_or_default() {
                if st[reg as usize].is_none() && written[reg as usize] {
                    if trace() {
                        eprintln!("jit: register {} has no single type where native code leaves, at {}", reg, pc);
                    }
                    return Err(Reject::Never);
                }
            }
            continue;
        }
        let u = uses(op, &inl);
        for s in u.rslots.iter().chain(&u.wslots) {
            if !slot_ty.contains_key(s) {
                return Err(not_now(8));
            }
        }
        for reg in &u.rregs {
            if st[*reg as usize].is_none() {
                if trace() {
                    eprintln!("jit: register {} has no single type at {}: {:?}", reg, pc, op);
                }
                return Err(Reject::Never);
            }
        }
        if !types_ok(op, st, &slot_ty, &inl) {
            if trace() {
                eprintln!("jit: unsupported types at {}: {:?}", pc, op);
            }
            return Err(Reject::Never);
        }
    }

    // Must-defined analysis of variables: those read before written need
    // their value at entry.
    let nslots = code.vars.len();
    let n = r.end - r.start + 1;
    let mut state: Vec<Option<Vec<bool>>> = vec![None; n];
    state[0] = Some(vec![false; nslots]);
    let mut needs_entry = vec![false; nslots];
    let mut work = vec![r.start];
    let mut succ = Vec::new();
    while let Some(pc) = work.pop() {
        if pc >= r.end || inl.leaves_at(pc) {
            continue;
        }
        let Some(mut sd) = state[pc - r.start].clone() else { continue };
        let op = &ops[pc];
        let u = uses(op, &inl);
        for s in &u.rslots {
            if !sd[*s as usize] {
                needs_entry[*s as usize] = true;
            }
        }
        for s in &u.wslots {
            sd[*s as usize] = true;
        }
        successors(op, pc, &mut succ);
        for &t in &succ {
            let k = t - r.start;
            let next = match &state[k] {
                None => Some(sd.clone()),
                Some(s0) => {
                    let s1: Vec<bool> = s0.iter().zip(&sd).map(|(a, b)| *a && *b).collect();
                    (s1 != *s0).then_some(s1)
                }
            };
            if let Some(st) = next {
                state[k] = Some(st);
                work.push(t);
            }
        }
    }
    let mut written = vec![false; nslots];
    let mut writable = vec![false; nslots];
    let mut assigned = vec![false; nslots];
    for pc in native_ops() {
        for s in uses(&ops[pc], &inl).wslots {
            written[s as usize] = true;
        }
        match &ops[pc] {
            Op::SetIndex { slot, .. } | Op::SetIndex2 { slot, .. } => writable[*slot as usize] = true,
            Op::SetVar { slot, .. } => assigned[*slot as usize] = true,
            _ => {}
        }
    }
    let vlive = vector_liveness(code, &r);
    // Where native code leaves, every register the VM goes on to read must
    // be one native code restores or does not touch.
    for pc in r.start..r.end {
        let Some(st) = &states[pc - r.start] else { continue };
        if !inl.leaves_at(pc) {
            continue;
        }
        for (reg, live) in vlive[pc - r.start].iter().enumerate() {
            if *live && st[reg].is_none() && reg_written[reg] {
                if trace() {
                    eprintln!("jit: register {} has no single type where native code leaves, at {}", reg, pc);
                }
                return Err(Reject::Never);
            }
        }
    }
    // A vector register views a variable's vector: the loop must not
    // change that vector while the register is live (R's value would be
    // the old one).
    for pc in native_ops() {
        let (Op::SetVar { slot, .. } | Op::SetIndex { slot, .. } | Op::SetIndex2 { slot, .. }) = &ops[pc] else {
            continue;
        };
        if !slot_ty.get(slot).is_some_and(|t| t.is_vector()) {
            continue;
        }
        let after = &vlive[pc + 1 - r.start];
        for (reg, live) in after.iter().enumerate() {
            let views = native_ops()
                .any(|q| matches!(&ops[q], Op::GetVar { dst, slot: s2, .. } if *dst as usize == reg && s2 == slot));
            if *live && views {
                if trace() {
                    eprintln!("jit: register {} views a vector the loop changes, at {}", reg, pc);
                }
                return Err(Reject::Never);
            }
        }
    }

    // Layout of the value and vector buffers.
    let mut slots = Vec::new();
    let mut nvals = 0;
    let mut nvecs = 0;
    let mut slot_list: Vec<Slot> = slot_ty.keys().copied().collect();
    slot_list.sort();
    for s in slot_list {
        let ty = slot_ty[&s];
        let owned = ty.is_vector() && assigned[s as usize];
        if owned && !matches!(ty, JType::VecD | JType::VecI) {
            return Err(Reject::Never);
        }
        let ne = needs_entry[s as usize] || (ty.is_vector() && !owned);
        if ne && entry(s) != Some(ty) && !(ty == JType::VecI && entry(s) == Some(JType::Range)) {
            return Err(not_now(9));
        }
        let idx = if ty.is_vector() {
            nvecs += 1;
            nvecs - 1
        } else {
            nvals += if ty == JType::Range { 3 } else { 1 };
            nvals - if ty == JType::Range { 3 } else { 1 }
        };
        let flag = if written[s as usize] && !ne && (ty.is_scalar() || ty == JType::Range || owned) {
            nvals += 1;
            Some(nvals - 1)
        } else {
            None
        };
        slots.push(SlotInfo {
            slot: s,
            sym: code.vars[s as usize],
            ty,
            idx,
            needs_entry: ne,
            written: written[s as usize],
            flag,
            writable: writable[s as usize],
            owned,
        });
    }
    let width = |t: JType| match t {
        JType::Range => 3,
        JType::VecD | JType::VecI => 2,
        _ => 1,
    };
    let mut reg_idx = HashMap::new();
    for st in states.iter().flatten() {
        for (reg, t) in st.iter().enumerate() {
            if let Some(t) = t {
                if let std::collections::hash_map::Entry::Vacant(e) = reg_idx.entry(reg as Reg) {
                    e.insert(nvals);
                    nvals += 3.max(width(*t));
                }
            }
        }
    }
    // Registers only defined (never live at an op) still need a place.
    for pc in native_ops() {
        if let Some(st) = &states[pc - r.start] {
            for (reg, t) in defs(&ops[pc], st, &slot_ty, &inl) {
                if let (Some(_), std::collections::hash_map::Entry::Vacant(e)) = (t, reg_idx.entry(reg as Reg)) {
                    e.insert(nvals);
                    nvals += 3;
                }
            }
        }
    }
    let leaves: Vec<u32> = (r.start..r.end).filter(|pc| inl.leaves_at(*pc)).map(|pc| pc as u32).collect();
    let counter = (!leaves.is_empty()).then(|| {
        nvals += 1;
        nvals - 1
    });
    let builtins = native_ops().any(|pc| matches!(ops[pc], Op::CallBuiltin { .. }));
    nvals += 1;
    let vis = nvals - 1;
    let vis_at = visibility(code, &r, &inl);

    let mut deopt_regs = HashMap::new();
    let lay = Layout {
        slots: &slots,
        slot_ty: &slot_ty,
        states: &states,
        reg_idx: &reg_idx,
        live: &live,
        vlive: &vlive,
        counter,
        vis,
        vis_at: &vis_at,
    };
    let func = codegen(code, &r, &lay, &mut deopt_regs, &inl)?;
    let calls = calls.into_iter().map(|(site, c)| (site, c.clo, c.guard_syms)).collect();
    let mut nulled: Vec<Reg> = ops[r.start..r.end]
        .iter()
        .filter_map(|op| if let Op::Null { dst, dead: true } = op { Some(*dst) } else { None })
        .collect();
    nulled.sort_unstable();
    nulled.dedup();
    Ok(JitLoop {
        func,
        slots,
        reg_idx,
        live_in: live,
        deopt_regs,
        nulled,
        nvals,
        nvecs,
        nested,
        guards,
        calls,
        leaves,
        counter,
        stats: std::cell::Cell::new((0, 0)),
        builtins,
        vis,
    })
}

/// What `codegen` lays out: the variables, the register types per op, where
/// registers live in the value buffer, the live-in registers, the vector
/// registers live before each op, and the back-edge counter.
struct Layout<'a> {
    slots: &'a [SlotInfo],
    slot_ty: &'a HashMap<Slot, JType>,
    states: &'a [Option<RegState>],
    reg_idx: &'a HashMap<Reg, usize>,
    live: &'a [(Reg, JType)],
    vlive: &'a [Vec<bool>],
    counter: Option<usize>,
    vis: usize,
    vis_at: &'a [Vis],
}

/// What the VM's visibility flag is before an op: as at entry, set to a
/// known value, or either (then native code tracks it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Vis {
    Unreached,
    Entry,
    Is(bool),
    Varies,
}

/// The visibility an op leaves (`vm::run_with`: these ops set it).
fn sets_visible(op: &Op) -> Option<bool> {
    match op {
        Op::Visible { on } => Some(*on),
        Op::GetVar { .. }
        | Op::Index { .. }
        | Op::Index2 { .. }
        | Op::Builtin1 { .. }
        | Op::CallBuiltin { .. }
        | Op::Call { .. } => Some(true),
        _ => None,
    }
}

/// The visibility before each op of the region (and at its end), where the
/// VM can read it: a `return` native code leaves at, reached from the op
/// without passing one that sets it (`Vis::Unreached` elsewhere).
fn visibility(code: &Code, r: &Region, inl: &Inl) -> Vec<Vis> {
    let n = r.end - r.start + 1;
    let mut need = vec![false; n];
    let mut succ = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for pc in (r.start..r.end).rev() {
            let op = &code.ops[pc];
            let nd = if inl.leaves_at(pc) {
                matches!(op, Op::Return { .. })
            } else if sets_visible(op).is_some() {
                false
            } else {
                successors(op, pc, &mut succ);
                succ.iter().any(|t| *t >= r.start && *t < r.end && need[*t - r.start])
            };
            if nd && !need[pc - r.start] {
                need[pc - r.start] = true;
                changed = true;
            }
        }
    }
    let mut at = flow_visibility(code, r, inl);
    for (k, a) in at.iter_mut().enumerate() {
        if !need[k] {
            *a = Vis::Unreached;
        }
    }
    at
}

fn flow_visibility(code: &Code, r: &Region, inl: &Inl) -> Vec<Vis> {
    let n = r.end - r.start + 1;
    let mut at = vec![Vis::Unreached; n];
    at[0] = Vis::Entry;
    let mut work = vec![r.start];
    let mut succ = Vec::new();
    while let Some(pc) = work.pop() {
        if pc >= r.end || inl.leaves_at(pc) {
            continue;
        }
        let op = &code.ops[pc];
        let out = match sets_visible(op) {
            Some(on) => Vis::Is(on),
            None => at[pc - r.start],
        };
        successors(op, pc, &mut succ);
        for &t in &succ {
            if t < r.start || t > r.end {
                continue;
            }
            let old = at[t - r.start];
            let new = match (old, out) {
                (Vis::Unreached, x) => x,
                (a, b) if a == b => a,
                _ => Vis::Varies,
            };
            if new != old {
                at[t - r.start] = new;
                work.push(t);
            }
        }
    }
    at
}

/// The registers live before each op of the region (and at its end) when the
/// VM runs from there, as it does after native code leaves: a backward
/// analysis of the VM's reads, definite writes and jumps. A deopt restores
/// the vector registers live there (copies of what they view).
fn vector_liveness(code: &Code, r: &Region) -> Vec<Vec<bool>> {
    let n = r.end - r.start + 1;
    let mut live = vec![vec![false; code.nregs]; n];
    let mut succ = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for pc in (r.start..r.end).rev() {
            let op = &code.ops[pc];
            let mut out = vec![false; code.nregs];
            vm_successors(code, op, pc, &mut succ);
            // (A jump that writes a register first: a failing guard's, an
            // object's subset, a short-circuit `&&`/`||`.)
            let jump_writes = match op {
                Op::Guard { dst, skip, .. } => Some((*skip as usize, *dst)),
                Op::SubsetObj { dst, end, .. } => Some((*end as usize, *dst)),
                Op::AndOrShort { dst, target, .. } => Some((*target as usize, *dst)),
                _ => None,
            };
            for &t in &succ {
                if t >= r.start && t < r.end {
                    for (k, l) in live[t - r.start].iter().enumerate() {
                        let killed = jump_writes == Some((t, k as Reg)) && t != pc + 1;
                        out[k] |= *l && !killed;
                    }
                }
            }
            if let Some(w) = vm_writes(op) {
                out[w as usize] = false;
            }
            let empty = HashMap::new();
            let reads = leave_reads(op, code).unwrap_or_else(|| uses(op, &Inl::new(&empty)).rregs);
            for rd in reads {
                out[rd as usize] = true;
            }
            if out != live[pc - r.start] {
                live[pc - r.start] = out;
                changed = true;
            }
        }
    }
    live
}

/// Where the VM may go after op `pc`: [`successors`], and the jumps a failing
/// guard, an object's subset or a `switch` take.
fn vm_successors(code: &Code, op: &Op, pc: usize, out: &mut Vec<usize>) {
    successors(op, pc, out);
    match op {
        Op::Guard { skip, .. } => out.push(*skip as usize),
        Op::SubsetObj { end, .. } => out.push(*end as usize),
        Op::Switch { table, .. } => {
            let t = &code.switches[*table as usize];
            out.extend(t.by_name.iter().map(|x| *x as usize));
            out.extend(t.by_number.iter().flatten().map(|x| *x as usize));
            out.extend(t.default.map(|x| x as usize));
            out.push(t.null as usize);
        }
        _ => {}
    }
}

/// The register op `op` always writes when the VM executes it and goes on
/// to the next op.
fn vm_writes(op: &Op) -> Option<Reg> {
    match op {
        Op::Const { dst, .. }
        | Op::ConstD { dst, .. }
        | Op::ConstI { dst, .. }
        | Op::ConstL { dst, .. }
        | Op::GetVar { dst, .. }
        | Op::Arith { dst, .. }
        | Op::Neg { dst, .. }
        | Op::Rel { dst, .. }
        | Op::Not { dst, .. }
        | Op::Index { dst, .. }
        | Op::Subset { dst, .. }
        | Op::Index2 { dst, .. }
        | Op::CallBuiltin { dst, .. }
        | Op::Math1 { dst, .. }
        | Op::AndOrArg { dst, .. }
        | Op::AndOr { dst, .. }
        | Op::Eval { dst, .. }
        | Op::CallSpecial { dst, .. }
        | Op::Dollar { dst, .. }
        | Op::SlotRead { dst, .. }
        | Op::Missing { dst, .. }
        | Op::Null { dst, .. }
        | Op::Call { dst, .. }
        | Op::ArithL { dst, .. }
        | Op::RelL { dst, .. }
        | Op::Colon { dst, .. }
        | Op::Builtin1 { dst, .. } => Some(*dst),
        Op::ForRange { seq, .. } | Op::ForSeqLength { seq, .. } => Some(*seq),
        Op::ForInit { seq, .. } => Some(*seq + 1),
        _ => None,
    }
}

/// Register types before an op: `None` for undefined, or different types
/// on different paths.
type RegState = Vec<Option<JType>>;

/// Whether native code can run `op` with these operand types (whose
/// registers are known to have types).
fn types_ok(op: &Op, st: &RegState, slot_ty: &HashMap<Slot, JType>, inl: &Inl) -> bool {
    let rt = |reg: &Reg| st[*reg as usize].unwrap();
    let lt = |l: &Leaf| leaf_type(l, st, slot_ty);
    match op {
        Op::JumpIf { cond, .. } | Op::JumpIfNot { cond, .. } => {
            rt(cond) == JType::L
                || (inl.specs.is_some() && rt(cond) == JType::Dyn)
                || (inl.specs.is_none() && matches!(rt(cond), JType::I | JType::D))
        }
        Op::AndOrShort { x, .. } => rt(x) == JType::L,
        Op::AndOr { x, y, .. } => rt(x) == JType::L && rt(y) == JType::L,
        Op::Index { i, .. } => matches!(lt(i), Some(JType::D | JType::I)),
        Op::Index2 { slot, i, j, .. } => {
            matches!(slot_ty[slot], JType::MatD | JType::MatI)
                && matches!(lt(i), Some(JType::D | JType::I))
                && matches!(lt(j), Some(JType::D | JType::I))
        }
        Op::SetIndex2 { val, slot, i, j, .. } => {
            matches!(lt(i), Some(JType::D | JType::I))
                && matches!(lt(j), Some(JType::D | JType::I))
                && matches!(
                    (slot_ty[slot], rt(val)),
                    (JType::MatD, JType::D | JType::I | JType::L) | (JType::MatI, JType::I | JType::L)
                )
        }
        Op::SetIndex { val, slot, i, .. } => {
            matches!(lt(i), Some(JType::D | JType::I))
                && matches!(
                    (slot_ty[slot], rt(val)),
                    (JType::VecD | JType::MatD, JType::D | JType::I | JType::L)
                        | (JType::VecI | JType::MatI, JType::I | JType::L)
                )
        }
        // (seq_len only takes the fast path for integers and doubles)
        Op::ForRange { kind: RangeKind::Colon, a, b, .. } => rt(a).is_scalar() && rt(b).is_scalar(),
        Op::ForRange { a, .. } => matches!(rt(a), JType::D | JType::I),
        Op::ForInit { seq, .. } | Op::ForStep { seq, .. } => rt(seq) == JType::Range,
        Op::GetVar { slot, .. } => inl.specs.is_some() || loop_value(slot_ty[slot]),
        Op::Null { dead: true, .. } => true,
        _ => defs(op, st, slot_ty, inl).iter().all(|(_, t)| t.is_some()),
    }
}

/// The types a variable of a loop can have in a register: scalars, ranges
/// and plain vectors (whose register views the vector).
fn loop_value(t: JType) -> bool {
    t.is_scalar() || matches!(t, JType::Range | JType::VecD | JType::VecI)
}

fn leaf_type(l: &Leaf, st: &RegState, slot_ty: &HashMap<Slot, JType>) -> Option<JType> {
    match l {
        Leaf::D(_) => Some(JType::D),
        Leaf::I(_) => Some(JType::I),
        Leaf::Var(_, s) => slot_ty.get(s).copied(),
        Leaf::R(reg) => st[*reg as usize],
    }
}

/// The registers an op defines and their types (`None`: unknown).
fn defs(op: &Op, st: &RegState, slot_ty: &HashMap<Slot, JType>, inl: &Inl) -> Vec<(usize, Option<JType>)> {
    if inl.specs.is_some()
        && let Some(d) = closure::dyn_defs(op, st, slot_ty, inl)
    {
        return d;
    }
    let rt = |r: &Reg| st[*r as usize];
    let scalar = |t: Option<JType>| t.filter(|t| t.is_scalar());
    match op {
        Op::Call { dst, site, .. } => {
            let t = inl.calls.get(site).and_then(|ic| {
                let tys: Option<Vec<JType>> = ic.args.iter().map(|a| a.ty(st, slot_ty)).collect();
                inl.result(*site, tys?)
            });
            vec![(*dst as usize, t)]
        }
        Op::ConstD { dst, .. } => vec![(*dst as usize, Some(JType::D))],
        Op::ConstI { dst, .. } => vec![(*dst as usize, Some(JType::I))],
        Op::ConstL { dst, .. } => vec![(*dst as usize, Some(JType::L))],
        // A clear of a dead register (#357): it has no value native code
        // could read; the VM gets `NULL` there when native code leaves
        // (`JitLoop::nulled`).
        Op::Null { dst, dead: true } => vec![(*dst as usize, None)],
        Op::GetVar { dst, slot, .. } if inl.specs.is_some() => vec![(*dst as usize, slot_ty.get(slot).copied())],
        Op::GetVar { dst, slot, .. } => vec![(*dst as usize, slot_ty.get(slot).copied().filter(|t| loop_value(*t)))],
        Op::CallBuiltin { dst, args, .. } => {
            let t = inl.minmax.get(args).and_then(|(_, regs)| {
                let mut t = JType::I;
                for r in regs {
                    match rt(r)? {
                        JType::D => t = JType::D,
                        JType::I | JType::L => {}
                        _ => return None,
                    }
                }
                Some(t)
            });
            vec![(*dst as usize, t)]
        }
        Op::Builtin1 { f: Builtin1::Length, dst, a, .. } => vec![(*dst as usize, rt(a).map(|_| JType::I))],
        Op::Builtin1 { f: Builtin1::IsNull, dst, a, .. } => vec![(*dst as usize, rt(a).map(|_| JType::L))],
        Op::Arith { op, dst, a, b, .. } => {
            let t = match (rt(a), rt(b)) {
                (Some(x), Some(y)) => arith_type(*op, x, y),
                _ => None,
            };
            vec![(*dst as usize, t)]
        }
        Op::ArithL { op, dst, a, b, .. } => {
            let t = match (leaf_type(a, st, slot_ty), leaf_type(b, st, slot_ty)) {
                (Some(x), Some(y)) => arith_type(*op, x, y),
                _ => None,
            };
            vec![(*dst as usize, t)]
        }
        Op::Rel { dst, a, b, .. } => {
            let ok = scalar(rt(a)).is_some() && scalar(rt(b)).is_some();
            vec![(*dst as usize, ok.then_some(JType::L))]
        }
        Op::RelL { dst, a, b, .. } => {
            let ok = scalar(leaf_type(a, st, slot_ty)).is_some() && scalar(leaf_type(b, st, slot_ty)).is_some();
            vec![(*dst as usize, ok.then_some(JType::L))]
        }
        Op::Neg { dst, a, .. } => {
            let t = match rt(a) {
                Some(JType::D) => Some(JType::D),
                Some(JType::I | JType::L) => Some(JType::I),
                _ => None,
            };
            vec![(*dst as usize, t)]
        }
        Op::Not { dst, a, .. } | Op::AndOrArg { dst, a, .. } => {
            vec![(*dst as usize, scalar(rt(a)).map(|_| JType::L))]
        }
        Op::Math1 { f, dst, a, .. } => {
            let t = match (f, rt(a)) {
                (Math1::Abs, Some(JType::I | JType::L)) => Some(JType::I),
                (_, Some(JType::D | JType::I | JType::L)) => Some(JType::D),
                _ => None,
            };
            vec![(*dst as usize, t)]
        }
        Op::AndOrShort { dst, .. } | Op::AndOr { dst, .. } => vec![(*dst as usize, Some(JType::L))],
        Op::ForRange { seq, .. } => vec![(*seq as usize, Some(JType::Range))],
        Op::ForInit { seq, .. } => vec![(*seq as usize + 1, Some(JType::I))],
        Op::ForStep { idx, .. } => vec![(*idx as usize, Some(JType::I))],
        Op::Index { dst, slot, .. } | Op::Index2 { dst, slot, .. } => {
            let t = match slot_ty.get(slot) {
                Some(JType::VecD | JType::MatD) => Some(JType::D),
                Some(JType::VecI | JType::MatI) => Some(JType::I),
                _ => None,
            };
            vec![(*dst as usize, t)]
        }
        _ => Vec::new(),
    }
}

/// Register types before each op of the region (`None` for unreachable
/// ops), assigning variables the types stored into them. `Ok(None)`: a
/// variable got its type now; run again.
fn reg_flow(
    ops: &[Op],
    r: &Region,
    entry: &RegState,
    slot_ty: &mut HashMap<Slot, JType>,
    inl: &Inl,
) -> Result<Option<Vec<Option<RegState>>>, Reject> {
    let n = r.end - r.start + 1;
    let mut states: Vec<Option<RegState>> = vec![None; n];
    states[0] = Some(entry.clone());
    let mut work = vec![r.start];
    let mut succ = Vec::new();
    let mut new_slot = false;
    let mut steps = 0;
    while let Some(pc) = work.pop() {
        steps += 1;
        if steps > 200_000 {
            return Err(Reject::Never);
        }
        if pc >= r.end {
            continue;
        }
        let st = states[pc - r.start].clone().unwrap();
        let op = &ops[pc];
        // Native code leaves here: nothing after it runs.
        if (inl.specs.is_some() && closure::leaves(op, inl)) || inl.leaves_at(pc) {
            continue;
        }
        let mut assign = |slot: Slot, t: JType| -> Result<(), Reject> {
            match slot_ty.get(&slot) {
                Some(old) if *old == t => Ok(()),
                Some(_) => Err(not_now(3)),
                None => {
                    slot_ty.insert(slot, t);
                    new_slot = true;
                    Ok(())
                }
            }
        };
        match op {
            Op::SetVar { src, slot, .. } => {
                if let Some(t) = st[*src as usize] {
                    if !(if inl.specs.is_some() { t.is_scalar() || t == JType::Dyn } else { loop_value(t) }) {
                        return Err(Reject::Never);
                    }
                    assign(*slot, t)?;
                }
            }
            Op::ForStep { slot, .. } => assign(*slot, JType::I)?,
            _ => {}
        }
        let mut out = st.clone();
        for (reg, t) in defs(op, &st, slot_ty, inl) {
            out[reg] = t;
        }
        if matches!(op, Op::Return { .. }) {
            continue;
        }
        // A call whose result is not known yet: as far as known, it does
        // not return (a recursive call gets its type from the other paths).
        if inl.specs.is_some() && matches!(op, Op::Call { dst, .. } if out[*dst as usize].is_none()) {
            continue;
        }
        successors(op, pc, &mut succ);
        for &t in &succ {
            if t > r.end || t < r.start {
                return Err(Reject::Never);
            }
            let k = t - r.start;
            let merged = match &states[k] {
                None => Some(out.clone()),
                Some(old) => {
                    let m: RegState = old.iter().zip(&out).map(|(a, b)| if a == b { *a } else { None }).collect();
                    (m != *old).then_some(m)
                }
            };
            if let Some(m) = merged {
                states[k] = Some(m);
                work.push(t);
            }
        }
    }
    Ok(if new_slot { None } else { Some(states) })
}

// ----------------------------------------------------------------------
// Code generation

fn cl_type(t: JType) -> Type {
    match t {
        JType::D => types::F64,
        // (A pointer to a `VecView`, in a closure compiled whole.)
        JType::VecD | JType::VecI | JType::MatD | JType::MatI | JType::Dyn => types::I64,
        _ => types::I32,
    }
}

struct Vars {
    /// Per slot: (variable, type) for scalars; (ptr, len) variables for
    /// vectors.
    slot: HashMap<Slot, (Variable, JType)>,
    vec: HashMap<Slot, (Variable, Variable, JType)>,
    dims: HashMap<Slot, (Variable, Variable)>,
    flag: HashMap<Slot, Variable>,
    /// Per register and type: one variable, or three for a range.
    reg: HashMap<(Reg, JType), Variable>,
    range: HashMap<Reg, (Variable, Variable, Variable)>,
    lp: LoopVars,
}

/// The variables only a loop has.
#[derive(Default)]
struct LoopVars {
    /// Range variables: start, step, length.
    rslot: HashMap<Slot, (Variable, Variable, Variable)>,
    /// The capacity of an owned vector's buffer (see [`SlotInfo::owned`]).
    cap: HashMap<Slot, Variable>,
    /// Vector registers: pointer and length of the vector they view.
    vreg: HashMap<(Reg, JType), (Variable, Variable)>,
    /// The back edges taken.
    counter: Option<Variable>,
    /// Whether the last value is visible (`interp.visible`), as the VM sets it.
    vis: Option<Variable>,
}

/// What code generation for a loop has besides [`Gen`]'s: the views, the
/// [`LoopCtx`], the vector registers live before each op, and where the
/// back-edge counter is stored.
struct LoopGen<'a> {
    vecs: CValue,
    ctx: CValue,
    vlive: &'a [Vec<bool>],
    counter: Option<usize>,
    vis: usize,
    vis_at: &'a [Vis],
}

struct Gen<'a, 'b> {
    b: FunctionBuilder<'b>,
    v: Vars,
    slots: &'a [SlotInfo],
    states: Vec<Option<RegState>>,
    inl: &'a Inl<'a>,
    /// Inside an inlined callee: leave at this op (the call) instead, and
    /// where a `Return` stores the result and continues.
    deopt_at: Option<usize>,
    ret: Option<(Variable, Block)>,
    reg_idx: &'a HashMap<Reg, usize>,
    vals: CValue,
    blocks: Vec<Block>,
    start: usize,
    /// The op being translated, and the types of the registers it defines.
    cur: usize,
    cur_defs: Vec<(usize, Option<JType>)>,
    deopt: HashMap<usize, Block>,
    exit: Block,
    helpers: HashMap<&'static str, cranelift_codegen::ir::FuncRef>,
    /// Compiling a closure whole.
    whole: Option<closure::Whole<'a>>,
    /// Compiling a loop.
    lp: Option<LoopGen<'a>>,
}

const NA_I32: i64 = NA_INTEGER as i64;

impl Gen<'_, '_> {
    fn block(&self, pc: usize) -> Block {
        self.blocks[pc - self.start]
    }

    fn state(&self, pc: usize) -> &RegState {
        self.states[pc - self.start].as_ref().unwrap()
    }

    /// Store the variables, and the registers live at op `pc` (if any), to
    /// the value buffer. Returns those registers.
    fn store_all(&mut self, pc: Option<usize>) -> Vec<(Reg, JType)> {
        let mf = MemFlagsData::trusted();
        for s in self.slots {
            if let Some((var, _)) = self.v.slot.get(&s.slot) {
                let x = self.b.use_var(*var);
                self.b.ins().store(mf, x, self.vals, (s.idx * 8) as i32);
            }
            if let Some((a, b2, c)) = self.v.lp.rslot.get(&s.slot).copied() {
                for (k, var) in [a, b2, c].into_iter().enumerate() {
                    let x = self.b.use_var(var);
                    self.b.ins().store(mf, x, self.vals, ((s.idx + k) * 8) as i32);
                }
            }
            if s.owned {
                // (The pointer and capacity are the buffer's: see `h_reserve`.)
                let vecs = self.lp.as_ref().unwrap().vecs;
                let (_, lv, _) = self.v.vec[&s.slot];
                let x = self.b.use_var(lv);
                self.b.ins().store(mf, x, vecs, (s.idx * std::mem::size_of::<VecView>() + 8) as i32);
            }
            if let (Some(f), Some(idx)) = (self.v.flag.get(&s.slot), s.flag) {
                let x = self.b.use_var(*f);
                self.b.ins().store(mf, x, self.vals, (idx * 8) as i32);
            }
        }
        if let (Some(c), Some(idx)) = (self.v.lp.counter, self.lp.as_ref().and_then(|l| l.counter)) {
            let x = self.b.use_var(c);
            self.b.ins().store(mf, x, self.vals, (idx * 8) as i32);
        }
        if let Some(lp) = self.lp.as_ref() {
            // (At the end of the region for the exit.)
            let (vis, at) = (lp.vis, lp.vis_at[pc.unwrap_or(self.start + lp.vis_at.len() - 1) - self.start]);
            let x = match (at, self.v.lp.vis) {
                (Vis::Is(on), _) => Some(self.b.ins().iconst(types::I64, on as i64)),
                (Vis::Varies, Some(c)) => Some(self.b.use_var(c)),
                _ => None,
            };
            if let Some(x) = x {
                self.b.ins().store(mf, x, self.vals, (vis * 8) as i32);
            }
        }
        let mut live = Vec::new();
        let Some(pc) = pc else { return live };
        let st = self.state(pc).clone();
        for (reg, t) in st.iter().enumerate() {
            let (Some(t), Some(idx)) = (*t, self.reg_idx.get(&(reg as Reg)).copied()) else { continue };
            if t.is_vector() {
                // Only a live one: a dead one may view a buffer since resized.
                let lp = self.lp.as_ref().unwrap();
                if !lp.vlive[pc - self.start][reg] {
                    continue;
                }
                let (pv, lv) = self.v.lp.vreg[&(reg as Reg, t)];
                for (k, var) in [pv, lv].into_iter().enumerate() {
                    let x = self.b.use_var(var);
                    self.b.ins().store(mf, x, self.vals, ((idx + k) * 8) as i32);
                }
                live.push((reg as Reg, t));
                continue;
            }
            let reg = reg as Reg;
            if t == JType::Range {
                let (a, b2, c) = self.v.range[&reg];
                for (k, var) in [a, b2, c].into_iter().enumerate() {
                    let x = self.b.use_var(var);
                    self.b.ins().store(mf, x, self.vals, ((idx + k) * 8) as i32);
                }
            } else {
                let var = self.v.reg[&(reg, t)];
                let x = self.b.use_var(var);
                self.b.ins().store(mf, x, self.vals, (idx * 8) as i32);
            }
            live.push((reg, t));
        }
        live
    }

    /// The block that leaves native code to execute op `pc` in the VM.
    fn deopt_block(&mut self, pc: usize) -> Block {
        if let Some(w) = &self.whole {
            return w.bail;
        }
        let pc = self.deopt_at.unwrap_or(pc);
        if let Some(b) = self.deopt.get(&pc) {
            return *b;
        }
        let blk = self.b.create_block();
        self.b.set_cold_block(blk);
        self.deopt.insert(pc, blk);
        blk
    }

    /// Continue if `cond` is false, else leave at op `pc`.
    fn deopt_if(&mut self, cond: CValue, pc: usize) {
        let d = self.deopt_block(pc);
        let cont = self.b.create_block();
        self.b.ins().brif(cond, d, &[], cont, &[]);
        self.b.switch_to_block(cont);
    }

    fn call(&mut self, name: &'static str, args: &[CValue]) -> CValue {
        let f = self.helpers[name];
        let c = self.b.ins().call(f, args);
        self.b.inst_results(c)[0]
    }

    fn f64c(&mut self, x: f64) -> CValue {
        self.b.ins().f64const(x)
    }

    fn i32c(&mut self, x: i64) -> CValue {
        self.b.ins().iconst(types::I32, x)
    }

    fn is_na_i(&mut self, x: CValue) -> CValue {
        self.b.ins().icmp_imm_s(IntCC::Equal, x, NA_I32)
    }

    fn leaf(&mut self, l: &Leaf) -> (CValue, JType) {
        match l {
            Leaf::D(x) => (self.f64c(*x), JType::D),
            Leaf::I(x) => (self.i32c(*x as i64), JType::I),
            Leaf::Var(_, s) => self.var(*s),
            Leaf::R(r) => self.reg(*r),
        }
    }

    /// The value of variable `s`.
    fn var(&mut self, s: Slot) -> (CValue, JType) {
        if self.whole.is_some()
            && let Some(x) = self.whole_var(s)
        {
            return x;
        }
        let (var, t) = self.v.slot[&s];
        (self.b.use_var(var), t)
    }

    /// Register `r` before the current op.
    fn reg(&mut self, r: Reg) -> (CValue, JType) {
        let t = self.state(self.cur)[r as usize].unwrap();
        if t == JType::Dyn {
            return (self.dyn_reg(r), t);
        }
        let var = self.v.reg[&(r, t)];
        (self.b.use_var(var), t)
    }

    /// Define register `r` (with the type the current op gives it).
    fn set_reg(&mut self, r: Reg, x: CValue) {
        let t = self.cur_defs.iter().find(|(k, _)| *k == r as usize).and_then(|(_, t)| *t).unwrap();
        if t == JType::Dyn {
            // (Written in place, through `dyn_reg`.)
            return;
        }
        let var = self.v.reg[&(r, t)];
        self.b.def_var(var, x);
    }

    /// `int_to_real`: NA becomes NA_REAL.
    fn to_f64(&mut self, x: CValue, t: JType) -> CValue {
        if t == JType::D {
            return x;
        }
        let na = self.is_na_i(x);
        let f = self.b.ins().fcvt_from_sint(types::F64, x);
        let nar = self.f64c(NA_REAL);
        self.b.ins().select(na, nar, f)
    }

    /// The value of argument `k` of the call at `pc`.
    fn call_arg(&mut self, k: usize, a: &CallArg, pc: usize) -> (CValue, JType) {
        match a {
            CallArg::Leaf(l) => self.leaf(l),
            CallArg::Arith(op, x, y) => {
                let (x, y) = (self.leaf(x), self.leaf(y));
                let t = arith_type(*op, x.1, y.1).unwrap();
                (self.arith(*op, x, y, pc), t)
            }
            CallArg::Index(x, i, two) => self.index_arg(k, *x, i, *two, pc),
            CallArg::Nested(n) => {
                let x = self.call_arg(k, &n.1, pc);
                match &n.2 {
                    Some(b) => {
                        let y = self.call_arg(k, b, pc);
                        let t = arith_type(n.0, x.1, y.1).unwrap();
                        (self.arith(n.0, x, y, pc), t)
                    }
                    // (As `Op::Neg`.)
                    None if x.1 == JType::D => (self.b.ins().fneg(x.0), JType::D),
                    None => {
                        let na = self.is_na_i(x.0);
                        let neg = self.b.ins().ineg(x.0);
                        (self.b.ins().select(na, x.0, neg), JType::I)
                    }
                }
            }
        }
    }

    /// `vm::arith_fast`; leaves at `pc` where it gives up.
    fn arith(&mut self, op: ArithOp, a: (CValue, JType), b: (CValue, JType), pc: usize) -> CValue {
        if a.1 == JType::D || b.1 == JType::D {
            let x = self.to_f64(a.0, a.1);
            let y = self.to_f64(b.0, b.1);
            return match op {
                ArithOp::Plus => self.b.ins().fadd(x, y),
                ArithOp::Minus => self.b.ins().fsub(x, y),
                ArithOp::Times => self.b.ins().fmul(x, y),
                ArithOp::Div => self.b.ins().fdiv(x, y),
                ArithOp::Pow => self.call("robigo_pow", &[x, y]),
                ArithOp::Mod => {
                    // The VM gives the "loss of accuracy" warning.
                    let w = self.call("robigo_fmod_warns", &[x, y]);
                    let warns = self.b.ins().icmp_imm_s(IntCC::NotEqual, w, 0);
                    self.deopt_if(warns, pc);
                    self.call("robigo_fmod", &[x, y])
                }
                ArithOp::IDiv => self.call("robigo_fidiv", &[x, y]),
            };
        }
        let (x, y) = (a.0, b.0);
        match op {
            ArithOp::Plus | ArithOp::Minus | ArithOp::Times => {
                let nx = self.is_na_i(x);
                let ny = self.is_na_i(y);
                let na = self.b.ins().bor(nx, ny);
                let wx = self.b.ins().sextend(types::I64, x);
                let wy = self.b.ins().sextend(types::I64, y);
                let w = match op {
                    ArithOp::Plus => self.b.ins().iadd(wx, wy),
                    ArithOp::Minus => self.b.ins().isub(wx, wy),
                    _ => self.b.ins().imul(wx, wy),
                };
                let hi = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, w, i32::MAX as i64);
                let lo = self.b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, w, i32::MIN as i64);
                let ovf = self.b.ins().bor(hi, lo);
                let notna = self.b.ins().bxor_imm_s(na, 1);
                let bad = self.b.ins().band(ovf, notna);
                // Overflow: NA with a warning, which the VM gives.
                self.deopt_if(bad, pc);
                let r = self.b.ins().ireduce(types::I32, w);
                let nai = self.i32c(NA_I32);
                self.b.ins().select(na, nai, r)
            }
            ArithOp::Div => {
                let nx = self.is_na_i(x);
                let ny = self.is_na_i(y);
                let na = self.b.ins().bor(nx, ny);
                let fx = self.b.ins().fcvt_from_sint(types::F64, x);
                let fy = self.b.ins().fcvt_from_sint(types::F64, y);
                let q = self.b.ins().fdiv(fx, fy);
                let nar = self.f64c(NA_REAL);
                self.b.ins().select(na, nar, q)
            }
            ArithOp::Pow => self.call("robigo_ipow", &[x, y]),
            ArithOp::Mod => self.call("robigo_imod", &[x, y]),
            ArithOp::IDiv => self.call("robigo_iidiv", &[x, y]),
        }
    }

    /// `vm::rel_fast`.
    fn rel(&mut self, op: RelOp, a: (CValue, JType), b: (CValue, JType)) -> CValue {
        let c = if a.1 == JType::D || b.1 == JType::D {
            let x = self.to_f64(a.0, a.1);
            let y = self.to_f64(b.0, b.1);
            let na = self.b.ins().fcmp(FloatCC::Unordered, x, y);
            let cc = match op {
                RelOp::Eq => FloatCC::Equal,
                RelOp::Ne => FloatCC::NotEqual,
                RelOp::Lt => FloatCC::LessThan,
                RelOp::Le => FloatCC::LessThanOrEqual,
                RelOp::Gt => FloatCC::GreaterThan,
                RelOp::Ge => FloatCC::GreaterThanOrEqual,
            };
            let t = self.b.ins().fcmp(cc, x, y);
            (na, t)
        } else {
            let nx = self.is_na_i(a.0);
            let ny = self.is_na_i(b.0);
            let na = self.b.ins().bor(nx, ny);
            let cc = match op {
                RelOp::Eq => IntCC::Equal,
                RelOp::Ne => IntCC::NotEqual,
                RelOp::Lt => IntCC::SignedLessThan,
                RelOp::Le => IntCC::SignedLessThanOrEqual,
                RelOp::Gt => IntCC::SignedGreaterThan,
                RelOp::Ge => IntCC::SignedGreaterThanOrEqual,
            };
            let t = self.b.ins().icmp(cc, a.0, b.0);
            (na, t)
        };
        let t = self.b.ins().uextend(types::I32, c.1);
        let na = self.i32c(NA_LOGICAL as i64);
        self.b.ins().select(c.0, na, t)
    }

    /// A 0-based index from `i` (`vm::scalar_index`), checked against the
    /// length; leaves at `pc` otherwise.
    fn index(&mut self, i: (CValue, JType), len: CValue, pc: usize) -> CValue {
        let f = match i.1 {
            JType::D => i.0,
            _ => {
                let na = self.is_na_i(i.0);
                self.deopt_if(na, pc);
                self.b.ins().fcvt_from_sint(types::F64, i.0)
            }
        };
        let one = self.f64c(1.0);
        let lim = self.f64c(4503599627370496.0);
        let ge = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, f, one);
        let lt = self.b.ins().fcmp(FloatCC::LessThan, f, lim);
        let ok = self.b.ins().band(ge, lt);
        let bad = self.b.ins().bxor_imm_s(ok, 1);
        self.deopt_if(bad, pc);
        let t = self.b.ins().trunc(f);
        let k = self.b.ins().fcvt_to_sint_sat(types::I64, t);
        let k = self.b.ins().iadd_imm_s(k, -1);
        let out = self.b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, k, len);
        self.deopt_if(out, pc);
        k
    }

    fn element_offset(&mut self, op: &Op, slot: Slot, lv: Variable, pc: usize) -> CValue {
        let (i, j) = match op {
            Op::Index { i, .. } | Op::SetIndex { i, .. } => (i, None),
            Op::Index2 { i, j, .. } | Op::SetIndex2 { i, j, .. } => (i, Some(j)),
            _ => unreachable!(),
        };
        let iv = self.leaf(i);
        if let Some(j) = j {
            let (nr, nc) = self.v.dims[&slot];
            let nr = self.b.use_var(nr);
            let nc = self.b.use_var(nc);
            let row = self.index(iv, nr, pc);
            let jv = self.leaf(j);
            let col = self.index(jv, nc, pc);
            // R: subset.c:MATRIX_SUBSET_LOOP; subassign.c:MatrixAssign
            let off = self.b.ins().imul(col, nr);
            self.b.ins().iadd(row, off)
        } else {
            let len = self.b.use_var(lv);
            self.index(iv, len, pc)
        }
    }

    fn op(&mut self, pc: usize, op: &Op) {
        let next = pc + 1;
        let mut falls = true;
        // (As the VM's: an invisible value of a call before it must not last.)
        if self.whole.is_some()
            && matches!(
                op,
                Op::GetVar { .. }
                    | Op::Index { .. }
                    | Op::Index2 { .. }
                    | Op::Builtin1 { .. }
                    | Op::Subset { .. }
                    | Op::CallBuiltin { .. }
            )
        {
            self.set_visible(true);
        }
        if self.lp.is_some() && self.deopt_at.is_none() {
            let back = match op {
                Op::Jump { target } | Op::JumpIf { target, .. } | Op::JumpIfNot { target, .. } => {
                    *target as usize <= pc
                }
                Op::ForStep { body, .. } => *body as usize <= pc,
                _ => false,
            };
            if let (true, Some(c)) = (back, self.v.lp.counter) {
                let x = self.b.use_var(c);
                let x = self.b.ins().iadd_imm_s(x, 1);
                self.b.def_var(c, x);
            }
            if let (Some(on), Some(c)) = (sets_visible(op), self.v.lp.vis) {
                let x = self.b.ins().iconst(types::I64, on as i64);
                self.b.def_var(c, x);
            }
            if self.loop_op(pc, op) {
                let n = self.target(next);
                self.b.ins().jump(n, &[]);
                return;
            }
        }
        if self.whole.is_some() {
            match self.dyn_op(pc, op) {
                Some(true) => {
                    let n = self.target(next);
                    self.b.ins().jump(n, &[]);
                    return;
                }
                Some(false) => return,
                None => {}
            }
        }
        match op {
            Op::ConstD { dst, x } => {
                let v = self.f64c(*x);
                self.set_reg(*dst, v);
            }
            Op::ConstI { dst, x } => {
                let v = self.i32c(*x as i64);
                self.set_reg(*dst, v);
            }
            Op::ConstL { dst, x } => {
                let v = self.i32c(*x as i64);
                self.set_reg(*dst, v);
            }
            Op::GetVar { dst, slot, .. } => {
                let (x, _) = self.var(*slot);
                self.set_reg(*dst, x);
            }
            Op::Visible { on } if self.whole.is_some() => self.set_visible(*on),
            Op::SuperSet { src, slot, .. } => {
                let (x, _) = self.reg(*src);
                self.super_set(*slot, x);
            }
            Op::Builtin1 { f, dst, a, .. } => {
                let (x, t) = self.reg(*a);
                let v = self.builtin1(*f, x, t, pc);
                self.set_reg(*dst, v);
            }
            Op::SetVar { src, slot, .. } => {
                let (x, _) = self.reg(*src);
                let (var, _) = self.v.slot[slot];
                self.b.def_var(var, x);
                if let Some(f) = self.v.flag.get(slot).copied() {
                    let one = self.b.ins().iconst(types::I64, 1);
                    self.b.def_var(f, one);
                }
            }
            Op::Arith { op, dst, a, b, .. } => {
                let (x, y) = (self.reg(*a), self.reg(*b));
                let v = self.arith(*op, x, y, pc);
                self.set_reg(*dst, v);
            }
            Op::ArithL { op, dst, a, b, .. } => {
                let (x, y) = (self.leaf(a), self.leaf(b));
                let v = self.arith(*op, x, y, pc);
                self.set_reg(*dst, v);
            }
            Op::Rel { op, dst, a, b, .. } => {
                let (x, y) = (self.reg(*a), self.reg(*b));
                let v = self.rel(*op, x, y);
                self.set_reg(*dst, v);
            }
            Op::RelL { op, dst, a, b, .. } => {
                let (x, y) = (self.leaf(a), self.leaf(b));
                let v = self.rel(*op, x, y);
                self.set_reg(*dst, v);
            }
            Op::Neg { dst, a, .. } => {
                let (x, t) = self.reg(*a);
                let v = if t == JType::D {
                    self.b.ins().fneg(x)
                } else {
                    let na = self.is_na_i(x);
                    let n = self.b.ins().ineg(x);
                    self.b.ins().select(na, x, n)
                };
                self.set_reg(*dst, v);
            }
            Op::Not { dst, a, .. } => {
                let (x, t) = self.reg(*a);
                let (na, z) = if t == JType::D {
                    let na = self.b.ins().fcmp(FloatCC::Unordered, x, x);
                    let zero = self.f64c(0.0);
                    (na, self.b.ins().fcmp(FloatCC::Equal, x, zero))
                } else {
                    let na = self.is_na_i(x);
                    (na, self.b.ins().icmp_imm_s(IntCC::Equal, x, 0))
                };
                let z = self.b.ins().uextend(types::I32, z);
                let nal = self.i32c(NA_LOGICAL as i64);
                let v = self.b.ins().select(na, nal, z);
                self.set_reg(*dst, v);
            }
            Op::Math1 { f, dst, a, .. } => {
                let (x, t) = self.reg(*a);
                let v = if *f == Math1::Abs {
                    if t == JType::D {
                        self.b.ins().fabs(x)
                    } else {
                        let na = self.is_na_i(x);
                        let ab = self.b.ins().iabs(x);
                        self.b.ins().select(na, x, ab)
                    }
                } else {
                    let d = self.to_f64(x, t);
                    let r = match f {
                        Math1::Sqrt => self.b.ins().sqrt(d),
                        Math1::Floor => self.b.ins().floor(d),
                        Math1::Ceiling => self.b.ins().ceil(d),
                        Math1::Exp => self.call("robigo_exp", &[d]),
                        Math1::Sin => self.call("robigo_sin", &[d]),
                        Math1::Cos => self.call("robigo_cos", &[d]),
                        Math1::Tan => self.call("robigo_tan", &[d]),
                        Math1::Abs => unreachable!(),
                    };
                    // NaN from a number: the builtin warns.
                    let rn = self.b.ins().fcmp(FloatCC::Unordered, r, r);
                    let dok = self.b.ins().fcmp(FloatCC::Ordered, d, d);
                    let bad = self.b.ins().band(rn, dok);
                    self.deopt_if(bad, pc);
                    r
                };
                self.set_reg(*dst, v);
            }
            Op::AndOrArg { dst, a, .. } => {
                let (x, t) = self.reg(*a);
                let v = match t {
                    JType::L => x,
                    JType::I => {
                        let na = self.is_na_i(x);
                        let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, x, 0);
                        let nz = self.b.ins().uextend(types::I32, nz);
                        let nal = self.i32c(NA_LOGICAL as i64);
                        self.b.ins().select(na, nal, nz)
                    }
                    _ => {
                        let na = self.b.ins().fcmp(FloatCC::Unordered, x, x);
                        let zero = self.f64c(0.0);
                        let nz = self.b.ins().fcmp(FloatCC::NotEqual, x, zero);
                        let nz = self.b.ins().uextend(types::I32, nz);
                        let nal = self.i32c(NA_LOGICAL as i64);
                        self.b.ins().select(na, nal, nz)
                    }
                };
                self.set_reg(*dst, v);
            }
            Op::AndOrShort { x, dst, and, target } => {
                let (xv, _) = self.reg(*x);
                let want = if *and { 0 } else { 1 };
                let hit = self.b.ins().icmp_imm_s(IntCC::Equal, xv, want);
                let take = self.b.create_block();
                let cont = self.b.create_block();
                self.b.ins().brif(hit, take, &[], cont, &[]);
                self.b.switch_to_block(take);
                let c = self.i32c(want);
                self.set_reg(*dst, c);
                let t = self.target(*target as usize);
                self.b.ins().jump(t, &[]);
                self.b.switch_to_block(cont);
            }
            Op::AndOr { dst, x, y, and } => {
                let (x1, _) = self.reg(*x);
                let (x2, _) = self.reg(*y);
                // R: logic.c:do_logic2 (see the VM)
                let na1 = self.is_na_i(x1);
                let na2 = self.is_na_i(x2);
                let other = if *and {
                    self.b.ins().icmp_imm_s(IntCC::NotEqual, x2, 0)
                } else {
                    self.b.ins().icmp_imm_s(IntCC::Equal, x2, 0)
                };
                let to_na = self.b.ins().bor(na2, other);
                let nal = self.i32c(NA_LOGICAL as i64);
                let when_na1 = self.b.ins().select(to_na, nal, x2);
                let v = self.b.ins().select(na1, when_na1, x2);
                self.set_reg(*dst, v);
            }
            Op::JumpIf { cond, target, .. } | Op::JumpIfNot { cond, target, .. } => {
                let (c, t) = self.reg(*cond);
                // NA: the VM signals the error.
                let c = if t == JType::D {
                    // R: coerce.c:asLogical2 (`x != 0`, NaN as NA)
                    let na = self.b.ins().fcmp(FloatCC::Unordered, c, c);
                    self.deopt_if(na, pc);
                    let zero = self.f64c(0.0);
                    self.b.ins().fcmp(FloatCC::NotEqual, c, zero)
                } else {
                    let na = self.is_na_i(c);
                    self.deopt_if(na, pc);
                    c
                };
                let t = self.target(*target as usize);
                let n = self.target(next);
                if matches!(op, Op::JumpIf { .. }) {
                    self.b.ins().brif(c, t, &[], n, &[]);
                } else {
                    self.b.ins().brif(c, n, &[], t, &[]);
                }
                falls = false;
            }
            Op::Jump { target } => {
                let t = self.target(*target as usize);
                self.b.ins().jump(t, &[]);
                falls = false;
            }
            Op::ForRange { kind, a, b, seq, .. } => {
                let (start, step, n) = match kind {
                    RangeKind::Colon => {
                        let (x, tx) = self.reg(*a);
                        let (y, ty) = self.reg(*b);
                        let x = self.to_f64(x, tx);
                        let y = self.to_f64(y, ty);
                        // vm::colon_range: integer-valued ends in range.
                        let ok_x = self.int_valued(x);
                        let ok_y = self.int_valued(y);
                        let ok = self.b.ins().band(ok_x, ok_y);
                        let bad = self.b.ins().bxor_imm_s(ok, 1);
                        self.deopt_if(bad, pc);
                        let xi = self.b.ins().fcvt_to_sint_sat(types::I64, x);
                        let yi = self.b.ins().fcvt_to_sint_sat(types::I64, y);
                        let up = self.b.ins().icmp(IntCC::SignedLessThanOrEqual, xi, yi);
                        let one = self.b.ins().iconst(types::I64, 1);
                        let mone = self.b.ins().iconst(types::I64, -1);
                        let step = self.b.ins().select(up, one, mone);
                        let d = self.b.ins().isub(yi, xi);
                        let ad = self.b.ins().iabs(d);
                        let n = self.b.ins().iadd_imm_s(ad, 1);
                        (xi, step, n)
                    }
                    _ => {
                        // seq_len
                        let (x, t) = self.reg(*a);
                        let n = if t == JType::D {
                            let zero = self.f64c(0.0);
                            let lim = self.f64c(2147483647.0);
                            let ge = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, x, zero);
                            let lt = self.b.ins().fcmp(FloatCC::LessThan, x, lim);
                            let ok = self.b.ins().band(ge, lt);
                            let bad = self.b.ins().bxor_imm_s(ok, 1);
                            self.deopt_if(bad, pc);
                            self.b.ins().fcvt_to_sint_sat(types::I64, x)
                        } else {
                            let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, x, 0);
                            self.deopt_if(neg, pc);
                            self.b.ins().sextend(types::I64, x)
                        };
                        let one = self.b.ins().iconst(types::I64, 1);
                        (one, one, n)
                    }
                };
                let (vs, vt, vn) = self.v.range[seq];
                self.b.def_var(vs, start);
                self.b.def_var(vt, step);
                self.b.def_var(vn, n);
            }
            Op::ForInit { seq, .. } => {
                let z = self.i32c(0);
                self.set_reg(*seq + 1, z);
            }
            Op::ForStep { seq, idx, slot, body, .. } => {
                let (vs, vt, vn) = self.v.range[seq];
                let (iv, _) = self.reg(*idx);
                let i = self.b.ins().sextend(types::I64, iv);
                let n = self.b.use_var(vn);
                let done = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, i, n);
                let fin = self.b.create_block();
                let step = self.b.create_block();
                self.b.ins().brif(done, fin, &[], step, &[]);
                // Finished: an empty sequence leaves the variable NULL,
                // which the VM sets.
                self.b.switch_to_block(fin);
                let first = self.b.ins().icmp_imm_s(IntCC::Equal, i, 0);
                self.deopt_if(first, pc);
                let nb = self.target(next);
                self.b.ins().jump(nb, &[]);
                self.b.switch_to_block(step);
                let i1 = self.b.ins().iadd_imm_s(i, 1);
                let i1 = self.b.ins().ireduce(types::I32, i1);
                self.set_reg(*idx, i1);
                let s = self.b.use_var(vs);
                let st = self.b.use_var(vt);
                let off = self.b.ins().imul(i, st);
                let e = self.b.ins().iadd(s, off);
                let e = self.b.ins().ireduce(types::I32, e);
                let (var, _) = self.v.slot[slot];
                self.b.def_var(var, e);
                if let Some(f) = self.v.flag.get(slot).copied() {
                    let one = self.b.ins().iconst(types::I64, 1);
                    self.b.def_var(f, one);
                }
                let bb = self.target(*body as usize);
                self.b.ins().jump(bb, &[]);
                falls = false;
            }
            Op::Index { dst, slot, .. } | Op::Index2 { dst, slot, .. } => {
                let (pv, lv, t) = self.v.vec[slot];
                let k = self.element_offset(op, *slot, lv, pc);
                let p = self.b.use_var(pv);
                let (sz, ty) = if matches!(t, JType::VecD | JType::MatD) { (8, types::F64) } else { (4, types::I32) };
                let off = self.b.ins().imul_imm_s(k, sz);
                let addr = self.b.ins().iadd(p, off);
                let x = self.b.ins().load(ty, MemFlagsData::trusted(), addr, 0);
                self.set_reg(*dst, x);
            }
            Op::SetIndex { val, slot, .. } | Op::SetIndex2 { val, slot, .. } => {
                let (pv, lv, t) = self.v.vec[slot];
                let k = self.element_offset(op, *slot, lv, pc);
                let (x, xt) = self.reg(*val);
                let (sz, x) = if matches!(t, JType::VecD | JType::MatD) { (8, self.to_f64(x, xt)) } else { (4, x) };
                let p = self.b.use_var(pv);
                let off = self.b.ins().imul_imm_s(k, sz);
                let addr = self.b.ins().iadd(p, off);
                self.b.ins().store(MemFlagsData::trusted(), x, addr, 0);
            }
            Op::Call { dst, site, .. } => {
                let ic = &self.inl.calls[site];
                let (code, formal_slots, arg_list) = (ic.code.clone(), ic.formal_slots.clone(), ic.args.clone());
                let args: Vec<(CValue, JType)> =
                    arg_list.iter().enumerate().map(|(k, a)| self.call_arg(k, a, pc)).collect();
                if self.whole.is_some() {
                    let v = self.real_call(*site, *dst, &args);
                    self.set_reg(*dst, v);
                    let n = self.target(next);
                    self.b.ins().jump(n, &[]);
                    return;
                }
                let args: Vec<(CValue, JType)> = args;
                let ct = self.inl.callee(*site, args.iter().map(|a| a.1).collect()).unwrap();
                let v = self.inline_body(pc, &code, &formal_slots, &ct, &args);
                self.set_reg(*dst, v);
            }
            Op::Return { src } if self.whole.is_some() => {
                let (x, _) = self.reg(*src);
                self.return_value(x);
                falls = false;
            }
            Op::Return { src } => {
                let (rv, after) = self.ret.expect("Return outside an inlined callee");
                let (x, _) = self.reg(*src);
                self.b.def_var(rv, x);
                self.b.ins().jump(after, &[]);
                falls = false;
            }
            // Checked at entry, or no effect in native code (a cleared
            // register is dead: see `defs`).
            Op::Guard { .. }
            | Op::Visible { .. }
            | Op::LoopEnter { .. }
            | Op::LoopExit
            | Op::Null { dead: true, .. } => {}
            _ => unreachable!("unsupported op in native loop: {:?}", op),
        }
        if falls {
            let n = self.target(next);
            self.b.ins().jump(n, &[]);
        }
    }

    /// The body of a pure closure called at op `call` with argument values
    /// `args`, translated in place; its result. Anything its fast paths do
    /// not cover leaves native code at the call, which the VM then makes.
    fn inline_body(
        &mut self,
        call: usize,
        code: &Code,
        formal_slots: &[Option<Slot>],
        ct: &CalleeTy,
        args: &[(CValue, JType)],
    ) -> CValue {
        let len = code.ops.len();
        let res = self.b.declare_var(cl_type(ct.result));
        let after = self.b.create_block();
        // The callee's frame: its formals hold the arguments; its registers
        // get variables of their own.
        let mut slot = HashMap::new();
        for (k, s) in formal_slots.iter().enumerate() {
            if let Some(s) = s {
                let (x, t) = args[k];
                let var = self.b.declare_var(cl_type(t));
                self.b.def_var(var, x);
                slot.insert(*s, (var, t));
            }
        }
        let mut pairs: Vec<(Reg, JType)> = Vec::new();
        for (pc, st) in ct.states.iter().enumerate() {
            let Some(st) = st else { continue };
            for (reg, t) in st.iter().enumerate() {
                if let Some(t) = t {
                    pairs.push((reg as Reg, *t));
                }
            }
            if pc < len {
                for (reg, t) in defs(&code.ops[pc], st, &ct.slot_ty, self.inl) {
                    if let Some(t) = t {
                        pairs.push((reg as Reg, t));
                    }
                }
            }
        }
        pairs.sort_by_key(|(r, t)| (*r, *t as u8));
        pairs.dedup();
        let mut reg = HashMap::new();
        for (r, t) in pairs {
            let var = self.b.declare_var(cl_type(t));
            let z = if t == JType::D { self.f64c(0.0) } else { self.i32c(0) };
            self.b.def_var(var, z);
            reg.insert((r, t), var);
        }
        let blocks: Vec<Block> = (0..=len).map(|_| self.b.create_block()).collect();
        let end = self.b.create_block();
        self.b.ins().jump(blocks[0], &[]);
        let saved_states = std::mem::replace(&mut self.states, ct.states.clone());
        let saved_reg = std::mem::replace(&mut self.v.reg, reg);
        let saved_slot = std::mem::replace(&mut self.v.slot, slot);
        let saved_flag = std::mem::take(&mut self.v.flag);
        let saved_blocks = std::mem::replace(&mut self.blocks, blocks);
        let (saved_start, saved_cur, saved_exit, saved_deopt, saved_ret) =
            (self.start, self.cur, self.exit, self.deopt_at, self.ret);
        let saved_defs = std::mem::take(&mut self.cur_defs);
        self.start = 0;
        self.exit = end;
        self.deopt_at = Some(call);
        self.ret = Some((res, after));
        for pc in 0..len {
            let blk = self.block(pc);
            self.b.switch_to_block(blk);
            if self.states[pc].is_none() {
                let d = self.deopt_block(pc);
                self.b.ins().jump(d, &[]);
                continue;
            }
            self.cur = pc;
            self.cur_defs = defs(&code.ops[pc], self.state(pc), &ct.slot_ty, self.inl);
            self.op(pc, &code.ops[pc]);
        }
        let last = self.block(len);
        self.b.switch_to_block(last);
        self.b.ins().jump(end, &[]);
        // Falling off the end: the body's result register.
        self.b.switch_to_block(end);
        if self.states[len].is_some() {
            self.cur = len;
            let (x, _) = self.reg(code.result);
            self.b.def_var(res, x);
            self.b.ins().jump(after, &[]);
        } else {
            let d = self.deopt_block(len);
            self.b.ins().jump(d, &[]);
        }
        self.states = saved_states;
        self.v.reg = saved_reg;
        self.v.slot = saved_slot;
        self.v.flag = saved_flag;
        self.blocks = saved_blocks;
        self.start = saved_start;
        self.cur = saved_cur;
        self.exit = saved_exit;
        self.deopt_at = saved_deopt;
        self.ret = saved_ret;
        self.cur_defs = saved_defs;
        self.b.switch_to_block(after);
        self.b.use_var(res)
    }

    /// The ops of a loop on its range and vector variables and registers,
    /// and `max`/`min`; whether `op` was one.
    fn loop_op(&mut self, pc: usize, op: &Op) -> bool {
        let mf = MemFlagsData::trusted();
        match op {
            Op::GetVar { dst, slot, .. } => {
                if let Some((a, b2, c)) = self.v.lp.rslot.get(slot).copied() {
                    let (x, y, z) = (self.b.use_var(a), self.b.use_var(b2), self.b.use_var(c));
                    self.set_range(*dst, x, y, z);
                    return true;
                }
                if let Some((pv, lv, _)) = self.v.vec.get(slot).copied() {
                    let (p, l) = (self.b.use_var(pv), self.b.use_var(lv));
                    self.set_vreg(*dst, p, l);
                    return true;
                }
                false
            }
            Op::SetVar { src, slot, .. } => {
                if let Some((a, b2, c)) = self.v.lp.rslot.get(slot).copied() {
                    let (x, y, z) = self.range_reg(*src);
                    self.b.def_var(a, x);
                    self.b.def_var(b2, y);
                    self.b.def_var(c, z);
                } else if self.v.vec.contains_key(slot) {
                    let (p, l, t) = self.vreg(*src);
                    self.assign_vector(*slot, p, l, t);
                } else {
                    return false;
                }
                if let Some(f) = self.v.flag.get(slot).copied() {
                    let one = self.b.ins().iconst(types::I64, 1);
                    self.b.def_var(f, one);
                }
                true
            }
            Op::Builtin1 { f, dst, a, .. } => {
                let t = self.state(self.cur)[*a as usize].unwrap();
                if t.is_scalar() {
                    return false;
                }
                let v = match f {
                    Builtin1::Length => {
                        let len = if t == JType::Range { self.range_reg(*a).2 } else { self.vreg(*a).1 };
                        let big = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, len, i32::MAX as i64);
                        self.deopt_if(big, pc);
                        self.b.ins().ireduce(types::I32, len)
                    }
                    Builtin1::IsNull => self.i32c(0),
                    _ => unreachable!(),
                };
                self.set_reg(*dst, v);
                true
            }
            Op::CallBuiltin { dst, args, .. } => {
                let (is_max, regs) = self.inl.minmax[args].clone();
                let rt = self.cur_defs.iter().find(|(k, _)| *k == *dst as usize).and_then(|(_, t)| *t).unwrap();
                // R: summary.c:do_summary. NA or NaN: the VM computes it.
                // Ties keep the first (`x > cum` updates).
                let mut acc: Option<CValue> = None;
                for r in regs {
                    let (x, t) = self.reg(r);
                    let x = if rt == JType::D {
                        let na =
                            if t == JType::D { self.b.ins().fcmp(FloatCC::Unordered, x, x) } else { self.is_na_i(x) };
                        self.deopt_if(na, pc);
                        self.to_f64(x, t)
                    } else {
                        let na = self.is_na_i(x);
                        self.deopt_if(na, pc);
                        x
                    };
                    acc = Some(match acc {
                        None => x,
                        Some(a) => {
                            let better = if rt == JType::D {
                                self.b.ins().fcmp(if is_max { FloatCC::GreaterThan } else { FloatCC::LessThan }, x, a)
                            } else {
                                let cc = if is_max { IntCC::SignedGreaterThan } else { IntCC::SignedLessThan };
                                self.b.ins().icmp(cc, x, a)
                            };
                            self.b.ins().select(better, x, a)
                        }
                    });
                }
                self.set_reg(*dst, acc.unwrap());
                let _ = mf;
                true
            }
            _ => false,
        }
    }

    /// Define the range register `r`.
    fn set_range(&mut self, r: Reg, start: CValue, step: CValue, n: CValue) {
        let (vs, vt, vn) = self.v.range[&r];
        self.b.def_var(vs, start);
        self.b.def_var(vt, step);
        self.b.def_var(vn, n);
    }

    /// The range register `r` before the current op.
    fn range_reg(&mut self, r: Reg) -> (CValue, CValue, CValue) {
        let (vs, vt, vn) = self.v.range[&r];
        (self.b.use_var(vs), self.b.use_var(vt), self.b.use_var(vn))
    }

    /// Define the vector register `r` (with the type the current op gives it).
    fn set_vreg(&mut self, r: Reg, p: CValue, l: CValue) {
        let t = self.cur_defs.iter().find(|(k, _)| *k == r as usize).and_then(|(_, t)| *t).unwrap();
        let (pv, lv) = self.v.lp.vreg[&(r, t)];
        self.b.def_var(pv, p);
        self.b.def_var(lv, l);
    }

    /// The vector register `r` before the current op: pointer, length, type.
    fn vreg(&mut self, r: Reg) -> (CValue, CValue, JType) {
        let t = self.state(self.cur)[r as usize].unwrap();
        let (pv, lv) = self.v.lp.vreg[&(r, t)];
        (self.b.use_var(pv), self.b.use_var(lv), t)
    }

    /// `x <- v` for an owned vector `x` (`slot`) and a vector `v` of the
    /// same type: copy into `x`'s buffer, made larger first if need be.
    fn assign_vector(&mut self, slot: Slot, p: CValue, n: CValue, t: JType) {
        let (pv, lv, _) = self.v.vec[&slot];
        let cv = self.v.lp.cap[&slot];
        let idx = self.slots.iter().find(|s| s.slot == slot).unwrap().idx;
        let sz: i64 = if t == JType::VecD { 8 } else { 4 };
        let dst = self.b.use_var(pv);
        // (`x <- x`: nothing to copy.)
        let same = self.b.ins().icmp(IntCC::Equal, dst, p);
        let copy = self.b.create_block();
        let done = self.b.create_block();
        self.b.ins().brif(same, done, &[], copy, &[]);
        self.b.switch_to_block(copy);
        let cap = self.b.use_var(cv);
        let small = self.b.ins().icmp(IntCC::SignedGreaterThan, n, cap);
        let grow = self.b.create_block();
        let go = self.b.create_block();
        self.b.set_cold_block(grow);
        self.b.ins().brif(small, grow, &[], go, &[]);
        self.b.switch_to_block(grow);
        let lp = self.lp.as_ref().unwrap();
        let (vecs, ctx) = (lp.vecs, lp.ctx);
        let view = self.b.ins().iadd_imm_s(vecs, (idx * std::mem::size_of::<VecView>()) as i64);
        let np = self.call("robigo_reserve", &[ctx, view, n]);
        let nc = self.b.ins().load(types::I64, MemFlagsData::trusted(), view, 16);
        self.b.def_var(pv, np);
        self.b.def_var(cv, nc);
        self.b.ins().jump(go, &[]);
        self.b.switch_to_block(go);
        let dst = self.b.use_var(pv);
        self.copy_elements(dst, p, n, sz);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        self.b.def_var(lv, n);
    }

    /// Copy `n` elements of `sz` bytes from `src` to `dst` (not overlapping).
    fn copy_elements(&mut self, dst: CValue, src: CValue, n: CValue, sz: i64) {
        let ty = if sz == 8 { types::I64 } else { types::I32 };
        let head = self.b.create_block();
        let body = self.b.create_block();
        let out = self.b.create_block();
        self.b.append_block_param(head, types::I64);
        let z = self.b.ins().iconst(types::I64, 0);
        self.b.ins().jump(head, &[z.into()]);
        self.b.switch_to_block(head);
        let i = self.b.block_params(head)[0];
        let fin = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, i, n);
        self.b.ins().brif(fin, out, &[], body, &[]);
        self.b.switch_to_block(body);
        let off = self.b.ins().imul_imm_s(i, sz);
        let a = self.b.ins().iadd(src, off);
        let x = self.b.ins().load(ty, MemFlagsData::trusted(), a, 0);
        let d = self.b.ins().iadd(dst, off);
        self.b.ins().store(MemFlagsData::trusted(), x, d, 0);
        let i1 = self.b.ins().iadd_imm_s(i, 1);
        self.b.ins().jump(head, &[i1.into()]);
        self.b.switch_to_block(out);
    }

    /// `x == trunc(x) && INT_MIN < x <= INT_MAX` (false for NaN).
    fn int_valued(&mut self, x: CValue) -> CValue {
        let t = self.b.ins().trunc(x);
        let eq = self.b.ins().fcmp(FloatCC::Equal, x, t);
        let lo = self.f64c(i32::MIN as f64);
        let hi = self.f64c(i32::MAX as f64);
        let gt = self.b.ins().fcmp(FloatCC::GreaterThan, x, lo);
        let le = self.b.ins().fcmp(FloatCC::LessThanOrEqual, x, hi);
        let a = self.b.ins().band(eq, gt);
        self.b.ins().band(a, le)
    }

    /// The block for op `pc` (the exit block at the region's end).
    fn target(&self, pc: usize) -> Block {
        if pc == self.start + self.blocks.len() - 1 { self.exit } else { self.block(pc) }
    }
}

thread_local! {
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn codegen(
    code: &Code,
    r: &Region,
    lay: &Layout,
    deopt_regs: &mut HashMap<u32, Vec<(Reg, JType)>>,
    inl: &Inl,
) -> Result<LoopFn, Reject> {
    let (slots, slot_ty, states, reg_idx, live) = (lay.slots, lay.slot_ty, lay.states, lay.reg_idx, lay.live);
    MODULE.with(|m| {
        let mut m = m.borrow_mut();
        let module = m.get_or_insert_with(new_module);
        let ptr = module.target_config().pointer_type();
        let tcfg = module.target_config();
        let mut ctx = module.make_context();
        for _ in 0..3 {
            ctx.func.signature.params.push(AbiParam::new(ptr));
        }
        ctx.func.signature.returns.push(AbiParam::new(types::I32));
        let mut helper_ids: Vec<(&'static str, FuncId)> = Vec::new();
        for (name, _, params, ret) in HELPERS {
            let mut sig = module.make_signature();
            for p in *params {
                sig.params.push(AbiParam::new(*p));
            }
            sig.returns.push(AbiParam::new(*ret));
            let id = module.declare_function(name, Linkage::Import, &sig).map_err(|_| Reject::Never)?;
            helper_ids.push((name, id));
        }
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let mut helpers = HashMap::new();
            for (name, id) in &helper_ids {
                helpers.insert(*name, module.declare_func_in_func(*id, b.func));
            }
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let vals = b.block_params(entry)[0];
            let vecs = b.block_params(entry)[1];
            let lctx = b.block_params(entry)[2];
            let mf = MemFlagsData::trusted();
            let mut v = Vars {
                slot: HashMap::new(),
                vec: HashMap::new(),
                dims: HashMap::new(),
                flag: HashMap::new(),
                reg: HashMap::new(),
                range: HashMap::new(),
                lp: LoopVars::default(),
            };
            let view = |k: usize, field: usize| (k * std::mem::size_of::<VecView>() + 8 * field) as i32;
            for s in slots {
                if s.ty.is_scalar() || s.ty == JType::Range {
                    let n = if s.ty == JType::Range { 3 } else { 1 };
                    let ty = cl_type(s.ty);
                    let ty = if s.ty == JType::Range { types::I64 } else { ty };
                    let vs: Vec<Variable> = (0..n)
                        .map(|k| {
                            let var = b.declare_var(ty);
                            let x = b.ins().load(ty, mf, vals, ((s.idx + k) * 8) as i32);
                            b.def_var(var, x);
                            var
                        })
                        .collect();
                    if s.ty == JType::Range {
                        v.lp.rslot.insert(s.slot, (vs[0], vs[1], vs[2]));
                    } else {
                        v.slot.insert(s.slot, (vs[0], s.ty));
                    }
                    if let Some(fi) = s.flag {
                        let f = b.declare_var(types::I64);
                        let x = b.ins().load(types::I64, mf, vals, (fi * 8) as i32);
                        b.def_var(f, x);
                        v.flag.insert(s.slot, f);
                    }
                } else {
                    let pv = b.declare_var(ptr);
                    let lv = b.declare_var(types::I64);
                    let p = b.ins().load(ptr, mf, vecs, view(s.idx, 0));
                    let l = b.ins().load(types::I64, mf, vecs, view(s.idx, 1));
                    b.def_var(pv, p);
                    b.def_var(lv, l);
                    v.vec.insert(s.slot, (pv, lv, s.ty));
                    if s.owned {
                        let cv = b.declare_var(types::I64);
                        let c = b.ins().load(types::I64, mf, vecs, view(s.idx, 2));
                        b.def_var(cv, c);
                        v.lp.cap.insert(s.slot, cv);
                        if let Some(fi) = s.flag {
                            let f = b.declare_var(types::I64);
                            let x = b.ins().load(types::I64, mf, vals, (fi * 8) as i32);
                            b.def_var(f, x);
                            v.flag.insert(s.slot, f);
                        }
                    }
                    if matches!(s.ty, JType::MatD | JType::MatI) {
                        let nr = b.declare_var(types::I64);
                        let nc = b.declare_var(types::I64);
                        let r = b.ins().load(types::I64, mf, vecs, view(s.idx, 2));
                        let c = b.ins().load(types::I64, mf, vecs, view(s.idx, 3));
                        b.def_var(nr, r);
                        b.def_var(nc, c);
                        v.dims.insert(s.slot, (nr, nc));
                    }
                }
            }
            if lay.vis_at.contains(&Vis::Varies) {
                let c = b.declare_var(types::I64);
                let x = b.ins().load(types::I64, mf, vals, (lay.vis * 8) as i32);
                b.def_var(c, x);
                v.lp.vis = Some(c);
            }
            if lay.counter.is_some() {
                let c = b.declare_var(types::I64);
                let z = b.ins().iconst(types::I64, 0);
                b.def_var(c, z);
                v.lp.counter = Some(c);
            }
            // A variable for every (register, type) that occurs; live-ins
            // loaded, others zero.
            let mut pairs: Vec<(Reg, JType)> = Vec::new();
            for (k, st) in states.iter().enumerate() {
                let Some(st) = st else { continue };
                for (reg, t) in st.iter().enumerate() {
                    if let Some(t) = t {
                        pairs.push((reg as Reg, *t));
                    }
                }
                if r.start + k < r.end && !inl.leaves_at(r.start + k) {
                    for (reg, t) in defs(&code.ops[r.start + k], st, slot_ty, inl) {
                        if let Some(t) = t {
                            pairs.push((reg as Reg, t));
                        }
                    }
                }
            }
            pairs.sort_by_key(|(r, t)| (*r, *t as u8));
            pairs.dedup();
            for (reg, t) in pairs {
                let li = live.iter().any(|(r2, t2)| *r2 == reg && *t2 == t);
                let idx = reg_idx[&reg];
                if t == JType::Range {
                    if v.range.contains_key(&reg) {
                        continue;
                    }
                    let vs: Vec<Variable> = (0..3).map(|_| b.declare_var(types::I64)).collect();
                    for (k, var) in vs.iter().enumerate() {
                        let x = if li {
                            b.ins().load(types::I64, mf, vals, ((idx + k) * 8) as i32)
                        } else {
                            b.ins().iconst(types::I64, 0)
                        };
                        b.def_var(*var, x);
                    }
                    v.range.insert(reg, (vs[0], vs[1], vs[2]));
                } else if t.is_vector() {
                    let (pv, lv) = (b.declare_var(ptr), b.declare_var(types::I64));
                    let z = b.ins().iconst(types::I64, 0);
                    b.def_var(pv, z);
                    b.def_var(lv, z);
                    v.lp.vreg.insert((reg, t), (pv, lv));
                } else {
                    let ty = cl_type(t);
                    let var = b.declare_var(ty);
                    let x = if li {
                        b.ins().load(ty, mf, vals, (idx * 8) as i32)
                    } else if t == JType::D {
                        b.ins().f64const(0.0)
                    } else {
                        b.ins().iconst(types::I32, 0)
                    };
                    b.def_var(var, x);
                    v.reg.insert((reg, t), var);
                }
            }
            let blocks: Vec<Block> = (r.start..=r.end).map(|_| b.create_block()).collect();
            let exit = b.create_block();
            b.ins().jump(blocks[0], &[]);
            let mut g = Gen {
                b,
                v,
                slots,
                states: states.to_vec(),
                inl,
                deopt_at: None,
                ret: None,
                reg_idx,
                vals,
                blocks,
                start: r.start,
                cur: r.start,
                cur_defs: Vec::new(),
                deopt: HashMap::new(),
                exit,
                helpers,
                whole: None,
                lp: Some(LoopGen {
                    vecs,
                    ctx: lctx,
                    vlive: lay.vlive,
                    counter: lay.counter,
                    vis: lay.vis,
                    vis_at: lay.vis_at,
                }),
            };
            for pc in r.start..r.end {
                let blk = g.block(pc);
                g.b.switch_to_block(blk);
                if g.states[pc - r.start].is_none() || inl.leaves_at(pc) {
                    // Unreachable in native code, or an op it does not have.
                    let d = g.deopt_block(pc);
                    g.b.ins().jump(d, &[]);
                    continue;
                }
                g.cur = pc;
                g.cur_defs = defs(&code.ops[pc], g.state(pc), slot_ty, inl);
                g.op(pc, &code.ops[pc]);
            }
            let last = g.block(r.end);
            g.b.switch_to_block(last);
            g.b.ins().jump(exit, &[]);
            g.b.switch_to_block(exit);
            g.store_all(None);
            let ex = g.b.ins().iconst(types::I32, EXIT as i64);
            g.b.ins().return_(&[ex]);
            let mut deopts: Vec<(usize, Block)> = g.deopt.iter().map(|(k, v)| (*k, *v)).collect();
            deopts.sort();
            for (pc, blk) in deopts {
                g.b.switch_to_block(blk);
                let live = if g.states[pc - r.start].is_some() { g.store_all(Some(pc)) } else { g.store_all(None) };
                deopt_regs.insert(pc as u32, live);
                let p = g.b.ins().iconst(types::I32, pc as i64);
                g.b.ins().return_(&[p]);
            }
            g.b.seal_all_blocks();
            g.b.finalize(tcfg);
        }
        let n = NEXT.with(|c| {
            let v = c.get();
            c.set(v + 1);
            v
        });
        let id = module
            .declare_function(&format!("robigo_loop_{}", n), Linkage::Local, &ctx.func.signature)
            .map_err(|_| Reject::Never)?;
        if let Err(e) = module.define_function(id, &mut ctx) {
            if trace() {
                eprintln!("jit: codegen failed: {:?}", e);
            }
            return Err(Reject::Never);
        }
        module.clear_context(&mut ctx);
        module.finalize_definitions().map_err(|_| Reject::Never)?;
        let f = module.get_finalized_function(id);
        // SAFETY: the function was built with the `LoopFn` signature.
        Ok(unsafe { std::mem::transmute::<*const u8, LoopFn>(f) })
    })
}
