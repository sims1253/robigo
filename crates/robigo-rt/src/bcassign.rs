//! Complex assignment in compiled code (docs/g3.md §9.1): what GNU R's
//! byte-code compiler makes of `f(x, args) <- value`, `x[i] <- value`,
//! `names(x)[2] <- value`, `x$a <<- value` (cmp.R's `cmpComplexAssign`,
//! `cmpGetterCall`, `cmpSetterCall`, the getter and setter inline handlers
//! of `$`, `[`, `[[`), run as bcEval runs it (`STARTASSIGN`,
//! `GETTER_CALL`, `SWAP`, `SETTER_CALL`, `DOLLAR`, `DOLLARGETS`, the
//! subset and subassign dispatch ops, `ENDASSIGN`). The AST's `applydefine`
//! ([`crate::assign`]) stays for the `Eval` profile.
//!
//! Where the two differ, observably:
//!
//! - **`*tmp*` is never bound.** The object goes to a getter or setter as
//!   an argument: an evaluated promise of `*tmp*` for a closure or a
//!   special, the value for a builtin.
//! - **The calls are the compiler's.** A getter's is the place with
//!   `*tmp*` (`` `[`(`*tmp*`, i) ``), a setter's is `` `f<-`(`*tmp*`,
//!   args, value = vexpr) `` with the right-hand side's *expression* (and
//!   `*vtmp*` for the inner setters), where the AST's holds a promise of
//!   the value. `sys.call()`, `match.call()` and errors reported against
//!   the call show it.
//! - **No `CTXT_CCODE` record.** `error()` in a setter reports what
//!   `getCurrentCall` finds: the setter's call on the original object,
//!   inflated back into an assignment (`R_getBCInterpreterExpression`).
//! - **`$`, `[`, `[[` and their setters are frozen** when base's at
//!   compile time (their handlers), with indices compiled into the code:
//!   an object dispatches (`tryDispatch`, `tryAssignDispatch`), anything
//!   else goes to the default methods with the evaluated indices.
//!
//! # The plan
//!
//! [`crate::compile`] makes a plan per assignment, a VECSXP the code's
//! pool keeps: the target symbol ([`P_SYM`]), the getter steps from the
//! inside out ([`P_GETTERS`]) and the setter steps from the outside in
//! ([`P_SETTERS`]). A step ([`make_step`]) is its kind, its call constant,
//! what it calls (a function symbol or expression; `$`'s member), the
//! setter's value expression, and the expression `error()` reports.

#![allow(unsafe_code)]

use crate::ir::{R_ENV, Reg};
use crate::links;
use crate::prims::{self, Kind};
use crate::specials::{Evaluated, ev_promise};
use crate::vm::Act;
use robigo_heap::types::*;
use robigo_heap::{NeedGc, Sexp};
use robigo_rtcore::{Args, Cx, Out, RResult, RawBinding, Ref, Root, Scope};

// A plan's elements.
pub const P_SYM: usize = 0;
pub const P_GETTERS: usize = 1;
pub const P_SETTERS: usize = 2;
pub const PLAN_LEN: usize = 3;

// A step's elements.
const S_KIND: usize = 0;
const S_CALL: usize = 1;
const S_WHAT: usize = 2;
const S_VEXPR: usize = 3;
const S_REPORT: usize = 4;
const S_FINAL: usize = 5;
const STEP_LEN: usize = 6;

/// A call of the function the step names (`GETFUN`, or `CHECKFUN` of an
/// expression): `GETTER_CALL` / `SETTER_CALL`.
pub const K_GENERIC: u8 = 0;
/// `[` / `[<-` with plain indices (`cmpSubsetGetterDispatch`,
/// `cmpSubassignDispatch`).
pub const K_SUB: u8 = 1;
/// `[[` / `[[<-` likewise.
pub const K_SUB2: u8 = 2;
/// `$` / `$<-` of a name (`DOLLAR`, `DOLLARGETS`).
pub const K_DOLLAR: u8 = 3;
/// `[` / `[<-` with empty or named arguments (`cmpGetterDispatch`,
/// `cmpSetterDispatch`): the default method gets them as they are.
pub const K_SUB_PUNT: u8 = 4;
/// `[[` / `[[<-` likewise.
pub const K_SUB2_PUNT: u8 = 5;

/// cmp.R's `checkAssign` for the target `place`: a symbol, or calls whose
/// functions are symbols (or `pkg::f`) down to a symbol. Otherwise the
/// compiler leaves the assignment to `<-` itself (`cmpSpecial`), the AST's.
///
/// # Safety
/// `place` is live.
// R: cmp.R:checkAssign
pub unsafe fn check_assign(cx: &Cx, place: Sexp) -> bool {
    let st = prims::state(cx);
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        let mut p = place;
        if p.typ() == SYMSXP || (p.typ() == STRSXP && p.std_len() == 1) {
            return true;
        }
        while p.typ() == LANGSXP {
            let fun = p.car();
            let colon = fun.typ() == LANGSXP && {
                let el = crate::cenv::elements(cx, fun, false);
                el.len() == 3
                    && el[0].0.typ() == SYMSXP
                    && (el[0].0 == st.syms.double_colon || el[0].0 == st.syms.triple_colon)
            };
            if fun.typ() != SYMSXP && !colon {
                return false;
            }
            let rest = p.cdr();
            p = if rest == nil { nil } else { rest.car() };
        }
        p.typ() == SYMSXP
    }
}

