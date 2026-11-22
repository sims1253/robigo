//! `ROBIGO_BC_CROSSCHECK` (V3): a closure's translated byte code compared
//! with what the from-source compiler (V2's port of cmp.R) makes of its
//! source, the two IRs normalized: each value numbered by its definition
//! and each read by the definition it reads (a `Move` forwards it),
//! `Clear`s, `Nop`s and the clear flags dropped, `LdScalar` read as
//! `LdConst`, constants compared by `identical()`, jump targets by the
//! instructions they land on. Equal decisions give equal code; the
//! first difference is reported, classed as a gap the plan expects (a
//! divergence of the from-source port to fix later) or not.
//!
//! A closure's [`Finding`] is its [`Verdict`] and its line of the report.
//! The report goes to the file `ROBIGO_BC_CROSSCHECK_REPORT` names (one
//! line per closure, `same:` for one that does not differ), else to
//! stderr: `scripts/rt-ci.py` runs the conformance suite with both set and
//! fails on an unexpected line, and base.rdb's test counts the verdicts.

#![allow(unsafe_code)]

use crate::code::Built;
use crate::ir::{Ins, Op};
use robigo_heap::Sexp;
use robigo_rtcore::Cx;
use std::collections::HashMap;
use std::io::Write as _;

/// What the crosscheck of a closure found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The two codes are the same.
    Same,
    /// The first difference is of a class the plan expects.
    Expected,
    /// The first difference is of no expected class.
    Unexpected,
    /// The from-source compiler leaves the closure uncompiled, or its
    /// compile would force a binding.
    Skipped,
    /// The translation cannot follow the byte code.
    Untranslated,
}

/// A closure's verdict and its line of the report.
pub struct Finding {
    pub verdict: Verdict,
    pub line: String,
}

impl Finding {
    pub fn new(verdict: Verdict, line: String) -> Finding {
        Finding { verdict, line }
    }

    /// The line, to the report.
    pub fn report(&self) {
        report(&self.line);
    }
}

/// Whether `ROBIGO_BC_CROSSCHECK` is set (read once).
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ROBIGO_BC_CROSSCHECK").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// What an instruction field holds.
#[derive(Clone, Copy, PartialEq)]
enum K {
    /// A register it reads.
    R,
    /// A register it writes.
    W,
    /// The first register of a window it reads (`c` values, two per
    /// value with names).
    Win,
    /// A pool index.
    C,
    /// A pool index, or 0 for none.
    C0,
    /// A value compared as it is.
    I,
    /// Not compared (a call site's number, a jump target's half, unused).
    X,
}
use K::*;

/// The kinds of `a, b, c, e, f, g` and whether `(f, g)` is a jump target.
fn kinds(op: Op) -> ([K; 6], bool) {
    match op {
        Op::LdConst | Op::LdScalar | Op::GetVar | Op::GetDdVar | Op::GetVarMissOk | Op::Quote | Op::EvalConst => {
            ([W, X, X, C, X, X], false)
        }
        Op::SetVar | Op::SetVar2 => ([R, X, X, C, X, X], false),
        Op::Jmp | Op::BackJmp => ([X; 6], true),
        Op::BrFalse => ([X, R, X, C, X, X], true),
        Op::Guard => ([C0, X, C, C, X, X], true),
        Op::GetFun => ([W, X, C, C, I, X], false),
        // (a translated call's promise codes: the from-source compiler's
        // promises have none)
        Op::Call | Op::CallCodes => ([W, R, X, C, X, X], false),
        Op::CallBuiltin | Op::Internal => ([W, Win, I, C, C, C0], false),
        Op::Arith | Op::Relop | Op::Logic => ([W, R, R, C, C, I], false),
        Op::Arith1 | Op::Not => ([W, R, X, C, C, I], false),
        Op::AndOr1 => ([W, R, I, C, X, X], true),
        Op::AndOr2 => ([W, R, I, C, X, X], false),
        Op::MkClosure => ([W, X, C0, C, C, I], false),
        Op::Missing => ([W, X, X, C, C, X], false),
        Op::Return | Op::PpCheck => ([X, R, X, C0, X, X], false),
        Op::Ret => ([X, R, X, X, X, I], false),
        Op::Loop => ([X, X, X, X, X, X], true),
        Op::ForPrep => ([R, X, C, C, X, X], false),
        Op::ForStep => ([R, X, C, C, X, X], true),
        Op::ForDone => ([R, X, X, X, X, X], false),
        Op::SetVis => ([X, X, X, X, X, I], false),
        Op::ErrorCall => ([X, X, C0, C, C, I], false),
        Op::SpecialFn => ([W, X, C, C, X, X], false),
        Op::ComplexAssign => ([W, R, X, C, X, I], false),
        Op::BaseGuard => ([X, X, X, C, X, X], true),
        Op::BcAssign => ([W, R, X, C, X, I], false),
        Op::Switch => ([X, R, C, C, I, X], false),
        Op::StartSubset => ([W, R, I, C, X, X], true),
        Op::DfltSubset => ([W, Win, I, C, X, I], false),
        Op::Dollar => ([W, R, C, C, X, X], false),
        Op::Math1 => ([W, Win, I, C, C0, I], false),
        Op::BreakJump => ([X, X, X, X, X, I], false),
        _ => ([X; 6], false),
    }
}

