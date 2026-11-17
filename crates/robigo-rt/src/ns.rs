//! Namespaces (L2): GNU R's `envir.c` namespace entries over the
//! isolate's namespace registry (`R_NamespaceRegistry`, a hashed
//! environment binding each namespace's name to it; base's from the
//! start): `registerNamespace`, `unregisterNamespace`,
//! `getRegisteredNamespace`, `isRegisteredNamespace`,
//! `getNamespaceRegistry`, `isNamespaceEnv`, `importIntoEnv`, `topenv`,
//! `getNamespaceValue`, `::` and `:::` (`R_getNSValue`, which calls base's
//! `loadNamespace` for a namespace not loaded yet), and version.c's
//! `internalsID`.
//!
//! What a namespace is, as GNU R tests it (`R_IsNamespaceEnv`): base's, or
//! an environment whose `.__NAMESPACE__.` binding is an environment with a
//! `spec` binding that is a non-empty character vector. A package
//! environment (`R_IsPackageEnv`) has a `name` attribute starting with
//! "package:". Both read bindings without running code: an active binding
//! or an argument slot in those places is taken as absent.

#![allow(unsafe_code)]

use robigo_heap::Sexp;
use robigo_heap::types::*;
use robigo_rtcore::{Args, BindingValue, Cx, Encoding, Lookup, Out, RResult, RSym, Ref, Root, Scope, View};

/// `R_INTERNALS_UUID` (Defn.h).
const R_INTERNALS_UUID: &[u8] = b"2fdf6c18-697a-4ba7-b8ef-11c0d92f1327";

// ---- what a namespace is ----------------------------------------------------

/// The object `rho`'s frame binds `name` to, read without running code
/// (`R_findVarInFrame` of a plain binding), if any.
fn frame_value(cx: &Cx, rho: Sexp, name: &[u8]) -> Option<Sexp> {
    let sym = cx.find_symbol(name)?;
    // SAFETY: a live object (the caller's).
    let r = unsafe { Ref::from_sexp(rho, cx) };
    let View::Env(e) = r.view() else { return None };
    match cx.frame_binding(e, sym) {
        Lookup::Bound(b) => match b.get(cx) {
            BindingValue::Value(v) => Some(v.sexp()),
            _ => None,
        },
        _ => None,
    }
}

/// `R_NamespaceEnvSpec(rho)` for a namespace other than base's: the
/// `spec` of its `.__NAMESPACE__.`, if `rho` is one.
// R: envir.c:R_IsNamespaceEnv, R_NamespaceEnvSpec
pub(crate) fn namespace_spec(cx: &Cx, rho: Sexp) -> Option<Sexp> {
    // SAFETY: live objects (the caller's and the bindings').
    unsafe {
        if rho.typ() != ENVSXP {
            return None;
        }
        let info = frame_value(cx, rho, b".__NAMESPACE__.")?;
        if info.typ() != ENVSXP {
            return None;
        }
        let spec = frame_value(cx, info, b"spec")?;
        (spec.typ() == STRSXP && spec.std_len() > 0).then_some(spec)
    }
}

/// `R_IsNamespaceEnv(rho)`.
// R: envir.c:R_IsNamespaceEnv
pub(crate) fn is_namespace_env(cx: &Cx, rho: Sexp) -> bool {
    rho == cx.base_namespace().sexp() || namespace_spec(cx, rho).is_some()
}

/// The `name` attribute of `rho`, if it is a non-empty STRSXP.
pub(crate) fn name_attr(cx: &Cx, rho: Sexp) -> Option<Sexp> {
    let name_sym = cx.find_symbol(b"name")?.as_ref().sexp();
    // SAFETY: a live object and its attribute list.
    unsafe {
        let mut a = rho.attrib_raw();
        while a != cx.nil().sexp() {
            if a.tag() == name_sym {
                let v = a.car();
                return (v.typ() == STRSXP && v.std_len() > 0).then_some(v);
            }
            a = a.cdr();
        }
    }
    None
}

/// `R_IsPackageEnv(rho)`: its `name` attribute starts with "package:".
// R: envir.c:R_IsPackageEnv
pub(crate) fn is_package_env(cx: &Cx, rho: Sexp) -> bool {
    // SAFETY: live objects.
    unsafe {
        rho.typ() == ENVSXP
            && name_attr(cx, rho)
                .is_some_and(|n| (*(n.std_data() as *const Sexp)).char_bytes().starts_with(b"package:"))
    }
}

