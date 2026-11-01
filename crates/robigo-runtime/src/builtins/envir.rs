//! Environments and frames: GNU R's envir.c and context.c.

use std::rc::Rc;

use super::{arg, internal, lgl_arg, prim, str_arg};
use crate::coerce::{as_integer, as_logical};
use crate::env::{BIND_ACTIVE, Env};
use crate::error::{Flow, RResult};
use crate::interp::{Context, Interp};
use crate::sym::{Sym, s};
use crate::value::*;

pub static DEFS: &[Builtin] = &[
    prim("globalenv", do_globalenv, 0),
    prim("emptyenv", do_emptyenv, 0),
    prim("baseenv", do_baseenv, 0),
    prim("environment<-", do_set_environment, 2),
    prim("as.environment", do_as_environment, 1),
    internal("environment", do_environment, 1),
    internal("new.env", do_new_env, 3),
    internal("parent.env", do_parent_env, 1),
    internal("parent.env<-", do_set_parent_env, 2),
    internal("environmentName", do_environment_name, 1),
    internal("assign", do_assign, 4).invisible(),
    internal("get", do_get, 4),
    internal("get0", do_get0, 5),
    internal("mget", do_mget, 5),
    internal("exists", do_exists, 4),
    internal("remove", do_remove, 3).invisible(),
    internal("ls", do_ls, 3),
    internal("env2list", do_env2list, 3),
    internal("list2env", do_list2env, 2),
    internal("sys.call", do_sys_call, 1),
    internal("sys.function", do_sys_function, 1),
    internal("sys.frame", do_sys_frame, 1),
    internal("sys.nframe", do_sys_nframe, 0),
    internal("sys.calls", do_sys_calls, 0),
    internal("sys.frames", do_sys_frames, 0),
    internal("sys.parent", do_sys_parent, 1),
    internal("sys.parents", do_sys_parents, 0),
    internal("sys.on.exit", do_sys_on_exit, 0),
    internal("parent.frame", do_parent_frame, 1),
    internal("eval", do_eval, 3).custom_vis(),
    internal("delayedAssign", do_delayed_assign, 4).invisible(),
    internal("makeActiveBinding", do_make_active_binding, 3).invisible(),
    internal("lockEnvironment", do_lock_environment, 2).invisible(),
    internal("environmentIsLocked", do_environment_is_locked, 1),
    internal("lockBinding", do_lock_binding, 2).invisible(),
    internal("unlockBinding", do_unlock_binding, 2).invisible(),
    internal("bindingIsLocked", do_binding_is_locked, 2),
    internal("bindingIsActive", do_binding_is_active, 2),
    internal("topenv", do_topenv, 2),
    internal("isNamespaceEnv", do_is_namespace_env, 1),
    internal("search", do_search, 0),
    super::prim("pos.to.env", do_pos_to_env, 1),
];

fn do_globalenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.global.clone()))
}

fn do_emptyenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.empty.clone()))
}

fn do_baseenv(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::Env(i.base.clone()))
}

// R: coerce.c:asCharacterFactor
pub fn as_character_factor(v: &Value) -> RResult<Value> {
    if !v.inherits("factor") {
        return Err(Flow::error("attempting to coerce non-factor"));
    }
    let Some(Value::Str(labels)) = v.get_attr(s::LEVELS) else {
        return Err(Flow::error("malformed factor"));
    };
    let codes = match v {
        Value::Int(x) | Value::Lgl(x) => &x.data,
        _ => return Err(Flow::error("malformed factor")),
    };
    let mut out: Vec<RStr> = Vec::with_capacity(codes.len());
    for &c in codes {
        if c == NA_INTEGER {
            out.push(None);
        } else if c >= 1 && c as usize <= labels.data.len() {
            out.push(labels.data[c as usize - 1].clone());
        } else {
            return Err(Flow::error("malformed factor"));
        }
    }
    Ok(Value::strs(out))
}

/// Convert something to an environment (R's `simple_as_environment` plus
/// `pos.to.env` for numbers).
pub fn as_env(interp: &mut Interp, v: &Value, call: &Value) -> RResult<Env> {
    match v {
        Value::Env(e) => Ok(e.clone()),
        Value::Real(_) | Value::Int(_) => {
            let n = as_integer(v);
            pos_to_env(interp, n, call)
        }
        Value::Str(sv) if sv.data.len() == 1 => {
            let name = sv.data[0].as_deref().unwrap_or("");
            // R: envir.c:matchEnvir
            match name {
                ".GlobalEnv" => Ok(interp.global.clone()),
                "package:base" => Ok(interp.base.clone()),
                _ => {
                    let mut t = interp.global.parent();
                    while let Some(e) = t {
                        if e.ptr_eq(&interp.empty) {
                            break;
                        }
                        if let Some(Value::Str(n)) = Value::Env(e.clone()).get_attr(s::NAME) {
                            if n.data.first().and_then(|s| s.as_deref()) == Some(name) {
                                return Ok(e);
                            }
                        }
                        t = e.parent();
                    }
                    Err(Flow::error_call(call.clone(), format!("no item called \"{}\" on the search list", name)))
                }
            }
        }
        Value::List(_) => {
            // R: envir.c:do_as_environment calls list2env(x, NULL, emptyenv()),
            // whose new.env() is hashed only for more than 100 elements
            // (`hash = (length(x) > 100), size = max(29L, length(x))`)
            let e = if v.len() > 100 {
                Env::new_hashed(Some(interp.empty.clone()), v.len() as i32)
            } else {
                Env::new(Some(interp.empty.clone()))
            };
            list_into_env(v, &e);
            Ok(e)
        }
        Value::Null => Err(Flow::error_call(call.clone(), "using 'as.environment(NULL)' is defunct")),
        _ => Err(Flow::error_call(call.clone(), "invalid object for 'as.environment'")),
    }
}

