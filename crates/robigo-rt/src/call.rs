//! Calls: closures (GNU R's `applyClosure`), builtins, specials reached
//! through a value, and loops' context regions.
//!
//! # The seam with C1
//!
//! Argument binding is C1's (docs/g3.md §6.2-6.4: slot frames, delayed and
//! forced slots, memoized match plans). Until it lands this module binds
//! arguments exactly as GNU R's AST interpreter does: a PROMSXP per
//! argument (`promiseArgs`), `matchArgs`' three passes, a pairlist frame
//! (`NewEnvironment`), default promises with `MISSING` 2 — which is also
//! D4's control variant (m1-plan X2: "`args::bind` allocates promises as
//! `eval.c:promiseArgs` does"). Everything that depends on how arguments
//! are bound goes through [`apply_closure`] and [`promise_args`], the
//! interface C1's `args::bind` replaces.

#![allow(unsafe_code)]

use crate::compile;
use crate::ir::{Profile, R_ENV, R_SCRATCH, R_SCRATCH2, Reg};
use crate::prims::{self, Kind};
use crate::vm::{self, Act, Flow, alloc_retry};
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Safepoint, Sexp};
use robigo_rtcore::{Args, Cx, Left, Out, RResult, RawBinding, RawFunctionContext, Ref, Root, Slot, ctxt};
use std::ptr::NonNull;

/// `findFun3(sym, rho, call)` into register `a`: the lookup that may run
/// code (a promise forced) and GNU R's errors.
pub fn find_fun(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp) -> RResult<()> {
    // R: eval.c:eval, "This picks the correct/better error expression for
    // replacement calls running in the AST interpreter."
    let g = cx.global_context();
    let ecall = if g.callflag() == ctxt::CCODE { g.call().sexp() } else { call };
    // SAFETY: an immortal symbol and a live call (a constant of the rooted
    // code, or the context's call).
    let (s, c) = unsafe { (act.scratch(R_SCRATCH, sym), act.scratch(R_SCRATCH2, ecall)) };
    let r = cx.find_fun3(s, act.root(R_ENV), Some(c), act.out(a));
    act.clear(R_SCRATCH);
    act.clear(R_SCRATCH2);
    r
}

/// The generic call: the function in register `rf`, the call `call`
/// (`eval`'s LANGSXP case after the function is found).
// R: eval.c:eval
pub fn call(cx: &mut Cx, act: &Act, dst: Reg, rf: Reg, call: Sexp) -> RResult<()> {
    let fun = act.obj(rf);
    // SAFETY: a register's object is live.
    match unsafe { fun.typ() } {
        CLOSXP => cx.scope(|cx, s| {
            // SAFETY: a constant of the rooted code.
            let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
            let args = promise_args(cx, act, call)?;
            // SAFETY: fresh, of this isolate.
            let args = s.root(cx, unsafe { Ref::from_sexp(args, cx) });
            apply_closure(cx, callr, act.root(rf), args, act.root(R_ENV), None, act.out(dst))
        }),
        BUILTINSXP => call_builtin_list(cx, act, dst, fun, call),
        SPECIALSXP => call_special(cx, act, dst, fun, call),
        _ => Err(cx.error("attempt to apply non-function")),
    }
}

/// What building a promise list or a frame can fail with under `&Cx`.
enum Fail {
    Gc(NeedGc),
    Dots,
}

impl From<NeedGc> for Fail {
    fn from(e: NeedGc) -> Fail {
        Fail::Gc(e)
    }
}

/// `promiseArgs(CDR(call), rho)`: a promise per argument, `...` expanded
/// into promises of its elements, empty arguments `R_MissingArg`, tags
/// kept. Under the `Bc` profile a constant argument is its value
/// (`PUSHCONSTARG`, eval.c:8042-8050).
// R: eval.c:promiseArgs
pub fn promise_args(cx: &mut Cx, act: &Act, call: Sexp) -> RResult<Sexp> {
    let profile = prims::state(cx).profile();
    let mut retried = None;
    loop {
        // SAFETY: the call is a constant of the rooted code and the
        // environment is rooted; nothing collects in this interval.
        match unsafe { build_promises(cx, call, act.rho(), profile) } {
            Ok(x) => return Ok(x),
            Err(Fail::Dots) => return Err(cx.error("'...' used in an incorrect context")),
            Err(Fail::Gc(e)) if retried == Some(e) => return alloc_retry(cx, |_| Err(e)),
            Err(Fail::Gc(e)) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
        }
    }
}