// ---- argument checks -----------------------------------------------------------

/// `checkNSname(call, name)` / `checkVarName(call, name)`: a symbol, or
/// the first string as one; otherwise `errorcall(call, msg)`.
// R: envir.c:checkNSname, checkVarName
fn check_name<'s>(cx: &mut Cx, s: Scope<'s>, call: Option<Root<'_>>, x: Root<'_>, msg: &str) -> RResult<Root<'s>> {
    let v = x.bind(cx);
    match v.sexptype() {
        SYMSXP => Ok(s.root(cx, v)),
        STRSXP if v.len().unwrap_or(0) >= 1 => install_first_at(cx, s, x, 0),
        _ => Err(cx.error_call(call, msg)),
    }
}

/// `R_findVarInFrame(rho, sym)`, written to `out` (R_UnboundValue when
/// unbound).
fn find_in_frame(cx: &mut Cx, rho: Root<'_>, sym: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.find_var_in_frame3(rho, sym, true, out)
}

/// `getVarValInFrame(rho, sym, unbound_ok)`: the binding's value, a
/// promise forced (in the empty environment); GNU R's error when unbound
/// unless `unbound_ok`.
// R: envir.c:getVarValInFrame
fn get_var_val_in_frame(cx: &mut Cx, rho: Root<'_>, sym: Root<'_>, unbound_ok: bool, out: Out<'_>) -> RResult<()> {
    let r = out.root();
    find_in_frame(cx, rho, sym, out)?;
    if !unbound_ok && r.bind(cx).sexp() == cx.unbound().sexp() {
        let n = String::from_utf8_lossy(sym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
        return Err(cx.error(&format!("object '{n}' not found")));
    }
    if r.bind(cx).sexptype() == PROMSXP {
        cx.scope(|cx, s| {
            let p = s.root(cx, r.bind(cx));
            let empty = s.root(cx, cx.empty_env());
            let o = r.reset(cx);
            crate::eval(cx, p, empty, o)
        })?;
    }
    Ok(())
}

/// `callR1(fun, arg)`: `fun(x)` evaluated in a new environment of the base
/// namespace binding `x` to `arg`.
// R: envir.c:callR1
fn call_r1(cx: &mut Cx, fun: &[u8], arg: Root<'_>, out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let rho = s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(cx.base_namespace(), false, 0)))?;
        let x = cx.install_rooted(s, b"x")?;
        cx.define_var(x, arg, rho)?;
        let f = cx.install_rooted(s, fun)?;
        let call = s.capture(cx, |cx, o| cx.build(o, |cx| cx.lang2(f.bind(cx), x.bind(cx))))?;
        crate::eval(cx, call, rho, out)
    })
}

/// `R_getNSValue(call, ns, name, exported)`: `name` in namespace `ns` (a
/// namespace, or its name: loaded if it is not yet), exported only if
/// `exported`.
// R: envir.c:R_getNSValue
pub fn get_ns_value(
    cx: &mut Cx,
    call: Option<Root<'_>>,
    ns: Root<'_>,
    name: Root<'_>,
    exported: bool,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let ns = if is_namespace_env(cx, ns.bind(cx).sexp()) {
            ns
        } else {
            let pkg = check_name(cx, s, call, ns, "bad namespace name")?;
            let reg = s.root(cx, cx.namespace_registry());
            let nsv = s.capture(cx, |cx, o| find_in_frame(cx, reg, pkg, o))?;
            let nsv = if nsv.bind(cx).sexp() == cx.unbound().sexp() {
                s.capture(cx, |cx, o| load_namespace(cx, pkg, o))?
            } else {
                nsv
            };
            if !is_namespace_env(cx, nsv.bind(cx).sexp()) {
                return Err(cx.error_call(call, "bad namespace"));
            }
            nsv
        };
        let name = check_name(cx, s, call, name, "bad variable name")?;
        // base or non-exported variables
        if ns.bind(cx).sexp() == cx.base_namespace().sexp() || !exported {
            return get_var_val_in_frame(cx, ns, name, false, out);
        }
        // exported variables
        let nssym = cx.install_rooted(s, b".__NAMESPACE__.")?;
        let info = s.capture(cx, |cx, o| get_var_val_in_frame(cx, ns, nssym, false, o))?;
        let exports_sym = cx.install_rooted(s, b"exports")?;
        let exports = s.capture(cx, |cx, o| get_var_val_in_frame(cx, info, exports_sym, false, o))?;
        let export_name = s.capture(cx, |cx, o| get_var_val_in_frame(cx, exports, name, true, o))?;
        if export_name.bind(cx).sexp() != cx.unbound().sexp() {
            let e = check_name(cx, s, call, export_name, "bad variable name")?;
            return crate::eval(cx, e, ns, out);
        }
        // lazydata
        let lazy_sym = cx.install_rooted(s, b"lazydata")?;
        let ld = s.capture(cx, |cx, o| get_var_val_in_frame(cx, info, lazy_sym, false, o))?;
        let res = out.root();
        get_var_val_in_frame(cx, ld, name, true, out)?;
        if res.bind(cx).sexp() != cx.unbound().sexp() {
            return Ok(());
        }
        let nsname = s.capture(cx, |cx, o| call_r1(cx, b"getNamespaceName", ns, o))?;
        let nm = match nsname.bind(cx).view() {
            View::Str(v) if v.len() == 1 => v.get(0).bytes().to_vec(),
            _ => return Err(cx.error_call(call, "bad value returned by `getNamespaceName'")),
        };
        let sym_name = name.bind(cx).as_sym().unwrap().name().bytes().to_vec();
        let msg = format!(
            "'{}' is not an exported object from 'namespace:{}'",
            String::from_utf8_lossy(&sym_name),
            String::from_utf8_lossy(&nm)
        );
        Err(cx.error_call(call, &msg))
    })
}