/// Whether the normal form drops `i`.
fn dropped(i: &Ins) -> bool {
    matches!(i.op, Op::Nop | Op::Clear | Op::ClearRange | Op::Move | Op::LoopEnd)
}

/// One instruction normalized: its op, flags (no clear flags), operands.
#[derive(PartialEq)]
struct Norm {
    op: Op,
    fl: u8,
    regs: Vec<u32>,
    imm: Vec<u16>,
    consts: Vec<Sexp>,
    target: Option<usize>,
}

/// The instructions control can reach from the first, a jump `returns`
/// says is a return ending its path.
fn reachable(ins: &[Ins], returns: impl Fn(usize) -> bool) -> Vec<bool> {
    let mut live = vec![false; ins.len()];
    let mut todo = vec![0usize];
    while let Some(k) = todo.pop() {
        if k >= ins.len() || live[k] {
            continue;
        }
        live[k] = true;
        let i = &ins[k];
        let (_, jumps) = kinds(i.op);
        match i.op {
            Op::Jmp if returns(k) => {}
            Op::Jmp | Op::BackJmp => todo.push(i.target()),
            Op::Ret | Op::Return | Op::ErrorCall | Op::LoopEnd | Op::BreakJump => {}
            Op::Loop => {
                todo.push(k + 1);
                todo.push(i.target() + 1);
            }
            _ => {
                if jumps {
                    todo.push(i.target());
                }
                todo.push(k + 1);
            }
        }
        if i.op == Op::Switch {
            // (the jump tables after it: one is taken)
            let mut j = k + 1;
            while j < ins.len() && ins[j].op == Op::Jmp {
                todo.push(j);
                j += 1;
            }
        }
    }
    live
}

/// Where control at `k` leaves the code, if it does without doing more
/// than jump, drop (the instructions the normal form drops) and set the
/// visibility: the register the `Ret` (or `Return`) it reaches returns,
/// read through the `Move`s on the way, and the visibility it leaves (the
/// `Ret`'s, else the last `SetVis`'s: 1 visible, 2 invisible, 0 as it
/// was), and the instructions on the way.
fn exit(ins: &[Ins], k: usize) -> Option<(u16, u16, Vec<usize>)> {
    // the Moves and Clears on the way, in order: (to, from) a Move, (lo, hi)
    // the registers lo..hi a Clear clears (`from` None)
    let (mut j, mut ops, mut vis, mut path) = (k, Vec::new(), 0u16, Vec::new());
    for _ in 0..=ins.len() {
        let i = ins.get(j)?;
        path.push(j);
        match i.op {
            Op::Ret | Op::Return => {
                // the value read back through the Moves; a Clear of it on
                // the way returns no value of the instructions before
                let mut r = i.b;
                for &(a, b, mv) in ops.iter().rev() {
                    if mv && a == r {
                        r = b;
                    } else if !mv && (a..b).contains(&r) {
                        return None;
                    }
                }
                let g = if i.op == Op::Ret { i.g } else { 0 };
                return Some((r, if g != 0 { g } else { vis }, path));
            }
            Op::Jmp => j = i.target(),
            Op::Nop => j += 1,
            Op::Clear => {
                ops.push((i.a, i.a + 1, false));
                j += 1;
            }
            Op::ClearRange => {
                ops.push((i.a, i.b, false));
                j += 1;
            }
            Op::Move => {
                ops.push((i.a, i.b, true));
                j += 1;
            }
            Op::SetVis => {
                vis = if i.g != 0 { 1 } else { 2 };
                j += 1;
            }
            _ => return None,
        }
    }
    None
}

