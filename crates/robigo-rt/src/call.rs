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
use crate::vm::{self, Act, Flow};
use robigo_heap::Safepoint;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Args, Cx, Left, Out, RResult, Ref, Root, Slot, ctxt};

/// `findFun3(sym, rho, call)` into register `a`: the lookup that may run
/// code (a promise forced) and GNU R's errors.
///
/// # Safety
/// `sym` is a live symbol and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
pub unsafe fn find_fun(cx: &mut Cx, act: &Act, a: Reg, sym: Sexp, call: Sexp) -> RResult<()> {
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
/// (`eval`'s LANGSXP case after the function is found, or bcEval's `CALL`
/// when `bc`); a closure's arguments are bound with call site `site`'s
/// match plan.
///
/// # Safety
/// `call` is a live call of `cx`'s isolate (a constant of `act`'s code, or
/// rooted).
// R: eval.c:eval, eval.c:bcEval CALL
pub unsafe fn call(cx: &mut Cx, act: &Act, dst: Reg, rf: Reg, call: Sexp, site: u16, bc: bool) -> RResult<()> {
    let fun = act.obj(rf);
    // SAFETY: a register's object is live.
    match unsafe { fun.typ() } {
        CLOSXP => {
            // a pure closure called from byte code: its body run inline when
            // nothing the call would make can be observed (crate::inline),
            // its byte code or, for a closure GNU R does not compile, its
            // `Eval` code
            if bc
                // SAFETY: a register's live closure.
                && unsafe { fun.extra() } & robigo_heap::extra::closure::NO_INLINE == 0
                // SAFETY: the call is a constant of the caller's rooted code.
                && unsafe { crate::inline::site(cx, act, dst, fun, call, site) }?
            {
                return Ok(());
            }
            // SAFETY: the caller's contract: `call` is live.
            unsafe { args::call_closure(cx, act, dst, rf, call, site) }
        }
        // SAFETY: `fun` is register `rf`'s object, live while the activation
        // runs; `call` is live (the caller's contract).
        BUILTINSXP if bc => unsafe { call_builtin_list(cx, act, dst, fun, call, bc) },
        // SAFETY: `fun` is register `rf`'s object, live while the activation
        // runs; `call` is live (the caller's contract).
        SPECIALSXP if bc => unsafe { call_special(cx, act, dst, fun, call) },
        BUILTINSXP | SPECIALSXP => {
            // eval's check_stack_balance (bcEval's CALL checks nothing):
            // `save` after PROTECT(op), the primitive's entry one more
            // protection up
            // SAFETY: a live primitive of this isolate.
            let name = unsafe { prims::def(cx, fun) }.name;
            let save = cx.pp_save(-1);
            // SAFETY: as above.
            if unsafe { fun.typ() } == BUILTINSXP {
                // SAFETY: `fun` is register `rf`'s object, live while the activation
                // runs; `call` is live (the caller's contract).
                unsafe { call_builtin_list(cx, act, dst, fun, call, bc) }?;
            } else {
                // SAFETY: `fun` is register `rf`'s object, live while the activation
                // runs; `call` is live (the caller's contract).
                unsafe { call_special(cx, act, dst, fun, call) }?;
            }
            cx.check_stack_balance(name.as_bytes(), save);
            Ok(())
        }
        // byte code's function is a symbol's (GETFUN finds only functions)
        // or a value CHECKFUN checks: its error() is reported against the
        // expression bcEval is at, the call
        // R: eval.c:bcEval CHECKFUN
        _ if bc => {
            // SAFETY: the caller's contract: a live call.
            unsafe { cx.set_bc_expression(call) };
            let e = cx.error("attempt to apply non-function");
            // SAFETY: R_NilValue.
            unsafe { cx.set_bc_expression(cx.nil().sexp()) };
            Err(e)
        }
        _ => Err(cx.error("attempt to apply non-function")),
    }
}

/// [`call`] by compiled code of a translated call whose promises hold
/// their own code (`MAKEPROM`'s BCODESXPs, `codes`, by argument:
/// [`args::arg_code`]): a closure's promises are made of them (not run
/// inline yet), a builtin's arguments are their values (`MAKEPROM`'s
/// `bcEval(code, rho)`), a special gets the call as it is. Its own
/// functions all through, so that [`call`] and what the VM's `Call`
/// inlines of it stay as they are. `codes_k`: the codes' pool index in
/// `act`'s code (read from it where used: a collection may move them).
///
/// # Safety
/// As for [`call`]; `codes_k` is a pool index of `act`'s code.
// R: eval.c:bcEval CALL, MAKEPROM
#[inline(never)]
pub unsafe fn call_codes(
    cx: &mut Cx,
    act: &Act,
    dst: Reg,
    rf: Reg,
    call: Sexp,
    codes_k: u16,
    site: u16,
) -> RResult<()> {
    let fun = act.obj(rf);
    // SAFETY: a register's object is live; the caller's contract.
    unsafe {
        match fun.typ() {
            CLOSXP => args::call_closure_codes(cx, act, dst, rf, call, codes_k, site),
            BUILTINSXP => {
                // (rooted at once by the callee: nothing collects before)
                let codes = crate::code::View::of(act.obj(crate::ir::R_CODE)).k(codes_k);
                call_builtin_list_codes(cx, act, dst, fun, call, codes)
            }
            SPECIALSXP => call_special(cx, act, dst, fun, call),
            // R: eval.c:bcEval CHECKFUN (as in `call`)
            _ => {
                cx.set_bc_expression(call);
                let e = cx.error("attempt to apply non-function");
                cx.set_bc_expression(cx.nil().sexp());
                Err(e)
            }
        }
    }
}

/// GNU R's `applyClosure(call, op, arglist, rho, suppliedvars, TRUE)` for a
/// promise list `arglist` (made by C, by `R_forceAndCall`, by the `apply`
/// hook): matched and bound by `args::bind`, the body run in a function
/// context, then `R_CleanupEnvir` and `unpromiseArgs`. `supplied` is
/// usemethod's extra variables (`createS3Vars`'s, or None).
// R: eval.c:applyClosure
pub fn apply_closure(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    arglist: Root<'_>,
    rho: Root<'_>,
    supplied: Option<&crate::dispatch::S3Vars<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    args::apply(cx, call, op, Source::List, Some(arglist), rho, None, supplied, true, out)
}

/// `applyClosure(call, op, arglist, rho, suppliedvars, FALSE)`: as
/// [`apply_closure`], the promises of `arglist` left as they are (a
/// method dispatched to shares the generic's).
// R: eval.c:applyClosure
pub fn apply_closure_keep(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    arglist: Root<'_>,
    rho: Root<'_>,
    supplied: Option<&crate::dispatch::S3Vars<'_>>,
    out: Out<'_>,
) -> RResult<()> {
    args::apply(cx, call, op, Source::List, Some(arglist), rho, None, supplied, false, out)
}

/// `applyClosure(call, op, cptr->promargs, rho, suppliedvars, FALSE)` for
/// S3 dispatch's method, `cptr` the generic's record `generic`: as
/// [`apply_closure_keep`] of the generic's `promargs`, its arguments read
/// from the generic's record and frame without making the list where they
/// can be (`args::Source::Generic`).
// R: objects.c:dispatchMethod, applyMethod
pub fn apply_closure_generic(
    cx: &mut Cx,
    call: Root<'_>,
    op: Root<'_>,
    generic: robigo_rtcore::ContextId,
    rho: Root<'_>,
    supplied: &crate::dispatch::S3Vars<'_>,
    out: Out<'_>,
) -> RResult<()> {
    args::apply(cx, call, op, Source::Generic(generic), None, rho, None, Some(supplied), false, out)
}

/// The code object a call of closure `op` runs, after GNU R's JIT
/// decision for the call ([`crate::jit::closure_profile`]): its 4th word,
/// compiled on first use for its formals (whose shape the frame takes).
///
/// - `Eval`: closures made by one `function` expression share its body and
///   formals, so the code object is also cached on the body when it is a
///   call.
/// - `Bc`: the compiled code GNU R would hold as the closure's body, with
///   cmp.R's decisions for the closure's environment, taken when it was
///   compiled (by the JIT, by the code that made it); an installed closure
///   is compiled at its first call. Never shared through the body: the
///   decisions depend on the environment.
#[inline(always)]
pub fn closure_code(cx: &mut Cx, op: Root<'_>) -> RResult<Sexp> {
    // the common case: a compiled closure (no forced profile, byte code
    // enabled) whose `Bc` code for its formals is attached, as
    // `closure_profile` and the check below would find it
    let f = op.bind(cx).sexp();
    let st = prims::state(cx);
    // SAFETY: a rooted closure: its 4th word is R_NilValue or a code object.
    unsafe {
        if st.forced.is_none()
            && !st.jit.get().disable_bytecode
            && crate::jit::compiled(f)
            && crate::code::is_closure_code(cx, f.aux(), Profile::Bc, f.car())
        {
            return Ok(f.aux());
        }
    }
    closure_code_slow(cx, op)
}

/// [`closure_code`] past its common case.
#[inline(never)]
fn closure_code_slow(cx: &mut Cx, op: Root<'_>) -> RResult<Sexp> {
    let profile = crate::jit::closure_profile(cx, op)?;
    let f = op.bind(cx).sexp();
    // SAFETY: a rooted closure: its 4th word is R_NilValue or a code object
    // this module stored; its body's 4th word too, if it is a call.
    unsafe {
        let (formals, body) = (f.car(), f.cdr());
        let c = f.aux();
        if crate::code::is_closure_code(cx, c, profile, formals) {
            return Ok(c);
        }
        if profile == Profile::Eval
            && body.typ() == LANGSXP
            && let Some(c) = crate::code::cached(cx, body.aux(), profile)
            && crate::code::is_closure_code(cx, c, profile, formals)
        {
            f.set_aux(c);
            return Ok(c);
        }
    }
    if profile == Profile::Bc {
        // byte code a write into formals made stale: GNU R keeps its byte
        // code (a write into the body expression does not reach it) and
        // reads the formals at the call, so the instructions stay and the
        // frame shape and match plans are made again (docs/g3.md §9.3)
        // SAFETY: a rooted closure: its 4th word is R_NilValue or a code
        // object.
        if unsafe { crate::code::formals_stale_bc(cx, op.bind(cx).sexp().aux(), op.bind(cx).sexp().car()) } {
            let code = cx.retry_alloc(|cx| {
                // SAFETY: the rooted closure's code object and formals;
                // nothing collects in this interval.
                unsafe {
                    let f = op.bind(cx).sexp();
                    crate::code::reshape(cx, f.aux(), f.car())
                }
            })?;
            // SAFETY: a rooted closure and its code: a traced word.
            unsafe { crate::jit::set_compiled(op.bind(cx).sexp(), code) };
            return Ok(code);
        }
        // (a JIT-compiled closure whose code was not kept, or a forced
        // profile: an installed closure's code is made with it)
        if let Some(code) = compile::compile_closure_bc(cx, op, false)? {
            let f = op.bind(cx).sexp();
            // SAFETY: a rooted closure and its code: a traced word. A
            // BCODESXP body is byte code GNU R holds: marked so, the next
            // call takes the common case
            unsafe {
                if crate::jit::bytecode_body(op.bind(cx)) {
                    crate::jit::set_compiled(f, code);
                } else {
                    f.set_aux(code);
                }
            }
            return Ok(code);
        }
        // cmp.R leaves it uncompiled (an installed closure never is: that
        // would be a gap of the port): it runs as GNU R's AST would
        let f = op.bind(cx).sexp();
        // SAFETY: a rooted closure.
        unsafe {
            crate::jit::decompile(cx, f);
            f.set_gp(f.gp() | crate::jit::NOJIT_MASK);
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
            crate::code::cache_on(body, code);
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
            run_builtin(cx, &def, call, args, env, out)
        }
        // R_forceAndCall: a special takes the call's own arguments, under
        // its PROTECT(fun) and PROTECT(e)
        SPECIALSXP => cx.with_pp_excess(2, |cx| crate::apply::apply_special(cx, call, fun, env, out)),
        _ => Err(cx.error("attempt to apply non-function")),
    }
}

// ---- builtins -----------------------------------------------------------------

/// A builtin `fun` (or a builtin `.Internal`) called with the `n` evaluated
/// arguments in registers `base..` (their names after them if `names`),
/// for `call`: GNU R's `eval` BUILTINSXP case after `evalList`.
///
/// # Safety
/// `fun` is a live BUILTINSXP and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
pub unsafe fn call_builtin_regs(
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
        let p = act.window(base, if names { 2 * n } else { n });
        Args::from_raw(p, names.then(|| p.add(n)), n as u32, any_alt)
    };
    // the arguments hold no links while the builtin runs
    // R: eval.c:BUILTIN_CALL_FRAME_ARGS, eval.c:evalList
    let r = invoke_builtin(cx, act, dst, args, (base, if names { 2 * n } else { n }));
    for r in base..base + if names { 2 * n } else { n } as Reg {
        act.clear(r);
    }
    r
}