/// [`promise_args`]' `&Cx` phase.
///
/// # Safety
/// `call` a live call and `rho` a live environment of `cx`'s isolate,
/// reachable from roots.
unsafe fn build_promises(cx: &Cx, call: Sexp, rho: Sexp, profile: Profile) -> Result<Sexp, Fail> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    let mut items: Buf<(Sexp, Sexp)> = Buf::new((Sexp::NULL, Sexp::NULL));
    // SAFETY: the caller's contract; the walk only reads.
    unsafe {
        let mut el = call.cdr();
        while el != nil {
            let a = el.car();
            if a == dots {
                match cx.var_lookup_raw(dots, rho) {
                    RawBinding::Value(h) if h.typ() == DOTSXP || h == nil => {
                        let mut h = h;
                        while h != nil {
                            items.push((h.car(), h.tag()));
                            h = h.cdr();
                        }
                    }
                    RawBinding::Value(h) if h == missing => {}
                    _ => return Err(Fail::Dots),
                }
            } else {
                items.push((a, el.tag()));
            }
            el = el.cdr();
        }
        // build from the end: CONS(value, rest), tags set
        let mut list = nil;
        for &(a, tag) in items.iter().rev() {
            let v = if a == missing {
                missing
            } else if profile == Profile::Bc && is_constant(a) {
                a
            } else {
                mk_promise(cx, a, rho)?
            };
            list = cx.alloc_node_raw(LISTSXP, v, list, tag)?;
        }
        Ok(list)
    }
}

/// A constant argument of the `Bc` profile: not a symbol, call, promise
/// or byte code.
///
/// # Safety
/// `a` is live.
unsafe fn is_constant(a: Sexp) -> bool {
    // SAFETY: the caller's contract.
    !matches!(unsafe { a.typ() }, SYMSXP | LANGSXP | PROMSXP | BCODESXP | DOTSXP)
}

/// `mkPROMISE(expr, rho)`: PRVALUE R_UnboundValue, PRSEEN 0, the code
/// marked not mutable ("precaution to ensure code does not get modified via
/// substitute() and the like").
///
/// # Safety
/// `expr` and `rho` are live objects of `cx`'s isolate, reachable from roots.
// R: memory.c:mkPROMISE
pub unsafe fn mk_promise(cx: &Cx, expr: Sexp, rho: Sexp) -> Result<Sexp, NeedGc> {
    // SAFETY: the caller's contract; REFCNT is a header field.
    unsafe {
        if !expr.has_flag(robigo_heap::flags::IMMORTAL) {
            expr.set_refcnt(u16::MAX);
        }
        cx.alloc_node_raw(PROMSXP, cx.unbound().sexp(), expr, rho)
    }
}

/// GNU R's `applyClosure(call, op, arglist, rho, suppliedvars, TRUE)`:
/// match the arguments, make the frame, run the body in a function context
/// (`R_execClosure`), then release what the call no longer reaches
/// (`R_CleanupEnvir`, `unpromiseArgs`). `supplied` is usemethod's extra
/// variables (S1's; must be None until then).
// R: eval.c:applyClosure, applyClosure_core, make_applyClosure_env, R_execClosure
pub fn apply_closure(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    arglist: Root<'_>,
    rho: Root<'_>,
    supplied: Option<Root<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    assert!(supplied.is_none(), "robigo: usemethod's supplied variables are S1's");
    cx.safepoint(Safepoint::Entry)?;
    let res = out.root();
    cx.scope(|cx, s| {
        let newrho = make_env(cx, call, op, arglist)?;
        // SAFETY: fresh, of this isolate.
        let newrho = s.root(cx, unsafe { Ref::from_sexp(newrho, cx) });
        let code = closure_code(cx, op)?;
        // SAFETY: a code object of this isolate.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        // R: eval.c:applyClosure_core, "If we have a generic function we
        // need to use the sysparent of the generic as the sysparent of the
        // method"
        let g = cx.global_context();
        let sysparent = if g.callflag() == ctxt::GENERIC { g.sysparent().sexp() } else { rho.bind(cx).sexp() };
        let spec = RawFunctionContext {
            callflag: ctxt::RETURN,
            call: call.bind(cx).sexp(),
            cloenv: newrho.bind(cx).sexp(),
            sysparent,
            callfun: op.bind(cx).sexp(),
            promargs: Ok(arglist.bind(cx).sexp()),
        };
        // (sysparent is the caller's frame or a record's: rooted)
        let mut on_exit = |cx: &mut Cx, expr: Root<'_>, env: Root<'_>| -> RResult<()> {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                crate::eval::eval(cx, expr, env, o)
            })
        };
        // SAFETY: every object of `spec` is rooted in this scope or by the
        // caller (`sysparent`: the caller's environment, or a field of a
        // live record).
        unsafe {
            cx.function_context_raw(&spec, out, &mut on_exit, |cx, _id, out| {
                let slot = NonNull::new_unchecked(out.root().as_ptr() as *mut Slot);
                vm::exec(cx, code.bind(cx).sexp(), newrho.bind(cx).sexp(), slot, true)
            })?;
        }
        // R: eval.c:applyClosure_core (ADJUST_ENVIR_REFCNTS)
        let val = res.bind(cx).sexp();
        // SAFETY: rooted objects of this isolate.
        unsafe {
            cleanup_envir(cx, newrho.bind(cx).sexp(), val);
            unpromise_args(cx, arglist.bind(cx).sexp());
        }
        Ok(())
    })
}