/// A step of a plan.
///
/// # Safety
/// The objects are live, reachable from the source or immortal.
#[allow(clippy::too_many_arguments)]
pub unsafe fn make_step(
    cx: &Cx,
    kind: u8,
    call: Sexp,
    what: Sexp,
    vexpr: Option<Sexp>,
    report: Sexp,
    fin: bool,
) -> Result<Sexp, crate::cenv::Need> {
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract; a fresh vector, nothing collects.
    unsafe {
        let k = cx.alloc_vector_raw(INTSXP, 1)?;
        *(k.std_data() as *mut i32) = kind as i32;
        let f = cx.alloc_vector_raw(LGLSXP, 1)?;
        *(f.std_data() as *mut i32) = fin as i32;
        let s = cx.alloc_vector_raw(VECSXP, STEP_LEN)?;
        s.set_elt(S_KIND, k);
        s.set_elt(S_CALL, call);
        s.set_elt(S_WHAT, what);
        s.set_elt(S_VEXPR, vexpr.unwrap_or(nil));
        s.set_elt(S_REPORT, report);
        s.set_elt(S_FINAL, f);
        Ok(s)
    }
}

/// `inflateAssignmentCall(expr)`: `` `f<-`(x, args, value = v) `` as
/// `f(x, args) <- v` (unchanged unless it has two arguments and its last is
/// tagged `value`).
///
/// # Safety
/// `expr` is a live call whose function is a symbol.
// R: eval.c:inflateAssignmentCall
pub unsafe fn inflate_assignment_call(cx: &Cx, expr: Sexp) -> Result<Sexp, NeedGc> {
    let st = prims::state(cx);
    let nil = cx.nil().sexp();
    // SAFETY: the caller's contract.
    unsafe {
        if expr.cdr() == nil || expr.cdr().cdr() == nil {
            return Ok(expr);
        }
        let name = Ref::from_sexp(expr.car(), cx).as_sym().map(|s| s.name().bytes().to_vec()).unwrap_or_default();
        if name.len() <= 2 || !name.ends_with(b"<-") {
            return Ok(expr);
        }
        let args = crate::cenv::elements(cx, expr, true);
        let (rhs, rtag) = *args.last().unwrap();
        if rtag != st.syms.value {
            return Ok(expr);
        }
        let non = cx.install(&name[..name.len() - 2])?.as_ref().sexp();
        let mut l = nil;
        for &(a, t) in args[..args.len() - 1].iter().rev() {
            l = cx.alloc_node_raw(LISTSXP, a, l, t)?;
        }
        let lhs = cx.alloc_node_raw(LANGSXP, non, l, nil)?;
        let r = cx.alloc_node_raw(LISTSXP, rhs, nil, nil)?;
        let l = cx.alloc_node_raw(LISTSXP, lhs, r, nil)?;
        cx.alloc_node_raw(LANGSXP, st.syms.assign, l, nil)
    }
}

/// A `Ref` of `x`, a live object reachable from a root for this interval.
fn raw(cx: &Cx, x: Sexp) -> Ref<'_> {
    // SAFETY: each caller passes a cell of a rooted list, a binding's value
    // read in this interval, or a constant of the rooted plan.
    unsafe { Ref::from_sexp(x, cx) }
}

/// A step's field.
fn field<'g>(cx: &'g Cx, step: Root<'_>, i: usize) -> Ref<'g> {
    let s = step.bind(cx);
    s.as_vector::<Ref<'_>>().expect("robigo: a plan step").get(i)
}

fn kind(cx: &Cx, step: Root<'_>) -> u8 {
    let k = field(cx, step, S_KIND);
    // SAFETY: a step's kind is an integer scalar.
    unsafe { *(k.sexp().std_data() as *const i32) as u8 }
}

/// The complex assignment of `plan` (`super_`: `<<-`), the right-hand
/// side's value in register `rhs`: bcEval's `STARTASSIGN`, the getter
/// calls, the setter calls and `ENDASSIGN`; the value in `dst`, invisible.
// R: eval.c: STARTASSIGN, STARTASSIGN2, ENDASSIGN, ENDASSIGN2 (bcEval)
pub fn run(cx: &mut Cx, act: &Act, dst: Reg, rhs: Reg, plan: Sexp, super_: bool) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code; a register's value.
        let plan = s.root(cx, unsafe { Ref::from_sexp(plan, cx) });
        let saverhs = {
            let v = crate::vm::boxed(cx, act, rhs)?;
            // SAFETY: a register's live object.
            s.root(cx, unsafe { Ref::from_sexp(v, cx) })
        };
        act.clear(rhs);
        let rho = act.root(R_ENV);
        // INCLNK_stack_commit: the stack's values hold links meanwhile
        let old = links::commit(cx);
        let r = body(cx, s, plan, rho, saverhs, super_);
        links::release(cx, old);
        r?;
        cx.set_visible(false);
        act.out(dst).set(cx, saverhs.bind(cx));
        Ok(())
    })
}

