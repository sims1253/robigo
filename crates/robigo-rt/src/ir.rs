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

/// `Op::Internal`'s `c` for a special `.Internal`: its arguments are the
/// inner call's expressions, unevaluated (names.c's eval column Z = 0).
pub const SPECIAL_INTERNAL: u16 = u16::MAX - 1;

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
    /// code); otherwise jump to the generic call. With `a` (not 0) it is
    /// a check site's (`eval`'s `int save = R_PPStackTop` for a builtin or
    /// special): the protect stack's top into register `a` as an integer,
    /// for its [`Op::PpCheck`].
    Guard,
    /// `a <- findFun3(consts[e], rho, consts[c])`.
    GetFun,
    /// The generic call: `a <- (b)(args of consts[e])` with the function
    /// in `b`, dispatched on its type at run time (closure: its arguments
    /// bound by `args::bind`, with call site `c`'s match plan; builtin: the
    /// arguments evaluated; special: the call compiled). A translated
    /// call whose promises hold their own code is [`Op::CallCodes`].
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
    /// decides the result (`&&` FALSE, `||` TRUE). [`F_VIS`] on either
    /// path, as bcEval's AND1ST sets R_Visible jumping or not.
    AndOr1,
    /// `&&` / `||`, second operand `b`: `a <- combine(a, b)`.
    AndOr2,
    /// `a <- function(formals = consts[e]) body = consts[f]` in `rho`
    /// (`mkCLOSXP`). With `c` (not 0) the closure is compiled code made by
    /// compiled code (`MAKECLOSURE`): its code is `consts[c]`; with `g` 1
    /// it is base's installed byte code (compiled at its first call).
    MkClosure,
    /// `quote(consts[e])`: `a <- consts[e]`, marked not mutable.
    Quote,
    /// `a <- missing(consts[e])` (`do_missing`), for the call `consts[f]`.
    Missing,
    /// `.Internal(consts[e])`: `a <-` the internal named by the call's
    /// function, its arguments evaluated as a builtin's. With `g` (not 0)
    /// it stands for the call `consts[g]` of a wrapper the `Bc` profile
    /// inlined (cmp.R's `cmpSimpleInternal`): the expression `error()`
    /// reports.
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
    /// promise, `...`, byte code), or a call the AST evaluates (a failed
    /// [`Op::BaseGuard`]'s).
    EvalConst,
    /// Raise GNU R's error `consts[f]` (a CHARSXP) for the call `consts[e]`
    /// (`errorcall` if `g` != 0, else `error`): a static error of a
    /// special's arguments, raised when the special runs, or an `.Internal`'s
    /// checkArity error. Under byte code an `error` is reported as at the
    /// expression `consts[c]` when `c` is not 0 (the site's, as
    /// [`Op::Internal`]'s `g`).
    ErrorCall,
    /// `for`'s end: `DECREMENT_LINKS` of the sequence (register `a`).
    ForDone,
    /// A special that is a function over its call (`[`, `$<-`, `switch`,
    /// `on.exit`, ...): `a <- PRIMFUN(consts[c])(consts[e], rho)`, as
    /// `eval`'s SPECIALSXP case calls it (its guard emitted). With `g` 1
    /// it is an `.Internal` (`NextMethod`): `do_internal` calls it, under
    /// its PROTECT(args) and with its `check_stack_balance`.
    SpecialFn,
    /// A complex assignment (`f(x) <- value`, `x[i] <- value`, `x$a <<-
    /// value`): `a <- applydefine(consts[e])` with the right-hand side's
    /// value in `b` (`g`: do_set's code, 1 `<-`, 2 `<<-`, 3 `=`).
    ComplexAssign,
    /// bcEval's `BASEGUARD` for the call `consts[e]` of an inlined base
    /// function: unless `findFun(CAR(call), rho)` is base's binding of it
    /// (`SymbolValue`), jump to `(f, g)`, where the call is evaluated as
    /// `eval` does ([`Op::EvalConst`]).
    BaseGuard,
    /// A complex assignment compiled as cmp.R compiles it
    /// (`cmpComplexAssign`): the right-hand side's value in `b`, the plan
    /// `consts[e]` ([`crate::bcassign`]) run as bcEval's `STARTASSIGN`,
    /// getter calls, setter calls and `ENDASSIGN` do (`g` 1: `<<-`'s
    /// `STARTASSIGN2`, `ENDASSIGN2`); `a <-` the value, invisible.
    BcAssign,
    /// bcEval's `SWITCH` for the call `consts[e]` (cmp.R's `switch`
    /// handler): the value of `EXPR` in `b` (cleared) selects an entry of
    /// the jump tables that follow, one [`Op::Jmp`] each: the character
    /// table, one per name of `consts[c]` (NULL for none; the last name is
    /// "", the default), then the numeric table of `f` entries (the last
    /// the default).
    Switch,
    /// bcEval's `STARTSUBSET_N` and `STARTSUBSET` (cmp.R's `[` and `[[`
    /// handlers) for the call `consts[e]`: the object in `b`; an object
    /// that dispatches on `[` (`c` = 1) or `[[` (2) (`tryDispatch`, its
    /// other arguments the call's, promised) leaves the method's value in
    /// `a` and jumps to `(f, g)`.
    StartSubset,
    /// The default subset after [`Op::StartSubset`]: `a <-
    /// do_subset_dflt` (`g` = 1) or `do_subset2_dflt` (2) for the call
    /// `consts[e]` with the `c` arguments in `b..` (the object first; names
    /// after them with [`F_NAMES`]); `R_Visible` on.
    DfltSubset,
    /// bcEval's `DOLLAR` for the call `consts[e]`, the member the symbol
    /// `consts[c]`: `a <- b$member`, dispatching on `$` for an object;
    /// `R_Visible` on.
    Dollar,
    /// `a <- ` the value of `consts[e]` as bcEval's `GETVAR_MISSOK` reads
    /// it: a missing argument (bound to R_MissingArg, or a promise
    /// `R_isMissing` finds missing) is R_MissingArg.
    GetVarMissOk,
    /// `do_for`, `do_while` and `do_repeat` in the `Eval` profile: a loop
    /// evaluated in the global environment is compiled and run as byte
    /// code (`R_compileAndExecute`), the loop `consts[e]` at depth `d`; it
    /// then jumps to `(f, g)`, past the loop's own code.
    TopLoop,
    /// `eval`'s `check_stack_balance(op, save)` in the `Eval` profile, at
    /// the end of an inlined builtin's or special's code: the protect
    /// stack's top against register `b` (its guard saved it), reported
    /// for the primitive `consts[e]` with GNU R's top at the site, the
    /// activation's root protections plus the instruction's protect
    /// offset (robigo_rtcore's ppacct).
    PpCheck,
    /// `break` and `next` in the `Eval` profile: the `longjmp` to the
    /// loop's context restores the protect stack's top it saved
    /// (`R_restore_globals`; the VM jumps there instead).
    PpRestore,
    /// `a <-` the protect stack's top as an integer: a check site's `save`
    /// where no guard takes it (`.Internal` reached through a value).
    PpSave,
    /// A statement boundary of a `{` in the `Eval` profile (docs/g3.md
    /// §9.3): GNU R's `do_begin` reads each next statement from the live
    /// list, so when a write into the code's source has made this code
    /// stale, the rest of the `{` is `do_begin`'s from the cell after
    /// `consts[e]` (the `{` call at its start, else the cell of the
    /// statement just done), with the value so far in `b` ([`NO_REG`]:
    /// none, NULL), each statement evaluated by `eval` at depth `d`: `a <-`
    /// its value, and a jump to `(f, g)`, past the `{`'s own code; `c` 1:
    /// the statement done was an assignment, invisible should it be the
    /// last. Byte code has none: GNU R's keeps running.
    Resume,
    /// A loop iteration boundary in the `Eval` profile (§9.3): when a write
    /// into the code's source has made this code stale, the iteration is
    /// GNU R's: for `while` (`c` its call, else [`NO_CONST`]) the condition
    /// `CAR(consts[b])` (`do_while`'s `args`, read at each iteration)
    /// evaluated into `a`, FALSE ending the loop (the instruction after
    /// `(f, g)`); then the body `consts[e]` (the object the loop read when
    /// it started) evaluated into `a`, and a jump to the back-edge `(f, g)`.
    /// Both at depth `d`.
    LoopResume,
    /// bcEval's scalar math (`MATH1`, `SQRT`, `EXP`, `LOG`, `LOGBASE`;
    /// eval.c's `FastMath1`, `DO_MATH1`, `DO_LOG`, `DO_LOGBASE`): `c`
    /// operands from `b` (1, or 2 for `LOGBASE`), kind `g` ([`MATH_SQRT`]
    /// ..., or a `math1funs` index), the call `consts[e]`. A plain scalar
    /// operand's value is the C function's, its NaN of a non-NaN operand
    /// warning "NaNs produced" against the call; anything else calls the
    /// builtin `consts[f]` (0: `log`'s, `do_log_builtin`) on the window.
    /// The value into `a`, visible.
    Math1,
    /// [`Op::Call`] of a translated call whose promises hold their own
    /// code: `consts[f]`, its arguments' `MAKEPROM` BCODESXPs (a list by
    /// argument, R_NilValue for one that has none; `args::arg_code`), each
    /// promise's code as GNU R's PRCODE. Its own op, so that `Call`, the
    /// from-source compiler's, stays as it is on the hot path.
    ///
    /// Not inlined yet: `crate::inline` runs `Call` only (`qualifies`, the
    /// fused `GetFun`+`Call` step, the run's op match), so a translated
    /// closure whose body calls with codes is no candidate, and a call
    /// with codes is always the real call. It comes back with an entry of
    /// its own once translated code is the common path (V4 step 3, after
    /// L5; docs/ledger/g3/2026-11-17-v4-2-promise-code.md).
    CallCodes,
}

