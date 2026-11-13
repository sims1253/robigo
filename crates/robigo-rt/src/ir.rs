//! The IR (docs/g3.md §9.2): one instruction set for the VM and, later, the
//! native tiers. A code object is a sequence of [`Ins`] over the registers
//! of an activation: value-stack slots, tagged object / real / int / logical
//! / empty (§2.3, §9.2), so scalars stay unboxed between operations.
//!
//! # Effects and deoptimization
//!
//! Every instruction is one step of GNU R's evaluation: between two
//! instructions the VM state is exact (the registers, the program counter,
//! the context chain, `R_Visible`, the pending exits), so a native tier may
//! leave its code at any instruction boundary and the VM resumes there
//! (deopt at op granularity). [`Op::effect`] classifies each operation: the
//! pure ones may be reordered or repeated by a tier, the others are
//! committed in order and never replayed (no warning, callback or visible
//! store happens twice).
//!
//! # Evaluation points
//!
//! GNU R's `eval` counts every evaluation (`evalcount`, eval.c:1085) and
//! raises its depth for every non-self-evaluating one (eval.c:1144). An
//! instruction that stands for one `eval` of GNU R's carries [`F_TICK`]
//! (and [`F_DEPTH`] unless it evaluates a constant) with its static depth
//! `d` within the code object, so the VM keeps GNU R's counts in GNU R's
//! unit and order without an `eval` call per node (§3 item 7).
//!
//! # Dead registers (D9)
//!
//! An operand register read for the last time is cleared by the reading
//! instruction ([`F_CLR_B`], [`F_CLR_C`]), so no dead slot keeps an object
//! alive across a later safepoint (§2.3, last bullet: clear at last use).

/// A register of an activation: an index into its frame of value-stack
/// slots.
pub type Reg = u16;

/// The activation's environment (`rho`): always an object.
pub const R_ENV: Reg = 0;
/// The running code object (rooted here; the VM reloads its instruction
/// pointer from it after every safepoint).
pub const R_CODE: Reg = 1;
/// A scratch register for the raw layer's root arguments (a symbol, a
/// call), written just before the call that reads it and cleared after.
pub const R_SCRATCH: Reg = 2;
/// A second scratch register.
pub const R_SCRATCH2: Reg = 3;
/// The first register the compiler allocates.
pub const R_FIRST: Reg = 4;

/// The instruction is an evaluation point: tick GNU R's `evalcount`.
pub const F_TICK: u8 = 1;
/// The evaluation point is not a constant: GNU R raises its depth to
/// `base + d` there (and errors past `R_Expressions`).
pub const F_DEPTH: u8 = 2;
/// Clear operand register `b` after reading it (its last use).
pub const F_CLR_B: u8 = 4;
/// Clear operand register `c` after reading it (its last use).
pub const F_CLR_C: u8 = 8;
/// The value is the result of an evaluation in tail position: set
/// `R_Visible` to TRUE (GNU R's `eval` does on entry; only the last
/// evaluation of a code object can be observed).
pub const F_VIS: u8 = 16;
/// `CallBuiltin`: the argument window is followed by one name slot per
/// argument.
pub const F_NAMES: u8 = 32;

