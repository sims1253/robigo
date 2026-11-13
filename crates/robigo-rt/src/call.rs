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
use robigo_rtcore::{
    Args, Cx, Left, Out, RResult, RawBinding, RawFunctionContext, Ref, Root, Scope, Slot, SlotInit, ctxt,
};
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
            let args = promise_args(cx, act.root(R_ENV), callr)?;
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
pub fn promise_args(cx: &mut Cx, rho: Root<'_>, call: Root<'_>) -> RResult<Sexp> {
    let profile = prims::state(cx).profile();
    let mut retried = None;
    loop {
        // SAFETY: the call and the environment are rooted and read afresh;
        // nothing collects in this interval.
        match unsafe { build_promises(cx, call.bind(cx).sexp(), rho.bind(cx).sexp(), profile) } {
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
        let code = closure_code(cx, op)?;
        // SAFETY: a code object of this isolate.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        let newrho = s.out(cx);
        let newrho_r = newrho.root();
        make_env(cx, call, op, arglist, rho, code, newrho)?;
        let newrho = newrho_r;
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
        // R: eval.c:applyClosure_core (ADJUST_ENVIR_REFCNTS):
        // `unpromiseArgs`; `R_CleanupEnvir` of a slot frame is C1's (its
        // slot-world release, docs/g3.md §4)
        let _ = res;
        // SAFETY: a rooted pairlist of this isolate.
        unsafe { unpromise_args(cx, arglist.bind(cx).sexp()) };
        Ok(())
    })
}

/// The code object of closure `op`: its 4th word, compiled on first use
/// for its formals (whose shape the frame takes). Closures made by one
/// `function` expression share its body and formals, so the code object is
/// also cached on the body when it is a call.
pub fn closure_code(cx: &mut Cx, op: Root<'_>) -> RResult<Sexp> {
    let profile = prims::state(cx).profile();
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure: its 4th word is R_NilValue or a code object
    // this module stored; its body's 4th word too, if it is a call.
    unsafe {
        let (formals, body) = (f.car(), f.cdr());
        let c = f.aux();
        if crate::code::is_closure_code(c, profile, formals) {
            return Ok(c);
        }
        if body.typ() == LANGSXP {
            let c = body.aux();
            if crate::code::is_closure_code(c, profile, formals) {
                f.set_aux(c);
                return Ok(c);
            }
        }
    }
    // SAFETY: the rooted closure's body and formals.
    let code = unsafe { compile::compile_closure(cx, op)? };
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure and a code object: 4th words are traced.
    unsafe {
        f.set_aux(code);
        let body = f.cdr();
        if body.typ() == LANGSXP {
            body.set_aux(code);
        }
    }
    Ok(code)
}