fn list_into_env(v: &Value, e: &Env) {
    let items = crate::coerce::to_list(v);
    if let Some(Value::Str(n)) = v.names() {
        for (nm, val) in n.data.iter().zip(items) {
            if let Some(nm) = nm {
                if !nm.is_empty() {
                    e.set_local(Sym::new(nm), val);
                }
            }
        }
    }
}

fn pos_to_env(interp: &mut Interp, n: i32, call: &Value) -> RResult<Env> {
    if n == -1 {
        // R: envir.c:do_pos2env: the calling environment of the innermost
        // function context.
        return match interp.current_frame() {
            Some(i) => Ok(interp.frames[i].sysparent.clone()),
            None => Err(Flow::error_call(call.clone(), "no enclosing environment")),
        };
    }
    if n == NA_INTEGER || n < 1 {
        return Err(Flow::error_call(call.clone(), "invalid 'pos' argument"));
    }
    // Walk the search path: global, ..., base.
    let mut e = Some(interp.global.clone());
    let mut k = 1;
    while let Some(env) = e {
        if k == n {
            return Ok(env);
        }
        if env.ptr_eq(&interp.empty) {
            break;
        }
        e = env.parent();
        k += 1;
    }
    Err(Flow::error_call(call.clone(), "invalid 'pos' argument"))
}

fn do_as_environment(i: &mut Interp, call: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    if v.is_object() {
        if let Some(r) = i.try_dispatch("as.environment", call, &args, env)? {
            return Ok(r);
        }
    }
    // R: envir.c:do_as_environment (OBJSXP)
    if let Value::Obj(_) = &v {
        return match i.get_s4_data_slot(&v, Some("environment"))? {
            Value::Env(e) => Ok(Value::Env(e)),
            _ => Err(Flow::error_call(call.clone(), "S4 object does not extend class \"environment\"")),
        };
    }
    Ok(Value::Env(as_env(i, &v, call)?))
}

// R: builtin.c:do_envirgets
fn do_set_environment(i: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &Env) -> RResult<Value> {
    super::check_arity(&args, 2, "environment<-", call)?;
    let mut f = std::mem::replace(&mut args[0].value, Value::Null);
    let e = s4_as_env(i, std::mem::replace(&mut args[1].value, Value::Null))?;
    if !matches!(e, Value::Env(_) | Value::Null) {
        return Err(Flow::error("replacement object is not an environment"));
    }
    if let Value::Closure(c) = &mut f {
        let Value::Env(e) = e else {
            return Err(Flow::error("use of NULL environment is defunct"));
        };
        Rc::make_mut(c).env = e;
        Ok(f)
    } else if matches!(f, Value::Builtin(_)) && !e.is_null() {
        i.warning("setting environment(<primitive function>) is not possible and trying it is deprecated")?;
        Ok(f)
    } else {
        crate::attrib::set_attrib(f, s::DOT_ENVIRONMENT, e)
    }
}

fn do_environment(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match arg(&args, 0) {
        Value::Null => {
            // The calling environment of `environment()`.
            let Some(idx) = i.current_frame() else { return Ok(Value::Env(i.global.clone())) };
            Ok(Value::Env(i.frames[idx].sysparent.clone()))
        }
        Value::Closure(c) => Ok(Value::Env(c.env.clone())),
        Value::Builtin(_) => Ok(Value::Null),
        v => Ok(v.get_attr(s::DOT_ENVIRONMENT).unwrap_or(Value::Null)),
    }
}

/// builtin.c's `simple_as_environment`: an S4 object extending
/// "environment" stands for its environment; anything else is unchanged.
fn s4_as_env(i: &mut Interp, v: Value) -> RResult<Value> {
    match i.simple_as_environment(&v)? {
        Some(e) => Ok(Value::Env(e)),
        None => Ok(v),
    }
}

fn do_new_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let parent = match s4_as_env(i, arg(&args, 1))? {
        Value::Env(e) => e,
        Value::Null => return Err(Flow::error_call(call.clone(), "use of NULL environment is defunct")),
        _ => return Err(Flow::error_call(call.clone(), "'enclos' must be an environment")),
    };
    // R: builtin.c:do_newenv
    let hash = crate::coerce::as_integer(&arg(&args, 0));
    if hash == 0 {
        return Ok(Value::Env(Env::new(Some(parent))));
    }
    let size = match crate::coerce::as_integer(&arg(&args, 2)) {
        NA_INTEGER => 0,
        n => n,
    };
    Ok(Value::Env(Env::new_hashed(Some(parent), size)))
}