/// `b`'s instructions normalized: unreachable ones dropped, a way out of
/// the code (a `Ret`, a jump or a `SetVis` that leads to one,
/// [`exit`]) read as a `Ret` of the value it returns with the visibility
/// it leaves, a `Return` as one that keeps the visibility, a jump to the
/// next instruction dropped.
fn normalize(b: &Built) -> Vec<Norm> {
    // where a chain of jumps from `k` lands
    let land = |mut k: usize| {
        for _ in 0..b.ins.len() {
            match b.ins.get(k) {
                Some(i) if i.op == Op::Jmp => k = i.target(),
                _ => break,
            }
        }
        k
    };
    // a jump that leaves the code
    let ret = |k: usize| exit(&b.ins, k).map(|(r, vis, _)| (r, vis));
    let live = reachable(&b.ins, |k| ret(k).is_some());
    let mut keep: Vec<bool> = b.ins.iter().zip(&live).map(|(i, &l)| l && !dropped(i)).collect();
    // the instructions a jump lands on (one that leaves the code lands
    // nowhere: it is read as the way out)
    let mut targets = std::collections::HashSet::new();
    for (k, i) in b.ins.iter().enumerate() {
        if live[k] && kinds(i.op).1 && !(i.op == Op::Jmp && ret(k).is_some()) {
            targets.insert(i.target());
        }
    }
    // a `SetVis` on its way out is read as the `Ret`: the instructions it
    // falls through to on the way are its, but for one a jump lands on
    let mut vis_exit = vec![None; b.ins.len()];
    for (k, i) in b.ins.iter().enumerate() {
        if live[k]
            && i.op == Op::SetVis
            && let Some((r, vis, path)) = exit(&b.ins, k)
        {
            vis_exit[k] = Some((r, vis));
            for (n, &j) in path.iter().enumerate().skip(1) {
                if j != k + n || targets.contains(&j) {
                    break;
                }
                keep[j] = false;
            }
        }
    }
    // the normal position of each instruction (a dropped one's: the next's)
    let positions = |keep: &[bool]| {
        let mut at = Vec::with_capacity(keep.len() + 1);
        let mut n = 0;
        for &k in keep {
            at.push(n);
            n += k as usize;
        }
        at.push(n);
        at
    };
    let at0 = positions(&keep);
    for (k, i) in b.ins.iter().enumerate() {
        if keep[k] && i.op == Op::Jmp && ret(k).is_none() && at0[land(k).min(b.ins.len())] == at0[k] + 1 {
            keep[k] = false;
        }
    }
    let at = positions(&keep);
    // each register's current value: a definition's number, or (never
    // written) the register itself
    let mut defs: HashMap<u16, u32> = HashMap::new();
    let mut ndefs = 0u32;
    let read = |defs: &HashMap<u16, u32>, r: u16| defs.get(&r).copied().unwrap_or(1 << 20 | r as u32);
    let mut out: Vec<Norm> = Vec::new();
    // the instruction the last normalized one was made of
    let mut last: Option<usize> = None;
    // the values at an instruction forward jumps land on, from each jump,
    // and whether control falls into the next instruction: where no path
    // falls in and the jumps agree, the values are theirs, not those the
    // code before left (a join only a jump reaches: `a && return(x)`'s);
    // elsewhere they are numbered in the code's order
    let mut landing: HashMap<usize, Vec<HashMap<u16, u32>>> = HashMap::new();
    // (not where a back edge lands too: its values are the loop's)
    let back: std::collections::HashSet<usize> = (0..b.ins.len())
        .filter(|&k| live[k] && kinds(b.ins[k].op).1 && b.ins[k].target() <= k)
        .map(|k| b.ins[k].target())
        .collect();
    let mut fell = true;
    for (k, i) in b.ins.iter().enumerate() {
        if let Some(paths) = landing.remove(&k)
            && !fell
            && !back.contains(&k)
            && paths.iter().all(|p| *p == paths[0])
        {
            defs = paths.into_iter().next().unwrap_or_default();
        }
        'one: {
            if i.op == Op::Move && live[k] {
                // the value forwarded
                let v = read(&defs, i.b);
                defs.insert(i.a, v);
                break 'one;
            }
            if !keep[k] {
                break 'one;
            }
            // a way out: the value returned, the visibility left
            let r = match i.op {
                Op::Ret => Some((i.b, i.g)),
                Op::Return => Some((i.b, 0)),
                Op::Jmp => ret(k),
                Op::SetVis => vis_exit[k],
                _ => None,
            };
            if let Some((r, mut vis)) = r {
                // the instruction before, when only it falls through to the
                // way out (no jump lands between) and runs no code after it
                // sets the visibility (a load of a constant or a closure, not a
                // variable, whose promise may be forced after): its F_VIS is the
                // way out's visibility, unless a SetVis or the Ret sets it
                if let Some(p) = last
                    && !(p + 1..=k).any(|j| targets.contains(&j))
                    && let Some(prev) = out.last_mut()
                    && matches!(prev.op, Op::LdConst | Op::Quote | Op::MkClosure)
                    && prev.fl & crate::ir::F_VIS != 0
                {
                    prev.fl &= !crate::ir::F_VIS;
                    if vis == 0 {
                        vis = 1;
                    }
                }
                out.push(Norm {
                    op: Op::Ret,
                    fl: 0,
                    regs: vec![read(&defs, r)],
                    imm: vec![vis],
                    consts: Vec::new(),
                    target: None,
                });
                last = Some(k);
                break 'one;
            }
            let (ks, jumps) = kinds(i.op);
            let mut fields = [i.a, i.b, i.c, i.e, i.f, i.g];
            // an inlined simple `.Internal` wrapper (cmpSimpleInternal): the
            // port keeps the wrapper's call, the one an error reports, in `g`
            // beside the inlined call `e`; GNU R's byte code has the wrapper's
            // call as the call constant itself. Both report the same call, so
            // the call compared is the reported one (the builtin, `f`, and the
            // arguments' registers are compared as they are).
            if i.op == Op::Internal && i.g != 0 {
                fields[3] = i.g;
                fields[5] = 0;
            }
            let mut v = Norm {
                op: match i.op {
                    Op::LdScalar => Op::LdConst,
                    Op::CallCodes => Op::Call,
                    op => op,
                },
                fl: i.fl & !(crate::ir::F_CLR_B | crate::ir::F_CLR_C),
                regs: Vec::new(),
                imm: Vec::new(),
                consts: Vec::new(),
                target: None,
            };
            for (j, (&k, &x)) in ks.iter().zip(&fields).enumerate() {
                if jumps && j >= 4 {
                    continue;
                }
                match k {
                    R => v.regs.push(read(&defs, x)),
                    W => {
                        // (after the reads: the operands come first)
                    }
                    Win => {
                        let n = if i.c == crate::ir::SPECIAL_INTERNAL || i.c == u16::MAX {
                            0
                        } else {
                            i.c as usize * if i.fl & crate::ir::F_NAMES != 0 { 2 } else { 1 }
                        };
                        for r in x..x + n as u16 {
                            v.regs.push(read(&defs, r));
                        }
                    }
                    C | C0 if x as usize >= crate::code::FIRST_CONST => {
                        if let Some(&c) = b.consts.get(x as usize - crate::code::FIRST_CONST) {
                            v.consts.push(c);
                        }
                    }
                    C | C0 => v.imm.push(x),
                    I => v.imm.push(x),
                    X => {}
                }
            }
            if jumps {
                // (a jump to a return lands where that return is)
                let t = if ret(i.target()).is_some() { i.target() } else { land(i.target()) };
                v.target = Some(at.get(t).copied().unwrap_or(usize::MAX));
            }
            for (&k, &x) in ks.iter().zip(&fields) {
                if k == W {
                    defs.insert(x, ndefs);
                    v.regs.push(ndefs);
                    ndefs += 1;
                }
            }
            out.push(v);
            last = Some(k);
        }
        fell = live[k]
            && !matches!(
                i.op,
                Op::Jmp | Op::BackJmp | Op::Ret | Op::Return | Op::ErrorCall | Op::LoopEnd | Op::BreakJump
            );
        // (a jump out of the code lands nowhere: its way out read the
        // values; a back edge's are the loop's)
        if live[k] && kinds(i.op).1 && i.op != Op::Loop && i.target() > k && !(i.op == Op::Jmp && ret(k).is_some()) {
            landing.entry(i.target()).or_default().push(defs.clone());
        }
    }
    out
}