/// The operations. Operand fields of [`Ins`]: `a` is the destination (or
/// the source of a store), `b` and `c` operands, `e`, `f` constant-pool
/// indices; a jump target is `(f, g)` as one `u32` ([`Ins::target`]).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `a <- consts[e]`: a constant (an evaluation point of a
    /// self-evaluating object, or a value the code needs).
    LdConst,
    /// `a <- consts[e]` as an unboxed scalar (a plain length-1 double,
    /// integer or logical constant consumed by arithmetic: its identity is
    /// not observable there).
    LdScalar,
    /// `a <- eval(sym)` for `sym = consts[e]`: `R_findVar`, GNU R's errors
    /// ("object '%s' not found", a missing argument), a promise forced.
    GetVar,
    /// `a <- eval(..N)` for `sym = consts[e]` (`ddfindVar`).
    GetDdVar,
    /// `defineVar(consts[e], a, rho)` (`<-`, `=`), the value boxed.
    SetVar,
    /// `setVar(consts[e], a, ENCLOS(rho))` (`<<-`).
    SetVar2,
    /// Unconditional jump.
    Jmp,
    /// `if`'s condition: `asLogicalNoNA(b, consts[e])` (the call for its
    /// errors); jump when FALSE.
    BrFalse,
    /// The guard of an inlined base function: `consts[e]` (a symbol) names
    /// `consts[f2]` (`c` holds that index) from `rho` (`findFun3`, run no
    /// code); otherwise jump to the generic call.
    Guard,
    /// `a <- findFun3(consts[e], rho, consts[c])`.
    GetFun,
    /// The generic call: `a <- (b)(args of consts[e])` with the function
    /// in `b`, dispatched on its type at run time (closure: its arguments
    /// bound by `args::bind`, with call site `c`'s match plan; builtin: the
    /// arguments evaluated; special: the call compiled).
    Call,
    /// A builtin `consts[f]` called with the evaluated arguments in
    /// registers `b..b+c` (and their names after them with [`F_NAMES`]),
    /// for the call `consts[e]`.
    CallBuiltin,
    /// Binary arithmetic `a <- b OP c` (`OP` in `g`: arith.c's codes), the
    /// scalar kernel inline, else the builtin `consts[f]` for the call
    /// `consts[e]` (`b`, `c` consecutive).
    Arith,
    /// Unary arithmetic `a <- OP b` (`g`: PLUSOP or MINUSOP).
    Arith1,
    /// Comparison `a <- b OP c` (`g`: relop.c's codes).
    Relop,
    /// `a <- !b`.
    Not,
    /// Elementwise `&` (`g` = 1) or `|` (2): `a <- b OP c`.
    Logic,
    /// `&&` (`c` = 1) / `||` (2), first operand: `a <- asLogical2(b)` with
    /// do_logic2's checks for the call `consts[e]`; jump to `(f, g)` when it
    /// decides the result (`&&` FALSE, `||` TRUE).
    AndOr1,
    /// `&&` / `||`, second operand `b`: `a <- combine(a, b)`.
    AndOr2,
    /// `a <- function(formals = consts[e]) body = consts[f]` in `rho`
    /// (`mkCLOSXP`), sharing the compiled body cached at `consts[c]`.
    MkClosure,
    /// `quote(consts[e])`: `a <- consts[e]`, marked not mutable.
    Quote,
    /// `a <- missing(consts[e])` (`do_missing`), for the call `consts[f]`.
    Missing,
    /// `.Internal(consts[e])`: `a <-` the internal named by the call's
    /// function, its arguments evaluated as a builtin's.
    Internal,
    /// `return(b)`: leave the closure whose body this is, or jump to the
    /// function context of `rho` (`findcontext`).
    Return,
    /// The end of a code object: its value is `b`. `g`: the visibility of
    /// the value (0 keep, 1 visible, 2 invisible).
    Ret,
    /// A loop with a context (`CTXT_LOOP`, as `do_for`/`do_while`/
    /// `do_repeat` push): run the region up to its [`Op::LoopEnd`] at
    /// target `(f, g)`, inside the record; `break`/`next` jumps that land
    /// there from other code continue the loop, clearing the region's
    /// temporaries (registers from `a`).
    Loop,
    /// The end of a loop's region (the region's code returns here).
    LoopEnd,
    /// `for`'s set-up for the call `consts[c]`: the sequence in `a`
    /// (checked, link count raised), `defineVar(consts[e], R_NilValue)`;
    /// `a..a+5` become the loop's state (the sequence or the pairlist node,
    /// the index, the length, the loop variable's value, its binding cell).
    ForPrep,
    /// `for`'s step: the next element bound to `consts[e]`, or jump to
    /// `(f, g)` when the sequence is done.
    ForStep,
    /// A loop's safepoint at its back-edge, then jump to `(f, g)`.
    BackJmp,
    /// `break`/`next` in code whose loop is not lexically enclosing
    /// (`g` = CTXT_BREAK or CTXT_NEXT): `findcontext(g, rho)`.
    BreakJump,
    /// Clear register `a` (a value dropped before its region ends).
    Clear,
    /// Clear registers `a..b` (the temporaries a `break` or `next`
    /// abandons).
    ClearRange,
    /// `a <- b` (a copy between registers).
    Move,
    /// Set `R_Visible` to `g != 0`.
    SetVis,
    /// No operation (an evaluation point with nothing else to do).
    Nop,
    /// `a <- eval(consts[e])` for a constant `eval` treats specially (a
    /// promise, `...`, byte code).
    EvalConst,
    /// Raise GNU R's error `consts[f]` (a CHARSXP) for the call `consts[e]`
    /// (`errorcall` if `g` != 0, else `error`): a static error of a
    /// special's arguments, raised when the special runs.
    ErrorCall,
    /// `for`'s end: `DECREMENT_LINKS` of the sequence (register `a`).
    ForDone,
    /// A special that is a function over its call (`[`, `$<-`, `switch`,
    /// `on.exit`, ...): `a <- PRIMFUN(consts[c])(consts[e], rho)`, as
    /// `eval`'s SPECIALSXP case calls it (its guard emitted).
    SpecialFn,
    /// A complex assignment (`f(x) <- value`, `x[i] <- value`, `x$a <<-
    /// value`): `a <- applydefine(consts[e])` with the right-hand side's
    /// value in `b` (`g`: do_set's code, 1 `<-`, 2 `<<-`, 3 `=`).
    ComplexAssign,
}

