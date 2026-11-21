//! V3: GNU R's byte code translated into the IR under the `Bc` profile
//! (docs/g3.md §6.1, §9.1; lead decision F9). A BCODESXP is GNU R's
//! compiler's (cmp.R's) code: its decisions (what it inlined, behind which
//! guard, what it folded) are in its opcodes, so the translation follows
//! them and never decides again.
//!
//! GNU R runs the code on a value stack (bcEval). The translation walks
//! the reachable opcodes in order, keeping that stack as registers
//! allocated in order, so code paths that join at a label meet with the
//! same registers. A call's frame (`GETFUN`, `GETBUILTIN`) holds its
//! arguments as the pushes made them. The IR it emits is the from-source
//! compiler's ([`Comp`]'s emitters) in its register layout: an operation's
//! value register is allocated before the code of its operands, below
//! them. The stack code does not say where an operand's code begins, so a
//! first pass follows the stack to learn it (an entry's *starts*) and the
//! second reserves the register there. Equal decisions give equal code,
//! which `ROBIGO_BC_CROSSCHECK` compares.
//!
//! Where the shapes differ:
//! - **`RETURN`** at the end of each branch of a tail `if` becomes a jump
//!   to the code's one `Ret`, the shape the from-source compiler emits.
//! - **A promise's code** (`MAKEPROM`'s BCODESXP) is the promise's, as
//!   GNU R's PRCODE: the call carries its arguments' codes (`Call`'s `f`)
//!   untranslated, each translated at its promise's first force with that
//!   frame's slot indices and kept on itself (`compile::bytecode_code`);
//!   what reads the promise's
//!   expression reads the code's first constant (`bytecodeExpr`). The
//!   from-source compiler's promises hold their expressions, its code
//!   cached on them.
//! - An opcode not translated yet raises "robigo: ... is not implemented
//!   yet" when it runs (AGENTS rule 6).

#![allow(unsafe_code)]

use super::{Comp, Tail};
use crate::bcassign as ba;
use crate::cenv::{Cenv, Need};
use crate::ir::*;
use crate::prims::Kind;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::Cx;

/// GNU R 4.6.2's opcodes (eval.c's enum, C1-BC's table), by number.
#[allow(dead_code)]
pub(crate) mod opc {
    pub const RETURN: u8 = 1;
    pub const GOTO: u8 = 2;
    pub const BRIFNOT: u8 = 3;
    pub const POP: u8 = 4;
    pub const DUP: u8 = 5;
    pub const PRINTVALUE: u8 = 6;
    pub const STARTLOOPCNTXT: u8 = 7;
    pub const ENDLOOPCNTXT: u8 = 8;
    pub const DOLOOPNEXT: u8 = 9;
    pub const DOLOOPBREAK: u8 = 10;
    pub const STARTFOR: u8 = 11;
    pub const STEPFOR: u8 = 12;
    pub const ENDFOR: u8 = 13;
    pub const SETLOOPVAL: u8 = 14;
    pub const INVISIBLE: u8 = 15;
    pub const LDCONST: u8 = 16;
    pub const LDNULL: u8 = 17;
    pub const LDTRUE: u8 = 18;
    pub const LDFALSE: u8 = 19;
    pub const GETVAR: u8 = 20;
    pub const DDVAL: u8 = 21;
    pub const SETVAR: u8 = 22;
    pub const GETFUN: u8 = 23;
    pub const GETGLOBFUN: u8 = 24;
    pub const GETSYMFUN: u8 = 25;
    pub const GETBUILTIN: u8 = 26;
    pub const GETINTLBUILTIN: u8 = 27;
    pub const CHECKFUN: u8 = 28;
    pub const MAKEPROM: u8 = 29;
    pub const DOMISSING: u8 = 30;
    pub const SETTAG: u8 = 31;
    pub const DODOTS: u8 = 32;
    pub const PUSHARG: u8 = 33;
    pub const PUSHCONSTARG: u8 = 34;
    pub const PUSHNULLARG: u8 = 35;
    pub const PUSHTRUEARG: u8 = 36;
    pub const PUSHFALSEARG: u8 = 37;
    pub const CALL: u8 = 38;
    pub const CALLBUILTIN: u8 = 39;
    pub const CALLSPECIAL: u8 = 40;
    pub const MAKECLOSURE: u8 = 41;
    pub const UMINUS: u8 = 42;
    pub const UPLUS: u8 = 43;
    pub const ADD: u8 = 44;
    pub const SUB: u8 = 45;
    pub const MUL: u8 = 46;
    pub const DIV: u8 = 47;
    pub const EXPT: u8 = 48;
    pub const SQRT: u8 = 49;
    pub const EXP: u8 = 50;
    pub const EQ: u8 = 51;
    pub const NE: u8 = 52;
    pub const LT: u8 = 53;
    pub const LE: u8 = 54;
    pub const GE: u8 = 55;
    pub const GT: u8 = 56;
    pub const AND: u8 = 57;
    pub const OR: u8 = 58;
    pub const NOT: u8 = 59;
    pub const DOTSERR: u8 = 60;
    pub const STARTASSIGN: u8 = 61;
    pub const ENDASSIGN: u8 = 62;
    pub const STARTSUBSET: u8 = 63;
    pub const DFLTSUBSET: u8 = 64;
    pub const STARTSUBASSIGN: u8 = 65;
    pub const DFLTSUBASSIGN: u8 = 66;
    pub const STARTC: u8 = 67;
    pub const DFLTC: u8 = 68;
    pub const STARTSUBSET2: u8 = 69;
    pub const DFLTSUBSET2: u8 = 70;
    pub const STARTSUBASSIGN2: u8 = 71;
    pub const DFLTSUBASSIGN2: u8 = 72;
    pub const DOLLAR: u8 = 73;
    pub const DOLLARGETS: u8 = 74;
    pub const ISNULL: u8 = 75;
    pub const ISLOGICAL: u8 = 76;
    pub const ISINTEGER: u8 = 77;
    pub const ISDOUBLE: u8 = 78;
    pub const ISCOMPLEX: u8 = 79;
    pub const ISCHARACTER: u8 = 80;
    pub const ISSYMBOL: u8 = 81;
    pub const ISOBJECT: u8 = 82;
    pub const ISNUMERIC: u8 = 83;
    pub const VECSUBSET: u8 = 84;
    pub const MATSUBSET: u8 = 85;
    pub const VECSUBASSIGN: u8 = 86;
    pub const MATSUBASSIGN: u8 = 87;
    pub const AND1ST: u8 = 88;
    pub const AND2ND: u8 = 89;
    pub const OR1ST: u8 = 90;
    pub const OR2ND: u8 = 91;
    pub const GETVAR_MISSOK: u8 = 92;
    pub const DDVAL_MISSOK: u8 = 93;
    pub const VISIBLE: u8 = 94;
    pub const SETVAR2: u8 = 95;
    pub const STARTASSIGN2: u8 = 96;
    pub const ENDASSIGN2: u8 = 97;
    pub const SETTER_CALL: u8 = 98;
    pub const GETTER_CALL: u8 = 99;
    pub const SWAP: u8 = 100;
    pub const DUP2ND: u8 = 101;
    pub const SWITCH: u8 = 102;
    pub const RETURNJMP: u8 = 103;
    pub const STARTSUBSET_N: u8 = 104;
    pub const STARTSUBASSIGN_N: u8 = 105;
    pub const VECSUBSET2: u8 = 106;
    pub const MATSUBSET2: u8 = 107;
    pub const VECSUBASSIGN2: u8 = 108;
    pub const MATSUBASSIGN2: u8 = 109;
    pub const STARTSUBSET2_N: u8 = 110;
    pub const STARTSUBASSIGN2_N: u8 = 111;
    pub const SUBSET_N: u8 = 112;
    pub const SUBSET2_N: u8 = 113;
    pub const SUBASSIGN_N: u8 = 114;
    pub const SUBASSIGN2_N: u8 = 115;
    pub const LOG: u8 = 116;
    pub const LOGBASE: u8 = 117;
    pub const MATH1: u8 = 118;
    pub const DOTCALL: u8 = 119;
    pub const COLON: u8 = 120;
    pub const SEQALONG: u8 = 121;
    pub const SEQLEN: u8 = 122;
    pub const BASEGUARD: u8 = 123;
    pub const INCLNK: u8 = 124;
    pub const DECLNK: u8 = 125;
    pub const DECLNK_N: u8 = 126;
    pub const INCLNKSTK: u8 = 127;
    pub const DECLNKSTK: u8 = 128;
}

use opc::*;

/// One decoded instruction: its position in the code and its operands.
#[derive(Clone, Copy, Debug)]
struct BcIns {
    at: usize,
    op: u8,
    a: [i32; 4],
}

/// Code position → instruction index of a code's instructions (`dec`, whose
/// positions increase), dense from the first instruction's position. A
/// position where no instruction starts, or outside them, is None: up to one
/// past the last start (a target at the code's end) the table says so, and
/// beyond it the lookup does.
#[derive(Clone, Default)]
struct PosIndex {
    base: usize,
    ix: Vec<u32>,
}

impl PosIndex {
    fn of(dec: &[BcIns]) -> PosIndex {
        let base = dec.first().map_or(0, |i| i.at);
        let top = dec.last().map_or(base, |i| i.at);
        let mut ix = vec![u32::MAX; top - base + 2];
        for (k, i) in dec.iter().enumerate() {
            debug_assert!(k == 0 || dec[k - 1].at < i.at, "robigo: byte-code positions increase");
            ix[i.at - base] = u32::try_from(k).expect("robigo: a code of fewer than 2^32 instructions");
        }
        PosIndex { base, ix }
    }

    /// The instruction starting at position `p`.
    fn get(&self, p: usize) -> Option<usize> {
        let k = *self.ix.get(p.checked_sub(self.base)?)?;
        (k != u32::MAX).then_some(k as usize)
    }
}

/// Whether control at instruction `j` of `code` reaches a `RETURN` or
/// `RETURNJMP` at once or through `GOTO`s and assignments (`SETVAR`,
/// `SETVAR2`, a complex assignment from `STARTASSIGN` to its `ENDASSIGN`)
/// only.
fn returns_after(code: &[BcIns], mut j: usize) -> bool {
    for _ in 0..code.len() {
        match code.get(j) {
            Some(n) if matches!(n.op, RETURN | RETURNJMP) => return true,
            // (an assignment of `return()`'s operand: bcEval's SETVAR keeps
            // the value on the stack and R_Visible as its load set it)
            Some(n) if matches!(n.op, SETVAR | SETVAR2) => j += 1,
            // (a complex assignment of it: past its ENDASSIGN, which keeps
            // the value; a step that sets R_Visible after the value's load
            // sets it at run time)
            Some(n) if matches!(n.op, STARTASSIGN | STARTASSIGN2) => {
                let mut depth = 0;
                loop {
                    match code.get(j).map(|c| c.op) {
                        Some(STARTASSIGN | STARTASSIGN2) => depth += 1,
                        Some(ENDASSIGN | ENDASSIGN2) => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        Some(_) => {}
                        None => return false,
                    }
                    j += 1;
                }
                j += 1;
            }
            Some(n) if n.op == GOTO => match code.binary_search_by_key(&(n.a[0] as usize), |c| c.at) {
                Ok(p) => j = p,
                Err(_) => return false,
            },
            _ => return false,
        }
    }
    false
}

/// Decode the portable code `code` (the version word first) into
/// instructions, or the position of what is not one.
fn decode(code: &[i32]) -> Result<Vec<BcIns>, usize> {
    let mut out = Vec::new();
    let mut at = 1;
    while at < code.len() {
        let op = code[at];
        let Some(n) = robigo_rtbuiltins::bytecode::argc(op) else { return Err(at) };
        if at + n >= code.len() {
            return Err(at);
        }
        let mut a = [0; 4];
        a[..n].copy_from_slice(&code[at + 1..at + 1 + n]);
        out.push(BcIns { at, op: op as u8, a });
        at += 1 + n;
    }
    Ok(out)
}

/// Where a stack entry's computation begins: the instructions (one per
/// path to it) whose code first pushes toward it. Pass 2 reserves a
/// consumer's result register at its first operand's starts, below the
/// operands, where the from-source compiler allocates `dst` before it
/// compiles them: the IR's operations take their value's register outside
/// their operands' window ([`crate::code::Verified`] checks it).
type Starts = Vec<usize>;

/// Add `from`'s starts to `into`.
fn union(into: &mut Starts, from: &[usize]) {
    for &s in from {
        if !into.contains(&s) {
            into.push(s);
        }
    }
}