/// Whether two normalized instructions are the same: their constants
/// `identical()` (a complex assignment's plan but for the translation's
/// promise codes, [`crate::bcassign::same_but_codes`]), their jump targets
/// equal if `targets`.
fn same(cx: &Cx, a: &Norm, b: &Norm, targets: bool) -> bool {
    if a.op != b.op || a.fl != b.fl || a.regs != b.regs || a.imm != b.imm || (targets && a.target != b.target) {
        return false;
    }
    let eq = |x: Sexp, y: Sexp| {
        x == y || {
            // SAFETY: the pools' live constants (or their live elements).
            let (x, y) = unsafe { (robigo_rtcore::Ref::from_sexp(x, cx), robigo_rtcore::Ref::from_sexp(y, cx)) };
            robigo_rtbuiltins::identical::compute_identical(cx, x, y, 0).unwrap_or(false)
        }
    };
    a.consts.len() == b.consts.len()
        && a.consts.iter().zip(&b.consts).all(|(&x, &y)| {
            if a.op == Op::BcAssign {
                // SAFETY: `BcAssign`'s constant is its plan, live.
                unsafe { crate::bcassign::same_but_codes(x, y, &eq) }
            } else {
                eq(x, y)
            }
        })
}

/// A difference's class, if the plan expects it: a known gap of the
/// from-source port (V2) the translation does not share.
fn expected(cx: &Cx, nt: &[Norm], ns: &[Norm], k: usize) -> Option<&'static str> {
    let (t, s) = (nt.get(k), ns.get(k));
    let op = |n: Option<&Norm>| n.map(|n| n.op);
    match (op(t), op(s)) {
        // a nested function: the port compiles its source at once, the
        // translation translates its byte code when it is first called
        (Some(Op::MkClosure), Some(Op::MkClosure)) => Some("a nested function's code (compiled at once by the port)"),
        // a loop as cmp.R lays it out: no context when no `break` or `next`
        // can leave it from elsewhere (checkSkipLoopCntxt), a `for` loop's
        // step after its body; the port makes a context always, the step
        // first
        (_, Some(Op::Loop | Op::ForStep)) | (Some(Op::Loop | Op::ForStep), _) => {
            Some("a loop's shape (checkSkipLoopCntxt, cmpForBody)")
        }
        // a non-tail numeric switch's empty alternative: should its stop()
        // return, the translation raises not-implemented where bcEval goes
        // on with its stack unbalanced; the port falls into the default
        (Some(Op::ErrorCall), _) => Some("an unbalanced join the translation refuses (a switch's stop())"),
        // tryInline guards a base function whose handler then declines
        // (`sum(...)`: cmpBuiltin with `...`) and calls it; the port calls
        // it unguarded
        (Some(Op::BaseGuard), Some(Op::GetFun)) => Some("guard of a declined inline handler (tryInline)"),
        // `%%` and `%/%`: bcEval has no opcode for them, so cmp.R calls the
        // builtin (cmpBuiltin, CALLBUILTIN); the port's Arith computes
        // do_arith's MODOP and IDIVOP, the same value and conditions
        (Some(Op::CallBuiltin), Some(Op::Arith)) if t.zip(s).is_some_and(|(t, s)| modulus_call(cx, t, s)) => {
            Some("`%%` or `%/%`: cmp.R's CALLBUILTIN, the port's Arith")
        }
        _ => None,
    }
}