fn do_parent_env(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    match s4_as_env(i, arg(&args, 0))? {
        Value::Env(e) => {
            if e.ptr_eq(&i.empty) {
                return Err(Flow::error("the empty environment has no parent"));
            }
            Ok(e.parent().map(Value::Env).unwrap_or(Value::Null))
        }
        _ => Err(Flow::error("argument is not an environment")),
    }
}

fn do_set_parent_env(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let raw_env = arg(&args, 0);
    if raw_env.is_null() {
        return Err(Flow::error("use of NULL environment is defunct"));
    }
    let env = match s4_as_env(i, raw_env)? {
        Value::Env(e) => e,
        _ => return Err(Flow::error("argument is not an environment")),
    };
    if env.ptr_eq(&i.empty) {
        return Err(Flow::error("can not set parent of the empty environment"));
    }
    if env.0.locked.get() {
        if env.ptr_eq(&i.base_ns) || env.namespace_name().is_some() {
            return Err(Flow::error("can not set the parent environment of a namespace"));
        }
        // R: builtin.c:R_IsImportsEnv — a locked environment whose parent is
        // the base namespace and whose "name" attribute starts with
        // "imports:". R-level code creates those with `new.env()` plus
        // `attr(impenv, "name") <- ...` (rlib/base/R/namespace.R), so the
        // name is an attribute, not a special name.
        if env.is_imports_env(&i.base_ns) {
            return Err(Flow::error("can not set the parent environment of package imports"));
        }
    }
    let raw_parent = arg(&args, 1);
    if raw_parent.is_null() {
        return Err(Flow::error("use of NULL environment is defunct"));
    }
    let parent = match s4_as_env(i, raw_parent)? {
        Value::Env(e) => e,
        _ => return Err(Flow::error("'parent' is not an environment")),
    };
    // R: memory.c:SET_ENCLOS — reject a parent chain that closes a cycle.
    if env.parent_cycle(Some(&parent)) {
        return Err(Flow::error("cycles in parent chains are not allowed"));
    }
    env.set_parent(Some(parent));
    Ok(arg(&args, 0))
}