/// [`call_builtin_regs`] for a builtin function that is no primitive of
/// the table (`log`'s evaluated form, `do_log_builtin`: bcEval's `LOG` and
/// `LOGBASE` slow path), its value visible.
///
/// # Safety
/// As for [`call_builtin_regs`].
pub unsafe fn call_fn_regs(
    cx: &mut Cx,
    act: &Act,
    dst: Reg,
    f: crate::prims::BuiltinFn,
    call: Sexp,
    base: Reg,
    n: usize,
) -> RResult<()> {
    // SAFETY: a constant of the rooted code.
    unsafe { act.scratch(R_SCRATCH, call) };
    let mut any_alt = false;
    for r in base..base + n as Reg {
        let x = vm::boxed(cx, act, r)?;
        // SAFETY: a register's object is live.
        any_alt |= unsafe { x.is_altrep() };
    }
    // SAFETY: `n` object slots from `base`, rooted by the activation.
    let args = unsafe { Args::from_raw(act.slot(base), None, n as u32, any_alt) };
    // an argument that is one of robigo's compact sequences expanded, as
    // a builtin's ([`run_builtin`]: GNU R reads it through `REAL()`)
    if any_alt {
        cx.materialize_args(args)?;
    }
    // SAFETY: this running activation's registers.
    unsafe { crate::links::skip_args(act.base(), base, n) };
    cx.note_effect();
    cx.set_visible(true);
    let r = f(cx, act.root(R_SCRATCH), args, act.root(R_ENV), act.out(dst)).and_then(|()| {
        cx.set_visible(true);
        cx.safepoint(Safepoint::Return)
    });
    // SAFETY: as above.
    unsafe { crate::links::unskip_args(act.base()) };
    act.clear(R_SCRATCH);
    for r in base..base + n as Reg {
        act.clear(r);
    }
    r
}