/// The two passes, or two paths to one label, disagree about the stack: a
/// translator bug (a debug build stops; a release build refuses the code).
fn mismatch() -> Need {
    debug_assert!(false, "robigo: the byte-code translation lost track of GNU R's stack");
    Need::Fail
}

/// A call's frame on GNU R's stack: what the function is, and its
/// arguments as pushed.
#[derive(Clone)]
struct Frame {
    fun: Fun,
    /// The register the call's value goes to, allocated first (the
    /// from-source compiler's `dst`).
    dst: Reg,
    /// The register a builtin's first argument goes to (the window), or
    /// the register the function is in (`GETFUN`).
    base: Reg,
    args: Vec<PArg>,
    /// `CHECKFUN`'s function's code (instruction indices from, to).
    fun_code: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
enum Fun {
    /// `GETFUN`'s function, in `base`, the `GetFun` instruction's index
    /// (its call is the `CALL`'s, written then) and the symbol; or
    /// `CHECKFUN`'s, a value computed into `base`.
    Found { get: Option<usize>, sym: Option<Sexp> },
    /// `GETBUILTIN`'s primitive, or `GETINTLBUILTIN`'s `.Internal`.
    Builtin { prim: Sexp, internal: bool },
    /// `STARTSUBSET`'s (`STARTSUBSET2`'s) arguments for `DFLTSUBSET`: the
    /// object first (in `base`), then the pushes, a window; its call.
    Dispatch { sub2: bool, call: Sexp },
    /// A complex assignment's `STARTSUBSET` (`[`'s getter) or
    /// `STARTSUBASSIGN` (`[<-`'s setter) with arguments for the default
    /// method: no registers (pass 1 notes their code).
    AssignDispatch { sub2: bool, call: Sexp },
}

impl Fun {
    /// Whether the pushes are values in a window from `base` (a builtin's
    /// arguments, or the default method's).
    fn window(&self) -> bool {
        matches!(self, Fun::Builtin { .. } | Fun::Dispatch { .. })
    }
}

/// An `_N` subset's `STARTSUBSET_N` for its `VECSUBSET`, `MATSUBSET` or
/// `SUBSET_N`: the value's register, the object's, `[[`'s or `[`'s.
#[derive(Clone, Copy)]
struct Sub {
    dst: Reg,
    obj: Reg,
    sub2: bool,
    /// A complex assignment's getter or setter (on its stack's values).
    assign: bool,
}

/// A pushed argument.
#[derive(Clone, Copy)]
struct PArg {
    kind: PKind,
    tag: Option<Sexp>,
    /// A complex assignment's `PUSHARG`: its value's code (instruction
    /// indices from, to).
    code: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
enum PKind {
    /// `MAKEPROM`'s code (a BCODESXP, or an expression the compiler left).
    Prom(Sexp),
    /// A value in the builtin's window (`PUSHARG`, or a constant loaded).
    Val,
    /// `PUSHCONSTARG` and the like, in a closure call's frame (the call's
    /// own argument: the IR's call makes it from the call).
    Const,
    Missing,
    Dots,
}

/// An entry of GNU R's stack, and where its computation began.
#[derive(Clone)]
struct Item {
    what: What,
    starts: Starts,
}

#[derive(Clone)]
enum What {
    Val(Reg),
    Frame(Frame),
    /// `INCLNKSTK`'s entry (bcEval's saved protection offset), which
    /// `DECLNKSTK` drops from under the value: no register (the links it
    /// counts are `BcAssign`'s to commit, as in the from-source code).
    Mark,
    /// `STARTFOR`'s loop state: [`super::FOR_STATE`] registers from `state` (the
    /// sequence first), the loop variable and the `for` call.
    For {
        state: Reg,
        sym: Sexp,
        call: Sexp,
    },
    /// `STARTLOOPCNTXT`'s context: the `Loop` instruction's index.
    LoopCtx {
        at: usize,
    },
    /// A complex assignment's value on GNU R's stack (the target's cell,
    /// a left-hand value, the right-hand side's copy): pass 1's only.
    Slot,
}

/// A complex assignment as pass 1 found it, `STARTASSIGN` to `ENDASSIGN`
/// (instruction indices): pass 2 translates it into [`Op::BcAssign`] of a
/// plan ([`crate::bcassign`]) made from its steps' opcodes.
#[derive(Clone)]
struct Assign {
    end: usize,
    sym: Sexp,
    getters: Vec<Step>,
    setters: Vec<Step>,
}

/// A getter or setter of a complex assignment: what its opcodes say.
#[derive(Clone)]
struct Step {
    /// [`crate::bcassign`]'s kind (`K_GENERIC`, `K_SUB`, ...).
    kind: u8,
    /// Its last instruction: the expression bcEval is at there is what
    /// `error()` reports.
    at: usize,
    /// Its call constant.
    call: Sexp,
    /// The function (a symbol, or an expression `CHECKFUN` checks), or
    /// `$`'s member.
    what: Sexp,
    /// A setter's value expression.
    vexpr: Option<Sexp>,
    /// The code of each argument after the object, in order.
    args: Vec<StepArg>,
    /// `CHECKFUN`'s function's code.
    fun_code: Option<(usize, usize)>,
}

/// An argument's code in a step.
#[derive(Clone, Copy)]
enum StepArg {
    /// Inline code (instruction indices from, to), which leaves its value.
    Code(usize, usize),
    /// `MAKEPROM`'s code.
    Prom(Sexp),
    /// None (a constant, a missing argument).
    None,
}

/// A label's state: the stack, the register count, and where the code
/// toward each depth's slot begins (`Tr::open`).
type Lab = (Vec<Item>, Reg, Vec<usize>);

/// What one instruction's translation keeps, by instruction index (one
/// record per instruction, one past the last too: an `open` start can be
/// the code's end), in one allocation per pass.
#[derive(Default)]
struct At {
    /// The state at a label a jump reached.
    label: Option<Lab>,
    /// The IR position of the instruction's translation (its first IR
    /// instruction).
    ir: Option<usize>,
    /// Pass 1: the consumer's first operand's starts.
    cons: Option<Starts>,
    /// Pass 2: the consumer's reserved value register.
    res: Option<Reg>,
    /// Pass 1: the complex assignment this `STARTASSIGN` begins.
    assigned: Option<Assign>,
    /// The plan: the consumers whose value registers this instruction
    /// reserves, outermost (lowest register) first ([`reserve_at`]).
    res_at: Vec<usize>,
}

/// What pass 1 learned for pass 2.
struct Plan {
    /// Pass 1's records, their reservations made ([`reserve_at`]) and
    /// their complex assignments.
    at: Vec<At>,
    /// The stack's depth at each instruction: pass 2 meets the same.
    depth: Vec<Option<usize>>,
    /// The code's positions' instruction indices, for pass 2.
    ix: PosIndex,
}

/// The translation state of one code object, in one pass.
struct Tr {
    code: Vec<BcIns>,
    /// The code's constants (a VECSXP).
    consts: Sexp,
    /// The code's positions' instruction indices.
    ix: PosIndex,
    /// Each instruction's record.
    at: Vec<At>,
    /// Jumps to patch: the IR instruction and the code position.
    patches: Vec<(usize, usize)>,
    /// `RETURN`s that jump to the code's end.
    returns: Vec<usize>,
    /// Base guards waiting for their label: (label, the guard's IR index,
    /// the call, the guard's starts).
    guards: Vec<(usize, usize, Sexp, Starts)>,
    stack: Vec<Item>,
    /// The instruction being translated (its index).
    cur: usize,
    /// A base guard's starts at a depth: the first entry its inline code
    /// pushes there begins at the guard, as the fallback's value does.
    pending: Option<(usize, Starts)>,
    /// Pass 2's plan (`None` in pass 1).
    plan: Option<Plan>,
    /// Pass 1: the stack's depth at each instruction.
    depth: Vec<Option<usize>>,
    /// The `_N` subsets begun (`STARTSUBSET_N`), innermost last.
    subsets: Vec<Sub>,
    /// Where the code toward the slot at each depth begins (an
    /// instruction index): after the instruction that last wrote the slot
    /// below. A stack entry's code begins at `open[its depth]`.
    open: Vec<usize>,
    /// Pass 1: the complex assignments begun, innermost last (by their
    /// `STARTASSIGN`), and those ended.
    assigning: Vec<(usize, Assign)>,
    /// Pass 2: the last instruction of a complex assignment translated
    /// whole.
    skip_to: Option<usize>,
}

impl Tr {
    /// The expression bcEval is at, at code position `at`: the constant
    /// the code's `expressionsIndex` names there (what
    /// `R_getBCInterpreterExpression` reports, and an opcode without a
    /// call constant is compiled from).
    ///
    /// # Safety
    /// The code's constants are live.
    unsafe fn expr_at(&self, cx: &Cx, at: usize) -> Option<Sexp> {
        // SAFETY: the caller's contract: a VECSXP of live constants.
        unsafe {
            let n = self.consts.std_len() as usize;
            let p = self.consts.std_data() as *const Sexp;
            let index = (0..n).rev().map(|j| *p.add(j)).find(|&x| {
                x.typ() == INTSXP
                    && robigo_rtbuiltins::attrib::inherits(robigo_rtcore::Ref::from_sexp(x, cx), b"expressionsIndex")
            })?;
            if at >= index.std_len() as usize {
                return None;
            }
            let k = *(index.std_data() as *const i32).add(at);
            (0..n as i32).contains(&k).then(|| *p.add(k as usize))
        }
    }