fn body(cx: &mut Cx, s: Scope<'_>, plan: Root<'_>, rho: Root<'_>, saverhs: Root<'_>, super_: bool) -> RResult<()> {
    let sym = s.root(cx, field(cx, plan, P_SYM));
    // the right-hand side counts a reference meanwhile (STARTASSIGN's
    // `if (refrhs) INCREMENT_REFCNT`, STARTASSIGN2's always;
    // FIXUP_RHS_NAMED does nothing under reference counting), dropped on
    // every path out as the AST's applydefine drops it here
    let refrhs = (super_ || saverhs.bind(cx).maybe_referenced()).then(|| {
        let p = std::ptr::NonNull::new(saverhs.as_ptr() as *mut robigo_rtcore::Slot).unwrap();
        // SAFETY: a slot of the caller's scope, which outlives the guard,
        // holding the value.
        unsafe {
            let mut l = links::Linked::slots(p);
            l.link_next();
            l
        }
    });
    // STARTASSIGN: the target's binding cell and value
    let (cell, value) = if super_ {
        let cell = crate::assign::find_var_loc(cx, s, sym, rho)?;
        let en = crate::assign::enclos(cx, s, rho);
        let o = s.out(cx);
        let v = o.root();
        crate::eval::eval(cx, sym, en, o)?;
        (cell, v)
    } else {
        let o = s.out(cx);
        let cell = o.root();
        let found = cx.find_var_loc_in_frame(rho, sym, o)?;
        let v = s.out(cx).root();
        let o = v.reset(cx);
        if found {
            cx.var_loc_value(cell, o)?;
        } else {
            o.set(cx, cx.unbound());
        }
        let r = v.bind(cx);
        if r == cx.unbound() || r.sexptype() == PROMSXP {
            let (v, cell) = crate::assign::ensure_local(cx, s, sym, rho)?;
            (cell, v)
        } else {
            (cell, v)
        }
    };
    let has_cell = cell.bind(cx).sexptype() != NILSXP;
    let maybe_in_assign = has_cell && cx.assignment_pending(cell.bind(cx));
    if has_cell {
        cx.set_assignment_pending(cell, true);
    }
    if maybe_in_assign || value.bind(cx).maybe_shared() {
        dup(cx, s, value)?;
    }
    let mut vals = vec![value];
    // the getter calls, from the inside out
    let getters = s.root(cx, field(cx, plan, P_GETTERS));
    let ng = getters.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
    for j in 0..ng {
        let step = s.root(cx, getters.bind(cx).as_vector::<Ref<'_>>().unwrap().get(j));
        let lhs = *vals.last().unwrap();
        let v = with_expression(cx, step, |cx| getter(cx, s, step, lhs, rho))?;
        // SWAP: an extracted value that may be shared, or whose container
        // may be (it went through a closure), is copied
        let x = v.bind(cx);
        if x.maybe_referenced() && (x.maybe_shared() || lhs.bind(cx).maybe_shared()) {
            dup(cx, s, v)?;
        }
        vals.push(v);
    }
    // the setter calls, from the outside in
    let setters = s.root(cx, field(cx, plan, P_SETTERS));
    let ns = setters.bind(cx).as_vector::<Ref<'_>>().unwrap().len();
    let mut rhs = saverhs;
    for k in 0..ns {
        let step = s.root(cx, setters.bind(cx).as_vector::<Ref<'_>>().unwrap().get(k));
        let lhs = vals[vals.len() - 1 - k];
        rhs = with_expression(cx, step, |cx| setter(cx, s, step, lhs, rhs, rho))?;
    }
    // ENDASSIGN
    if has_cell {
        cx.set_assignment_pending(cell, false);
    }
    if super_ {
        let en = crate::assign::enclos(cx, s, rho);
        cx.set_var(sym, rhs, en)?;
    } else {
        cx.define_var(sym, rhs, rho)?;
    }
    if let Some(l) = refrhs {
        l.release();
    }
    Ok(())
}

/// Run `f` with the expression bcEval is at the step's (`setcurexpr`):
/// what `error()` reports there ([`Cx::error_context_call`]).
fn with_expression<'s>(cx: &mut Cx, step: Root<'_>, f: impl FnOnce(&mut Cx) -> RResult<Root<'s>>) -> RResult<Root<'s>> {
    let report = field(cx, step, S_REPORT).sexp();
    let fin = {
        let f = field(cx, step, S_FINAL);
        // SAFETY: a logical scalar.
        unsafe { *(f.sexp().std_data() as *const i32) != 0 }
    };
    // SAFETY: a constant of the plan, which the rooted code keeps.
    unsafe { cx.set_bc_expression_final(report, fin) };
    let r = f(cx);
    // SAFETY: R_NilValue.
    unsafe { cx.set_bc_expression(cx.nil().sexp()) };
    r
}

/// `x` replaced by `shallow_duplicate(x)` in its root.
fn dup(cx: &mut Cx, s: Scope<'_>, x: Root<'_>) -> RResult<()> {
    let o = s.out(cx);
    let d = o.root();
    cx.shallow_duplicate(x, o)?;
    x.replace_from(cx, d);
    Ok(())
}

// ---- getters ----------------------------------------------------------------------

/// A getter step on the object `lhs`: the extracted value.
fn getter<'s>(cx: &mut Cx, s: Scope<'s>, step: Root<'_>, lhs: Root<'_>, rho: Root<'_>) -> RResult<Root<'s>> {
    let call = s.root(cx, field(cx, step, S_CALL));
    let what = s.root(cx, field(cx, step, S_WHAT));
    let out = s.out(cx);
    let v = out.root();
    match kind(cx, step) {
        K_DOLLAR => {
            // DOLLAR
            if lhs.bind(cx).is_object() {
                let ncall = dollar_call(cx, s, call, what, None)?;
                if dispatch(cx, s, "$", ncall, lhs, rho, &out)? {
                    cx.set_visible(true);
                    return Ok(v);
                }
            }
            let name = s.root(cx, what.bind(cx).as_sym().unwrap().name().as_ref());
            robigo_rtbuiltins::subset::subset3_dflt(cx, lhs, name, call, out)?;
            cx.set_visible(true);
        }
        k @ (K_SUB | K_SUB2 | K_SUB_PUNT | K_SUB2_PUNT) => {
            let sub2 = matches!(k, K_SUB2 | K_SUB2_PUNT);
            if lhs.bind(cx).is_object() && dispatch(cx, s, if sub2 { "[[" } else { "[" }, call, lhs, rho, &out)? {
                cx.set_visible(true);
                return Ok(v);
            }
            let ev = default_args(cx, s, call, lhs, rho, None)?;
            let args = ev.args(cx);
            if sub2 {
                robigo_rtbuiltins::subset::do_subset2_dflt(cx, call, args, rho, out)?;
            } else {
                robigo_rtbuiltins::subset::do_subset_dflt(cx, call, args, rho, out)?;
            }
            cx.set_visible(true);
        }
        _ => generic(cx, s, call, what, lhs, None, rho, out)?,
    }
    Ok(v)
}

// ---- setters ----------------------------------------------------------------------

/// A setter step on the object `lhs` with the value `rhs`: the object
/// with the part replaced.
fn setter<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    step: Root<'_>,
    lhs: Root<'_>,
    rhs: Root<'_>,
    rho: Root<'_>,
) -> RResult<Root<'s>> {
    let call = s.root(cx, field(cx, step, S_CALL));
    let what = s.root(cx, field(cx, step, S_WHAT));
    let vexpr = s.root(cx, field(cx, step, S_VEXPR));
    let lhs = s.root(cx, lhs.bind(cx));
    let out = s.out(cx);
    let v = out.root();
    cx.mark_assignment_call(call);
    // (MAYBE_SHARED(lhs): shallow_duplicate, in every setter op)
    if lhs.bind(cx).maybe_shared() {
        dup(cx, s, lhs)?;
    }
    match kind(cx, step) {
        K_DOLLAR => {
            // DOLLARGETS
            if lhs.bind(cx).is_object() {
                let ncall = dollar_call(cx, s, call, what, Some(rhs))?;
                if dispatch(cx, s, "$<-", ncall, lhs, rho, &out)? {
                    return Ok(v);
                }
            }
            robigo_rtbuiltins::subassign::subassign3_dflt(cx, call, lhs, what, rhs, out)?;
        }
        k @ (K_SUB | K_SUB2 | K_SUB_PUNT | K_SUB2_PUNT) => {
            let sub2 = matches!(k, K_SUB2 | K_SUB2_PUNT);
            if lhs.bind(cx).is_object() {
                // tryAssignDispatch: the call's last argument a promise of
                // the value
                let ncall = rhs_call(cx, s, call, rhs)?;
                if dispatch(cx, s, if sub2 { "[[<-" } else { "[<-" }, ncall, lhs, rho, &out)? {
                    return Ok(v);
                }
            }
            let ev = default_args(cx, s, call, lhs, rho, Some(rhs))?;
            let args = ev.args(cx);
            if sub2 {
                robigo_rtbuiltins::subassign::do_subassign2_dflt(cx, call, args, rho, out)?;
            } else {
                robigo_rtbuiltins::subassign::do_subassign_dflt(cx, call, args, rho, out)?;
            }
        }
        _ => generic(cx, s, call, what, lhs, Some((vexpr, rhs)), rho, out)?,
    }
    Ok(v)
}

