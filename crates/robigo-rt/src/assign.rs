//! Complex assignment (eval.c's `applydefine` and `evalseq`): `f(x) <-
//! value`, `x[i] <- value`, `names(x)[2] <- value`, `x$a <<- value`.
//!
//! As GNU R's AST interpreter does it (docs/g3.md §4: "`applydefine`/
//! `evalseq` (with `DISABLE_REFCNT` on `*tmp*`) ... ported as written"):
//! the target is made local (`EnsureLocal`), the getter calls are
//! evaluated with `*tmp*` bound to the value one level in, then each
//! replacement function is called as `` `f<-`(`*tmp*`, args..., value =
//! <promise>) `` (marked an assignment call), inside a `CTXT_CCODE` record
//! whose call is the assignment (so `error()` reports it) and whose
//! cleanup removes `*tmp*`. The right-hand side is evaluated first, by the
//! compiled code; the values on the stack hold links while the assignment
//! runs ([`crate::links`]).
//!
//! The calls built here are new each time, so they are evaluated without
//! compiling them ([`crate::call::eval_call`]); their arguments are the
//! source's expressions, compiled once.

#![allow(unsafe_code)]

use crate::apply::eval_call;
use crate::eval::eval;
use crate::ir::{R_ENV, Reg};
use crate::links;
use crate::prims;
use crate::vm::Act;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, Left, RResult, Ref, Root, Scope, View, ctxt};

/// `ASSIGNBUFSIZ`: the longest assignment function name, with `<-` and NUL.
const ASSIGNBUFSIZ: usize = 32;

/// `getAssignFcnSymbol(fun)`: `[<-`, `[[<-`, `$<-` directly; any other
/// `f` as `f<-`, interned once (`R_ReplaceFunsTable`).
// R: eval.c:getAssignFcnSymbol, installAssignFcnSymbol
fn assign_fcn_symbol(cx: &mut Cx, fun: Sexp) -> RResult<Sexp> {
    let st = prims::state(cx);
    let sy = &st.syms;
    if fun == sy.subset {
        return Ok(sy.subassign);
    } else if fun == sy.subset2 {
        return Ok(sy.subassign2);
    } else if fun == sy.dollar {
        return Ok(sy.dollar_gets);
    }
    let (id, name) = {
        // SAFETY: an immortal symbol.
        let sym = unsafe { Ref::from_sexp(fun, cx) }.as_sym().unwrap();
        (sym.id(), sym.name().bytes().to_vec())
    };
    if let Some(&v) = st.assign_fns.borrow().get(&id) {
        return Ok(v);
    }
    if name.len() + 3 > ASSIGNBUFSIZ {
        let msg = format!("overlong name in '{}'", String::from_utf8_lossy(&name));
        return Err(cx.error(&msg));
    }
    let mut buf = name;
    buf.extend_from_slice(b"<-");
    let v = cx.retry_alloc(|cx| cx.install(&buf).map(|y| y.as_ref().sexp()))?;
    prims::state(cx).assign_fns.borrow_mut().insert(id, v);
    Ok(v)
}

/// The replacement function of an assignment target's call head `head`
/// (a symbol, or `pkg::f` / `pkg:::f`), rooted in `s`.
// R: eval.c:applydefine (the head cases)
fn replacement_fun<'s>(cx: &mut Cx, s: Scope<'s>, head: Root<'_>) -> RResult<Root<'s>> {
    let h = head.bind(cx);
    if h.sexptype() == SYMSXP {
        let f = assign_fcn_symbol(cx, h.sexp())?;
        // SAFETY: an immortal symbol.
        return Ok(s.root(cx, unsafe { Ref::from_sexp(f, cx) }));
    }
    let syms = &prims::state(cx).syms;
    let (dc, tc) = (syms.double_colon, syms.triple_colon);
    if let Some(c) = h.as_cons()
        && h.sexptype() == LANGSXP
        && (c.car().sexp() == dc || c.car().sexp() == tc)
        && h.len() == Some(3)
        && let Some(c2) = c.cdr().as_cons()
        && let Some(c3) = c2.cdr().as_cons()
        && c3.car().sexptype() == SYMSXP
    {
        // (the package part is rooted: interning below may collect)
        let (op, pkg, name) = (c.car().sexp(), s.root(cx, c2.car()), c3.car().sexp());
        let f = assign_fcn_symbol(cx, name)?;
        let r = cx.retry_alloc(|cx| {
            let nil = cx.nil().sexp();
            // SAFETY: immortal symbols and a part of the rooted call;
            // nothing collects in this interval.
            unsafe {
                let l3 = cx.alloc_node_raw(LISTSXP, f, nil, nil)?;
                let l2 = cx.alloc_node_raw(LISTSXP, pkg.bind(cx).sexp(), l3, nil)?;
                cx.alloc_node_raw(LANGSXP, op, l2, nil)
            }
        })?;
        // SAFETY: fresh.
        return Ok(s.root(cx, unsafe { Ref::from_sexp(r, cx) }));
    }
    Err(cx.error("invalid function in complex assignment"))
}