    /// Pass 1's findings: each consumer's register reserved at each of
    /// its first operand's starts.
    fn plan(self) -> Plan {
        let mut at = self.at;
        reserve_at(&mut at);
        Plan { at, depth: self.depth, ix: self.ix }
    }
}

/// Pass 1's consumers (each record's `cons`, by consumer index) by the
/// instructions their first operands begin at: at each instruction, the
/// consumers whose value registers it reserves, outermost (the highest
/// index) first, into its `res_at`.
fn reserve_at(at: &mut [At]) {
    // (in index order: each vector is sorted below, its consumers distinct,
    // so the order of the pushes does not matter)
    for c in 0..at.len() {
        for s in at[c].cons.take().into_iter().flatten() {
            at[s].res_at.push(c);
        }
    }
    // of two consumers whose operands begin at one instruction, the later
    // one's encloses the other's: its register is the lower
    for a in at.iter_mut() {
        a.res_at.sort_unstable_by(|a, b| b.cmp(a));
    }
}

/// Meet the stack `from` (a path's, at a label) with `into` (the label's):
/// the same entries in the same registers, their starts joined.
fn merge(into: &mut [Item], from: &[Item]) -> Result<(), Need> {
    if into.len() != from.len() {
        return Err(mismatch());
    }
    for (a, b) in into.iter_mut().zip(from) {
        let same = match (&a.what, &b.what) {
            (What::Val(x), What::Val(y)) => x == y,
            (What::Frame(f), What::Frame(g)) => f.dst == g.dst && f.base == g.base && f.args.len() == g.args.len(),
            (What::Mark, What::Mark) => true,
            (What::For { state: a, .. }, What::For { state: b, .. }) => a == b,
            (What::LoopCtx { at: a }, What::LoopCtx { at: b }) => a == b,
            (What::Slot, What::Slot) => true,
            _ => false,
        };
        if !same {
            return Err(mismatch());
        }
        union(&mut a.starts, &b.starts);
    }
    Ok(())
}

/// A path reaching the label `to` with the stack `st` and `next`
/// registers: the label's state, or met with it.
fn meet(t: &mut Tr, to: usize, st: Vec<Item>, next: Reg) -> Result<(), Need> {
    // (a jump's target is one of its instruction's successors, which
    // `reachable` found to be an instruction's start)
    let k = t.ix.get(to).ok_or(Need::Fail)?;
    if let Some((have, n, open)) = t.at[k].label.as_mut() {
        if *n != next {
            return Err(mismatch());
        }
        join_open(open, &t.open);
        merge(have, &st)
    } else {
        t.at[k].label = Some((st, next, t.open.clone()));
        Ok(())
    }
}

/// Two paths' `open` met: the code toward a slot begins after both paths'
/// last writes of the slot below (the join, in cmp.R's structured code).
fn join_open(into: &mut Vec<usize>, from: &[usize]) {
    if into.len() < from.len() {
        into.resize(from.len(), 0);
    }
    for (a, &b) in into.iter_mut().zip(from) {
        *a = (*a).max(b);
    }
}

/// Note a getter or setter of the innermost complex assignment (its
/// getters come first).
fn record(t: &mut Tr, setter: bool, step: Step) -> Result<(), Need> {
    let (_, a) = t.assigning.last_mut().ok_or(Need::Fail)?;
    if setter {
        a.setters.push(step);
    } else {
        if !a.setters.is_empty() {
            return Err(Need::Fail);
        }
        a.getters.push(step);
    }
    Ok(())
}

/// A frame's pushes as a step's arguments' code.
fn step_args(args: &[PArg]) -> Vec<StepArg> {
    args.iter()
        .map(|a| match (a.kind, a.code) {
            (_, Some((from, to))) => StepArg::Code(from, to),
            (PKind::Prom(c), None) => StepArg::Prom(c),
            _ => StepArg::None,
        })
        .collect()
}

/// A call's last argument (a setter call's `value = vexpr`).
///
/// # Safety
/// `call` is a live call.
unsafe fn last_arg(call: Sexp) -> Sexp {
    // SAFETY: the caller's contract.
    unsafe {
        let mut c = call;
        while c.cdr().typ() == LISTSXP {
            c = c.cdr();
        }
        c.car()
    }
}

/// The starts of an entry pushed now: the instruction's own, or a base
/// guard's just before it at this depth.
fn starts_new(t: &mut Tr) -> Starts {
    match t.pending.take() {
        Some((d, s)) if d == t.stack.len() => s,
        p => {
            t.pending = p;
            vec![t.cur]
        }
    }
}

/// Whether control can fall from `op` into the next instruction.
fn falls_through(op: u8) -> bool {
    !matches!(op, RETURN | GOTO | RETURNJMP | SWITCH | STARTFOR)
}

/// The code positions an INTSXP of offsets (`SWITCH`'s) names; R_NilValue
/// names none.
///
/// # Safety
/// `x` is live.
unsafe fn offsets(x: Sexp) -> Result<Vec<usize>, Need> {
    // SAFETY: the caller's contract.
    unsafe {
        match x.typ() {
            NILSXP => Ok(Vec::new()),
            INTSXP => {
                let v = std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize);
                v.iter().map(|&o| usize::try_from(o).map_err(|_| Need::Fail)).collect()
            }
            _ => Err(Need::Fail),
        }
    }
}

/// The code positions each instruction may go to.
///
/// # Safety
/// `consts` are the code's live constants.
unsafe fn successors(i: &BcIns, next: Option<usize>, consts: Sexp) -> Result<Vec<usize>, Need> {
    let mut s = Vec::new();
    // SAFETY: the caller's contract: a VECSXP's elements.
    let k = |x: i32| unsafe { *(consts.std_data() as *const Sexp).add(x as usize) };
    match i.op {
        // SAFETY: as above.
        SWITCH => unsafe {
            s.extend(offsets(k(i.a[2]))?);
            s.extend(offsets(k(i.a[3]))?);
        },
        STARTFOR => {
            s.push(i.a[2] as usize);
            // (the body, which STEPFOR jumps back to)
            s.extend(next);
        }
        STARTLOOPCNTXT | STEPFOR => s.push(i.a[if i.op == STEPFOR { 0 } else { 1 }] as usize),
        GOTO => s.push(i.a[0] as usize),
        BRIFNOT | BASEGUARD | AND1ST | OR1ST | STARTSUBSET | STARTSUBSET2 | STARTSUBSET_N | STARTSUBSET2_N
        | STARTSUBASSIGN | STARTSUBASSIGN2 | STARTSUBASSIGN_N | STARTSUBASSIGN2_N => s.push(i.a[1] as usize),
        _ => {}
    }
    if falls_through(i.op)
        && let Some(n) = next
    {
        s.push(n);
    }
    Ok(s)
}

/// The instructions control reaches from the first.
///
/// # Safety
/// `consts` are the code's live constants.
unsafe fn reachable(dec: &[BcIns], index: &PosIndex, consts: Sexp) -> Result<Vec<bool>, Need> {
    let mut live = vec![false; dec.len()];
    let mut todo = vec![0usize];
    while let Some(k) = todo.pop() {
        if k >= dec.len() || live[k] {
            continue;
        }
        live[k] = true;
        let next = dec.get(k + 1).map(|i| i.at);
        // SAFETY: the caller's contract.
        for p in unsafe { successors(&dec[k], next, consts)? } {
            match index.get(p) {
                Some(j) => todo.push(j),
                None => return Err(Need::Fail),
            }
        }
    }
    Ok(live)
}

impl<'g> Comp<'g> {
    /// Translate the byte code `bc` (a live BCODESXP) into this code, its
    /// value into `dst` (the code object's result). A malformed code
    /// vector is a compiler error (GNU R's `bcEval` would misread it).
    ///
    /// Two passes: the first, on a scratch compiler, follows the stack to
    /// learn where each operation's operands begin; the second emits the
    /// code, each operation's value register reserved there.
    ///
    /// # Safety
    /// `bc` is a live BCODESXP of the closure or expression being
    /// compiled.
    pub(super) unsafe fn translate(&mut self, bc: Sexp, dst: Reg) -> Result<(), Need> {
        // SAFETY: the caller's contract; a BCODESXP's code and constants.
        let (code, consts) = unsafe {
            let c = bc.car();
            if c.typ() != INTSXP {
                return Err(Need::Fail);
            }
            (std::slice::from_raw_parts(c.std_data() as *const i32, c.std_len() as usize).to_vec(), bc.cdr())
        };
        let Ok(dec) = decode(&code) else { return Err(Need::Fail) };
        // SAFETY: as above.
        unsafe { self.translate_ins(&dec, consts, dst) }
    }

    /// Translate the instructions `dec` (a code's, or an argument's slice
    /// of one ending in a RETURN) with the constants `consts`, the value
    /// into `dst`: the two passes.
    ///
    /// # Safety
    /// `consts` are the live byte code's constants.
    unsafe fn translate_ins(&mut self, dec: &[BcIns], consts: Sexp, dst: Reg) -> Result<(), Need> {
        let ix = PosIndex::of(dec);
        // SAFETY: the caller's contract.
        let live = unsafe { reachable(dec, &ix, consts)? };
        debug_assert_eq!(dst, R_FIRST);
        let mut p = Comp::new(self.cx, self.st, Profile::Bc, Some(Cenv::for_translation()));
        // (the constants pool sized for the byte code's constants at once,
        // so it does not rehash while each pass fills it)
        // SAFETY: the caller's contract: a VECSXP.
        let nk = unsafe { consts.std_len() } as usize;
        p.pool.reserve(nk);
        self.pool.reserve(nk);
        let r = p.alloc();
        // SAFETY: the caller's contract.
        let plan = match unsafe { p.pass(dec, &live, consts, ix, None, r) } {
            Ok(t) => t.plan(),
            Err(e) => {
                self.bc_untranslated = p.bc_untranslated.take();
                return Err(e);
            }
        };
        // SAFETY: as above.
        unsafe { self.pass(dec, &live, consts, PosIndex::default(), Some(plan), dst) }.map(drop)
    }

    /// The code of instructions `from..to` of `t`'s code (an argument's,
    /// which leaves its value), a code object for `arg`, kept on it as a
    /// call's argument code is ([`crate::code::cache_on`]: what
    /// [`crate::bcassign`]'s evaluation of the argument runs).
    ///
    /// # Safety
    /// `t`'s constants are live; `arg` is a live call.
    unsafe fn slice_code(&mut self, t: &Tr, from: usize, to: usize, arg: Sexp) -> Result<(), Need> {
        if from >= to || to >= t.code.len() {
            return Err(mismatch());
        }
        let mut dec = t.code[from..to].to_vec();
        // the value left: the code's, at the position after it
        dec.push(BcIns { at: t.code[to].at, op: RETURN, a: [0; 4] });
        let cenv = self.bc.take().expect("robigo: byte code outside the Bc profile");
        let mut sub = Comp::new(self.cx, self.st, Profile::Bc, Some(cenv));
        let r = sub.alloc();
        // SAFETY: the caller's contract.
        let res = unsafe { sub.translate_ins(&dec, t.consts, r) };
        let res = res.map(|()| {
            let mut i = Ins::new(Op::Ret);
            i.b = r;
            sub.emit(i);
            sub.finish()
        });
        self.bc = sub.bc.take();
        if res.is_err() && sub.bc_untranslated.is_some() {
            self.bc_untranslated = sub.bc_untranslated.take();
        }
        let b = res?;
        // SAFETY: as above.
        unsafe {
            let built = crate::code::build(self.cx, &super::verified(b), arg, None)?;
            crate::code::cache_on(arg, built);
            self.arg_codes.push(built);
            self.arg_codes.append(&mut sub.arg_codes);
        }
        Ok(())
    }

    /// One pass over the reachable instructions, in order: pass 1 (no
    /// `plan`) learns where operands begin, pass 2 emits the code.
    ///
    /// # Safety
    /// `consts` are the live byte code's constants.
    unsafe fn pass(
        &mut self,
        dec: &[BcIns],
        live: &[bool],
        consts: Sexp,
        ix: PosIndex,
        mut plan: Option<Plan>,
        dst: Reg,
    ) -> Result<Tr, Need> {
        // (pass 2's index is pass 1's, carried by the plan)
        let ix = match plan.as_mut() {
            Some(p) => std::mem::take(&mut p.ix),
            None => ix,
        };
        // (by instruction index; one past the last for what an `open`
        // start, the code's end, can key)
        let n = dec.len();
        let mut t = Tr {
            code: dec.to_vec(),
            consts,
            ix,
            at: (0..=n).map(|_| At::default()).collect(),
            patches: Vec::new(),
            returns: Vec::new(),
            guards: Vec::new(),
            stack: Vec::new(),
            cur: 0,
            pending: None,
            plan,
            depth: vec![None; dec.len()],
            subsets: Vec::new(),
            open: vec![0],
            assigning: Vec::new(),
            skip_to: None,
        };
        // `dst` is the code's value (the first register)
        self.free_to(dst);
        let mut fell = true;
        for k in 0..t.code.len() {
            if !live[k] {
                fell = false;
                continue;
            }
            // (a complex assignment translated whole)
            if let Some(end) = t.skip_to {
                if k <= end {
                    continue;
                }
                t.skip_to = None;
            }
            let i = t.code[k];
            t.cur = k;
            // a label: the stack a jump left there, met by control falling in
            match (t.at[k].label.clone(), fell) {
                // a path falling in with values the label's state lacks: a
                // non-tail numeric switch's empty alternative, whose stop()
                // call cmp.R lets fall into the default. Should `stop`
                // return (a redefinition), bcEval goes on with that value
                // left on its stack, a later operation's operand: that path
                // raises "not implemented yet" instead (AGENTS rule 6);
                // the jumps to the label go on as the code says
                (Some((st, next, open)), true)
                    if st.len() < t.stack.len()
                        && next <= self.next
                        && t.stack[st.len()..].iter().all(|e| matches!(e.what, What::Val(_))) =>
                {
                    let nil = self.nil();
                    self.unimplemented(
                        nil,
                        "a stack left unbalanced where the byte code joins (a 'switch' whose stop() returned)",
                    )?;
                    t.stack = st;
                    self.next = next;
                    t.open = open;
                    t.pending = None;
                }
                (Some((st, next, open)), true) => {
                    if next != self.next {
                        return Err(mismatch());
                    }
                    merge(&mut t.stack, &st)?;
                    join_open(&mut t.open, &open);
                }
                (Some((st, next, open)), false) => {
                    t.stack = st;
                    self.next = next;
                    t.open = open;
                    t.pending = None;
                }
                (None, true) => {}
                // the target of a back-edge yet to come (a loop body after
                // the jump into its loop): the stack the jump before left
                // there, which that back-edge meets
                (None, false) => {
                    t.at[k].label = Some((t.stack.clone(), self.next, t.open.clone()));
                }
            }
            match &t.plan {
                None => t.depth[k] = Some(t.stack.len()),
                Some(p) if p.depth[k] != Some(t.stack.len()) => return Err(mismatch()),
                Some(_) => {}
            }
            // base guards whose label this is: their fallbacks
            self.guard_joins(&mut t, i.at)?;
            t.at[k].ir = Some(self.pc());
            // the value registers of the operations whose operands begin here
            if let Some(cs) = t.plan.as_ref().map(|p| &p.at[k].res_at).filter(|v| !v.is_empty()).cloned() {
                for c in cs {
                    let r = self.alloc();
                    if *t.at[c].res.get_or_insert(r) != r {
                        return Err(mismatch());
                    }
                }
            }
            // (a value `RETURN` or `RETURNJMP` returns, next or through
            // GOTOs: `return(x)`'s operand is in tail position, its
            // visibility the function's, though cmp.R compiles it outside
            // the tail (make.nonTailCallContext), so a branch of
            // `return(if ...)` jumps to the RETURN; bcEval's loads set
            // R_Visible either way)
            let tail = returns_after(&t.code, k + 1);
            // SAFETY: constants of the live byte code.
            unsafe { self.op(&mut t, i, tail)? };
            fell = falls_through(i.op);
            // the code toward the next slot begins after an instruction that
            // wrote the top one (not after a pop or a jump)
            if !matches!(i.op, POP | BRIFNOT | SWITCH | RETURN | RETURNJMP | ENDLOOPCNTXT | GOTO | BASEGUARD) {
                let d = t.stack.len();
                if t.open.len() <= d {
                    t.open.resize(d + 1, 0);
                }
                t.open[d] = k + 1;
            }
        }
        if !t.guards.is_empty() {
            return Err(mismatch());
        }
        // the end: every RETURN jumps here
        let end = self.pc();
        for &j in &t.returns {
            self.patch(j, end);
        }
        for &(at, pos) in &t.patches {
            let target = t.ix.get(pos).and_then(|k| t.at[k].ir).ok_or(Need::Fail)?;
            self.patch(at, target);
        }
        Ok(t)
    }