/// `make_applyClosure_env`: `matchArgs_RC`, then the frame
/// (`NewEnvironment` with the defaults, MISSING 2: N1b's closure frame of
/// the code's shape), `NO_SPECIAL_SYMBOLS`; written to `out`.
// R: eval.c:make_applyClosure_env
fn make_env(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    arglist: Root<'_>,
    rho: Root<'_>,
    code: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let mut retried = None;
    let m = loop {
        // SAFETY: rooted inputs; nothing collects in this interval.
        let r = unsafe { match_args(cx, op.bind(cx).sexp().car(), arglist.bind(cx).sexp()) };
        match r {
            Ok(m) => break m,
            Err(MatchFail::Gc(e)) if retried == Some(e) => return alloc_retry(cx, |_| Err(e)).map(|_| ()),
            Err(MatchFail::Gc(e)) => {
                cx.need_gc(e)?;
                retried = Some(e);
            }
            Err(MatchFail::Error(m)) => {
                let msg = m.message(cx);
                return Err(cx.error_call(Some(call), &msg));
            }
        }
    };
    let missing = cx.missing_arg().sexp();
    cx.scope(|cx, s| {
        // root every matched value at once (a fresh `...` list among them):
        // nothing has collected since `match_args`
        let n = m.vals.len();
        let mut roots: Buf<Option<Root<'_>>> = Buf::filled(None, n);
        for i in 0..n {
            let v = m.vals[i];
            if v != missing {
                // SAFETY: a value of the rooted argument list, or a fresh
                // `...` list of them.
                roots[i] = Some(s.root(cx, unsafe { Ref::from_sexp(v, cx) }));
            }
        }
        // the defaults: the formals' own expressions
        let mut inits: Buf<SlotInit<'_>> = Buf::filled(SlotInit::Missing, n);
        let mut f = op.bind(cx).sexp();
        // SAFETY: the rooted closure's formals, a pairlist of n cells.
        unsafe {
            f = f.car();
            for i in 0..n {
                inits[i] = match roots[i] {
                    Some(r) => SlotInit::Value(r),
                    None if f.car() != missing => SlotInit::Default(s.root(cx, Ref::from_sexp(f.car(), cx))),
                    None => SlotInit::Missing,
                };
                f = f.cdr();
            }
        }
        // SAFETY: the code's shape, a rooted part of the code object.
        let shape = s.root(cx, unsafe { Ref::from_sexp(crate::code::shape(code.bind(cx).sexp()), cx) });
        // SAFETY: CLOENV of the rooted closure.
        let enclos = s.root(cx, unsafe { Ref::from_sexp(op.bind(cx).sexp().tag(), cx) });
        let env = out.root();
        cx.new_closure_frame(shape, enclos, rho, inits.as_slice(), out)?;
        if cx.env_has_no_special_symbols(env.bind(cx)) {
            cx.set_no_special_symbols(env);
        }
        Ok(())
    })
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

/// What `matchArgs` made: per formal, the value bound (R_MissingArg when
/// none, a fresh DOTSXP for `...`) and its `MISSING`.
struct Matched {
    vals: Buf<Sexp>,
}

/// [`make_env`]'s `&Cx` phase: `matchArgs_NR` (with side arrays for
/// `ARGUSED`).
///
/// # Safety
/// Live, rooted objects of `cx`'s isolate.
// R: match.c:matchArgs_NR, eval.c:make_applyClosure_env, memory.c:NewEnvironment
unsafe fn match_args(cx: &Cx, formals: Sexp, supplied: Sexp) -> Result<Matched, MatchFail> {
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
        let _ = miss;
        Ok(Matched { vals: val })
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

/// The isolate's `apply` hook (B1's [`robigo_rtcore::ApplyFn`]): call
/// `fun` with the evaluated arguments (or promises) `args` for `call` from
/// `env`, as `R_forceAndCall` and `lapply`'s `FUN(X[[i]], ...)` do: a
/// closure gets them as its promise list, a builtin directly.
pub fn apply_hook(
    cx: &mut Cx,
    call: Root<'_>,
    fun: Root<'_>,
    args: Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let f = fun.bind(cx).sexp();
    // SAFETY: a rooted object.
    match unsafe { f.typ() } {
        CLOSXP => cx.scope(|cx, s| {
            let list = s.out(cx);
            let lr = list.root();
            cx.build(list, |cx| {
                let mut l = cx.nil();
                for i in (0..args.len()).rev() {
                    let v = args.get(cx, i);
                    // SAFETY: rooted values, symbols and the list built so
                    // far; nothing collects in this interval.
                    l = unsafe {
                        let tag = args.name(cx, i).map_or(cx.nil().sexp(), |n| n.as_ref().sexp());
                        Ref::from_sexp(cx.alloc_node_raw(LISTSXP, v.sexp(), l.sexp(), tag)?, cx)
                    };
                }
                Ok(l)
            })?;
            apply_closure(cx, call, fun, lr, env, None, out)
        }),
        BUILTINSXP => {
            // SAFETY: a live primitive of this isolate.
            let def = *unsafe { prims::def(cx, f) };
            let Kind::Builtin(fun, _) = def.kind else { unreachable!() };
            let flag = def.print();
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            fun(cx, call, args, env, out)?;
            if flag < 2 {
                cx.set_visible(flag != 1);
            }
            Ok(())
        }
        // R_forceAndCall: a special takes the call's own arguments
        SPECIALSXP => crate::apply::apply_special(cx, call, fun, env, out),
        _ => Err(cx.error("attempt to apply non-function")),
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
    // the call and the function in the scratch registers before anything
    // can collect (boxing may)
    // SAFETY: constants of the rooted code.
    unsafe {
        act.scratch(R_SCRATCH, call);
        act.scratch(R_SCRATCH2, fun);
    }
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
    let r = invoke_builtin(cx, act, dst, args);
    for r in base..base + if names { 2 * n } else { n } as Reg {
        act.clear(r);
    }
    r
}

/// Call the builtin in the scratch register `R_SCRATCH2` with `args`
/// (rooted) for the call in `R_SCRATCH`, the value into register `dst`,
/// `R_Visible` as `PRIMPRINT` says; a safepoint on return. Clears the
/// scratch registers.
fn invoke_builtin(cx: &mut Cx, act: &Act, dst: Reg, args: Args<'_>) -> RResult<()> {
    let fun = act.obj(R_SCRATCH2);
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
    let r = f(cx, act.root(R_SCRATCH), args, act.root(R_ENV), act.out(dst));
    act.clear(R_SCRATCH);
    act.clear(R_SCRATCH2);
    r?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    // a builtin's return is a safepoint (docs/g3.md §3 item 2)
    cx.safepoint(Safepoint::Return)
}

/// A builtin called with its arguments evaluated at run time, for the
/// call in register-free form ([`builtin_evallist`]).
pub fn call_builtin_list(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: constants of the rooted code (or a register's function).
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
        builtin_evallist(cx, callr, funr, act.root(R_ENV), act.out(dst))
    })
}

/// `evalList` over the call's arguments in order, each `...` looked up
/// when reached and expanded, the values gathered in an uncounted pairlist
/// as GNU R's `CONS_NR` list, each holding a link until all are evaluated
/// (`INCREMENT_LINKS`, then `DECREMENT_LINKS` of all), then the builtin
/// `fun` called with them as a window, `R_Visible` as `PRIMPRINT` says, a
/// safepoint on return.
// R: eval.c:evalList, eval.c:eval (BUILTINSXP)
pub fn builtin_evallist(cx: &mut Cx, callr: Root<'_>, funr: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    cx.scope(|cx, s| {
        // the values so far (head, tail), and the cursor over the call's
        // arguments
        let head = s.out(cx).root();
        let tail = s.out(cx).root();
        // SAFETY: the rest of the rooted call.
        let el = s.root(cx, unsafe { Ref::from_sexp(callr.bind(cx).sexp().cdr(), cx) });
        let mut n = 0;
        let mut count = 0usize;
        loop {
            let e = el.bind(cx);
            let Some(cell) = e.as_cons() else { break };
            n += 1;
            let a = cell.car().sexp();
            if a == dots {
                // R_findVar(R_DotsSymbol, rho), then each element in turn
                cx.scope(|cx, s2| {
                    // SAFETY: an immortal symbol.
                    let d = s2.root(cx, unsafe { Ref::from_sexp(dots, cx) });
                    let h = s2.out(cx);
                    let hr = h.root();
                    cx.find_var(d, rho, h)?;
                    let hv = hr.bind(cx);
                    if !(hv.sexptype() == DOTSXP || hv.sexptype() == NILSXP) {
                        if hv.sexp() != missing {
                            return Err(cx.error("'...' used in an incorrect context"));
                        }
                        return Ok(());
                    }
                    while let Some(c) = hr.bind(cx).as_cons() {
                        let (v, t) = (s2.root(cx, c.car()), c.tag().sexp());
                        let o = s2.out(cx);
                        let or = o.root();
                        crate::eval::eval(cx, v, rho, o)?;
                        // SAFETY: a rooted value.
                        unsafe { crate::links::increment_links(or.bind(cx).sexp()) };
                        append(cx, head, tail, or, t)?;
                        count += 1;
                        // SAFETY: the rest of a rooted list (`reset` does
                        // not collect).
                        let next = unsafe { hr.bind(cx).sexp().cdr() };
                        let o = hr.reset(cx);
                        // SAFETY: as above.
                        o.set(cx, unsafe { Ref::from_sexp(next, cx) });
                    }
                    Ok(())
                })?;
            } else if a == missing {
                let msg = format!("argument {n} is empty");
                return Err(cx.error_call(Some(callr), &msg));
            } else {
                let t = cell.tag().sexp();
                cx.scope(|cx, s2| {
                    // SAFETY: a part of the rooted call.
                    let ar = s2.root(cx, unsafe { Ref::from_sexp(a, cx) });
                    let o = s2.out(cx);
                    let or = o.root();
                    crate::eval::eval(cx, ar, rho, o)?;
                    // SAFETY: a rooted value.
                    unsafe { crate::links::increment_links(or.bind(cx).sexp()) };
                    append(cx, head, tail, or, t)
                })?;
                count += 1;
            }
            // SAFETY: the rest of the rooted call (`reset` does not collect).
            let next = unsafe { el.bind(cx).sexp().cdr() };
            let o = el.reset(cx);
            // SAFETY: as above.
            o.set(cx, unsafe { Ref::from_sexp(next, cx) });
        }
        // the window: values, then names; the links dropped
        let values = s.frame(cx, count);
        let names = s.frame(cx, count);
        let mut any_alt = false;
        let mut c = head.bind(cx);
        for i in 0..count {
            let cell = c.as_cons().expect("robigo: evalList's list ended early");
            // SAFETY: the list's values and tags are live, and the slots are
            // this scope's; each value's link was raised above.
            unsafe {
                values.add(i).write(Slot::object(cell.car().sexp()));
                names.add(i).write(Slot::object(cell.tag().sexp()));
                crate::links::decrement_links(cell.car().sexp());
            }
            any_alt |= cell.car().is_package_altrep();
            c = cell.cdr();
        }
        // SAFETY: `count` object slots and `count` name slots of this scope.
        let args = unsafe { Args::from_raw(values, Some(names), count as u32, any_alt) };
        invoke_builtin_root(cx, funr, callr, args, rho, out)
    })
}