/// The code object of closure `op`: its 4th word, compiled on first use
/// (the body's own cache shared when the body is a call).
pub fn closure_code(cx: &mut Cx, op: Root<'_>) -> RResult<Sexp> {
    let profile = prims::state(cx).profile();
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure: its 4th word is R_NilValue or a code object
    // this module stored.
    unsafe {
        let c = f.aux();
        if crate::code::is_code(c, profile) {
            return Ok(c);
        }
    }
    cx.scope(|cx, s| {
        // SAFETY: BODY of a rooted closure.
        let body = s.root(cx, unsafe { Ref::from_sexp(f.cdr(), cx) });
        let code = crate::eval::expr_code(cx, body)?;
        let f = op.bind(cx).sexp();
        // SAFETY: a rooted closure and a code object: the 4th word is
        // traced.
        unsafe { f.set_aux(code) };
        Ok(code)
    })
}

/// `make_applyClosure_env`: `matchArgs_RC`, `NewEnvironment`, the default
/// promises (`MISSING` 2), `NO_SPECIAL_SYMBOLS`.
// R: eval.c:make_applyClosure_env
fn make_env(cx: &mut Cx, call: Root<'_>, op: Root<'_>, arglist: Root<'_>) -> RResult<Sexp> {
    let mut retried = None;
    loop {
        // SAFETY: rooted inputs; nothing collects in this interval.
        let r = unsafe {
            let op = op.bind(cx).sexp();
            build_env(cx, op.car(), op.tag(), arglist.bind(cx).sexp())
        };
        match r {
            Ok(e) => return Ok(e),
            Err(MatchFail::Gc(e)) if retried == Some(e) => return alloc_retry(cx, |_| Err(e)),
            Err(MatchFail::Gc(e)) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
            Err(MatchFail::Error(m)) => {
                let msg = m.message(cx);
                return Err(cx.error_call(Some(call), &msg));
            }
        }
    }
}

/// `matchArgs`' errors (reported against the call).
enum MatchError {
    /// "formal argument \"%s\" matched by multiple actual arguments"
    FormalTwice(Sexp),
    /// "argument %d matches multiple formal arguments"
    ArgTwice(usize),
    /// "unused argument(s) %s"
    Unused,
}

impl MatchError {
    fn message(&self, cx: &Cx) -> String {
        match *self {
            MatchError::FormalTwice(s) => {
                // SAFETY: a formal's tag, a live symbol.
                let name = unsafe { Ref::from_sexp(s, cx) }.as_sym().unwrap().name();
                format!(
                    "formal argument \"{}\" matched by multiple actual arguments",
                    String::from_utf8_lossy(name.bytes())
                )
            }
            MatchError::ArgTwice(i) => format!("argument {i} matches multiple formal arguments"),
            // the message deparses the unused arguments (deparse1line):
            // B3's deparse
            MatchError::Unused => "robigo: the 'unused argument' error (deparse) is not implemented yet".to_owned(),
        }
    }
}

enum MatchFail {
    Gc(NeedGc),
    Error(MatchError),
}

impl From<NeedGc> for MatchFail {
    fn from(e: NeedGc) -> MatchFail {
        MatchFail::Gc(e)
    }
}