// ---- dispatch -----------------------------------------------------------------------

/// `tryDispatch(generic, call, x, rho, &value)`: S3 dispatch on `x`, the
/// call's first argument (an evaluated promise of `*tmp*`), the rest
/// promised from the call. True when a method ran (its value in `out`).
// R: eval.c:tryDispatch
fn dispatch(
    cx: &mut Cx,
    s: Scope<'_>,
    generic: &'static str,
    call: Root<'_>,
    x: Root<'_>,
    rho: Root<'_>,
    out: &Out<'_>,
) -> RResult<bool> {
    let c = call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap();
    let (first_expr, rest) = (s.root(cx, c.car()), s.root(cx, c.cdr()));
    let first = ev_promise(cx, s, first_expr, x)?;
    crate::dispatch::dispatch_special(cx, generic, call, first, rest, rho, out)
}

/// `duplicate(call)` with its last argument a promise of the value
/// `mkRHSPROMISE(CAR(last), rhs)` (`tryAssignDispatch`).
// R: eval.c:tryAssignDispatch
fn rhs_call<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, rhs: Root<'_>) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let n = o.root();
    cx.duplicate(call, o)?;
    let mut last = n.bind(cx).sexp();
    // SAFETY: a fresh copy of a call with arguments.
    unsafe {
        while last.cdr() != cx.nil().sexp() {
            last = last.cdr();
        }
    }
    let l = s.root(cx, raw(cx, last));
    // SAFETY: a cell of the rooted copy.
    let e = s.root(cx, unsafe { Ref::from_sexp(last.car(), cx) });
    let p = crate::assign::rhs_promise(cx, s, e, rhs)?;
    cx.set_car(l, p);
    Ok(n)
}