/// Call the builtin `fun` with `args` (rooted) for `call`, the value into
/// `out`, `R_Visible` as `PRIMPRINT` says; a safepoint on return.
fn invoke_builtin_root(
    cx: &mut Cx,
    fun: Root<'_>,
    call: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // SAFETY: a rooted primitive of this isolate.
    let def = *unsafe { prims::def(cx, fun.bind(cx).sexp()) };
    let Kind::Builtin(f, _) = def.kind else {
        unreachable!("robigo: a special called as a builtin");
    };
    let flag = def.print();
    // R: eval.c:eval, BUILTINSXP
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    f(cx, call, args, rho, out)?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    // a builtin's return is a safepoint (docs/g3.md §3 item 2)
    cx.safepoint(Safepoint::Return)
}

/// Append `CONS_NR(value, R_NilValue)` with tag `tag` to the list
/// `head`..`tail` (both roots; empty while `head` is): a cell that counts
/// no references, as evalList's.
fn append(cx: &mut Cx, head: Root<'_>, tail: Root<'_>, value: Root<'_>, tag: Sexp) -> RResult<()> {
    let cell = alloc_retry(cx, |cx| {
        let nil = cx.nil().sexp();
        // SAFETY: a rooted value and an immortal tag; nothing collects.
        unsafe {
            let v = value.bind(cx).sexp();
            let c = cx.alloc_node_raw(LISTSXP, v, nil, tag)?;
            // CONS_NR: the construction's counts undone, and none later
            v.decref();
            if tag != nil {
                tag.decref();
            }
            c.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
            Ok(c)
        }
    })?;
    // (nothing below collects: `cell` stays valid)
    if head.bind(cx).sexp() == cx.nil().sexp() {
        // (an empty root binds as R_NilValue)
        let o = head.reset(cx);
        // SAFETY: fresh, of this isolate.
        o.set(cx, unsafe { Ref::from_sexp(cell, cx) });
    } else {
        // SAFETY: the last cell of the rooted list, which counts nothing.
        unsafe { tail.bind(cx).sexp().set_cdr(cell) };
    }
    let o = tail.reset(cx);
    // SAFETY: as above.
    o.set(cx, unsafe { Ref::from_sexp(cell, cx) });
    Ok(())
}

