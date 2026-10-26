//! Namespaces and the search path: `::`, `:::`, the namespace registry,
//! `attach`/`detach` (GNU R's envir.c).

use super::{arg, internal, special};
use crate::env::Env;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::Sym;
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    special("::", do_colon2, 2).custom_vis(),
    special(":::", do_colon3, 2).custom_vis(),
    internal("getNamespaceValue", do_get_ns_value, 3),
    internal("registerNamespace", do_register_ns, 2),
    internal("unregisterNamespace", do_unregister_ns, 1),
    internal("getRegisteredNamespace", do_get_registered_ns, 1),
    internal("isRegisteredNamespace", do_is_registered_ns, 1),
    internal("getNamespaceRegistry", do_get_ns_registry, 0),
    internal("importIntoEnv", do_import_into_env, 4),
    internal("attach", do_attach, 3),
    internal("detach", do_detach, 1),
    internal("builtins", do_builtins, 1),
    internal("mkUnbound", do_mk_unbound, 1),
];

// R: envir.c:checkNSname
fn check_ns_name(call: &Value, v: &Value) -> RResult<Sym> {
    match v {
        Value::Sym(s) => Ok(*s),
        Value::Str(s) if !s.data.is_empty() => Ok(Sym::new(s.data[0].as_deref().unwrap_or("NA"))),
        _ => Err(Flow::error_call(call.clone(), "bad namespace name")),
    }
}

// R: envir.c:checkVarName
fn check_var_name(call: &Value, v: &Value) -> RResult<Sym> {
    match v {
        Value::Sym(s) => Ok(*s),
        Value::Str(s) if !s.data.is_empty() => Ok(Sym::new(s.data[0].as_deref().unwrap_or("NA"))),
        _ => Err(Flow::error_call(call.clone(), "bad variable name")),
    }
}

/// Call `fun(x)` with `x` bound to `arg` in a new environment enclosed by
/// the base namespace.
// R: envir.c:callR1
fn call_r1(i: &mut Interp, fun: &str, arg: Value) -> RResult<Value> {
    let rho = Env::new(Some(i.base_ns.clone()));
    rho.set_local(Sym::new("x"), arg);
    let expr = Value::call_sym(fun, vec![Value::sym("x")]);
    i.eval(&expr, &rho)
}

/// A variable's value in one frame, forcing a promise.
// R: envir.c:getVarValInFrame
fn var_in_frame(i: &mut Interp, rho: &Env, sym: Sym, unbound_ok: bool) -> RResult<Option<Value>> {
    match rho.get_local(sym) {
        None if unbound_ok => Ok(None),
        None => Err(Flow::error(format!("object '{}' not found", sym))),
        Some(Value::Promise(p)) => i.force_promise(&p).map(Some),
        Some(v) => Ok(Some(v)),
    }
}

fn is_namespace_env(i: &Interp, e: &Env) -> bool {
    e.ptr_eq(&i.base_ns) || crate::dispatch::is_namespace(e)
}

/// The namespace `pkg`, loading it with base R's `loadNamespace` if needed.
pub fn get_namespace(i: &mut Interp, pkg: &str) -> RResult<Env> {
    if let Some(Value::Env(e)) = i.ns_registry.get_local(Sym::new(pkg)) {
        return Ok(e);
    }
    match call_r1(i, "loadNamespace", Value::Sym(Sym::new(pkg)))? {
        Value::Env(e) if is_namespace_env(i, &e) => Ok(e),
        _ => Err(Flow::error_call(Value::Null, "bad namespace")),
    }
}