/// The call's `i`th argument expression, rooted.
fn arg<'s>(cx: &mut Cx, s: Scope<'s>, call: Root<'_>, i: usize) -> Root<'s> {
    let mut a = call.bind(cx).as_cons().unwrap().cdr();
    for _ in 0..i {
        a = a.as_cons().unwrap().cdr();
    }
    let v = a.as_cons().unwrap().car();
    s.root(cx, v)
}

/// `checkArity` for a special of two arguments: the call's argument count.
fn check_arity2(cx: &mut Cx, call: Root<'_>, name: &str) -> RResult<()> {
    let mut n = 0;
    let mut a = call.bind(cx).as_cons().map(|c| c.cdr()).unwrap_or(cx.nil());
    while let Some(c) = a.as_cons() {
        n += 1;
        a = c.cdr();
    }
    crate::prims::check_arity(cx, call, n, name, 2, false)
}

/// `pkg::name`: `R_getNSValue(R_NilValue, pkg, name, TRUE)`.
// R: envir.c:do_colon2
pub fn do_colon2(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, "::")?;
    cx.scope(|cx, s| {
        let (ns, name) = (arg(cx, s, call, 0), arg(cx, s, call, 1));
        // "use R_NilValue for the call to avoid changing the error message"
        get_ns_value(cx, None, ns, name, true, out)
    })
}

/// `pkg:::name`: `R_getNSValue(call, pkg, name, FALSE)`.
// R: envir.c:do_colon3
pub fn do_colon3(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, ":::")?;
    cx.scope(|cx, s| {
        let (ns, name) = (arg(cx, s, call, 0), arg(cx, s, call, 1));
        get_ns_value(cx, Some(call), ns, name, false, out)
    })
}

/// `getNamespaceValue(ns, name, exported)`.
// R: envir.c:do_getNSValue
pub fn do_get_ns_value(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "getNamespaceValue", 3, true)?;
    let exported = robigo_rtbuiltins::util::as_logical(cx, args.get(cx, 2));
    // (eval column 211: R_Visible as the lookup left it)
    get_ns_value(cx, None, args.root(0), args.root(1), exported != 0, out)
}