/// `replaceCall(fun, val, args, rhs)`: `fun(val, args..., value = rhs)`,
/// marked an assignment call.
// R: eval.c:replaceCall
fn replace_call<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    fun: Root<'_>,
    val: Sexp,
    args: Root<'_>,
    rhs: Root<'_>,
) -> RResult<Root<'s>> {
    let value = prims::state(cx).syms.value;
    let c = cx.retry_alloc(|cx| {
        let nil = cx.nil().sexp();
        // SAFETY: rooted objects and immortal symbols; nothing collects in
        // this interval.
        unsafe {
            let mut items = Vec::new();
            let mut a = args.bind(cx).sexp();
            while a != nil {
                items.push((a.car(), a.tag()));
                a = a.cdr();
            }
            let mut l = cx.alloc_node_raw(LISTSXP, rhs.bind(cx).sexp(), nil, value)?;
            for &(car, tag) in items.iter().rev() {
                l = cx.alloc_node_raw(LISTSXP, car, l, tag)?;
            }
            l = cx.alloc_node_raw(LISTSXP, val, l, nil)?;
            cx.alloc_node_raw(LANGSXP, fun.bind(cx).sexp(), l, nil)
        }
    })?;
    // SAFETY: fresh.
    let r = s.root(cx, unsafe { Ref::from_sexp(c, cx) });
    cx.mark_assignment_call(r);
    Ok(r)
}

/// `R_mkEVPROMISE_NR(expr, value)`: the right-hand side's promise, which
/// counts no references.
fn rhs_promise<'s>(cx: &mut Cx, s: Scope<'s>, expr: Root<'_>, value: Root<'_>) -> RResult<Root<'s>> {
    let p = cx.retry_alloc(|cx| {
        let (unbound, nil) = (cx.unbound().sexp(), cx.nil().sexp());
        // SAFETY: a rooted part of the call and a rooted value, read in this
        // attempt; nothing collects in this interval.
        unsafe {
            let p = cx.alloc_node_raw(PROMSXP, unbound, expr.bind(cx).sexp(), nil)?;
            p.set_flag(robigo_heap::flags::NO_TRACKREFS, true);
            p.set_car(value.bind(cx).sexp());
            Ok(p)
        }
    })?;
    // SAFETY: fresh.
    Ok(s.root(cx, unsafe { Ref::from_sexp(p, cx) }))
}

/// The enclosure of the environment in `rho`.
fn enclos<'s>(cx: &Cx, s: Scope<'s>, rho: Root<'_>) -> Root<'s> {
    match rho.bind(cx).view() {
        View::Env(e) => s.root(cx, e.enclos()),
        _ => unreachable!("robigo: not an environment"),
    }
}

/// `x` replaced by `shallow_duplicate(x)` in its root.
fn dup<'s>(cx: &mut Cx, s: Scope<'s>, x: Root<'s>) -> RResult<()> {
    let o = s.out(cx);
    let d = o.root();
    cx.shallow_duplicate(x, o)?;
    x.replace_from(cx, d);
    Ok(())
}