/// A special reached through a value (`(if)(TRUE, 1)`, `f <- quote;
/// f(x)`, `` `[`(x, 1) ``): applied to its call in the activation's
/// environment.
pub(crate) fn call_special(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code, and a register's function.
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
        act.clear(dst);
        crate::apply::apply_special(cx, callr, funr, act.root(R_ENV), act.out(dst))
    })
}

/// `matchArgs_NR(formals, CDR(call), call)` for a special's formals (no
/// `...`): each formal's argument expression, `R_MissingArg` when none
/// matched; GNU R's errors against the call.
// R: match.c:matchArgs_NR
pub fn match_builtin_args<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, formals: Root<'_>) -> RResult<Vec<Root<'s>>> {
    let r = {
        // SAFETY: rooted inputs; nothing collects in this interval.
        unsafe { match_args(cx, formals.bind(cx).sexp(), call.bind(cx).sexp().cdr()) }
    };
    match r {
        Ok(m) => {
            let n = m.vals.len();
            // SAFETY: the values are parts of the rooted call (no `...`
            // list is made: the formals have none).
            Ok((0..n).map(|i| s.root(cx, unsafe { Ref::from_sexp(m.vals[i], cx) })).collect())
        }
        Err(MatchFail::Gc(e)) => {
            cx.need_gc(e)?;
            match_builtin_args(cx, s, call, formals)
        }
        Err(MatchFail::Error(m)) => {
            let msg = m.message(cx);
            Err(cx.error_call(Some(call), &msg))
        }
    }
}

