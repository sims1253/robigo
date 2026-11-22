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
/// evaluator ([`crate::install`] registers it). Objects are read through
/// the roots after every call that may collect.
// R: eval.c:eval
pub fn eval(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let profile = prims::state(cx).forced.unwrap_or(Profile::Eval);
    eval_in(cx, e, rho, profile, false, out)
}

/// `eval(e, rho)` of an expression GNU R holds as byte code (the code of a
/// promise compiled code made): a call runs its `Bc` code, compiled with
/// cmp.R's decisions for `rho`; anything else is evaluated as `eval` does
/// (a symbol's or a constant's byte code is the lookup or the constant).
/// The isolate's `eval_bytecode` hook.
// R: eval.c:eval (BCODESXP), bcEval
pub fn eval_bytecode(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let profile = prims::state(cx).forced.unwrap_or(Profile::Bc);
    eval_in(cx, e, rho, profile, true, out)
}

/// An expression of compiled code evaluated in place, as bcEval runs the
/// code compiled into its own (a subscript, an argument of a builtin
/// called by a complex assignment): no evaluation of its own, so the
/// depth is the code's ([`vm::exec`] adds one level, taken off here).
pub fn eval_bytecode_inline(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    match e.bind(cx).sexptype() {
        LANGSXP => {}
        BCODESXP => return bc_eval_inline(cx, e, rho, out),
        _ => return eval_bytecode(cx, e, rho, out),
    }
    let profile = prims::state(cx).forced.unwrap_or(Profile::Bc);
    check_env(cx, rho.bind(cx).sexp())?;
    let depth = cx.eval_depth();
    let r = cx.scope(|cx, s| {
        let code = expr_code(cx, e, rho, profile)?;
        // SAFETY: a fresh or cached code object.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        let slot = out_slot(&out);
        cx.set_eval_depth(depth - 1);
        // SAFETY: rooted code and environment; an empty out slot.
        unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false, false) }.map(drop)
    });
    cx.set_eval_depth(depth);
    r
}

/// `bcEval(e, rho)` of byte code `e` called by compiled code itself, with
/// no `eval` of its own: `MAKEPROM`'s for a builtin's argument (a
/// translated call's promise code). The translation kept on `e`
/// ([`crate::compile::bytecode_code`]) at the code's own depth
/// ([`vm::exec`] adds one level, taken off here), bcEval's entry poll
/// counted; byte code disabled, of a version bcEval does not run, or a
/// forced `Eval` profile evaluates its source (bcEval's fallback,
/// `eval(bytecodeExpr(body), rho)`).
// R: eval.c:bcEval
fn bc_eval_inline(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let st = prims::state(cx);
    let source = st.forced == Some(Profile::Eval)
        || st.jit.get().disable_bytecode
        || !robigo_rtbuiltins::bytecode::version_ok(e.bind(cx));
    if source {
        return cx.scope(|cx, s| {
            let src = s.root(cx, robigo_rtbuiltins::bytecode::expr(cx, e.bind(cx)));
            eval(cx, src, rho, out)
        });
    }
    check_env(cx, rho.bind(cx).sexp())?;
    let depth = cx.eval_depth();
    let r = cx.scope(|cx, s| {
        let code = crate::compile::bytecode_code(cx, e, rho)?;
        // SAFETY: a fresh or cached code object.
        let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
        let slot = out_slot(&out);
        cx.set_eval_depth(depth - 1);
        // SAFETY: rooted code and environment; an empty out slot.
        unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false, false) }.map(drop)
    });
    cx.set_eval_depth(depth);
    r
}

