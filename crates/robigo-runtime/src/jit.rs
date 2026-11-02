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
use crate::compile::{Code, Leaf, Math1, NO_GUARD, Op, RangeKind, Reg, Slot};
use crate::sym::Sym;
use crate::value::{NA_INTEGER, NA_LOGICAL, NA_REAL};

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

type LoopFn = extern "C" fn(*mut u64, *const VecView) -> u32;

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
}

/// A call of a pure closure (see `vm::inline_info`) without free
/// variables, run inline in native code.
pub struct InlineCall {
    pub code: std::rc::Rc<Code>,
    /// The slot of each formal in the callee's code.
    pub formal_slots: Vec<Option<Slot>>,
    pub guard_syms: Vec<Sym>,
    /// The arguments (constants and variables of the caller).
    pub args: Vec<Leaf>,
    pub clo: std::rc::Rc<crate::value::Closure>,
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
}

impl<'a> Inl<'a> {
    pub fn new(calls: &'a HashMap<u32, InlineCall>) -> Inl<'a> {
        Inl { calls, memo: RefCell::new(HashMap::new()) }
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
            out[reg] = Some(t?);
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

fn uses(op: &Op, inl: &Inl) -> Uses {
    let mut u = Uses::default();
    match op {
        Op::Call { dst, site, .. } => {
            if let Some(ic) = inl.calls.get(site) {
                for a in &ic.args {
                    leaf_uses(a, &mut u);
                }
            }
            u.wregs.push(*dst);
        }
        Op::ConstD { dst, .. } | Op::ConstI { dst, .. } | Op::ConstL { dst, .. } => u.wregs.push(*dst),
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
        | Op::SetIndex2 { .. } => true,
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
    let inl = Inl::new(&calls);
    let Op::LoopEnter { brk, .. } = code.ops[enter] else { return Err(Reject::Never) };
    let r = Region { start: enter + 1, end: brk as usize };
    if r.end > code.ops.len() || r.start >= r.end {
        return Err(Reject::Never);
    }
    let ops = &code.ops[..];
    let mut nested = Vec::new();
    let mut guards = Vec::new();
    for pc in r.start..r.end {
        let op = &ops[pc];
        let inline_call = matches!(op, Op::Call { site, .. } if calls.contains_key(site));
        if !inline_call && !supported(op, &r) {
            if trace() {
                eprintln!("jit: unsupported at {}: {:?}", pc, op);
            }
            return Err(Reject::Never);
        }
        match op {
            Op::LoopEnter { brk, nxt } => nested.push((pc as u32, *brk, *nxt)),
            Op::Guard { sym, .. } => guards.push((*sym, pc)),
            Op::ArithL { sym, g, .. } | Op::RelL { sym, g, .. } if *g != NO_GUARD => guards.push((*sym, pc)),
            _ => {}
        }
    }

    // Variables have one type each; registers have a type per op (the VM
    // reuses them across statements), found by a forward analysis.
    let mut slot_ty: HashMap<Slot, JType> = HashMap::new();
    for pc in r.start..r.end {
        if let Op::Index { slot, .. }
        | Op::SetIndex { slot, .. }
        | Op::Index2 { slot, .. }
        | Op::SetIndex2 { slot, .. } = &ops[pc]
        {
            match entry(*slot) {
                Some(t) if t.is_vector() => {
                    slot_ty.insert(*slot, t);
                }
                _ => return Err(not_now(1)),
            }
        }
    }
    for pc in r.start..r.end {
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
    for pc in r.start..r.end {
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
    // Check every reachable op's operands.
    for pc in r.start..r.end {
        let Some(st) = &states[pc - r.start] else { continue };
        let op = &ops[pc];
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
        let rt = |reg: &Reg| st[*reg as usize].unwrap();
        let lt = |l: &Leaf| leaf_type(l, st, &slot_ty);
        let ok = match op {
            Op::JumpIf { cond, .. } | Op::JumpIfNot { cond, .. } => rt(cond) == JType::L,
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
            Op::GetVar { slot, .. } => slot_ty[slot].is_scalar(),
            _ => defs(op, st, &slot_ty, &inl).iter().all(|(_, t)| t.is_some()),
        };
        if !ok {
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
        if pc >= r.end {
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
    for pc in r.start..r.end {
        for s in uses(&ops[pc], &inl).wslots {
            written[s as usize] = true;
        }
        if let Op::SetIndex { slot, .. } | Op::SetIndex2 { slot, .. } = &ops[pc] {
            writable[*slot as usize] = true;
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
        let ne = needs_entry[s as usize] || ty.is_vector();
        if ne && entry(s) != Some(ty) {
            return Err(not_now(9));
        }
        let idx = if ty.is_scalar() {
            nvals += 1;
            nvals - 1
        } else {
            nvecs += 1;
            nvecs - 1
        };
        let flag = if written[s as usize] && !ne && ty.is_scalar() {
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
        });
    }
    let mut reg_idx = HashMap::new();
    for st in states.iter().flatten() {
        for (reg, t) in st.iter().enumerate() {
            if let Some(t) = t {
                if let std::collections::hash_map::Entry::Vacant(e) = reg_idx.entry(reg as Reg) {
                    e.insert(nvals);
                    nvals += if *t == JType::Range { 3 } else { 1 };
                }
            }
        }
    }
    // Registers only defined (never live at an op) still need a place.
    for pc in r.start..r.end {
        if let Some(st) = &states[pc - r.start] {
            for (reg, t) in defs(&ops[pc], st, &slot_ty, &inl) {
                if let (Some(t), std::collections::hash_map::Entry::Vacant(e)) = (t, reg_idx.entry(reg as Reg)) {
                    e.insert(nvals);
                    nvals += if t == JType::Range { 3 } else { 1 };
                }
            }
        }
    }

    let mut deopt_regs = HashMap::new();
    let func = codegen(code, &r, &slots, &slot_ty, &states, &reg_idx, &live, &mut deopt_regs, &inl)?;
    let calls = calls.into_iter().map(|(site, c)| (site, c.clo, c.guard_syms)).collect();
    Ok(JitLoop { func, slots, reg_idx, live_in: live, deopt_regs, nvals, nvecs, nested, guards, calls })
}

/// Register types before an op: `None` for undefined, or different types
/// on different paths.
type RegState = Vec<Option<JType>>;

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
    let rt = |r: &Reg| st[*r as usize];
    let scalar = |t: Option<JType>| t.filter(|t| t.is_scalar());
    match op {
        Op::Call { dst, site, .. } => {
            let t = inl.calls.get(site).and_then(|ic| {
                let tys: Option<Vec<JType>> = ic.args.iter().map(|a| leaf_type(a, st, slot_ty)).collect();
                inl.callee(*site, tys?).map(|c| c.result)
            });
            vec![(*dst as usize, t)]
        }
        Op::ConstD { dst, .. } => vec![(*dst as usize, Some(JType::D))],
        Op::ConstI { dst, .. } => vec![(*dst as usize, Some(JType::I))],
        Op::ConstL { dst, .. } => vec![(*dst as usize, Some(JType::L))],
        Op::GetVar { dst, slot, .. } => vec![(*dst as usize, scalar(slot_ty.get(slot).copied()))],
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
                    if !t.is_scalar() {
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
            if let (Some(f), Some(idx)) = (self.v.flag.get(&s.slot), s.flag) {
                let x = self.b.use_var(*f);
                self.b.ins().store(mf, x, self.vals, (idx * 8) as i32);
            }
        }
        let mut live = Vec::new();
        let Some(pc) = pc else { return live };
        let st = self.state(pc).clone();
        for (reg, t) in st.iter().enumerate() {
            let (Some(t), Some(idx)) = (*t, self.reg_idx.get(&(reg as Reg)).copied()) else { continue };
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
            Leaf::Var(_, s) => {
                let (var, t) = self.v.slot[s];
                (self.b.use_var(var), t)
            }
            Leaf::R(r) => self.reg(*r),
        }
    }

    /// Register `r` before the current op.
    fn reg(&mut self, r: Reg) -> (CValue, JType) {
        let t = self.state(self.cur)[r as usize].unwrap();
        let var = self.v.reg[&(r, t)];
        (self.b.use_var(var), t)
    }

    /// Define register `r` (with the type the current op gives it).
    fn set_reg(&mut self, r: Reg, x: CValue) {
        let t = self.cur_defs.iter().find(|(k, _)| *k == r as usize).and_then(|(_, t)| *t).unwrap();
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
                let (var, _) = self.v.slot[slot];
                let x = self.b.use_var(var);
                self.set_reg(*dst, x);
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
                let (c, _) = self.reg(*cond);
                // NA: the VM signals the error.
                let na = self.is_na_i(c);
                self.deopt_if(na, pc);
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
                let (code, formal_slots, arg_leaves) = (ic.code.clone(), ic.formal_slots.clone(), ic.args.clone());
                let args: Vec<(CValue, JType)> = arg_leaves.iter().map(|a| self.leaf(a)).collect();
                let ct = self.inl.callee(*site, args.iter().map(|a| a.1).collect()).unwrap();
                let v = self.inline_body(pc, &code, &formal_slots, &ct, &args);
                self.set_reg(*dst, v);
            }
            Op::Return { src } => {
                let (rv, after) = self.ret.expect("Return outside an inlined callee");
                let (x, _) = self.reg(*src);
                self.b.def_var(rv, x);
                self.b.ins().jump(after, &[]);
                falls = false;
            }
            // Checked at entry, or no effect in native code.
            Op::Guard { .. } | Op::Visible { .. } | Op::LoopEnter { .. } | Op::LoopExit => {}
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

#[allow(clippy::too_many_arguments)]
fn codegen(
    code: &Code,
    r: &Region,
    slots: &[SlotInfo],
    slot_ty: &HashMap<Slot, JType>,
    states: &[Option<RegState>],
    reg_idx: &HashMap<Reg, usize>,
    live: &[(Reg, JType)],
    deopt_regs: &mut HashMap<u32, Vec<(Reg, JType)>>,
    inl: &Inl,
) -> Result<LoopFn, Reject> {
    MODULE.with(|m| {
        let mut m = m.borrow_mut();
        let module = m.get_or_insert_with(new_module);
        let ptr = module.target_config().pointer_type();
        let tcfg = module.target_config();
        let mut ctx = module.make_context();
        ctx.func.signature.params.push(AbiParam::new(ptr));
        ctx.func.signature.params.push(AbiParam::new(ptr));
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
            let mf = MemFlagsData::trusted();
            let mut v = Vars {
                slot: HashMap::new(),
                vec: HashMap::new(),
                dims: HashMap::new(),
                flag: HashMap::new(),
                reg: HashMap::new(),
                range: HashMap::new(),
            };
            for s in slots {
                if s.ty.is_scalar() {
                    let ty = cl_type(s.ty);
                    let var = b.declare_var(ty);
                    let x = b.ins().load(ty, mf, vals, (s.idx * 8) as i32);
                    b.def_var(var, x);
                    v.slot.insert(s.slot, (var, s.ty));
                    if let Some(fi) = s.flag {
                        let f = b.declare_var(types::I64);
                        let x = b.ins().load(types::I64, mf, vals, (fi * 8) as i32);
                        b.def_var(f, x);
                        v.flag.insert(s.slot, f);
                    }
                } else {
                    let pv = b.declare_var(ptr);
                    let lv = b.declare_var(types::I64);
                    let p = b.ins().load(ptr, mf, vecs, (s.idx * std::mem::size_of::<VecView>()) as i32);
                    let l = b.ins().load(types::I64, mf, vecs, (s.idx * std::mem::size_of::<VecView>() + 8) as i32);
                    b.def_var(pv, p);
                    b.def_var(lv, l);
                    v.vec.insert(s.slot, (pv, lv, s.ty));
                    if matches!(s.ty, JType::MatD | JType::MatI) {
                        let nr = b.declare_var(types::I64);
                        let nc = b.declare_var(types::I64);
                        let r =
                            b.ins().load(types::I64, mf, vecs, (s.idx * std::mem::size_of::<VecView>() + 16) as i32);
                        let c =
                            b.ins().load(types::I64, mf, vecs, (s.idx * std::mem::size_of::<VecView>() + 24) as i32);
                        b.def_var(nr, r);
                        b.def_var(nc, c);
                        v.dims.insert(s.slot, (nr, nc));
                    }
                }
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
                if r.start + k < r.end {
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
            };
            for pc in r.start..r.end {
                let blk = g.block(pc);
                g.b.switch_to_block(blk);
                if g.states[pc - r.start].is_none() {
                    // Unreachable in native code.
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