fn do_environment_name(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let v = arg(&args, 0);
    let name = match &v {
        Value::Env(e) => {
            if e.ptr_eq(&i.global) {
                "R_GlobalEnv".to_string()
            } else if e.ptr_eq(&i.base) {
                "base".to_string()
            } else if e.ptr_eq(&i.empty) {
                "R_EmptyEnv".to_string()
            } else if let Some(n) = e.package_env_name() {
                n.to_string()
            } else if let Some(n) = e.namespace_name() {
                n.to_string()
            } else if let Some(Value::Str(n)) = v.get_attr(s::NAME) {
                n.data.first().cloned().flatten().map(|s| s.to_string()).unwrap_or_default()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    Ok(Value::str(&name))
}

fn env_arg(i: &mut Interp, v: &Value, call: &Value, what: &str) -> RResult<Env> {
    match v {
        Value::Env(e) => Ok(e.clone()),
        // R: envir.c:simple_as_environment (assign, remove, ls, the locking
        // functions, makeActiveBinding; mget, list2env and delayedAssign
        // check for an environment first, below).
        Value::Obj(_) => match i.simple_as_environment(v)? {
            Some(e) => Ok(e),
            None => Err(Flow::error_call(call.clone(), format!("invalid '{}' argument", what))),
        },
        Value::Null => Err(Flow::error_call(call.clone(), "use of NULL environment is defunct")),
        _ => {
            let e = as_env(i, v, call);
            e.map_err(|_| Flow::error_call(call.clone(), format!("invalid '{}' argument", what)))
        }
    }
}

fn do_assign(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = match arg(&args, 0) {
        Value::Str(s) if !s.data.is_empty() => match &s.data[0] {
            Some(n) => n.clone(),
            None => return Err(Flow::error_call(call.clone(), "invalid first argument")),
        },
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    let value = arg(&args, 1);
    let env = env_arg(i, &arg(&args, 2), call, "envir")?;
    let inherits = lgl_arg(&arg(&args, 3), "inherits")?;
    let sym = Sym::new(&name);
    if inherits {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if e.has_local(sym) {
                i.define(sym, value.clone(), &e)?;
                return Ok(value);
            }
            cur = e.parent();
        }
        let g = i.global.clone();
        i.define(sym, value.clone(), &g)?;
    } else {
        i.define(sym, value.clone(), &env)?;
    }
    Ok(value)
}

// R: envir.c:str2mode, util.c:TypeTable
fn lookup_mode(v: &Value) -> RResult<crate::value::RString> {
    let mode = match v {
        Value::Str(s) if !s.data.is_empty() => s.data[0].clone().unwrap_or_else(|| crate::value::RString::from("NA")),
        _ => return Err(Flow::error("invalid 'mode' argument")),
    };
    match &*mode {
        "NULL" | "symbol" | "pairlist" | "closure" | "environment" | "promise" | "language" | "special" | "builtin"
        | "char" | "logical" | "integer" | "double" | "complex" | "character" | "..." | "any" | "expression"
        | "list" | "externalptr" | "bytecode" | "weakref" | "raw" | "S4" | "object" | "numeric" | "name"
        | "function" => Ok(mode),
        _ => Err(Flow::error(format!("invalid 'mode' argument '{}'", mode))),
    }
}

// R: envir.c:findVar1mode. Lookup uses mode, merging integer/double and
// all three function types, rather than requiring the exact storage type.
fn mode_matches(v: &Value, mode: &str) -> bool {
    match mode {
        "any" => true,
        "function" | "closure" | "builtin" | "special" => v.is_function(),
        "numeric" | "integer" | "double" => matches!(v, Value::Int(_) | Value::Real(_)),
        "name" => matches!(v, Value::Sym(_)),
        "S4" | "object" => false,
        other => v.type_name() == other,
    }
}

// R: envir.c:do_get (shared by get, get0 and exists).
fn lookup_args(i: &mut Interp, args: &[Arg]) -> RResult<(Sym, Env, crate::value::RString, bool)> {
    let sym = match arg(args, 0) {
        Value::Sym(s) => s,
        Value::Str(s) if !s.data.is_empty() && s.data[0].as_deref() != Some("") => {
            if s.data.len() > 1 {
                return Err(Flow::error("first argument has length > 1"));
            }
            Sym::new(s.data[0].as_deref().unwrap_or("NA"))
        }
        _ => return Err(Flow::error("invalid first argument")),
    };
    let env = match arg(args, 1) {
        Value::Int(_) | Value::Real(_) => {
            let n = as_integer(&arg(args, 1));
            if n == NA_INTEGER {
                return Err(Flow::error("NA argument is invalid"));
            }
            if n == 0 {
                i.global.clone()
            } else {
                match nth_frame(i, n, i.current_frame())? {
                    Some(k) => i.frames[k].cloenv.clone(),
                    None => i.global.clone(),
                }
            }
        }
        Value::Env(e) => e,
        Value::Null => return Err(Flow::error("use of NULL environment is defunct")),
        // R: envir.c:simple_as_environment
        v => match i.simple_as_environment(&v)? {
            Some(e) => e,
            None => return Err(Flow::error("invalid 'envir' argument")),
        },
    };
    let mode = lookup_mode(&arg(args, 2))?;
    let inherits = lgl_arg(&arg(args, 3), "inherits")?;
    Ok((sym, env, mode, inherits))
}

/// Look up a variable with a mode filter (R's `findVar1mode`).
fn find_var_mode(i: &mut Interp, sym: Sym, env: &Env, mode: &str, inherits: bool) -> RResult<Option<Value>> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        // R: envir.c:findVarInFrame3 reads SYMBOL_BINDING_VALUE in base;
        // the missing-argument symbol has itself as its binding value.
        if sym.is_empty() && (e.ptr_eq(&i.base) || e.ptr_eq(&i.base_ns)) {
            let v = Value::missing_arg();
            return Ok(mode_matches(&v, mode).then_some(v));
        }
        let b = e.0.frame().borrow().get(sym).cloned();
        if let Some(b) = b {
            let mut v = b.value;
            if b.flags & BIND_ACTIVE != 0 {
                v = i.call_value(&v, vec![], &e)?;
            }
            if mode != "any" || matches!(v, Value::Promise(_)) {
                v = i.force(v)?;
            }
            if mode_matches(&v, mode) {
                return Ok(Some(v));
            }
        }
        if !inherits {
            break;
        }
        cur = e.parent();
    }
    Ok(None)
}

fn do_get(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let (sym, env, mode, inherits) = lookup_args(i, &args)?;
    let name = sym.name();
    match find_var_mode(i, sym, &env, &mode, inherits)? {
        Some(v) => {
            if v.is_missing_arg() {
                return Err(crate::conditions::missing_arg_error(sym, call.clone(), "getMissingError"));
            }
            Ok(v)
        }
        None => {
            if &*mode == "any" {
                Err(Flow::error(format!("object '{}' not found", name)))
            } else {
                Err(Flow::error(format!("object '{}' of mode '{}' was not found", name, mode)))
            }
        }
    }
}

fn do_get0(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let (sym, env, mode, inherits) = lookup_args(i, &args)?;
    match find_var_mode(i, sym, &env, &mode, inherits)? {
        Some(v) if v.is_missing_arg() => {
            Err(crate::conditions::missing_arg_error(sym, call.clone(), "getMissingError"))
        }
        Some(v) => Ok(v),
        None => Ok(arg(&args, 4)),
    }
}

fn do_mget(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let names = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        _ => return Err(Flow::error_call(call.clone(), "invalid first argument")),
    };
    if let Value::Obj(_) = arg(&args, 1) {
        return Err(Flow::error("second argument must be an environment"));
    }
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let mode = str_arg(&arg(&args, 2), "mode")?;
    let ifnotfound = arg(&args, 3);
    let inherits = lgl_arg(&arg(&args, 4), "inherits")?;
    let mut out = Vec::new();
    for (k, n) in names.iter().enumerate() {
        let n = n.clone().unwrap_or_else(|| crate::value::RString::from("NA"));
        match find_var_mode(i, Sym::new(&n), &env, &mode, inherits)? {
            Some(v) => out.push(v),
            None => {
                let items = crate::coerce::to_list(&ifnotfound);
                if items.is_empty() {
                    return Err(Flow::error_call(call.clone(), format!("value for '{}' not found", n)));
                }
                let d = items[k % items.len()].clone();
                if d.is_function() {
                    let r = i.call_value(&d, vec![Arg::pos(Value::str(&n))], &env)?;
                    out.push(r);
                } else {
                    out.push(d);
                }
            }
        }
    }
    let mut v = Value::list(out);
    v.set_attr_raw(s::NAMES, Value::strs(names));
    Ok(v)
}