/// [`eval`] with a call compiled for `profile`. `promise`: a byte-code
/// promise's code, which `forcePromise` evaluates under its PROTECT(e):
/// byte code's own `eval` nests the level the protection goes with, any
/// other code starts one protection deeper (robigo_rtcore's ppacct; the
/// AST's promises get theirs from `Cx::eval_promise_code`).
fn eval_in(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, profile: Profile, promise: bool, out: Out<'_>) -> RResult<()> {
    cx.set_visible(true);
    let t = e.bind(cx).sexptype();
    if t == LANGSXP {
        // the compiled call ticks and raises the depth at its own root
        check_env(cx, rho.bind(cx).sexp())?;
        return cx.scope(|cx, s| {
            let code = expr_code(cx, e, rho, profile)?;
            // SAFETY: a fresh or cached code object.
            let pp = (promise && unsafe { crate::code::View::of(code) }.profile == Profile::Eval) as i32;
            // SAFETY: a fresh or cached code object.
            let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
            let slot = out_slot(&out);
            cx.with_pp_excess(pp, |cx| {
                // SAFETY: rooted code and environment; an empty out slot.
                unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false, false) }.map(drop)
            })
        });
    }
    if promise {
        return cx.with_pp_excess(1, |cx| eval_in(cx, e, rho, profile, false, out));
    }
    // R: "this is needed even for self-evaluating objects or something like
    // 'while (TRUE) NULL' will not be interruptable"
    tick(cx)?;
    match t {
        NILSXP | LISTSXP | LGLSXP | INTSXP | REALSXP | STRSXP | CPLXSXP | RAWSXP | OBJSXP | SPECIALSXP | BUILTINSXP
        | ENVSXP | CLOSXP | VECSXP | EXTPTRSXP | WEAKREFSXP | EXPRSXP => {
            // (ENSURE_NAMEDMAX(e) does nothing under reference counting,
            // Defn.h:337-340)
            let x = e.bind(cx);
            out.set(cx, x);
            return Ok(());
        }
        _ => {}
    }
    check_env(cx, rho.bind(cx).sexp())?;
    let depthsave = cx.eval_depth();
    cx.eval_depth_at(depthsave + 1)?;
    if t == SYMSXP && symbol_fast(cx, e.bind(cx).sexp(), rho.bind(cx).sexp(), &out) {
        cx.set_eval_depth(depthsave);
        return Ok(());
    }
    // one level deeper, no protection more (robigo_rtcore's ppacct)
    let r = cx.with_pp_excess(-1, |cx| match t {
        // (a symbol is immortal)
        // SAFETY: `e` is a rooted symbol.
        SYMSXP => unsafe { symbol_value(cx, e.bind(cx).sexp(), rho, out) },
        PROMSXP => force(cx, e, out),
        // bcEval(e, rho): the translated byte code (V3), its code kept on
        // the BCODESXP; byte code disabled, or of a version bcEval does not
        // run, evaluates its source expression (bcEval's fallback)
        // R: eval.c:eval (BCODESXP), bcEval
        BCODESXP => cx.scope(|cx, s| {
            let st = prims::state(cx);
            let source = st.forced == Some(Profile::Eval)
                || st.jit.get().disable_bytecode
                || !robigo_rtbuiltins::bytecode::version_ok(e.bind(cx));
            if source {
                let src = s.root(cx, robigo_rtbuiltins::bytecode::expr(cx, e.bind(cx)));
                return eval(cx, src, rho, out);
            }
            let code = crate::compile::bytecode_code(cx, e, rho)?;
            // SAFETY: a fresh or cached code object.
            let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
            let slot = out_slot(&out);
            // (the code object's entry nests the level `eval` raised)
            cx.set_eval_depth(depthsave);
            // SAFETY: rooted code and environment; an empty out slot.
            unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false, false) }.map(drop)
        }),
        DOTSXP => Err(cx.error("'...' used in an incorrect context")),
        _ => {
            let msg = format!("unimplemented type '{}' in 'eval'\n", type_name(t));
            Err(cx.error(&msg))
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
            // a forced argument: its promise's value
            RawBinding::Forced(v) => {
                let slot = out_slot(out);
                if slot.read() != Slot::EMPTY {
                    return false;
                }
                slot.write(Slot::object(v));
                return true;
            }
            _ => return false,
        };
        // (an evaluated promise's value, unless it is unboxed)
        let v = if v.typ() == PROMSXP {
            if v.promise_tag() != 0 {
                return false;
            }
            v.car()
        } else {
            v
        };
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
pub(crate) fn type_name(t: u8) -> &'static str {
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

