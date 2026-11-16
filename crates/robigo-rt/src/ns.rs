//! `::` and `:::` (GNU R's `envir.c`): a variable of a namespace by name;
//! `R_FindNamespace`, `topenv` and `isNamespaceEnv`; and the seam where
//! GNU R's C calls `loadNamespace` ([`load_namespace`]).
//!
//! INTERIM(L2): until L2 evaluates base's namespace code, robigo-rt has
//! base's namespace and one more, the compiler package, which
//! [`load_namespace`] makes from the pinned tarball's source as GNU R's
//! `loadNamespace` makes a namespace (`makeNamespace`, the code evaluated
//! into it, `.onLoad`, NAMESPACE's exports, `sealNamespace`). Any other
//! package raises "not implemented yet". L2 replaces the body of
//! [`load_namespace`] with GNU R's `loadNamespace`; its callers stay.

use robigo_heap::types::*;
#[allow(unused_imports)]
use robigo_rtcore::Scope;
use robigo_rtcore::{Cx, Encoding, Out, RResult, RSym, Ref, Root, View};

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

/// `getVarValInFrame(rho, sym, unbound_ok)`: `sym`'s value in `rho`'s
/// frame, a promise forced (in the empty environment, as GNU R does);
/// unbound is GNU R's error unless `unbound_ok` (then `R_UnboundValue`).
// R: envir.c:getVarValInFrame
fn var_val_in_frame(cx: &mut Cx, rho: Root<'_>, sym: Root<'_>, unbound_ok: bool, out: Out<'_>) -> RResult<()> {
    let res = out.root();
    cx.find_var_in_frame3(rho, sym, true, out)?;
    if !unbound_ok && res.bind(cx).sexp() == cx.unbound().sexp() {
        let n = String::from_utf8_lossy(sym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
        return Err(cx.error(&format!("object '{n}' not found")));
    }
    if res.bind(cx).sexptype() == PROMSXP {
        cx.scope(|cx, s| {
            let p = s.root(cx, res.bind(cx));
            let empty = s.root(cx, cx.empty_env());
            let o = res.reset(cx);
            crate::eval(cx, p, empty, o)
        })?;
    }
    Ok(())
}

/// A symbol of `name`, rooted in `s`.
fn sym<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    cx.install_rooted(s, name)
}

/// `R_getNSValue(call, ns, name, exported)`: `name` in the namespace `ns`
/// (a namespace, or a package name to look up, loading it if need be);
/// for an exported lookup, through the namespace's exports, then its
/// lazy data.
// R: envir.c:R_getNSValue
fn get_ns_value(
    cx: &mut Cx,
    call: Option<Root<'_>>,
    ns: usize,
    name: usize,
    src: Root<'_>,
    exported: bool,
    out: Out<'_>,
) -> RResult<()> {
    cx.scope(|cx, s| {
        let nsr = s.root(cx, arg(cx, src, ns));
        let name = s.root(cx, arg(cx, src, name));
        let nsv = if crate::dispatch::is_namespace_env(cx, nsr.bind(cx)) {
            nsr
        } else {
            let pkg = match as_name(cx, s, nsr)? {
                Some(p) => p,
                None => return Err(cx.error_call(call, "bad namespace name")),
            };
            let reg = s.root(cx, cx.namespace_registry());
            let v = s.out(cx).root();
            let o = v.reset(cx);
            cx.find_var_in_frame3(reg, pkg, true, o)?;
            if v.bind(cx).sexp() == cx.unbound().sexp() {
                let o = v.reset(cx);
                load_namespace(cx, pkg, o)?;
            }
            if !crate::dispatch::is_namespace_env(cx, v.bind(cx)) {
                return Err(cx.error_call(call, "bad namespace"));
            }
            v
        };
        let sym = match as_name(cx, s, name)? {
            Some(p) => p,
            None => return Err(cx.error_call(call, "bad variable name")),
        };
        // base or non-exported variables
        if nsv.bind(cx).sexp() == cx.base_namespace().sexp() || !exported {
            return var_val_in_frame(cx, nsv, sym, false, out);
        }
        // exported variables
        let info_o = s.out(cx);
        let info = info_o.root();
        let nss = sym_of(cx, s, b".__NAMESPACE__.")?;
        var_val_in_frame(cx, nsv, nss, false, info_o)?;
        let exports_o = s.out(cx);
        let exports = exports_o.root();
        let es = sym_of(cx, s, b"exports")?;
        var_val_in_frame(cx, info, es, false, exports_o)?;
        let ename_o = s.out(cx);
        let ename = ename_o.root();
        var_val_in_frame(cx, exports, sym, true, ename_o)?;
        if ename.bind(cx).sexp() != cx.unbound().sexp() {
            let en = match as_name(cx, s, ename)? {
                Some(p) => p,
                None => return Err(cx.error_call(call, "bad variable name")),
            };
            return crate::eval(cx, en, nsv, out);
        }
        // lazydata
        let ld_o = s.out(cx);
        let ld = ld_o.root();
        let ls = sym_of(cx, s, b"lazydata")?;
        var_val_in_frame(cx, info, ls, false, ld_o)?;
        let res = out.root();
        var_val_in_frame(cx, ld, sym, true, out)?;
        if res.bind(cx).sexp() != cx.unbound().sexp() {
            return Ok(());
        }
        // getNamespaceName(ns): base's is "base", another's its spec's name
        let nsname = namespace_name(cx, s, nsv)?;
        let n = String::from_utf8_lossy(sym.bind(cx).as_sym().unwrap().name().bytes()).into_owned();
        Err(cx.error_call(call, &format!("'{n}' is not an exported object from 'namespace:{nsname}'")))
    })
}