/// `pmatch(formal, tag, exact)`: `exact` or a prefix match.
///
/// # Safety
/// Both are live symbols.
// R: match.c:pmatch, psmatch
unsafe fn pmatch(formal: Sexp, tag: Sexp, exact: bool) -> bool {
    // SAFETY: the caller's contract.
    let (f, t) = unsafe { (formal.car().char_bytes(), tag.car().char_bytes()) };
    if exact { f == t } else { f.starts_with(t) }
}

/// [`make_env`]'s `&Cx` phase: `matchArgs_NR` (with side arrays for
/// `ARGUSED`), then the frame.
///
/// # Safety
/// Live, rooted objects of `cx`'s isolate.
// R: match.c:matchArgs_NR, eval.c:make_applyClosure_env, memory.c:NewEnvironment
unsafe fn build_env(cx: &Cx, formals: Sexp, savedrho: Sexp, supplied: Sexp) -> Result<Sexp, MatchFail> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    // SAFETY: the caller's contract; the lists are only read until the
    // frame is built from fresh cells.
    unsafe {
        let mut fs: Buf<Sexp> = Buf::new(Sexp::NULL);
        let mut f = formals;
        while f != nil {
            fs.push(f);
            f = f.cdr();
        }
        let mut sup: Buf<Sexp> = Buf::new(Sexp::NULL);
        let mut b = supplied;
        while b != nil {
            sup.push(b);
            b = b.cdr();
        }
        // actuals: value and MISSING per formal
        let mut val: Buf<Sexp> = Buf::filled(missing, fs.len());
        let mut miss: Buf<u16> = Buf::filled(1, fs.len());
        let mut fargused: Buf<u8> = Buf::filled(0, fs.len());
        let mut argused: Buf<u8> = Buf::filled(0, sup.len());
        // First pass: exact matches by tag
        for (fi, &f) in fs.iter().enumerate() {
            let ftag = f.tag();
            if ftag != dots && ftag != nil {
                for (bi, &b) in sup.iter().enumerate() {
                    let btag = b.tag();
                    if btag != nil && pmatch(ftag, btag, true) {
                        if fargused[fi] == 2 {
                            return Err(MatchFail::Error(MatchError::FormalTwice(ftag)));
                        }
                        if argused[bi] == 2 {
                            return Err(MatchFail::Error(MatchError::ArgTwice(bi + 1)));
                        }
                        val[fi] = b.car();
                        if b.car() != missing {
                            miss[fi] = 0;
                        }
                        argused[bi] = 2;
                        fargused[fi] = 2;
                    }
                }
            }
        }
        // Second pass: partial matches based on tags
        let mut dots_at: Option<usize> = None;
        let mut seendots = false;
        for (fi, &f) in fs.iter().enumerate() {
            if fargused[fi] == 0 {
                if f.tag() == dots && !seendots {
                    dots_at = Some(fi);
                    seendots = true;
                } else {
                    for (bi, &b) in sup.iter().enumerate() {
                        if argused[bi] != 2 && b.tag() != nil && pmatch(f.tag(), b.tag(), seendots) {
                            if argused[bi] != 0 {
                                return Err(MatchFail::Error(MatchError::ArgTwice(bi + 1)));
                            }
                            if fargused[fi] == 1 {
                                return Err(MatchFail::Error(MatchError::FormalTwice(f.tag())));
                            }
                            // (R_warn_partial_match_args: options are B2's;
                            // GNU R's default is FALSE)
                            val[fi] = b.car();
                            if b.car() != missing {
                                miss[fi] = 0;
                            }
                            argused[bi] = 1;
                            fargused[fi] = 1;
                        }
                    }
                }
            }
        }
        // Third pass: matches based on order
        let (mut fi, mut bi) = (0, 0);
        let mut seendots = false;
        while fi < fs.len() && bi < sup.len() && !seendots {
            let f = fs[fi];
            if f.tag() == dots {
                seendots = true;
                fi += 1;
            } else if val[fi] != missing {
                fi += 1;
            } else if argused[bi] != 0 || sup[bi].tag() != nil {
                bi += 1;
            } else {
                val[fi] = sup[bi].car();
                if sup[bi].car() != missing {
                    miss[fi] = 0;
                }
                argused[bi] = 1;
                bi += 1;
                fi += 1;
            }
        }
        if let Some(di) = dots_at {
            // Gobble up all unused actuals
            miss[di] = 0;
            let mut d = nil;
            for i in (0..sup.len()).rev() {
                if argused[i] == 0 {
                    d = cx.alloc_node_raw(DOTSXP, sup[i].car(), d, sup[i].tag())?;
                }
            }
            if d != nil {
                val[di] = d;
            }
        } else if argused.iter().any(|&u| u == 0) {
            return Err(MatchFail::Error(MatchError::Unused));
        }
        // the frame: the actuals in formals order, tagged with the formals
        // (NewEnvironment), then the defaults, which need the environment
        let mut frame = nil;
        for fi in (0..fs.len()).rev() {
            frame = cx.alloc_node_raw(LISTSXP, val[fi], frame, fs[fi].tag())?;
            frame.set_gp((frame.gp() & !15) | miss[fi]);
        }
        let env = cx.closure_env_raw(frame, savedrho)?;
        // "Use the default code for unbound formals."
        let mut a = frame;
        for &f in fs.iter() {
            if a.car() == missing && f.car() != missing {
                let p = mk_promise(cx, f.car(), env)?;
                a.set_car(p);
                a.set_gp((a.gp() & !15) | 2);
            }
            a = a.cdr();
        }
        Ok(env)
    }
}