/// [`Op::Math1`]'s kinds past `math1funs`' indices.
pub const MATH_SQRT: u16 = 100;
pub const MATH_EXP: u16 = 101;
pub const MATH_LOG: u16 = 102;
pub const MATH_LOGBASE: u16 = 103;

/// [`Op::Resume`]'s `b` when no statement has run yet.
pub const NO_REG: Reg = Reg::MAX;
/// [`Op::LoopResume`]'s `c` for `for` and `repeat`.
pub const NO_CONST: u16 = u16::MAX;

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
    /// Every operation, in discriminant order: [`Op::from_u8`]'s table.
    pub const ALL: [Op; 56] = [
        Op::LdConst,
        Op::LdScalar,
        Op::GetVar,
        Op::GetDdVar,
        Op::SetVar,
        Op::SetVar2,
        Op::Jmp,
        Op::BrFalse,
        Op::Guard,
        Op::GetFun,
        Op::Call,
        Op::CallBuiltin,
        Op::Arith,
        Op::Arith1,
        Op::Relop,
        Op::Not,
        Op::Logic,
        Op::AndOr1,
        Op::AndOr2,
        Op::MkClosure,
        Op::Quote,
        Op::Missing,
        Op::Internal,
        Op::Return,
        Op::Ret,
        Op::Loop,
        Op::LoopEnd,
        Op::ForPrep,
        Op::ForStep,
        Op::BackJmp,
        Op::BreakJump,
        Op::Clear,
        Op::ClearRange,
        Op::Move,
        Op::SetVis,
        Op::Nop,
        Op::EvalConst,
        Op::ErrorCall,
        Op::ForDone,
        Op::SpecialFn,
        Op::ComplexAssign,
        Op::BaseGuard,
        Op::BcAssign,
        Op::Switch,
        Op::StartSubset,
        Op::DfltSubset,
        Op::Dollar,
        Op::GetVarMissOk,
        Op::TopLoop,
        Op::PpCheck,
        Op::PpRestore,
        Op::PpSave,
        Op::Resume,
        Op::LoopResume,
        Op::Math1,
        Op::CallCodes,
    ];

    /// The operation whose discriminant is `x`, or None: how an
    /// instruction read from outside the process (the translation cache,
    /// `crate::tcache`) is decoded, never by reinterpreting its bytes.
    pub const fn from_u8(x: u8) -> Option<Op> {
        Some(match x {
            0 => Op::LdConst,
            1 => Op::LdScalar,
            2 => Op::GetVar,
            3 => Op::GetDdVar,
            4 => Op::SetVar,
            5 => Op::SetVar2,
            6 => Op::Jmp,
            7 => Op::BrFalse,
            8 => Op::Guard,
            9 => Op::GetFun,
            10 => Op::Call,
            11 => Op::CallBuiltin,
            12 => Op::Arith,
            13 => Op::Arith1,
            14 => Op::Relop,
            15 => Op::Not,
            16 => Op::Logic,
            17 => Op::AndOr1,
            18 => Op::AndOr2,
            19 => Op::MkClosure,
            20 => Op::Quote,
            21 => Op::Missing,
            22 => Op::Internal,
            23 => Op::Return,
            24 => Op::Ret,
            25 => Op::Loop,
            26 => Op::LoopEnd,
            27 => Op::ForPrep,
            28 => Op::ForStep,
            29 => Op::BackJmp,
            30 => Op::BreakJump,
            31 => Op::Clear,
            32 => Op::ClearRange,
            33 => Op::Move,
            34 => Op::SetVis,
            35 => Op::Nop,
            36 => Op::EvalConst,
            37 => Op::ErrorCall,
            38 => Op::ForDone,
            39 => Op::SpecialFn,
            40 => Op::ComplexAssign,
            41 => Op::BaseGuard,
            42 => Op::BcAssign,
            43 => Op::Switch,
            44 => Op::StartSubset,
            45 => Op::DfltSubset,
            46 => Op::Dollar,
            47 => Op::GetVarMissOk,
            48 => Op::TopLoop,
            49 => Op::PpCheck,
            50 => Op::PpRestore,
            51 => Op::PpSave,
            52 => Op::Resume,
            53 => Op::LoopResume,
            54 => Op::Math1,
            55 => Op::CallCodes,
            _ => return None,
        })
    }
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
    /// (`PUSHCONSTARG`), promises it makes are byte code, polls at
    /// `bcEval`'s points. The inlining and frozen-binding decisions
    /// are cmp.R's ([`crate::cenv`]), taken when GNU R compiles.
    Bc = 1,
}

/// A code object's header, the first bytes of its instruction blob.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Header {
    /// [`MAGIC`].
    pub magic: u32,
    pub profile: Profile,
    /// Whether a compiled caller may run this closure code inline
    /// ([`crate::inline`]): 0 not yet decided, then [`crate::inline`]'s
    /// verdict, decided once for the code's formals (a copy for other
    /// formals, [`crate::code::reshape`], starts at 0 again).
    pub inline: u8,
    /// With an `inline` verdict of yes: the closure's formals (the first
    /// slots of its shape; locals follow).
    pub inline_formals: u8,
    pub _pad: [u8; 1],
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

#[cfg(test)]
mod op_tests {
    use super::Op;

    // Op::from_u8 decodes every discriminant, in order, and nothing past
    // the last
    #[test]
    fn from_u8_round_trips() {
        for (i, &op) in Op::ALL.iter().enumerate() {
            assert_eq!(op as u8 as usize, i);
            assert_eq!(Op::from_u8(i as u8), Some(op));
        }
        assert_eq!(Op::from_u8(Op::ALL.len() as u8), None);
        assert_eq!(Op::from_u8(u8::MAX), None);
    }
}