// R: envir.c:R_getNSValue
fn get_ns_value(i: &mut Interp, call: &Value, ns: &Value, name: &Value, exported: bool) -> RResult<Value> {
    let ns = match ns {
        Value::Env(e) if is_namespace_env(i, e) => e.clone(),
        other => {
            let pkg = check_ns_name(call, other)?;
            let found = i.ns_registry.get_local(pkg);
            let v = match found {
                Some(v) => v,
                None => call_r1(i, "loadNamespace", Value::Sym(pkg))?,
            };
            match v {
                Value::Env(e) if is_namespace_env(i, &e) => e,
                _ => return Err(Flow::error_call(call.clone(), "bad namespace")),
            }
        }
    };
    let name = check_var_name(call, name)?;
    if ns.ptr_eq(&i.base_ns) || !exported {
        return Ok(var_in_frame(i, &ns, name, false)?.unwrap());
    }
    let info = match var_in_frame(i, &ns, Sym::new(".__NAMESPACE__."), false)? {
        Some(Value::Env(e)) => e,
        _ => return Err(Flow::error("bad namespace information")),
    };
    let exports = match var_in_frame(i, &info, Sym::new("exports"), false)? {
        Some(Value::Env(e)) => e,
        _ => return Err(Flow::error("bad namespace exports")),
    };
    if let Some(export_name) = var_in_frame(i, &exports, name, true)? {
        let sym = check_var_name(call, &export_name)?;
        return i.eval(&Value::Sym(sym), &ns);
    }
    if let Some(Value::Env(ld)) = var_in_frame(i, &info, Sym::new("lazydata"), false)? {
        if let Some(v) = var_in_frame(i, &ld, name, true)? {
            return Ok(v);
        }
    }
    let nsname = match call_r1(i, "getNamespaceName", Value::Env(ns))? {
        Value::Str(s) if s.data.len() == 1 => s.data[0].as_deref().unwrap_or("NA").to_string(),
        _ => return Err(Flow::error_call(call.clone(), "bad value returned by `getNamespaceName'")),
    };
    Err(Flow::error_call(
        call.clone(),
        format!("'{}' is not an exported object from 'namespace:{}'", name, nsname),
    ))
}

fn do_colon2(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), format!("{} arguments passed to '::' which requires 2", args.len())));
    }
    i.visible = true;
    // R reports errors without a call here, "to avoid changing the error message".
    get_ns_value(i, &Value::Null, &args[0].value, &args[1].value, true)
}

fn do_colon3(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), format!("{} arguments passed to ':::' which requires 2", args.len())));
    }
    i.visible = true;
    get_ns_value(i, call, &args[0].value, &args[1].value, false)
}

fn do_get_ns_value(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let exported = crate::coerce::as_logical(&arg(&args, 2)) != 0;
    get_ns_value(i, &Value::Null, &arg(&args, 0), &arg(&args, 1), exported)
}

// R: envir.c:do_regNS
fn do_register_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = check_ns_name(call, &arg(&args, 0))?;
    if i.ns_registry.has_local(name) {
        return Err(Flow::error_call(call.clone(), "namespace already registered"));
    }
    i.ns_registry.set_local(name, arg(&args, 1));
    Ok(Value::Null)
}

// R: envir.c:do_unregNS
fn do_unregister_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = check_ns_name(call, &arg(&args, 0))?;
    if !i.ns_registry.remove(name) {
        return Err(Flow::error_call(call.clone(), "namespace not registered"));
    }
    Ok(Value::Null)
}

fn registered_ns_name(call: &Value, v: &Value) -> RResult<Sym> {
    let v = match v {
        Value::Str(_) | Value::Sym(_) => v.clone(),
        other => crate::coerce::coerce_vector(other, VType::Str, &mut Default::default())?,
    };
    check_ns_name(call, &v)
}

// R: envir.c:do_getRegNS (op 0)
fn do_get_registered_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = registered_ns_name(call, &arg(&args, 0))?;
    Ok(i.ns_registry.get_local(name).unwrap_or(Value::Null))
}

// R: envir.c:do_getRegNS (op 1)
fn do_is_registered_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = registered_ns_name(call, &arg(&args, 0))?;
    Ok(Value::lgl(i.ns_registry.has_local(name)))
}

