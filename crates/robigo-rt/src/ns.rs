//! `::` and `:::` (GNU R's `envir.c`): a variable of a namespace by name.
//! Base's namespace is the only one robigo-rt has; another raises "not
//! implemented yet" (`loadNamespace`).

use robigo_heap::types::*;
#[allow(unused_imports)]
use robigo_rtcore::Scope;
use robigo_rtcore::{Cx, Lookup, Out, RResult, Ref, Root, View};

/// `checkArity` for a special of two arguments: the call's argument count.
fn check_arity2(cx: &mut Cx, call: Root<'_>, name: &str) -> RResult<()> {
    let mut n = 0;
    let mut a = call.bind(cx).as_cons().map(|c| c.cdr()).unwrap_or(cx.nil());
    while let Some(c) = a.as_cons() {
        n += 1;
        a = c.cdr();
    }
    if n == 2 {
        return Ok(());
    }
    let msg = format!("{n} argument{} passed to '{name}' which requires 2", if n == 1 { "" } else { "s" });
    Err(cx.error_call(Some(call), &msg))
}

/// The call's `i`th argument expression.
fn arg<'g>(cx: &'g Cx, call: Root<'_>, i: usize) -> Ref<'g> {
    let mut a = call.bind(cx).as_cons().unwrap().cdr();
    for _ in 0..i {
        a = a.as_cons().unwrap().cdr();
    }
    a.as_cons().unwrap().car()
}

/// `checkNSname` / `checkVarName`: a symbol, or the first string as one.
fn as_name<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, x: Root<'_>) -> RResult<Option<Root<'s>>> {
    let v = x.bind(cx);
    match v.view() {
        View::Sym(_) => Ok(Some(s.root(cx, v))),
        _ if v.sexptype() == STRSXP && v.len().unwrap_or(0) >= 1 => {
            let b = v.as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(0).bytes().to_vec();
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.install(&b)?.as_ref()))?;
            Ok(Some(r))
        }
        _ => Ok(None),
    }
}

/// `R_getNSValue(call, ns, name, exported)`.
// R: envir.c:R_getNSValue
fn get_ns_value(
    cx: &mut Cx,
    call: Option<Root<'_>>,
    ns: usize,
    name: usize,
    src: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let nsr = s.root(cx, arg(cx, src, ns));
        let name = s.root(cx, arg(cx, src, name));
        let pkg = match as_name(cx, s, nsr)? {
            Some(p) => p,
            None => return Err(cx.error_call(call, "bad namespace name")),
        };
        let reg = s.root(cx, cx.namespace_registry());
        let nsv = s.out(cx).root();
        let o = nsv.reset(cx);
        cx.find_var_in_frame3(reg, pkg, true, o)?;
        if nsv.bind(cx).sexp() == cx.unbound().sexp() {
            return Err(cx.error("robigo: loadNamespace is not implemented yet"));
        }
        if nsv.bind(cx).sexp() != cx.base_namespace().sexp() {
            return Err(cx.error("robigo: namespaces other than base are not implemented yet"));
        }
        let sym = match as_name(cx, s, name)? {
            Some(p) => p,
            None => return Err(cx.error_call(call, "bad variable name")),
        };
        // base: getVarValInFrame(ns, name, FALSE)
        let res = out.root();
        let base = s.root(cx, cx.base_env());
        let bound = {
            let e = match cx.base_env().view() {
                View::Env(e) => e,
                _ => unreachable!(),
            };
            !matches!(cx.frame_binding(e, sym.bind(cx).as_sym().unwrap()), Lookup::Unbound)
        };
        if !bound {
            let n = String::from_utf8_lossy(sym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
            return Err(cx.error(&format!("object '{n}' not found")));
        }
        cx.find_var_in_frame3(base, sym, true, out)?;
        if res.bind(cx).sexptype() == PROMSXP {
            let p = s.root(cx, res.bind(cx));
            let empty = s.root(cx, cx.empty_env());
            let o = res.reset(cx);
            crate::eval(cx, p, empty, o)?;
        }
        Ok(())
    })
}

/// `pkg::name`: `R_getNSValue(R_NilValue, pkg, name, TRUE)`.
// R: envir.c:do_colon2
pub fn do_colon2(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, "::")?;
    // "use R_NilValue for the call to avoid changing the error message"
    get_ns_value(cx, None, 0, 1, call, out)
}

/// `pkg:::name`: `R_getNSValue(call, pkg, name, FALSE)`.
// R: envir.c:do_colon3
pub fn do_colon3(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, ":::")?;
    get_ns_value(cx, Some(call), 0, 1, call, out)
}
