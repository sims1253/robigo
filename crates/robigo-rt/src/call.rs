//! Calls: closures (GNU R's `applyClosure`, through `args::bind`),
//! builtins, specials reached through a value, and loops' context regions.
//!
//! Argument binding is `args`' (docs/g3.md §6.2-6.4): slot frames with
//! delayed and forced slots, memoized match plans. [`apply_closure`] is
//! GNU R's `applyClosure` for a promise list (C, the `apply` hook);
//! the VM's own closure calls bind from the call site
//! ([`crate::args::call_closure`]).

#![allow(unsafe_code)]

use crate::args::{self, Source};
use crate::compile;
use crate::ir::{Profile, R_ENV, R_SCRATCH, R_SCRATCH2, Reg};
use crate::prims::{self, Kind};
use crate::vm::{self, Act, Flow, alloc_retry};
use robigo_heap::Safepoint;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Cx, Left, Out, RResult, Ref, Root, Slot, ctxt};

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
/// (`eval`'s LANGSXP case after the function is found); a closure's
/// arguments are bound with call site `site`'s match plan.
// R: eval.c:eval
pub fn call(cx: &mut Cx, act: &Act, dst: Reg, rf: Reg, call: Sexp, site: u16) -> RResult<()> {
    let fun = act.obj(rf);
    // SAFETY: a register's object is live.
    match unsafe { fun.typ() } {
        CLOSXP => args::call_closure(cx, act, dst, rf, call, site),
        BUILTINSXP => call_builtin_list(cx, act, dst, fun, call),
        SPECIALSXP => call_special(cx, act, dst, fun, call),
        _ => Err(cx.error("attempt to apply non-function")),
    }
}

/// GNU R's `applyClosure(call, op, arglist, rho, suppliedvars, TRUE)` for a
/// promise list `arglist` (made by C, by `R_forceAndCall`, by the `apply`
/// hook): matched and bound by `args::bind`, the body run in a function
/// context, then `R_CleanupEnvir` and `unpromiseArgs`. `supplied` is
/// usemethod's extra variables (S1's; must be None until then).
// R: eval.c:applyClosure
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
    args::apply(cx, call, op, Source::List, Some(arglist), rho, None, out)
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
        SPECIALSXP => Err(cx.error("robigo: applying a special to evaluated arguments is not implemented yet")),
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

/// A builtin called with its arguments evaluated at run time: `evalList`
/// over the call's arguments in order, each `...` looked up when reached
/// and expanded, the values gathered in an uncounted pairlist as GNU R's
/// `CONS_NR` list, then passed as a window.
// R: eval.c:evalList
pub fn call_builtin_list(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    cx.scope(|cx, s| {
        // SAFETY: constants of the rooted code (or a register's function).
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
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
                    cx.find_var(d, act.root(R_ENV), h)?;
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
                        crate::eval::eval(cx, v, act.root(R_ENV), o)?;
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
                    crate::eval::eval(cx, ar, act.root(R_ENV), o)?;
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
        // the window: values, then names
        let values = s.frame(cx, count);
        let names = s.frame(cx, count);
        let mut any_alt = false;
        let mut c = head.bind(cx);
        for i in 0..count {
            let cell = c.as_cons().expect("robigo: evalList's list ended early");
            // SAFETY: the list's values and tags are live, and the slots are
            // this scope's.
            unsafe {
                values.add(i).write(Slot::object(cell.car().sexp()));
                names.add(i).write(Slot::object(cell.tag().sexp()));
            }
            any_alt |= cell.car().is_package_altrep();
            c = cell.cdr();
        }
        // SAFETY: `count` object slots and `count` name slots of this scope.
        let args = unsafe { Args::from_raw(values, Some(names), count as u32, any_alt) };
        // SAFETY: the rooted call and function.
        unsafe {
            act.scratch(R_SCRATCH, callr.bind(cx).sexp());
            act.scratch(R_SCRATCH2, funr.bind(cx).sexp());
        }
        invoke_builtin(cx, act, dst, args)
    })
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
/// f(x)`): its call compiled with the special inlined, run in `rho`.
fn call_special(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    // SAFETY: a live primitive of this isolate.
    let off = unsafe { prims::offset(fun) };
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        let code = compile::compile_special(cx, callr, off)?;
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