/// Whether the translation's `t` calls the builtin `%%` (or `%/%`) where
/// the port's `s` computes it (`Arith` with MODOP, IDIVOP): the same
/// primitive (the port's fallback), the same call and flags, the same
/// two operands, the same value defined.
fn modulus_call(cx: &Cx, t: &Norm, s: &Norm) -> bool {
    use robigo_rtbuiltins::kernel::arith::{IDIVOP, MODOP};
    let name = match s.imm.as_slice() {
        [g] if *g == MODOP as u16 => "%%",
        [g] if *g == IDIVOP as u16 => "%/%",
        _ => return false,
    };
    // (CallBuiltin: two arguments and no `g`, the call, the builtin;
    // Arith: the call, the builtin)
    let ([e, f], [se, sf]) = (t.consts.as_slice(), s.consts.as_slice()) else { return false };
    // SAFETY: the pool's live constant.
    let prim = unsafe { robigo_rtcore::Ref::from_sexp(*f, cx) };
    t.imm == [2, 0]
        && t.fl == s.fl
        && t.regs == s.regs
        && f == sf
        && crate::prims::prim_name(cx, prim) == Some(name)
        && {
            // SAFETY: the pools' live constants.
            let (x, y) = unsafe { (robigo_rtcore::Ref::from_sexp(*e, cx), robigo_rtcore::Ref::from_sexp(*se, cx)) };
            robigo_rtbuiltins::identical::compute_identical(cx, x, y, 0).unwrap_or(false)
        }
}

