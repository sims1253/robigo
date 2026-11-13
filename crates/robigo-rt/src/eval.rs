//! `eval` (GNU R's, eval.c:1081): the evaluator's entry point, what the
//! isolate's `eval` hook calls (`cx.eval`), and the parts of it the
//! VM's instructions share: variable lookup, promise forcing, the polls.
//!
//! A call is evaluated by compiling it (once: the code object hangs off the
//! call, as GNU R's byte code hangs off a closure's body) and running the
//! code; a symbol, a promise or a constant is evaluated directly. There is
//! no AST interpreter (docs/g3.md §9.1).

#![allow(unsafe_code)]

use crate::ir::Profile;
use crate::prims;
use crate::vm::{self, Act};
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Out, RResult, RawBinding, Ref, Root, Slot};
use std::ptr::NonNull;

/// GNU R's `eval(e, rho)`, the value written to `out`: the isolate's
/// evaluator ([`crate::install`] registers it).
// R: eval.c:eval
pub fn eval(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let (e, rho) = (e.bind(cx).sexp(), rho.bind(cx).sexp());
    // SAFETY: both are rooted by the caller.
    unsafe { eval_raw(cx, e, rho, out) }
}

/// [`eval`] on raw objects.
///
/// # Safety
/// `e` and `rho` are live objects of `cx`'s isolate, rooted by the caller.
// R: eval.c:eval
pub unsafe fn eval_raw(cx: &mut Cx, e: Sexp, rho: Sexp, out: Out<'_>) -> RResult<()> {
    cx.set_visible(true);
    // SAFETY: the caller's contract.
    let t = unsafe { e.typ() };
    if t == LANGSXP {
        // the compiled call ticks and raises the depth at its own root
        return cx.scope(|cx, s| {
            // SAFETY: rooted by the caller.
            let er = s.root(cx, unsafe { Ref::from_sexp(e, cx) });
            check_env(cx, rho)?;
            let code = expr_code(cx, er)?;
            // SAFETY: a fresh or cached code object.
            let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
            let slot = out_slot(&out);
            // SAFETY: rooted code and environment; an empty out slot.
            unsafe { vm::exec(cx, code.bind(cx).sexp(), rho, slot, false) }
        });
    }
    // R: "this is needed even for self-evaluating objects or something like
    // 'while (TRUE) NULL' will not be interruptable"
    tick(cx)?;
    match t {
        NILSXP | LISTSXP | LGLSXP | INTSXP | REALSXP | STRSXP | CPLXSXP | RAWSXP | OBJSXP | SPECIALSXP | BUILTINSXP
        | ENVSXP | CLOSXP | VECSXP | EXTPTRSXP | WEAKREFSXP | EXPRSXP => {
            // ENSURE_NAMEDMAX(e)
            // SAFETY: a live object; REFCNT is a header field.
            unsafe { mark_not_mutable(e) };
            // SAFETY: rooted by the caller.
            out.set(cx, unsafe { Ref::from_sexp(e, cx) });
            return Ok(());
        }
        _ => {}
    }
    check_env(cx, rho)?;
    let depthsave = cx.eval_depth();
    cx.eval_depth_at(depthsave + 1)?;
    if t == SYMSXP && symbol_fast(cx, e, rho, &out) {
        cx.set_eval_depth(depthsave);
        return Ok(());
    }
    let r = cx.scope(|cx, s| {
        // SAFETY: rooted by the caller.
        let rr = s.root(cx, unsafe { Ref::from_sexp(rho, cx) });
        match t {
            SYMSXP => symbol_value(cx, e, rr, out),
            PROMSXP => {
                // SAFETY: as above.
                let p = s.root(cx, unsafe { Ref::from_sexp(e, cx) });
                force(cx, p, out)
            }
            BCODESXP => Err(cx.error("robigo: byte code (BCODESXP) is not implemented yet (M3)")),
            DOTSXP => Err(cx.error("'...' used in an incorrect context")),
            _ => {
                let msg = format!("unimplemented type '{}' in 'eval'\n", type_name(t));
                Err(cx.error(&msg))
            }
        }
    });
    cx.set_eval_depth(depthsave);
    r
}