/// `EnsureLocal(symbol, rho, &loc)`: the target's value, copied into
/// `rho`'s frame if it is not there or may be shared; and its binding cell.
// R: eval.c:EnsureLocal
fn ensure_local<'s>(cx: &mut Cx, s: Scope<'s>, sym: Root<'_>, rho: Root<'_>) -> RResult<(Root<'s>, Root<'s>)> {
    let o = s.out(cx);
    let f = o.root();
    cx.find_var_in_frame3(rho, sym, true, o)?;
    let vl = s.out(cx);
    let vr = vl.root();
    if f.bind(cx) != cx.unbound() {
        // (for promises)
        eval(cx, sym, rho, vl)?;
        if vr.bind(cx).maybe_shared() {
            // GNU R's R_shallow_duplicate_attr may defer the data's copy in
            // an ALTREP wrapper; the copy is made here
            dup(cx, s, vr)?;
            cx.define_var(sym, vr, rho)?;
        }
    } else {
        let en = enclos(cx, s, rho);
        eval(cx, sym, en, vl)?;
        if vr.bind(cx) == cx.unbound() {
            let msg = format!("object '{}' not found", crate::eval::sym_name(cx, sym.bind(cx).sexp()));
            return Err(cx.error(&msg));
        }
        dup(cx, s, vr)?;
        cx.define_var(sym, vr, rho)?;
    }
    let o = s.out(cx);
    let cell = o.root();
    if !cx.find_var_loc_in_frame(rho, sym, o)? {
        cell.reset(cx).set(cx, cx.nil());
    }
    Ok((vr, cell))
}

/// `R_findVarLoc(sym, rho)`: the binding cell of `sym` from `rho` out
/// (R_NilValue for none).
// R: envir.c:R_findVarLoc
fn find_var_loc<'s>(cx: &mut Cx, s: Scope<'s>, sym: Root<'_>, rho: Root<'_>) -> RResult<Root<'s>> {
    let cur = s.root(cx, rho.bind(cx));
    loop {
        if cur.bind(cx) == cx.empty_env() || cur.bind(cx).sexptype() != ENVSXP {
            return Ok(s.root(cx, cx.nil()));
        }
        let o = s.out(cx);
        let cell = o.root();
        if cx.find_var_loc_in_frame(cur, sym, o)? {
            return Ok(cell);
        }
        let next = enclos(cx, s, cur);
        cur.replace_from(cx, next);
    }
}

/// `evalseq(expr, rho, forcelocal, tmploc, &ploc)`: the values of the
/// target's nested parts, innermost (the symbol's) first, and the target
/// symbol; the getter calls evaluated with `*tmp*` (`tmploc`) bound to the
/// value one level in. `ploc` is the target's binding cell.
// R: eval.c:evalseq
#[allow(clippy::too_many_arguments)]
fn evalseq<'s>(
    cx: &mut Cx,
    s: Scope<'s>,
    expr: Root<'_>,
    rho: Root<'_>,
    forcelocal: bool,
    tmploc: Root<'_>,
    ploc: &mut Option<Root<'s>>,
    vals: &mut Vec<Root<'s>>,
) -> RResult<Root<'s>> {
    let e = expr.bind(cx);
    match e.sexptype() {
        NILSXP => Err(cx.error("invalid (NULL) left side of assignment")),
        SYMSXP => {
            let sym = s.root(cx, e);
            let (nval, cell) = if forcelocal {
                ensure_local(cx, s, sym, rho)?
            } else {
                let en = enclos(cx, s, rho);
                let o = s.out(cx);
                let v = o.root();
                eval(cx, sym, en, o)?;
                let cell = find_var_loc(cx, s, sym, en)?;
                (v, cell)
            };
            let has_cell = cell.bind(cx).sexptype() != NILSXP;
            let maybe_in_assign = has_cell && cx.assignment_pending(cell.bind(cx));
            if has_cell {
                cx.set_assignment_pending(cell, true);
            }
            if maybe_in_assign || nval.bind(cx).maybe_shared() {
                dup(cx, s, nval)?;
            }
            *ploc = Some(cell);
            vals.push(nval);
            Ok(sym)
        }
        LANGSXP => {
            let c = e.as_cons().unwrap();
            let (head, rest) = (s.root(cx, c.car()), c.cdr().as_cons());
            let Some(inner) = rest else {
                return Err(cx.error("invalid (NULL) left side of assignment"));
            };
            let inner_expr = s.root(cx, inner.car());
            let args = s.root(cx, inner.cdr());
            let sym = evalseq(cx, s, inner_expr, rho, forcelocal, tmploc, ploc, vals)?;
            let inner_val = *vals.last().unwrap();
            cx.set_var_loc_value(tmploc, inner_val)?;
            // nexpr = LCONS(CAR(expr), LCONS(*tmp*, CDDR(expr)))
            let tmpval = prims::state(cx).syms.tmpval;
            let n = cx.retry_alloc(|cx| {
                let nil = cx.nil().sexp();
                // SAFETY: rooted parts of the call and an immortal symbol;
                // nothing collects in this interval.
                unsafe {
                    let l = cx.alloc_node_raw(LISTSXP, tmpval, args.bind(cx).sexp(), nil)?;
                    cx.alloc_node_raw(LANGSXP, head.bind(cx).sexp(), l, nil)
                }
            })?;
            // SAFETY: fresh.
            let nexpr = s.root(cx, unsafe { Ref::from_sexp(n, cx) });
            let o = s.out(cx);
            let nval = o.root();
            eval_call(cx, nexpr, rho, o)?;
            // duplicate if it may be shared, or if the container has become
            // possibly shared by going through a closure
            let nv = nval.bind(cx);
            if nv.maybe_referenced() && (nv.maybe_shared() || inner_val.bind(cx).maybe_shared()) {
                dup(cx, s, nval)?;
            }
            vals.push(nval);
            Ok(sym)
        }
        _ => Err(cx.error("target of assignment expands to non-language object")),
    }
}

