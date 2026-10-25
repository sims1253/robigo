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

fn name_of(v: &Value, what: &str) -> RResult<String> {
    match v {
        Value::Sym(s) => Ok(s.name().to_string()),
        Value::Str(s) if s.data.len() == 1 && s.data[0].is_some() => Ok(s.data[0].as_ref().unwrap().to_string()),
        _ => Err(Flow::error(format!("bad {} name", what))),
    }
}

/// Find a namespace environment, or report R's loadNamespace error.
pub fn get_namespace(i: &mut Interp, pkg: &str) -> RResult<Env> {
    if pkg == "base" {
        return Ok(i.base_ns.clone());
    }
    if let Some(e) = i.namespaces.get(pkg) {
        return Ok(e.clone());
    }
    let call = Value::call_sym("loadNamespace", vec![Value::sym("x")]);
    Err(Flow::error_call(call, format!("there is no package called \u{2018}{}\u{2019}", pkg)))
}

fn ns_value(i: &mut Interp, call: &Value, args: Vec<Arg>, exported: bool) -> RResult<Value> {
    if args.len() != 2 {
        return Err(Flow::error_call(call.clone(), "2 arguments passed to '::' which requires 2"));
    }
    let pkg = name_of(&args[0].value, "package")?;
    let name = name_of(&args[1].value, "variable")?;
    let ns = get_namespace(i, &pkg)?;
    let sym = Sym::new(&name);
    i.visible = true;
    match ns.get_local(sym) {
        Some(v) => i.force(v),
        None => {
            if pkg == "base" {
                return Err(Flow::error_call(
                    call.clone(),
                    format!("object '{}' not found", name),
                ));
            }
            if exported {
                Err(Flow::error_call(
                    call.clone(),
                    format!("'{}' is not an exported object from 'namespace:{}'", name, pkg),
                ))
            } else {
                Err(Flow::error_call(call.clone(), format!("object '{}' not found", name)))
            }
        }
    }
}

fn do_colon2(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    ns_value(i, call, args, true)
}

fn do_colon3(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    ns_value(i, call, args, false)
}

fn do_get_ns_value(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let ns = arg(&args, 0);
    let name = name_of(&arg(&args, 1), "variable")?;
    let exported = crate::coerce::as_logical(&arg(&args, 2)) == TRUE;
    let pkg = match &ns {
        Value::Env(e) => {
            let sym = Sym::new(&name);
            return match e.get_local(sym) {
                Some(v) => i.force(v),
                None => Err(Flow::error_call(call.clone(), format!("object '{}' not found", name))),
            };
        }
        other => name_of(other, "namespace")?,
    };
    ns_value(i, call, vec![Arg::pos(Value::str(&pkg)), Arg::pos(Value::str(&name))], exported)
}

fn do_register_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = name_of(&arg(&args, 0), "namespace")?;
    let Value::Env(e) = arg(&args, 1) else {
        return Err(Flow::error_call(call.clone(), "bad namespace"));
    };
    if i.namespaces.contains_key(&name) {
        return Err(Flow::error_call(call.clone(), "namespace already registered"));
    }
    i.namespaces.insert(name, e);
    Ok(Value::Null)
}

fn do_unregister_ns(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = name_of(&arg(&args, 0), "namespace")?;
    if i.namespaces.remove(&name).is_none() {
        return Err(Flow::error_call(call.clone(), "namespace not registered"));
    }
    Ok(Value::Null)
}

fn do_get_registered_ns(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = name_of(&arg(&args, 0), "namespace")?;
    if name == "base" {
        return Ok(Value::Env(i.base_ns.clone()));
    }
    Ok(i.namespaces.get(&name).map(|e| Value::Env(e.clone())).unwrap_or(Value::Null))
}

fn do_is_registered_ns(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = name_of(&arg(&args, 0), "namespace")?;
    Ok(Value::lgl(name == "base" || i.namespaces.contains_key(&name)))
}

fn do_get_ns_registry(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let e = Env::new(Some(i.empty.clone()));
    e.set_local(Sym::new("base"), Value::Env(i.base_ns.clone()));
    for (k, v) in &i.namespaces {
        e.set_local(Sym::new(k), Value::Env(v.clone()));
    }
    Ok(Value::Env(e))
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