/// `eval`'s SYMSXP case when the binding holds a plain value or an
/// evaluated promise (no `...`, `..N`, active binding or error): written
/// to `out`. False: the full path must run.
fn symbol_fast(cx: &Cx, sym: Sexp, rho: Sexp, out: &Out<'_>) -> bool {
    // SAFETY: `sym` and `rho` are rooted by `eval`'s caller; the value
    // found is written to a root at once.
    unsafe {
        if sym.gp() & robigo_rtcore::DDVAL != 0 || sym == prims::state(cx).syms.dots {
            return false;
        }
        let v = match cx.var_lookup_raw(sym, rho) {
            RawBinding::Value(v) => v,
            _ => return false,
        };
        let v = if v.typ() == PROMSXP { v.car() } else { v };
        if v == cx.unbound().sexp() || v == cx.missing_arg().sexp() {
            return false;
        }
        let slot = out_slot(out);
        if slot.read() != Slot::EMPTY {
            return false;
        }
        slot.write(Slot::object(v));
    }
    true
}

/// `eval`'s check of its environment argument.
fn check_env(cx: &mut Cx, rho: Sexp) -> RResult<()> {
    // SAFETY: rooted by the caller of eval.
    let t = unsafe { rho.typ() };
    if t == ENVSXP {
        return Ok(());
    }
    // (these are deliberately not translated in GNU R)
    let msg = format!("'rho' must be an environment not {}: detected in C-level eval", type_name(t));
    Err(cx.error(&msg))
}

/// `R_typeToChar`'s names for the types `eval` reports.
fn type_name(t: u8) -> &'static str {
    match t {
        NILSXP => "NULL",
        SYMSXP => "symbol",
        LISTSXP => "pairlist",
        CLOSXP => "closure",
        ENVSXP => "environment",
        PROMSXP => "promise",
        LANGSXP => "language",
        SPECIALSXP => "special",
        BUILTINSXP => "builtin",
        CHARSXP => "char",
        LGLSXP => "logical",
        INTSXP => "integer",
        REALSXP => "double",
        CPLXSXP => "complex",
        STRSXP => "character",
        DOTSXP => "...",
        ANYSXP => "any",
        EXPRSXP => "expression",
        VECSXP => "list",
        EXTPTRSXP => "externalptr",
        BCODESXP => "bytecode",
        WEAKREFSXP => "weakref",
        RAWSXP => "raw",
        OBJSXP => "object",
        _ => "unknown",
    }
}

/// The slot an `Out` writes.
fn out_slot(out: &Out<'_>) -> NonNull<Slot> {
    // SAFETY: an `Out`'s slot is a live value-stack slot.
    unsafe { NonNull::new_unchecked(out.root().as_ptr() as *mut Slot) }
}

/// The code object that evaluates `e`: for a call, cached in its 4th word
/// (compiled on first use); for anything else, compiled for this use.
pub fn expr_code(cx: &mut Cx, e: Root<'_>) -> RResult<Sexp> {
    let profile = prims::state(cx).profile();
    let x = e.bind(cx).sexp();
    // SAFETY: rooted; a call's 4th word is R_NilValue or a code object.
    unsafe {
        if x.typ() == LANGSXP {
            let c = x.aux();
            if crate::code::is_code(c, profile) {
                return Ok(c);
            }
        }
        let code = crate::compile::compile(cx, x)?;
        let x = e.bind(cx).sexp();
        if x.typ() == LANGSXP {
            // the code object counts in the call's 4th word (a traced
            // field), not in any R-visible count
            x.set_aux(code);
        }
        Ok(code)
    }
}

/// GNU R's `ENSURE_NAMEDMAX` / `MARK_NOT_MUTABLE`.
///
/// # Safety
/// `x` is live.
pub unsafe fn mark_not_mutable(x: Sexp) {
    // SAFETY: the caller's contract.
    unsafe {
        if !x.has_flag(robigo_heap::flags::IMMORTAL) {
            x.set_refcnt(u16::MAX);
        }
    }
}