/// `SET_TEMPVARLOC_FROM_CAR(loc, lhs)`: `*tmp*` bound to the value (copied
/// first if it may be shared).
fn set_tempvarloc(cx: &mut Cx, s: Scope<'_>, tmploc: Root<'_>, v: Root<'_>) -> RResult<()> {
    if v.bind(cx).maybe_shared() {
        let o = s.out(cx);
        let d = o.root();
        cx.shallow_duplicate(v, o)?;
        v.replace_from(cx, d);
    }
    cx.set_var_loc_value(tmploc, v)
}

/// `applydefine(call, op, args, rho)` for the assignment `call` (`code`:
/// do_set's, 1 `<-`, 2 `<<-`, 3 `=`), the right-hand side's value in
/// register `rhs`: the value assigned is in register `dst`, invisible.
// R: eval.c:applydefine
pub fn applydefine(cx: &mut Cx, act: &Act, dst: Reg, rhs: Reg, call: Sexp, code: u16) -> RResult<()> {
    cx.scope(|cx, s| {
        // SAFETY: a constant of the rooted code; a register's value.
        let callr = s.root(cx, unsafe { Ref::from_sexp(call, cx) });
        let rho = act.root(R_ENV);
        let saverhs = {
            let v = crate::vm::boxed(cx, act, rhs)?;
            // SAFETY: a register's live object.
            s.root(cx, unsafe { Ref::from_sexp(v, cx) })
        };
        act.clear(rhs);
        // the stack's values hold links meanwhile (R_BCProtTop as it was)
        let old = links::commit(cx);
        let r = applydefine_body(cx, s, callr, rho, saverhs, code);
        links::release(cx, old);
        r?;
        cx.set_visible(false);
        act.out(dst).set(cx, saverhs.bind(cx));
        Ok(())
    })
}

/// [`applydefine`] once the links are committed.
fn applydefine_body(
    cx: &mut Cx,
    s: Scope<'_>,
    call: Root<'_>,
    rho: Root<'_>,
    saverhs: Root<'_>,
    code: u16,
) -> RResult<()> {
    // the right-hand side holds a reference while the assignment runs
    // (`if (refrhs) INCREMENT_REFCNT(saverhs)`), dropped on every path out
    let refrhs = saverhs.bind(cx).maybe_referenced().then(|| {
        let p = saverhs.as_ptr();
        // SAFETY: a slot of the caller's scope, which outlives the guard,
        // holding the value.
        unsafe {
            let mut l = links::Linked::slots(p);
            l.link_next();
            l
        }
    });
    let r = rho.bind(cx);
    if r == cx.base_namespace() {
        return Err(cx.error_call(Some(call), "cannot do complex assignments in base namespace"));
    }
    if r == cx.base_env() {
        return Err(cx.error_call(Some(call), "cannot do complex assignments in base environment"));
    }
    let tmpval = prims::state(cx).syms.tmpval;
    // SAFETY: an immortal symbol.
    let tmpsym = s.root(cx, unsafe { Ref::from_sexp(tmpval, cx) });
    let o = s.out(cx);
    let tmploc = o.root();
    let old_tmpval = if cx.find_var_loc_in_frame(rho, tmpsym, o)? {
        let o = s.out(cx);
        let v = o.root();
        cx.var_loc_value(tmploc, o)?;
        if cx.binding_is_locked(tmpsym, rho)? {
            return Err(cx.error("existing `*tmp*` binding is locked"));
        }
        if cx.binding_is_active(tmpsym, rho)? {
            return Err(cx.error("existing `*tmp*` binding is an active binding"));
        }
        Some(v)
    } else {
        let nil = s.root(cx, cx.nil());
        cx.define_var(tmpsym, nil, rho)?;
        let o = tmploc.reset(cx);
        cx.find_var_loc_in_frame(rho, tmpsym, o)?;
        if tmploc.bind(cx).sexptype() == LISTSXP {
            cx.disable_binding_refcnt(tmploc);
        }
        None
    };
    // a CTXT_CCODE record, so that error() reports the assignment; its
    // cleanup (tmp_cleanup) removes `*tmp*` when an exit passes it
    let base = s.root(cx, cx.base_env());
    let body = cx.with_context(ctxt::CCODE, call, base, base, None, |cx, _id| {
        cx.scope(|cx, s| assign_in_context(cx, s, call, rho, saverhs, tmploc, code))
    });
    match body {
        Ok(()) => {}
        Err(Left::Exit(e)) => {
            // tmp_cleanup
            cx.unbind_var(tmpsym, rho)?;
            return Err(e);
        }
        Err(Left::Landed(_)) => unreachable!("robigo: an exit landed at complex assignment's record"),
    }
    match old_tmpval {
        None => cx.unbind_var(tmpsym, rho)?,
        Some(v) => cx.set_var_loc_value(tmploc, v)?,
    }
    if let Some(l) = refrhs {
        l.release();
    }
    Ok(())
}