/// `$`'s call for dispatch: `duplicate(call)` with the member as a string
/// (`ScalarString(PRINTNAME(symbol))`, "hack to avoid evaluating the
/// symbol"), and for `$<-` the value a promise (`mkRHSPROMISE`).
// R: eval.c: DOLLAR, DOLLARGETS
fn dollar_call<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Root<'_>,
    member: Root<'_>,
    rhs: Option<Root<'_>>,
) -> RResult<Root<'s>> {
    let o = s.out(cx);
    let n = o.root();
    cx.duplicate(call, o)?;
    let name = s.out(cx).root();
    let o = name.reset(cx);
    cx.build(o, |cx| {
        let nm = member.bind(cx).as_sym().unwrap();
        let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
        v.set(0, nm.name());
        Ok(v.finish().into())
    })?;
    let third = s.root(cx, raw(cx, n.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr().sexp()));
    cx.set_car(third, name);
    if let Some(rhs) = rhs {
        let fourth = s.root(cx, raw(cx, third.bind(cx).as_cons().unwrap().cdr().sexp()));
        let e = s.root(cx, fourth.bind(cx).as_cons().unwrap().car());
        let p = crate::assign::rhs_promise(cx, s, e, rhs)?;
        cx.set_car(fourth, p);
    }
    Ok(n)
}

// ---- the default methods' arguments ------------------------------------------------

/// The arguments of a subset or subassign default method: the object,
/// then the call's arguments after `*tmp*` (but a setter's `value`),
/// evaluated as compiled code evaluates them (`cmpIndices`,
/// `cmpBuiltinArgs(missingOK = TRUE)`: an empty argument, or a symbol
/// bound to the missing argument, is R_MissingArg; tags kept), then the
/// setter's value tagged `value`.
fn default_args<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    call: Root<'_>,
    x: Root<'_>,
    rho: Root<'_>,
    value: Option<Root<'_>>,
) -> RResult<Evaluated<'s>> {
    let st_value = prims::state(cx).syms.value;
    let nil = cx.nil().sexp();
    let args = args_of(cx, s, call, value.is_some());
    let mut ev = Evaluated::new(cx, s, args.len() + 1 + value.is_some() as usize);
    ev.next(nil).set(cx, x.bind(cx));
    for (a, tag) in args {
        let o = ev.next(tag);
        eval_arg(cx, a, rho, true, o)?;
    }
    if let Some(v) = value {
        ev.next(st_value).set(cx, v.bind(cx));
    }
    Ok(ev)
}

/// The argument expressions of a step's call after `*tmp*` (a setter's
/// without its `value`), rooted, with their tags.
fn args_of<'s>(cx: &Cx, s: Scope<'s>, call: Root<'_>, setter: bool) -> Vec<(Root<'s>, Sexp)> {
    let mut v = Vec::new();
    let mut c = call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().cdr();
    while let Some(cell) = c.as_cons() {
        if setter && cell.cdr().sexp() == cx.nil().sexp() {
            break;
        }
        v.push((s.root(cx, cell.car()), cell.tag().sexp()));
        c = cell.cdr();
    }
    v
}

