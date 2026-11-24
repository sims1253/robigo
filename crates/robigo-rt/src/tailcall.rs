//! eval.c's `Exec(expr, envir)` and `Tailcall(FUN, ...)` (`do_tailcall`)
//! and the loop that runs their continuations (`handle_exec_continuation`).
//!
//! A call in tail position of a closure's body, in a record with no
//! `on.exit` code, does not run inside that closure. `do_tailcall` jumps
//! to the closure's function record with the call, its environment and its
//! function: an [`ExitKind::Exec`] exit. It lands there as a `return`
//! would: the jump's legs run as for any jump to that record. Since the
//! record is the innermost one (`R_GlobalContext`, the `jump_OK` test),
//! there are no records in between: the jump leaves only the body's
//! evaluation (the VM's registers, its scope's slots), and the landing
//! restores the record's saved state (the protect stack's top among it, as
//! `R_jumpctxt` does). The record then ends as for a `return`, and the exit
//! is pending again, with no target (robigo-rtcore's `land_exec`). The
//! closure call that made the record ([`crate::args::apply`], every closure
//! call's frame) finishes as `applyClosure_core` does and calls
//! [`landed`], which runs the continuation in place of the call's value:
//! a loop, so a chain of tail calls takes no stack.
//!
//! The continuation's own call is made by [`crate::args::apply_exec`],
//! which hands a continuation landing in its record back to this loop
//! instead of starting another. A normal return stays on `apply`'s path:
//! the exit only ever reaches its error branch.

use robigo_rtcore::types::*;
use robigo_rtcore::{Cx, ExitKind, Out, RError, RResult, Ref, Root, ctxt};

use crate::prims;

/// The continuation's list: `do_tailcall`'s `val` without the token, which
/// the exit's kind stands for.
const EXPR: usize = 0;
const ENV: usize = 1;
const FUN: usize = 2;

/// `checkTailPosition(call, code, rho)`: `call` is `code`, or the last
/// expression of a `{`, or either branch of an `if`, recursively. The
/// test is by identity: the call is the cell of the body itself, in the
/// AST interpreter and in compiled code alike, because the compiler's
/// constant pool holds the body's own call cells, not copies (`compile.rs`'s
/// `k`), and `code` is `BODY_EXPR`, the body's AST (a byte-code body's
/// source: its constant 0, whose cells cmp.R's constants share too). Where
/// GNU R's pool gives a call the first `identical()` constant
/// (`do_putconst`), so does robigo-rt's under `Bc` (compile.rs's `kv`
/// for a call's constant): the same call text twice in a compiled body is
/// in tail position only at its first appearance there.
// R: eval.c:checkTailPosition
fn check_tail_position(cx: &Cx, call: Ref<'_>, code: Ref<'_>) -> bool {
    let st = prims::state(cx);
    let (brace, if_) = (st.syms.brace, st.syms.if_);
    let mut code = code;
    loop {
        if call.sexp() == code.sexp() {
            return true;
        }
        let Some(c) = code.as_cons().filter(|_| code.sexptype() == LANGSXP) else { return false };
        if c.car().sexp() == brace {
            let mut x = c;
            while let Some(n) = x.cdr().as_cons() {
                x = n;
            }
            code = x.car();
        } else if c.car().sexp() == if_ {
            let rest = c.cdr();
            let Some(cond) = rest.as_cons() else { return false };
            let Some(yes) = cond.cdr().as_cons() else { return false };
            if check_tail_position(cx, call, yes.car()) {
                return true;
            }
            match yes.cdr().as_cons() {
                Some(no) => code = no.car(),
                None => return false,
            }
        } else {
            return false;
        }
    }
}