/// A loop's region in its `CTXT_LOOP` record (`do_for`, `do_while`,
/// `do_repeat`): run from `start` to the region's end; a `break` jump
/// landing here ends the loop, a `next` restarts the region (a back-edge:
/// its safepoint, and bcEval's poll in the `Bc` profile). Either way the
/// region's temporaries, registers `regs..nregs`, are dead (D9).
pub fn loop_region(cx: &mut Cx, act: &mut Act, start: usize, regs: Reg, nregs: Reg, profile: Profile) -> RResult<Flow> {
    loop {
        let r = cx.scope(|cx, s| {
            let nil = s.root(cx, cx.nil());
            let base = s.root(cx, cx.base_env());
            // R: eval.c:do_for, begincontext(&cntxt, CTXT_LOOP, R_NilValue,
            // rho, R_BaseEnv, R_NilValue, R_NilValue)
            let env = act.root(R_ENV);
            cx.with_context(ctxt::LOOP, nil, env, base, None, |cx, _id| vm::run(cx, act, start))
        });
        let landed = match r {
            Ok(f) => return Ok(f),
            Err(Left::Exit(e)) => return Err(e),
            Err(Left::Landed(info)) => info,
        };
        for r in regs..nregs {
            act.clear(r);
        }
        if landed.mask != ctxt::NEXT {
            return Ok(Flow::RegionEnd);
        }
        cx.safepoint(Safepoint::Loop)?;
        if profile == Profile::Bc {
            crate::eval::bc_tick(cx)?;
        }
    }
}

/// A small vector on the stack (the common case: a call's arguments and a
/// closure's formals), spilling to the heap past [`INLINE`] elements: the
/// argument matcher runs on every call and must not allocate.
const INLINE: usize = 8;

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