fn do_exists(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let (sym, env, mode, inherits) = lookup_args(i, &args)?;
    if &*mode == "any" {
        let mut cur = Some(env);
        while let Some(e) = cur {
            if e.has_local(sym) {
                return Ok(Value::lgl(true));
            }
            if !inherits {
                break;
            }
            cur = e.parent();
        }
        return Ok(Value::lgl(false));
    }
    Ok(Value::lgl(find_var_mode(i, sym, &env, &mode, inherits)?.is_some()))
}

fn do_remove(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let names = match arg(&args, 0) {
        Value::Str(s) => s.data.clone(),
        Value::Null => return Ok(Value::Null),
        _ => return Err(Flow::error("invalid first argument")),
    };
    // R: envir.c:do_remove uses context errors for environment validation.
    let env = match arg(&args, 1) {
        Value::Null => return Err(Flow::error("use of NULL environment is defunct")),
        Value::Env(e) => e,
        v => as_env(i, &v, call).map_err(|_| Flow::error("invalid 'envir' argument"))?,
    };
    let inherits = lgl_arg(&arg(&args, 2), "inherits")?;
    for n in names.iter() {
        let n = n.clone().unwrap_or_else(|| crate::value::RString::from("NA"));
        let sym = Sym::new(&n);
        let mut cur = Some(env.clone());
        let mut done = false;
        // R: envir.c:do_remove, RemoveVariable
        while let Some(e) = cur {
            if e.ptr_eq(&i.empty) {
                break;
            }
            if e.ptr_eq(&i.base_ns) {
                return Err(Flow::error("cannot remove variables from base namespace"));
            }
            if e.ptr_eq(&i.base) {
                return Err(Flow::error("cannot remove variables from the base environment"));
            }
            if e.0.locked.get() {
                return Err(Flow::error("cannot remove bindings from a locked environment"));
            }
            if e.remove(sym) {
                done = true;
                break;
            }
            if !inherits {
                break;
            }
            cur = e.parent();
        }
        if !done {
            i.warning(format!("object '{}' not found", n))?;
        }
    }
    Ok(Value::Null)
}

fn env_names(e: &Env, all: bool, sorted: bool) -> Vec<String> {
    let syms = if sorted { e.symbols_unordered() } else { e.symbols() };
    let mut names: Vec<String> =
        syms.into_iter().map(|s| s.name().to_string()).filter(|n| all || !n.starts_with('.')).collect();
    if sorted {
        names.sort();
    }
    names
}

fn do_ls(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "envir")?;
    let all = lgl_arg(&arg(&args, 1), "all.names")?;
    let sorted = lgl_arg(&arg(&args, 2), "sorted")?;
    Ok(Value::strs_from(env_names(&env, all, sorted)))
}

fn do_env2list(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "x")?;
    let all = lgl_arg(&arg(&args, 1), "all.names")?;
    let sorted = lgl_arg(&arg(&args, 2), "sorted")?;
    let names = env_names(&env, all, sorted);
    let mut vals = Vec::new();
    for n in &names {
        let v = env.get_local(Sym::new(n)).unwrap_or(Value::Null);
        vals.push(i.force(v)?);
    }
    let mut v = Value::list(vals);
    // R: envir.c:do_env2list — an empty environment yields a list with no
    // names attribute (k == 0 returns before setAttrib), so it prints as
    // `list()`, not `named list()`.
    if !names.is_empty() {
        v.set_attr_raw(s::NAMES, Value::strs_from(names));
    }
    Ok(v)
}

fn do_list2env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let x = arg(&args, 0);
    if let Value::Obj(_) = arg(&args, 1) {
        return Err(Flow::error("'envir' argument must be an environment"));
    }
    let env = env_arg(i, &arg(&args, 1), call, "envir")?;
    let names = x.names();
    let items = crate::coerce::to_list(&x);
    if !items.is_empty() {
        match names {
            Some(Value::Str(n)) => {
                for (nm, v) in n.data.iter().zip(items) {
                    i.define(Sym::new(nm.as_deref().unwrap_or("NA")), v, &env)?;
                }
            }
            _ => {
                return Err(Flow::error_call(
                    call.clone(),
                    "names(x) must be a character vector of the same length as x",
                ));
            }
        }
    }
    Ok(Value::Env(env))
}

// ----- frames ---------------------------------------------------------

/// Function frames (excluding builtin dispatch frames), outermost first.
/// Function frames up to `c`, innermost first.
fn fn_frames_from(i: &Interp, c: usize) -> impl Iterator<Item = usize> + Clone + '_ {
    (0..=c).rev().filter(|&k| !i.frames[k].builtin)
}