/// Call the builtin in the scratch register `R_SCRATCH2` with `args`
/// (rooted) for the call in `R_SCRATCH`, the value into register `dst`,
/// `R_Visible` as `PRIMPRINT` says; a safepoint on return. Clears the
/// scratch registers.
fn invoke_builtin(cx: &mut Cx, act: &Act, dst: Reg, args: Args<'_>, regs: (Reg, usize)) -> RResult<()> {
    let fun = act.obj(R_SCRATCH2);
    // SAFETY: a live primitive of this isolate.
    let def = *unsafe { prims::def(cx, fun) };
    // (the scratch registers hold a code constant and a primitive, which
    // live on elsewhere: clearing them after the return's safepoint delays
    // no object's death)
    // the arguments in registers `regs` (none for a list) hold no links
    // while the builtin runs
    // R: eval.c:BUILTIN_CALL_FRAME_ARGS, eval.c:evalList
    // SAFETY: this running activation's registers.
    unsafe { crate::links::skip_args(act.base(), regs.0, regs.1) };
    let r = run_builtin(cx, &def, act.root(R_SCRATCH), args, act.root(R_ENV), act.out(dst));
    // SAFETY: as above.
    unsafe { crate::links::unskip_args(act.base()) };
    act.clear(R_SCRATCH);
    act.clear(R_SCRATCH2);
    r
}