/// `registerNamespace(name, env)`.
// R: envir.c:do_regNS
pub fn do_reg_ns(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "registerNamespace", 2, true)?;
    cx.scope(|cx, s| {
        let name = check_name(cx, s, Some(call), args.root(0), "bad namespace name")?;
        let reg = s.root(cx, cx.namespace_registry());
        let v = s.capture(cx, |cx, o| find_in_frame(cx, reg, name, o))?;
        if v.bind(cx).sexp() != cx.unbound().sexp() {
            return Err(cx.error_call(Some(call), "namespace already registered"));
        }
        cx.define_var(name, args.root(1), reg)
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `unregisterNamespace(name)`.
// R: envir.c:do_unregNS
pub fn do_unreg_ns(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "unregisterNamespace", 1, true)?;
    cx.scope(|cx, s| {
        let name = check_name(cx, s, Some(call), args.root(0), "bad namespace name")?;
        let reg = s.root(cx, cx.namespace_registry());
        let v = s.capture(cx, |cx, o| find_in_frame(cx, reg, name, o))?;
        if v.bind(cx).sexp() == cx.unbound().sexp() {
            return Err(cx.error_call(Some(call), "namespace not registered"));
        }
        cx.remove_var_from_frame(name, reg)
    })?;
    out.set(cx, cx.nil());
    Ok(())
}

/// `getRegisteredNamespace(name)` (CODE 0: the namespace or NULL) and
/// `isRegisteredNamespace(name)` (1).
// R: envir.c:do_getRegNS
pub fn do_get_reg_ns<const CODE: i32>(
    cx: &mut Cx,
    call: Root<'_>,
    args: Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    let fname = if CODE == 0 { "getRegisteredNamespace" } else { "isRegisteredNamespace" };
    crate::prims::check_arity(cx, call, args.len(), fname, 1, true)?;
    cx.scope(|cx, s| {
        // coerceVector(CAR(args), SYMSXP): a string's first element, a symbol
        let x = args.get(cx, 0);
        let (t, n) = (x.sexptype(), x.len().unwrap_or(0));
        let name = match t {
            SYMSXP => s.root(cx, x),
            STRSXP if n >= 1 => install_first_at(cx, s, args.root(0), 0)?,
            _ => {
                let r = args.root(0);
                let coerced = s.capture(cx, |cx, o| robigo_rtbuiltins::coerce::coerce_vector(cx, r, SYMSXP, o))?;
                check_name(cx, s, Some(call), coerced, "bad namespace name")?
            }
        };
        let reg = s.root(cx, cx.namespace_registry());
        let v = s.capture(cx, |cx, o| find_in_frame(cx, reg, name, o))?;
        let unbound = v.bind(cx).sexp() == cx.unbound().sexp();
        if CODE == 0 {
            if unbound {
                out.set(cx, cx.nil());
            } else {
                out.set(cx, v.bind(cx));
            }
            Ok(())
        } else {
            cx.build_scalar(out, robigo_rtcore::Lgl(!unbound as i32))
        }
    })
}

/// `getNamespaceRegistry()`.
// R: envir.c:do_getNSRegistry
pub fn do_get_ns_registry(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "getNamespaceRegistry", 0, true)?;
    out.set(cx, cx.namespace_registry());
    Ok(())
}

/// `isNamespaceEnv(env)`.
// R: envir.c:do_isNSEnv
pub fn do_is_ns_env(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "isNamespaceEnv", 1, true)?;
    let is = is_namespace_env(cx, args.get(cx, 0).sexp());
    cx.build_scalar(out, robigo_rtcore::Lgl(is as i32))
}

/// `topenv(envir, matchThisEnv)`: the first of `envir`'s enclosures that
/// is `target`, global, base, a package or namespace environment, or
/// binds `.packageName`; the global environment if none is.
// R: envir.c:topenv, do_topenv
pub fn do_topenv(cx: &mut Cx, call: Root<'_>, args: Args<'_>, env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "topenv", 2, true)?;
    cx.scope(|cx, s| {
        let e = args.get(cx, 0);
        let envir = s.root(cx, if e.sexptype() == ENVSXP { e } else { env.bind(cx) });
        let t = args.get(cx, 1);
        let target = if t.sexptype() == ENVSXP { Some(t.sexp()) } else { None };
        let pkg_name = s.root(cx, cx.sym(RSym::DotPackageName));
        let cur = s.root(cx, envir.bind(cx));
        loop {
            let c = cur.bind(cx).sexp();
            if c == cx.empty_env().sexp() {
                out.set(cx, cx.global_env());
                return Ok(());
            }
            let top = Some(c) == target
                || c == cx.global_env().sexp()
                || c == cx.base_env().sexp()
                || c == cx.base_namespace().sexp()
                || is_package_env(cx, c)
                || is_namespace_env(cx, c)
                || cx.exists_var_in_frame(cur, pkg_name)?;
            if top {
                out.set(cx, cur.bind(cx));
                return Ok(());
            }
            cur.replace_with(cx, |cx| match cur.bind(cx).view() {
                View::Env(e) => Ok(e.enclos()),
                _ => Ok(cx.empty_env()),
            })?;
        }
    })
}