fn do_get_ns_registry(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.ns_registry.clone()))
}

fn do_import_into_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let (Value::Env(imp), Value::Env(exp)) = (arg(&args, 0), arg(&args, 2)) else {
        return Err(Flow::error_call(call.clone(), "bad environment"));
    };
    let (Value::Str(impn), Value::Str(expn)) = (arg(&args, 1), arg(&args, 3)) else {
        return Err(Flow::error_call(call.clone(), "invalid 'names' argument"));
    };
    if impn.data.len() != expn.data.len() {
        return Err(Flow::error_call(call.clone(), "length of import and export names must match"));
    }
    for (a, b) in impn.data.iter().zip(expn.data.iter()) {
        let (Some(a), Some(b)) = (a, b) else { continue };
        let bsym = Sym::new(b);
        let mut cur = Some(exp.clone());
        let mut found = None;
        while let Some(e) = cur {
            if let Some(v) = e.get_local(bsym) {
                found = Some(v);
                break;
            }
            cur = e.parent();
        }
        let Some(v) = found else {
            return Err(Flow::error_call(call.clone(), format!("exported symbol '{}' has no value", b)));
        };
        i.define(Sym::new(a), v, &imp)?;
    }
    Ok(Value::Null)
}

fn do_attach(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let what = arg(&args, 0);
    let pos = crate::coerce::as_integer(&arg(&args, 1));
    let name = arg(&args, 2);
    if pos == NA_INTEGER {
        return Err(Flow::error_call(call.clone(), "'pos' must be an integer"));
    }
    let new = Env::new(None);
    match &what {
        Value::Null => {}
        Value::List(_) => {
            if let Some(Value::Str(n)) = what.names() {
                for (k, v) in n.data.iter().zip(crate::coerce::to_list(&what)) {
                    if let Some(k) = k {
                        new.set_local(Sym::new(k), v);
                    }
                }
            }
        }
        Value::Env(e) => {
            for s in e.symbols() {
                if let Some(v) = e.get_local(s) {
                    new.set_local(s, v);
                }
            }
        }
        _ => return Err(Flow::error_call(call.clone(), "'attach' only works for lists, data frames and environments")),
    }
    let mut nv = Value::Env(new.clone());
    nv.set_attr_raw(Sym::new("name"), name);
    // Insert after position pos - 1 on the search path.
    let mut before = i.global.clone();
    let mut k = 2;
    while k < pos {
        match before.parent() {
            Some(p) if !p.ptr_eq(&i.base) => before = p,
            _ => break,
        }
        k += 1;
    }
    new.set_parent(before.parent());
    new.set_search(true);
    before.set_parent(Some(new.clone()));
    Ok(nv)
}

fn do_detach(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let pos = crate::coerce::as_integer(&arg(&args, 0));
    if pos == NA_INTEGER || pos < 2 {
        return Err(Flow::error_call(call.clone(), "invalid 'pos' argument"));
    }
    let mut before = i.global.clone();
    for _ in 2..pos {
        match before.parent() {
            Some(p) => before = p,
            None => return Err(Flow::error_call(call.clone(), "invalid 'pos' argument")),
        }
    }
    let Some(target) = before.parent() else {
        return Err(Flow::error_call(call.clone(), "invalid 'pos' argument"));
    };
    if target.ptr_eq(&i.base) {
        return Err(Flow::error_call(call.clone(), "detaching \"package:base\" is not allowed"));
    }
    before.set_parent(target.parent());
    Ok(Value::Env(target))
}

fn do_builtins(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut names: Vec<String> = i.base.symbols().into_iter().map(|s| s.name().to_string()).collect();
    names.sort();
    Ok(Value::strs_from(names))
}

fn do_mk_unbound(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    if let Value::Sym(s) = arg(&args, 0) {
        i.base.remove(s);
    }
    Ok(Value::Null)
}