/// What an operation may do that a native tier must keep in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// No effect beyond its destination register: may be moved or redone.
    Pure,
    /// May raise an error (an exit), but has no other effect.
    Fails,
    /// May run code, signal, store or collect: committed exactly once, in
    /// order.
    Any,
}

impl Op {
    /// The operation's effect class (docs/g3.md §9.2).
    pub const fn effect(self) -> Effect {
        match self {
            Op::LdConst | Op::LdScalar | Op::Jmp | Op::LoopEnd | Op::Clear | Op::ClearRange | Op::Move | Op::Nop => {
                Effect::Pure
            }
            Op::BrFalse | Op::Not | Op::Quote | Op::Guard => Effect::Fails,
            _ => Effect::Any,
        }
    }
}

/// One instruction: 16 bytes, plain data.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ins {
    pub op: Op,
    /// `F_*` flags.
    pub fl: u8,
    /// The static evaluation depth of an evaluation point (`F_TICK`).
    pub d: u16,
    pub a: Reg,
    pub b: Reg,
    pub c: u16,
    pub e: u16,
    pub f: u16,
    pub g: u16,
}

const _: () = assert!(size_of::<Ins>() == 16);

impl Ins {
    pub const fn new(op: Op) -> Ins {
        Ins { op, fl: 0, d: 0, a: 0, b: 0, c: 0, e: 0, f: 0, g: 0 }
    }

    /// The jump target `(f, g)`.
    #[inline(always)]
    pub fn target(&self) -> usize {
        self.f as usize | (self.g as usize) << 16
    }

    pub fn set_target(&mut self, t: usize) {
        assert!(t <= u32::MAX as usize, "robigo: a code object too large to compile");
        self.f = t as u16;
        self.g = (t >> 16) as u16;
    }
}

/// What a code object was compiled as.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// `eval.c`'s observable behaviour (the AST interpreter's).
    Eval = 0,
    /// GNU R's byte-code compiler's (cmp.R): constant arguments are values
    /// (`PUSHCONSTARG`), polls at `bcEval`'s points. The inlining decisions
    /// are V2's port of cmp.R (until then: the `Eval` profile's guards).
    Bc = 1,
}

/// A code object's header, the first bytes of its instruction blob.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Header {
    /// [`MAGIC`].
    pub magic: u32,
    pub profile: Profile,
    pub _pad: [u8; 3],
    /// Registers of an activation, the fixed ones included.
    pub nregs: u32,
    /// Instructions after the header.
    pub nins: u32,
    /// The deepest evaluation point (`d`).
    pub max_depth: u32,
    /// Closure call sites ([`Op::Call`]'s `c`): the match-plan cache has
    /// one entry per site.
    pub nsites: u32,
    /// This code object's identity for caches that must not key on an
    /// address (docs/g3.md §9.3): a per-isolate counter that never repeats.
    pub stamp: u64,
}

pub const MAGIC: u32 = 0x5247_3176; // "v1GR"

/// No call site (a call built at run time).
pub const NO_SITE: u16 = u16::MAX;

const _: () = assert!(size_of::<Header>() == 32 && size_of::<Header>() <= INS_OFFSET);

/// The instructions start at this byte offset of the blob (16-aligned).
pub const INS_OFFSET: usize = 32;