/// An argument expression evaluated as compiled code does in this frame:
/// a call by its code (compiled with the assignment), at the code's own
/// depth; a symbol looked up (`GETVAR`, or `GETVAR_MISSOK` with
/// `missing_ok`: a binding to the missing argument is R_MissingArg); an
/// empty argument R_MissingArg; a constant itself.
fn eval_arg(cx: &mut Cx, a: Root<'_>, rho: Root<'_>, missing_ok: bool, out: Out<'_>) -> RResult<()> {
    let e = a.bind(cx);
    let missing = cx.missing_arg();
    if e == missing {
        out.set(cx, missing);
        return Ok(());
    }
    match e.sexptype() {
        LANGSXP => crate::eval::eval_bytecode_inline(cx, a, rho, out),
        SYMSXP => {
            if missing_ok
                // SAFETY: an immortal symbol and a rooted environment.
                && let RawBinding::Value(v) = unsafe { cx.var_lookup_raw(e.sexp(), rho.bind(cx).sexp()) }
                && v == missing.sexp()
            {
                out.set(cx, missing);
                return Ok(());
            }
            crate::eval::symbol_value(cx, e.sexp(), rho, out)
        }
        _ => {
            out.set(cx, e);
            Ok(())
        }
    }
}

// ---- GETTER_CALL, SETTER_CALL --------------------------------------------------------

/// A getter (`value` None) or setter call of the function the step names
/// (`what`: a symbol, found by `findFun`, or an expression, evaluated and
/// checked), on the object `lhs`: by the function's type, as
/// `GETTER_CALL` / `SETTER_CALL` call it.
// R: eval.c: GETFUN, CHECKFUN, GETTER_CALL, SETTER_CALL (bcEval)
#[allow(clippy::too_many_arguments)]
fn generic(
    cx: &mut Cx,
    s: Scope<'_>,
    call: Root<'_>,
    what: Root<'_>,
    lhs: Root<'_>,
    value: Option<(Root<'_>, Root<'_>)>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let fun = s.out(cx).root();
    if what.bind(cx).sexptype() == SYMSXP {
        // "could not find function" reported as error() reports it
        let o = fun.reset(cx);
        cx.find_fun3(what, rho, None, o)?;
    } else {
        let o = fun.reset(cx);
        crate::eval::eval_bytecode_inline(cx, what, rho, o)?;
        if !matches!(fun.bind(cx).sexptype(), CLOSXP | BUILTINSXP | SPECIALSXP) {
            return Err(cx.error("attempt to apply non-function"));
        }
    }
    let tmp = prims::state(cx).syms.tmpval;
    // SAFETY: an immortal symbol.
    let tmpr = s.root(cx, unsafe { Ref::from_sexp(tmp, cx) });
    let setter = value.is_some();
    match fun.bind(cx).sexptype() {
        BUILTINSXP => {
            let args = args_of(cx, s, call, setter);
            let n = builtin_count(cx, &args, rho)?;
            let mut ev = Evaluated::new(cx, s, 1 + n + setter as usize);
            ev.next(cx.nil().sexp()).set(cx, lhs.bind(cx));
            builtin_args(cx, &mut ev, &args, rho)?;
            if let Some((_, rhs)) = value {
                let vs = prims::state(cx).syms.value;
                ev.next(vs).set(cx, rhs.bind(cx));
            }
            check_for_missings(cx, &ev, call)?;
            let a = ev.args(cx);
            call_builtin(cx, call, fun, a, rho, out)
        }
        SPECIALSXP => {
            // duplicate(CDR(call)), the object an evaluated promise of
            // `*tmp*` that counts nothing, a setter's value one of its
            // expression; the special gets the call made of them
            let o = s.out(cx);
            let args = o.root();
            let rest = s.root(cx, call.bind(cx).as_cons().unwrap().cdr());
            cx.duplicate(rest, o)?;
            let p = ev_promise(cx, s, tmpr, lhs)?;
            cx.set_car(args, p);
            if let Some((vexpr, rhs)) = value {
                let mut last = args.bind(cx).sexp();
                // SAFETY: a fresh copy of the call's arguments.
                unsafe {
                    while last.cdr() != cx.nil().sexp() {
                        last = last.cdr();
                    }
                }
                let l = s.root(cx, raw(cx, last));
                let p = crate::assign::rhs_promise(cx, s, vexpr, rhs)?;
                cx.set_car(l, p);
            }
            let o = s.out(cx);
            let ncall = o.root();
            let head = s.root(cx, call.bind(cx).as_cons().unwrap().car());
            cx.build(o, |cx| cx.lcons(head.bind(cx), args.bind(cx)))?;
            crate::apply::apply_special(cx, ncall, fun, rho, out)
        }
        _ => {
            // a promise list: `*tmp*` and a setter's value evaluated
            // promises (counted), the arguments as compiled code makes them
            let list = closure_args(cx, s, call, setter, rho)?;
            let first = mk_ev_promise(cx, s, tmpr, lhs)?;
            let o = s.out(cx);
            let full = o.root();
            cx.build(o, |cx| cx.cons(first.bind(cx), list.bind(cx)))?;
            if let Some((vexpr, rhs)) = value {
                let p = mk_ev_promise(cx, s, vexpr, rhs)?;
                append(cx, s, full, p, prims::state(cx).syms.value)?;
            }
            crate::call::apply_closure(cx, call, fun, full, rho, None, out)
        }
    }
}

