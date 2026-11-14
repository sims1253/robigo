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
    eval_in(cx, e, rho, profile, out)
}

/// `eval(e, rho)` of an expression GNU R holds as byte code (the code of a
/// promise compiled code made): a call runs its `Bc` code, compiled with
/// cmp.R's decisions for `rho`; anything else is evaluated as `eval` does
/// (a symbol's or a constant's byte code is the lookup or the constant).
/// The isolate's `eval_bytecode` hook.
// R: eval.c:eval (BCODESXP), bcEval
pub fn eval_bytecode(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    let profile = prims::state(cx).forced.unwrap_or(Profile::Bc);
    eval_in(cx, e, rho, profile, out)
}

/// [`eval`] with a call compiled for `profile`.
fn eval_in(cx: &mut Cx, e: Root<'_>, rho: Root<'_>, profile: Profile, out: Out<'_>) -> RResult<()> {
    cx.set_visible(true);
    let t = e.bind(cx).sexptype();
    if t == LANGSXP {
        // the compiled call ticks and raises the depth at its own root
        check_env(cx, rho.bind(cx).sexp())?;
        return cx.scope(|cx, s| {
            let code = expr_code(cx, e, rho, profile)?;
            // SAFETY: a fresh or cached code object.
            let code = s.root(cx, unsafe { Ref::from_sexp(code, cx) });
            let slot = out_slot(&out);
            // SAFETY: rooted code and environment; an empty out slot.
            unsafe { vm::exec(cx, code.bind(cx).sexp(), rho.bind(cx).sexp(), slot, false) }
        });
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
    let r = match t {
        // (a symbol is immortal)
        SYMSXP => symbol_value(cx, e.bind(cx).sexp(), rho, out),
        PROMSXP => force(cx, e, out),
        BCODESXP => Err(cx.error("robigo: byte code (BCODESXP) is not implemented yet (M3)")),
        DOTSXP => Err(cx.error("'...' used in an incorrect context")),
        _ => {
            let msg = format!("unimplemented type '{}' in 'eval'\n", type_name(t));
            Err(cx.error(&msg))
        }
    };
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
                Slot::write_counted(slot, Slot::object(v));
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
        Slot::write_counted(slot, Slot::object(v));
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
            && let Some(c) = crate::code::cached(x.aux(), profile)
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

/// `eval(sym, rho)` into register `a` (`..N` when `dd`), the slow path:
/// active bindings, promises to force, GNU R's errors.
pub fn get_var(cx: &mut Cx, act: &Act, a: crate::ir::Reg, sym: Sexp, dd: bool) -> RResult<()> {
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
                act.clear(crate::ir::R_SCRATCH);
                return r;
            }
            // a promise object bound to the symbol: forced
            // SAFETY: a live binding value.
            RawBinding::Value(v) if unsafe { v.typ() } == PROMSXP => {
                // SAFETY: the promise is bound in a rooted frame; the
                // scratch register keeps it for the force.
                let p = unsafe { act.scratch(crate::ir::R_SCRATCH, v) };
                let r = force(cx, p, act.out(a));
                act.clear(crate::ir::R_SCRATCH);
                return r;
            }
            _ => {}
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
    let r = cx.force_arg(frame, index, out);
    cx.set_eval_depth(depthsave);
    r
}

/// The profile `ROBIGO_RT_PROFILE` forces, if any (for tests).
pub fn profile(cx: &Cx) -> Option<Profile> {
    prims::state(cx).forced
}

/// `.Internal(eval(expr, envir, enclos))`: `expr` (a call or symbol, or
/// each element of an expression vector) evaluated in `envir` (an
/// environment, a list or pairlist made into one under `enclos`, or a
/// frame number), inside a `CTXT_RETURN` record so that `return()` and
/// `sys.function()` see the evaluation as GNU R's do.
// R: eval.c:do_eval
pub fn do_eval(cx: &mut Cx, call: Root<'_>, args: robigo_rtcore::Args<'_>, rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    use robigo_rtcore::{Frame, FunctionContext, Promargs, View, ctxt};
    if args.len() != 3 {
        let n = args.len();
        let msg = format!("{n} argument{} passed to .Internal(eval) which requires 3", if n == 1 { "" } else { "s" });
        return Err(cx.error(&msg));
    }
    let _ = call;
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
                                pairs.push((cell.tag().as_sym().map(|t| t.name().bytes().to_vec()), cell.car()));
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
        if t == PROMSXP {
            return eval(cx, expr, rho, out);
        }
        if !matches!(t, LANGSXP | SYMSXP | EXPRSXP) {
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
        let promargs = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut l = cx.nil();
                for i in (0..args.len()).rev() {
                    l = cx.cons(args.get(cx, i), l)?;
                }
                Ok(l)
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
}