/// Compare the translation `t` of closure `what`'s byte code with the
/// from-source code `s`: the first difference, classed, or none.
pub fn compare(cx: &Cx, what: &str, t: &Built, s: &Built) -> Finding {
    let (nt, ns) = (normalize(t), normalize(s));
    let n = nt.len().max(ns.len());
    // the first difference of what the instructions do; then of where
    // they jump (a difference later on moves the targets before it)
    let differs = |targets: bool| {
        (0..n).find(|&k| match (nt.get(k), ns.get(k)) {
            (Some(a), Some(b)) => !same(cx, a, b, targets),
            _ => true,
        })
    };
    let Some(k) = differs(false).or_else(|| differs(true)) else {
        // (every closure compared has its line: the report says how many)
        return Finding::new(Verdict::Same, format!("same: {what}"));
    };
    // (a symbol constant by its name, a loaded constant deparsed, for
    // triage)
    let syms = |x: &Norm| {
        x.consts
            .iter()
            .filter_map(|&c| {
                // SAFETY: the pools' live constants.
                let r = unsafe { robigo_rtcore::Ref::from_sexp(c, cx) };
                if let Some(s) = r.as_sym() {
                    return Some(format!(" `{}`", String::from_utf8_lossy(s.name_bytes())));
                }
                if matches!(r.sexptype(), robigo_heap::types::BUILTINSXP | robigo_heap::types::SPECIALSXP) {
                    return crate::prims::prim_name(cx, r).map(|n| format!(" .Primitive({n})"));
                }
                (x.op == Op::LdConst).then(|| {
                    let d = crate::deparse::deparse1line(cx, r, false, 0).unwrap_or_default();
                    format!(" {{{}}}", String::from_utf8_lossy(&d).chars().take(40).collect::<String>())
                })
            })
            .collect::<String>()
    };
    let show = |x: Option<&Norm>| match x {
        Some(x) => format!(
            "{:?} fl {:#x} regs {:?} imm {:?} consts {}{} target {:?}",
            x.op,
            x.fl,
            x.regs,
            x.imm,
            x.consts.len(),
            syms(x),
            x.target
        ),
        None => "the end".to_owned(),
    };
    let (a, b) = (nt.get(k), ns.get(k));
    // `...` evaluated as a value: cmpSym's DOTSERR raises at once, the port
    // reads `...` and raises the same error then
    let dots = matches!((a, b), (Some(y), Some(x)) if y.op == Op::ErrorCall && x.op == Op::GetVar
        // SAFETY: GetVar's symbol constant, of the pool.
        && x.consts.first().is_some_and(|&sym| unsafe { robigo_rtcore::Ref::from_sexp(sym, cx) }
            .as_sym().is_some_and(|s| s.name_bytes() == b"...")));
    let (verdict, class) = match expected(cx, &nt, &ns, k) {
        _ if dots => {
            (Verdict::Expected, "expected ('...' as a value (cmpSym's DOTSERR; the port reads it))".to_owned())
        }
        Some(c) => (Verdict::Expected, format!("expected ({c})")),
        None => (Verdict::Unexpected, "unexpected".to_owned()),
    };
    let mut line = format!("{class}: {what}: at {k}: translated {}; from source {}", show(a), show(b));
    if std::env::var_os("ROBIGO_BC_CROSSCHECK_DUMP").is_some() {
        for (j, (x, y)) in (0..n).map(|j| (nt.get(j), ns.get(j))).enumerate() {
            line.push_str(&format!("\n    {j}: {} | {}", show(x), show(y)));
        }
    }
    Finding::new(verdict, line)
}