/// `Exec(expr, envir)` (`PRIMVAL(op) == 0`) and `Tailcall(FUN, ...)`
/// (1): the call `expr`, or `FUN(...)`, made in place of the calling
/// closure's own return when the call is in its tail position and its
/// record has no `on.exit` code; otherwise evaluated where it stands.
// R: eval.c:do_tailcall
pub fn do_tailcall<const TAILCALL: bool>(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: run again on the call over it)
    if args.is_some() {
        return crate::specials::rerun_apart(cx, call, _op, args, rho, out, do_tailcall::<TAILCALL>);
    }
    cx.scope(|cx, s| {
        let missing = cx.missing_arg().sexp();
        let (expr, env) = if !TAILCALL {
            // exec
            // (installed here: a rare path, kept out of the shared symbols)
            let formals = [cx.install_rooted(s, b"expr")?, cx.install_rooted(s, b"envir")?];
            let formals = formals.map(|f| f.bind(cx).sexp());
            let matched = crate::args::match_builtin_args(cx, s, call, &formals)?;
            // evalListKeepMissing(args, rho) of the matched list (expr,
            // envir): each value linked until both are evaluated, `...`
            // expanded
            let margs = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let mut l = cx.nil();
                    for m in matched.iter().rev() {
                        l = cx.cons(m.bind(cx), l)?;
                    }
                    cx.lcons(cx.nil(), l)
                })
            })?;
            let ev = crate::specials::eval_call_args_keep_missing(cx, s, margs, rho)?;
            // CAR(args), CADR(args): R_NilValue past the list's end
            let nil = s.root(cx, cx.nil());
            let vals = [0, 1].map(|i| if i < ev.len() { ev.root(i) } else { nil });
            let mut expr = vals[0];
            if expr.bind(cx).sexp() == missing {
                let lc = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
                return Err(cx.missing_arg_error("expr", Some(lc), "tailcallError"));
            }
            let x = expr.bind(cx);
            if x.sexptype() == EXPRSXP && x.len() == Some(1) {
                let e0 = x.as_vector::<Ref<'_>>().expect("robigo: an expression vector").get(0);
                expr = s.root(cx, e0);
            }
            if expr.bind(cx).sexptype() != LANGSXP {
                return Err(cx.error("\"expr\" must be a call expression"));
            }
            let env = if vals[1].bind(cx).sexp() == missing { rho } else { vals[1] };
            (expr, env)
        } else {
            // tailcall: (could do argument matching here)
            let args = call.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr());
            let first = args.as_cons().map(|c| c.car().sexp());
            if first.is_none_or(|f| f == missing) {
                let lc = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
                return Err(cx.missing_arg_error("FUN", Some(lc), "tailcallRecError"));
            }
            let args = s.root(cx, args);
            let expr = s.capture(cx, |cx, o| {
                cx.build(o, |cx| {
                    let a = args.bind(cx).as_cons().expect("robigo: checked: an argument");
                    cx.lcons(a.car(), a.cdr())
                })
            })?;
            (expr, rho)
        };
        // A jump should only be used if there are no on.exit expressions
        // and the call is in tail position.
        let jump_ok = {
            let g = cx.global_context();
            let f = g.callfun();
            g.conexit() == cx.nil()
                && g.callflag() & ctxt::FUNCTION != 0
                && g.cloenv().sexp() == rho.bind(cx).sexp()
                && f.sexptype() == CLOSXP
                && match f.view() {
                    // BODY_EXPR(R_GlobalContext->callfun)
                    robigo_rtcore::View::Closure(c) => {
                        check_tail_position(cx, call.bind(cx), robigo_rtbuiltins::bytecode::expr(cx, c.body()))
                    }
                    _ => false,
                }
        };
        if !jump_ok {
            // (maybe have an optional diagnostic about why no tail call?)
            return crate::eval(cx, expr, rho, out);
        }
        // computing the function before the jump allows the idiom
        // Tailcall(sys.function(), ...) to be used
        let head = s.root(cx, expr.bind(cx).as_cons().expect("robigo: checked: a call").car());
        let fun = s.out(cx).root();
        let o = fun.reset(cx);
        let h = head.bind(cx);
        if h.sexptype() == STRSXP && h.len() == Some(1) {
            let sym = robigo_rtbuiltins::text::install_tr_char(cx, s, head, 0)?;
            cx.find_fun3(sym, env, Some(call), o)?;
        } else if h.sexptype() == SYMSXP {
            // (might need to adjust the call here as in eval())
            cx.find_fun3(head, env, Some(call), o)?;
        } else {
            crate::eval(cx, head, env, o)?;
        }
        let val = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<Ref<'_>>(3)?;
                v.set(EXPR, expr.bind(cx));
                v.set(ENV, env.bind(cx));
                v.set(FUN, fun.bind(cx));
                Ok(v.finish().into())
            })
        })?;
        let target = cx.global_context().id();
        Err(cx.exec_to(target, val))
    })
}