/// The context for `sys.xxx()`: the function frame whose environment is
/// the caller of the sys.xxx closure (R's getLexicalContext).
fn lexical_context(i: &Interp) -> Option<usize> {
    let cur = i.current_frame()?;
    let t = i.frames[cur].sysparent.clone();
    (0..=cur).rev().find(|&k| !i.frames[k].builtin && i.frames[k].cloenv.ptr_eq(&t))
}

/// framedepth: number of function frames from cptr outwards (inclusive).
fn framedepth(i: &Interp, cptr: Option<usize>) -> i32 {
    match cptr {
        None => 0,
        Some(c) => fn_frames_from(i, c).count() as i32,
    }
}

/// Walk function frames from cptr outwards; return the frame index for
/// R's `n` (after conversion), or None for "top level".
fn nth_frame(i: &Interp, n: i32, cptr: Option<usize>) -> RResult<Option<usize>> {
    let mut n = if n > 0 { framedepth(i, cptr) - n } else { -n };
    if n < 0 {
        return Err(Flow::error("not that many frames on the stack"));
    }
    for k in cptr.into_iter().flat_map(|c| fn_frames_from(i, c)) {
        if n == 0 {
            return Ok(Some(k));
        }
        n -= 1;
    }
    if n == 0 {
        return Ok(None);
    }
    Err(Flow::error("not that many frames on the stack"))
}

fn do_sys_call(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' argument"));
    }
    let cptr = lexical_context(i);
    Ok(match nth_frame(i, n, cptr)? {
        Some(k) => i.frames[k].call.clone(),
        None => Value::Null,
    })
}

fn do_sys_function(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' value"));
    }
    let cptr = lexical_context(i);
    match nth_frame(i, n, cptr)? {
        Some(k) => Ok(i.frames[k].function.clone()),
        None => Err(Flow::error("not that many frames on the stack")),
    }
}

fn do_sys_frame(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'which' argument"));
    }
    if n == 0 {
        return Ok(Value::Env(i.global.clone()));
    }
    let cptr = lexical_context(i);
    Ok(match nth_frame(i, n, cptr)? {
        Some(k) => Value::Env(i.frames[k].cloenv.clone()),
        None => Value::Env(i.global.clone()),
    })
}

fn do_sys_nframe(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    Ok(Value::int(framedepth(i, cptr)))
}

fn do_sys_calls(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut out = Vec::new();
    for k in 1..=nframe {
        if let Some(f) = nth_frame(i, k, cptr)? {
            out.push(Arg::pos(i.frames[f].call.clone()));
        }
    }
    Ok(Value::pairlist(out))
}

fn do_sys_frames(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut out = Vec::new();
    for k in 1..=nframe {
        if let Some(f) = nth_frame(i, k, cptr)? {
            out.push(Arg::pos(Value::Env(i.frames[f].cloenv.clone())));
        }
    }
    Ok(Value::pairlist(out))
}

/// R_sysparent
fn sysparent(i: &Interp, mut n: i32, cptr: Option<usize>) -> i32 {
    let Some(c) = cptr else { return 0 };
    let mut frames = fn_frames_from(i, c);
    while n > 1 && frames.next().is_some() {
        n -= 1;
    }
    let Some(first) = frames.clone().next() else { return 0 };
    let s = &i.frames[first].sysparent;
    if s.ptr_eq(&i.global) {
        return 0;
    }
    let mut j = 0;
    let mut nn = n;
    for k in frames {
        j += 1;
        if i.frames[k].cloenv.ptr_eq(s) {
            nn = j;
        }
    }
    let r = j - nn + 1;
    r.max(0)
}

fn do_sys_parent(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER {
        return Err(Flow::error("invalid 'n' argument"));
    }
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let mut k = nframe;
    while n > 0 {
        k = sysparent(i, nframe - k + 1, cptr);
        n -= 1;
    }
    Ok(Value::int(k))
}

fn do_sys_parents(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let cptr = lexical_context(i);
    let nframe = framedepth(i, cptr);
    let out: Vec<i32> = (0..nframe).map(|k| sysparent(i, nframe - k, cptr)).collect();
    Ok(Value::ints(out))
}

fn do_sys_on_exit(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let Some(c) = lexical_context(i) else { return Ok(Value::Null) };
    let oe = &i.frames[c].on_exit;
    Ok(match oe.len() {
        0 => Value::Null,
        1 => oe[0].clone(),
        _ => Value::lang(Value::Sym(s::BRACE), oe.iter().map(|e| Arg::pos(e.clone())).collect()),
    })
}

fn do_parent_frame(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    if n == NA_INTEGER || n < 1 {
        return Err(Flow::error("invalid 'n' value"));
    }
    // R_findParentContext(R_GlobalContext, n), where R_GlobalContext is
    // the frame of the parent.frame closure itself.
    let Some(mut cptr) = i.current_frame() else { return Ok(Value::Env(i.global.clone())) };
    let mut n = n;
    loop {
        let target = i.frames[cptr].sysparent.clone();
        let found = (0..cptr).rev().find(|&k| !i.frames[k].builtin && i.frames[k].cloenv.ptr_eq(&target));
        match found {
            Some(k) => {
                if n == 1 {
                    return Ok(Value::Env(i.frames[k].sysparent.clone()));
                }
                n -= 1;
                cptr = k;
            }
            None => return Ok(Value::Env(i.global.clone())),
        }
    }
}