/// `importIntoEnv(impenv, impnames, expenv, expnames)`: copy bindings,
/// promises unforced and active bindings kept.
// R: envir.c:do_importIntoEnv
pub fn do_import_into_env(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "importIntoEnv", 4, true)?;
    let (impenv, impnames, expenv, expnames) = (args.root(0), args.root(1), args.root(2), args.root(3));
    for (e, what) in [(impenv, "import"), (expenv, "export")] {
        match e.bind(cx).sexptype() {
            NILSXP => return Err(cx.error("use of NULL environment is defunct")),
            ENVSXP => {}
            OBJSXP if e.bind(cx).is_s4() => {
                return Err(cx.error("robigo: importIntoEnv with an S4 environment object is not implemented yet"));
            }
            _ => return Err(cx.error(&format!("bad {what} environment argument"))),
        }
    }
    if impnames.bind(cx).sexptype() != STRSXP || expnames.bind(cx).sexptype() != STRSXP {
        return Err(cx.error("invalid 'names' argument"));
    }
    let n = impnames.bind(cx).len().unwrap_or(0);
    if n != expnames.bind(cx).len().unwrap_or(0) {
        return Err(cx.error("length of import and export names must match"));
    }
    for i in 0..n {
        cx.scope(|cx, s| {
            let impsym = install_first_at(cx, s, impnames, i)?;
            let expsym = install_first_at(cx, s, expnames, i)?;
            // the binding: in the first frame of expenv's chain that has it
            let cur = s.root(cx, expenv.bind(cx));
            let mut found: Option<(Root<'_>, bool)> = None;
            while cur.bind(cx).sexp() != cx.empty_env().sexp() {
                let c = cur.bind(cx).sexp();
                if c == cx.base_namespace().sexp() || c == cx.base_env().sexp() {
                    found = symvalue(cx, s, expsym);
                    break;
                }
                let o = s.out(cx);
                let cell = o.root();
                if cx.find_var_loc_in_frame(cur, expsym, o)? {
                    // SAFETY: a binding cell (a LISTSXP) of a rooted frame.
                    let (v, active) = unsafe {
                        let c = cell.bind(cx).sexp();
                        (c.car(), c.gp() & robigo_heap::gp::cell::ACTIVE_BINDING != 0)
                    };
                    // SAFETY: reachable from the cell.
                    found = Some((s.root(cx, unsafe { Ref::from_sexp(v, cx) }), active));
                    break;
                }
                cur.replace_with(cx, |cx| match cur.bind(cx).view() {
                    View::Env(e) => Ok(e.enclos()),
                    _ => Ok(cx.empty_env()),
                })?;
            }
            let (val, active) = match found {
                Some(f) => f,
                // binding = expsym: SYMVALUE
                None => match symvalue(cx, s, expsym) {
                    Some(f) => f,
                    None => {
                        let nm = String::from_utf8_lossy(expsym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
                        return Err(cx.error(&format!("exported symbol '{nm}' has no value")));
                    }
                },
            };
            if active { cx.make_active_binding(impsym, val, impenv) } else { cx.define_var(impsym, val, impenv) }
        })?;
    }
    out.set(cx, cx.nil());
    Ok(())
}

/// `SYMVALUE(sym)`, base's binding as its cell holds it (a promise not
/// forced, an active binding's function not called), and whether it is an
/// active binding; None when unbound.
fn symvalue<'s>(cx: &Cx, s: Scope<'s>, sym: Root<'_>) -> Option<(Root<'s>, bool)> {
    let (Some(base), Some(y)) = (crate::dispatch::env_ref(cx.base_env()), sym.bind(cx).as_sym()) else { return None };
    let Lookup::Bound(b) = cx.frame_binding(base, y) else { return None };
    match b.get(cx) {
        BindingValue::Value(v) => Some((s.root(cx, v), false)),
        BindingValue::Active(f) => Some((s.root(cx, f), true)),
        // (base's cells hold objects)
        BindingValue::Deferred => None,
    }
}

