//! `missing(x)`: GNU R's `R_missing` and `R_isMissing` (envir.c), over a
//! frame's bindings as the raw layer reads them ([`Cx::frame_arg_raw`]):
//! `MISSING` bits, and what an argument holds (a value, a promise, a
//! delayed or forced argument slot) without materializing it.

#![allow(unsafe_code)]

use crate::prims;
use crate::vm::Act;
use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{ArgValue, Cx, RResult, RawArg};

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

/// The length of a pairlist.
///
/// # Safety
/// `x` is a live object.
unsafe fn list_len(mut x: Sexp) -> i64 {
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

/// What one binding (or a `...` element) says about missingness:
/// `MISSING`, whether it is active, and its value.
#[derive(Clone, Copy)]
struct Seen {
    missing: u16,
    active: bool,
    value: ArgValue,
}

/// The binding of `var` (a `..N` symbol: the N-th element of `...`) in
/// `rho`'s frame. Ok(None): no binding; Ok(Some(None)): a `..N` past the
/// end of `...` (missing).
///
/// # Safety
/// `var` a live symbol, `rho` a live environment, both rooted.
unsafe fn binding(cx: &mut Cx, rho: Sexp, var: Sexp) -> RResult<Option<Option<Seen>>> {
    let dots = prims::state(cx).syms.dots;
    // SAFETY: the caller's contract.
    unsafe {
        let dd = var.gp() & robigo_rtcore::DDVAL != 0;
        let (s, ddv) =
            if dd { (dots, robigo_rtcore::dd_val(crate::eval::sym_name(cx, var).as_bytes())) } else { (var, 0) };
        let b = match cx.frame_arg_raw(rho, s) {
            RawArg::Bound { missing, active, value } => Seen { missing, active, value },
            RawArg::Base => return Ok(Some(Some(Seen { missing: 0, active: false, value: ArgValue::Scalar }))),
            RawArg::Unbound => return Ok(None),
            RawArg::Special => {
                return Err(cx.error("robigo: 'missing' in this kind of environment is not implemented yet"));
            }
        };
        if !dd {
            return Ok(Some(Some(b)));
        }
        // the N-th cell of `...`: its MISSING and CAR
        let v = match b.value {
            ArgValue::Obj(v) => v,
            _ => return Ok(Some(Some(Seen { missing: 0, active: false, value: ArgValue::Scalar }))),
        };
        let len = if v.typ() == DOTSXP { list_len(v) } else { 0 };
        if len < ddv || v == cx.missing_arg().sexp() {
            return Ok(Some(None));
        }
        let mut c = v;
        for _ in 1..ddv {
            c = c.cdr();
        }
        Ok(Some(Some(Seen { missing: c.gp() & 15, active: false, value: ArgValue::Obj(c.car()) })))
    }
}

/// `R_missing(var, rho)` for `missing()` evaluated in the activation's
/// environment.
// R: envir.c:R_missing
pub fn r_missing(cx: &mut Cx, act: &Act, var: Sexp) -> RResult<bool> {
    let rho = act.rho();
    // SAFETY: an immortal symbol and the rooted environment.
    let b = match unsafe { binding(cx, rho, var)? } {
        None => {
            // R: "it wasn't an argument to the function"
            let name = crate::eval::sym_name(cx, var);
            let name = name.strip_prefix("..").map_or(name.clone(), |_| "...".to_owned());
            return Err(cx.error(&format!("'missing({name})' did not find an argument")));
        }
        Some(None) => return Ok(true),
        Some(Some(b)) => b,
    };
    let missing_arg = cx.missing_arg().sexp();
    if b.missing != 0 || b.value == ArgValue::Obj(missing_arg) {
        return Ok(true);
    }
    // deal with the promise
    let mut seen = Vec::new();
    // SAFETY: values of a rooted frame.
    unsafe { promise_missing(cx, b.value, &mut seen) }
}

/// The promise half of `R_missing` and `R_isMissing`: an unforced
/// promise (or delayed argument) whose code is a symbol is missing if that
/// symbol is a missing argument where the promise was made.
///
/// # Safety
/// The value's objects are live and reachable from a rooted frame.
unsafe fn promise_missing(cx: &mut Cx, value: ArgValue, seen: &mut Vec<(usize, usize)>) -> RResult<bool> {
    // SAFETY: the caller's contract.
    unsafe {
        let (expr, env) = match value {
            ArgValue::Obj(p) if p.typ() == PROMSXP => {
                let p = root_promise(p);
                // R_missing looks at any promise; R_isMissing at unforced
                // ones only, which `R_isMissing(sym, R_NilValue)` (a forced
                // promise's PRENV) also answers FALSE
                (p.cdr(), p.tag())
            }
            ArgValue::Delayed { expr, env } => (expr, env),
            ArgValue::Forced { expr } => (expr, cx.nil().sexp()),
            _ => return Ok(false),
        };
        if expr.typ() != SYMSXP {
            return Ok(false);
        }
        is_missing(cx, expr, env, seen)
    }
}

/// `R_isMissing(symbol, rho)`. GNU R marks a promise PRSEEN while it looks
/// through it, and a cycle answers TRUE; here the bindings looked through
/// are remembered instead, so no argument slot is written.
///
/// # Safety
/// `symbol` live, `rho` live (an environment or R_NilValue), reachable from
/// a rooted frame.
// R: envir.c:R_isMissing
unsafe fn is_missing(cx: &mut Cx, symbol: Sexp, rho: Sexp, seen: &mut Vec<(usize, usize)>) -> RResult<bool> {
    let missing_arg = cx.missing_arg().sexp();
    if symbol == missing_arg {
        return Ok(true);
    }
    // check for infinite recursion
    cx.check_c_stack()?;
    // SAFETY: the caller's contract.
    unsafe {
        if rho == cx.base_env().sexp() || rho == cx.base_namespace().sexp() || rho.typ() != ENVSXP {
            return Ok(false);
        }
        let b = match binding(cx, rho, symbol)? {
            None => return Ok(false),
            Some(None) => return Ok(true),
            Some(Some(b)) => b,
        };
        if b.missing == 1 || b.value == ArgValue::Obj(missing_arg) {
            return Ok(true);
        }
        if b.active {
            return Ok(false);
        }
        let unforced = match b.value {
            ArgValue::Obj(p) => {
                let p = root_promise(p);
                p.typ() == PROMSXP && p.car() == cx.unbound().sexp()
            }
            ArgValue::Delayed { .. } => true,
            _ => false,
        };
        if !unforced {
            return Ok(false);
        }
        let key = (rho.0 as usize, symbol.0 as usize);
        if seen.contains(&key) {
            return Ok(true);
        }
        seen.push(key);
        let r = promise_missing(cx, b.value, seen);
        seen.pop();
        r
    }
}

/// `R_isMissing(symbol, rho)` for the specials' argument lists
/// (`evalListKeepMissing`).
///
/// # Safety
/// `symbol` a live symbol and `rho` a live environment, reachable from
/// roots.
pub unsafe fn r_is_missing(cx: &mut Cx, symbol: Sexp, rho: Sexp) -> RResult<bool> {
    let mut seen = Vec::new();
    // SAFETY: the caller's contract.
    unsafe { is_missing(cx, symbol, rho, &mut seen) }
}