/// The part of `applydefine` inside its record: `evalseq`, the
/// replacement calls from the outside in, the final assignment.
fn assign_in_context(
    cx: &mut Cx,
    s: Scope<'_>,
    call: Root<'_>,
    rho: Root<'_>,
    rhs: Root<'_>,
    tmploc: Root<'_>,
    code: u16,
) -> RResult<()> {
    // expr = CAR(args): the target; CADR(args): the rhs expression
    let (target, rhs_expr) = {
        let c = call.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap();
        (c.car(), c.cdr().as_cons().unwrap().car())
    };
    // (rooted: evalseq below may collect)
    let rhs_expr = s.root(cx, rhs_expr);
    let mut expr = s.root(cx, target);
    let mut vals = Vec::new();
    let mut lhsloc = None;
    let inner = s.root(cx, expr.bind(cx).as_cons().unwrap().cdr().as_cons().map_or(cx.nil(), |c| c.car()));
    let lhs_sym = evalseq(cx, s, inner, rho, code == 1 || code == 3, tmploc, &mut lhsloc, &mut vals)?;
    let rhsprom = rhs_promise(cx, s, rhs_expr, rhs)?;
    let tmpval = prims::state(cx).syms.tmpval;
    // the outermost value is the last pushed
    let mut lhs = vals.len();
    loop {
        let (head, inner_is_call, args) = {
            let c = expr.bind(cx).as_cons().unwrap();
            let rest = c.cdr().as_cons();
            (
                s.root(cx, c.car()),
                rest.is_some_and(|r| r.car().sexptype() == LANGSXP),
                s.root(cx, rest.map_or(cx.nil(), |r| r.cdr())),
            )
        };
        let fun = replacement_fun(cx, s, head)?;
        lhs -= 1;
        set_tempvarloc(cx, s, tmploc, vals[lhs])?;
        let rcall = replace_call(cx, s, fun, tmpval, args, rhsprom)?;
        if !inner_is_call {
            // the last replacement: its value is assigned to the target
            let o = s.out(cx);
            let value = o.root();
            eval_call(cx, rcall, rho, o)?;
            if let Some(cell) = lhsloc {
                cx.set_assignment_pending(cell, false);
            }
            if code == 2 {
                let en = enclos(cx, s, rho);
                cx.set_var(lhs_sym, value, en)?;
            } else {
                cx.define_var(lhs_sym, value, rho)?;
            }
            cx.set_visible(false);
            return Ok(());
        }
        let o = s.out(cx);
        let v = o.root();
        eval_call(cx, rcall, rho, o)?;
        // SET_PRVALUE(rhsprom, rhs); SET_PRCODE(rhsprom, rhs): stores into a
        // promise that counts nothing
        // SAFETY: the rooted promise and a rooted value.
        unsafe {
            let p = rhsprom.bind(cx).sexp();
            let x = v.bind(cx).sexp();
            p.set_car(x);
            p.set_cdr(x);
        }
        let next = s.root(cx, expr.bind(cx).as_cons().unwrap().cdr().as_cons().unwrap().car());
        expr = next;
    }
}