    /// The fallback code of the base guards whose label is `at`, before
    /// it: the inline code's value is the stack's top, and a failed guard
    /// evaluates the call as `eval` does into the same register (bcEval's
    /// `DO_BASEGUARD`).
    fn guard_joins(&mut self, t: &mut Tr, at: usize) -> Result<(), Need> {
        let mut k = 0;
        while k < t.guards.len() {
            if t.guards[k].0 != at {
                k += 1;
                continue;
            }
            let (_, g, call, starts) = t.guards.remove(k);
            let Some(Item { what: What::Val(r), starts: s }) = t.stack.last_mut() else { return Err(mismatch()) };
            union(s, &starts);
            let r = *r;
            let j = self.emit(Ins::new(Op::Jmp));
            let fallback = self.pc();
            self.patch(g, fallback);
            let mut ev = Ins::new(Op::EvalConst);
            ev.a = r;
            ev.e = self.k(call);
            self.emit(ev);
            let end = self.pc();
            self.patch(j, end);
        }
        Ok(())
    }

    /// Push a new value register.
    fn push(&mut self, t: &mut Tr) -> Reg {
        let starts = starts_new(t);
        let r = self.alloc();
        t.stack.push(Item { what: What::Val(r), starts });
        r
    }

    /// Pop a value register (it and those above are free).
    fn pop(&mut self, t: &mut Tr) -> Result<Reg, Need> {
        match t.stack.pop() {
            Some(Item { what: What::Val(r), .. }) => {
                self.free_to(r);
                Ok(r)
            }
            _ => Err(Need::Fail),
        }
    }

    /// The top value register.
    fn top(&self, t: &Tr) -> Result<Reg, Need> {
        match t.stack.last() {
            Some(Item { what: What::Val(r), .. }) => Ok(*r),
            _ => Err(Need::Fail),
        }
    }