// ---- the polls ------------------------------------------------------------------

/// `eval`'s evaluation count (eval.c:1091): one per evaluation; past 1000,
/// GNU R's interrupt check and pending finalizers (docs/g3.md §3 item 7).
/// True when the poll ran (code may have run, a collection may have moved
/// objects).
#[inline(always)]
pub fn tick(cx: &mut Cx) -> RResult<bool> {
    let c = cx.evalcount().get() + 1;
    cx.evalcount().set(c);
    if c > 1000 {
        poll(cx)?;
        cx.evalcount().set(0);
        return Ok(true);
    }
    Ok(false)
}

/// The poll of [`tick`]: `R_CheckUserInterrupt()`, then
/// `R_RunPendingFinalizers()`.
// R: eval.c:eval (the evalcount block)
#[cold]
#[inline(never)]
fn poll(cx: &mut Cx) -> RResult<()> {
    cx.check_user_interrupt()?;
    crate::fin::run_pending_finalizers(cx);
    Ok(())
}

/// `bcEval`'s `BC_CHECK_SIGINT()` (eval.c:6252): the `Bc` profile's count,
/// polled past `BC_COUNT_DELTA` (1023).
#[inline(always)]
pub fn bc_tick(cx: &mut Cx) -> RResult<bool> {
    let c = cx.bc_evalcount().get() + 1;
    cx.bc_evalcount().set(c);
    if c > 1023 {
        poll(cx)?;
        cx.bc_evalcount().set(0);
        return Ok(true);
    }
    Ok(false)
}

/// `BC_CHECK_SIGINT_LOOP(i)` (eval.c:6261): a `for` loop's poll every
/// 1024 iterations by its index.
#[inline(always)]
pub fn bc_tick_loop(cx: &mut Cx, i: usize) -> RResult<()> {
    if i & 1023 == 0 {
        poll(cx)?;
        cx.bc_evalcount().set(0);
    }
    Ok(())
}

// ---- variables -------------------------------------------------------------------

/// The fast path of `eval(sym)` into register `a`: a bound value that is
/// not a promise to force, `R_MissingArg` or an active binding. False when
/// the slow path ([`get_var`]) must run.
#[inline(always)]
pub fn get_var_fast(cx: &Cx, act: &Act, a: crate::ir::Reg, sym: Sexp) -> bool {
    // SAFETY: an immortal symbol and the activation's rooted environment;
    // the value found is live until the next safepoint and stored at once.
    unsafe {
        let v = match cx.var_lookup_raw(sym, act.rho()) {
            RawBinding::Value(v) => v,
            // a local the VM stored unboxed: read unboxed
            RawBinding::Scalar(x) => {
                act.set(
                    a,
                    match x {
                        robigo_rtcore::Scalar::Real(r) => Slot::real(r),
                        robigo_rtcore::Scalar::Int(i) => Slot::int(i),
                        robigo_rtcore::Scalar::Lgl(l) => Slot::logical(l.0),
                    },
                );
                return true;
            }
            _ => return false,
        };
        let v = if v.typ() == PROMSXP {
            let pv = v.car();
            if pv == cx.unbound().sexp() {
                return false;
            }
            pv
        } else {
            if v == cx.missing_arg().sexp() || v == cx.unbound().sexp() {
                return false;
            }
            v
        };
        act.set(a, Slot::object(v));
    }
    true
}