/// A special `.Internal` `fun` (`lapply`, `vapply`) called for its inner
/// `call`: the call's argument expressions (and their tags) as the window,
/// unevaluated, with the activation's environment. GNU R's `do_internal`
/// for a SPECIALSXP: `R_Visible` on, then `PRIMFUN(INTERNAL(fun))(s, ...,
/// args, env)`.
///
/// # Safety
/// `fun` is a live SPECIALSXP and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
// R: names.c:do_internal
pub unsafe fn call_special_internal(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    // SAFETY: a live primitive of this isolate.
    let name = unsafe { prims::def(cx, fun) }.name;
    let save = cx.pp_save(0);
    // the internal runs under do_internal's PROTECT(args)
    cx.with_pp_excess(1, |cx| call_special_args(cx, act, dst, fun, call, true))?;
    cx.check_stack_balance(name.as_bytes(), save);
    Ok(())
}

/// A special of the builtin registry, primitive or `.Internal`
/// (`internal`, which sets `R_Visible` on first), called with `call`'s
/// argument expressions: GNU R's `eval` SPECIALSXP case, `PRIMFUN(op)(e,
/// op, CDR(e), rho)`.
// R: eval.c:eval (SPECIALSXP)
fn call_special_args(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp, internal: bool) -> RResult<()> {
    // SAFETY: constants of the rooted code.
    unsafe {
        act.scratch(R_SCRATCH, call);
        act.scratch(R_SCRATCH2, fun);
    }
    cx.scope(|cx, s| {
        let (vals, names) = {
            let c = act.root(R_SCRATCH).bind(cx);
            let mut vals = Vec::new();
            let mut names = Vec::new();
            let mut a = c.as_cons().and_then(|c| c.cdr().as_cons());
            while let Some(cell) = a {
                vals.push(cell.car());
                names.push(cell.tag().as_sym());
                a = cell.cdr().as_cons();
            }
            (vals, names)
        };
        let named = names.iter().any(Option::is_some);
        let args = s.args(cx, &vals, named.then_some(&names[..]));
        if internal {
            cx.set_visible(true);
        }
        invoke_builtin(cx, act, dst, args, (0, 0))
    })
}