fn install_first_at<'s>(cx: &mut Cx, s: Scope<'s>, v: Root<'_>, i: usize) -> RResult<Root<'s>> {
    let c = v.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().unwrap().get(i);
    let name = robigo_rtbuiltins::text::to_native(robigo_rtbuiltins::coerce::text(c), cx.locale())
        .map(|b| b.into_owned())
        .unwrap_or_else(|_| c.bytes().to_vec());
    if name.is_empty() {
        return Err(cx.error("attempt to use zero-length variable name"));
    }
    cx.install_rooted(s, &name)
}

/// `internalsID()`: `R_INTERNALS_UUID`.
// R: version.c:do_internalsID
pub fn do_internals_id(cx: &mut Cx, _call: Root<'_>, _args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    // (no checkArity: arguments are ignored)
    cx.build(out, |cx| cx.mk_string(R_INTERNALS_UUID, robigo_rtcore::Encoding::Native))
}

/// `environmentName(env)`: "R_GlobalEnv", "base", "R_EmptyEnv", a package
/// environment's name, a namespace's name (its spec's first element), the
/// `name` attribute, or "".
// R: builtin.c:do_envirName
pub fn do_envir_name(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "environmentName", 1, true)?;
    let e = args.get(cx, 0);
    if e.sexptype() == OBJSXP && e.is_s4() {
        return Err(cx.error("robigo: environmentName() of an S4 environment object is not implemented yet"));
    }
    let x = e.sexp();
    let fixed: Option<&[u8]> = if e.sexptype() != ENVSXP {
        Some(b"")
    } else if x == cx.global_env().sexp() {
        Some(b"R_GlobalEnv")
    } else if x == cx.base_env().sexp() {
        Some(b"base")
    } else if x == cx.empty_env().sexp() {
        Some(b"R_EmptyEnv")
    } else if x == cx.base_namespace().sexp() {
        Some(b"base")
    } else {
        None
    };
    if let Some(b) = fixed {
        return cx.build(out, |cx| cx.mk_string(b, robigo_rtcore::Encoding::Native));
    }
    // the first element of the package name or the spec, as a string vector
    let first = |cx: &Cx, x: Sexp| if is_package_env(cx, x) { name_attr(cx, x) } else { namespace_spec(cx, x) };
    if first(cx, x).is_some() {
        return cx.build(out, |cx| {
            // read again on each try: a collection between tries may move it
            let v = first(cx, args.get(cx, 0).sexp()).expect("robigo: the name read before");
            // SAFETY: a live STRSXP of at least one element, reachable from
            // the rooted argument, read with nothing able to collect since.
            let c = unsafe { Ref::from_sexp(*(v.std_data() as *const Sexp), cx) }.as_char().unwrap();
            let mut s = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
            s.set(0, c);
            Ok(s.finish().into())
        });
    }
    let a = robigo_rtbuiltins::attrib::get_attrib(cx, e, b"name").ok().flatten();
    match a {
        Some(v) => {
            out.set(cx, v);
            Ok(())
        }
        None => cx.build(out, |cx| cx.mk_string(b"", robigo_rtcore::Encoding::Native)),
    }
}

// ---- loading a namespace (C1-BC's seam) -----------------------------------

/// A symbol of `name`, rooted in `s`.
fn sym<'s>(cx: &mut Cx, s: Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    cx.install_rooted(s, name)
}

/// `R_FindNamespace(info)`: `getNamespace(info)`, the registered namespace
/// of that name, else [`load_namespace`]'s.
// R: envir.c:R_FindNamespace; namespace.R:getNamespace
pub fn find_namespace(cx: &mut Cx, name: &[u8], out: Out<'_>) -> RResult<()> {
    cx.scope(|cx, s| {
        let pkg = sym(cx, s, name)?;
        let reg = s.root(cx, cx.namespace_registry());
        let res = out.root();
        cx.find_var_in_frame3(reg, pkg, true, out)?;
        if res.bind(cx).sexp() == cx.unbound().sexp() {
            let o = res.reset(cx);
            load_namespace(cx, pkg, o)?;
        }
        Ok(())
    })
}

/// `loadNamespace(package)` where GNU R's C calls it, the namespace to
/// `out`: GNU R's `callR1(R_loadNamespaceSymbol, pkg)`, base's
/// `loadNamespace`, except that robigo-rt makes the compiler package's
/// namespace from source itself ([`load_compiler`]). INTERIM(L2): the
/// compiler goes through `loadNamespace` too once that runs from source
/// (it waits on B5a's file builtins).
// R: namespace.R:loadNamespace
pub fn load_namespace(cx: &mut Cx, pkg: Root<'_>, out: Out<'_>) -> RResult<()> {
    let name = pkg.bind(cx).as_sym().unwrap().name().bytes().to_vec();
    if name == b"compiler" {
        // INTERIM(L2): replaced by loadNamespace
        return load_compiler(cx, pkg, out);
    }
    // GNU R's callR1(R_loadNamespaceSymbol, pkg): base's loadNamespace
    call_r1(cx, b"loadNamespace", pkg, out)
}

