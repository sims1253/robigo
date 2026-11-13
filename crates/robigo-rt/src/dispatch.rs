//! S3 dispatch's hand-off points (S1's package, objects.c): `UseMethod`
//! and the internal-generic dispatch of the specials `[`, `[[`, `$` and
//! their setters (`DispatchOrEval` with the first argument evaluated into a
//! promise). Until S1 lands, each raises "not implemented yet" (AGENTS
//! rule 6): an object reaching them must not silently take the default.

use robigo_rtcore::{Cx, Out, RResult, Root};

/// `do_usemethod(call, op, args, env)`: `UseMethod(generic, object)`.
/// S1 ports objects.c's `usemethod`; a method's value leaves the generic's
/// closure by `findcontext(CTXT_RETURN, env, ans)`, which the VM already
/// lands at the closure's function record.
// R: objects.c:do_usemethod
pub fn do_usemethod(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, _out: Out<'_>) -> RResult<()> {
    // (the loader's stubs' text, against the call)
    Err(cx.error_call(Some(call), "robigo: the primitive 'UseMethod' is not implemented yet"))
}

/// `DispatchOrEval(call, op, generic, args, rho, &ans, 0, 0)` for a
/// special whose first argument has been evaluated to an object:
/// `first` holds the promise GNU R makes for it (`R_mkEVPROMISE_NR(expr,
/// value)`), `rest` the call's other argument expressions. `Ok(true)`: a
/// method ran and wrote `out`; `Ok(false)`: no method applies, and the
/// caller evaluates the rest and runs the default (`out` untouched).
// R: eval.c:DispatchOrEval
pub fn dispatch_special(
    cx: &mut Cx,
    generic: &'static str,
    _call: Root<'_>,
    _first: Root<'_>,
    _rest: Root<'_>,
    _rho: Root<'_>,
    _out: &Out<'_>,
) -> RResult<bool> {
    let msg = format!("robigo: S3 dispatch of '{generic}' on an object is not implemented yet");
    Err(cx.error(&msg))
}