/// `R_CleanupEnvir(rho, val)`: release the bindings and promises of a
/// call's frame that nothing can reach any more.
///
/// # Safety
/// Live objects of `cx`'s isolate.
// R: eval.c:R_CleanupEnvir, countCycleRefs, clearPromise, cleanupEnvDots
unsafe fn cleanup_envir(cx: &Cx, rho: Sexp, val: Sexp) {
    if val == rho {
        return;
    }
    let nil = cx.nil().sexp();
    let unbound = cx.unbound().sexp();
    // SAFETY: the caller's contract; the frame is GNU R's pairlist (N1b's
    // slot frames replace this walk).
    unsafe {
        let frame = robigo_heap::env::frame(rho);
        let mut refs = rho.refcnt() as i32;
        if refs > 0 {
            // countCycleRefs
            let mut crefs = 0;
            let mut b = frame;
            while b != nil && b.refcnt() == 1 {
                let v = b.car();
                if val != v {
                    match v.typ() {
                        PROMSXP if v.refcnt() == 1 && v.tag() == rho => crefs += 1,
                        CLOSXP if v.refcnt() == 1 && v.tag() == rho => crefs += 1,
                        ENVSXP if v == rho => crefs += 1,
                        _ => {}
                    }
                }
                b = b.cdr();
            }
            refs -= crefs;
        }
        if refs == 0 {
            let mut b = frame;
            while b != nil && b.refcnt() == 1 {
                let v = b.car();
                if v.refcnt() == 1 && v != val {
                    match v.typ() {
                        PROMSXP => clear_promise(v, unbound, nil),
                        DOTSXP => {
                            let mut d = v;
                            while d != nil && d.refcnt() == 1 {
                                let pv = d.car();
                                if pv.refcnt() == 1 && pv.typ() == PROMSXP {
                                    clear_promise(pv, unbound, nil);
                                }
                                d.set_car(nil);
                                d = d.cdr();
                            }
                        }
                        _ => {}
                    }
                }
                b.set_car(nil);
                b = b.cdr();
            }
            robigo_heap::env::set_enclos(rho, cx.empty_env().sexp());
        }
    }
}

/// `clearPromise(p)`.
///
/// # Safety
/// `p` is a live promise.
unsafe fn clear_promise(p: Sexp, unbound: Sexp, nil: Sexp) {
    // SAFETY: the caller's contract (no PROMISE_TAG values yet: C1's).
    unsafe {
        p.set_car(unbound);
        p.set_tag(nil);
        p.set_cdr(nil);
    }
}

/// `unpromiseArgs(pargs)`.
///
/// # Safety
/// `pargs` is a live pairlist no longer referenced.
// R: eval.c:unpromiseArgs
unsafe fn unpromise_args(cx: &Cx, mut pargs: Sexp) {
    let nil = cx.nil().sexp();
    let unbound = cx.unbound().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        while pargs != nil {
            let v = pargs.car();
            if v.typ() == PROMSXP && v.refcnt() == 1 {
                clear_promise(v, unbound, nil);
            }
            pargs.set_car(nil);
            pargs = pargs.cdr();
        }
    }
}