/// An `Exec`/`Tailcall` that landed in a closure call's record: the rest
/// of `applyClosure_core` after `R_execClosure` returned the continuation
/// (`R_CleanupEnvir(newrho, val)`, `unpromiseArgs(arglist)` when the call
/// owns `arglist`), then `handle_exec_continuation`. For
/// [`crate::args::apply_exec`]'s call (`EXEC`) the exit is pending again
/// for the loop that made it; otherwise the loop runs here, its value
/// written to `res`, the call's out slot (boxed).
// R: eval.c:applyClosure_core, applyClosure, handle_exec_continuation
#[cold]
#[inline(never)]
pub(crate) fn landed<const EXEC: bool>(
    cx: &mut Cx,
    e: RError,
    newrho: Root<'_>,
    unpromise: Option<Root<'_>>,
    res: Root<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        // the continuation, one slot for the whole loop
        let o = s.out(cx);
        let val = o.root();
        cx.resolve_exit(e, Some(o));
        cx.cleanup_envir(newrho, Some(val));
        cx.end_call(newrho, true);
        if let Some(l) = unpromise {
            cx.unpromise_args(l);
        }
        if EXEC {
            return Err(cx.exit(robigo_rtcore::Exit {
                info: robigo_rtcore::ExitInfo { kind: ExitKind::Exec, mask: 0, target: 0 },
                value: Some(val),
                condition: None,
                call: None,
            }));
        }
        // each step in its own scope: a chain of tail calls takes no stack
        while !cx.scope(|cx, s| step(cx, s, val, res))? {}
        Ok(())
    })
}

/// One continuation of [`landed`]'s loop, `val`'s: its value in `res` and
/// true, or false with the next continuation in `val`.
// R: eval.c:handle_exec_continuation
fn step(cx: &mut Cx, s: robigo_rtcore::Scope<'_>, val: Root<'_>, res: Root<'_>) -> RResult<bool> {
    let (call, rho, op) = {
        let v = val.bind(cx).as_vector::<Ref<'_>>().expect("robigo: a continuation");
        (v.get(EXPR), v.get(ENV), v.get(FUN))
    };
    let (call, rho, op) = (s.root(cx, call), s.root(cx, rho), s.root(cx, op));
    // SET_VECTOR_ELT(val, 2, R_NilValue): to drop REFCNT
    let nil = s.root(cx, cx.nil());
    cx.owned_list(val)?.set(ENV, nil);
    if op.bind(cx).sexptype() != CLOSXP {
        // Ideally this should handle BUILTINSXP/SPECIALSXP calls in the
        // standard way as in eval() or bceval(). For now, just build a new
        // call and eval.
        let expr = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let rest = call.bind(cx).as_cons().map_or(cx.nil(), |c| c.cdr());
                cx.lcons(op.bind(cx), rest)
            })
        })?;
        let o = res.reset(cx);
        crate::eval(cx, expr, rho, o)?;
        return Ok(true);
    }
    let arglist = s.out(cx).root();
    let o = arglist.reset(cx);
    crate::dispatch::promise_call_args(cx, call, rho, o)?;
    let o = res.reset(cx);
    match crate::args::apply_exec(cx, call, op, arglist, rho, o) {
        Ok(()) => {
            cx.cleanup_envir(rho, Some(res));
            Ok(true)
        }
        Err(e) if cx.exit_info(&e).kind == ExitKind::Exec => {
            let o = val.reset(cx);
            cx.resolve_exit(e, Some(o));
            cx.cleanup_envir(rho, Some(val));
            Ok(false)
        }
        Err(e) => Err(e),
    }
}