/// `R_mkEVPROMISE(expr, value)`: an evaluated promise (counted).
fn mk_ev_promise<'s>(cx: &mut Cx, s: Scope<'s>, expr: Root<'_>, value: Root<'_>) -> RResult<Root<'s>> {
    let p = cx.retry_alloc(|cx| {
        // SAFETY: rooted objects; nothing collects in this interval.
        unsafe {
            let p = crate::args::mk_promise(cx, expr.bind(cx).sexp(), cx.nil().sexp())?;
            p.set_car(value.bind(cx).sexp());
            Ok(p)
        }
    })?;
    // SAFETY: fresh.
    Ok(s.root(cx, unsafe { Ref::from_sexp(p, cx) }))
}

/// Append `x` tagged `tag` to the non-empty pairlist in `list`.
fn append(cx: &mut Cx, s: Scope<'_>, list: Root<'_>, x: Root<'_>, tag: Sexp) -> RResult<()> {
    let o = s.out(cx);
    let cell = o.root();
    cx.build(o, |cx| {
        // SAFETY: an immortal symbol.
        let t = unsafe { Ref::from_sexp(tag, cx) }.as_sym().unwrap();
        cx.cons_tagged(x.bind(cx), cx.nil(), t)
    })?;
    let mut last = list.bind(cx).sexp();
    // SAFETY: a rooted pairlist.
    unsafe {
        while last.cdr() != cx.nil().sexp() {
            last = last.cdr();
        }
    }
    let l = s.root(cx, raw(cx, last));
    cx.set_cdr(l, cell);
    Ok(())
}

/// How many values a builtin's arguments make (`...` counts its
/// elements).
fn builtin_count(cx: &mut Cx, args: &[(Root<'_>, Sexp)], rho: Root<'_>) -> RResult<usize> {
    let dots = prims::state(cx).syms.dots;
    let mut n = 0;
    for (a, _) in args {
        if a.bind(cx).sexp() == dots {
            // SAFETY: an immortal symbol and a rooted environment.
            if let RawBinding::Value(h) = unsafe { cx.var_lookup_raw(dots, rho.bind(cx).sexp()) } {
                let h = raw(cx, h);
                if h.sexptype() == DOTSXP {
                    n += h.len().unwrap_or(0);
                }
            }
        } else {
            n += 1;
        }
    }
    Ok(n)
}

/// A builtin's arguments as compiled code makes them (`cmpCallArgs` for a
/// builtin frame): a call or symbol evaluated now (`MAKEPROM`'s `bcEval`),
/// a constant itself (marked not mutable), an empty argument R_MissingArg,
/// `...` its elements evaluated (`DODOTS`).
// R: eval.c: MAKEPROM, PUSHCONSTARG, DOMISSING, DODOTS (builtin frames)
fn builtin_args(cx: &mut Cx, ev: &mut Evaluated<'_>, args: &[(Root<'_>, Sexp)], rho: Root<'_>) -> RResult<()> {
    let dots = prims::state(cx).syms.dots;
    for &(a, tag) in args {
        if a.bind(cx).sexp() == dots {
            dots_values(cx, ev, rho)?;
            continue;
        }
        let o = ev.next(tag);
        let t = a.bind(cx).sexptype();
        if !matches!(t, SYMSXP | LANGSXP) {
            // SAFETY: a constant of the call.
            unsafe { crate::eval::mark_not_mutable(a.bind(cx).sexp()) };
        }
        if a.bind(cx) == cx.missing_arg() {
            o.set(cx, cx.missing_arg());
            continue;
        }
        eval_arg(cx, a, rho, false, o)?;
    }
    Ok(())
}

/// `DODOTS` for a builtin: each element of `...` evaluated, with its tag.
fn dots_values(cx: &mut Cx, ev: &mut Evaluated<'_>, rho: Root<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let dots = prims::state(cx).syms.dots;
        // SAFETY: an immortal symbol.
        let d = s.root(cx, unsafe { Ref::from_sexp(dots, cx) });
        let h = s.out(cx);
        let hr = h.root();
        cx.find_var(d, rho, h)?;
        let t = hr.bind(cx).sexptype();
        if !(t == DOTSXP || t == NILSXP) {
            if hr.bind(cx) != cx.missing_arg() {
                return Err(cx.error("'...' used in an incorrect context"));
            }
            return Ok(());
        }
        while let Some(c) = hr.bind(cx).as_cons() {
            let (car, tag) = (s.root(cx, c.car()), c.tag().sexp());
            let o = ev.next(tag);
            crate::eval::eval(cx, car, rho, o)?;
            let next = s.root(cx, hr.bind(cx).as_cons().unwrap().cdr());
            hr.replace_from(cx, next);
        }
        Ok(())
    })
}