// ---- builtins -----------------------------------------------------------------

/// A builtin `fun` (or a builtin `.Internal`) called with the `n` evaluated
/// arguments in registers `base..` (their names after them if `names`),
/// for `call`: GNU R's `eval` BUILTINSXP case after `evalList`.
pub fn call_builtin_regs(
    cx: &mut Cx,
    act: &Act,
    dst: Reg,
    fun: Sexp,
    call: Sexp,
    base: Reg,
    n: usize,
    names: bool,
) -> RResult<()> {
    let mut any_alt = false;
    for r in base..base + n as Reg {
        let x = vm::boxed(cx, act, r)?;
        // SAFETY: a register's object is live.
        any_alt |= unsafe { x.is_altrep() };
    }
    // SAFETY: `n` object slots from `base` (and `n` name slots after them),
    // rooted by the activation for the call.
    let args = unsafe {
        let p = act.slot(base);
        Args::from_raw(p, names.then(|| p.add(n)), n as u32, any_alt)
    };
    let r = invoke_builtin(cx, act, dst, fun, call, args);
    for r in base..base + if names { 2 * n } else { n } as Reg {
        act.clear(r);
    }
    r
}

/// Call the builtin `fun` with `args` (rooted) for `call`, the value into
/// register `dst`, `R_Visible` as `PRIMPRINT` says; a safepoint on return.
fn invoke_builtin(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp, args: Args<'_>) -> RResult<()> {
    // SAFETY: a live primitive of this isolate.
    let def = *unsafe { prims::def(cx, fun) };
    let Kind::Builtin(f, _) = def.kind else {
        unreachable!("robigo: a special called as a builtin");
    };
    let flag = def.print();
    // R: eval.c:eval, BUILTINSXP
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    // SAFETY: a live call (a constant of the rooted code).
    let callr = unsafe { act.scratch(R_SCRATCH, call) };
    let r = f(cx, callr, args, act.root(R_ENV), act.out(dst));
    act.clear(R_SCRATCH);
    r?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    // a builtin's return is a safepoint (docs/g3.md §3 item 2)
    cx.safepoint(Safepoint::Return)
}

/// A builtin called with its arguments evaluated at run time: `evalList`
/// over the call's arguments, `...` expanded, in order.
// R: eval.c:evalList
pub fn call_builtin_list(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    let nil = cx.nil().sexp();
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    // The arguments as (expression, tag, position): `...` expanded by a
    // lookup that runs no code.
    let mut items: Vec<(Sexp, Sexp, usize)> = Vec::new();
    // SAFETY: the call is a constant of the rooted code, the `...` list is
    // bound in the rooted frame; this walk only reads.
    unsafe {
        let mut el = call.cdr();
        let mut n = 0;
        while el != nil {
            n += 1;
            let a = el.car();
            if a == dots {
                match cx.var_lookup_raw(dots, act.rho()) {
                    RawBinding::Value(h) if h.typ() == DOTSXP || h == nil => {
                        let mut h = h;
                        while h != nil {
                            items.push((h.car(), h.tag(), n));
                            h = h.cdr();
                        }
                    }
                    RawBinding::Value(h) if h == missing => {}
                    _ => return Err(cx.error("'...' used in an incorrect context")),
                }
            } else {
                items.push((a, el.tag(), n));
            }
            el = el.cdr();
        }
    }
    let n = items.len();
    cx.scope(|cx, s| {
        // the values' window, then the expressions (roots) and the names
        let values = s.frame(cx, n);
        let exprs = s.frame(cx, n);
        let names = s.frame(cx, n);
        // SAFETY: the expressions and tags were read in this interval from
        // rooted objects; the slots are this scope's.
        unsafe {
            for (i, &(e, t, _)) in items.iter().enumerate() {
                exprs.add(i).write(Slot::object(e));
                names.add(i).write(Slot::object(t));
            }
        }
        let mut any_alt = false;
        for (i, &(_, _, pos)) in items.iter().enumerate() {
            // SAFETY: slots of this scope.
            let (e, out) = unsafe { (Root::from_raw(exprs.add(i)), robigo_rtcore::Out::from_raw(values.add(i))) };
            if e.bind(cx).sexp() == missing {
                let msg = format!("argument {pos} is empty");
                // SAFETY: a live constant.
                let c = unsafe { act.scratch(R_SCRATCH, call) };
                return Err(cx.error_call(Some(c), &msg));
            }
            crate::eval::eval(cx, e, act.root(R_ENV), out)?;
            // SAFETY: as above.
            any_alt |= unsafe { Root::from_raw(values.add(i)) }.bind(cx).is_package_altrep();
        }
        // SAFETY: `n` object slots of this scope, and `n` name slots.
        let args = unsafe { Args::from_raw(values, Some(names), n as u32, any_alt) };
        // the function and the call: the function is in a register (the
        // generic call's) or a constant; the call a constant
        invoke_builtin(cx, act, dst, fun, call, args)
    })
}