/// The compiler package's namespace, made as GNU R's `loadNamespace` makes
/// one from source: `makeNamespace`'s environments, registered; the
/// package's R code (the pinned tarball's `cmp.R`, unmodified) evaluated
/// into it as `sys.source` does; `.onLoad`; NAMESPACE's exports; then
/// `sealNamespace`. The library is the tarball's `src/library` (no R_HOME
/// tree before L4). INTERIM(L2): replaced by loadNamespace.
// R: namespace.R:loadNamespace, makeNamespace, sealNamespace, runHook
fn load_compiler(cx: &mut Cx, pkg: Root<'_>, out: Out<'_>) -> RResult<()> {
    let dir = crate::base::source_dir().map_err(|m| cx.error(&format!("robigo: loading the compiler package: {m}")))?;
    let lib = dir.join("src/library");
    let pkgdir = lib.join("compiler");
    let read = |cx: &mut Cx, f: &std::path::Path| {
        std::fs::read_to_string(f)
            .map_err(|e| cx.error(&format!("robigo: loading the compiler package: {}: {e}", f.display())))
    };
    let code = read(cx, &pkgdir.join("R/cmp.R"))?;
    let namespace = read(cx, &pkgdir.join("NAMESPACE"))?;
    let lib_s = lib.display().to_string();
    let path_s = pkgdir.display().to_string();
    cx.scope(|cx, s| {
        let nil = s.root(cx, cx.nil());
        let base_ns = s.root(cx, cx.base_namespace());
        let base_env = s.root(cx, cx.base_env());
        let new_env = |cx: &mut Cx, parent: Root<'_>| {
            s.capture(cx, |cx, o| cx.build(o, |cx| cx.new_env(parent.bind(cx), true, 0)))
        };
        let string = |cx: &mut Cx, t: &[u8]| s.capture(cx, |cx, o| cx.build(o, |cx| cx.mk_string(t, Encoding::Native)));
        let define = |cx: &mut Cx, env: Root<'_>, n: &[u8], v: Root<'_>| -> RResult<()> {
            let k = sym(cx, s, n)?;
            cx.define_var(k, v, env)
        };
        // makeNamespace(name, version, lib)
        let impenv = new_env(cx, base_ns)?;
        let iname = string(cx, b"imports:compiler")?;
        robigo_rtbuiltins::attrib::set_attrib(cx, impenv, b"name", iname)?;
        let env = new_env(cx, impenv)?;
        let info = new_env(cx, base_env)?;
        define(cx, env, b".__NAMESPACE__.", info)?;
        let spec = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(2)?;
                v.set(0, cx.mk_char(b"compiler", Encoding::Native)?);
                v.set(1, cx.mk_char(R_VERSION.as_bytes(), Encoding::Native)?);
                let mut nm = cx.alloc_vector::<robigo_rtcore::Char<'_>>(2)?;
                nm.set(0, cx.mk_char(b"name", Encoding::Native)?);
                nm.set(1, cx.mk_char(b"version", Encoding::Native)?);
                let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
                a.apply(cx, &mut v)?;
                Ok(v.finish().into())
            })
        })?;
        define(cx, info, b"spec", spec)?;
        let exports = new_env(cx, base_env)?;
        define(cx, info, b"exports", exports)?;
        let dimpenv = new_env(cx, base_env)?;
        let lname = string(cx, b"lazydata:compiler")?;
        robigo_rtbuiltins::attrib::set_attrib(cx, dimpenv, b"name", lname)?;
        define(cx, info, b"lazydata", dimpenv)?;
        let imports = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<Ref<'_>>(1)?;
                v.set(0, cx.alloc_copy(&[robigo_rtcore::Lgl(1)])?.finish().into());
                let mut nm = cx.alloc_vector::<robigo_rtcore::Char<'_>>(1)?;
                nm.set(0, cx.mk_char(b"base", Encoding::Native)?);
                let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                a.install(robigo_rtbuiltins::attrib::Attr { tag: cx.sym(RSym::Names), value: nm.finish().into() });
                a.apply(cx, &mut v)?;
                Ok(v.finish().into())
            })
        })?;
        define(cx, info, b"imports", imports)?;
        let path = string(cx, path_s.as_bytes())?;
        define(cx, info, b"path", path)?;
        define(cx, info, b"dynlibs", nil)?;
        let s3 = s.capture(cx, |cx, o| {
            cx.build(o, |cx| {
                let mut v = cx.alloc_vector::<robigo_rtcore::Char<'_>>(0)?;
                let mut a = robigo_rtbuiltins::attrib::Attrs::new();
                a.install(robigo_rtbuiltins::attrib::Attr {
                    tag: cx.sym(RSym::Dim),
                    value: cx.alloc_copy(&[0i32, 4])?.finish().into(),
                });
                a.apply(cx, &mut v)?;
                Ok(v.finish().into())
            })
        })?;
        define(cx, info, b"S3methods", s3)?;
        let s3t = new_env(cx, base_env)?;
        define(cx, env, b".__S3MethodsTable__.", s3t)?;
        // .Internal(registerNamespace(name, env)); unregistered if the
        // load fails (loadNamespace's on.exit)
        let reg = s.root(cx, cx.namespace_registry());
        cx.define_var(pkg, env, reg)?;
        let r = (|| -> RResult<()> {
            // env$.packageName <- package
            let pname = string(cx, b"compiler")?;
            define(cx, env, b".packageName", pname)?;
            // sys.source(codeFile, env): each expression evaluated in env
            let exprs = s.capture(cx, |cx, o| crate::lower::parse(cx, &code, o))?;
            let n = exprs.bind(cx).len().unwrap_or(0);
            for i in 0..n {
                cx.scope(|cx, t| {
                    let e = t.root(cx, exprs.bind(cx).as_vector::<Ref<'_>>().unwrap().get(i));
                    let o = t.out(cx);
                    crate::eval(cx, e, env, o)
                })?;
            }
            // runHook(".onLoad", env, package.lib, package)
            let onload = sym(cx, s, b".onLoad")?;
            let libname = string(cx, lib_s.as_bytes())?;
            let pkgname = string(cx, b"compiler")?;
            let hook =
                s.capture(cx, |cx, o| cx.build(o, |cx| cx.lang3(onload.bind(cx), libname.bind(cx), pkgname.bind(cx))))?;
            s.capture(cx, |cx, o| crate::eval(cx, hook, env, o))?;
            // namespaceExport(ns, exports): each export bound to its name
            let spec_exprs = s.capture(cx, |cx, o| crate::lower::parse(cx, &namespace, o))?;
            for name in exports_of(cx, spec_exprs) {
                let v = string(cx, &name)?;
                define(cx, exports, &name, v)?;
            }
            // sealNamespace(ns)
            cx.lock_environment(env, true)?;
            cx.lock_environment(impenv, true)
        })();
        if let Err(e) = r {
            cx.remove_var_from_frame(pkg, reg)?;
            return Err(e);
        }
        out.set(cx, env.bind(cx));
        Ok(())
    })
}

/// R's version, the version of its base packages.
const R_VERSION: &str = "4.6.2";

/// The names a NAMESPACE file's `export(...)` directives name.
// R: namespace.R:parseNamespaceFile (export)
fn exports_of(cx: &Cx, exprs: Root<'_>) -> Vec<Vec<u8>> {
    let mut names = Vec::new();
    let v = exprs.bind(cx);
    let Some(l) = v.as_vector::<Ref<'_>>() else { return names };
    for i in 0..l.len() {
        let Some(c) = l.get(i).as_cons().filter(|c| c.as_ref().sexptype() == LANGSXP) else { continue };
        if c.car().as_sym().is_none_or(|f| f.name().bytes() != b"export") {
            continue;
        }
        let mut a = c.cdr().as_cons();
        while let Some(cell) = a {
            let x = cell.car();
            if let Some(sy) = x.as_sym() {
                names.push(sy.name().bytes().to_vec());
            } else if let Some(sv) = x.as_vector::<robigo_rtcore::Char<'_>>().filter(|_| x.sexptype() == STRSXP) {
                for k in 0..sv.len() {
                    names.push(sv.get(k).bytes().to_vec());
                }
            }
            a = cell.cdr().as_cons();
        }
    }
    names
}