fn do_eval(i: &mut Interp, call: &Value, args: Vec<Arg>, _env: &Env) -> RResult<Value> {
    let expr = arg(&args, 0);
    let mut envir = arg(&args, 1);
    let enclos = arg(&args, 2);
    // R: eval.c:do_eval — an S4 object's data part (usually an environment).
    if envir.is_s4() && matches!(envir, Value::Obj(_)) {
        envir = i.get_s4_data_slot(&envir, None)?;
    }
    let enclos_env = match &enclos {
        Value::Env(e) => Some(e.clone()),
        Value::Null => None,
        _ => return Err(Flow::error_call(call.clone(), "invalid 'enclos' argument of type 'list'")),
    };
    let env = match &envir {
        Value::Env(e) => e.clone(),
        Value::Null => enclos_env.clone().unwrap_or_else(|| i.base.clone()),
        // R: eval.c:do_eval: NewEnvironment() over the list's elements, in
        // order (with repeated names, lookups find the first)
        Value::List(_) => {
            let e = Env::new(Some(enclos_env.clone().unwrap_or_else(|| i.base.clone())));
            if let Some(Value::Str(n)) = envir.names() {
                for (nm, val) in n.data.iter().zip(crate::coerce::to_list(&envir)) {
                    if let Some(nm) = nm.as_deref().filter(|nm| !nm.is_empty()) {
                        e.bind_arg(Sym::new(nm), val, 0);
                    }
                }
            }
            e
        }
        Value::Pairlist(p) => {
            let e = Env::new(Some(enclos_env.clone().unwrap_or_else(|| i.base.clone())));
            for a in &p.items {
                if let Some(t) = a.tag {
                    e.bind_arg(t, a.value.clone(), 0);
                }
            }
            e
        }
        Value::Int(_) | Value::Real(_) => {
            // R: eval.c:do_eval
            if envir.len() != 1 {
                return Err(Flow::error("numeric 'envir' arg not of length one"));
            }
            let n = as_integer(&envir);
            if n == NA_INTEGER {
                return Err(Flow::error(format!("invalid 'envir' argument of type '{}'", envir.type_name())));
            }
            // R: eval.c:do_eval uses R_sysframe(n, R_GlobalContext).
            if n == 0 {
                i.global.clone()
            } else {
                let cptr = i.current_frame();
                match nth_frame(i, n, cptr)? {
                    Some(k) => i.frames[k].cloenv.clone(),
                    None => i.global.clone(),
                }
            }
        }
        _ => {
            return Err(Flow::error_call(
                call.clone(),
                format!("invalid 'envir' argument of type '{}'", envir.type_name()),
            ));
        }
    };
    // eval establishes a function-like context so that return() and
    // sys.function() behave as in GNU R.
    let ecall = i.caller_call();
    let sysparent = match i.current_frame() {
        Some(k) => i.frames[k].cloenv.clone(),
        None => i.global.clone(),
    };
    i.frames.push(Context {
        call: ecall,
        function: Value::Builtin(super::lookup_internal("eval").unwrap()),
        cloenv: env.clone(),
        sysparent,
        promargs: vec![],
        on_exit: vec![],
        handlers: i.handlers.clone(),
        restarts: i.restarts.clone(),
        generic: false,
        builtin: false,
    });
    let r = (|| -> RResult<Value> {
        match &expr {
            Value::Expr(ev) => {
                let mut v = Value::Null;
                for e in ev.data.iter() {
                    v = i.eval(e, &env)?;
                }
                Ok(v)
            }
            // R: eval.c:do_eval also evaluates BCODESXP, rather than returning it.
            #[cfg(feature = "bytecode-fidelity")]
            Value::Bytecode(_) => i.eval(&expr, &env),
            Value::Sym(_) | Value::Lang(_) | Value::Promise(_) => i.eval(&expr, &env),
            v => {
                i.visible = true;
                Ok(v.clone())
            }
        }
    })();
    let r = match r {
        Err(Flow::Return(v, t)) if t.ptr_eq(&env) => Ok(v),
        r => r,
    };
    let vis = i.visible;
    let ctx = i.frames.pop().unwrap();
    i.handlers = ctx.handlers;
    i.restarts = ctx.restarts;
    i.visible = vis;
    r
}

fn do_delayed_assign(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let name = str_arg(&arg(&args, 0), "x")?;
    let value = arg(&args, 1);
    for (k, what) in [(2, "eval.env"), (3, "assign.env")] {
        if let Value::Obj(_) = arg(&args, k) {
            return Err(Flow::error(format!("invalid '{}' argument", what)));
        }
    }
    let eval_env = env_arg(i, &arg(&args, 2), call, "eval.env")?;
    let assign_env = env_arg(i, &arg(&args, 3), call, "assign.env")?;
    let p = Value::Promise(Rc::new(Promise::new(value, eval_env)));
    i.define(Sym::new(&name), p, &assign_env)?;
    Ok(Value::Null)
}