/// A builtin called with its arguments evaluated at run time, for the
/// call in register-free form ([`builtin_evallist`]; `bc`: by compiled
/// code).
///
/// # Safety
/// `fun` is a live BUILTINSXP and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
pub unsafe fn call_builtin_list(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp, bc: bool) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: constants of the rooted code (or a register's function).
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
        builtin_evallist(cx, callr, funr, act.root(R_ENV), act.out(dst), bc, None)
    })
}

/// [`call_builtin_list`] by compiled code of a translated call whose
/// arguments' code is `codes` ([`args::arg_code`]).
///
/// # Safety
/// As for [`call_builtin_list`]; `codes` is a constant of `act`'s code.
#[inline(never)]
unsafe fn call_builtin_list_codes(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp, codes: Sexp) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: constants of the rooted code (or a register's function).
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
        // SAFETY: as above.
        let codes = s.root(cx, unsafe { Ref::from_sexp(codes, cx) });
        builtin_evallist(cx, callr, funr, act.root(R_ENV), act.out(dst), true, Some(codes))
    })
}

/// `evalList` over the call's arguments in order, each `...` looked up
/// when reached and expanded, the values gathered in an uncounted pairlist
/// as GNU R's `CONS_NR` list, each holding a link until all are evaluated
/// (`INCREMENT_LINKS`, then `DECREMENT_LINKS` of all), then the builtin
/// `fun` called with them as a window, `R_Visible` as `PRIMPRINT` says, a
/// safepoint on return.
///
/// With `bc`, as compiled code's generic call of a builtin evaluates them
/// (bcEval's `MAKEPROM`, `DOMISSING`, `DODOTS`, then `checkForMissings`):
/// an argument call by its compiled code (`bcEval(code, rho)`: a
/// translated call's own promise code from `codes`), an empty argument
/// R_MissingArg, reported ("argument %d is empty") only once all are
/// evaluated. `codes` is read only with `bc` (the AST's evaluation has no
/// promise codes), so a caller passes it only with `bc`.
// R: eval.c:evalList, eval.c:eval (BUILTINSXP), eval.c:bcEval MAKEPROM, checkForMissings, signalMissingArgError
pub fn builtin_evallist(
    cx: &mut Cx,
    callr: Root<'_>,
    funr: Root<'_>,
    rho: Root<'_>,
    out: Out<'_>,
    bc: bool,
    codes: Option<Root<'_>>,
) -> RResult<()> {
    debug_assert!(codes.is_none() || bc, "robigo: promise codes without the Bc profile's evaluation");
    let missing = cx.missing_arg().sexp();
    let dots = prims::state(cx).syms.dots;
    cx.scope(|cx, s| {
        // the values so far (head, tail), and the cursor over the call's
        // arguments
        let head = s.out(cx).root();
        let tail = s.out(cx).root();
        // the values' links, dropped before the call, or on an early exit
        // SAFETY: `head` is a slot of this scope, which outlives the guard.
        let linked = unsafe { crate::links::Linked::list(head) };
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
                        // under PROTECT(h), and the head's after the first
                        // value (the builtin's entry is one more than
                        // eval's PROTECT(op)); byte code protects neither
                        let pp = if bc { 0 } else { (count > 0) as i32 };
                        cx.with_pp_excess(pp, |cx| crate::eval::eval(cx, v, rho, o))?;
                        append(cx, head, tail, or, t)?;
                        // (linked once in the list, which the guard walks)
                        // SAFETY: a rooted value.
                        unsafe { crate::links::increment_links(or.bind(cx).sexp()) };
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
            } else if a == missing && !bc {
                let msg = format!("argument {n} is empty");
                return Err(cx.error_call(Some(callr), &msg));
            } else {
                let t = cell.tag().sexp();
                cx.scope(|cx, s2| {
                    // SAFETY: a part of the rooted call.
                    let ar = s2.root(cx, unsafe { Ref::from_sexp(a, cx) });
                    let o = s2.out(cx);
                    let or = o.root();
                    // SAFETY: the rooted codes (R_NilValue or a VECSXP).
                    let code = codes.map_or(Sexp::NULL, |c| unsafe { args::arg_code(c.bind(cx).sexp(), n - 1) });
                    if a == missing {
                        // DOMISSING
                        o.set(cx, cx.missing_arg());
                    } else if bc && !code.is_null() {
                        // MAKEPROM's bcEval(code, rho): the argument's own
                        // promise code
                        // SAFETY: a constant of the rooted caller's code.
                        let c = s2.root(cx, unsafe { Ref::from_sexp(code, cx) });
                        crate::eval::eval_bytecode_inline(cx, c, rho, o)?;
                    } else if bc && ar.bind(cx).sexptype() == LANGSXP {
                        crate::eval::eval_bytecode_inline(cx, ar, rho, o)?;
                    } else if bc {
                        crate::eval::eval(cx, ar, rho, o)?;
                    } else {
                        // under eval's PROTECT(op), and the head's after
                        // the first value
                        let pp = (count > 0) as i32 - 1;
                        cx.with_pp_excess(pp, |cx| crate::eval::eval(cx, ar, rho, o))?;
                    }
                    append(cx, head, tail, or, t)?;
                    // (linked once in the list, which the guard walks)
                    // SAFETY: a rooted value.
                    unsafe { crate::links::increment_links(or.bind(cx).sexp()) };
                    Ok(())
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
            // this scope's.
            unsafe {
                values.add(i).write(Slot::object(cell.car().sexp()));
                names.add(i).write(Slot::object(cell.tag().sexp()));
            }
            any_alt |= cell.car().is_altrep();
            c = cell.cdr();
        }
        linked.release();
        if bc {
            // checkForMissings: an empty argument of the call is an error
            // now (a computed R_MissingArg is not)
            let mut c = head.bind(cx);
            let mut found = false;
            while let Some(cell) = c.as_cons() {
                found |= cell.car().sexp() == missing;
                c = cell.cdr();
            }
            if found {
                let mut k = 0;
                let mut c = callr.bind(cx).as_cons().unwrap().cdr();
                while let Some(cell) = c.as_cons() {
                    k += 1;
                    if cell.car().sexp() == missing {
                        let msg = format!("argument {k} is empty");
                        return Err(cx.error_call(Some(callr), &msg));
                    }
                    c = cell.cdr();
                }
            }
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
    run_builtin(cx, &def, call, args, rho, out)
}

/// `eval`'s call of a builtin: `R_Visible` set from `PRIMPRINT` before and
/// after the call (unless the builtin decides), and the safepoint every
/// builtin's return is (docs/g3.md §3 item 2). Every builtin call of the
/// evaluator goes through here.
// R: eval.c:eval (BUILTINSXP)
#[inline(always)]
pub(crate) fn run_builtin(
    cx: &mut Cx,
    def: &prims::PrimDef,
    call: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let Kind::Builtin(f, _) = def.kind else {
        unreachable!("robigo: a special called as a builtin");
    };
    materialize_args(cx, def, args)?;
    let flag = def.print();
    // a builtin may change what an argument expression evaluates to or
    // look at an argument: values computed early are dropped (crate::early)
    cx.note_effect();
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    f(cx, call, args, rho, out)?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    cx.safepoint(Safepoint::Return)
}

/// Append `CONS_NR(value, R_NilValue)` with tag `tag` to the list
/// `head`..`tail` (both roots; empty while `head` is): a cell that counts
/// no references, as evalList's.
fn append(cx: &mut Cx, head: Root<'_>, tail: Root<'_>, value: Root<'_>, tag: Sexp) -> RResult<()> {
    let cell = cx.retry_alloc(|cx| {
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
///
/// # Safety
/// `fun` is a live SPECIALSXP and `call` a live call of `cx`'s isolate
/// (constants of `act`'s code, or rooted).
pub(crate) unsafe fn call_special(cx: &mut Cx, act: &Act, dst: Reg, fun: Sexp, call: Sexp) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code, and a register's function.
        let (callr, funr) = unsafe { (s.root(cx, Ref::from_sexp(call, cx)), s.root(cx, Ref::from_sexp(fun, cx))) };
        act.clear(dst);
        crate::apply::apply_special(cx, callr, funr, act.root(R_ENV), act.out(dst))
    })
}

/// A loop's region in its `CTXT_LOOP` record (`do_for`, `do_while`,
/// `do_repeat`): run from `start` to the region's end; a `break` jump
/// landing here ends the loop, a `next` restarts the region (a back-edge:
/// its safepoint, and bcEval's poll in the `Bc` profile). Either way the
/// region's temporaries, registers `regs..nregs`, are dead (D9).
pub fn loop_region(cx: &mut Cx, act: &mut Act, start: usize, regs: Reg, nregs: Reg, profile: Profile) -> RResult<Flow> {
    loop {
        let r = loop_context(cx, act, start);
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

/// GNU R's builtins read an argument's data through `INTEGER`, `REAL` and
/// their `_RO` forms, which expand an instance of one of robigo's ALTREP
/// classes (its `Dataptr`): so does this, for each argument that is one,
/// before the builtin runs. The builtins GNU R writes against the element
/// and region accessors instead are left to read the object as it is:
/// `length` and `seq_along` (its length), and the summaries (`sum` takes
/// the class's `Sum`; `do_summary` materializes for the rest itself).
// R: memory.c:DATAPTR, altrep.c:ALTVEC_DATAPTR_EX; summary.c:do_summary
#[inline(always)]
pub(crate) fn materialize_args(cx: &mut Cx, def: &prims::PrimDef, args: Args<'_>) -> RResult<()> {
    if args.any_altrep() { materialize_args_slow(cx, def, args) } else { Ok(()) }
}

#[cold]
#[inline(never)]
fn materialize_args_slow(cx: &mut Cx, def: &prims::PrimDef, args: Args<'_>) -> RResult<()> {
    if reads_by_element(def.name) {
        return Ok(());
    }
    cx.materialize_args(args)
}

/// The builtins that leave their ALTREP arguments unexpanded in GNU R
/// ([`materialize_args`]):
/// - those written against `XLENGTH` and the element, region and summary
///   methods: `length`, `seq_along`, the summaries;
/// - the foreign calls, which hand their arguments to C as they are (C's
///   `DATAPTR` then expands one: robigo-rtapi's `api.c`);
/// - the type and attribute queries, which read no data (coerce.c's
///   `do_is`, `do_typeof`, `do_isvector`; attrib.c's `do_attributes`,
///   `do_attr`, `do_data_class`, `do_oldclass`, `do_inherits`, `do_names`,
///   `do_dim`, `do_dimnames`), and `invisible`;
/// - those that store or compare a value without reading its data:
///   `assign`, and `identical` and the compiler's `putconst`
///   (`R_compute_identical` reads the data only of two vectors of one type
///   and length; robigo-rtbuiltins' reads it by element);
/// - `serialize`, which writes a compact sequence's state
///   (`Serialized_state`), expanded or not.
///
/// `list` (`do_makelist`) only stores its arguments, but it materializes
/// them here: the builtins that take a list of values (`attributes<-`,
/// `dimnames<-`, `.Internal(paste(list(...)))`, ...) read its elements as
/// their own arguments, and not all of them read by element yet (AC2).
// R: names.c's rows of these
fn reads_by_element(name: &str) -> bool {
    matches!(
        name,
        "length"
            | "seq_along"
            | "sum"
            | "min"
            | "max"
            | "prod"
            | ".Call"
            | ".External"
            | ".External2"
            | "typeof"
            | "is.vector"
            | "is.null"
            | "is.logical"
            | "is.integer"
            | "is.double"
            | "is.complex"
            | "is.character"
            | "is.symbol"
            | "is.name"
            | "is.environment"
            | "is.list"
            | "is.pairlist"
            | "is.expression"
            | "is.raw"
            | "is.object"
            | "isS4"
            | "is.numeric"
            | "is.matrix"
            | "is.array"
            | "is.atomic"
            | "is.recursive"
            | "is.call"
            | "is.language"
            | "is.function"
            | "attributes"
            | "attr"
            | "class"
            | "oldClass"
            | "inherits"
            | "names"
            | "dim"
            | "dimnames"
            | "invisible"
            | "assign"
            | "identical"
            | "putconst"
            | "serialize"
            | "serializeb"
            | "serializeToConn"
    )
}

/// One run of a loop's region in its `CTXT_LOOP` record. Out of line: the
/// record would otherwise sit in `vm::run`'s frame, which every R call level
/// keeps (X9).
#[inline(never)]
fn loop_context(cx: &mut Cx, act: &mut Act, start: usize) -> Result<Flow, Left> {
    cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        let base = s.root(cx, cx.base_env());
        // R: eval.c:do_for, begincontext(&cntxt, CTXT_LOOP, R_NilValue,
        // rho, R_BaseEnv, R_NilValue, R_NilValue)
        // SAFETY: the activation's environment register, live for the
        // whole activation, which outlives this call. Only `exec` writes
        // R_ENV, before `run` starts (the compiler allocates from
        // R_FIRST), so the root reads the environment throughout. It is
        // not tied to `act`'s borrow, which `run` needs mutably.
        let env = unsafe { Root::from_raw(act.slot(R_ENV)) };
        cx.with_context(ctxt::LOOP, nil, env, base, None, |cx, _id| vm::run(cx, act, start))
    })
}