/// A line of the report. The conformance runner's parallel cases append to
/// one file: `writeln!` on an unbuffered `File` wrote the record and its
/// newline apart, another process's record landed between them, and two
/// lines merged into one (rt-ci counted 87,318 closures compared instead of
/// 97,485 with 24 jobs). So the whole record, newline and DUMP lines
/// included, is written under the file's exclusive lock: a short write's
/// remainder still follows it at once. A record that cannot be written is
/// said on stderr, which the conformance compares, so its case fails
/// instead of the report quietly losing it.
pub fn report(line: &str) {
    match std::env::var_os("ROBIGO_BC_CROSSCHECK_REPORT") {
        Some(p) => {
            let mut record = String::with_capacity(line.len() + 1);
            record.push_str(line);
            record.push('\n');
            let written = std::fs::OpenOptions::new().create(true).append(true).open(&p).and_then(|mut f| {
                f.lock()?;
                let r = f.write_all(record.as_bytes());
                let _ = f.unlock();
                r
            });
            if let Err(e) = written {
                eprintln!("robigo: bc crosscheck: cannot write the report {}: {e}", p.to_string_lossy());
            }
        }
        None => eprintln!("robigo: bc crosscheck: {line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(op: Op, a: u16, b: u16, g: u16) -> Ins {
        let mut i = Ins::new(op);
        (i.a, i.b, i.g) = (a, b, g);
        i
    }

    fn jmp(to: usize) -> Ins {
        let mut i = Ins::new(Op::Jmp);
        i.set_target(to);
        i
    }

    /// The register and the visibility a way out from 0 returns.
    fn out(code: &[Ins]) -> Option<(u16, u16)> {
        exit(code, 0).map(|(r, vis, _)| (r, vis))
    }

    #[test]
    fn a_way_out_reads_its_value_through_moves_and_clears() {
        // a Clear of the value returned: no value of the code before
        assert_eq!(out(&[jmp(1), ins(Op::Clear, 1, 0, 0), ins(Op::Ret, 0, 1, 0)]), None);
        assert_eq!(out(&[ins(Op::ClearRange, 0, 2, 0), ins(Op::Ret, 0, 1, 0)]), None);
        // of another register, or past the range's end: the value
        assert_eq!(out(&[jmp(1), ins(Op::Clear, 2, 0, 0), ins(Op::Ret, 0, 1, 0)]), Some((1, 0)));
        assert_eq!(out(&[ins(Op::ClearRange, 0, 1, 0), ins(Op::Ret, 0, 1, 0)]), Some((1, 0)));
        // a Move's source read before a Clear of it: the source's value
        let moved = [ins(Op::Move, 3, 1, 0), ins(Op::Clear, 1, 0, 0), ins(Op::Ret, 0, 3, 0)];
        assert_eq!(out(&moved), Some((1, 0)));
        // a Move of a register cleared before it: none
        let cleared = [ins(Op::Clear, 1, 0, 0), ins(Op::Move, 3, 1, 0), ins(Op::Ret, 0, 3, 0)];
        assert_eq!(out(&cleared), None);
        // anything else on the way: no way out
        assert_eq!(out(&[ins(Op::GetVar, 1, 0, 0), ins(Op::Ret, 0, 1, 0)]), None);
    }

    #[test]
    fn a_way_out_leaves_the_last_visibility_set() {
        assert_eq!(out(&[ins(Op::SetVis, 0, 0, 0), ins(Op::Ret, 0, 1, 0)]), Some((1, 2)));
        assert_eq!(out(&[ins(Op::SetVis, 0, 0, 1), jmp(2), ins(Op::Ret, 0, 1, 0)]), Some((1, 1)));
        // the Ret's own visibility sets it last
        assert_eq!(out(&[ins(Op::SetVis, 0, 0, 1), ins(Op::Ret, 0, 1, 2)]), Some((1, 2)));
        // a Return keeps it
        assert_eq!(out(&[ins(Op::Return, 0, 4, 0)]), Some((4, 0)));
    }
}