fn do_make_active_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = match arg(&args, 0) {
        Value::Sym(s) => s,
        Value::Str(v) if v.data.len() == 1 => Sym::new(v.data[0].as_deref().unwrap_or("")),
        _ => return Err(Flow::error_call(call.clone(), "not a symbol")),
    };
    let f = arg(&args, 1);
    if !f.is_function() {
        return Err(Flow::error_call(call.clone(), "not a function"));
    }
    let env = env_arg(i, &arg(&args, 2), call, "env")?;
    env.set_local_flags(sym, f, BIND_ACTIVE);
    Ok(Value::Null)
}

fn do_lock_environment(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    // R: envir.c:do_lockEnv -- asRbool(bindings) runs before the
    // environment check; its errors are errorcall(call) on the internal's
    // own call.
    let b = arg(&args, 1);
    if b.is_atomic() && b.len() > 1 {
        return Err(Flow::error_call(call.clone(), format!("'length = {}' in coercion to 'logical(1)'", b.len())));
    }
    // R: asLogical2 -- a list or empty vector is NA, not a converted
    // element.
    let bindings = crate::coerce::as_logical(&b);
    if bindings == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "NA in coercion to boolean"));
    }
    // R: envir.c:R_LockEnvironment -- an S4 object's data slot is resolved
    // to the environment; everything else that is not an ENVSXP (NULL
    // included) reports "not an environment" with error()'s context.
    let arg0 = arg(&args, 0);
    let env = match &arg0 {
        Value::Env(e) => e.clone(),
        Value::Obj(_) => match i.simple_as_environment(&arg0)? {
            Some(e) => e,
            None => return Err(Flow::error("not an environment")),
        },
        _ => return Err(Flow::error("not an environment")),
    };
    env.lock(bindings != 0);
    Ok(Value::Null)
}

fn do_environment_is_locked(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let env = env_arg(i, &arg(&args, 0), call, "env")?;
    Ok(Value::lgl(env.0.locked.get()))
}

fn sym_arg(v: &Value, call: &Value) -> RResult<Sym> {
    match v {
        Value::Sym(s) => Ok(*s),
        Value::Str(x) if x.data.len() == 1 => Ok(Sym::new(x.data[0].as_deref().unwrap_or(""))),
        _ => Err(Flow::error_call(call.clone(), "not a symbol")),
    }
}

fn do_lock_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    if !env.set_binding_locked(sym, true) {
        return Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym)));
    }
    Ok(Value::Null)
}

fn do_unlock_binding(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    if !env.set_binding_locked(sym, false) {
        return Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym)));
    }
    Ok(Value::Null)
}

fn do_binding_is_locked(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    match env.binding_flags(sym) {
        Some(f) => Ok(Value::lgl(f & crate::env::BIND_LOCKED != 0)),
        None => Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym))),
    }
}

fn do_binding_is_active(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let sym = sym_arg(&arg(&args, 0), call)?;
    let env = env_arg(i, &arg(&args, 1), call, "env")?;
    match env.binding_flags(sym) {
        Some(f) => Ok(Value::lgl(f & BIND_ACTIVE != 0)),
        None => Err(Flow::error_call(call.clone(), format!("no binding for \"{}\"", sym))),
    }
}

fn do_topenv(i: &mut Interp, _: &Value, args: Vec<Arg>, env: &Env) -> RResult<Value> {
    let e = match arg(&args, 0) {
        Value::Env(e) => e,
        _ => env.clone(),
    };
    Ok(Value::Env(i.topenv(&e)))
}

fn do_is_namespace_env(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    Ok(Value::lgl(match arg(&args, 0) {
        Value::Env(e) => e.ptr_eq(&i.base_ns) || crate::dispatch::is_namespace(&e),
        _ => false,
    }))
}

fn do_search(i: &mut Interp, _: &Value, _: Vec<Arg>, _: &Env) -> RResult<Value> {
    let mut out = vec![".GlobalEnv".to_string()];
    let mut e = i.global.parent();
    while let Some(env) = e {
        if env.ptr_eq(&i.base) {
            out.push("package:base".into());
            break;
        }
        let name = match env.get_attr_name() {
            Some(n) => n,
            None => "<unknown>".into(),
        };
        out.push(name);
        e = env.parent();
    }
    Ok(Value::strs_from(out))
}

fn do_pos_to_env(i: &mut Interp, call: &Value, args: Vec<Arg>, _: &Env) -> RResult<Value> {
    let n = as_integer(&arg(&args, 0));
    Ok(Value::Env(pos_to_env(i, n, call)?))
}

impl Env {
    pub fn get_attr_name(&self) -> Option<String> {
        let a = self.0.attrs.borrow();
        match a.as_ref().and_then(|a| a.get(s::NAME)) {
            Some(Value::Str(v)) => v.data.first().cloned().flatten().map(|s| s.to_string()),
            _ => None,
        }
    }
}

#[allow(dead_code)]
fn unused(_: bool) -> Lgl {
    as_logical(&Value::Null)
}