/// [`sym`] under another name, for the lookups above.
fn sym_of<'s>(cx: &mut Cx, s: robigo_rtcore::Scope<'s>, name: &[u8]) -> RResult<Root<'s>> {
    sym(cx, s, name)
}

/// `getNamespaceName(ns)`: "base", or the first element of the
/// namespace's `spec`.
// R: namespace.R:getNamespaceName
fn namespace_name(cx: &mut Cx, s: robigo_rtcore::Scope<'_>, ns: Root<'_>) -> RResult<String> {
    if ns.bind(cx).sexp() == cx.base_namespace().sexp() {
        return Ok("base".to_owned());
    }
    let info_o = s.out(cx);
    let info = info_o.root();
    let nss = sym(cx, s, b".__NAMESPACE__.")?;
    var_val_in_frame(cx, ns, nss, false, info_o)?;
    let spec_o = s.out(cx);
    let spec = spec_o.root();
    let ss = sym(cx, s, b"spec")?;
    var_val_in_frame(cx, info, ss, false, spec_o)?;
    match spec.bind(cx).as_vector::<robigo_rtcore::Char<'_>>().filter(|v| !v.is_empty()) {
        Some(v) => Ok(String::from_utf8_lossy(v.get(0).bytes()).into_owned()),
        None => Err(cx.error("bad value returned by `getNamespaceName'")),
    }
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
/// `out`. INTERIM(L2): replaced by loadNamespace. robigo-rt makes the
/// compiler package's namespace from source ([`load_compiler`]); another
/// package is not implemented yet.
// R: namespace.R:loadNamespace
pub fn load_namespace(cx: &mut Cx, pkg: Root<'_>, out: Out<'_>) -> RResult<()> {
    let name = pkg.bind(cx).as_sym().unwrap().name().bytes().to_vec();
    if name == b"compiler" {
        // INTERIM(L2): replaced by loadNamespace
        return load_compiler(cx, pkg, out);
    }
    Err(cx.error("robigo: loadNamespace is not implemented yet"))
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

/// `.Internal(topenv(envir, matchThisEnv))`: [`crate::dispatch::topenv`]
/// from `envir` (the caller's environment if it is not one), `target` NULL
/// unless an environment.
// R: envir.c:do_topenv
pub fn do_topenv(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "topenv", 2, true)?;
    let e = args.get(cx, 0);
    let envir = if e.sexptype() == ENVSXP { e } else { env.bind(cx) };
    let t = args.get(cx, 1);
    let target = (t.sexptype() == ENVSXP).then_some(t);
    let r = crate::dispatch::topenv(cx, target, envir);
    out.set(cx, r);
    Ok(())
}

/// `.Internal(isNamespaceEnv(x))`: `R_IsNamespaceEnv(x)`.
// R: envir.c:do_isNSEnv
pub fn do_isnsenv(
    cx: &mut Cx,
    call: Root<'_>,
    args: robigo_rtcore::Args<'_>,
    _env: Root<'_>,
    out: Out<'_>,
) -> RResult<()> {
    crate::prims::check_arity(cx, call, args.len(), "isNamespaceEnv", 1, true)?;
    let b = crate::dispatch::is_namespace_env(cx, args.get(cx, 0));
    cx.build_scalar(out, robigo_rtcore::Lgl(b as i32))
}

/// `pkg::name`: `R_getNSValue(R_NilValue, pkg, name, TRUE)`.
// R: envir.c:do_colon2
pub fn do_colon2(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, "::")?;
    // "use R_NilValue for the call to avoid changing the error message"
    get_ns_value(cx, None, 0, 1, call, true, out)
}

/// `pkg:::name`: `R_getNSValue(call, pkg, name, FALSE)`.
// R: envir.c:do_colon3
pub fn do_colon3(cx: &mut Cx, call: Root<'_>, _op: Root<'_>, _rho: Root<'_>, out: Out<'_>) -> RResult<()> {
    check_arity2(cx, call, ":::")?;
    get_ns_value(cx, Some(call), 0, 1, call, false, out)
}
