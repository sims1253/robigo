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
//! The report goes to the file `ROBIGO_BC_CROSSCHECK_REPORT` names (one
//! line per closure compared, `same:` for one that does not differ), else
//! to stderr: `scripts/rt-ci.py` runs
//! the conformance suite with both set and fails on an unexpected line.

#![allow(unsafe_code)]

use crate::code::Built;
use crate::ir::{Ins, Op};
use robigo_heap::Sexp;
use robigo_rtcore::Cx;
use std::collections::HashMap;
use std::io::Write as _;

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
        Op::ErrorCall => ([X, X, X, C, C, I], false),
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

/// `b`'s instructions normalized: unreachable ones dropped, a jump to a
/// `Ret` (or a `Return`) read as the `Ret` itself, a jump to the next
/// instruction dropped.
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
    // a jump to a `Ret` returns its register
    let ret = |k: usize| match b.ins.get(land(k)) {
        Some(r) if r.op == Op::Ret => Some(r.b),
        _ => None,
    };
    let live = reachable(&b.ins, |k| ret(k).is_some());
    let mut keep: Vec<bool> = b.ins.iter().zip(&live).map(|(i, &l)| l && !dropped(i)).collect();
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
    let mut out = Vec::new();
    for (k, i) in b.ins.iter().enumerate() {
        if i.op == Op::Move && live[k] {
            // the value forwarded
            let v = read(&defs, i.b);
            defs.insert(i.a, v);
            continue;
        }
        if !keep[k] {
            continue;
        }
        // a return of a value: `Ret`, `Return`, a jump to a `Ret`
        let r = match i.op {
            Op::Ret | Op::Return => Some(i.b),
            Op::Jmp => ret(k),
            _ => None,
        };
        if let Some(r) = r {
            out.push(Norm {
                op: Op::Ret,
                fl: 0,
                regs: vec![read(&defs, r)],
                imm: Vec::new(),
                consts: Vec::new(),
                target: None,
            });
            continue;
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
fn expected(t: Option<&Norm>, s: Option<&Norm>, s_next: Option<&Norm>) -> Option<&'static str> {
    let op = |n: Option<&Norm>| n.map(|n| n.op);
    // (the two the same but for the port's F_VIS)
    let vis_only = matches!((t, s), (Some(x), Some(y)) if x.op == y.op
        && x.regs == y.regs
        && x.imm == y.imm
        && x.consts.len() == y.consts.len()
        && (x.fl ^ y.fl) == crate::ir::F_VIS);
    match (op(t), op(s)) {
        // a constant cmp.R folds (constantFold: `:`, a negated literal)
        // that the port computes
        (Some(Op::LdConst), Some(Op::LdConst))
            if matches!(op(s_next), Some(Op::LdConst | Op::CallBuiltin | Op::Arith | Op::Arith1)) =>
        {
            Some("a constant cmp.R folds (constantFold)")
        }
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
        (Some(Op::CallBuiltin), Some(Op::Arith))
            if s.is_some_and(|s| {
                s.imm.first().is_some_and(|&g| {
                    g == robigo_rtbuiltins::kernel::arith::MODOP as u16
                        || g == robigo_rtbuiltins::kernel::arith::IDIVOP as u16
                })
            }) =>
        {
            Some("`%%` or `%/%`: cmp.R's CALLBUILTIN, the port's Arith")
        }
        // a tail return's shape: GNU R's code returns from each branch (a
        // tail switch's alternatives, `return(if ...)`), the port jumps to
        // one return, the branch's visibility set where its value is made
        (Some(Op::Ret), Some(Op::Jmp | Op::SetVis)) => Some("a tail return's shape (one return a branch jumps to)"),
        // the same value, made visible where it is loaded (the port's
        // F_VIS) or at the return it reaches (GNU R's RETURN)
        _ if vis_only => Some("a tail return's shape (one return a branch jumps to)"),
        _ => None,
    }
}

/// Compare the translation `t` of closure `what`'s byte code with the
/// from-source code `s`, and report the first difference.
pub fn compare(cx: &Cx, what: &str, t: &Built, s: &Built) {
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
        report(&format!("same: {what}"));
        return;
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
                    return Some(format!(" `{}`", String::from_utf8_lossy(s.name().bytes())));
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
            .as_sym().is_some_and(|s| s.name().bytes() == b"...")));
    let class = match expected(a, b, ns.get(k + 1)) {
        _ if dots => "expected ('...' as a value (cmpSym's DOTSERR; the port reads it))".to_owned(),
        Some(c) => format!("expected ({c})"),
        None => "unexpected".to_owned(),
    };
    report(&format!("{class}: {what}: at {k}: translated {}; from source {}", show(a), show(b)));
    if std::env::var_os("ROBIGO_BC_CROSSCHECK_DUMP").is_some() {
        for (j, (x, y)) in (0..n).map(|j| (nt.get(j), ns.get(j))).enumerate() {
            report(&format!("    {j}: {} | {}", show(x), show(y)));
        }
    }
}

/// A line of the report.
pub fn report(line: &str) {
    match std::env::var_os("ROBIGO_BC_CROSSCHECK_REPORT") {
        Some(p) => {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{line}");
            }
        }
        None => eprintln!("robigo: bc crosscheck: {line}"),
    }
}