    /// The innermost call frame.
    fn frame<'t>(&self, t: &'t mut Tr) -> Result<&'t mut Frame, Need> {
        t.stack
            .iter_mut()
            .rev()
            .find_map(|i| match &mut i.what {
                What::Frame(f) => Some(f),
                _ => None,
            })
            .ok_or(Need::Fail)
    }

    /// Whether the entry `j` below the top is a complex assignment's value.
    fn top_slot(&self, t: &Tr, j: usize) -> bool {
        t.stack.len() > j && matches!(t.stack[t.stack.len() - 1 - j].what, What::Slot)
    }

    /// Push a complex assignment's value (no register).
    fn push_slot(&mut self, t: &mut Tr) {
        let starts = starts_new(t);
        t.stack.push(Item { what: What::Slot, starts });
    }

    /// A complex assignment's `STARTSUBSET` or `STARTSUBASSIGN` frame.
    fn push_dispatch(&mut self, t: &mut Tr, sub2: bool, call: Sexp) {
        let starts = starts_new(t);
        let fun = Fun::AssignDispatch { sub2, call };
        let f = Frame { fun, dst: self.next, base: self.next, args: Vec::new(), fun_code: None };
        t.stack.push(Item { what: What::Frame(f), starts });
    }

    /// The code of the `n` values on top of the stack (indices), in order:
    /// each from where its code began to where the next one's did (the
    /// last's to the instruction being translated).
    fn index_codes(&self, t: &Tr, n: usize) -> Result<Vec<StepArg>, Need> {
        let len = t.stack.len();
        if len < n {
            return Err(Need::Fail);
        }
        let at = |d: usize| t.open.get(d).copied().ok_or(Need::Fail);
        (0..n)
            .map(|j| {
                let from = at(len - n + j)?;
                let to = if j + 1 < n { at(len - n + j + 1)? } else { t.cur };
                Ok(StepArg::Code(from, to))
            })
            .collect()
    }

    /// The plan of the complex assignment `a` (what
    /// [`Comp::bc_assign_plan`] makes from source): its steps as their
    /// opcodes say, each argument's code kept untranslated in the plan
    /// (`S_CODES`), translated at its promise's first force
    /// (`compile::bytecode_code`).
    ///
    /// # Safety
    /// `t`'s constants are live.
    // R: cmp.R:cmpComplexAssign, cmpGetterCall, cmpSetterCall
    unsafe fn assign_plan(&mut self, t: &Tr, a: &Assign) -> Result<Sexp, Need> {
        let cx = self.cx;
        // SAFETY: the caller's contract; every object made here is reached
        // from the plan, which the code's pool keeps, and nothing collects.
        unsafe {
            let mut g = Vec::with_capacity(a.getters.len());
            for s in &a.getters {
                g.push(self.step(t, s, false)?);
            }
            let mut st = Vec::with_capacity(a.setters.len());
            for s in &a.setters {
                st.push(self.step(t, s, true)?);
            }
            let getters = cx.alloc_vector_raw(VECSXP, g.len())?;
            for (j, &x) in g.iter().enumerate() {
                getters.set_elt(j, x);
            }
            let setters = cx.alloc_vector_raw(VECSXP, st.len())?;
            for (j, &x) in st.iter().enumerate() {
                setters.set_elt(j, x);
            }
            let plan = cx.alloc_vector_raw(VECSXP, ba::PLAN_LEN)?;
            plan.set_elt(ba::P_SYM, a.sym);
            plan.set_elt(ba::P_GETTERS, getters);
            plan.set_elt(ba::P_SETTERS, setters);
            Ok(plan)
        }
    }

    /// A plan's step ([`ba::make_step`]): the expression `error()` reports
    /// the one bcEval is at (a setter call's inflated into its assignment,
    /// `R_getBCInterpreterExpression`'s).
    ///
    /// # Safety
    /// As for [`Comp::assign_plan`].
    unsafe fn step(&mut self, t: &Tr, s: &Step, setter: bool) -> Result<Sexp, Need> {
        let cx = self.cx;
        // SAFETY: the caller's contract: live constants.
        unsafe {
            // the arguments after the object, in order (a setter's `value`
            // after them)
            let first = s.call.cdr();
            if first.typ() != LISTSXP {
                return Err(Need::Fail);
            }
            let mut cell = first.cdr();
            // (the promises' codes by argument: the step's, as `Call`'s)
            let nil = self.nil();
            let mut codes = Vec::new();
            for arg in &s.args {
                if cell.typ() != LISTSXP {
                    return Err(Need::Fail);
                }
                let e = cell.car();
                codes.push(nil);
                match *arg {
                    StepArg::Code(from, to) if e.typ() == LANGSXP => self.slice_code(t, from, to, e)?,
                    // (translated at its first force, as a call's)
                    StepArg::Prom(code) if code.typ() == BCODESXP => *codes.last_mut().unwrap() = code,
                    _ => {}
                }
                cell = cell.cdr();
            }
            let codes = self.codes_vector(&codes)?;
            if let Some((from, to)) = s.fun_code
                && s.what.typ() == LANGSXP
            {
                self.slice_code(t, from, to, s.what)?;
            }
            let Some(cexpr) = t.expr_at(cx, t.code[s.at].at) else {
                self.bc_untranslated = Some("byte code without its expressions index".to_owned());
                return Err(Need::Fail);
            };
            let fin = setter && s.call.car().typ() == SYMSXP;
            let report = if fin { ba::inflate_assignment_call(cx, cexpr)? } else { cexpr };
            ba::make_step(cx, s.kind, s.call, s.what, s.vexpr, report, fin, codes)
        }
    }

    /// The `n` values on top of the stack, popped: the first one's
    /// register (they are consecutive: a window) and starts.
    fn pop_window(&mut self, t: &mut Tr, n: usize) -> Result<(Reg, Starts), Need> {
        let mut x: Option<Reg> = None;
        let mut starts = Vec::new();
        for _ in 0..n {
            match t.stack.pop() {
                Some(Item { what: What::Val(r), starts: s }) => {
                    if x.is_some_and(|above| r + 1 != above) {
                        return Err(mismatch());
                    }
                    x = Some(r);
                    starts = s;
                }
                _ => return Err(Need::Fail),
            }
        }
        Ok((x.ok_or(Need::Fail)?, starts))
    }

    /// The value register of the instruction being translated, whose
    /// first operand (in `x`) begins at `starts`: pass 2's reservation,
    /// below the operands; pass 1 notes the starts and gives `x`
    /// (registers do not matter there).
    fn reserve(&mut self, t: &mut Tr, x: Reg, starts: &Starts) -> Result<Reg, Need> {
        match &t.plan {
            None => {
                t.at[t.cur].cons = Some(starts.clone());
                Ok(x)
            }
            Some(_) => match t.at[t.cur].res {
                Some(r) if r < x => Ok(r),
                _ => Err(mismatch()),
            },
        }
    }

    /// Push the value in `dst`, the top register.
    fn push_at(&mut self, t: &mut Tr, dst: Reg, starts: Starts) {
        self.next = dst + 1;
        t.stack.push(Item { what: What::Val(dst), starts });
    }

    /// The `n` operands on top of the stack, popped for the instruction
    /// being translated, whose value then is the top: the register it goes
    /// to ([`Comp::reserve`]) and the operands' window.
    fn consume(&mut self, t: &mut Tr, n: usize) -> Result<(Reg, Reg), Need> {
        let (x, starts) = self.pop_window(t, n)?;
        let dst = self.reserve(t, x, &starts)?;
        self.push_at(t, dst, starts);
        Ok((dst, x))
    }

    /// A jump to code position `to`, patched at the end; the stack there
    /// is this one.
    fn jump(&mut self, t: &mut Tr, ins: Ins, to: usize) -> Result<(), Need> {
        let at = self.emit(ins);
        t.patches.push((at, to));
        let st = t.stack.clone();
        meet(t, to, st, self.next)
    }

    /// The primitive named by symbol `sym` (a builtin or special of the
    /// table), its `PRIMOFFSET`.
    fn prim_of(&self, sym: Sexp) -> Option<usize> {
        self.st.by_sym.get(&(sym.as_ptr() as usize)).copied()
    }

    /// Translate one instruction.
    ///
    /// # Safety
    /// `t`'s constants are live.
    unsafe fn op(&mut self, t: &mut Tr, i: BcIns, tail: bool) -> Result<(), Need> {
        let consts = t.consts;
        // SAFETY: the caller's contract: a VECSXP's element.
        let k = |x: i32| unsafe { *(consts.std_data() as *const Sexp).add(x as usize) };
        let planning = t.plan.is_none();
        match i.op {
            RETURN => {
                let r = self.top(t)?;
                if r != R_FIRST {
                    let mut m = Ins::new(Op::Move);
                    m.a = R_FIRST;
                    m.b = r;
                    self.emit(m);
                }
                // the last instruction falls into the end's Ret
                if t.code.last().is_some_and(|l| l.at != i.at) {
                    let j = self.emit(Ins::new(Op::Jmp));
                    t.returns.push(j);
                }
                self.pop(t)?;
            }
            GOTO => {
                // a back-edge: a safepoint and bcEval's poll; a forward
                // jump plain, as the from-source code's (bcEval's GOTO
                // counts toward its poll either way)
                let to = i.a[0] as usize;
                let op = if to <= i.at { Op::BackJmp } else { Op::Jmp };
                self.jump(t, Ins::new(op), to)?
            }
            BRIFNOT => {
                let r = self.pop(t)?;
                let mut br = Ins::new(Op::BrFalse);
                br.b = r;
                br.fl |= F_CLR_B;
                br.e = self.k(k(i.a[0]));
                self.jump(t, br, i.a[1] as usize)?;
            }
            POP => {
                let r = self.pop(t)?;
                self.clear(r);
            }
            INVISIBLE => self.set_vis(false),
            VISIBLE => self.set_vis(true),
            LDCONST => {
                let r = self.push(t);
                // SAFETY: a constant of the byte code.
                unsafe { self.const_value(k(i.a[0]), r, 0, tail.into()) };
            }
            LDNULL => {
                let r = self.push(t);
                self.ld_nil(r, tail);
            }
            LDTRUE | LDFALSE => {
                let r = self.push(t);
                let v = self.cx.logical_value((i.op == LDTRUE) as i32).sexp();
                // SAFETY: an immortal constant.
                unsafe { self.const_value(v, r, 0, tail.into()) };
            }
            GETVAR | DDVAL | GETVAR_MISSOK | DDVAL_MISSOK => {
                let r = self.push(t);
                let mut g = Ins::new(match i.op {
                    DDVAL => Op::GetDdVar,
                    // (a `..n` too, as the from-source code's missing-OK
                    // read)
                    GETVAR_MISSOK | DDVAL_MISSOK => Op::GetVarMissOk,
                    _ => Op::GetVar,
                });
                g.a = r;
                g.e = self.k(k(i.a[0]));
                if tail {
                    g.fl |= F_VIS;
                }
                self.emit(g);
            }
            SETVAR | SETVAR2 => {
                let r = self.top(t)?;
                let sym = k(i.a[0]);
                let mut s = Ins::new(if i.op == SETVAR2 { Op::SetVar2 } else { Op::SetVar });
                s.a = r;
                s.e = self.k(sym);
                if i.op == SETVAR && !self.locals.contains(&sym) {
                    self.locals.push(sym);
                }
                self.emit(s);
            }
            GETFUN => {
                let starts = starts_new(t);
                let dst = self.alloc();
                let r = self.alloc();
                let mut g = Ins::new(Op::GetFun);
                g.a = r;
                g.e = self.k(k(i.a[0]));
                let get = self.emit(g);
                let fun = Fun::Found { get: Some(get), sym: Some(k(i.a[0])) };
                let f = Frame { fun, dst, base: r, args: Vec::new(), fun_code: None };
                t.stack.push(Item { what: What::Frame(f), starts });
            }
            GETBUILTIN | GETINTLBUILTIN => {
                let sym = k(i.a[0]);
                let prim = if i.op == GETINTLBUILTIN {
                    // SAFETY: a live symbol: its INTERNAL.
                    unsafe { robigo_rtcore::Ref::from_sexp(sym, self.cx) }.as_sym().map(|s| self.cx.internal(s).sexp())
                } else {
                    // getPrimitive: the symbol's value, which for a name base
                    // binds to a primitive of another name is that one's
                    // (`as.numeric`, base's `as.double`: robigo-rt's table
                    // has a row of the name too)
                    // SAFETY: a live symbol.
                    let alias = unsafe { crate::cenv::alias_of(self.cx, sym) }
                        .and_then(|t| self.st.defs.iter().position(|d| d.name == t && !d.internal()));
                    alias.or_else(|| self.prim_of(sym)).map(|off| self.st.objs[off].bind(self.cx).sexp())
                };
                // SAFETY: a live primitive.
                let Some(prim) = prim.filter(|p| unsafe { p.typ() } == BUILTINSXP) else {
                    return self.untranslated(i, "byte code calling a builtin robigo-rt lacks");
                };
                let starts = starts_new(t);
                let dst = self.alloc();
                let fun = Fun::Builtin { prim, internal: i.op == GETINTLBUILTIN };
                let f = Frame { fun, dst, base: self.next, args: Vec::new(), fun_code: None };
                t.stack.push(Item { what: What::Frame(f), starts });
            }
            MAKEPROM | PUSHCONSTARG | PUSHNULLARG | PUSHTRUEARG | PUSHFALSEARG | DOMISSING | DODOTS => {
                let fun = self.frame(t)?.fun;
                let missing = self.cx.missing_arg().sexp();
                let constant = match i.op {
                    PUSHCONSTARG => Some(k(i.a[0])),
                    PUSHNULLARG => Some(self.nil()),
                    PUSHTRUEARG | PUSHFALSEARG => Some(self.cx.logical_value((i.op == PUSHTRUEARG) as i32).sexp()),
                    // the default method's empty argument (cmpBuiltinArgs'
                    // missingOK): R_MissingArg
                    DOMISSING if matches!(fun, Fun::Dispatch { .. }) => Some(missing),
                    _ => None,
                };
                let kind = match (i.op, constant) {
                    (_, Some(v)) if fun.window() => {
                        // a window: the constant loaded into its register
                        let r = self.alloc();
                        let f = self.frame(t)?;
                        if r != f.base + f.args.len() as Reg {
                            return Err(mismatch());
                        }
                        if v == missing {
                            let mut l = Ins::new(Op::LdConst);
                            l.a = r;
                            l.e = self.k(v);
                            self.emit(l);
                        } else {
                            // SAFETY: a constant of the byte code.
                            unsafe { self.const_value(v, r, 0, Tail::No) };
                        }
                        PKind::Val
                    }
                    (MAKEPROM, _) => PKind::Prom(k(i.a[0])),
                    (DOMISSING, _) => PKind::Missing,
                    (DODOTS, _) => PKind::Dots,
                    _ => PKind::Const,
                };
                self.frame(t)?.args.push(PArg { kind, tag: None, code: None });
            }
            PUSHARG => {
                // the value stays in its register: the builtin's window (a
                // complex assignment's default method's: its code noted)
                let from = t.open.get(t.stack.len().wrapping_sub(1)).copied().unwrap_or(0);
                let Some(Item { what: What::Val(r), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let cur = t.cur;
                let f = self.frame(t)?;
                if f.fun.window() && r != f.base + f.args.len() as Reg {
                    return Err(mismatch());
                }
                let code = matches!(f.fun, Fun::AssignDispatch { .. }).then_some((from, cur));
                f.args.push(PArg { kind: PKind::Val, tag: None, code });
                if !f.fun.window() {
                    self.free_to(r);
                }
            }
            SETTAG => {
                let tag = k(i.a[0]);
                let f = self.frame(t)?;
                match f.args.last_mut() {
                    Some(a) => a.tag = Some(tag),
                    None => return Err(Need::Fail),
                }
            }
            CALL => {
                let call = k(i.a[0]);
                let Some(Item { what: What::Frame(f), starts }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::Found { get, .. } = f.fun else { return Err(Need::Fail) };
                // SAFETY: a constant call of the byte code.
                unsafe { self.call_found(&f, get, call, planning)? };
                self.push_at(t, f.dst, starts);
            }
            CALLBUILTIN => {
                let call = k(i.a[0]);
                let Some(Item { what: What::Frame(f), starts }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::Builtin { prim, internal } = f.fun else { return Err(Need::Fail) };
                // SAFETY: as above.
                unsafe { self.call_builtin(&f, prim, internal, call)? };
                // the expression bcEval is at, for error() and warning():
                // the expressions index's (a `.Internal` wrapper's call)
                // SAFETY: the code's live constants.
                if let Some(e) = unsafe { t.expr_at(self.cx, i.at) }
                    && e != call
                {
                    let ke = self.k(e);
                    if internal {
                        self.set_internal_site(ke);
                    } else {
                        let at = self.ins.len() - 1;
                        self.ins[at].g = ke;
                    }
                }
                self.push_at(t, f.dst, starts);
            }
            CALLSPECIAL => {
                let call = k(i.a[0]);
                let r = self.push(t);
                // SAFETY: as above: a call whose function is a symbol.
                let off = unsafe { self.prim_of(call.car()) };
                // (a special of the registry, `rep`, is a function over its
                // call too)
                // SAFETY: a live primitive.
                let special = |o: usize| unsafe { self.st.objs[o].bind(self.cx).sexp().typ() } == SPECIALSXP;
                match off.map(|o| (o, self.st.defs[o].kind)) {
                    // pass 1 wants the stack only (a special's code caches
                    // its arguments' code on them)
                    Some((o, _)) if planning && special(o) => {}
                    // a language special: what its PRIMFUN does, compiled
                    // SAFETY: as above.
                    Some((_, Kind::Special(s))) => unsafe { self.special(s, call, call.cdr(), r, 0, tail.into())? },
                    // getPrimitive(symbol, SPECIALSXP) on the call
                    Some((o, _)) if special(o) => {
                        let prim = self.st.objs[o].bind(self.cx).sexp();
                        let mut s = Ins::new(Op::SpecialFn);
                        s.a = r;
                        s.e = self.k(call);
                        s.c = self.k(prim);
                        s.fl |= F_DEPTH;
                        self.emit(s);
                    }
                    _ => return self.untranslated(i, "byte code calling a special robigo-rt lacks"),
                }
            }
            MAKECLOSURE => {
                let fb = k(i.a[0]);
                // SAFETY: MAKECLOSURE's constant: list(formals, body, srcref?)
                let (formals, body) = unsafe {
                    if fb.typ() != VECSXP || fb.std_len() < 2 {
                        return Err(Need::Fail);
                    }
                    let p = fb.std_data() as *const Sexp;
                    (*p, *p.add(1))
                };
                let r = self.push(t);
                let mut m = Ins::new(Op::MkClosure);
                m.a = r;
                m.e = self.k(formals);
                m.f = self.k(body);
                if tail {
                    m.fl |= F_VIS;
                }
                self.emit(m);
            }
            ADD | SUB | MUL | DIV | EXPT | EQ | NE | LT | LE | GE | GT | AND | OR => {
                let call = k(i.a[0]);
                let (kind, code, name): (Op, u8, &[u8]) = match i.op {
                    ADD => (Op::Arith, 1, b"+"),
                    SUB => (Op::Arith, 2, b"-"),
                    MUL => (Op::Arith, 3, b"*"),
                    DIV => (Op::Arith, 4, b"/"),
                    EXPT => (Op::Arith, 5, b"^"),
                    EQ => (Op::Relop, 1, b"=="),
                    NE => (Op::Relop, 2, b"!="),
                    LT => (Op::Relop, 3, b"<"),
                    LE => (Op::Relop, 4, b"<="),
                    GE => (Op::Relop, 5, b">="),
                    GT => (Op::Relop, 6, b">"),
                    AND => (Op::Logic, 1, b"&"),
                    _ => (Op::Logic, 2, b"|"),
                };
                let (dst, x) = self.consume(t, 2)?;
                self.inline_op(kind, code, name, call, dst, x, true, tail)?;
            }
            UMINUS | UPLUS | NOT => {
                let call = k(i.a[0]);
                let (dst, x) = self.consume(t, 1)?;
                match i.op {
                    NOT => self.inline_op(Op::Not, 0, b"!", call, dst, x, false, tail)?,
                    UPLUS => self.inline_op(Op::Arith1, 1, b"+", call, dst, x, false, tail)?,
                    _ => self.inline_op(Op::Arith1, 2, b"-", call, dst, x, false, tail)?,
                }
            }
            SQRT | EXP => {
                // FastMath1: a number's value at once, else the builtin
                let call = k(i.a[0]);
                let name: &[u8] = if i.op == SQRT { b"sqrt" } else { b"exp" };
                let Ok(prim) = self.prim_named(name) else {
                    return self.untranslated(i, "byte code calling a builtin robigo-rt lacks");
                };
                let (dst, x) = self.consume(t, 1)?;
                let kind = if i.op == SQRT { MATH_SQRT } else { MATH_EXP };
                self.math1(call, Some(prim), dst, x, 1, kind);
            }
            ISNULL | ISLOGICAL | ISINTEGER | ISDOUBLE | ISCOMPLEX | ISCHARACTER | ISSYMBOL | ISOBJECT => {
                // DO_ISTEST, DO_ISTYPE: the builtin's test, no dispatch; the
                // call is the expression the code is at (the opcode has none)
                let name: &[u8] = match i.op {
                    ISNULL => b"is.null",
                    ISLOGICAL => b"is.logical",
                    ISINTEGER => b"is.integer",
                    ISDOUBLE => b"is.double",
                    ISCOMPLEX => b"is.complex",
                    ISCHARACTER => b"is.character",
                    ISSYMBOL => b"is.symbol",
                    _ => b"is.object",
                };
                // SAFETY: the code's live constants.
                let Some(call) = (unsafe { t.expr_at(self.cx, i.at) }) else {
                    return self.untranslated(i, "byte code without its expressions index");
                };
                let (dst, x) = self.consume(t, 1)?;
                self.builtin_on(i, name, call, dst, x, 1)?;
            }
            SEQLEN | SEQALONG | COLON => {
                // a compact sequence for plain operands, else the builtin:
                // the builtin's value either way
                let call = k(i.a[0]);
                let (name, n): (&[u8], usize) = match i.op {
                    SEQLEN => (b"seq_len", 1),
                    SEQALONG => (b"seq_along", 1),
                    _ => (b":", 2),
                };
                let (dst, x) = self.consume(t, n)?;
                self.builtin_on(i, name, call, dst, x, n as u16)?;
            }
            MATH1 => {
                // a plain double's value (the table's C function), else the
                // builtin the call names (DO_MATH1's getPrimitive(CAR(call)));
                // getMath1Fun's check that the two agree
                let call = k(i.a[0]);
                let idx = i.a[1] as usize;
                // SAFETY: a call constant: its function, a symbol.
                let fname = unsafe { robigo_rtcore::Ref::from_sexp(call.car(), self.cx) }
                    .as_sym()
                    .map(|s| s.name_bytes().to_vec());
                let table = robigo_rtbuiltins::math::BC_MATH1.get(idx).map(|e| e.0.as_bytes());
                if fname.as_deref() != table {
                    return self.untranslated(
                        i,
                        "byte code whose MATH1 does not match its call (math1 compiler/interpreter mismatch)",
                    );
                }
                // SAFETY: a call constant.
                let prim = unsafe { self.prim_of(call.car()) }
                    .map(|o| self.st.objs[o].bind(self.cx).sexp())
                    // SAFETY: a live primitive.
                    .filter(|p| unsafe { p.typ() } == BUILTINSXP);
                let Some(prim) = prim else {
                    return self.untranslated(i, "byte code calling a builtin robigo-rt lacks");
                };
                let (dst, x) = self.consume(t, 1)?;
                self.math1(call, Some(prim), dst, x, 1, idx as u16);
            }
            DOTCALL => {
                // `.Call` on the function and the arguments (DO_DOTCALL's
                // do_dotcall)
                let call = k(i.a[0]);
                let n = i.a[1] as usize + 1;
                let (dst, x) = self.consume(t, n)?;
                self.builtin_on(i, b".Call", call, dst, x, n as u16)?;
            }
            LOG | LOGBASE => {
                // R_log or logbase of numbers at once, else log's evaluated
                // form (do_log_builtin)
                let call = k(i.a[0]);
                let n = if i.op == LOG { 1 } else { 2 };
                let (dst, x) = self.consume(t, n as usize)?;
                self.math1(call, None, dst, x, n, if i.op == LOG { MATH_LOG } else { MATH_LOGBASE });
            }
            DOLLAR if self.top_slot(t, 0) => {
                // a complex assignment's `$` getter
                let (call, sym) = (k(i.a[0]), k(i.a[1]));
                t.stack.pop();
                self.push_slot(t);
                let step = Step {
                    kind: ba::K_DOLLAR,
                    at: t.cur,
                    call,
                    what: sym,
                    vexpr: None,
                    args: Vec::new(),
                    fun_code: None,
                };
                record(t, false, step)?;
            }
            DOLLAR => {
                let (call, sym) = (k(i.a[0]), k(i.a[1]));
                let (dst, x) = self.consume(t, 1)?;
                let mut d = Ins::new(Op::Dollar);
                d.a = dst;
                d.b = x;
                d.e = self.k(call);
                d.c = self.k(sym);
                self.emit(d);
            }
            STARTSUBSET_N | STARTSUBSET2_N | STARTSUBSET | STARTSUBSET2 if self.top_slot(t, 0) => {
                // a complex assignment's `[` or `[[` getter: a method's
                // value at the label in the copy's place, as here
                let call = k(i.a[0]);
                let sub2 = matches!(i.op, STARTSUBSET2_N | STARTSUBSET2);
                let st = t.stack.clone();
                meet(t, i.a[1] as usize, st, self.next)?;
                if matches!(i.op, STARTSUBSET_N | STARTSUBSET2_N) {
                    t.subsets.push(Sub { dst: 0, obj: 0, sub2, assign: true });
                } else {
                    self.push_dispatch(t, sub2, call);
                }
            }
            STARTSUBSET_N | STARTSUBSET2_N | STARTSUBSET | STARTSUBSET2 => {
                // an object dispatches, its method's value going to the
                // label in the object's place; else the default method
                let call = k(i.a[0]);
                let to = i.a[1] as usize;
                let sub2 = matches!(i.op, STARTSUBSET2_N | STARTSUBSET2);
                let Some(Item { what: What::Val(x), starts }) = t.stack.last().cloned() else { return Err(Need::Fail) };
                let dst = self.reserve(t, x, &starts)?;
                let mut s = Ins::new(Op::StartSubset);
                s.a = dst;
                s.b = x;
                s.e = self.k(call);
                s.c = 1 + sub2 as u16;
                let at = self.emit(s);
                t.patches.push((at, to));
                let mut st = t.stack.clone();
                *st.last_mut().unwrap() = Item { what: What::Val(dst), starts: starts.clone() };
                meet(t, to, st, dst + 1)?;
                if matches!(i.op, STARTSUBSET_N | STARTSUBSET2_N) {
                    // the indices follow as values
                    t.subsets.push(Sub { dst, obj: x, sub2, assign: false });
                } else {
                    // the default method's arguments: the object (its tag
                    // the call's), then the pushes (DO_STARTDISPATCH)
                    let nil = self.nil();
                    // SAFETY: a call constant.
                    let rest = unsafe { call.cdr() };
                    if rest == nil {
                        return Err(Need::Fail);
                    }
                    // SAFETY: as above.
                    let tag = unsafe { rest.tag() };
                    let obj = PArg { kind: PKind::Val, tag: (tag != nil).then_some(tag), code: None };
                    let f = Frame { fun: Fun::Dispatch { sub2, call }, dst, base: x, args: vec![obj], fun_code: None };
                    t.stack.push(Item { what: What::Frame(f), starts });
                }
            }
            VECSUBSET | MATSUBSET | SUBSET_N | VECSUBSET2 | MATSUBSET2 | SUBSET2_N => {
                let call = k(i.a[0]);
                let sub2 = matches!(i.op, VECSUBSET2 | MATSUBSET2 | SUBSET2_N);
                let nidx = match i.op {
                    VECSUBSET | VECSUBSET2 => 1,
                    MATSUBSET | MATSUBSET2 => 2,
                    _ => i.a[1] as usize,
                };
                let s = t.subsets.pop().ok_or(Need::Fail)?;
                if s.assign {
                    // a complex assignment's getter: its indices' code
                    if s.sub2 != sub2 {
                        return Err(mismatch());
                    }
                    let args = self.index_codes(t, nidx)?;
                    for _ in 0..nidx {
                        self.pop(t)?;
                    }
                    if !self.top_slot(t, 0) {
                        return Err(mismatch());
                    }
                    t.stack.pop();
                    self.push_slot(t);
                    let kind = if sub2 { ba::K_SUB2 } else { ba::K_SUB };
                    // SAFETY: a call constant.
                    let what = unsafe { call.car() };
                    record(t, false, Step { kind, at: t.cur, call, what, vexpr: None, args, fun_code: None })?;
                    return Ok(());
                }
                let (x, starts) = self.pop_window(t, 1 + nidx)?;
                if x != s.obj || s.sub2 != sub2 {
                    return Err(mismatch());
                }
                self.dflt_subset(call, s.dst, x, 1 + nidx, sub2, &[]);
                self.push_at(t, s.dst, starts);
            }
            DFLTSUBSET | DFLTSUBSET2 if matches!(t.stack.last().map(|e| &e.what), Some(What::Frame(f)) if matches!(f.fun, Fun::AssignDispatch { .. })) =>
            {
                // a complex assignment's `[` or `[[` getter, for the default
                // method
                let Some(Item { what: What::Frame(f), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::AssignDispatch { sub2, call } = f.fun else { return Err(Need::Fail) };
                if sub2 != (i.op == DFLTSUBSET2) || !self.top_slot(t, 0) {
                    return Err(mismatch());
                }
                t.stack.pop();
                self.push_slot(t);
                let kind = if sub2 { ba::K_SUB2_PUNT } else { ba::K_SUB_PUNT };
                // SAFETY: a call constant.
                let what = unsafe { call.car() };
                let args = step_args(&f.args);
                record(t, false, Step { kind, at: t.cur, call, what, vexpr: None, args, fun_code: None })?;
            }
            DFLTSUBSET | DFLTSUBSET2 => {
                let Some(Item { what: What::Frame(f), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::Dispatch { sub2, call } = f.fun else { return Err(Need::Fail) };
                let Some(Item { what: What::Val(x), starts }) = t.stack.pop() else { return Err(Need::Fail) };
                if x != f.base || sub2 != (i.op == DFLTSUBSET2) {
                    return Err(mismatch());
                }
                if f.args.iter().any(|a| !matches!(a.kind, PKind::Val)) {
                    return self.untranslated(i, "byte code subsetting with '...'");
                }
                let tags: Vec<Option<Sexp>> = f.args.iter().map(|a| a.tag).collect();
                self.dflt_subset(call, f.dst, x, f.args.len(), sub2, &tags);
                self.push_at(t, f.dst, starts);
            }
            CHECKFUN => {
                // the function computed into its register: a call's frame,
                // its value's register below it (`attempt to apply
                // non-function` is the call's to raise, before any
                // argument is evaluated)
                let from = t.open.get(t.stack.len().wrapping_sub(1)).copied().unwrap_or(0);
                let Some(Item { what: What::Val(r), starts }) = t.stack.pop() else { return Err(Need::Fail) };
                let dst = self.reserve(t, r, &starts)?;
                self.next = r + 1;
                let fun = Fun::Found { get: None, sym: None };
                let f = Frame { fun, dst, base: r, args: Vec::new(), fun_code: Some((from, t.cur)) };
                t.stack.push(Item { what: What::Frame(f), starts });
            }
            INCLNKSTK => {
                let starts = starts_new(t);
                t.stack.push(Item { what: What::Mark, starts });
            }
            DECLNKSTK => {
                // the value takes the mark's place, and its starts
                let Some(Item { what: v @ What::Val(_), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let Some(Item { what: What::Mark, starts }) = t.stack.pop() else { return Err(Need::Fail) };
                t.stack.push(Item { what: v, starts });
            }
            SWITCH => {
                // EXPR popped; its value picks a jump of the tables that
                // follow (the names', then the numeric offsets')
                let (call, names) = (k(i.a[0]), k(i.a[1]));
                // SAFETY: constants of the byte code.
                let (co, io) = unsafe { (offsets(k(i.a[2]))?, offsets(k(i.a[3]))?) };
                // SAFETY: as above.
                let nn = unsafe { if names.typ() == STRSXP { names.std_len() as usize } else { 0 } };
                if io.is_empty() || nn != co.len() || (nn == 0 && self.nil() != names) {
                    return self.untranslated(i, "a malformed switch");
                }
                let r = self.pop(t)?;
                let mut sw = Ins::new(Op::Switch);
                sw.b = r;
                sw.e = self.k(call);
                sw.c = self.k(names);
                sw.f = u16::try_from(io.len()).map_err(|_| Need::Fail)?;
                self.emit(sw);
                for &to in co.iter().chain(&io) {
                    self.jump(t, Ins::new(Op::Jmp), to)?;
                }
            }
            STARTFOR => {
                // the sequence is the first of the loop's state registers
                let (call, sym, to) = (k(i.a[0]), k(i.a[1]), i.a[2] as usize);
                let Some(Item { what: What::Val(s), starts }) = t.stack.pop() else { return Err(Need::Fail) };
                self.next = s + 1;
                if self.alloc_n(super::FOR_STATE - 1) != s + 1 {
                    return Err(mismatch());
                }
                if !self.locals.contains(&sym) {
                    self.locals.push(sym);
                }
                let mut p = Ins::new(Op::ForPrep);
                p.a = s;
                p.e = self.k(sym);
                p.c = self.k(call);
                self.emit(p);
                t.stack.push(Item { what: What::For { state: s, sym, call }, starts });
                // to the label (STEPFOR, or a loop context's start); the
                // body after this is STEPFOR's jump's
                let body = t.code.get(t.cur + 1).map(|n| n.at).ok_or(Need::Fail)?;
                if to != body {
                    self.jump(t, Ins::new(Op::Jmp), to)?;
                }
                let st = t.stack.clone();
                meet(t, body, st, self.next)?;
            }
            STEPFOR => {
                // the next element, back to the body (a back-edge); the
                // end falls through
                let (state, sym, call) = t
                    .stack
                    .iter()
                    .rev()
                    .find_map(|e| match e.what {
                        What::For { state, sym, call } => Some((state, sym, call)),
                        _ => None,
                    })
                    .ok_or(Need::Fail)?;
                let mut s = Ins::new(Op::ForStep);
                s.a = state;
                s.e = self.k(sym);
                s.c = self.k(call);
                let at = self.emit(s);
                let after = t.code.get(t.cur + 1).map(|n| n.at).ok_or(Need::Fail)?;
                t.patches.push((at, after));
                self.jump(t, Ins::new(Op::BackJmp), i.a[0] as usize)?;
            }
            ENDFOR => {
                // the loop's value, NULL, in the sequence's place
                let Some(Item { what: What::For { state, .. }, starts }) = t.stack.pop() else {
                    return Err(Need::Fail);
                };
                let mut d = Ins::new(Op::ForDone);
                d.a = state;
                self.emit(d);
                for r in state..state + super::FOR_STATE as Reg {
                    self.clear(r);
                }
                self.next = state;
                let r = self.alloc();
                self.ld_nil(r, false);
                t.stack.push(Item { what: What::Val(r), starts });
            }
            STARTLOOPCNTXT => {
                // the loop's context: its region runs inside the record; a
                // `break` lands at the label (ENDLOOPCNTXT), a `next` here
                let mut l = Ins::new(Op::Loop);
                // SAFETY: the code's live constants.
                if let Some(e) = unsafe { t.expr_at(self.cx, i.at) } {
                    l.e = self.k(e);
                }
                // (registers from here are the region's temporaries)
                l.a = self.next;
                let at = self.emit(l);
                let starts = starts_new(t);
                t.stack.push(Item { what: What::LoopCtx { at }, starts });
                let st = t.stack.clone();
                meet(t, i.a[1] as usize, st, self.next)?;
            }
            ENDLOOPCNTXT => {
                let Some(Item { what: What::LoopCtx { at }, .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let end = self.emit(Ins::new(Op::LoopEnd));
                self.patch(at, end);
            }
            STARTASSIGN | STARTASSIGN2 if !planning => {
                // pass 2: the assignment whole, `BcAssign` of the plan its
                // opcodes make, the right-hand side's register its value
                let a = t.plan.as_ref().and_then(|p| p.at[t.cur].assigned.clone()).ok_or_else(mismatch)?;
                let r = self.top(t)?;
                // SAFETY: the code's live constants.
                let plan = unsafe { self.assign_plan(t, &a)? };
                let mut b = Ins::new(Op::BcAssign);
                b.a = r;
                b.b = r;
                b.e = self.k(plan);
                b.g = (i.op == STARTASSIGN2) as u16;
                self.emit(b);
                t.skip_to = Some(a.end);
            }
            STARTASSIGN | STARTASSIGN2 => {
                // pass 1: above the right-hand side, the target's cell, its
                // value and the right-hand side's copy
                if !matches!(t.stack.last().map(|e| &e.what), Some(What::Val(_))) {
                    return Err(Need::Fail);
                }
                for _ in 0..3 {
                    self.push_slot(t);
                }
                let a = Assign { end: 0, sym: k(i.a[0]), getters: Vec::new(), setters: Vec::new() };
                t.assigning.push((t.cur, a));
            }
            ENDASSIGN | ENDASSIGN2 => {
                // the cell and the new value off: the right-hand side's left
                if !planning || !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                t.stack.truncate(t.stack.len() - 2);
                let (start, mut a) = t.assigning.pop().ok_or(Need::Fail)?;
                if a.sym != k(i.a[0]) || a.setters.len() != a.getters.len() + 1 {
                    return self.untranslated(i, "a complex assignment of a shape cmp.R does not make");
                }
                a.end = t.cur;
                t.at[start].assigned = Some(a);
            }
            DUP2ND if !t.assigning.is_empty() && self.top_slot(t, 1) => self.push_slot(t),
            SWAP if !t.assigning.is_empty() && t.stack.len() >= 2 => {
                let n = t.stack.len();
                t.stack.swap(n - 1, n - 2);
            }
            GETTER_CALL | SETTER_CALL if !t.assigning.is_empty() => {
                // a generic getter or setter: its frame's function and
                // promises, on the left-hand value (and the right-hand side)
                let call = k(i.a[0]);
                let Some(Item { what: What::Frame(f), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::Found { sym, .. } = f.fun else { return Err(Need::Fail) };
                let setter = i.op == SETTER_CALL;
                // (below the frame: the left-hand value and the right-hand
                // side, which a setter's value replaces, a getter's goes
                // above)
                if !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                if setter {
                    t.stack.truncate(t.stack.len() - 2);
                }
                self.next = f.dst;
                self.push_slot(t);
                // SAFETY: a call constant.
                let what = sym.unwrap_or_else(|| unsafe { call.car() });
                // (the first argument is `*tmp*`'s PUSHNULLARG)
                let args = step_args(f.args.get(1..).unwrap_or(&[]));
                let vexpr = setter.then(|| k(i.a[1]));
                let step = Step { kind: ba::K_GENERIC, at: t.cur, call, what, vexpr, args, fun_code: f.fun_code };
                record(t, setter, step)?;
            }
            DOLLARGETS if !t.assigning.is_empty() => {
                let (call, sym) = (k(i.a[0]), k(i.a[1]));
                if !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                t.stack.truncate(t.stack.len() - 2);
                self.push_slot(t);
                // SAFETY: a call constant.
                let vexpr = Some(unsafe { last_arg(call) });
                let step =
                    Step { kind: ba::K_DOLLAR, at: t.cur, call, what: sym, vexpr, args: Vec::new(), fun_code: None };
                record(t, true, step)?;
            }
            STARTSUBASSIGN_N | STARTSUBASSIGN2_N | STARTSUBASSIGN | STARTSUBASSIGN2 if !t.assigning.is_empty() => {
                // a `[<-` or `[[<-` setter: a method's value at the label in
                // the left-hand value's place, the right-hand side off
                let call = k(i.a[0]);
                let sub2 = matches!(i.op, STARTSUBASSIGN2_N | STARTSUBASSIGN2);
                if !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                let mut st = t.stack.clone();
                st.pop();
                meet(t, i.a[1] as usize, st, self.next)?;
                if matches!(i.op, STARTSUBASSIGN_N | STARTSUBASSIGN2_N) {
                    t.subsets.push(Sub { dst: 0, obj: 0, sub2, assign: true });
                } else {
                    self.push_dispatch(t, sub2, call);
                }
            }
            VECSUBASSIGN | MATSUBASSIGN | SUBASSIGN_N | VECSUBASSIGN2 | MATSUBASSIGN2 | SUBASSIGN2_N
                if !t.assigning.is_empty() =>
            {
                let call = k(i.a[0]);
                let sub2 = matches!(i.op, VECSUBASSIGN2 | MATSUBASSIGN2 | SUBASSIGN2_N);
                let nidx = match i.op {
                    VECSUBASSIGN | VECSUBASSIGN2 => 1,
                    MATSUBASSIGN | MATSUBASSIGN2 => 2,
                    _ => i.a[1] as usize,
                };
                let s = t.subsets.pop().ok_or(Need::Fail)?;
                if !s.assign || s.sub2 != sub2 {
                    return Err(mismatch());
                }
                let args = self.index_codes(t, nidx)?;
                for _ in 0..nidx {
                    self.pop(t)?;
                }
                if !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                t.stack.truncate(t.stack.len() - 2);
                self.push_slot(t);
                let kind = if sub2 { ba::K_SUB2 } else { ba::K_SUB };
                // SAFETY: a call constant.
                let (what, vexpr) = unsafe { (call.car(), Some(last_arg(call))) };
                record(t, true, Step { kind, at: t.cur, call, what, vexpr, args, fun_code: None })?;
            }
            DFLTSUBASSIGN | DFLTSUBASSIGN2 if !t.assigning.is_empty() => {
                let Some(Item { what: What::Frame(f), .. }) = t.stack.pop() else { return Err(Need::Fail) };
                let Fun::AssignDispatch { sub2, call } = f.fun else { return Err(Need::Fail) };
                if sub2 != (i.op == DFLTSUBASSIGN2) || !self.top_slot(t, 0) || !self.top_slot(t, 1) {
                    return Err(mismatch());
                }
                t.stack.truncate(t.stack.len() - 2);
                self.push_slot(t);
                let kind = if sub2 { ba::K_SUB2_PUNT } else { ba::K_SUB_PUNT };
                // SAFETY: a call constant.
                let (what, vexpr) = unsafe { (call.car(), Some(last_arg(call))) };
                let args = step_args(&f.args);
                record(t, true, Step { kind, at: t.cur, call, what, vexpr, args, fun_code: None })?;
            }
            DOTSERR => {
                // `...` evaluated as a value (cmpSym): bcEval's error(),
                // reported against the context's call (so no call of its
                // own: R_NilValue fills the operand); the value it never
                // pushes keeps the stack's shape for what follows
                let nil = self.nil();
                self.error_op(nil, "'...' used in an incorrect context", false)?;
                self.push(t);
            }
            RETURNJMP => {
                // `return` in a promise: out of the function's context
                let r = self.pop(t)?;
                let mut ret = Ins::new(Op::Return);
                ret.b = r;
                self.emit(ret);
            }
            AND1ST | OR1ST => {
                let call = k(i.a[0]);
                let r = self.top(t)?;
                let mut a = Ins::new(Op::AndOr1);
                a.a = r;
                a.b = r;
                a.c = if i.op == AND1ST { 1 } else { 2 };
                a.e = self.k(call);
                // a tail `&&` (its join a way to the RETURN): bcEval's
                // AND1ST sets R_Visible on the path it jumps too, which is
                // the only one when its AND2ND is unreachable
                // (`a && return(x)`)
                let join = t.code.binary_search_by_key(&(i.a[1] as usize), |c| c.at);
                if join.is_ok_and(|j| returns_after(&t.code, j)) {
                    a.fl |= F_VIS;
                }
                self.jump(t, a, i.a[1] as usize)?;
            }
            AND2ND | OR2ND => {
                // the value replaces the first operand's (bcEval's
                // SETSTACK(-2)), where AND1ST's jump leaves it
                let call = k(i.a[0]);
                let y = self.pop(t)?;
                let x = self.top(t)?;
                let mut a = Ins::new(Op::AndOr2);
                a.a = x;
                a.b = y;
                a.c = if i.op == AND2ND { 1 } else { 2 };
                a.e = self.k(call);
                a.fl |= F_CLR_B;
                // (bcEval's AND2ND sets R_Visible; its AND1ST's jump the
                // same, above)
                if tail {
                    a.fl |= F_VIS;
                }
                self.emit(a);
            }
            BASEGUARD => {
                let call = k(i.a[0]);
                // the guarded code's value begins here, on both paths
                let depth = t.stack.len();
                let starts = match t.pending.take() {
                    Some((d, s)) if d == depth => s,
                    _ => vec![t.cur],
                };
                t.pending = Some((depth, starts.clone()));
                let mut g = Ins::new(Op::BaseGuard);
                g.e = self.k(call);
                // (a register for the function the guard finds)
                g.a = self.alloc();
                self.free_to(g.a);
                let at = self.emit(g);
                t.guards.push((i.a[1] as usize, at, call, starts));
            }
            _ => return self.untranslated(i, "this byte-code opcode"),
        }
        Ok(())
    }

    /// An opcode the translation does not know: the translation stops,
    /// and the code raises "robigo: ... is not implemented yet" when it
    /// runs ([`Comp::untranslated`]'s message, AGENTS rule 6).
    fn untranslated(&mut self, i: BcIns, what: &str) -> Result<(), Need> {
        let name = robigo_rtbuiltins::bytecode::OPCODES.get(i.op as usize).map_or("?", |o| o.0);
        self.bc_untranslated = Some(format!("{what} ({name})"));
        Err(Need::Fail)
    }

    /// An inlined arithmetic, comparison or logic operation on the window
    /// from `x` (two operands if `binary`), its value into `dst`.
    #[allow(clippy::too_many_arguments)]
    fn inline_op(
        &mut self,
        kind: Op,
        code: u8,
        name: &[u8],
        call: Sexp,
        dst: Reg,
        x: Reg,
        binary: bool,
        tail: bool,
    ) -> Result<(), Need> {
        let prim = self.prim_named(name)?;
        let mut i = Ins::new(kind);
        i.a = dst;
        i.b = x;
        i.c = x + 1;
        i.g = code as u16;
        i.e = self.k(call);
        i.f = self.k(prim);
        i.fl |= F_DEPTH | F_CLR_B | if binary { F_CLR_C } else { 0 };
        if tail {
            i.fl |= F_VIS;
        }
        self.emit(i);
        Ok(())
    }

    /// The primitive named `name`.
    fn prim_named(&self, name: &[u8]) -> Result<Sexp, Need> {
        let off = self.st.defs.iter().position(|d| d.name.as_bytes() == name && !d.internal()).ok_or(Need::Fail)?;
        Ok(self.st.objs[off].bind(self.cx).sexp())
    }

    /// The builtin `name` called on the `n` values from register `x` for
    /// `call`, its value into `dst`.
    fn builtin_on(&mut self, op: BcIns, name: &[u8], call: Sexp, dst: Reg, x: Reg, n: u16) -> Result<(), Need> {
        let Ok(prim) = self.prim_named(name) else {
            let what = format!("byte code calling '{}', which robigo-rt lacks", String::from_utf8_lossy(name));
            return self.untranslated(op, &what);
        };
        self.builtin_call(prim, call, dst, x, n);
        Ok(())
    }

    /// bcEval's scalar math ([`Op::Math1`]) of kind `kind` on the `n`
    /// operands from `x` for `call`, the builtin `prim` its slow path (None:
    /// `log`'s), its value into `dst`.
    fn math1(&mut self, call: Sexp, prim: Option<Sexp>, dst: Reg, x: Reg, n: u16, kind: u16) {
        let mut i = Ins::new(Op::Math1);
        i.a = dst;
        i.b = x;
        i.c = n;
        i.e = self.k(call);
        i.f = prim.map_or(0, |p| self.k(p));
        i.g = kind;
        i.fl |= F_DEPTH;
        self.emit(i);
    }

    /// The builtin `prim` called on the `n` values from register `x` for
    /// `call`, its value into `dst`.
    fn builtin_call(&mut self, prim: Sexp, call: Sexp, dst: Reg, x: Reg, n: u16) {
        let mut i = Ins::new(Op::CallBuiltin);
        i.a = dst;
        i.b = x;
        i.c = n;
        i.e = self.k(call);
        i.f = self.k(prim);
        i.fl |= F_DEPTH;
        self.emit(i);
    }

    /// The default subset method (`[`, or `[[` if `sub2`) on the window of
    /// `n` values from `x` (the object first) for `call`, its value into
    /// `dst`; `tags`, if any is a name, the arguments' names (after the
    /// window).
    fn dflt_subset(&mut self, call: Sexp, dst: Reg, x: Reg, n: usize, sub2: bool, tags: &[Option<Sexp>]) {
        let named = tags.iter().any(Option::is_some);
        if named {
            for (j, tag) in tags.iter().enumerate() {
                let r = self.alloc();
                debug_assert_eq!(r, x + (n + j) as Reg);
                let mut l = Ins::new(Op::LdConst);
                l.a = r;
                l.e = self.k(tag.unwrap_or_else(|| self.nil()));
                self.emit(l);
            }
        }
        let mut df = Ins::new(Op::DfltSubset);
        df.a = dst;
        df.b = x;
        df.c = n as u16;
        df.e = self.k(call);
        df.g = 1 + sub2 as u16;
        if named {
            df.fl |= F_NAMES;
        }
        self.emit(df);
    }

    /// `CALL` of `GETFUN`'s function: the generic call of `call`
    /// ([`Comp::generic_call`]'s shape), with its arguments' promise codes
    /// (`MAKEPROM`'s BCODESXPs, by argument: [`Op::CallCodes`]'s `f`),
    /// kept untranslated: each is translated at its promise's first force
    /// (`compile::bytecode_code`). Pass 1 (`planning`) keeps none.
    ///
    /// # Safety
    /// `call` is a live call constant of the byte code.
    unsafe fn call_found(&mut self, f: &Frame, get: Option<usize>, call: Sexp, planning: bool) -> Result<(), Need> {
        if f.args.iter().any(|a| matches!(a.kind, PKind::Val)) {
            // a value pushed into a closure call's frame: the IR's call
            // makes its arguments from the call alone
            self.bc_untranslated = Some("byte code pushing a value into a call's frame (PUSHARG)".to_owned());
            return Err(Need::Fail);
        }
        let mut codes_k = 0;
        // SAFETY: the caller's contract.
        unsafe {
            // GetFun reports against the call (findFun3's)
            if let Some(get) = get {
                self.ins[get].c = self.k(call);
            }
            // each MAKEPROM's code, by argument (the pushes are the call's
            // arguments, in order): the promise holds it, as GNU R's PRCODE
            let mut cell = call.cdr();
            let nil = self.nil();
            let mut codes = Vec::new();
            for a in &f.args {
                if cell == nil {
                    return Err(Need::Fail);
                }
                codes.push(match a.kind {
                    // the promise holds MAKEPROM's BCODESXP; its code is
                    // translated at its first force (compile::bytecode_code),
                    // as GNU R's bcEval runs it only then
                    PKind::Prom(code) if code.typ() == BCODESXP => code,
                    _ => nil,
                });
                cell = cell.cdr();
            }
            // (pass 1's code and pool are discarded: no vector for them)
            if !planning && let Some(v) = self.codes_vector(&codes)? {
                codes_k = self.k(v);
            }
        }
        let mut i = Ins::new(if codes_k != 0 { Op::CallCodes } else { Op::Call });
        i.a = f.dst;
        i.b = f.base;
        i.e = self.k(call);
        i.f = codes_k;
        i.c = if self.sites < NO_SITE {
            self.sites += 1;
            self.sites - 1
        } else {
            NO_SITE
        };
        // (the function's register cleared after, as the from-source
        // compiler's call)
        i.fl |= F_DEPTH | F_CLR_B;
        self.emit(i);
        Ok(())
    }

    /// The promise codes of a call's arguments, by argument
    /// (R_NilValue where an argument has none), as one VECSXP: a
    /// translated call's (`Op::CallCodes`'s `f`) or a step's (`S_CODES`).
    /// None when no argument has one.
    fn codes_vector(&self, codes: &[Sexp]) -> Result<Option<Sexp>, Need> {
        let nil = self.nil();
        if codes.iter().all(|&c| c == nil) {
            return Ok(None);
        }
        let v = self.cx.alloc_vector_raw(VECSXP, codes.len())?;
        for (j, &c) in codes.iter().enumerate() {
            // SAFETY: element `j` of a fresh vector of `codes.len()`, each a
            // live code object or R_NilValue (no collection since: the
            // allocation does not collect).
            unsafe { v.set_elt(j, c) };
        }
        Ok(Some(v))
    }

    /// `CALLBUILTIN`: the builtin `prim` on the frame's argument window.
    ///
    /// # Safety
    /// `call` is a live call constant of the byte code.
    unsafe fn call_builtin(&mut self, f: &Frame, prim: Sexp, internal: bool, call: Sexp) -> Result<(), Need> {
        let n = f.args.len();
        let mut i = Ins::new(if internal { Op::Internal } else { Op::CallBuiltin });
        i.a = f.dst;
        i.e = self.k(call);
        i.f = self.k(prim);
        i.fl |= F_DEPTH;
        if f.args.iter().any(|a| !matches!(a.kind, PKind::Val)) {
            // `...`, a missing argument or a promise in a builtin's frame:
            // its window holds values only (cmp.R calls such a builtin
            // through CALL)
            self.bc_untranslated = Some("byte code calling a builtin with '...' or a missing argument".to_owned());
            return Err(Need::Fail);
        }
        // SAFETY: `prim` is a live primitive of this isolate (the frame's).
        let off = unsafe { crate::prims::offset(prim) };
        if internal && !crate::call::internal_arity_ok(&self.st.defs[off], n) {
            // an .Internal of the wrong count: the VM's register call does
            // not check it, so the arguments (evaluated above, as GNU R's
            // frame holds them) are followed by checkArity's error, against
            // the context as for every .Internal (Rf_checkArityCall's error())
            // R: util.c:Rf_checkArityCall
            let def = &self.st.defs[off];
            let msg = crate::prims::arity_message(n, def.name, def.arity as usize, true);
            return self.error_op(call, &msg, false);
        }
        let named = f.args.iter().any(|a| a.tag.is_some());
        if named {
            for (j, a) in f.args.iter().enumerate() {
                let r = self.alloc();
                debug_assert_eq!(r, f.base + (n + j) as Reg);
                let mut l = Ins::new(Op::LdConst);
                l.a = r;
                l.e = self.k(a.tag.unwrap_or_else(|| self.nil()));
                self.emit(l);
            }
            i.fl |= F_NAMES;
        }
        i.b = f.base;
        i.c = n as u16;
        self.emit(i);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::opc::*;
    use super::{At, BcIns, PosIndex, decode, reachable, reserve_at};

    /// A jump to the code's end position (one past its last word) or into
    /// an operand is no instruction's start: `reachable` fails as the
    /// hashed index's miss failed (`Need::Fail`), not out of bounds.
    #[test]
    fn a_jump_to_the_end_or_an_operand_fails_as_before() {
        // version, GOTO 4 (the end: no word there), RETURN
        let to_end = [12, GOTO as i32, 4, RETURN as i32];
        // version, GOTO 2 (its own operand), RETURN
        let to_operand = [12, GOTO as i32, 2, RETURN as i32];
        // version, GOTO 3 (the RETURN), RETURN: translatable
        let ok = [12, GOTO as i32, 3, RETURN as i32];
        for (code, fails) in [(&to_end, true), (&to_operand, true), (&ok, false)] {
            let dec = decode(code).unwrap();
            let ix = PosIndex::of(&dec);
            assert_eq!(ix.get(4), None);
            // SAFETY: GOTO and RETURN read no constant.
            let r = unsafe { reachable(&dec, &ix, robigo_heap::Sexp::NULL) };
            assert_eq!(r.is_err(), fails, "{code:?}");
            if let Err(e) = r {
                assert!(matches!(e, crate::compile::Need::Fail), "{code:?}");
            }
        }
        // a slice's positions start where it does (an argument's code):
        // positions below and past it miss
        let slice = [BcIns { at: 40, op: RETURN, a: [0; 4] }];
        let ix = PosIndex::of(&slice);
        assert_eq!((ix.get(39), ix.get(40), ix.get(41), ix.get(1000)), (None, Some(0), None, None));
    }

    /// A consumer whose operand begins at the code's end (an `open` start
    /// equal to the instruction count) is reserved there, as the hashed
    /// plan keyed it, in bounds.
    #[test]
    fn a_start_at_the_codes_end_is_reserved_there() {
        // three instructions: consumers 0 and 2 begin at 3 (the end), 1 at 1
        let mut at: Vec<At> = (0..4).map(|_| At::default()).collect();
        for (c, s) in [(0, 3), (1, 1), (2, 3)] {
            at[c].cons = Some(vec![s]);
        }
        reserve_at(&mut at);
        let r: Vec<Vec<usize>> = at.iter().map(|a| a.res_at.clone()).collect();
        assert_eq!(r, vec![vec![], vec![1], vec![], vec![2, 0]]);
    }

    #[test]
    fn the_opcode_numbers_are_eval_cs() {
        let at = |n: &str| robigo_rtbuiltins::bytecode::OPCODES.iter().position(|&(m, _)| m == n).unwrap() as u8;
        for (n, v) in [
            ("RETURN", RETURN),
            ("LDCONST", LDCONST),
            ("GETFUN", GETFUN),
            ("CALL", CALL),
            ("MAKECLOSURE", MAKECLOSURE),
            ("ISNULL", ISNULL),
            ("ISNUMERIC", ISNUMERIC),
            ("AND1ST", AND1ST),
            ("SWITCH", SWITCH),
            ("SUBASSIGN2_N", SUBASSIGN2_N),
            ("BASEGUARD", BASEGUARD),
            ("DECLNKSTK", DECLNKSTK),
        ] {
            assert_eq!(at(n), v, "{n}");
        }
    }
}