/// `eval(sym, rho)` into register `a` (`..N` when `dd`), the slow path:
/// active bindings, promises to force, GNU R's errors.
pub fn get_var(cx: &mut Cx, act: &Act, a: crate::ir::Reg, sym: Sexp, dd: bool) -> RResult<()> {
    if !dd && sym != prims::state(cx).syms.dots {
        // an unforced promise bound to the symbol: force it (the common
        // slow path, an argument's first read)
        // SAFETY: an immortal symbol and the rooted environment.
        if let RawBinding::Value(v) = unsafe { cx.var_lookup_raw(sym, act.rho()) }
            // SAFETY: a live binding value.
            && unsafe { v.typ() } == PROMSXP
        {
            // SAFETY: the promise is bound in a rooted frame; the scratch
            // register keeps it for the force.
            let p = unsafe { act.scratch(crate::ir::R_SCRATCH, v) };
            let r = force(cx, p, act.out(a));
            act.clear(crate::ir::R_SCRATCH);
            return r;
        }
    }
    if dd {
        dd_value(cx, sym, act.root(crate::ir::R_ENV), act.out(a))
    } else {
        symbol_value(cx, sym, act.root(crate::ir::R_ENV), act.out(a))
    }
}

/// `eval`'s SYMSXP case: `R_findVar`, then GNU R's errors, a promise
/// forced.
// R: eval.c:eval (SYMSXP)
pub fn symbol_value(cx: &mut Cx, sym: Sexp, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let st = prims::state(cx);
    if sym == st.syms.dots {
        return Err(cx.error("'...' used in an incorrect context"));
    }
    // SAFETY: a live symbol.
    if unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0 {
        return dd_value(cx, sym, rho, out);
    }
    let res = out.root();
    cx.scope(|cx, s| {
        // SAFETY: an immortal symbol.
        let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
        cx.find_var(sr, rho, out)?;
        finish_value(cx, sym, rho, res, s)
    })
}

/// The rest of `eval`'s SYMSXP case once the value is in `res`: an unbound
/// or missing value is GNU R's error, a promise is forced.
fn finish_value(cx: &mut Cx, sym: Sexp, rho: Root<'_>, res: Root<'_>, s: robigo_rtcore::Scope<'_>) -> RResult<()> {
    let v = res.bind(cx).sexp();
    if v == cx.unbound().sexp() {
        // R: errorcall_cpy(getLexicalCall(rho), _("object '%s' not found"), EncodeChar(PRINTNAME(e)))
        let name = sym_name(cx, sym);
        let call = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
        let msg = format!("object '{name}' not found");
        return Err(cx.error_call(Some(call), &msg));
    }
    if v == cx.missing_arg().sexp() {
        // R: R_MissingArgError(e, getLexicalCall(rho), "evalError")
        let name = sym_name(cx, sym);
        let call = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
        let msg = if name.is_empty() {
            "argument is missing, with no default".to_owned()
        } else {
            format!("argument \"{name}\" is missing, with no default")
        };
        return Err(cx.error_call(Some(call), &msg));
    }
    // SAFETY: a live object.
    if unsafe { v.typ() } == PROMSXP {
        let p = s.root(cx, res.bind(cx));
        let o = res.reset(cx);
        force(cx, p, o)?;
    }
    Ok(())
}

/// A symbol's name (`CHAR(PRINTNAME(sym))`; GNU R's `EncodeChar`
/// escaping is the text layer's).
pub fn sym_name(cx: &Cx, sym: Sexp) -> String {
    // SAFETY: a live symbol.
    let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym().unwrap();
    String::from_utf8_lossy(s.name().bytes()).into_owned()
}

/// `ddfindVar(sym, rho)` and the rest of `eval`'s SYMSXP case.
// R: envir.c:ddfindVar
fn dd_value(cx: &mut Cx, sym: Sexp, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let name = sym_name(cx, sym);
    let i = robigo_rtcore::dd_val(name.as_bytes());
    let dots = prims::state(cx).syms.dots;
    let res = out.root();
    cx.scope(|cx, s| {
        // SAFETY: an immortal symbol.
        let d = s.root(cx, unsafe { Ref::from_sexp(dots, cx) });
        let vl = s.out(cx);
        let vlr = vl.root();
        cx.find_var(d, rho, vl)?;
        let vl = vlr.bind(cx).sexp();
        if vl == cx.unbound().sexp() {
            let msg = format!("..{i} used in an incorrect context, no ... to look in");
            return Err(cx.error(&msg));
        }
        // length_DOTS(vl)
        // SAFETY: a rooted object.
        let len = if unsafe { vl.typ() } == DOTSXP { vlr.bind(cx).len().unwrap_or(0) as i64 } else { 0 };
        if len < i {
            let msg = if i == 1 {
                format!("the ... list contains fewer than {i} element")
            } else {
                format!("the ... list contains fewer than {i} elements")
            };
            return Err(cx.error(&msg));
        }
        // nthcdr(vl, i - 1), then CAR
        let mut c = vl;
        for _ in 1..i {
            // SAFETY: a live DOTSXP list longer than i - 1.
            c = unsafe { c.cdr() };
        }
        // SAFETY: as above.
        let v = unsafe { c.car() };
        // SAFETY: an element of a rooted list.
        out.set(cx, unsafe { Ref::from_sexp(v, cx) });
        finish_value(cx, sym, rho, res, s)
    })
}