/// The code object that evaluates `e` under `profile` (in `rho`, whose
/// compile environment the `Bc` profile's decisions read): for a call,
/// cached in its 4th word (compiled on first use, again when the profile
/// it holds is the other one); for anything else, compiled for this use.
pub fn expr_code(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, profile: Profile) -> RResult<Sexp> {
    let x = e.bind(cx).sexp();
    // SAFETY: rooted; a call's 4th word is R_NilValue or a code object.
    unsafe {
        if x.typ() == LANGSXP
            && let Some(c) = crate::code::cached(cx, x.aux(), profile)
        {
            return Ok(c);
        }
        let code = crate::compile::compile(cx, e, rho, profile)?;
        let x = e.bind(cx).sexp();
        if x.typ() == LANGSXP {
            // the code object counts in the call's 4th word (a traced
            // field), not in any R-visible count
            crate::code::cache_on(x, code);
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
///
/// # Safety
/// `sym` is a live symbol and `shape` R_NilValue or a live frame shape of
/// `cx`'s isolate.
#[inline(always)]
pub unsafe fn get_var_fast(cx: &Cx, act: &Act, a: crate::ir::Reg, sym: Sexp, slot: u16, shape: Sexp) -> bool {
    // SAFETY: an immortal symbol and the activation's rooted environment;
    // the value found is live until the next safepoint and stored at once.
    unsafe {
        // a formal or local of the running closure: its slot, directly
        let found = if slot != 0 { cx.frame_slot_raw(act.rho(), shape, slot as usize - 1) } else { None };
        let found = match found {
            Some(b) => b,
            // a name the shape lacks: a frame of that shape binding nothing
            // else is passed over
            None if slot == 0 => cx.var_lookup_skip_raw(sym, act.rho(), shape),
            // (an S3 method's argument, an alias, read through)
            None => cx.slot_lookup_raw(sym, act.rho(), shape, slot as usize - 1),
        };
        let v = match found {
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
            // a forced argument: its promise's value
            RawBinding::Forced(v) => {
                act.set(a, Slot::object(v));
                return true;
            }
            _ => return false,
        };
        let v = if v.typ() == PROMSXP {
            let pv = v.car();
            if pv == cx.unbound().sexp() || v.promise_tag() != 0 {
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

/// Whether bcEval's `getvar(symbol, rho, dd, keepmiss = TRUE)` reads `sym`
/// as R_MissingArg: bound to it, or to a promise not yet forced that
/// `R_isMissing` finds missing; for a `..N` symbol, the element
/// `ddfindVar` finds (whose errors it raises).
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate.
// R: eval.c:getvar
pub unsafe fn keepmiss_missing(cx: &mut Cx, sym: Sexp, rho: Root<'_>) -> RResult<bool> {
    let missing = cx.missing_arg().sexp();
    let unforced = |cx: &Cx, v: Sexp| {
        // SAFETY: a live binding value.
        unsafe { v.typ() == PROMSXP && v.promise_tag() == 0 && v.car() == cx.unbound().sexp() }
    };
    // SAFETY: a live symbol; gp is a header field.
    let check = if unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0 {
        let x = cx.scope(|cx, s| -> RResult<Sexp> {
            let o = s.out(cx);
            let v = o.root();
            dd_find(cx, sym, rho, o)?;
            Ok(v.bind(cx).sexp())
        })?;
        // (the element is held by `...`'s binding in a rooted frame)
        if x == missing {
            return Ok(true);
        }
        unforced(cx, x)
    } else {
        // SAFETY: an immortal symbol and a rooted environment.
        match unsafe { cx.var_lookup_raw(sym, rho.bind(cx).sexp()) } {
            RawBinding::Value(v) if v == missing => return Ok(true),
            RawBinding::Value(v) => unforced(cx, v),
            RawBinding::Thunk(_) => true,
            _ => false,
        }
    };
    if !check {
        return Ok(false);
    }
    // SAFETY: the symbol and the rooted environment.
    unsafe { crate::missing::r_is_missing(cx, sym, rho.bind(cx).sexp()) }
}

/// GETVAR_MISSOK (DDVAL_MISSOK for a `..N` symbol): [`get_var`], but a
/// missing argument is R_MissingArg ([`keepmiss_missing`]).
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate.
// R: eval.c:getvar
pub unsafe fn get_var_missok(cx: &mut Cx, act: &Act, a: crate::ir::Reg, sym: Sexp, bc: bool) -> RResult<()> {
    // SAFETY: the caller's contract: a live symbol.
    if unsafe { keepmiss_missing(cx, sym, act.root(crate::ir::R_ENV)) }? {
        let m = cx.missing_arg().sexp();
        // SAFETY: R_MissingArg is immortal.
        unsafe { act.set(a, Slot::object(m)) };
        return Ok(());
    }
    // SAFETY: a live symbol; gp is a header field.
    let dd = unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0;
    // SAFETY: the caller's contract: a live symbol.
    unsafe { get_var(cx, act, a, sym, dd, bc) }
}

/// `eval(sym, rho)` into register `a` (`..N` when `dd`), the slow path:
/// active bindings, promises to force, GNU R's errors.
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate.
pub unsafe fn get_var(cx: &mut Cx, act: &Act, a: crate::ir::Reg, sym: Sexp, dd: bool, bc: bool) -> RResult<()> {
    // a missing argument's error: byte code's (`getvar`) or the AST
    // interpreter's (`eval`)
    let subclass = if bc { "getvarError" } else { "evalError" };
    if !dd && sym != prims::state(cx).syms.dots {
        // SAFETY: an immortal symbol and the rooted environment.
        match unsafe { cx.var_lookup_raw(sym, act.rho()) } {
            // an argument's first read (the common slow path): its slot
            // forced in place, no promise made
            RawBinding::Thunk(loc) => {
                // SAFETY: the frame holding the slot, reachable from the
                // rooted environment; the scratch register keeps it.
                let f = unsafe { act.scratch(crate::ir::R_SCRATCH, loc.frame()) };
                let r = cx.force_arg(f, loc.index(), act.out(a));
                act.kill(crate::ir::R_SCRATCH, u32::MAX);
                return r;
            }
            // a promise object bound to the symbol: forced
            // SAFETY: a live binding value.
            RawBinding::Value(v) if unsafe { v.typ() } == PROMSXP => {
                // SAFETY: the promise is bound in a rooted frame; the
                // scratch register keeps it for the force.
                let p = unsafe { act.scratch(crate::ir::R_SCRATCH, v) };
                let r = force(cx, p, act.out(a));
                act.kill(crate::ir::R_SCRATCH, u32::MAX);
                return r;
            }
            // byte code reads an active binding twice: bcEval's inlined
            // GETVAR looks the variable up (findVarEX: the binding's function
            // runs) to see whether it is a promise of byte code to force in
            // place, and then reads it again through getvar(), which runs the
            // function again for its value. (Only an unforced promise would
            // make the first read's value matter; an active binding's
            // function returns a value.)
            // R: eval.c:DO_GETVAR (INLINE_GETVAR), eval.c:getvar
            RawBinding::Active(_) if bc => {
                cx.scope(|cx, s| {
                    // SAFETY: an immortal symbol.
                    let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
                    let o = s.out(cx);
                    cx.find_var(sr, act.root(crate::ir::R_ENV), o)
                })?;
            }
            _ => {}
        }
    }
    if dd {
        dd_value(cx, sym, act.root(crate::ir::R_ENV), act.out(a), subclass)
    } else {
        // SAFETY: the caller's contract: a live symbol.
        unsafe { symbol_value_as(cx, sym, act.root(crate::ir::R_ENV), act.out(a), subclass) }
    }
}

/// `eval`'s SYMSXP case: `R_findVar`, then GNU R's errors, a promise
/// forced.
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate.
// R: eval.c:eval (SYMSXP)
pub unsafe fn symbol_value(cx: &mut Cx, sym: Sexp, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    // SAFETY: the caller's contract.
    unsafe { symbol_value_as(cx, sym, rho, out, "evalError") }
}

/// [`symbol_value`] whose missing argument raises the `missingArgError`
/// of subclass `subclass` (`evalError` from `eval`, `getvarError` from
/// byte code's `getvar`).
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate.
pub unsafe fn symbol_value_as(
    cx: &mut Cx,
    sym: Sexp,
    rho: Root<'_>,
    out: Out<'_>,
    subclass: &'static str,
) -> RResult<()> {
    let st = prims::state(cx);
    if sym == st.syms.dots {
        return Err(cx.error("'...' used in an incorrect context"));
    }
    // SAFETY: a live symbol.
    if unsafe { sym.gp() } & robigo_rtcore::DDVAL != 0 {
        return dd_value(cx, sym, rho, out, subclass);
    }
    // an argument not yet read: its slot forced in place
    // SAFETY: an immortal symbol and a rooted environment.
    if let RawBinding::Thunk(loc) = unsafe { cx.var_lookup_raw(sym, rho.bind(cx).sexp()) } {
        return cx.scope(|cx, s| {
            // SAFETY: the frame holding the slot, reachable from `rho`.
            let f = s.root(cx, unsafe { Ref::from_sexp(loc.frame(), cx) });
            cx.force_arg(f, loc.index(), out)
        });
    }
    let res = out.root();
    cx.scope(|cx, s| {
        // SAFETY: an immortal symbol.
        let sr = s.root(cx, unsafe { Ref::from_sexp(sym, cx) });
        cx.find_var(sr, rho, out)?;
        finish_value(cx, sym, rho, res, s, subclass)
    })
}

/// The rest of `eval`'s SYMSXP case once the value is in `res`: an unbound
/// or missing value is GNU R's error, a promise is forced.
fn finish_value(
    cx: &mut Cx,
    sym: Sexp,
    rho: Root<'_>,
    res: Root<'_>,
    s: robigo_rtcore::Scope<'_>,
    subclass: &'static str,
) -> RResult<()> {
    let mut v = res.bind(cx).sexp();
    if v == cx.unbound().sexp() && sym == cx.missing_arg().sexp() {
        // the empty symbol R_MissingArg is its own value (names.c:
        // mkSymMarker): evaluating it is the missing-argument error
        v = sym;
    }
    if v == cx.unbound().sexp() {
        // R: errorcall_cpy(getLexicalCall(rho), _("object '%s' not found"), EncodeChar(PRINTNAME(e)))
        // SAFETY: `finish_value`'s callers (this module) pass a live symbol.
        let name = unsafe { sym_name(cx, sym) };
        let call = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
        let msg = format!("object '{name}' not found");
        return Err(cx.error_call(Some(call), &msg));
    }
    if v == cx.missing_arg().sexp() {
        // R: R_MissingArgError(e, getLexicalCall(rho), "evalError"), or
        // getvar's "getvarError"
        // SAFETY: `finish_value`'s callers (this module) pass a live symbol.
        let name = unsafe { sym_name(cx, sym) };
        let call = s.root(cx, cx.lexical_context(rho.bind(cx)).call());
        return Err(cx.missing_arg_error(&name, Some(call), subclass));
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
///
/// # Safety
/// `sym` is a live symbol of `cx`'s isolate: its PRINTNAME is read.
pub unsafe fn sym_name(cx: &Cx, sym: Sexp) -> String {
    // SAFETY: a live symbol.
    let s = unsafe { Ref::from_sexp(sym, cx) }.as_sym().unwrap();
    String::from_utf8_lossy(s.name_bytes()).into_owned()
}

/// `ddfindVar(sym, rho)` and the rest of `eval`'s SYMSXP case.
// R: envir.c:ddfindVar
fn dd_value(cx: &mut Cx, sym: Sexp, rho: Root<'_>, out: Out<'_>, subclass: &'static str) -> RResult<()> {
    let res = out.root();
    cx.scope(|cx, s| {
        dd_find(cx, sym, rho, out)?;
        finish_value(cx, sym, rho, res, s, subclass)
    })
}

/// `ddfindVar(sym, rho)`: the element of `...` a `..N` symbol names, as
/// bound (a promise is not forced), or GNU R's errors.
// R: envir.c:ddfindVar, ddfindInFrame
fn dd_find(cx: &mut Cx, sym: Sexp, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    // SAFETY: `dd_find`'s callers (this module) pass a live symbol.
    let name = unsafe { sym_name(cx, sym) };
    let i = robigo_rtcore::dd_val(name.as_bytes());
    if i <= 0 {
        return Err(cx.error(&format!("indexing '...' with non-positive index {i}")));
    }
    let dots = prims::state(cx).syms.dots;
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
        Ok(())
    })
}

// ---- promises ----------------------------------------------------------------------

/// `forcePromise(e)` and the value (`PRVALUE`) written to `out`: the
/// isolate's `force` hook (rtcore's port, `Cx::force_promise`, with the
/// pending-promise protocol and immediate values).
// R: eval.c:forcePromise
pub fn force(cx: &mut Cx, p: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.force_promise(p, out)
}

/// `eval(p, rho)` of the promise GNU R's binding in slot `index` of
/// `frame` holds (the `force_arg` hook: `findFun3` forcing an argument):
/// `eval`'s evaluation count and depth for the promise, then the slot
/// forced in place.
// R: eval.c:eval (PROMSXP)
pub fn force_arg_eval(cx: &mut Cx, frame: Root<'_>, index: usize, out: Out<'_>) -> RResult<()> {
    cx.set_visible(true);
    tick(cx)?;
    let depthsave = cx.eval_depth();
    cx.eval_depth_at(depthsave + 1)?;
    // (one level deeper, no protection more: robigo_rtcore's ppacct)
    let r = cx.with_pp_excess(-1, |cx| cx.force_arg(frame, index, out));
    cx.set_eval_depth(depthsave);
    r
}

/// The profile `ROBIGO_RT_PROFILE` forces, if any (for tests).
pub fn profile(cx: &Cx) -> Option<Profile> {
    prims::state(cx).forced
}

/// `.Internal(returnValue(default))`: the value the function whose
/// `on.exit` code is running returns (`R_ExitContext`'s), marked not
/// mutable; `default` when no exit is under way or it carries no value (an
/// error, a jump through the function).
// R: eval.c:do_returnValue
pub fn do_return_value(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "returnValue", 1, true)?;
    cx.scope(|cx, s| {
        let v = cx.exit_context().and_then(|c| c.return_value());
        match v {
            Some(v) => {
                let v = s.root(cx, v);
                cx.mark_not_mutable(v);
                out.set(cx, v.bind(cx));
            }
            None => out.set(cx, args.get(cx, 0)),
        }
        Ok(())
    })
}

/// `.Internal(withVisible(x))`, a special: `x` evaluated in `rho`, and
/// `list(value =, visible =)` with `R_Visible` as that evaluation left it.
// R: eval.c:do_withVisible
/// `args` is GNU R's: None for the call's own arguments, or `applyMethod`'s
/// promise list (run again on the call over it, `specials::rerun_apart`).
pub fn do_with_visible(
    cx: &mut Cx,
    call: Root<'_>,
    _op: Root<'_>,
    args: Option<Root<'_>>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // (applyMethod's promise list: run again on the call over it)
    if args.is_some() {
        return crate::specials::rerun_apart(cx, call, _op, args, rho, out, do_with_visible);
    }
    use robigo_rtbuiltins::attrib::{Attr, Attrs};
    use robigo_rtcore::{Char, Encoding, Lgl, RSym};
    let n = call.bind(cx).len().unwrap_or(1).saturating_sub(1);
    crate::prims::check_arity(cx, call, n, "withVisible", 1, true)?;
    cx.scope(|cx, s| {
        let e = s.root(cx, call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().car());
        let o = s.out(cx);
        let x = o.root();
        eval(cx, e, rho, o)?;
        let visible = cx.visible();
        cx.build(out, |cx| {
            let mut ret = cx.alloc_vector::<Ref<'_>>(2)?;
            let mut nm = cx.alloc_vector::<Char<'_>>(2)?;
            nm.set(0, cx.mk_char(b"value", Encoding::Native)?);
            nm.set(1, cx.mk_char(b"visible", Encoding::Native)?);
            ret.set(0, x.bind(cx));
            ret.set(1, cx.alloc_copy(&[Lgl(visible as i32)])?.finish().into());
            let mut a = Attrs::new();
            a.install(Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
            a.apply(cx, &mut ret)?;
            Ok(ret.finish().into())
        })
    })
}

/// `.Internal(Recall(...))`, in `Recall <- function(...)`: the function
/// that called `Recall` called again with `Recall`'s arguments (the
/// promises of its record), from the environment that function was called
/// from. A special: `rho` is `Recall`'s frame.
// R: eval.c:do_recall
pub fn do_recall(
    cx: &mut Cx,
    _call: Root<'_>,
    _op: Root<'_>,
    // (unread: GNU R's do_recall calls again with the closure's own
    // promargs, never the special's `args`)
    _args: Option<Root<'_>>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    use robigo_rtcore::ctxt;
    cx.scope(|cx, s| {
        let outside = "'Recall' called from outside a closure";
        // the record of the closure whose frame is rho (Recall's): its
        // arguments
        let me = {
            let r = rho.bind(cx).sexp();
            let mut c = Some(cx.global_context());
            while let Some(x) = c {
                if x.callflag() == ctxt::RETURN && x.cloenv().sexp() == r {
                    break;
                }
                c = x.next();
            }
            c.map(|x| x.id())
        };
        // the env Recall was called from, and the record of the closure
        // whose frame it is (searched from Recall's)
        let sp = cx.global_context().sysparent().sexp();
        let cptr = me.and_then(|me| {
            let mut c = cx.context(me);
            while let Some(x) = c {
                if x.callflag() == ctxt::RETURN && x.cloenv().sexp() == sp {
                    return Some(x.id());
                }
                c = x.next();
            }
            None
        });
        let (Some(me), Some(cptr)) = (me, cptr) else {
            return Err(cx.error(outside));
        };
        cx.materialize_promargs(me)?;
        let args =
            s.root(cx, cx.context(me).expect("robigo: Recall's record, found above").promargs().unwrap_or(cx.nil()));
        let (ccall, callfun, sysparent) = {
            let c = cx.context(cptr).expect("robigo: the record of the closure Recall was called from, found above");
            (c.call(), c.callfun(), c.sysparent())
        };
        let ccall = s.root(cx, ccall);
        let sysparent = s.root(cx, sysparent);
        // the function recorded in the record, else the call's function
        // found by name or evaluated
        let fun = if callfun != cx.nil() {
            s.root(cx, callfun)
        } else {
            // CAR(cptr->call), as GNU R reads it (R_NilValue's CAR is itself)
            let head = s.root(cx, ccall.bind(cx).as_cons().map_or(cx.nil(), |c| c.car()));
            let o = s.out(cx);
            let f = o.root();
            if head.bind(cx).sexptype() == SYMSXP {
                cx.find_fun3(head, sysparent, None, o)?;
            } else {
                eval(cx, head, sysparent, o)?;
            }
            f
        };
        if fun.bind(cx).sexptype() != CLOSXP {
            return Err(cx.error(outside));
        }
        crate::call::apply_closure(cx, ccall, fun, args, sysparent, None, out)
    })
}

/// `.Internal(eval(expr, envir, enclos))`: `expr` (a call or symbol, or
/// each element of an expression vector) evaluated in `envir` (an
/// environment, a list or pairlist made into one under `enclos`, or a
/// frame number), inside a `CTXT_RETURN` record so that `return()` and
/// `sys.function()` see the evaluation as GNU R's do.
// R: eval.c:do_eval
pub fn do_eval(cx: &mut Cx, call: Root<'_>, args: robigo_rtcore::Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    use robigo_rtcore::{Frame, FunctionContext, Promargs, View, ctxt};
    crate::prims::check_arity(cx, call, args.len(), "eval", 3, true)?;
    cx.scope(|cx, s| {
        let expr = args.root(0);
        let encl = {
            let e = args.get(cx, 2);
            match e.sexptype() {
                NILSXP => s.root(cx, cx.base_env()),
                ENVSXP => s.root(cx, e),
                _ => {
                    let msg =
                        format!("invalid 'enclos' argument of type '{}'", robigo_rtbuiltins::util::type_to_char(e));
                    return Err(cx.error(&msg));
                }
            }
        };
        let envarg = args.get(cx, 1);
        let env: Root<'_> = match envarg.sexptype() {
            NILSXP => encl,
            ENVSXP => s.root(cx, envarg),
            OBJSXP if envarg.is_s4() => {
                return Err(robigo_rtbuiltins::util::not_yet(
                    cx,
                    None,
                    "eval() in an S4 environment (R_getS4DataSlot)",
                ));
            }
            LISTSXP | VECSXP => {
                // NewEnvironment(R_NilValue, <the named elements>, encl)
                let e = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(encl.bind(cx), false, 0)))?;
                let items: Vec<(Root<'_>, Root<'_>)> = {
                    let v = args.get(cx, 1);
                    let mut pairs: Vec<(Option<Vec<u8>>, Ref<'_>)> = Vec::new();
                    match v.view() {
                        View::List(l) => {
                            let names = robigo_rtbuiltins::attrib::names(v)
                                .and_then(|n| n.as_vector::<robigo_rtcore::Char<'_>>());
                            for i in 0..l.len() {
                                let nm = names.map(|n| n.get(i)).filter(|c| !c.bytes().is_empty());
                                pairs.push((nm.map(|c| c.bytes().to_vec()), l.get(i)));
                            }
                        }
                        _ => {
                            let mut c = v.as_cons();
                            while let Some(cell) = c {
                                pairs.push((cell.tag().as_sym().map(|t| t.name_bytes().to_vec()), cell.car()));
                                c = cell.cdr().as_cons();
                            }
                        }
                    }
                    if pairs.iter().any(|(n, _)| n.is_none()) && v.sexptype() == LISTSXP {
                        return Err(robigo_rtbuiltins::util::not_yet(
                            cx,
                            None,
                            "eval() in a pairlist with untagged elements",
                        ));
                    }
                    // NewEnvironment's frame is the list itself: of two
                    // equal names, the first is the one found
                    let mut seen = std::collections::HashSet::new();
                    let rooted: Vec<(Vec<u8>, Root<'_>)> = pairs
                        .into_iter()
                        .filter_map(|(n, v)| n.filter(|n| seen.insert(n.clone())).map(|n| (n, s.root(cx, v))))
                        .collect();
                    let mut out = Vec::new();
                    for (n, vr) in rooted {
                        let sym = s.capture(cx, |cx, o| cx.build(o, |cx| Ok(cx.install(&n)?.into())))?;
                        out.push((sym, vr));
                    }
                    out
                };
                for (sym, val) in items {
                    cx.define_var(sym, val, e)?;
                }
                e
            }
            INTSXP | REALSXP => {
                if envarg.len() != Some(1) {
                    return Err(cx.error("numeric 'envir' arg not of length one"));
                }
                let frame = robigo_rtbuiltins::coerce::vector::as_integer_warn(cx, args.root(1))?;
                if frame == i32::MIN {
                    let msg = format!(
                        "invalid 'envir' argument of type '{}'",
                        robigo_rtbuiltins::util::type_to_char(args.get(cx, 1))
                    );
                    return Err(cx.error(&msg));
                }
                let f = {
                    let g = cx.global_context();
                    cx.sys_frame(frame, g).map(|f| match f {
                        Frame::Global => cx.global_env(),
                        Frame::Env(e) => e,
                    })
                };
                match f {
                    Ok(e) => s.root(cx, e),
                    Err(e) => return Err(cx.walk_error(e)),
                }
            }
            _ => {
                let msg = format!(
                    "invalid 'envir' argument of type '{}'",
                    robigo_rtbuiltins::util::type_to_char(args.get(cx, 1))
                );
                return Err(cx.error(&msg));
            }
        };
        let t = expr.bind(cx).sexptype();
        // do_eval evaluates under PROTECT(env) and, but for a promise,
        // PROTECT(expr) (robigo_rtcore's ppacct)
        if t == PROMSXP {
            return cx.with_pp_excess(1, |cx| eval(cx, expr, rho, out));
        }
        if !matches!(t, LANGSXP | SYMSXP | EXPRSXP | BCODESXP) {
            out.set(cx, expr.bind(cx));
            return Ok(());
        }
        // the record: R_GlobalContext->call, env, rho, args, op
        let gcall = s.root(cx, cx.global_context().call());
        let op = {
            let st = prims::state(cx);
            let i = st.defs.iter().position(|d| d.internal() && d.name == "eval").expect("robigo: eval in the table");
            s.root(cx, st.objs[i].bind(cx))
        };
        // (the record's args are evalList's list, whose cells count no
        // references, CONS_NR: the expression's references stay its
        // bindings', so a write into it while it runs is in place)
        let promargs = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let nil = cx.nil().sexp();
                let mut l = nil;
                for i in (0..args.len()).rev() {
                    let v = args.get(cx, i).sexp();
                    // SAFETY: the call's live values and the list made so
                    // far; nothing collects in this interval
                    l = unsafe {
                        let c = cx.alloc_node_raw(LISTSXP, v, l, nil)?;
                        // CONS_NR: the construction's counts undone, and
                        // none later
                        v.decref();
                        if l != nil {
                            l.decref();
                        }
                        c.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
                        c
                    };
                }
                // SAFETY: a live object of this isolate: the first cell made
                // above, or with no arguments R_NilValue (immortal).
                Ok(unsafe { Ref::from_sexp(l, cx) })
            })
        })?;
        let spec = FunctionContext {
            callflag: ctxt::RETURN,
            call: gcall,
            cloenv: env,
            sysparent: rho,
            callfun: op,
            promargs: Promargs::Filled(promargs),
        };
        let mut on_exit = |cx: &mut Cx, e: Root<'_>, env: Root<'_>| -> RResult<()> {
            cx.scope(|cx, s| {
                let o = s.out(cx);
                eval(cx, e, env, o)
            })
        };
        cx.function_context(&spec, out, &mut on_exit, |cx, _id, out| {
            cx.with_pp_excess(2, |cx| {
                if t == EXPRSXP {
                    let n = expr.bind(cx).len().unwrap_or(0);
                    if n == 0 {
                        out.set(cx, cx.nil());
                        return Ok(());
                    }
                    cx.scope(|cx, s| {
                        let last = s.out(cx).root();
                        for i in 0..n {
                            let e = match expr.bind(cx).view() {
                                View::Expr(v) => s.root(cx, v.get(i)),
                                _ => unreachable!("robigo: an expression vector"),
                            };
                            let r = s.capture(cx, |cx, o| eval(cx, e, env, o))?;
                            last.replace_from(cx, r);
                        }
                        out.set(cx, last.bind(cx));
                        Ok(())
                    })
                } else {
                    eval(cx, expr, env, out)
                }
            })
        })
    })
}