/// `checkForMissings(args, call)`: an empty argument of the call is
/// "argument %d is empty".
// R: eval.c:checkForMissings, signalMissingArgError
fn check_for_missings(cx: &mut Cx, ev: &Evaluated<'_>, call: Root<'_>) -> RResult<()> {
    let missing = cx.missing_arg();
    if !(0..ev.len()).any(|i| ev.root(i).bind(cx) == missing) {
        return Ok(());
    }
    let mut k = 1;
    let mut c = call.bind(cx).as_cons().unwrap().cdr();
    while let Some(cell) = c.as_cons() {
        if cell.car() == missing {
            return Err(cx.error_call(Some(call), &format!("argument {k} is empty")));
        }
        k += 1;
        c = cell.cdr();
    }
    Ok(())
}

/// A closure's promise list as compiled code makes it (`cmpCallArgs`): a
/// call or symbol a promise of byte code, a constant itself (marked not
/// mutable), an empty argument R_MissingArg, `...` a promise of each
/// element (`DODOTS`); tags kept.
// R: eval.c: MAKEPROM, PUSHCONSTARG, DOMISSING, DODOTS (closure frames)
fn closure_args<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, setter: bool, rho: Root<'_>) -> RResult<Root<'s>> {
    let args = args_of(cx, s, call, setter);
    let dots = prims::state(cx).syms.dots;
    let missing = cx.missing_arg().sexp();
    let mut items: Vec<(Root<'s>, Sexp)> = Vec::new();
    for (a, tag) in args {
        let x = a.bind(cx).sexp();
        if x == dots {
            // SAFETY: an immortal symbol and a rooted environment.
            match unsafe { cx.var_lookup_raw(dots, rho.bind(cx).sexp()) } {
                RawBinding::Value(h) if raw(cx, h).sexptype() == DOTSXP || h == cx.nil().sexp() => {
                    let hr = s.root(cx, raw(cx, h));
                    let mut c = hr.bind(cx);
                    let mut elts = Vec::new();
                    while let Some(cell) = c.as_cons() {
                        elts.push((s.root(cx, cell.car()), cell.tag().sexp()));
                        c = cell.cdr();
                    }
                    for (e, t) in elts {
                        if e.bind(cx).sexp() == missing {
                            items.push((e, t));
                        } else {
                            let p = promise(cx, s, e, rho, false)?;
                            items.push((p, t));
                        }
                    }
                }
                RawBinding::Value(h) if h == missing => {}
                _ => return Err(cx.error("'...' used in an incorrect context")),
            }
            continue;
        }
        if x == missing {
            items.push((a, tag));
            continue;
        }
        // SAFETY: a live part of the call.
        match unsafe { x.typ() } {
            SYMSXP | LANGSXP => {
                let p = promise(cx, s, a, rho, true)?;
                items.push((p, tag));
            }
            _ => {
                // SAFETY: as above.
                unsafe { crate::eval::mark_not_mutable(x) };
                items.push((a, tag));
            }
        }
    }
    let o = s.out(cx);
    let l = o.root();
    cx.build(o, |cx| {
        let mut l = cx.nil();
        for (v, t) in items.iter().rev() {
            l = if *t == cx.nil().sexp() {
                cx.cons(v.bind(cx), l)?
            } else {
                // SAFETY: a tag is a symbol (immortal).
                let t = unsafe { Ref::from_sexp(*t, cx) }.as_sym().unwrap();
                cx.cons_tagged(v.bind(cx), l, t)?
            };
        }
        Ok(l)
    })?;
    Ok(l)
}

/// `mkPROMISE(code, rho)`, its code byte code's (`bytecode`).
fn promise<'s>(cx: &mut Cx, s: Scope<'s>, e: Root<'_>, rho: Root<'_>, bytecode: bool) -> RResult<Root<'s>> {
    let p = cx.retry_alloc(|cx| {
        // SAFETY: rooted objects; nothing collects in this interval.
        unsafe {
            let p = crate::args::mk_promise(cx, e.bind(cx).sexp(), rho.bind(cx).sexp())?;
            if bytecode {
                p.set_extra(p.extra() | robigo_heap::extra::BYTECODE);
            }
            Ok(p)
        }
    })?;
    // SAFETY: fresh.
    Ok(s.root(cx, unsafe { Ref::from_sexp(p, cx) }))
}

/// Call the builtin `fun` with `args` for `call`, `R_Visible` as its
/// `PRIMPRINT` says.
fn call_builtin(
    cx: &mut Cx,
    call: Root<'_>,
    fun: Root<'_>,
    args: Args<'_>,
    rho: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    // SAFETY: a rooted primitive of this isolate.
    let def = *unsafe { prims::def(cx, fun.bind(cx).sexp()) };
    let Kind::Builtin(f, _) = def.kind else { unreachable!("robigo: a special called as a builtin") };
    let flag = def.print();
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    f(cx, call, args, rho, out)?;
    if flag < 2 {
        cx.set_visible(flag != 1);
    }
    Ok(())
}