// ---- promises ----------------------------------------------------------------------

/// GNU R's `PRSEEN` (the promise's gp).
const PRSEEN_EVALUATING: u16 = 1;
const PRSEEN_INTERRUPTED: u16 = 2;

/// `forcePromise(e)` and the value (`PRVALUE`) written to `out`.
// R: eval.c:forcePromise
pub fn force(cx: &mut Cx, p: Root<'_>, out: Out<'_>) -> RResult<()> {
    let ps = p.bind(cx).sexp();
    // SAFETY: a rooted promise.
    let v = unsafe { ps.car() };
    if v != cx.unbound().sexp() {
        // SAFETY: PRVALUE of a rooted promise.
        out.set(cx, unsafe { Ref::from_sexp(v, cx) });
        return Ok(());
    }
    cx.safepoint(robigo_heap::Safepoint::Entry)?;
    let ps = p.bind(cx).sexp();
    // SAFETY: as above; PRSEEN is the gp field.
    let seen = unsafe { ps.gp() };
    if seen != 0 {
        let call = cx.scope(|cx, s| s.root(cx, cx.global_context().call()).bind(cx).sexp());
        if seen == PRSEEN_EVALUATING {
            return cx.scope(|cx, s| {
                // SAFETY: the context's call, a live object.
                let c = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
                Err(cx.error_call(
                    Some(c),
                    "promise already under evaluation: recursive default argument reference or earlier problems?",
                ))
            });
        }
        // SAFETY: as above.
        unsafe { ps.set_gp(PRSEEN_EVALUATING) };
        cx.scope(|cx, s| {
            // SAFETY: the context's call, a live object.
            let c = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
            cx.warning_call(Some(c), "restarting interrupted promise evaluation")
        })?;
    }
    // SAFETY: as above.
    unsafe { ps.set_gp(PRSEEN_EVALUATING) };
    let res = out.root();
    let r = cx.scope(|cx, s| {
        // SAFETY: PRCODE and PRENV of a rooted promise.
        let (code, env) = unsafe { (ps.cdr(), ps.tag()) };
        // SAFETY: as above.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        // SAFETY: as above.
        let env = s.root(cx, unsafe { Ref::from_sexp(env, cx) });
        eval(cx, code, env, out)
    });
    let ps = p.bind(cx).sexp();
    match r {
        Err(e) => {
            // a jump out of the evaluation: R_restore_globals marks the
            // pending promise interrupted
            // SAFETY: as above.
            unsafe { ps.set_gp(PRSEEN_INTERRUPTED) };
            Err(e)
        }
        Ok(()) => {
            let val = res.bind(cx).sexp();
            // SAFETY: a rooted promise and value: SET_PRVALUE (counted),
            // ENSURE_NAMEDMAX, SET_PRSEEN(0), SET_PRENV(R_NilValue).
            unsafe {
                ps.set_car(val);
                mark_not_mutable(val);
                ps.set_gp(0);
                ps.set_tag(cx.nil().sexp());
            }
            Ok(())
        }
    }
}

/// The profile code is compiled for (for tests).
pub fn profile(cx: &Cx) -> Profile {
    prims::state(cx).profile()
}