/// A special reached through a value (`(if)(TRUE, 1)`, `f <- quote;
/// f(x)`): its call compiled with the special inlined, run in `rho`.
fn call_special(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    // SAFETY: a live primitive of this isolate.
    let off = unsafe { prims::offset(fun) };
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        // SAFETY: rooted.
        let code = unsafe { compile::compile_special(cx, callr.bind(cx).sexp(), off)? };
        // SAFETY: fresh.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        act.clear(dst);
        // SAFETY: the code and environment rooted; the destination is an
        // empty register of the activation.
        unsafe { vm::exec(cx, code.bind(cx).sexp(), act.rho(), act.slot(dst), false) }
    })
}

/// A loop's region in its `CTXT_LOOP` record (`do_for`, `do_while`,
/// `do_repeat`): run from `start` to the region's end; a `break` jump
/// landing here ends the loop, a `next` restarts the region.
pub fn loop_region(cx: &mut Cx, act: &mut Act, start: usize) -> RResult<Flow> {
    loop {
        let r = cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let base = s.root(cx, cx.base_env());
            // R: eval.c:do_for, begincontext(&cntxt, CTXT_LOOP, R_NilValue,
            // rho, R_BaseEnv, R_NilValue, R_NilValue)
            let env = act.root(R_ENV);
            cx.with_context(ctxt::LOOP, nil, env, base, None, |cx, _id| vm::run(cx, act, start))
        });
        match r {
            Ok(f) => return Ok(f),
            Err(Left::Landed(info)) if info.mask & ctxt::BREAK != 0 && info.mask & ctxt::NEXT == 0 => {
                return Ok(Flow::RegionEnd);
            }
            Err(Left::Landed(info)) if info.mask == ctxt::NEXT => continue,
            Err(Left::Landed(_)) => return Ok(Flow::RegionEnd),
            Err(Left::Exit(e)) => return Err(e),
        }
    }
}

/// A small vector on the stack (the common case: a call's arguments and a
/// closure's formals), spilling to the heap past [`INLINE`] elements: the
/// argument matcher runs on every call and must not allocate.
const INLINE: usize = 16;

struct Buf<T: Copy> {
    inline: [T; INLINE],
    len: usize,
    heap: Vec<T>,
}

impl<T: Copy> Buf<T> {
    fn new(fill: T) -> Buf<T> {
        Buf { inline: [fill; INLINE], len: 0, heap: Vec::new() }
    }

    fn filled(v: T, n: usize) -> Buf<T> {
        let mut b = Buf::new(v);
        if n <= INLINE {
            b.len = n;
        } else {
            b.heap = vec![v; n];
            b.len = n;
        }
        b
    }

    fn push(&mut self, v: T) {
        if self.len < INLINE && self.heap.is_empty() {
            self.inline[self.len] = v;
        } else {
            if self.heap.is_empty() {
                self.heap.extend_from_slice(&self.inline[..self.len]);
            }
            self.heap.push(v);
        }
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len
    }

    fn as_slice(&self) -> &[T] {
        if self.heap.is_empty() { &self.inline[..self.len] } else { &self.heap }
    }

    fn as_mut_slice(&mut self) -> &mut [T] {
        if self.heap.is_empty() { &mut self.inline[..self.len] } else { &mut self.heap }
    }

    fn iter(&self) -> std::slice::Iter<'_, T> {
        self.as_slice().iter()
    }
}

impl<T: Copy> std::ops::Index<usize> for Buf<T> {
    type Output = T;
    fn index(&self, i: usize) -> &T {
        &self.as_slice()[i]
    }
}

impl<T: Copy> std::ops::IndexMut<usize> for Buf<T> {
    fn index_mut(&mut self, i: usize) -> &mut T {
        &mut self.as_mut_slice()[i]
    }
}
