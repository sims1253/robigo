//! `missing(x)`: GNU R's `R_missing` and `R_isMissing` (envir.c), over the
//! frame's binding cells. C1's argument slots answer the same questions
//! from slot kinds (docs/g3.md §6.3, "missing" rows); this is the pairlist
//! frame's version.

#![allow(unsafe_code)]

use crate::prims;
use crate::vm::Act;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Cx, RResult, RawCell};

/// `MISSING` of a binding cell (the low 4 bits of gp).
///
/// # Safety
/// `cell` is a live binding cell.
unsafe fn missing_bits(cell: Sexp) -> u16 {
    // SAFETY: the caller's contract.
    unsafe { cell.gp() & 15 }
}

/// `findRootPromise(p)`: the innermost promise of a chain of promises.
///
/// # Safety
/// `p` is live.
unsafe fn root_promise(mut p: Sexp) -> Sexp {
    // SAFETY: the caller's contract; PREXPR of a promise is its code.
    unsafe {
        if p.typ() == PROMSXP {
            while p.cdr().typ() == PROMSXP {
                p = p.cdr();
            }
        }
    }
    p
}

/// `nthcdr(s, n)` of a `...` list.
///
/// # Safety
/// `s` is a live list at least `n + 1` long.
unsafe fn nthcdr(mut s: Sexp, n: i64) -> Sexp {
    for _ in 0..n.max(0) {
        // SAFETY: the caller's contract.
        s = unsafe { s.cdr() };
    }
    s
}

/// `R_missing(var, rho)` for `missing()` evaluated in the activation's
/// environment.
// R: envir.c:R_missing
pub fn r_missing(cx: &mut Cx, act: &Act, var: Sexp) -> RResult<bool> {
    let rho = act.rho();
    let st = prims::state(cx);
    let dots = st.syms.dots;
    // SAFETY: a live symbol.
    let dd = unsafe { var.gp() } & robigo_rtcore::DDVAL != 0;
    let (s, ddv) = if dd { (dots, robigo_rtcore::dd_val(crate::eval::sym_name(cx, var).as_bytes())) } else { (var, 0) };
    // SAFETY: an immortal symbol and the rooted environment; the walk reads
    // only, apart from the SETCAR R_isMissing makes (a cell's own value).
    unsafe {
        let t = match cx.frame_cell_raw(rho, s) {
            RawCell::Cell(c) => c,
            RawCell::Base(v) if v != cx.unbound().sexp() => return Ok(false),
            RawCell::Special => {
                return Err(cx.error("robigo: 'missing' in this kind of environment is not implemented yet"));
            }
            _ => {
                // R: "it wasn't an argument to the function"
                let name = crate::eval::sym_name(cx, s);
                return Err(cx.error(&format!("'missing({name})' did not find an argument")));
            }
        };
        let mut t = t;
        if dd {
            let v = t.car();
            let len = if v.typ() == DOTSXP { robigo_list_len(v) } else { 0 };
            if len < ddv || v == cx.missing_arg().sexp() {
                return Ok(true);
            }
            t = nthcdr(v, ddv - 1);
        }
        if missing_bits(t) != 0 || t.car() == cx.missing_arg().sexp() {
            return Ok(true);
        }
        let v = t.car();
        if v.typ() != PROMSXP {
            return Ok(false);
        }
        let v = root_promise(v);
        let expr = v.cdr();
        if expr.typ() != SYMSXP {
            return Ok(false);
        }
        is_missing(cx, expr, v.tag())
    }
}

/// The length of a pairlist.
///
/// # Safety
/// `x` is a live pairlist.
unsafe fn robigo_list_len(mut x: Sexp) -> i64 {
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while matches!(x.typ(), LISTSXP | DOTSXP | LANGSXP) {
            n += 1;
            x = x.cdr();
        }
    }
    n
}

/// `R_isMissing(symbol, rho)`: whether the promise's symbol is a missing
/// argument where the promise was made.
///
/// # Safety
/// `symbol` is live, `rho` a live environment (or R_NilValue), both
/// reachable from the activation's roots.
// R: envir.c:R_isMissing
unsafe fn is_missing(cx: &mut Cx, symbol: Sexp, rho: Sexp) -> RResult<bool> {
    if symbol == cx.missing_arg().sexp() {
        return Ok(true);
    }
    // check for infinite recursion
    cx.check_c_stack()?;
    let st = prims::state(cx);
    let dots = st.syms.dots;
    // SAFETY: the caller's contract.
    unsafe {
        let dd = symbol.gp() & robigo_rtcore::DDVAL != 0;
        let (s, ddv) =
            if dd { (dots, robigo_rtcore::dd_val(crate::eval::sym_name(cx, symbol).as_bytes())) } else { (symbol, 0) };
        if rho == cx.base_env().sexp() || rho == cx.base_namespace().sexp() || rho.typ() != ENVSXP {
            return Ok(false);
        }
        let mut vl = match cx.frame_cell_raw(rho, s) {
            RawCell::Cell(c) => c,
            _ => return Ok(false),
        };
        if dd {
            let v = vl.car();
            let len = if v.typ() == DOTSXP { robigo_list_len(v) } else { 0 };
            if len < ddv || v == cx.missing_arg().sexp() {
                return Ok(true);
            }
            vl = nthcdr(v, ddv - 1);
        }
        if missing_bits(vl) == 1 || vl.car() == cx.missing_arg().sexp() {
            return Ok(true);
        }
        if vl.gp() & (1 << 15) != 0 {
            // an active binding
            return Ok(false);
        }
        // SETCAR(vl, findRootPromise(CAR(vl)))
        let rp = root_promise(vl.car());
        if rp != vl.car() {
            vl.set_car(rp);
        }
        let p = vl.car();
        if p.typ() == PROMSXP && p.car() == cx.unbound().sexp() && p.cdr().typ() == SYMSXP {
            // PRSEEN detects cycles: a cycle is a missing argument
            if p.gp() == 1 {
                return Ok(true);
            }
            let oldseen = p.gp();
            p.set_gp(1);
            // (the cell keeps `p` alive; R_isMissing reads, it runs no code)
            let r = is_missing(cx, p.cdr(), p.tag());
            p.set_gp(oldseen);
            return r;
        }
        Ok(false)
    }
}
